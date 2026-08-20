//! File-loading discovery primitives — Phase 1 of the file-loading rebuild.
//!
//! Pure functions over disk paths. No salsa, no project handles, no
//! service state. Salsa-tracked wrappers (`discover_root`,
//! `peek_neighbours_for`) live in `sysml-ide-db` and call into the
//! §"Salsa wiring (cross-phase view)".
//!
//! Three primitives:
//!
//! - [`pick_mode`] decides Strict / Discovered / DiscoveredViaManifest
//!   from an [`OpenTarget`].
//! - [`discover`] recursively scans a project root for `*.sysml` and
//!   `*.kerml` files, stopping at nested `sysml.toml` boundaries.
//! - [`peek_neighbours`] does a lightweight tree-sitter pass over a
//!   file's siblings to build a name → file index for IM010 diagnostic
//!   enrichment (P5).
//!
//! these primitives serve, and §5.2 for the mode-selection rule.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sysml_manifest::{load_manifest, walk_up, ManifestError, SysmlManifest, MANIFEST_FILENAME};
use thiserror::Error;
use walkdir::WalkDir;

/// Source-file extensions that count as SysML content.
const SYSML_EXTS: &[&str] = &["sysml", "kerml"];

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// User's intent when opening a context. Built by each transport
/// (CLI / LSP / MCP) and handed to `SysmlService::open_context` in P2.
///
/// NOTE: this Phase-1 shape uses [`PathBuf`] for the file form rather
/// than the [`url::Url`] sketched in `file-loading-model.md` §5.1.
/// `url` is a transport-layer concern (LSP / MCP / REST) and stays
/// out of the leaf `sysml-project` crate. The service wrapper in P2
/// performs URL → path normalisation before calling into here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenTarget {
    /// "Open this file." Strict unless an ancestor `sysml.toml` exists.
    File(PathBuf),
    /// "Open this folder." Explicit project intent. Always discovers.
    Folder(PathBuf),
    /// Synthetic buffer (Monaco preview / paste-into-tool). Always Strict;
    /// `uri` is opaque (e.g. `inmemory://`) and `content` is the buffer.
    Synthetic { uri: String, content: String },
}

/// What [`pick_mode`] decided to do with an [`OpenTarget`].
///
/// The `Strict` variant intentionally carries an `Option<PathBuf>`:
/// a real file knows its disk path (peek_neighbours uses it),
/// a synthetic buffer doesn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeDecision {
    /// Strict single-file (or synthetic) mode. Stdlib only.
    Strict { path: Option<PathBuf> },
    /// Discovered project — recursive scan, no manifest.
    Discovered { root: PathBuf },
    /// Discovered project rooted at the manifest's directory.
    DiscoveredViaManifest {
        root: PathBuf,
        manifest_path: PathBuf,
    },
}

/// Why a project exists. Carried on `OpenContext` in P2 for diagnostics
/// and code-action wiring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectKind {
    /// Single file or synthetic buffer. Stdlib only.
    Strict,
    /// Folder opened, no `sysml.toml` found.
    Discovered,
    /// `sysml.toml` at or above the opened root.
    DiscoveredViaManifest,
}

/// Rich result of [`discover`]: everything needed to populate a project
/// in `SysmlService::open_context`.
///
/// Named [`DiscoveredProject`] rather than `DiscoveryResult` to avoid
/// shadowing the existing `discover::DiscoveryResult` (KerML-format
/// `.project.json` / `.workspace.json` walker). Both are re-exported
/// from `lib.rs`.
#[derive(Debug, Clone, Default)]
pub struct DiscoveredProject {
    /// Canonicalised project root.
    pub root: PathBuf,
    /// `(manifest_path, parsed_manifest)` if root contains `sysml.toml`.
    pub manifest: Option<(PathBuf, SysmlManifest)>,
    /// Source files inside root, excluding contents of nested sub-projects.
    pub files: Vec<PathBuf>,
    /// Nested `sysml.toml` projects found under root — Cargo-style isolation
    /// boundaries. Caller decides whether to discover into them with a
    /// fresh call.
    pub sub_projects: Vec<(PathBuf, SysmlManifest)>,
    /// True iff scanning was aborted because the file cap was reached.
    /// (When `true`, [`discover`] returns [`DiscoveryError::CapExceeded`];
    /// this field is reserved for non-fatal future caps.)
    pub capped: bool,
    /// Non-fatal hints collected during scan.
    pub warnings: Vec<DiagnosticHint>,
}

/// A soft hint emitted during discovery. P5's diagnostic layer can lift
/// each into a proper `SysmlDiagnostic` (e.g. IM012 family).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticHint {
    /// Stable code, e.g. `"discovery-skip"`, `"discovery-symlink-skipped"`.
    pub code: String,
    pub message: String,
    pub path: Option<PathBuf>,
}

/// Hard error from [`discover`].
#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error(
        "file count exceeded cap of {cap} while scanning {root}; \
         add `[discovery] max_files = N` to sysml.toml to raise"
    )]
    CapExceeded { root: PathBuf, cap: usize },

    #[error("failed to load manifest at {path}: {source}")]
    Manifest {
        path: PathBuf,
        #[source]
        source: ManifestError,
    },

    #[error("i/o error scanning {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Name → files declaring it. Used by IM010 enrichment in P5 to point at
/// the neighbour that defines a Strict-mode unresolved name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NeighbourIndex {
    pub entries: HashMap<String, Vec<PathBuf>>,
}

impl NeighbourIndex {
    /// Files declaring `name` at top level, if any.
    pub fn lookup(&self, name: &str) -> &[PathBuf] {
        self.entries
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// True iff the index has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// pick_mode
// ---------------------------------------------------------------------------

/// Decide which mode to open `target` in.
///
/// Implements the rule from `file-loading-model.md` §5.2:
///
/// - `File(path)`: walk up from `path.parent()`. First ancestor with
///   `sysml.toml` ⇒ `DiscoveredViaManifest`. Otherwise ⇒ `Strict`.
/// - `Folder(path)`: walk up from `path`. First ancestor (including
///   `path` itself) with `sysml.toml` ⇒ `DiscoveredViaManifest`.
///   Otherwise ⇒ `Discovered { root: path }`.
/// - `Synthetic`: always `Strict { path: None }`.
///
/// This is a pure decision over the directory tree — it does not load
/// the manifest. The caller (service layer) loads it once mode is picked.
pub fn pick_mode(target: &OpenTarget) -> ModeDecision {
    match target {
        OpenTarget::File(path) => {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            match find_manifest_dir(parent) {
                Some(manifest_dir) => ModeDecision::DiscoveredViaManifest {
                    manifest_path: manifest_dir.join(MANIFEST_FILENAME),
                    root: manifest_dir,
                },
                None => ModeDecision::Strict {
                    path: Some(path.clone()),
                },
            }
        }
        OpenTarget::Folder(path) => match find_manifest_dir(path) {
            Some(manifest_dir) => ModeDecision::DiscoveredViaManifest {
                manifest_path: manifest_dir.join(MANIFEST_FILENAME),
                root: manifest_dir,
            },
            None => ModeDecision::Discovered { root: path.clone() },
        },
        OpenTarget::Synthetic { .. } => ModeDecision::Strict { path: None },
    }
}

/// Walk up from `start` until a directory containing `sysml.toml` is
/// found. Returns the *directory*, not the manifest path.
fn find_manifest_dir(start: &Path) -> Option<PathBuf> {
    walk_up(start, |dir| {
        if dir.join(MANIFEST_FILENAME).is_file() {
            Some(dir.to_path_buf())
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// discover
// ---------------------------------------------------------------------------

/// Recursively scan `root` for `*.sysml` / `*.kerml` source files.
///
/// Nested `sysml.toml` files mark sub-project boundaries: their contents
/// are NOT added to the returned `files` list. Instead, the `(dir,
/// manifest)` lands in `sub_projects`. Hidden directories (any name
/// starting with `.`) are skipped, except for `root` itself if it
/// happens to be hidden.
///
/// Symbolic links are not followed (`follow_links(false)`).
///
/// **Path form**: emitted paths preserve the form of the input `root` —
/// if `root` is relative, output paths are relative; if absolute, they
/// are absolute. No symlink resolution. This keeps the URIs the salsa
/// host stores aligned with whatever shape callers (CLI, LSP, MCP)
/// passed in. Boundary checks use paths from the same walk so they
/// stay self-consistent.
///
/// Errors:
/// - [`DiscoveryError::CapExceeded`] if scanning would produce more than
///   `max_files` source files.
/// - [`DiscoveryError::Manifest`] if a `sysml.toml` (root or nested) is
///   malformed.
/// - [`DiscoveryError::Io`] if `root` does not exist as a directory.
pub fn discover(root: &Path, max_files: usize) -> Result<DiscoveredProject, DiscoveryError> {
    if !root.is_dir() {
        return Err(DiscoveryError::Io {
            path: root.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "discovery root is not a directory",
            ),
        });
    }
    let root = root.to_path_buf();

    let manifest = load_manifest_at(&root)?;

    let mut project = DiscoveredProject {
        root: root.clone(),
        manifest,
        files: Vec::new(),
        sub_projects: Vec::new(),
        capped: false,
        warnings: Vec::new(),
    };

    // Two-pass scan: WalkDir visits a directory's entries in
    // platform-dependent (readdir) order, so a nested `inner.sysml`
    // may be yielded before its sibling `sysml.toml`. Locate every
    // nested manifest first, then walk files knowing which subtrees
    // are off-limits.

    let hidden_filter = |e: &walkdir::DirEntry| -> bool {
        let p = e.path();
        if p == root {
            return true;
        }
        match p.file_name().and_then(|n| n.to_str()) {
            Some(name) if name.starts_with('.') => false,
            _ => true,
        }
    };

    // Pass 1: nested manifests.
    let mut sub_project_dirs: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(hidden_filter)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                project.warnings.push(DiagnosticHint {
                    code: "discovery-skip".to_string(),
                    message: format!("skipped: {err}"),
                    path: err.path().map(Path::to_path_buf),
                });
                continue;
            }
        };
        let path = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }
        if path.file_name().map(|n| n == MANIFEST_FILENAME).unwrap_or(false)
            && path.parent() != Some(&root)
        {
            let parent = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.clone());
            // A nested manifest under another nested manifest is the
            // inner one's problem, not ours — we treat the outermost
            // boundary as authoritative.
            if sub_project_dirs.iter().any(|sp| parent.starts_with(sp)) {
                continue;
            }
            let m = load_manifest(path).map_err(|e| DiscoveryError::Manifest {
                path: path.to_path_buf(),
                source: e,
            })?;
            project.sub_projects.push((parent.clone(), m));
            sub_project_dirs.push(parent);
        }
    }

    // Pass 2: source files outside any sub-project.
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(hidden_filter)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue, // already recorded in pass 1
        };
        let path = entry.path();
        if !entry.file_type().is_file() || !is_sysml_source(path) {
            continue;
        }
        // Inside a nested sub-project? Skip.
        if sub_project_dirs.iter().any(|sp| path.starts_with(sp)) {
            continue;
        }
        if project.files.len() >= max_files {
            return Err(DiscoveryError::CapExceeded {
                root,
                cap: max_files,
            });
        }
        project.files.push(path.to_path_buf());
    }

    project.files.sort();
    project.sub_projects.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(project)
}

fn load_manifest_at(
    dir: &Path,
) -> Result<Option<(PathBuf, SysmlManifest)>, DiscoveryError> {
    let candidate = dir.join(MANIFEST_FILENAME);
    if !candidate.is_file() {
        return Ok(None);
    }
    let m = load_manifest(&candidate).map_err(|e| DiscoveryError::Manifest {
        path: candidate.clone(),
        source: e,
    })?;
    Ok(Some((candidate, m)))
}

fn is_sysml_source(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SYSML_EXTS.contains(&e))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// peek_neighbours
// ---------------------------------------------------------------------------

/// Scan `file`'s sibling `.sysml`/`.kerml` files and extract top-level
/// `package` / `namespace` / `*_def` / `*_usage` names from each.
///
/// Used by Strict-mode IM010 enrichment (P5) to point the user at the
/// neighbour where the unresolved name is defined.
///
/// Best-effort: a file that fails to parse silently contributes no
/// entries. Reads files into memory but does NOT load them into any
/// project or salsa db.
pub fn peek_neighbours(file: &Path) -> NeighbourIndex {
    let mut idx = NeighbourIndex::default();
    let Some(parent) = file.parent() else {
        return idx;
    };
    let Ok(rd) = std::fs::read_dir(parent) else {
        return idx;
    };

    let target_canon = file.canonicalize().ok();

    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_sysml_source(&path) {
            continue;
        }
        // Skip the file itself.
        if path == file {
            continue;
        }
        if let (Some(t), Ok(p)) = (target_canon.as_ref(), path.canonicalize()) {
            if &p == t {
                continue;
            }
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        for name in extract_top_level_names(&source) {
            idx.entries.entry(name).or_default().push(path.clone());
        }
    }

    // Stable file order per name keeps tests + diagnostics deterministic.
    for vec in idx.entries.values_mut() {
        vec.sort();
        vec.dedup();
    }

    idx
}

/// Parse `source` with tree-sitter and pull every named declaration
/// reachable from `source_file`. Covers top-level `package_decl`,
/// `namespace_decl`, `library_package`, every `*_def` / `*_usage` rule,
/// and their named children — peek_neighbours' job is "tell the user
/// which neighbour file declares this name," so a `part def Foo` nested
/// inside `package Bar` should be findable as `Foo` here. The output
/// is a flat list of bare names; the importer surfaces them via plain
/// neighbour-file notes, not qualified paths.
///
/// Best-effort: returns `Vec::new()` on parser-init failure or malformed
/// input.
fn extract_top_level_names(source: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_sysml::language())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out = Vec::new();

    // Iterative DFS so we don't blow the stack on deeply-nested
    // namespaces. Skip the root itself; collect every descendant that
    // carries a `name` field.
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(name_node) = child.child_by_field_name("name") {
                if let Ok(text) = name_node.utf8_text(bytes) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push(strip_quotes(trimmed));
                    }
                }
            }
            stack.push(child);
        }
    }

    out
}

/// `'foo bar'` (quoted_name) → `foo bar`. Otherwise pass through.
fn strip_quotes(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
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

    // -- helpers ------------------------------------------------------------

    /// Create a temp dir with files specified as (relative_path, content).
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

    // -- pick_mode ----------------------------------------------------------

    #[test]
    fn pick_mode_strict_for_orphan_file() {
        let dir = scratch(&[("foo.sysml", "package Foo;")]);
        let target = OpenTarget::File(dir.path().join("foo.sysml"));
        let mode = pick_mode(&target);
        assert!(
            matches!(mode, ModeDecision::Strict { path: Some(_) }),
            "got {mode:?}"
        );
    }

    #[test]
    fn pick_mode_discovered_via_manifest_for_file_with_ancestor_toml() {
        let dir = scratch(&[
            ("sysml.toml", MIN_MANIFEST),
            ("src/foo.sysml", "package Foo;"),
        ]);
        let target = OpenTarget::File(dir.path().join("src/foo.sysml"));
        match pick_mode(&target) {
            ModeDecision::DiscoveredViaManifest { root, manifest_path } => {
                assert_eq!(
                    root.canonicalize().unwrap(),
                    dir.path().canonicalize().unwrap()
                );
                assert_eq!(manifest_path.file_name().unwrap(), MANIFEST_FILENAME);
            }
            other => panic!("expected DiscoveredViaManifest, got {other:?}"),
        }
    }

    #[test]
    fn pick_mode_discovered_for_orphan_folder() {
        let dir = scratch(&[("foo.sysml", "package Foo;")]);
        let target = OpenTarget::Folder(dir.path().to_path_buf());
        match pick_mode(&target) {
            ModeDecision::Discovered { root } => assert_eq!(root, dir.path()),
            other => panic!("expected Discovered, got {other:?}"),
        }
    }

    #[test]
    fn pick_mode_folder_with_root_manifest_is_discovered_via_manifest() {
        let dir = scratch(&[("sysml.toml", MIN_MANIFEST), ("foo.sysml", "package Foo;")]);
        let target = OpenTarget::Folder(dir.path().to_path_buf());
        assert!(matches!(
            pick_mode(&target),
            ModeDecision::DiscoveredViaManifest { .. }
        ));
    }

    #[test]
    fn pick_mode_folder_inside_manifest_walks_up() {
        let dir = scratch(&[
            ("sysml.toml", MIN_MANIFEST),
            ("src/inner/foo.sysml", "package Foo;"),
        ]);
        // Opening the inner folder should still bind to the manifest root.
        let target = OpenTarget::Folder(dir.path().join("src/inner"));
        match pick_mode(&target) {
            ModeDecision::DiscoveredViaManifest { root, .. } => {
                assert_eq!(
                    root.canonicalize().unwrap(),
                    dir.path().canonicalize().unwrap()
                );
            }
            other => panic!("expected DiscoveredViaManifest, got {other:?}"),
        }
    }

    #[test]
    fn pick_mode_synthetic_always_strict() {
        let target = OpenTarget::Synthetic {
            uri: "inmemory://buffer-1".into(),
            content: "package Foo;".into(),
        };
        assert_eq!(pick_mode(&target), ModeDecision::Strict { path: None });
    }

    #[test]
    fn pick_mode_carries_file_path_in_strict() {
        let dir = scratch(&[("foo.sysml", "package Foo;")]);
        let path = dir.path().join("foo.sysml");
        let target = OpenTarget::File(path.clone());
        match pick_mode(&target) {
            ModeDecision::Strict { path: Some(p) } => assert_eq!(p, path),
            other => panic!("expected Strict with path, got {other:?}"),
        }
    }

    // -- discover -----------------------------------------------------------

    #[test]
    fn discover_empty_folder_returns_empty_set() {
        let dir = scratch(&[]);
        let res = discover(dir.path(), 1000).unwrap();
        assert!(res.files.is_empty());
        assert!(res.sub_projects.is_empty());
        assert!(res.manifest.is_none());
        assert!(!res.capped);
    }

    #[test]
    fn discover_finds_top_level_files() {
        let dir = scratch(&[
            ("a.sysml", "package A;"),
            ("b.kerml", "package B;"),
            ("README.md", "ignored"),
        ]);
        let res = discover(dir.path(), 1000).unwrap();
        assert_eq!(res.files.len(), 2);
        assert!(res.files.iter().any(|p| p.file_name().unwrap() == "a.sysml"));
        assert!(res.files.iter().any(|p| p.file_name().unwrap() == "b.kerml"));
    }

    #[test]
    fn discover_recurses_subdirs() {
        let dir = scratch(&[
            ("top.sysml", "package T;"),
            ("nested/deep/leaf.sysml", "package L;"),
        ]);
        let res = discover(dir.path(), 1000).unwrap();
        assert_eq!(res.files.len(), 2);
    }

    #[test]
    fn discover_skips_hidden_dirs() {
        let dir = scratch(&[
            ("visible.sysml", "package V;"),
            (".hidden/buried.sysml", "package H;"),
            (".git/index.sysml", "package G;"),
        ]);
        let res = discover(dir.path(), 1000).unwrap();
        assert_eq!(res.files.len(), 1);
        assert_eq!(res.files[0].file_name().unwrap(), "visible.sysml");
    }

    #[test]
    fn discover_returns_root_manifest() {
        let dir = scratch(&[("sysml.toml", MIN_MANIFEST), ("a.sysml", "package A;")]);
        let res = discover(dir.path(), 1000).unwrap();
        let (path, m) = res.manifest.expect("manifest");
        assert_eq!(path.file_name().unwrap(), MANIFEST_FILENAME);
        assert_eq!(m.project.name, "test");
    }

    #[test]
    fn discover_isolates_nested_subprojects() {
        let dir = scratch(&[
            ("outer.sysml", "package Outer;"),
            ("sub/sysml.toml", MIN_MANIFEST),
            ("sub/inner.sysml", "package Inner;"),
            ("sub/deeper/leaf.sysml", "package Leaf;"),
        ]);
        let res = discover(dir.path(), 1000).unwrap();
        // Only outer.sysml is in files; sub/ contents stay in sub-project.
        assert_eq!(res.files.len(), 1);
        assert_eq!(res.files[0].file_name().unwrap(), "outer.sysml");
        assert_eq!(res.sub_projects.len(), 1);
        let (sub_dir, sub_mani) = &res.sub_projects[0];
        assert_eq!(sub_dir.file_name().unwrap(), "sub");
        assert_eq!(sub_mani.project.name, "test");
    }

    #[test]
    fn discover_multiple_nested_subprojects() {
        let dir = scratch(&[
            ("root.sysml", "package R;"),
            ("a/sysml.toml", MIN_MANIFEST),
            ("a/x.sysml", "package AX;"),
            ("b/sysml.toml", MIN_MANIFEST),
            ("b/y.sysml", "package BY;"),
            ("loose/z.sysml", "package Z;"),
        ]);
        let res = discover(dir.path(), 1000).unwrap();
        // root.sysml + loose/z.sysml — sub-project contents excluded.
        assert_eq!(res.files.len(), 2);
        assert_eq!(res.sub_projects.len(), 2);
        // Sorted by path.
        assert_eq!(res.sub_projects[0].0.file_name().unwrap(), "a");
        assert_eq!(res.sub_projects[1].0.file_name().unwrap(), "b");
    }

    #[test]
    fn discover_cap_at_exact_limit_is_ok() {
        let dir = scratch(&[
            ("a.sysml", "package A;"),
            ("b.sysml", "package B;"),
            ("c.sysml", "package C;"),
        ]);
        let res = discover(dir.path(), 3).unwrap();
        assert_eq!(res.files.len(), 3);
    }

    #[test]
    fn discover_cap_over_limit_errors() {
        let dir = scratch(&[
            ("a.sysml", "package A;"),
            ("b.sysml", "package B;"),
            ("c.sysml", "package C;"),
            ("d.sysml", "package D;"),
        ]);
        let err = discover(dir.path(), 3).unwrap_err();
        match err {
            DiscoveryError::CapExceeded { cap, .. } => assert_eq!(cap, 3),
            other => panic!("expected CapExceeded, got {other:?}"),
        }
    }

    #[test]
    fn discover_cap_error_message_suggests_manifest() {
        let dir = scratch(&[("a.sysml", ""), ("b.sysml", "")]);
        let err = discover(dir.path(), 1).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("max_files"), "msg = {msg}");
        assert!(msg.contains("sysml.toml"), "msg = {msg}");
    }

    #[test]
    fn discover_malformed_root_manifest_errors() {
        let dir = scratch(&[
            ("sysml.toml", "this is not toml = "),
            ("a.sysml", "package A;"),
        ]);
        let err = discover(dir.path(), 1000).unwrap_err();
        assert!(matches!(err, DiscoveryError::Manifest { .. }));
    }

    #[test]
    fn discover_malformed_nested_manifest_errors() {
        let dir = scratch(&[
            ("a.sysml", "package A;"),
            ("sub/sysml.toml", "[project\nname = \"oops\""),
        ]);
        let err = discover(dir.path(), 1000).unwrap_err();
        assert!(matches!(err, DiscoveryError::Manifest { .. }));
    }

    #[test]
    fn discover_files_are_sorted() {
        let dir = scratch(&[
            ("z.sysml", ""),
            ("a.sysml", ""),
            ("m.sysml", ""),
        ]);
        let res = discover(dir.path(), 1000).unwrap();
        let names: Vec<_> = res
            .files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.sysml", "m.sysml", "z.sysml"]);
    }

    #[test]
    fn discover_missing_root_errors() {
        let dir = scratch(&[]);
        let missing = dir.path().join("does/not/exist");
        let err = discover(&missing, 1000).unwrap_err();
        assert!(matches!(err, DiscoveryError::Io { .. }));
    }

    // -- peek_neighbours ----------------------------------------------------

    #[test]
    fn peek_finds_package_declared_in_neighbour() {
        let dir = scratch(&[
            ("a.sysml", "package Ports { part def WaterPort; }"),
            ("b.sysml", "package Connections;"),
        ]);
        let idx = peek_neighbours(&dir.path().join("b.sysml"));
        assert_eq!(idx.lookup("Ports").len(), 1);
        assert_eq!(idx.lookup("Ports")[0].file_name().unwrap(), "a.sysml");
    }

    #[test]
    fn peek_excludes_the_file_itself() {
        let dir = scratch(&[("a.sysml", "package Ports;")]);
        let idx = peek_neighbours(&dir.path().join("a.sysml"));
        assert!(idx.is_empty());
    }

    #[test]
    fn peek_returns_empty_for_lone_file() {
        let dir = scratch(&[("solo.sysml", "package Solo;")]);
        let idx = peek_neighbours(&dir.path().join("solo.sysml"));
        assert!(idx.is_empty());
    }

    #[test]
    fn peek_no_false_positive_for_absent_name() {
        let dir = scratch(&[
            ("a.sysml", "package Foo;"),
            ("b.sysml", "package Bar;"),
        ]);
        let idx = peek_neighbours(&dir.path().join("a.sysml"));
        assert!(idx.lookup("Nope").is_empty());
        assert_eq!(idx.lookup("Bar").len(), 1);
    }

    #[test]
    fn peek_finds_def_and_usage_names() {
        // `part def X` and `part x : X` both contribute top-level entries
        // when written outside any package — they are direct children of
        // source_file with a `name` field.
        let dir = scratch(&[
            ("defs.sysml", "part def Engine; part def Wheel;"),
            ("uses.sysml", "package Vehicles;"),
        ]);
        let idx = peek_neighbours(&dir.path().join("uses.sysml"));
        let engine_files = idx.lookup("Engine");
        let wheel_files = idx.lookup("Wheel");
        assert_eq!(engine_files.len(), 1, "Engine should be found");
        assert_eq!(wheel_files.len(), 1, "Wheel should be found");
    }

    #[test]
    fn peek_handles_quoted_names() {
        // `'with space'` is a quoted_name; strip the quotes for lookup.
        let dir = scratch(&[
            ("a.sysml", "package 'My Package';"),
            ("b.sysml", "package Other;"),
        ]);
        let idx = peek_neighbours(&dir.path().join("b.sysml"));
        assert!(
            !idx.lookup("My Package").is_empty()
                || !idx.lookup("'My Package'").is_empty(),
            "quoted name should be findable; got entries: {:?}",
            idx.entries.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn peek_skips_non_sysml_extensions() {
        let dir = scratch(&[
            ("ignored.txt", "package NotMe;"),
            ("ignored.rs", "package NotMe;"),
            ("real.sysml", "package Real;"),
            ("anchor.sysml", "package Anchor;"),
        ]);
        let idx = peek_neighbours(&dir.path().join("anchor.sysml"));
        assert!(idx.lookup("NotMe").is_empty());
        assert_eq!(idx.lookup("Real").len(), 1);
    }

    #[test]
    fn peek_includes_kerml_neighbours() {
        let dir = scratch(&[
            ("a.kerml", "package KermlPkg;"),
            ("b.sysml", "package SysmlPkg;"),
        ]);
        let idx = peek_neighbours(&dir.path().join("b.sysml"));
        assert_eq!(idx.lookup("KermlPkg").len(), 1);
    }

    #[test]
    fn peek_unparseable_neighbour_is_silent() {
        let dir = scratch(&[
            ("a.sysml", "this is not valid sysml @@@ {{{"),
            ("b.sysml", "package Good;"),
        ]);
        // Should not panic; bad neighbour just contributes nothing.
        let idx = peek_neighbours(&dir.path().join("b.sysml"));
        // a.sysml may still emit *some* names via partial parse, but the
        // absence of valid top-level package decl means most won't appear.
        // Key invariant: the call returns.
        let _ = idx;
    }

    #[test]
    fn peek_dedupes_same_name_across_calls_stable_order() {
        // Two neighbours each declare the same name; result should list
        // both files, sorted, deduplicated.
        let dir = scratch(&[
            ("alpha.sysml", "package Shared;"),
            ("beta.sysml", "package Shared;"),
            ("anchor.sysml", "package Anchor;"),
        ]);
        let idx = peek_neighbours(&dir.path().join("anchor.sysml"));
        let files = idx.lookup("Shared");
        assert_eq!(files.len(), 2);
        // Sorted by path → alpha before beta.
        assert!(
            files[0].file_name().unwrap() < files[1].file_name().unwrap(),
            "files not sorted: {files:?}"
        );
    }

    #[test]
    fn peek_does_not_recurse_into_subdirs() {
        // Sibling-only — peek_neighbours globs the parent dir, not the
        // whole tree.
        let dir = scratch(&[
            ("buried/inner.sysml", "package Inner;"),
            ("anchor.sysml", "package Anchor;"),
        ]);
        let idx = peek_neighbours(&dir.path().join("anchor.sysml"));
        assert!(idx.lookup("Inner").is_empty(), "must not see subdirs");
    }

    // -- strip_quotes -------------------------------------------------------

    #[test]
    fn strip_quotes_handles_quoted_and_bare() {
        assert_eq!(strip_quotes("'My Package'"), "My Package");
        assert_eq!(strip_quotes("BarePkg"), "BarePkg");
        assert_eq!(strip_quotes("''"), "");
        // Single quote at one end only — pass through unchanged.
        assert_eq!(strip_quotes("'half"), "'half");
        assert_eq!(strip_quotes("half'"), "half'");
    }
}
