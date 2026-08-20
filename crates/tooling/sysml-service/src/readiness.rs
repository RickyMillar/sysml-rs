//! Per-URI readiness predicate (Phase P-RA1).
//!
//! Any transport (LSP, MCP, REST, CLI) can ask
//! [`SysmlService::readiness_for`](crate::SysmlService::readiness_for)
//! whether a given URI is in a state where particular classes of
//! diagnostics can be honestly asked yet. The four sub-enums describe
//! the three orthogonal dimensions the service tracks plus a
//! human-friendly project-kind tag for diagnostic enrichment.
//!
//!
//! ## Derivation, not state
//!
//! `Readiness` is computed on demand from [`AnalysisHost`] state. It
//! does **not** add new locks, new RwLocks or new Arc-of-state on top
//! of the host. The only loading-progress detail this phase can't
//! observe — `Loading`, `Indexing { done, total }`, `Failed(…)` — is
//! populated by P-RA4's `ProgressBus` integration later. In P-RA1 the
//! service only reports the two steady states (`Unloaded` / `Loaded`,
//! `NotIndexed` / `Indexed`) plus the file's own readiness tier.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sysml_project::discovery::ProjectKind;

use sysml_ide_db::project_inputs::{
    PROJECT_KIND_DISCOVERED, PROJECT_KIND_DISCOVERED_VIA_MANIFEST, PROJECT_KIND_STRICT,
};

/// Re-export of the canonical [`sysml_span::DiagnosticTier`].
///
/// The enum was originally defined here as a placeholder during P-RA1.
/// P-RA2 moved the type to `sysml-span` (co-located with `Diagnostic`,
/// since each `Diagnostic` now carries a `tier` field). P-RA3 reconciles
/// the duplicate by importing the canonical definition and re-exporting
/// it under the same path, so any caller that wrote
/// `sysml_service::readiness::DiagnosticTier` still compiles.
///
/// New code should prefer `sysml_span::DiagnosticTier` directly.
pub use sysml_span::DiagnosticTier;

/// Whether the standard library has been loaded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LibraryReadiness {
    /// `enable_stdlib()` has not been called (or has not yet completed).
    Unloaded,
    /// Library load is in flight — populated by the `ProgressBus`
    /// (P-RA4). P-RA1 never returns this value.
    Loading,
    /// `library_graph()` is `Some` on the host.
    Loaded,
    /// Library load reported a fatal error. Populated by `ProgressBus`
    /// (P-RA4). The string is the user-visible failure cause.
    ///
    /// Carries [`Arc<str>`] because [`Readiness`] is shared across
    /// threads and we want a cheap clone; this is also why
    /// [`Readiness`] is `Clone` but not `Copy`.
    Failed(Arc<str>),
}

/// Whether the project the file belongs to has been indexed
/// (`ProjectFileSet` registered on the host).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProjectReadiness {
    /// No `ProjectFileSet` registered for this file's owning project,
    /// or the file has no owning project yet.
    NotIndexed,
    /// Workspace indexing is in flight. Populated by `ProgressBus`
    /// (P-RA4); P-RA1 never returns this.
    Indexing { done: u32, total: u32 },
    /// `ProjectFileSet` registered.
    Indexed,
}

/// Whether the file itself is in the analysis host yet.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileReadiness {
    /// `host.files().lookup(uri)` returned `None`.
    NotInDb,
    /// File is in the host but lives in no project yet (single-file or
    /// pre-indexing state). Parse-only queries are answerable; anything
    /// asking for cross-file resolution is not.
    ParsedOnly,
    /// File is in the host *and* carries a `project_id`. Workspace-
    /// aware salsa queries can run.
    Resolved,
}

/// A snapshot of "is this question answerable yet" for a single URI.
///
/// Cheap to compute — one [`AnalysisHost`] lock acquisition. Cheap to
/// share — [`Clone`] + [`Send`] + [`Sync`]; not [`Copy`] because the
/// `LibraryReadiness::Failed` variant carries an [`Arc<str>`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Readiness {
    /// Standard-library load state.
    pub library: LibraryReadiness,
    /// Indexing state of the project this file belongs to.
    pub project: ProjectReadiness,
    /// The file's own state.
    pub file: FileReadiness,
    /// How the file's project was opened, when known. Lets clients
    /// surface project-kind-specific UX (e.g., Strict-mode banner) and
    /// is the same value that drives `IM012` strict-mode enrichment in
    /// the diagnostic pipeline. `None` when the file isn't yet in a
    /// known project.
    pub project_kind: Option<ProjectKindWire>,
}

/// Wire-format mirror of [`ProjectKind`] (which is not `Serialize`).
///
/// The variants are the same as [`ProjectKind`]; the explicit
/// `serde(rename_all = "snake_case")` keeps the JSON encoding stable
/// across the `discovery::ProjectKind` API surface. Use [`From`]
/// conversions in either direction.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKindWire {
    /// Single file or synthetic buffer. Stdlib only.
    Strict,
    /// Folder opened, no `sysml.toml` found.
    Discovered,
    /// `sysml.toml` at or above the opened root.
    DiscoveredViaManifest,
}

impl From<ProjectKind> for ProjectKindWire {
    fn from(k: ProjectKind) -> Self {
        match k {
            ProjectKind::Strict => Self::Strict,
            ProjectKind::Discovered => Self::Discovered,
            ProjectKind::DiscoveredViaManifest => Self::DiscoveredViaManifest,
        }
    }
}

impl From<ProjectKindWire> for ProjectKind {
    fn from(k: ProjectKindWire) -> Self {
        match k {
            ProjectKindWire::Strict => Self::Strict,
            ProjectKindWire::Discovered => Self::Discovered,
            ProjectKindWire::DiscoveredViaManifest => Self::DiscoveredViaManifest,
        }
    }
}

/// Decode the `u8` carried on a `ProjectFileSet` salsa input into a
/// [`ProjectKindWire`]. Returns `None` for unknown values (defensive —
/// the constants are an enum on the salsa side, but the field type is
/// `u8`).
pub(crate) fn project_kind_from_u8(v: u8) -> Option<ProjectKindWire> {
    match v {
        PROJECT_KIND_STRICT => Some(ProjectKindWire::Strict),
        PROJECT_KIND_DISCOVERED => Some(ProjectKindWire::Discovered),
        PROJECT_KIND_DISCOVERED_VIA_MANIFEST => Some(ProjectKindWire::DiscoveredViaManifest),
        _ => None,
    }
}

impl Readiness {
    /// Construct an empty readiness — nothing loaded, nothing indexed,
    /// file absent. Useful as a default when the caller has no host
    /// access yet.
    pub fn empty() -> Self {
        Self {
            library: LibraryReadiness::Unloaded,
            project: ProjectReadiness::NotIndexed,
            file: FileReadiness::NotInDb,
            project_kind: None,
        }
    }

    /// Derive a [`Readiness`] for `uri` from an already-locked
    /// [`AnalysisHost`].
    ///
    /// This is the same derivation that powers
    /// [`crate::SysmlService::readiness_for`]; it's exposed here so the
    /// diagnostic pipeline can compute the gate without taking the
    /// service-level lock a second time. P-RA3 uses it inside
    /// `compute_full_diagnostics` to filter every diagnostic by
    /// `Readiness × DiagnosticTier` in one host-lock acquisition.
    pub fn from_host(host: &sysml_ide_db::AnalysisHost, uri: &str) -> Self {
        let library = if host.library_graph().is_some() {
            LibraryReadiness::Loaded
        } else {
            LibraryReadiness::Unloaded
        };

        let file_id = host.files().lookup(uri);
        let (file, project_handle) = match file_id {
            None => (FileReadiness::NotInDb, None),
            Some(fid) => match host.files().project_id(fid) {
                Some(pid) => (FileReadiness::Resolved, Some(pid)),
                None => (FileReadiness::ParsedOnly, None),
            },
        };

        let (project, project_kind) = match project_handle {
            Some(pid) => match host.project_file_set(pid) {
                Some(pfs) => {
                    let kind_u8 = pfs.kind(host.db());
                    (ProjectReadiness::Indexed, project_kind_from_u8(kind_u8))
                }
                None => (ProjectReadiness::NotIndexed, None),
            },
            None => (ProjectReadiness::NotIndexed, None),
        };

        Readiness {
            library,
            project,
            file,
            project_kind,
        }
    }

    /// True iff a diagnostic of the given `tier` can be honestly emitted
    /// in the current readiness state.
    ///
    /// The matrix:
    ///
    /// | Tier               | Required readiness                            |
    /// |--------------------|-----------------------------------------------|
    /// | `Parse`            | always                                        |
    /// | `StructuralLocal`  | `file != NotInDb`                             |
    /// | `NameResLocal`     | `file != NotInDb`                             |
    /// | `NameResLibrary`   | `library = Loaded`                            |
    /// | `NameResWorkspace` | `project = Indexed`                           |
    /// | `ImportHealth`     | `project = Indexed`                           |
    /// | `Semantic`         | `project = Indexed`                           |
    /// | `Constraint`       | `project = Indexed`                           |
    ///
    /// The two transient states `LibraryReadiness::Loading` and
    /// `ProjectReadiness::Indexing` count as "not yet ready" for every
    /// non-`Parse` tier; that's the whole point of the gate.
    pub fn answers(&self, tier: DiagnosticTier) -> bool {
        let file_present = !matches!(self.file, FileReadiness::NotInDb);
        let library_loaded = matches!(self.library, LibraryReadiness::Loaded);
        let project_indexed = matches!(self.project, ProjectReadiness::Indexed);
        match tier {
            DiagnosticTier::Parse => true,
            DiagnosticTier::StructuralLocal | DiagnosticTier::NameResLocal => file_present,
            DiagnosticTier::NameResLibrary => library_loaded,
            DiagnosticTier::NameResWorkspace
            | DiagnosticTier::ImportHealth
            | DiagnosticTier::Semantic
            | DiagnosticTier::Constraint => project_indexed,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::SysmlService;
    use sysml_project::discovery::OpenTarget;
    use tempfile::TempDir;

    fn fixture_workspace(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (rel, content) in files {
            let path = dir.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, content).unwrap();
        }
        dir
    }

    #[test]
    fn readiness_empty_service_reports_unloaded_and_not_indexed() {
        let service = SysmlService::empty();
        let r = service.readiness_for("nonexistent.sysml");
        assert_eq!(r.library, LibraryReadiness::Unloaded);
        assert_eq!(r.project, ProjectReadiness::NotIndexed);
        assert_eq!(r.file, FileReadiness::NotInDb);
        assert_eq!(r.project_kind, None);
    }

    #[test]
    fn readiness_after_enable_stdlib_reports_library_loaded() {
        let service = SysmlService::empty();
        // Stdlib enable is best-effort in tests; if it returns Err we
        // treat the test as a no-op rather than failing on environment.
        let loaded = service
            .host_arc()
            .lock()
            .unwrap()
            .enable_stdlib()
            .unwrap_or(false);
        if !loaded {
            // No embedded stdlib available; nothing to assert.
            return;
        }
        let r = service.readiness_for("nonexistent.sysml");
        assert_eq!(r.library, LibraryReadiness::Loaded);
    }

    #[test]
    fn readiness_after_open_context_folder_reports_indexed_and_resolved() {
        let dir = fixture_workspace(&[(
            "model.sysml",
            "package P { part def A; }",
        )]);
        let service = SysmlService::empty();
        let ctx = service
            .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
            .unwrap();
        let uri = ctx
            .loaded_uris
            .first()
            .expect("at least one file opened")
            .clone();
        let r = service.readiness_for(&uri);
        assert_eq!(
            r.project,
            ProjectReadiness::Indexed,
            "project should be Indexed after open_context"
        );
        assert_eq!(
            r.file,
            FileReadiness::Resolved,
            "file should be Resolved (in db + project_id) after open_context"
        );
        assert!(r.project_kind.is_some(), "project_kind should be known");
    }

    #[test]
    fn readiness_answers_parse_always_workspace_only_when_indexed() {
        // Parse tier: always answerable.
        let empty = Readiness::empty();
        assert!(empty.answers(DiagnosticTier::Parse));
        assert!(!empty.answers(DiagnosticTier::NameResWorkspace));
        assert!(!empty.answers(DiagnosticTier::ImportHealth));
        assert!(!empty.answers(DiagnosticTier::Semantic));
        assert!(!empty.answers(DiagnosticTier::Constraint));

        // NameResLibrary needs library loaded.
        assert!(!empty.answers(DiagnosticTier::NameResLibrary));
        let lib_ready = Readiness {
            library: LibraryReadiness::Loaded,
            ..Readiness::empty()
        };
        assert!(lib_ready.answers(DiagnosticTier::NameResLibrary));
        // Still no workspace.
        assert!(!lib_ready.answers(DiagnosticTier::NameResWorkspace));

        // Workspace tiers unlocked once project is indexed.
        let ws_ready = Readiness {
            project: ProjectReadiness::Indexed,
            ..Readiness::empty()
        };
        assert!(ws_ready.answers(DiagnosticTier::NameResWorkspace));
        assert!(ws_ready.answers(DiagnosticTier::ImportHealth));
        assert!(ws_ready.answers(DiagnosticTier::Semantic));
        assert!(ws_ready.answers(DiagnosticTier::Constraint));

        // Loading / Indexing transient states do *not* answer non-Parse.
        let mid_index = Readiness {
            library: LibraryReadiness::Loading,
            project: ProjectReadiness::Indexing { done: 1, total: 4 },
            ..Readiness::empty()
        };
        assert!(mid_index.answers(DiagnosticTier::Parse));
        assert!(!mid_index.answers(DiagnosticTier::NameResLibrary));
        assert!(!mid_index.answers(DiagnosticTier::NameResWorkspace));

        // Failed library is also "not answerable" for library-dependent tiers.
        let failed = Readiness {
            library: LibraryReadiness::Failed(Arc::from("disk error")),
            ..Readiness::empty()
        };
        assert!(!failed.answers(DiagnosticTier::NameResLibrary));
    }
}
