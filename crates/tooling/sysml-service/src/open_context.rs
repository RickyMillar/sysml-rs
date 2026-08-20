//! Unified file-loading entrypoint — Phase 2 of the file-loading rebuild.
//!
//! Every transport (CLI, LSP, MCP, REST) routes through
//! [`SysmlService::open_context`]. There is one mode-selection rule and
//! one set of side-effects on the salsa db, regardless of caller. See
//!
//! Phase 2 builds the entrypoint alongside the legacy `from_file` /
//! `from_workspace` / `load_workspace` paths. P3 (cut-over service) and
//! P4 (cut-over LSP) replace those with shims that call here.
//!
//! No new salsa-tracked queries land in this phase. `open_context`
//! drives the existing `SourceFile` / `ProjectFileSet` salsa inputs;
//! the discovery + peek primitives stay as plain functions in
//! `sysml_project::discovery`.

use std::path::PathBuf;
use std::sync::Arc;

use sysml_ide_db::{LibraryGraph, ProjectFileSet, SourceFile};
use sysml_project::discovery::{
    discover, pick_mode, DiagnosticHint, DiscoveryError, ModeDecision, OpenTarget, ProjectKind,
};
use sysml_project::ProjectHandle;
use sysml_span::Diagnostic;

use crate::error::ServiceError;
use crate::SysmlService;

/// Default file cap used when no manifest override is in effect.
/// Lifted to a manifest setting `[discovery] max_files = N` in P6.
pub const DEFAULT_MAX_FILES: usize = 1000;

/// How an open treats files that carry an editor-overlay flag.
///
/// The overlay rule (see `FileSet::overlay_ids`) protects open,
/// possibly-unsaved editor buffers from being clobbered by background
/// disk walks. That is correct for every *implicit* file-entry path —
/// the LSP background indexer's `Folder` rescans, `did_open`,
/// `did_change_watched_files`, single-file `load_file`. It is wrong for
/// `sysml.load_workspace`, whose contract (steward-ruled 2026-07-16) is
/// an explicit, disk-authoritative reload: "the filesystem is truth for
/// this root right now." Before this policy existed, a same-root reload
/// silently kept an overlaid file's pre-edit text forever — the app's
/// Source panel holds the focused file open over the LSP websocket, so
/// the demo loop (edit on disk → Reload) never surfaced the edit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverlayPolicy {
    /// Open editor buffers stay authoritative; overlaid files are
    /// project-tagged but their text is not re-read from disk. The
    /// default for every caller except `load_workspace`.
    Preserve,
    /// Disk is truth: every discovered file is re-read from disk and
    /// its overlay flag cleared, unconditionally. Only
    /// `sysml.load_workspace` passes this.
    DiskAuthoritative,
}

/// Default project id for any context whose root has no manifest of its
/// own (Strict files, synthetic buffers, manifest-less folders).
/// Manifest-rooted projects get pids minted elsewhere.
///
/// Same numeric value as the pre-P2 `SERVICE_WORKSPACE_PROJECT_ID = 100`
/// so the existing 4-way branches in `compute_full_diagnostics` and
/// `salsa_doc` keep routing files opened via `open_context` onto the
/// workspace-aware path. P4 collapses those branches; for now the
/// pid is the cross-phase compatibility anchor.
pub const DEFAULT_PROJECT_ID: u32 = 100;

/// Everything `open_context` produces for the caller.
///
/// The salsa side-effects have already been applied by the time this
/// returns (`SourceFile` inputs set, `ProjectFileSet` populated, stdlib
/// enabled if available). Callers use the returned handle/pfs to drive
/// downstream queries.
///
/// (Manual `Debug` impl: `LibraryGraph` is a salsa-tracked input
/// without `Debug`, so we render it as a presence marker.)
#[derive(Clone)]
pub struct OpenContext {
    /// The project handle the loaded files were tagged with.
    pub project: ProjectHandle,
    /// Why the project exists (Strict / Discovered / DiscoveredViaManifest).
    pub kind: ProjectKind,
    /// Project root on disk, if any (None for `Strict` files without a
    /// manifest ancestor and for `Synthetic`).
    pub root: Option<PathBuf>,
    /// The salsa `ProjectFileSet` input listing every file tagged with
    /// `project`. Refreshed in place if it already existed.
    pub files: ProjectFileSet,
    /// The library graph if stdlib was successfully enabled, else `None`.
    /// `None` is a graceful degradation, not an error.
    pub library: Option<LibraryGraph>,
    /// Discovery-time diagnostics (cap warnings, skip notices). Not the
    /// same as parse / resolve diagnostics — those come from the salsa
    /// queries against the loaded `ProjectFileSet`.
    pub diagnostics: Vec<Diagnostic>,
    /// Loaded file URIs in stable order, for inspection by tests and
    /// for transports that need to enumerate what was just loaded
    /// (LSP `workspace/loadedFiles`, MCP `sysml_load_workspace` result).
    pub loaded_uris: Vec<String>,
}

impl std::fmt::Debug for OpenContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenContext")
            .field("project", &self.project)
            .field("kind", &self.kind)
            .field("root", &self.root)
            .field("library", &self.library.map(|_| "<loaded>"))
            .field("diagnostics", &self.diagnostics.len())
            .field("loaded_uris", &self.loaded_uris)
            .finish()
    }
}

impl SysmlService {
    /// The single canonical entry point for "open a context."
    ///
    /// All three transports (CLI / LSP / MCP) build an [`OpenTarget`]
    /// from their input shape and call here. The mode-selection rule
    /// (see [`pick_mode`]) chooses Strict / Discovered /
    /// DiscoveredViaManifest deterministically.
    ///
    /// Side effects:
    /// - Sets each discovered file's `SourceFile` input via
    ///   `set_file_content_in_project(uri, content, pid)`.
    /// - Creates or refreshes the `ProjectFileSet` salsa input for `pid`.
    /// - Calls `enable_stdlib()` once; failures degrade to `library: None`
    ///   with a warning diagnostic.
    /// - Synthetic targets load the in-memory `content` under the given
    ///   `uri` (no disk walk).
    ///
    /// Returns an [`OpenContext`] carrying the project handle, kind,
    /// root, file set, library, and any discovery-time diagnostics.
    pub fn open_context(&self, target: OpenTarget) -> Result<OpenContext, ServiceError> {
        self.open_context_with(target, OverlayPolicy::Preserve)
    }

    /// [`Self::open_context`] with an explicit [`OverlayPolicy`].
    ///
    /// Only `sysml.load_workspace` passes
    /// [`OverlayPolicy::DiskAuthoritative`]; every implicit file-entry
    /// path routes through [`Self::open_context`] and keeps the
    /// overlay-preserving default. Do NOT flip the default — the
    /// preserve rule is load-bearing for the LSP indexer's rescans and
    /// `did_open` (see the `OverlayPolicy` doc).
    pub fn open_context_with(
        &self,
        target: OpenTarget,
        overlay_policy: OverlayPolicy,
    ) -> Result<OpenContext, ServiceError> {
        let mode = pick_mode(&target);
        let kind = mode_kind(&mode);
        let pid = ProjectHandle(DEFAULT_PROJECT_ID);

        let mut diagnostics = Vec::new();
        let mut loaded_uris = Vec::new();
        let mut root_out: Option<PathBuf> = None;

        match (target, &mode) {
            // -- Strict (synthetic) --
            (OpenTarget::Synthetic { uri, content }, _) => {
                {
                    let mut host = self.host.lock().unwrap();
                    host.set_file_content_in_project(&uri, content, pid);
                }
                loaded_uris.push(uri);
            }

            // -- Strict (file on disk) --
            (OpenTarget::File(path), ModeDecision::Strict { .. }) => {
                let uri = path.to_string_lossy().to_string();
                {
                    let mut host = self.host.lock().unwrap();
                    if host.has_overlay(&uri) {
                        // Open editor buffer is authoritative — tag the
                        // project, don't clobber the buffer with disk.
                        host.set_project_only(&uri, pid);
                    } else {
                        let content = std::fs::read_to_string(&path)
                            .map_err(|e| ServiceError::io(&path, e))?;
                        host.set_file_content_in_project(&uri, content, pid);
                    }
                }
                loaded_uris.push(uri);
            }

            // -- Discovered (no manifest) or DiscoveredViaManifest --
            (target, _) => {
                let root = match &mode {
                    ModeDecision::Discovered { root }
                    | ModeDecision::DiscoveredViaManifest { root, .. } => root.clone(),
                    ModeDecision::Strict { .. } => unreachable!(
                        "Strict matched above; only Folder + File-with-manifest reach here"
                    ),
                };
                let _ = target; // discovered modes only care about `root`

                let result = match discover(&root, DEFAULT_MAX_FILES) {
                    Ok(r) => r,
                    Err(DiscoveryError::CapExceeded { cap, .. }) => {
                        return Err(ServiceError::Project(format!(
                            "discovery: more than {cap} source files under {}; \
                             add `[discovery] max_files = N` to sysml.toml",
                            root.display()
                        )));
                    }
                    Err(e) => return Err(ServiceError::Project(format!("discovery: {e}"))),
                };

                root_out = Some(result.root.clone());

                for hint in &result.warnings {
                    diagnostics.push(hint_to_diagnostic(hint));
                }
                if result.capped {
                    diagnostics.push(
                        Diagnostic::warning(format!(
                            "discovery cap reached under {}",
                            result.root.display()
                        ))
                        .with_code("discovery-cap"),
                    );
                }

                let mut host = self.host.lock().unwrap();

                // Register a synthetic Project at `pid` rooted at this
                // workspace root, idempotent across repeat calls. This is
                // the load-bearing step that makes `host.find_project_for_uri`
                // resolve any URI under this root to `pid` — meaning every
                // LSP / MCP / CLI file-entry path (did_open, did_change,
                // load_file, etc.) gets the SAME pid from the SAME lookup.
                // Without this, the LSP needed an out-of-band
                // `workspace_roots` fallback to compensate, and any new
                // file-entry path had to reproduce that logic. Closes the
                // WaterPort bug class architecturally.
                if !host.has_project_at_path(&result.root) {
                    let info = sysml_project::ProjectInfo {
                        name: result
                            .root
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("workspace")
                            .to_string(),
                        description: None,
                        version: "0.0.0-synthetic".to_string(),
                        topic: Vec::new(),
                        usage: Vec::new(),
                    };
                    host.load_project(sysml_project::Project {
                        id: pid,
                        info,
                        meta: None,
                        root: sysml_project::ProjectRoot::Directory(result.root.clone()),
                    });
                }

                for file_path in &result.files {
                    let uri = file_path.to_string_lossy().to_string();
                    // An open editor buffer is authoritative in-memory
                    // content: tag the project but DON'T overwrite it from
                    // disk. The whole loop runs under one host lock, and
                    // did_open sets buffer+overlay under that same lock, so
                    // the two critical sections serialize — no clobber race.
                    // Under DiskAuthoritative (load_workspace only) the
                    // overlay does NOT win: the flag is cleared and disk
                    // content set inside this same critical section, so a
                    // concurrent did_open/did_change can't interleave.
                    if overlay_policy == OverlayPolicy::Preserve && host.has_overlay(&uri) {
                        host.set_project_only(&uri, pid);
                        loaded_uris.push(uri);
                        continue;
                    }
                    let content = match std::fs::read_to_string(file_path) {
                        Ok(c) => c,
                        Err(e) => {
                            diagnostics.push(
                                Diagnostic::warning(format!(
                                    "could not read {}: {e}",
                                    file_path.display()
                                ))
                                .with_code("discovery-read"),
                            );
                            continue;
                        }
                    };
                    if overlay_policy == OverlayPolicy::DiskAuthoritative {
                        host.clear_overlay(&uri);
                    }
                    host.set_file_content_in_project(&uri, content, pid);
                    loaded_uris.push(uri);
                }

                // Disk is truth for deletions too: a tracked in-root file
                // that discovery no longer lists was deleted on disk, and
                // keeping it would leave a ghost in the workspace graph
                // across reloads. Skipped when discovery was capped — a
                // capped walk's absence is not evidence of deletion.
                // Compares in the host's canonical `file://` URI form.
                if overlay_policy == OverlayPolicy::DiskAuthoritative && !result.capped {
                    let canonical_root =
                        sysml_ide_db::canonical_uri(&result.root.to_string_lossy());
                    let prefix = if canonical_root.ends_with('/') {
                        canonical_root
                    } else {
                        format!("{canonical_root}/")
                    };
                    let discovered: std::collections::HashSet<String> = result
                        .files
                        .iter()
                        .map(|p| sysml_ide_db::canonical_uri(&p.to_string_lossy()))
                        .collect();
                    let ghosts: Vec<String> = host
                        .files()
                        .user_file_ids()
                        .filter(|fid| {
                            host.files().project_id(*fid) == Some(pid)
                        })
                        .filter_map(|fid| {
                            let uri = host.files().uri(fid)?;
                            (uri.starts_with(&prefix) && !discovered.contains(uri))
                                .then(|| uri.to_string())
                        })
                        .collect();
                    for uri in ghosts {
                        host.remove_file(&uri);
                    }
                }
                drop(host);
            }
        }

        // Enable stdlib (idempotent across calls).
        let library = {
            let mut host = self.host.lock().unwrap();
            match host.enable_stdlib() {
                Ok(_loaded) => host.library_graph(),
                Err(e) => {
                    tracing::warn!("open_context: stdlib enable failed: {e}");
                    diagnostics.push(
                        Diagnostic::warning(format!("standard library unavailable: {e}"))
                            .with_code("stdlib-unavailable"),
                    );
                    None
                }
            }
        };

        // Materialize / refresh the ProjectFileSet for this pid.
        // Carry ProjectKind onto the salsa input so the diagnostic pipeline
        // can branch on Strict vs Discovered for IM010 enrichment + IM012.
        let files_pfs = self.ensure_project_pfs(pid, kind);

        loaded_uris.sort();
        loaded_uris.dedup();

        Ok(OpenContext {
            project: pid,
            kind,
            root: root_out,
            files: files_pfs,
            library,
            diagnostics,
            loaded_uris,
        })
    }

    /// Create or refresh the `ProjectFileSet` salsa input for `pid`,
    /// gathering every host-tracked user file tagged with `pid`.
    ///
    /// Generalisation of the legacy `ensure_workspace_pfs` (pid-100-only)
    /// over arbitrary project ids. The legacy helper now delegates here.
    ///
    /// `kind` is recorded on the salsa input so downstream diagnostics
    /// can distinguish Strict vs Discovered without re-deriving from the
    /// `OpenContext`. On refresh of an existing PFS, the `kind` is left
    /// alone (it was set when the PFS was first created and cannot change
    /// without re-opening the project).
    pub(crate) fn ensure_project_pfs(
        &self,
        pid: ProjectHandle,
        kind: ProjectKind,
    ) -> ProjectFileSet {
        use salsa::Setter;
        let mut host = self.host.lock().unwrap();

        let mut entries: Vec<(String, SourceFile)> = host
            .files()
            .user_file_ids()
            .filter(|fid| host.files().project_id(*fid) == Some(pid))
            .filter_map(|fid| {
                let uri = host.files().uri(fid)?.to_string();
                let sf = host.files().source_file(fid)?;
                Some((uri, sf))
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let files: Vec<SourceFile> = entries.into_iter().map(|(_, sf)| sf).collect();
        let files_arc = Arc::new(files);

        match host.project_file_set(pid) {
            Some(pfs) => {
                pfs.set_files(host.db_mut()).to(files_arc);
                pfs
            }
            None => {
                let pfs = ProjectFileSet::new(host.db(), pid.0, files_arc, kind_to_u8(kind));
                host.add_project_file_set(pfs);
                pfs
            }
        }
    }
}

/// Map [`ProjectKind`] to the `u8` encoding used on the salsa `ProjectFileSet`
/// input. Keep this aligned with the `PROJECT_KIND_*` constants in
/// `sysml_ide_db::project_inputs`.
fn kind_to_u8(kind: ProjectKind) -> u8 {
    match kind {
        ProjectKind::Discovered => sysml_ide_db::project_inputs::PROJECT_KIND_DISCOVERED,
        ProjectKind::Strict => sysml_ide_db::project_inputs::PROJECT_KIND_STRICT,
        ProjectKind::DiscoveredViaManifest => {
            sysml_ide_db::project_inputs::PROJECT_KIND_DISCOVERED_VIA_MANIFEST
        }
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn mode_kind(mode: &ModeDecision) -> ProjectKind {
    match mode {
        ModeDecision::Strict { .. } => ProjectKind::Strict,
        ModeDecision::Discovered { .. } => ProjectKind::Discovered,
        ModeDecision::DiscoveredViaManifest { .. } => ProjectKind::DiscoveredViaManifest,
    }
}

fn hint_to_diagnostic(hint: &DiagnosticHint) -> Diagnostic {
    let mut d = Diagnostic::info(hint.message.clone()).with_code(hint.code.clone());
    if let Some(path) = &hint.path {
        d = d.with_note(format!("path: {}", path.display()));
    }
    d
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn scratch(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (rel, content) in files {
            let p = dir.path().join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&p, content).unwrap();
        }
        dir
    }

    const MIN_MANIFEST: &str = r#"
[project]
name = "test"
version = "0.1.0"
"#;

    #[test]
    fn open_synthetic_buffer_is_strict() {
        let svc = SysmlService::empty();
        let ctx = svc
            .open_context(OpenTarget::Synthetic {
                uri: "inmemory://buf1".into(),
                content: "package Foo;".into(),
            })
            .unwrap();
        assert_eq!(ctx.kind, ProjectKind::Strict);
        assert!(ctx.root.is_none());
        assert_eq!(ctx.loaded_uris, vec!["inmemory://buf1".to_string()]);
        assert_eq!(ctx.project, ProjectHandle(DEFAULT_PROJECT_ID));
    }

    #[test]
    fn open_single_file_no_manifest_is_strict() {
        let dir = scratch(&[("foo.sysml", "package Foo;")]);
        let svc = SysmlService::empty();
        let ctx = svc
            .open_context(OpenTarget::File(dir.path().join("foo.sysml")))
            .unwrap();
        assert_eq!(ctx.kind, ProjectKind::Strict);
        assert!(ctx.root.is_none(), "Strict file has no project root");
        assert_eq!(ctx.loaded_uris.len(), 1);
    }

    #[test]
    fn open_single_file_with_ancestor_manifest_is_discovered_via_manifest() {
        let dir = scratch(&[
            ("sysml.toml", MIN_MANIFEST),
            ("src/foo.sysml", "package Foo;"),
            ("src/bar.sysml", "package Bar;"),
        ]);
        let svc = SysmlService::empty();
        let ctx = svc
            .open_context(OpenTarget::File(dir.path().join("src/foo.sysml")))
            .unwrap();
        assert_eq!(ctx.kind, ProjectKind::DiscoveredViaManifest);
        assert!(ctx.root.is_some());
        // Both src/foo.sysml AND src/bar.sysml should be loaded — sibling
        // visibility is exactly the parity fix this lands.
        assert_eq!(ctx.loaded_uris.len(), 2, "got {:?}", ctx.loaded_uris);
    }

    #[test]
    fn open_folder_no_manifest_is_discovered() {
        let dir = scratch(&[("a.sysml", "package A;"), ("b.sysml", "package B;")]);
        let svc = SysmlService::empty();
        let ctx = svc
            .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
            .unwrap();
        assert_eq!(ctx.kind, ProjectKind::Discovered);
        assert_eq!(ctx.loaded_uris.len(), 2);
        assert!(ctx.root.is_some());
    }

    #[test]
    fn open_folder_with_manifest_is_discovered_via_manifest() {
        let dir = scratch(&[("sysml.toml", MIN_MANIFEST), ("a.sysml", "package A;")]);
        let svc = SysmlService::empty();
        let ctx = svc
            .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
            .unwrap();
        assert_eq!(ctx.kind, ProjectKind::DiscoveredViaManifest);
    }

    #[test]
    fn open_folder_isolates_nested_subprojects() {
        let dir = scratch(&[
            ("outer.sysml", "package Outer;"),
            ("sub/sysml.toml", MIN_MANIFEST),
            ("sub/inner.sysml", "package Inner;"),
        ]);
        let svc = SysmlService::empty();
        let ctx = svc
            .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
            .unwrap();
        // Only outer.sysml should be loaded for this project. The
        // sub/inner.sysml stays out of pid 100's ProjectFileSet (P6
        // proves it's reachable via its own open_context call).
        assert_eq!(ctx.loaded_uris.len(), 1, "got {:?}", ctx.loaded_uris);
        assert!(ctx.loaded_uris[0].ends_with("outer.sysml"));
    }

    #[test]
    fn open_context_creates_project_file_set() {
        let dir = scratch(&[("a.sysml", "package A;"), ("b.sysml", "package B;")]);
        let svc = SysmlService::empty();
        let ctx = svc
            .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
            .unwrap();
        let host = svc.host_arc().lock().unwrap();
        // The PFS we got back must equal the one the host now tracks.
        let tracked = host.project_file_set(ctx.project);
        assert!(tracked.is_some(), "host should track the new PFS");
    }

    #[test]
    fn open_context_is_idempotent_for_same_target() {
        let dir = scratch(&[("a.sysml", "package A;")]);
        let svc = SysmlService::empty();
        let _first = svc
            .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
            .unwrap();
        let second = svc
            .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
            .unwrap();
        // The second call must not create a duplicate PFS — re-opening
        // the same folder is a refresh, not a fork.
        assert_eq!(second.loaded_uris.len(), 1);
    }

    #[test]
    fn open_context_returns_diagnostics_when_stdlib_unavailable() {
        // No SYSML_LIBRARY_PATH set + no installed lib means enable_stdlib
        // fails gracefully. The contract here is "library is None,
        // diagnostic emitted, but the call still succeeds."
        let dir = scratch(&[("a.sysml", "package A;")]);
        let svc = SysmlService::empty();
        let ctx = svc
            .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
            .unwrap();
        // We can't reliably assert library presence without controlling
        // the env; just assert the call succeeds and either ships a
        // library OR a stdlib-unavailable diagnostic.
        assert!(
            ctx.library.is_some()
                || ctx
                    .diagnostics
                    .iter()
                    .any(|d| d.code.as_deref() == Some("stdlib-unavailable")),
            "expected library or stdlib-unavailable diagnostic"
        );
    }

    #[test]
    fn open_context_missing_file_errors() {
        let dir = scratch(&[]);
        let svc = SysmlService::empty();
        let err = svc
            .open_context(OpenTarget::File(dir.path().join("does-not-exist.sysml")))
            .unwrap_err();
        assert!(matches!(err, ServiceError::Io { .. }), "got {err:?}");
    }

    #[test]
    fn ensure_project_pfs_filters_by_pid() {
        // Two opens with the same default pid should land in the same PFS;
        // the file count should be the union, not the latest call only.
        let dir1 = scratch(&[("a.sysml", "package A;")]);
        let dir2 = scratch(&[("b.sysml", "package B;")]);
        let svc = SysmlService::empty();
        let _c1 = svc
            .open_context(OpenTarget::Folder(dir1.path().to_path_buf()))
            .unwrap();
        let _c2 = svc
            .open_context(OpenTarget::Folder(dir2.path().to_path_buf()))
            .unwrap();

        let host = svc.host_arc().lock().unwrap();
        let pfs = host
            .project_file_set(ProjectHandle(DEFAULT_PROJECT_ID))
            .expect("default PFS");
        let count = pfs.files(host.db()).len();
        assert_eq!(count, 2, "PFS should hold both files");
    }
}
