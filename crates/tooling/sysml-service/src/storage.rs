//! Storage operations for model persistence.
//!
//! Wraps `sysml-store` functionality for storing and retrieving model snapshots.

use sysml_core::ModelGraph;
use sysml_id::{CommitId, ProjectId};
use sysml_store::{SnapshotMeta, Store};

use crate::error::ServiceError;

/// Store a model snapshot.
pub fn store_model(
    store: &mut dyn Store,
    project: &ProjectId,
    meta: SnapshotMeta,
    graph: &ModelGraph,
) -> Result<(), ServiceError> {
    store
        .put_snapshot(project, meta, graph)
        .map_err(ServiceError::from)
}

/// Load a model snapshot, returning the deserialized graph.
pub fn load_model(
    store: &dyn Store,
    project: &ProjectId,
    commit: &CommitId,
) -> Result<Option<ModelGraph>, ServiceError> {
    match store.get_snapshot(project, commit)? {
        Some(snapshot) => {
            let graph = snapshot
                .graph()
                .map_err(|e| ServiceError::Store(e.to_string()))?;
            Ok(Some(graph))
        }
        None => Ok(None),
    }
}

/// Get the latest commit ID for a project.
pub fn latest_commit(
    store: &dyn Store,
    project: &ProjectId,
) -> Result<Option<CommitId>, ServiceError> {
    store.latest(project).map_err(ServiceError::from)
}

/// List all projects in the store.
pub fn list_projects(store: &dyn Store) -> Result<Vec<ProjectId>, ServiceError> {
    store.list_projects().map_err(ServiceError::from)
}

/// List commits for a project (most recent first).
pub fn list_commits(
    store: &dyn Store,
    project: &ProjectId,
) -> Result<Vec<SnapshotMeta>, ServiceError> {
    store.list_commits(project).map_err(ServiceError::from)
}

/// Resolve a commit reference: a baseline name if one exists in the project,
/// otherwise treated as a commit id (which must have a stored snapshot).
///
/// Resolution order is documented on `sysml.store.diff`; in practice commit
/// ids are content hashes/uuids so a collision with a human baseline name
/// does not occur.
pub(crate) fn resolve_ref(
    store: &dyn Store,
    project: &ProjectId,
    reference: &str,
) -> Result<CommitId, ServiceError> {
    if let Some(commit) = store.get_baseline(project, reference)? {
        return Ok(commit);
    }
    let commit = CommitId::new(reference);
    if store.get_snapshot(project, &commit)?.is_none() {
        return Err(ServiceError::Store(format!(
            "'{reference}' is neither a baseline name nor a stored commit in {project}"
        )));
    }
    Ok(commit)
}

/// Resolve and load both sides of a snapshot pair (each ref = baseline
/// name or commit id; `to = None` → the project's latest commit).
fn load_snapshot_pair(
    store: &dyn Store,
    project: &ProjectId,
    from: &str,
    to: Option<&str>,
) -> Result<(ModelGraph, ModelGraph), ServiceError> {
    let from_commit = resolve_ref(store, project, from)?;
    let to_commit = match to {
        Some(r) => resolve_ref(store, project, r)?,
        None => store.latest(project)?.ok_or_else(|| {
            ServiceError::Store(format!("project {project} has no commits to diff against"))
        })?,
    };
    let load = |commit: &CommitId| -> Result<ModelGraph, ServiceError> {
        load_model(store, project, commit)?.ok_or_else(|| {
            ServiceError::Store(format!("snapshot for commit {commit} not found"))
        })
    };
    Ok((load(&from_commit)?, load(&to_commit)?))
}

/// Diff two snapshots (each referenced by baseline name or commit id;
/// `to = None` means the project's latest commit). Optional `element_ids`
/// narrows `modified` to those ids — the `changed_since` composition, per
/// the B3 steward ruling (no new semantics, pure filter).
pub fn diff_snapshots(
    store: &dyn Store,
    project: &ProjectId,
    from: &str,
    to: Option<&str>,
    element_ids: Option<&[sysml_id::ElementId]>,
) -> Result<sysml_core::diff::GraphDiff, ServiceError> {
    let (old, new) = load_snapshot_pair(store, project, from, to)?;
    let mut diff = sysml_core::diff::diff_graphs(&old, &new);
    if let Some(ids) = element_ids {
        let keep: std::collections::HashSet<_> = ids.iter().collect();
        diff.added.retain(|id| keep.contains(id));
        diff.removed.retain(|id| keep.contains(id));
        diff.modified.retain(|m| keep.contains(&m.id));
    }
    Ok(diff)
}

/// Store the given graph under a CONTENT-ADDRESSED commit id
/// (`ModelGraph::content_digest`) — the `sysml.store.save_workspace`
/// primitive. Idempotent: if a snapshot with the same digest already
/// exists, its stored metadata is returned and nothing is written —
/// digest equality is a provable content equivalence (the digest hashes
/// exactly the diff-compared field set), not a soft fallback. Reloading
/// an unchanged workspace therefore mints no new commit and burns no
/// slot of the in-memory store's commit cap.
pub fn save_workspace_snapshot(
    store: &mut dyn Store,
    project: &ProjectId,
    graph: &ModelGraph,
    message: &str,
) -> Result<SnapshotMeta, ServiceError> {
    // Fail hard on an empty graph: with a workspace root present this is
    // a mid-reload window (live-observed: `workspace.refresh` transiently
    // empties `__workspace__`), and silently committing it would record a
    // garbage snapshot whose diff marks every requirement removed.
    if graph.element_count() == 0 {
        return Err(ServiceError::Store(
            "refusing to snapshot an empty workspace graph — the workspace is \
             not loaded (or still loading); load it and retry"
                .to_owned(),
        ));
    }
    let commit = CommitId::new(graph.content_digest());
    if let Some(existing) = store.get_snapshot(project, &commit)? {
        return Ok(existing.meta.clone());
    }
    let mut meta = SnapshotMeta::new(commit, message);
    if let Some(parent) = store.latest(project)? {
        meta = meta.with_parent(parent);
    }
    store.put_snapshot(project, meta.clone(), graph)?;
    Ok(meta)
}

/// Suspect attribution between two stored snapshots (R9): diff the pair,
/// then attribute every change to requirement rows — nearest-requirement
/// owner walk plus transitive downstream `Derive` propagation. One home
/// for the composition (`sysml_query::suspect`); this is the thin
/// storage-side assembly.
pub fn suspect_requirements(
    store: &dyn Store,
    project: &ProjectId,
    from: &str,
    to: Option<&str>,
) -> Result<Vec<sysml_query::suspect::SuspectRecord>, ServiceError> {
    let (old, new) = load_snapshot_pair(store, project, from, to)?;
    let diff = sysml_core::diff::diff_graphs(&old, &new);
    Ok(sysml_query::suspect::attribute_diff_to_requirements(
        &old, &new, &diff,
    ))
}

/// Create a named, immutable baseline (`commit = None` → the latest commit).
pub fn create_baseline(
    store: &mut dyn Store,
    project: &ProjectId,
    name: &str,
    commit: Option<&CommitId>,
    provenance: Option<sysml_store::GitProvenance>,
) -> Result<sysml_store::BaselineMeta, ServiceError> {
    let commit = match commit {
        Some(c) => c.clone(),
        None => store.latest(project)?.ok_or_else(|| {
            ServiceError::Store(format!("project {project} has no commits to baseline"))
        })?,
    };
    store.create_baseline(project, name, &commit, provenance)?;
    // Read back the created record so the caller gets the stamped metadata.
    store
        .list_baselines(project)?
        .into_iter()
        .find(|b| b.name == name)
        .ok_or_else(|| {
            ServiceError::Store("baseline vanished immediately after creation".to_owned())
        })
}

/// List baselines for a project (most recently created first).
pub fn list_baselines(
    store: &dyn Store,
    project: &ProjectId,
) -> Result<Vec<sysml_store::BaselineMeta>, ServiceError> {
    store.list_baselines(project).map_err(ServiceError::from)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind};
    use sysml_store::InMemoryStore;

    fn test_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
        graph.add_element(pkg);
        graph
    }

    #[test]
    fn test_store_and_load() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("test-project");
        let commit = CommitId::new("v1");
        let graph = test_graph();
        let meta = SnapshotMeta::new(commit.clone(), "Initial commit");

        store_model(&mut store, &project, meta, &graph).unwrap();

        let loaded = load_model(&store, &project, &commit).unwrap();
        assert!(loaded.is_some());
        let loaded_graph = loaded.unwrap();
        assert_eq!(graph.element_count(), loaded_graph.element_count());
    }

    #[test]
    fn test_load_nonexistent() {
        let store = InMemoryStore::new();
        let project = ProjectId::new("missing");
        let commit = CommitId::new("v1");

        let loaded = load_model(&store, &project, &commit).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_latest_commit() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("test-project");
        let graph = test_graph();

        store_model(
            &mut store,
            &project,
            SnapshotMeta::new(CommitId::new("v1"), "First"),
            &graph,
        )
        .unwrap();
        store_model(
            &mut store,
            &project,
            SnapshotMeta::new(CommitId::new("v2"), "Second"),
            &graph,
        )
        .unwrap();

        let latest = latest_commit(&store, &project).unwrap().unwrap();
        assert_eq!(latest.as_str(), "v2");
    }

    #[test]
    fn test_list_projects() {
        let mut store = InMemoryStore::new();
        let graph = test_graph();

        store_model(
            &mut store,
            &ProjectId::new("project-a"),
            SnapshotMeta::new(CommitId::new("v1"), "A"),
            &graph,
        )
        .unwrap();
        store_model(
            &mut store,
            &ProjectId::new("project-b"),
            SnapshotMeta::new(CommitId::new("v1"), "B"),
            &graph,
        )
        .unwrap();

        let projects = list_projects(&store).unwrap();
        assert_eq!(projects.len(), 2);
    }

    #[test]
    fn diff_and_baselines_end_to_end() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("diff-proj");

        // v1: a requirement's doc element with original text.
        let mut g1 = ModelGraph::new();
        let mut doc = Element::new_with_kind(ElementKind::Documentation).with_name("d");
        let doc_id = doc.id.clone();
        doc.set_prop("body", "trip within 40 ms");
        g1.add_element(doc);
        store_model(
            &mut store,
            &project,
            SnapshotMeta::new(CommitId::new("v1"), "First"),
            &g1,
        )
        .unwrap();

        // Baseline it (defaulting to latest).
        let meta = create_baseline(&mut store, &project, "PDR", None, None).unwrap();
        assert_eq!(meta.commit.as_str(), "v1");

        // v2: text edited.
        let mut g2 = g1.clone();
        if let Some(el) = g2.elements.get_mut(&doc_id) {
            el.set_prop("body", "trip within 25 ms");
        }
        store_model(
            &mut store,
            &project,
            SnapshotMeta::new(CommitId::new("v2"), "Edit"),
            &g2,
        )
        .unwrap();

        // Diff from the BASELINE NAME to latest.
        let diff = diff_snapshots(&store, &project, "PDR", None, None).unwrap();
        assert!(diff.added.is_empty() && diff.removed.is_empty());
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.modified[0].id, doc_id);

        // element_ids filter: unrelated id → empty result.
        let other = sysml_id::ElementId::from_string("unrelated");
        let filtered =
            diff_snapshots(&store, &project, "PDR", None, Some(&[other])).unwrap();
        assert!(filtered.is_empty());

        // Unknown ref fails loud.
        assert!(diff_snapshots(&store, &project, "nope", None, None).is_err());

        assert_eq!(list_baselines(&store, &project).unwrap().len(), 1);
    }

    /// Empty graph → hard error, never a garbage commit (mid-reload guard).
    #[test]
    fn save_workspace_snapshot_rejects_empty_graph() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("empty-proj");
        let err = save_workspace_snapshot(&mut store, &project, &ModelGraph::new(), "x")
            .unwrap_err();
        assert!(err.to_string().contains("empty workspace graph"));
        assert!(list_commits(&store, &project).unwrap().is_empty());
    }

    /// save_workspace_snapshot: content-addressed, idempotent, parented.
    #[test]
    fn save_workspace_snapshot_is_idempotent_and_content_addressed() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("ws-proj");
        let graph = test_graph();

        let first = save_workspace_snapshot(&mut store, &project, &graph, "first").unwrap();
        assert_eq!(first.commit.as_str(), graph.content_digest());
        assert!(first.parent.is_none());

        // Unchanged content → the SAME commit back, nothing new minted.
        let again = save_workspace_snapshot(&mut store, &project, &graph, "again").unwrap();
        assert_eq!(again.commit, first.commit);
        assert_eq!(again.message, "first");
        assert_eq!(list_commits(&store, &project).unwrap().len(), 1);

        // Changed content → a new commit, parented on the previous one.
        let mut edited = graph.clone();
        let extra = Element::new_with_kind(ElementKind::PartUsage).with_name("extra");
        edited.add_element(extra);
        let second = save_workspace_snapshot(&mut store, &project, &edited, "edit").unwrap();
        assert_ne!(second.commit, first.commit);
        assert_eq!(second.parent.as_ref(), Some(&first.commit));
        assert_eq!(list_commits(&store, &project).unwrap().len(), 2);
    }

    /// suspect_requirements end-to-end: doc edit on a requirement's child
    /// between a baselined snapshot and latest → TextChanged on the row.
    #[test]
    fn suspect_requirements_attributes_doc_edit_to_requirement() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("suspect-proj");

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R1");
        let mut doc = Element::new_with_kind(ElementKind::Documentation);
        doc.owner = Some(req.id.clone());
        doc.set_prop("body", "trip within 40 ms");
        let doc_id = doc.id.clone();

        let mut g1 = ModelGraph::new();
        g1.add_element(req.clone());
        g1.add_element(doc);
        save_workspace_snapshot(&mut store, &project, &g1, "v1").unwrap();
        create_baseline(&mut store, &project, "PDR", None, None).unwrap();

        let mut g2 = g1.clone();
        if let Some(el) = g2.elements.get_mut(&doc_id) {
            el.set_prop("body", "trip within 25 ms");
        }
        save_workspace_snapshot(&mut store, &project, &g2, "v2").unwrap();

        let records = suspect_requirements(&store, &project, "PDR", None).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].requirement, req.id);
        assert_eq!(
            records[0].causes,
            vec![sysml_query::suspect::SuspectCause::TextChanged {
                element: doc_id,
                from: "trip within 40 ms".to_owned(),
                to: "trip within 25 ms".to_owned(),
            }]
        );

        // Baseline against itself: nothing suspect.
        let clean = suspect_requirements(&store, &project, "PDR", Some("PDR")).unwrap();
        assert!(clean.is_empty());
    }

    #[test]
    fn test_list_commits() {
        let mut store = InMemoryStore::new();
        let project = ProjectId::new("test-project");
        let graph = test_graph();

        store_model(
            &mut store,
            &project,
            SnapshotMeta::new(CommitId::new("v1"), "First"),
            &graph,
        )
        .unwrap();
        store_model(
            &mut store,
            &project,
            SnapshotMeta::new(CommitId::new("v2"), "Second"),
            &graph,
        )
        .unwrap();

        let commits = list_commits(&store, &project).unwrap();
        assert_eq!(commits.len(), 2);
        // Most recent first
        assert_eq!(commits[0].commit.as_str(), "v2");
    }
}
