//! Source input types for the salsa database.
//!
//! These are the "leaves" of the computation graph — values that are set
//! directly by the host (LSP server) rather than computed by queries.

use rustc_hash::FxHashMap;
use salsa::Setter;
use sysml_project::ProjectHandle;

/// Normalise a file URI to its canonical key form so that the same
/// physical file always maps to the same `FileId`, regardless of which
/// transport registered it. The LSP sends `file://...` URIs; the
/// service-layer workspace loader uses raw absolute paths. Without
/// canonicalisation, the two forms create independent `FileId`s and the
/// salsa `ProjectFileSet` built from one is invisible to queries keyed
/// on the other — the bug behind the WaterPort cross-file resolution
/// failure.
///
/// We canonicalise TO `file://...` form (not away from it) for two
/// reasons. (1) Whatever the LSP server hands to `client.publish_diagnostics`
/// must be a parseable URL — keeping the canonical key in URL form means
/// every callsite that emits the URI back to a transport has a guaranteed
/// `Url::parse` round-trip. The mirror choice (strip `file://`) silently
/// broke the LSP's workspace re-publish because `Url::parse("/path")`
/// rejects unprefixed paths and the publish step skipped that branch
/// without logging — VS Code stayed stuck on the pre-workspace diagnostic.
/// (2) Round-tripping through `Url::from_file_path` (rather than concat)
/// uniformly percent-encodes path segments (`%20` for spaces etc.) so
/// the same physical file always produces byte-identical keys even when
/// callers spell the URI differently.
///
/// Rules:
///   - `file://...` → pass through (already canonical; the LSP-side
///     normalisation in `lsp-server::canonical_file_uri` handles any
///     percent-encoding fixups before reaching us).
///   - absolute filesystem path (`/...`) → `Url::from_file_path` if it
///     succeeds, otherwise pass through unchanged.
///   - everything else (synthetic schemes `untitled:`, `inmemory://`,
///     relative paths) → pass through unchanged.
///
/// Dot-segments (`..`) and symlinks ARE resolved (via `path.canonicalize()`)
/// so a non-canonical spelling of a real file
/// (`file://<root>/model/../model/root.sysml`) maps to the SAME `FileId` as
/// its canonical form. When the path is not on disk (unsaved / synthetic /
/// not-yet-written), we fall back to a spelling-preserving URL round-trip.
/// Unifying the FileId this way makes an editor's dirty buffer clobber-able
/// by background re-indexing; that race is closed by the editor-overlay bit
/// (see [`FileSet::set_overlay`] / [`open_context`]) which `open_context`
/// consults to preserve overlaid buffer content instead of overwriting it
/// from disk. See
/// `protocol_phase1_tests::test_workspace_refresh_keeps_open_buffer_for_uri_alias_paths`.
///
/// PERF WATCH (principle 5): this is called per `FileSet` op (`file_id`,
/// `lookup`, `set_file_text*`, `set_project_only`, `has_overlay`, …) and now
/// does a `path.canonicalize()` stat for `file://`/path URIs. That cost is
/// O(file operations) — edits + indexing — NOT O(resolution steps): salsa
/// resolution is keyed by `FileId`, so canonicalisation happens only at the
/// URI→FileId boundary, dwarfed by the disk read each indexed file already
/// pays. If a future profile shows it on a hot path, cache the canonical key
/// per `FileId` (compute once at registration; have `lookup` consult an
/// alias→canonical map) rather than stat-per-call.
/// Public wrapper over the crate's one URI-canonicalization rule, for
/// callers outside the crate that must compare against [`FileSet`] keys
/// (e.g. `sysml-service::load_workspace`'s root-scope prefix — host URIs
/// are `file://` URLs, so a raw-path prefix never matches them).
pub fn canonical_uri(uri: &str) -> String {
    canonicalize_uri(uri)
}

/// Compute the checkout-independent identity root scope for a file: its
/// path relative to the owning project `root` (forward-slash), or `None`
/// when the file is not under `root` or the path can't be extracted.
///
/// Both sides are canonicalized when on disk so symlink / `..` spellings
/// agree with the canonical `SourceFile.name`. The machine-specific prefix
/// lives entirely in `root`, so the returned relative path is identical
/// across checkouts — the ADR-009 property `content_digest` / `CommitId`
/// rely on.
/// The parent directory of a `file://` / raw-path URI, used as a
/// fallback identity base for a lone file with no directory-rooted project
/// (strict single-file opens). Relativizing against the parent yields the
/// bare basename — checkout-independent, and collision-free because such
/// opens hold a single file per graph.
pub(crate) fn file_uri_parent(uri: &str) -> Option<std::path::PathBuf> {
    let path: std::path::PathBuf = if uri.starts_with("file://") {
        url::Url::parse(uri).ok().and_then(|u| u.to_file_path().ok())?
    } else if uri.starts_with('/') {
        std::path::PathBuf::from(uri)
    } else {
        return None;
    };
    let path = path.canonicalize().unwrap_or(path);
    path.parent().map(std::path::Path::to_path_buf)
}

pub(crate) fn project_relative_scope(uri: &str, root: &std::path::Path) -> Option<String> {
    let raw: std::path::PathBuf = if uri.starts_with("file://") {
        url::Url::parse(uri).ok().and_then(|u| u.to_file_path().ok())?
    } else if uri.starts_with('/') {
        std::path::PathBuf::from(uri)
    } else {
        return None;
    };
    let file = raw.canonicalize().unwrap_or(raw);
    let base = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf());
    let rel = file.strip_prefix(&base).ok()?;
    let s = rel.to_string_lossy().replace('\\', "/");
    (!s.is_empty()).then_some(s)
}

fn canonicalize_uri(uri: &str) -> String {
    let path: Option<std::path::PathBuf> = if uri.starts_with("file://") {
        url::Url::parse(uri).ok().and_then(|u| u.to_file_path().ok())
    } else if uri.starts_with('/') {
        Some(std::path::PathBuf::from(uri))
    } else {
        None
    };
    if let Some(path) = path {
        // Resolve `..` and symlinks when the file exists on disk so every
        // spelling of one physical file collapses to one key.
        if let Ok(canonical) = path.canonicalize() {
            if let Ok(url) = url::Url::from_file_path(&canonical) {
                return url.to_string();
            }
        }
        // Not on disk (unsaved/synthetic) — still normalise to URL form so
        // raw-path and `file://` spellings of the same path agree.
        if let Ok(url) = url::Url::from_file_path(&path) {
            return url.to_string();
        }
    }
    uri.to_owned()
}

/// Unique file identifier.
///
/// This is a plain integer ID, not a salsa-interned type. We manage
/// the URI → FileId mapping ourselves in [`AnalysisHost`](crate::AnalysisHost).
///
/// Following rust-analyzer's pattern where `vfs::FileId` is a simple newtype
/// that exists outside of salsa, and salsa inputs are indexed by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

/// Salsa input: the source text of a single file.
///
/// Each open file gets one `SourceFile` input in the database. When the user
/// edits the file, we call `set_text` on this input, which invalidates all
/// downstream queries that depend on it.
#[salsa::input(debug)]
pub struct SourceFile {
    /// The file name (e.g., URI or path) for diagnostics and display.
    /// Absolute for real files — spans/LSP need the absolute `file://` URI.
    #[returns(ref)]
    pub name: String,
    /// The source text of the file.
    #[returns(ref)]
    pub text: String,
    /// Checkout-independent identity root for this file's canonical keys
    /// (ADR-009). A workspace/project-relative locator, set by the loader
    /// when the owning project root is known; empty when unknown (tests,
    /// synthetic loads), in which case parsing falls back to `name` as the
    /// root scope — the pre-fix behaviour. Kept SEPARATE from `name` so
    /// spans stay absolute while element IDs / `content_digest` /
    /// `CommitId` become machine-independent. `#[default]` so the 59 test
    /// constructors that don't care keep the 2-arg `new(db, name, text)`.
    #[default]
    #[returns(ref)]
    pub root_scope: String,
}

/// Manages the mapping from file URIs to salsa inputs.
///
/// This lives outside of salsa (like rust-analyzer's `Files` struct)
/// because salsa inputs don't support lookup by arbitrary keys — they're
/// just opaque IDs. We maintain the URI → FileId → SourceFile mapping here.
#[derive(Debug, Default)]
pub struct FileSet {
    /// URI → FileId mapping.
    uri_to_id: FxHashMap<String, FileId>,
    /// FileId → SourceFile salsa input mapping.
    id_to_source: FxHashMap<FileId, SourceFile>,
    /// FileId → URI reverse mapping (for diagnostics, etc.).
    id_to_uri: FxHashMap<FileId, String>,
    /// FileId → ProjectHandle mapping (for project-aware resolution).
    id_to_project: FxHashMap<FileId, ProjectHandle>,
    /// FileIds whose `SourceFile` text is an authoritative editor overlay
    /// (an open, possibly-unsaved buffer) rather than disk content. The
    /// background workspace indexer (`open_context`) must NOT overwrite an
    /// overlaid file from disk — it tags the project via
    /// [`set_project_only`](Self::set_project_only) and leaves the text alone.
    /// This is the rust-analyzer vfs overlay-vs-disk distinction.
    overlay_ids: rustc_hash::FxHashSet<FileId>,
    /// Next file ID to assign.
    next_id: u32,
}

impl FileSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create a FileId for the given URI.
    pub fn file_id(&mut self, uri: &str) -> FileId {
        let canonical = canonicalize_uri(uri);
        if let Some(&id) = self.uri_to_id.get(&canonical) {
            return id;
        }
        let id = FileId(self.next_id);
        self.next_id += 1;
        self.uri_to_id.insert(canonical.clone(), id);
        self.id_to_uri.insert(id, canonical);
        id
    }

    /// Look up a FileId by URI without creating one. Accepts either the
    /// raw path form or the `file://` URI form for the same physical
    /// file — both normalize to the same key.
    pub fn lookup(&self, uri: &str) -> Option<FileId> {
        self.uri_to_id.get(&canonicalize_uri(uri)).copied()
    }

    /// Get the URI for a FileId.
    pub fn uri(&self, id: FileId) -> Option<&str> {
        self.id_to_uri.get(&id).map(|s| s.as_str())
    }

    /// Set the source text for a file, creating the salsa input if needed.
    ///
    /// Returns the FileId for the file.
    pub fn set_file_text(
        &mut self,
        db: &mut dyn salsa::Database,
        uri: &str,
        text: String,
    ) -> FileId {
        let id = self.file_id(uri);
        match self.id_to_source.get(&id) {
            Some(&source_file) => {
                // Update existing input — this triggers salsa invalidation.
                source_file.set_text(db).to(text);
            }
            None => {
                // Create new salsa input for this file. Use the canonical
                // URI as the SourceFile name so that downstream consumers
                // (diagnostic spans, hover, etc.) see one stable identity
                // regardless of how the URI was originally spelled.
                let canonical = canonicalize_uri(uri);
                let source_file = SourceFile::new(db, canonical, text);
                self.id_to_source.insert(id, source_file);
            }
        }
        id
    }

    /// Set the source text for a file with project association.
    ///
    /// Like `set_file_text` but also records which project owns the file.
    pub fn set_file_text_in_project(
        &mut self,
        db: &mut dyn salsa::Database,
        uri: &str,
        text: String,
        project_id: ProjectHandle,
    ) -> FileId {
        let id = self.set_file_text(db, uri, text);
        self.id_to_project.insert(id, project_id);
        id
    }

    /// Get the project that owns a file, if any.
    pub fn project_id(&self, id: FileId) -> Option<ProjectHandle> {
        self.id_to_project.get(&id).copied()
    }

    /// Tag a file with a project_id WITHOUT touching its salsa source text.
    ///
    /// Used by `index_workspace_files` for files that are also open in the
    /// editor: did_open has already set the (possibly unsaved) buffer
    /// content, so the indexer must not overwrite it from disk. Before
    /// this method, the indexer just `continue`d on open files, which
    /// left them with no project_id at all — so the workspace
    /// `ProjectFileSet` didn't include them and cross-file resolution
    /// failed for the very file the user is editing. The WaterPort bug.
    ///
    /// If the URI isn't registered yet, this is a no-op — the caller is
    /// expected to have ensured the file is loaded via did_open / a
    /// prior `set_file_text*` call.
    pub fn set_project_only(&mut self, uri: &str, project_id: ProjectHandle) {
        if let Some(&id) = self.uri_to_id.get(&canonicalize_uri(uri)) {
            self.id_to_project.insert(id, project_id);
        }
    }

    /// Set the checkout-independent identity root scope (ADR-009) for a
    /// file's `SourceFile` input. The loader computes this from the file's
    /// owning project root so element IDs / `content_digest` / `CommitId`
    /// are machine-independent; spans keep the absolute `name`. No-op if the
    /// URI isn't registered or the scope is unchanged (avoids a spurious
    /// salsa invalidation / re-parse).
    pub fn set_file_root_scope(
        &self,
        db: &mut dyn salsa::Database,
        uri: &str,
        root_scope: String,
    ) {
        let Some(id) = self.lookup(uri) else { return };
        let Some(&source_file) = self.id_to_source.get(&id) else {
            return;
        };
        if source_file.root_scope(db) != &root_scope {
            source_file.set_root_scope(db).to(root_scope);
        }
    }

    /// Mark a file's current `SourceFile` text as an authoritative editor
    /// overlay (an open buffer). While set, `open_context` preserves the
    /// text instead of overwriting it from disk. No-op if the URI isn't
    /// registered yet — the caller is expected to have set the buffer
    /// content (via `set_file_text*`) first, under the same host lock, so
    /// the overlay and the content it protects are established atomically.
    pub fn set_overlay(&mut self, uri: &str) {
        if let Some(&id) = self.uri_to_id.get(&canonicalize_uri(uri)) {
            self.overlay_ids.insert(id);
        }
    }

    /// Clear a file's editor-overlay status (e.g. on `did_close`), so the
    /// indexer is free to track disk content again.
    pub fn clear_overlay(&mut self, uri: &str) {
        if let Some(&id) = self.uri_to_id.get(&canonicalize_uri(uri)) {
            self.overlay_ids.remove(&id);
        }
    }

    /// Whether the file at `uri` is an active editor overlay. `open_context`
    /// consults this to decide between `set_project_only` (overlaid: keep the
    /// buffer) and `set_file_content_in_project` (disk-backed: load disk).
    pub fn has_overlay(&self, uri: &str) -> bool {
        self.uri_to_id
            .get(&canonicalize_uri(uri))
            .is_some_and(|id| self.overlay_ids.contains(id))
    }

    /// Get the SourceFile input for a FileId, if it exists.
    pub fn source_file(&self, id: FileId) -> Option<SourceFile> {
        self.id_to_source.get(&id).copied()
    }

    /// Remove a file from the set.
    pub fn remove(&mut self, uri: &str) -> Option<FileId> {
        let id = self.uri_to_id.remove(&canonicalize_uri(uri))?;
        self.id_to_source.remove(&id);
        self.id_to_uri.remove(&id);
        self.id_to_project.remove(&id);
        self.overlay_ids.remove(&id);
        Some(id)
    }

    /// Iterate over all file IDs (including stdlib bundle files).
    pub fn file_ids(&self) -> impl Iterator<Item = FileId> + '_ {
        self.id_to_source.keys().copied()
    }

    /// Iterate over file IDs whose owning project is *not* the stdlib bundle.
    ///
    /// Files registered without an explicit project (the typical user-file
    /// path) are included; files registered with the stdlib bundle project
    /// (see [`enable_stdlib`](crate::AnalysisHost::enable_stdlib)) are
    /// excluded. Callers that want a "user files only" view should prefer
    /// this over [`file_ids`](Self::file_ids).
    pub fn user_file_ids(&self) -> impl Iterator<Item = FileId> + '_ {
        let stdlib_pid = ProjectHandle(crate::host::STDLIB_BUNDLE_PROJECT_ID);
        self.id_to_source
            .keys()
            .copied()
            .filter(move |id| self.id_to_project.get(id).copied() != Some(stdlib_pid))
    }

    /// Get all files in a specific project.
    pub fn files_in_project(&self, project_id: ProjectHandle) -> impl Iterator<Item = SourceFile> + '_ {
        self.id_to_project
            .iter()
            .filter(move |(_, &pid)| pid == project_id)
            .filter_map(move |(&id, _)| self.id_to_source.get(&id).copied())
    }

    /// Number of tracked files.
    pub fn len(&self) -> usize {
        self.id_to_source.len()
    }

    /// Whether there are no tracked files.
    pub fn is_empty(&self) -> bool {
        self.id_to_source.is_empty()
    }
}
