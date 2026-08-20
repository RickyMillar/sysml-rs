//! # sysml-store
//!
//! Storage trait and types for SysML v2 model snapshots.
//!
//! This crate defines the interface for storing and retrieving model
//! snapshots with version control.
//!
//! It also hosts the [`SessionArchive`] trait and in-memory implementation
//! for completed-runtime-session persistence — see [`session_archive`] —
//! and the [`WorkflowStore`] sidecar event log for review/process facts
//! that live outside the model — see [`workflow_store`].

pub mod session_archive;
pub mod workflow_store;

pub use session_archive::{
    ArchiveError, ArchiveFilter, ArchivedEvidence, ArchivedSession, ArchivedSessionSummary,
    ArchivedVerdict, EvaluationMode, ExternalEvidence, GoldenMetadata, InMemorySessionArchive,
    MAX_ARCHIVED_SESSIONS, SessionArchive, SessionOrigin, VerdictCounts,
};
pub use workflow_store::{
    fold_element_state, ClearingRecord, InMemoryWorkflowStore, JsonlRecovery, JsonlWorkflowStore,
    NewWorkflowEvent, VerificationAttestationRecord, WorkflowElementState, WorkflowEvent,
    WorkflowEventKind, WorkflowStore, WorkflowStoreError, WORKFLOW_SCHEMA_VERSION,
};

use std::collections::{HashMap, VecDeque};
use sysml_core::json::{from_json_str, to_json_string};
use sysml_core::ModelGraph;
use sysml_id::{CommitId, ProjectId};
use thiserror::Error;

/// Errors that can occur during storage operations.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The requested project was not found.
    #[error("project not found: {0}")]
    ProjectNotFound(String),

    /// The requested commit was not found.
    #[error("commit not found: {0}")]
    CommitNotFound(String),

    /// Serialization failed.
    #[error("serialization error: {0}")]
    SerializationError(String),

    /// Deserialization failed.
    #[error("deserialization error: {0}")]
    DeserializationError(String),

    /// Database error.
    #[error("database error: {0}")]
    DatabaseError(String),

    /// Conflict (e.g., commit already exists).
    #[error("conflict: {0}")]
    Conflict(String),
}

/// Metadata about a snapshot.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotMeta {
    /// The commit ID.
    pub commit: CommitId,
    /// The parent commit ID (None for initial commit).
    pub parent: Option<CommitId>,
    /// Commit message.
    pub message: String,
    /// Timestamp (Unix epoch seconds).
    pub timestamp: u64,
}

impl SnapshotMeta {
    /// Create new snapshot metadata.
    pub fn new(commit: CommitId, message: impl Into<String>) -> Self {
        SnapshotMeta {
            commit,
            parent: None,
            message: message.into(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }

    /// Set the parent commit.
    pub fn with_parent(mut self, parent: CommitId) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Set the timestamp.
    pub fn with_timestamp(mut self, timestamp: u64) -> Self {
        self.timestamp = timestamp;
        self
    }
}

/// Git provenance captured at baseline creation (B6).
///
/// CORROBORATING metadata only, never identity: a baseline's identity is
/// its content-addressed commit digest, which reproduces the content
/// independent of git. This crate stays git-ignorant — the fields are
/// plain data captured by the service layer; absence is the honest state
/// for non-git workspaces (steward ruling 2026-07-16). The remote URL is
/// deliberately NOT recorded (mutable, privacy-leaking, non-identity).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GitProvenance {
    /// HEAD commit SHA at creation time.
    pub sha: String,
    /// Uncommitted changes existed under the workspace at creation time —
    /// the SHA alone does not reproduce the baselined content. Recorded
    /// honestly, never refused (the content digest is the trust anchor).
    pub dirty: bool,
    /// Branch name; `None` on a detached HEAD.
    pub branch: Option<String>,
}

/// One workspace file's identity in a session's provenance manifest
/// (B6 remainder, ninebar Phase 4 §6.2; steward ruling 2026-07-23).
///
/// `path` is workspace-root-RELATIVE so the manifest is reproducible
/// across machines and leaks no absolute path into downloaded reports or
/// blessed baselines (principle 7: honest by construction, not by
/// redaction). When a file resolves outside the root — or no root is
/// known — the canonical URI is kept verbatim: honest, and rare.
///
/// `content_hash` is SHA-256 (hex) of the file's UTF-8 text AS LOADED —
/// reflecting an unsaved editor overlay, not disk — i.e. the bytes the
/// session actually executed, never a manifest-declared checksum (which
/// may be stale). Distinct in concept from [`SessionProvenance::model_digest`]:
/// that hashes diff-compared *graph* fields (span-blind), this hashes
/// *raw file bytes*.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct FileProvenance {
    /// Workspace-relative path (canonical URI when outside/without a root).
    pub path: String,
    /// SHA-256 (hex) of the file's UTF-8 text as loaded.
    pub content_hash: String,
}

/// Provenance captured when an execution session is created (B6 remainder).
///
/// The reconstitution record for a run: which model content the session
/// executed against, corroborated by git when available. Captured once at
/// session mint time (every `*.start` path) and carried verbatim through
/// forks — a fork's orchestrator is a clone of the parent's model state,
/// so re-capturing at fork time would record a graph the fork does not
/// run. Threaded into [`session_archive::ArchivedSession`] on stop so
/// report exports can attribute evidence (`ReportProvenance` on the FE).
///
/// Deliberately NO run-config block (dt/target/kind): those fields are
/// only defined for a subset of session kinds and the report consumer's
/// `runConfig` shape wants different data — burden of proof on inclusion
/// (steward ruling 2026-07-17).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SessionProvenance {
    /// Content digest of the workspace-aware graph at session creation —
    /// the SAME `ModelGraph::content_digest()` the store's commits and
    /// baselines use, so digest equality against a baseline's commit id
    /// is real identity equivalence ("this session ran exactly that
    /// baseline"), not coincidence.
    pub model_digest: String,
    /// Git provenance at creation time; `None` = not captured (non-git
    /// workspace) — never fabricated. Corroborating only, same as
    /// [`BaselineMeta::provenance`].
    #[serde(default)]
    pub git: Option<GitProvenance>,
    /// Absolute workspace root the session executed against, when
    /// resolvable (same root-resolution rule as baseline provenance).
    #[serde(default)]
    pub workspace_root: Option<String>,
    /// Per-file manifest of the workspace at session creation (§6.2):
    /// every non-stdlib source file's [`FileProvenance`] (relative path +
    /// content hash), sorted by `path` for a byte-stable block (an
    /// unchanged workspace yields an identical manifest). Lets a Verify
    /// Report reconstitute exactly which files — at which content — were
    /// verified, beyond the whole-graph `model_digest`. Empty when
    /// captured outside a loaded workspace (unit tests) or read from a
    /// pre-§6.2 archive (`#[serde(default)]` — additive, old archives
    /// still deserialize).
    #[serde(default)]
    pub file_manifest: Vec<FileProvenance>,
}

/// A named, immutable baseline: a pointer to a commit.
///
/// Baselines are the trust anchor for suspect detection and reviews
/// (requirements workbench R9/R10): once created they can never be
/// renamed or retargeted, and the commit they reference is exempt from
/// in-memory eviction (mirroring the session archive's golden-pinning
/// pattern) so a baseline can always be resolved to a real snapshot.
/// There is deliberately no delete in v1 — immutability is the value.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BaselineMeta {
    /// Unique (per project) baseline name, e.g. `"B2 — PDR"`.
    pub name: String,
    /// The commit this baseline points at.
    pub commit: CommitId,
    /// Creation timestamp (Unix epoch seconds).
    pub created_at: u64,
    /// Git provenance at creation time; `None` = not captured (non-git
    /// workspace, or a baseline predating B6) — never fabricated.
    #[serde(default)]
    pub provenance: Option<GitProvenance>,
}

/// A stored snapshot containing metadata and model data.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Metadata about this snapshot.
    pub meta: SnapshotMeta,
    /// The serialized model data (JSON).
    pub data: String,
}

impl Snapshot {
    /// Create a new snapshot from a model graph.
    pub fn new(meta: SnapshotMeta, graph: &ModelGraph) -> Self {
        Snapshot {
            meta,
            data: to_json_string(graph),
        }
    }

    /// Deserialize the model graph.
    pub fn graph(&self) -> Result<ModelGraph, StoreError> {
        from_json_str(&self.data).map_err(|e| StoreError::DeserializationError(e.to_string()))
    }
}

/// Trait for model storage backends.
pub trait Store {
    /// Store a model snapshot.
    ///
    /// # Arguments
    ///
    /// * `project` - The project ID
    /// * `commit` - The commit ID for this snapshot
    /// * `graph` - The model graph to store
    /// * `meta` - Snapshot metadata
    ///
    /// # Returns
    ///
    /// Ok(()) on success, or a StoreError on failure.
    fn put_snapshot(
        &mut self,
        project: &ProjectId,
        meta: SnapshotMeta,
        graph: &ModelGraph,
    ) -> Result<(), StoreError>;

    /// Retrieve a model snapshot.
    ///
    /// # Arguments
    ///
    /// * `project` - The project ID
    /// * `commit` - The commit ID to retrieve
    ///
    /// # Returns
    ///
    /// The model graph if found, None otherwise.
    fn get_snapshot(
        &self,
        project: &ProjectId,
        commit: &CommitId,
    ) -> Result<Option<Snapshot>, StoreError>;

    /// Get the latest commit ID for a project.
    fn latest(&self, project: &ProjectId) -> Result<Option<CommitId>, StoreError>;

    /// List all commits for a project (most recent first).
    fn list_commits(&self, project: &ProjectId) -> Result<Vec<SnapshotMeta>, StoreError>;

    /// List all projects.
    fn list_projects(&self) -> Result<Vec<ProjectId>, StoreError>;

    /// Create a named, immutable baseline pointing at an existing commit.
    ///
    /// Fails with [`StoreError::Conflict`] if the name is already taken in
    /// this project (baselines are never renamed or retargeted), and with
    /// [`StoreError::CommitNotFound`] if the commit has no stored snapshot —
    /// a baseline to nothing would be unresolvable by construction.
    /// `provenance` is caller-captured git state (`None` = not captured).
    fn create_baseline(
        &mut self,
        project: &ProjectId,
        name: &str,
        commit: &CommitId,
        provenance: Option<GitProvenance>,
    ) -> Result<(), StoreError>;

    /// Resolve a baseline name to its commit. `Ok(None)` means the baseline
    /// was never created (an existing baseline always resolves — its commit
    /// is eviction-exempt).
    fn get_baseline(
        &self,
        project: &ProjectId,
        name: &str,
    ) -> Result<Option<CommitId>, StoreError>;

    /// List baselines for a project (most recently created first).
    fn list_baselines(&self, project: &ProjectId) -> Result<Vec<BaselineMeta>, StoreError>;
}

/// Maximum number of commits retained per project.
const MAX_COMMITS_PER_PROJECT: usize = 100;

/// An in-memory store implementation.
///
/// Commits per project are capped at [`MAX_COMMITS_PER_PROJECT`]; when the
/// limit is exceeded the oldest commit (and its snapshot data) are evicted.
#[derive(Debug, Default)]
pub struct InMemoryStore {
    /// Snapshots indexed by (project, commit).
    snapshots: HashMap<(String, String), Snapshot>,
    /// Latest commit for each project.
    latest: HashMap<String, CommitId>,
    /// All commits for each project (in order, oldest first).
    commits: HashMap<String, VecDeque<SnapshotMeta>>,
    /// Baselines per project (in creation order, oldest first).
    baselines: HashMap<String, Vec<BaselineMeta>>,
}

impl InMemoryStore {
    /// Create a new empty in-memory store.
    pub fn new() -> Self {
        InMemoryStore {
            snapshots: HashMap::new(),
            latest: HashMap::new(),
            commits: HashMap::new(),
            baselines: HashMap::new(),
        }
    }

    /// True when any baseline in `project` references `commit` — such
    /// commits are exempt from cap eviction.
    fn is_baselined(&self, project: &str, commit: &CommitId) -> bool {
        self.baselines
            .get(project)
            .is_some_and(|b| b.iter().any(|m| &m.commit == commit))
    }
}

impl Store for InMemoryStore {
    fn put_snapshot(
        &mut self,
        project: &ProjectId,
        meta: SnapshotMeta,
        graph: &ModelGraph,
    ) -> Result<(), StoreError> {
        let project_key = project.as_str().to_owned();
        let commit_key = meta.commit.as_str().to_owned();
        let key = (project_key.clone(), commit_key);

        if self.snapshots.contains_key(&key) {
            return Err(StoreError::Conflict(format!(
                "commit {} already exists",
                meta.commit
            )));
        }

        let snapshot = Snapshot::new(meta.clone(), graph);
        self.snapshots.insert(key, snapshot);
        self.latest.insert(project_key.clone(), meta.commit.clone());

        let commits = self.commits.entry(project_key.clone()).or_default();
        commits.push_back(meta);
        if commits.len() > MAX_COMMITS_PER_PROJECT {
            // Evict the oldest commit that is NOT referenced by a baseline
            // (baseline-pinned commits are eviction-exempt, mirroring the
            // session archive's golden pinning). If every commit is pinned,
            // the deque grows past the cap rather than breaking a baseline.
            let victim_pos = self.commits.get(&project_key).and_then(|commits| {
                commits
                    .iter()
                    .position(|meta| !self.is_baselined(&project_key, &meta.commit))
            });
            if let Some(pos) = victim_pos {
                if let Some(victim) = self
                    .commits
                    .get_mut(&project_key)
                    .and_then(|c| c.remove(pos))
                {
                    let old_key = (project_key, victim.commit.as_str().to_owned());
                    self.snapshots.remove(&old_key);
                }
            }
        }

        Ok(())
    }

    fn get_snapshot(
        &self,
        project: &ProjectId,
        commit: &CommitId,
    ) -> Result<Option<Snapshot>, StoreError> {
        let key = (project.as_str().to_owned(), commit.as_str().to_owned());
        Ok(self.snapshots.get(&key).cloned())
    }

    fn latest(&self, project: &ProjectId) -> Result<Option<CommitId>, StoreError> {
        Ok(self.latest.get(project.as_str()).cloned())
    }

    fn list_commits(&self, project: &ProjectId) -> Result<Vec<SnapshotMeta>, StoreError> {
        Ok(self
            .commits
            .get(project.as_str())
            .map(|d| d.iter().rev().cloned().collect())
            .unwrap_or_default())
    }

    fn list_projects(&self) -> Result<Vec<ProjectId>, StoreError> {
        Ok(self
            .commits
            .keys()
            .map(|k| ProjectId::new(k.clone()))
            .collect())
    }

    fn create_baseline(
        &mut self,
        project: &ProjectId,
        name: &str,
        commit: &CommitId,
        provenance: Option<GitProvenance>,
    ) -> Result<(), StoreError> {
        let project_key = project.as_str().to_owned();
        let snapshot_key = (project_key.clone(), commit.as_str().to_owned());
        if !self.snapshots.contains_key(&snapshot_key) {
            return Err(StoreError::CommitNotFound(format!(
                "cannot baseline {commit}: no snapshot stored for it"
            )));
        }
        let baselines = self.baselines.entry(project_key).or_default();
        if baselines.iter().any(|b| b.name == name) {
            return Err(StoreError::Conflict(format!(
                "baseline '{name}' already exists (baselines are immutable — pick a new name)"
            )));
        }
        baselines.push(BaselineMeta {
            name: name.to_owned(),
            commit: commit.clone(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            provenance,
        });
        Ok(())
    }

    fn get_baseline(
        &self,
        project: &ProjectId,
        name: &str,
    ) -> Result<Option<CommitId>, StoreError> {
        Ok(self
            .baselines
            .get(project.as_str())
            .and_then(|b| b.iter().find(|m| m.name == name))
            .map(|m| m.commit.clone()))
    }

    fn list_baselines(&self, project: &ProjectId) -> Result<Vec<BaselineMeta>, StoreError> {
        Ok(self
            .baselines
            .get(project.as_str())
            .map(|b| b.iter().rev().cloned().collect())
            .unwrap_or_default())
    }
}

// PostgreSQL backend (requires `postgres` feature)
#[cfg(feature = "postgres")]
pub mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::PostgresStore;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind};

    fn create_test_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();
        let elem = Element::new_with_kind(ElementKind::Package).with_name("Test");
        graph.add_element(elem);
        graph
    }

    #[test]
    fn in_memory_store_put_get() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("test-project");
        let commit = CommitId::new("v1");
        let graph = create_test_graph();
        let meta = SnapshotMeta::new(commit.clone(), "Initial commit");

        store.put_snapshot(&project, meta, &graph).unwrap();

        let snapshot = store.get_snapshot(&project, &commit).unwrap().unwrap();
        let restored = snapshot.graph().unwrap();

        assert_eq!(graph.element_count(), restored.element_count());
    }

    #[test]
    fn in_memory_store_latest() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("test-project");
        let graph = create_test_graph();

        let meta1 = SnapshotMeta::new(CommitId::new("v1"), "First");
        store.put_snapshot(&project, meta1, &graph).unwrap();

        let meta2 =
            SnapshotMeta::new(CommitId::new("v2"), "Second").with_parent(CommitId::new("v1"));
        store.put_snapshot(&project, meta2, &graph).unwrap();

        let latest = store.latest(&project).unwrap().unwrap();
        assert_eq!(latest.as_str(), "v2");
    }

    #[test]
    fn in_memory_store_list_commits() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("test-project");
        let graph = create_test_graph();

        store
            .put_snapshot(
                &project,
                SnapshotMeta::new(CommitId::new("v1"), "First"),
                &graph,
            )
            .unwrap();
        store
            .put_snapshot(
                &project,
                SnapshotMeta::new(CommitId::new("v2"), "Second"),
                &graph,
            )
            .unwrap();

        let commits = store.list_commits(&project).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].commit.as_str(), "v2"); // Most recent first
    }

    #[test]
    fn in_memory_store_list_projects() {
        let mut store = InMemoryStore::new();
        let graph = create_test_graph();

        store
            .put_snapshot(
                &ProjectId::new("project-a"),
                SnapshotMeta::new(CommitId::new("v1"), "A"),
                &graph,
            )
            .unwrap();
        store
            .put_snapshot(
                &ProjectId::new("project-b"),
                SnapshotMeta::new(CommitId::new("v1"), "B"),
                &graph,
            )
            .unwrap();

        let projects = store.list_projects().unwrap();
        assert_eq!(projects.len(), 2);
    }

    #[test]
    fn in_memory_store_conflict() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("test-project");
        let graph = create_test_graph();

        let meta = SnapshotMeta::new(CommitId::new("v1"), "First");
        store.put_snapshot(&project, meta.clone(), &graph).unwrap();

        let result = store.put_snapshot(&project, meta, &graph);
        assert!(matches!(result, Err(StoreError::Conflict(_))));
    }

    #[test]
    fn baseline_create_resolve_list_and_immutability() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("bl-test");
        let graph = create_test_graph();
        for c in ["c1", "c2"] {
            store
                .put_snapshot(&project, SnapshotMeta::new(CommitId::new(c), c), &graph)
                .unwrap();
        }

        // Baseline to a missing commit fails hard.
        assert!(matches!(
            store.create_baseline(&project, "B1", &CommitId::new("nope"), None),
            Err(StoreError::CommitNotFound(_))
        ));

        store
            .create_baseline(&project, "B1", &CommitId::new("c1"), None)
            .unwrap();
        store
            .create_baseline(&project, "B2", &CommitId::new("c2"), None)
            .unwrap();

        // Immutable: same name can never be re-created (even at same target).
        assert!(matches!(
            store.create_baseline(&project, "B1", &CommitId::new("c2"), None),
            Err(StoreError::Conflict(_))
        ));

        assert_eq!(
            store.get_baseline(&project, "B1").unwrap(),
            Some(CommitId::new("c1"))
        );
        assert_eq!(store.get_baseline(&project, "missing").unwrap(), None);

        // Most recently created first.
        let names: Vec<String> = store
            .list_baselines(&project)
            .unwrap()
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert_eq!(names, vec!["B2".to_string(), "B1".to_string()]);
        // Unknown project → empty, not error.
        assert!(store
            .list_baselines(&ProjectId::new("other"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn baselined_commit_is_eviction_exempt() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("bl-evict");
        let graph = create_test_graph();

        // First commit gets baselined, then we blow past the cap.
        store
            .put_snapshot(&project, SnapshotMeta::new(CommitId::new("c0"), "c0"), &graph)
            .unwrap();
        store
            .create_baseline(&project, "PDR", &CommitId::new("c0"), None)
            .unwrap();
        for i in 1..=super::MAX_COMMITS_PER_PROJECT + 5 {
            let id = format!("c{i}");
            store
                .put_snapshot(&project, SnapshotMeta::new(CommitId::new(&id), &id), &graph)
                .unwrap();
        }

        // The baselined commit survives, with its snapshot resolvable…
        let commits = store.list_commits(&project).unwrap();
        assert!(commits.iter().any(|m| m.commit.as_str() == "c0"));
        assert!(store
            .get_snapshot(&project, &CommitId::new("c0"))
            .unwrap()
            .is_some());
        // …while eviction fell on the oldest non-baselined commits instead.
        assert!(store
            .get_snapshot(&project, &CommitId::new("c1"))
            .unwrap()
            .is_none());
        assert_eq!(commits.len(), super::MAX_COMMITS_PER_PROJECT);
    }

    #[test]
    fn snapshot_meta_with_parent() {
        let meta = SnapshotMeta::new(CommitId::new("v2"), "Second")
            .with_parent(CommitId::new("v1"))
            .with_timestamp(1234567890);

        assert_eq!(meta.parent.unwrap().as_str(), "v1");
        assert_eq!(meta.timestamp, 1234567890);
    }

    #[test]
    fn in_memory_store_commit_cap() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("cap-test");
        let graph = create_test_graph();

        // Insert 105 commits (cap is 100)
        for i in 0..105 {
            let id = format!("c{}", i);
            let meta = SnapshotMeta::new(CommitId::new(id), format!("Commit {}", i));
            store.put_snapshot(&project, meta, &graph).unwrap();
        }

        // Only 100 should remain
        let commits = store.list_commits(&project).unwrap();
        assert_eq!(commits.len(), super::MAX_COMMITS_PER_PROJECT);

        // Oldest 5 (c0..c4) should be evicted
        let commit_ids: Vec<&str> = commits.iter().map(|c| c.commit.as_str()).collect();
        for i in 0..5 {
            let old_id = format!("c{}", i);
            assert!(
                !commit_ids.contains(&old_id.as_str()),
                "commit {} should have been evicted",
                old_id
            );
            // Snapshot data should also be gone
            let snapshot = store
                .get_snapshot(&project, &CommitId::new(old_id))
                .unwrap();
            assert!(snapshot.is_none());
        }

        // Newest commit should still exist
        assert!(commit_ids.contains(&"c104"));
        let snapshot = store
            .get_snapshot(&project, &CommitId::new("c104"))
            .unwrap();
        assert!(snapshot.is_some());
    }
}
