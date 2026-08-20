//! Pure CST navigation + diagnostic helpers used throughout the AST builder.
//!
//! Free functions in this module are not method receivers — they classify
//! tree-sitter node kinds and SysML keywords without needing builder state.
//! The `impl AstBuilder` block contributes the navigation methods that DO
//! need access to the builder's `source` / `file_path` fields (so they can
//! slice node text and build spans).

use super::AstBuilder;
use sysml_span::Span;
use tree_sitter::Node;

/// Map tree-sitter node kinds to human-readable context phrases.
pub(super) fn describe_context(kind: &str) -> Option<&'static str> {
    match kind {
        "source_file" => Some("at top level"),
        "package_body" => Some("in package body"),
        "definition_body" => Some("in definition body"),
        "usage_body" => Some("in usage body"),
        "action_body" => Some("in action body"),
        "state_body" => Some("in state body"),
        "relationship_body" => Some("in relationship body"),
        "package_decl" => Some("in package declaration"),
        "library_package" => Some("in library package"),
        "standard_def" => Some("in definition"),
        "action_def" => Some("in action definition"),
        "state_def" => Some("in state definition"),
        "requirement_def" => Some("in requirement definition"),
        "constraint_def" => Some("in constraint definition"),
        "enum_def" => Some("in enumeration definition"),
        "flow_def" => Some("in flow definition"),
        "standard_usage" => Some("in standard usage"),
        "action_usage" => Some("in action usage"),
        "state_usage" => Some("in state usage"),
        "control_flow_node" => Some("in control flow node"),
        "definition" => Some("in definition"),
        "usage" => Some("in usage"),
        "function_body" => Some("in function body"),
        "calc_body" => Some("in calc body"),
        "import_decl" => Some("in import declaration"),
        "supertype_list" => Some("in supertype list"),
        "typing" => Some("in type annotation"),
        _ => None,
    }
}

/// Walk up from a node to find the name of the nearest enclosing definition/package.
///
/// Given a `definition_body` or `package_body` node, walk to its parent
/// (the definition/package node) and extract the `name` field. This lets
/// error messages say "in definition body of `Foo`" instead of bare
/// "in definition body".
pub(super) fn find_enclosing_name<'a>(mut node: Node<'a>, source: &str) -> Option<String> {
    // Walk up at most 3 levels to find a named parent
    for _ in 0..3 {
        // Check if this node has a `name` field
        if let Some(name_node) = node.child_by_field_name("name") {
            let start = name_node.start_byte();
            let end = name_node.end_byte().min(source.len());
            let name = source[start..end].trim();
            if !name.is_empty() {
                return Some(name.to_owned());
            }
        }
        node = node.parent()?;
    }
    None
}

/// SysML v2 keywords for detecting misplaced keyword errors.
const SYSML_KEYWORDS: &[&str] = &[
    "about",
    "abstract",
    "accept",
    "action",
    "actor",
    "alias",
    "all",
    "allocate",
    "allocation",
    "analysis",
    "assert",
    "assign",
    "assume",
    "attribute",
    "binding",
    "calc",
    "case",
    "comment",
    "concern",
    "connect",
    "connection",
    "constraint",
    "decide",
    "def",
    "default",
    "dependency",
    "derived",
    "do",
    "doc",
    "else",
    "end",
    "entry",
    "enum",
    "exhibit",
    "exit",
    "expose",
    "feature",
    "filter",
    "first",
    "flow",
    "for",
    "fork",
    "frame",
    "from",
    "if",
    "import",
    "in",
    "include",
    "individual",
    "inout",
    "interface",
    "item",
    "join",
    "language",
    "library",
    "merge",
    "message",
    "metadata",
    "nonunique",
    "not",
    "objective",
    "occurrence",
    "of",
    "ordered",
    "out",
    "package",
    "parallel",
    "part",
    "perform",
    "port",
    "private",
    "protected",
    "public",
    "readonly",
    "redefines",
    "ref",
    "render",
    "rendering",
    "rep",
    "require",
    "requirement",
    "return",
    "satisfy",
    "send",
    "snapshot",
    "specializes",
    "stakeholder",
    "state",
    "subject",
    "subsets",
    "succession",
    "then",
    "timeslice",
    "to",
    "transition",
    "use",
    "variant",
    "variation",
    "verification",
    "verify",
    "view",
    "viewpoint",
    "while",
];

/// Check if the given text starts with a SysML keyword.
pub(super) fn starts_with_keyword(text: &str) -> Option<&'static str> {
    let first_word = text.split_whitespace().next()?;
    SYSML_KEYWORDS.iter().find(|&&kw| first_word == kw).copied()
}

impl<'a> AstBuilder<'a> {
    /// Format an error message with keyword detection and context.
    pub(super) fn format_error_message(
        &self,
        preview: &str,
        context: Option<&String>,
    ) -> String {
        if preview.is_empty() {
            match context {
                Some(ctx) => format!("Syntax error {}", ctx),
                None => "Syntax error".to_owned(),
            }
        } else if let Some(kw) = starts_with_keyword(preview) {
            match context {
                Some(ctx) => format!("Unexpected keyword `{}` {}", kw, ctx),
                None => format!("Unexpected keyword `{}`", kw),
            }
        } else {
            match context {
                Some(ctx) => format!("Syntax error near `{}` {}", preview, ctx),
                None => format!("Syntax error near `{}`", preview),
            }
        }
    }

    /// Find the name field of a node.
    pub(super) fn find_name_field(&self, node: &Node<'a>) -> Option<String> {
        // First try the "name" field
        if let Some(name_node) = node.child_by_field_name("name") {
            let text = self.node_text(&name_node);
            // Handle quoted names
            let text = text.trim_matches('\'').trim_matches('"');
            return Some(text.to_owned());
        }

        // Fall back to the first bare `identifier` child — but never the `ref`
        // field. `satisfy <ref> by …` / `verify <ref>` carry the satisfied
        // requirement as a `ref`-field identifier; that is a *reference* to
        // another element, not this usage's own name. Naming the usage after
        // its ref makes name resolution of `<ref>` self-resolve to the usage
        // (which then satisfies nothing). The usage is anonymous in that form.
        let ref_field_id = node.child_by_field_name("ref").map(|n| n.id());
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                if Some(child.id()) == ref_field_id {
                    return None;
                }
                return Some(self.node_text(&child).to_owned());
            }
        }
        None
    }

    /// Extract the declared short name from a `short_name` child
    /// (`<'REQ-001'>` / `<abc>` → `REQ-001` / `abc`).
    ///
    /// The grammar wraps the name in `< … >` (rules/types.js `short_name`);
    /// quoted names keep their quotes in the CST, so both delimiters are
    /// stripped here, mirroring `find_name_field`'s quote handling.
    pub(super) fn find_short_name(&self, node: &Node<'a>) -> Option<String> {
        let short_node = self.find_child_node(node, "short_name")?;
        let text = self
            .node_text(&short_node)
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim()
            .trim_matches('\'')
            .trim_matches('"')
            .to_owned();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Find the CST node that provides the element's name.
    ///
    /// Returns the tree-sitter node for the "name" field (or identifier fallback),
    /// which can be used to get a narrow `name_span` for hover/highlight.
    pub(super) fn find_name_node(&self, node: &Node<'a>) -> Option<Node<'a>> {
        if let Some(name_node) = node.child_by_field_name("name") {
            return Some(name_node);
        }
        // Mirror `find_name_field`: a `ref`-field identifier (`satisfy <ref>`)
        // is a reference, not this element's name — don't claim its span.
        let ref_field_id = node.child_by_field_name("ref").map(|n| n.id());
        if let Some(id_node) = self.find_child_node(node, "identifier") {
            if Some(id_node.id()) != ref_field_id {
                return Some(id_node);
            }
        }
        // Search inside feature_declaration children — usages like
        // `subject vehicle : Vehicle` have their name nested inside
        // a feature_declaration child, not directly on the usage node.
        if let Some(feat_decl) = self.find_child_node(node, "feature_declaration") {
            if let Some(name_node) = feat_decl.child_by_field_name("name") {
                return Some(name_node);
            }
            return self.find_child_node(&feat_decl, "identifier");
        }
        None
    }

    /// Find a qualified name in a node tree.
    pub(super) fn find_qualified_name(&self, node: &Node<'a>) -> Option<String> {
        // Look for qualified_name node
        if let Some(qname) = self.find_child_node(node, "qualified_name") {
            return Some(self.node_text(&qname).to_owned());
        }

        // Fall back to identifier
        self.find_child_text(node, "identifier")
    }

    /// Find a specific field's text content.
    pub(super) fn find_field_text(&self, node: &Node<'a>, field: &str) -> Option<String> {
        node.child_by_field_name(field)
            .map(|n| self.node_text(&n).to_owned())
    }

    /// Find a child node of a specific kind.
    pub(super) fn find_child_node(&self, node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
        let child_count = node.child_count();
        for i in 0..child_count {
            if let Some(child) = node.child(i) {
                if child.kind() == kind {
                    return Some(child);
                }
            }
        }
        None
    }

    /// Find the first named (non-anonymous) child of a node.
    ///
    /// Useful for skipping anonymous tokens like `=`, `:=`, `{`, `}` to reach
    /// the semantic content (e.g., the expression inside a `default_value` node).
    pub(super) fn find_first_named_child(&self, node: &Node<'a>) -> Option<Node<'a>> {
        let child_count = node.child_count();
        for i in 0..child_count {
            if let Some(child) = node.child(i) {
                if child.is_named() {
                    return Some(child);
                }
            }
        }
        None
    }

    /// For a `send_inline` / `send_action` CST node, return the text of the
    /// `via <port>` target — the port a `send <payload> via <port>` addresses.
    ///
    /// Grammar (`rules/states.js`, `rules/actions.js`):
    ///   `send_inline: seq("send", _expression, optional(seq("via", _expression)), ";")`
    ///   `send_action: seq("send", _expression, optional(seq(choice("to","via"), _expression)), ";")`
    /// The via target carries NO field, so it is read positionally: it is the
    /// SECOND named child (the first is the payload expression). We deliberately
    /// match only `via` (a MessageTransfer to a PORT, Transfers.kerml:67-74), not
    /// `to` (occurrence-addressed action message — a separate routing concern).
    /// No grammar regen is needed because the target is already isolated as a
    /// named child; this mirrors the L38 trigger `via_port` capture.
    pub(super) fn send_via_port_text(&self, node: &Node<'a>) -> Option<&'a str> {
        if !self.has_anonymous_child(node, "via") {
            return None;
        }
        let mut named = Vec::new();
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.is_named() {
                    named.push(child);
                }
            }
        }
        let via_node = named.get(1)?;
        let port = self.node_text(via_node).trim();
        if port.is_empty() {
            None
        } else {
            Some(port)
        }
    }

    /// Check if a node has an anonymous child with the given literal text.
    ///
    /// In tree-sitter, anonymous nodes represent literal tokens from the grammar
    /// (e.g., keywords like "assert", "constraint", operators like "=").
    pub(super) fn has_anonymous_child(&self, node: &Node<'a>, text: &str) -> bool {
        let child_count = node.child_count();
        for i in 0..child_count {
            if let Some(child) = node.child(i) {
                if !child.is_named() && self.node_text(&child) == text {
                    return true;
                }
            }
        }
        false
    }

    /// Find a child node's text content.
    pub(super) fn find_child_text(&self, node: &Node<'a>, kind: &str) -> Option<String> {
        self.find_child_node(node, kind)
            .map(|n| self.node_text(&n).to_owned())
    }

    /// Get the text content of a node.
    pub(super) fn node_text(&self, node: &Node<'a>) -> &'a str {
        let start = node.start_byte();
        let end = node.end_byte();
        if end <= self.source.len() {
            &self.source[start..end]
        } else {
            ""
        }
    }

    /// Create a Span from a node.
    pub(super) fn node_span(&self, node: &Node<'a>) -> Span {
        let mut span = Span::new(self.file_path, node.start_byte(), node.end_byte());
        let pos = node.start_position();
        span.line = Some(pos.row as u32 + 1); // tree-sitter is 0-indexed, Span is 1-indexed
        span.col = Some(pos.column as u32);
        span
    }

    /// Create a Span clamped to the first line of a node.
    ///
    /// Useful for import elements where the CST node may cover a wide region
    /// but diagnostics should point at just the statement line.
    pub(super) fn first_line_span(&self, node: &Node<'a>) -> Span {
        let start = node.start_byte();
        let node_end = node.end_byte().min(self.source.len());
        let text = &self.source[start..node_end];
        let end = text.find('\n').map(|pos| start + pos).unwrap_or(node_end);
        Span::new(self.file_path, start, end)
    }
}
