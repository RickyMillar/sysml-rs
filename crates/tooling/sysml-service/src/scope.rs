//! Graph-scope addressing: the one home for the `__workspace__` sentinel.
//!
//! Service commands address graphs by URI at the wire: either a loaded
//! file URI or the synthetic `__workspace__` value meaning "the merged,
//! library-aware workspace graph". Historically every command body
//! hand-compared `uri == "__workspace__"`; that stringly-typed branch is
//! collapsed here (workspace-scope-collapse plan, 2026-07-16). Parse the
//! wire URI ONCE at the command boundary and match on the typed scope —
//! no other module may compare the sentinel string directly (pinned by
//! `sentinel_string_has_one_home` below).

/// The synthetic wire URI addressing the merged workspace graph.
///
/// Minted into `load_workspace`'s `loaded_uris` so clients know to use
/// it; accepted by every uri-taking query command.
pub const WORKSPACE_URI: &str = "__workspace__";

/// A parsed graph scope: the merged workspace graph, or one loaded file.
///
/// The two arms are distinct *semantics*, not a fallback pair: `File`
/// addresses per-document projections (parse-only graphs, per-file
/// outline roots, single-file export) that LSP and CLI depend on, while
/// `Workspace` addresses the elaborated cross-file graph. Execution
/// commands are workspace-only and don't take a scope at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphScope {
    /// The merged, library-aware workspace graph.
    Workspace,
    /// One loaded file, addressed by its URI.
    File(String),
}

impl GraphScope {
    /// Resolve a wire URI to a typed scope. The only sentinel compare in
    /// the crate.
    pub fn parse(uri: &str) -> GraphScope {
        if uri == WORKSPACE_URI {
            GraphScope::Workspace
        } else {
            GraphScope::File(uri.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_routes_sentinel_to_workspace() {
        assert_eq!(GraphScope::parse("__workspace__"), GraphScope::Workspace);
        assert_eq!(
            GraphScope::parse("a.sysml"),
            GraphScope::File("a.sysml".to_string())
        );
    }

    /// Ratchet: the `__workspace__` string may appear in `lib.rs` only as
    /// doc text — the generated command docs quote the URI, so a reader has
    /// to be able to see it. Live code compares and passes [`WORKSPACE_URI`].
    ///
    /// The pin may fall and must never rise. Final target: zero raw
    /// occurrences outside this module, which needs the remaining doc strings
    /// to interpolate the constant rather than spell it.
    #[test]
    fn sentinel_string_has_one_home() {
        let lib = include_str!("lib.rs");
        let count = lib.matches("__workspace__").count();
        assert!(
            count <= SENTINEL_PIN,
            "raw __workspace__ occurrences in lib.rs grew to {count} (pin: {SENTINEL_PIN}); \
             route new code through GraphScope::parse / WORKSPACE_URI instead"
        );
    }

    const SENTINEL_PIN: usize = 26;
}
