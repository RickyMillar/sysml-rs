//! AST builder for converting tree-sitter CST to ModelGraph.
//!
//! This module provides semantic model construction from tree-sitter parse trees.
//! It uses the shared extraction structs from sysml-parser-trait to ensure consistent
//! model construction across different parser backends.
//!
//! ## Error Handling
//!
//! When tree-sitter produces ERROR nodes (due to syntax errors), this builder:
//! - Records the error as a diagnostic (up to [`MAX_DIAGNOSTICS_PER_ERROR_NODE`])
//! - Emits a summary when a multi-line ERROR node exceeds the per-node cap
//! - Skips the ERROR node's children
//! - Continues processing sibling nodes
//!
//! This enables graceful degradation: valid regions of the file produce
//! semantic elements while syntax errors are tracked as diagnostics.

use std::collections::HashMap;

use sysml_core::{CanonicalKey, ElementId, ModelGraph};
use sysml_span::Diagnostic;

mod connectors;
mod definitions;
mod dispatch;
mod imports;
mod keying;
mod node_helpers;
mod packages;
mod requirements;
mod states;
mod typings;
mod usages;

#[cfg(test)]
use node_helpers::starts_with_keyword;

/// Discriminator for `create_keyed_rels_from_type_refs` so a single helper
/// can route to the matching `*_with_key` builder instead of taking a
/// closure (the `*_with_key` builders all carry different signatures).
#[derive(Debug, Clone, Copy)]
pub(super) enum RelKind {
    Redefinition,
    Subsetting,
    CrossSubsetting,
}
use tree_sitter::{Node, Tree};

/// Maximum diagnostics emitted per tree-sitter ERROR node.
///
/// When a single ERROR node spans many lines (e.g. a parse failure that
/// cascades through the rest of the file), we cap the per-line diagnostics
/// to avoid flooding the user with noise. The first few lines are shown
/// individually, then a summary replaces the remainder.
const MAX_DIAGNOSTICS_PER_ERROR_NODE: usize = 3;

/// Result of building a ModelGraph from tree-sitter CST.
#[derive(Debug)]
pub struct ModelGraphResult {
    /// The constructed model graph (may be partial if there were errors).
    pub graph: ModelGraph,
    /// Diagnostics including syntax errors and warnings.
    pub diagnostics: Vec<Diagnostic>,
}

impl ModelGraphResult {
    /// Create a new result with an empty graph.
    pub fn new() -> Self {
        ModelGraphResult {
            graph: ModelGraph::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Check if there were any errors.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.is_error())
    }
}

impl Default for ModelGraphResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a ModelGraph from a tree-sitter parse tree.
///
/// This function walks the tree-sitter CST iteratively and constructs
/// semantic elements using the shared extraction structs from sysml-text.
///
/// # Arguments
///
/// * `tree` - The tree-sitter parse tree
/// * `source` - The source code that was parsed
/// * `file_path` - The file path for span information
///
/// # Returns
///
/// A `ModelGraphResult` containing the constructed graph and any diagnostics.
pub fn build_model_graph(tree: &Tree, source: &str, file_path: &str) -> ModelGraphResult {
    build_model_graph_scoped(tree, source, file_path, file_path)
}

/// Like [`build_model_graph`], but seeds the root canonical key from
/// `root_scope` instead of `file_path`.
///
/// `root_scope` is the checkout-independent identity root for the file: the
/// file's **project-relative path** (e.g. `a.sysml`), set by
/// `sysml-ide-db`'s `apply_project_root_scope`, NOT the absolute filesystem
/// path. It is **per-file, not a project-wide constant** — this is what keeps
/// two files that each declare `package P` as two distinct elements
/// (`a.sysml::P#Package` vs `b.sysml::P#Package`), matching the spec's
/// one-root-namespace-per-unit model (KerML §7.2.5.3 / §10.2, SysML §7.5.5).
/// See ADR-009's 2026-08-05 amendment: the original §Decision formula wrote a
/// project-wide `project_id` here, which — taken literally — would merge
/// same-named packages by name; that reading is REJECTED, do not "fix" the
/// code toward it. `file_path` is still used verbatim for spans (LSP needs
/// absolute `file://` URIs); only element identity is decoupled from the
/// machine path. Callers with no project context (e.g. the plain
/// [`build_model_graph`] entry) pass `file_path` as the scope, preserving the
/// pre-ADR-009-fix behaviour.
pub fn build_model_graph_scoped(
    tree: &Tree,
    source: &str,
    file_path: &str,
    root_scope: &str,
) -> ModelGraphResult {
    let mut result = ModelGraphResult::new();
    let mut builder = AstBuilder::new(source, file_path, root_scope);
    builder.build(tree.root_node(), &mut result);
    result
}

/// Internal builder state for constructing the ModelGraph.
struct AstBuilder<'a> {
    source: &'a str,
    file_path: &'a str,
    /// Checkout-independent root for the file's canonical keys (ADR-009).
    /// Distinct from `file_path`, which stays absolute for span emission.
    root_scope: &'a str,
    /// CST node IDs that have been consumed by a preceding sibling's processing
    /// (e.g., action_usage absorbed by entry_action). The build loop skips these.
    consumed_nodes: std::collections::HashSet<usize>,
    /// Counter for generating synthetic names for anonymous control-flow nodes
    /// (fork, join, merge, decide). Incremented per keyword type.
    anon_control_counters: [u32; 4], // [fork, join, merge, decide]
    /// Stashed source name from a `first X` succession_usage half, to be
    /// consumed by the immediately following `then Y` succession_usage.
    pending_first_source: Option<String>,
    /// Per-parent, per-kind sibling counters used to derive deterministic
    /// `CanonicalKey::for_anonymous` indices for elements minted by this
    /// builder (ADR-009 / S1.T11b).
    ///
    /// Keyed by `Some(parent_id)` for nested mints, and `None` for elements
    /// minted at file top level.
    sibling_counters: HashMap<Option<ElementId>, HashMap<&'static str, usize>>,
    /// Per-parent, per-kind set of names already minted, used to detect
    /// sibling name collisions (`part def Foo; part def Foo;`) and force
    /// the duplicates onto anonymous canonical keys so they remain distinct
    /// graph elements. S001 distinguishability then has both children to
    /// flag. See ADR-009 §Duplicates.
    seen_named_keys: HashMap<(Option<ElementId>, &'static str), std::collections::HashSet<String>>,
}

/// Work item for iterative tree traversal.
struct WorkItem<'a> {
    node: Node<'a>,
    parent_id: Option<ElementId>,
    /// Canonical key of the parent scope, used to derive reparse-stable IDs
    /// for elements minted while processing `node`. `None` only at the very
    /// top of the walk; `process_node` substitutes `CanonicalKey::root(file_path)`.
    parent_key: Option<CanonicalKey>,
}

impl<'a> AstBuilder<'a> {
    fn new(source: &'a str, file_path: &'a str, root_scope: &'a str) -> Self {
        AstBuilder {
            source,
            file_path,
            root_scope,
            consumed_nodes: std::collections::HashSet::new(),
            anon_control_counters: [0; 4],
            pending_first_source: None,
            sibling_counters: HashMap::new(),
            seen_named_keys: HashMap::new(),
        }
    }



}

#[cfg(test)]
mod tests;
