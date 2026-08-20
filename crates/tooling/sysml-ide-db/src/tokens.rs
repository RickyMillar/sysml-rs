//! Raw semantic tokens: tracked query for token extraction.
//!
//! Extracts semantic tokens from both the model graph (element spans) and the
//! CST (comments, keywords, strings, numbers, operators). Returns crate-local
//! types — the LSP layer maps `TokenCategory` to LSP token type indices.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::resolution::resolved_props;
use sysml_core::{ElementKind, Value};

use crate::parse;
use crate::source::SourceFile;
use crate::{Db, LibraryGraph, ProjectFileSet};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Semantic token category (IDE-agnostic).
///
/// The LSP layer maps these to numeric indices matching the server's
/// `SemanticTokensLegend` registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenCategory {
    Namespace,
    Type,
    Class,
    Struct,
    Property,
    Variable,
    Parameter,
    Function,
    Keyword,
    Comment,
    String,
    Number,
    Operator,
    Interface,
    Enum,
}

/// Modifier flags for semantic tokens.
///
/// These are the crate-internal modifier bits. They do **not** share bit
/// positions with the LSP `SEMANTIC_TOKEN_MODIFIERS` legend — the LSP layer
/// owns the wire-legend order and translates these via an explicit remap
/// (`sysml-lsp-server`'s `remap_modifiers`). Never forward these bits to a
/// client verbatim.
pub mod token_modifiers {
    pub const DEFINITION: u32 = 1 << 0;
    pub const ABSTRACT: u32 = 1 << 1;
    pub const READONLY: u32 = 1 << 2;
    pub const DERIVED: u32 = 1 << 3;
    /// The reference could not be resolved to a target. Emitted on bare
    /// (single-segment) `FeatureReferenceExpression`s that the resolver
    /// attempted but failed — an honest signal, not a fabricated colour.
    pub const UNRESOLVED: u32 = 1 << 4;
}

/// A raw semantic token with byte-offset positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawToken {
    /// Start byte offset in the source text.
    pub start: usize,
    /// End byte offset in the source text.
    pub end: usize,
    /// Token category.
    pub category: TokenCategory,
    /// Modifier flags (bitfield of `token_modifiers::*`).
    pub modifiers: u32,
}

/// All semantic tokens for a file.
///
/// Arc-wrapped for cheap cloning in salsa. Uses **content fingerprinting** for
/// equality (not pointer identity): `file_semantic_tokens` now depends on the
/// elaborated graph (`elaborate_file_best`), so any workspace-file edit re-runs the
/// query — but a semantically-unchanged token set must compare equal so salsa
/// skips the downstream LSP re-diff. Tokens are sorted at construction so the
/// fingerprint is order-stable despite `add_model_tokens`' HashMap iteration.
#[derive(Clone, Debug)]
pub struct FileTokens(Arc<FileTokensData>);

#[derive(Debug)]
struct FileTokensData {
    tokens: Vec<RawToken>,
    fingerprint: u64,
}

impl FileTokens {
    fn new(mut tokens: Vec<RawToken>) -> Self {
        // Deterministic order → stable fingerprint (and the shape the LSP
        // delta-encoder wants anyway).
        tokens.sort_by_key(|t| (t.start, t.end, t.category as u8, t.modifiers));
        let fingerprint = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            tokens.hash(&mut h);
            h.finish()
        };
        Self(Arc::new(FileTokensData {
            tokens,
            fingerprint,
        }))
    }

    /// The raw token list (sorted by start/end).
    pub fn tokens(&self) -> &[RawToken] {
        &self.0.tokens
    }

    /// Whether there are no tokens.
    pub fn is_empty(&self) -> bool {
        self.0.tokens.is_empty()
    }
}

salsa_arc_wrapper!(fingerprint, FileTokens, FileTokensData);

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

/// Extract semantic tokens from a file's resolved model graph and CST.
///
/// Combines three token sources:
/// 1. **Model tokens**: declaration spans from the graph (definitions, usages, …).
/// 2. **Reference tokens**: resolution-backed expression references
///    (`FeatureReferenceExpression`), coloured by their resolved target's kind.
/// 3. **CST tokens**: lexical tokens from tree-sitter (comments, keywords, …).
///
/// Depends on: `elaborate_file_best()` (Layer 3) + `parse_tree()` (Layer 1). The
/// ELABORATED-graph dependency carries every additive resolution prop the
/// reference-token emitters read: `resolved_props::FEATURE_REFERENCE` (written by
/// the name-resolution pass, inside `resolve_file_best`) AND
/// `resolvedTransition{Source,Target}` / `resolvedObjective` (written by the
/// *elaboration* pass, which runs only in `elaborate_file_best`, not plain
/// resolution — Phase B.2). Elaboration is additive, so model/CST/FRE tokens are
/// unchanged; it only adds the transition/objective resolved refs.
#[tracing::instrument(level = "debug", skip(db, project_files, library))]
#[salsa::tracked]
pub fn file_semantic_tokens(
    db: &dyn Db,
    source_file: SourceFile,
    project_files: Option<ProjectFileSet>,
    library: Option<LibraryGraph>,
) -> FileTokens {
    tracing::debug!(
        document_uri = source_file.name(db),
        "starting semantic token extraction"
    );
    let elaborated = crate::analysis::elaborate_file_best(db, source_file, project_files, library);
    let graph = elaborated.graph();
    let uri = source_file.name(db);

    let mut tokens = Vec::new();

    // 1. Model tokens from graph elements (declarations, at name_span)
    add_model_tokens(&mut tokens, graph, uri);

    // 2. CST tokens from tree-sitter
    if let Some(cached_tree) = parse::parse_tree(db, source_file) {
        let mut cursor = cached_tree.tree().walk();
        add_cst_tokens(&mut tokens, &mut cursor, 0);
    }

    // 3. Reference tokens: resolution-backed expression references (FRE).
    // These WIN over the CST walker's purely-syntactic guess: a bare
    // expression ref parses as `feature_chain > identifier`, which
    // `add_cst_tokens` colours TYPE — a wrong guess (the ref is a property /
    // variable, not a type). The resolution-backed token carries the correct
    // category (from the resolved target's kind), so we drop any earlier token
    // sharing its exact span and replace it. Real type refs (`: Real`) have no
    // reference token and keep their CST TYPE.
    let mut reference_tokens = collect_reference_tokens(graph, uri);
    // Transition source/target reference-site names (Phase B.2). The parser
    // records each name's byte span as `*NameStart`/`*NameEnd` int props and
    // elaboration stamps the resolved state as `resolvedTransition{Source,Target}`
    // — same resolution-backed / UNRESOLVED-fallback rule as FRE, just a
    // different span-carrier.
    collect_prop_ref_tokens(
        graph,
        uri,
        &ElementKind::TransitionUsage,
        &[
            (
                "sourceNameStart",
                "sourceNameEnd",
                resolved_props::TRANSITION_SOURCE,
            ),
            (
                "targetNameStart",
                "targetNameEnd",
                resolved_props::TRANSITION_TARGET,
            ),
        ],
        &mut reference_tokens,
    );
    // Subject reference sites (`subject <name>`) — the name references an
    // existing part; elaboration stamps `resolvedSubject` on the membership.
    collect_prop_ref_tokens(
        graph,
        uri,
        &ElementKind::SubjectMembership,
        &[("subjectNameStart", "subjectNameEnd", resolved_props::SUBJECT)],
        &mut reference_tokens,
    );
    // Assignment-action target sites (`<target> = …`) — the LHS references an
    // existing feature; elaboration stamps `resolvedAssignmentTarget`.
    collect_prop_ref_tokens(
        graph,
        uri,
        &ElementKind::AssignmentActionUsage,
        &[(
            "targetNameStart",
            "targetNameEnd",
            resolved_props::ASSIGNMENT_TARGET,
        )],
        &mut reference_tokens,
    );
    if !reference_tokens.is_empty() {
        let claimed: rustc_hash::FxHashSet<(usize, usize)> =
            reference_tokens.iter().map(|t| (t.start, t.end)).collect();
        tokens.retain(|t| !claimed.contains(&(t.start, t.end)));
        tokens.extend(reference_tokens);
    }

    // 4. Remove non-comment tokens that fall inside comment spans.
    // Doc comments like `doc /* ... */` are composite CST nodes; the parser may
    // also produce model-graph elements whose spans land inside the comment text,
    // causing spurious TYPE/OPERATOR tokens inside comments.
    let comment_spans: Vec<(usize, usize)> = tokens
        .iter()
        .filter(|t| t.category == TokenCategory::Comment)
        .map(|t| (t.start, t.end))
        .collect();
    if !comment_spans.is_empty() {
        tokens.retain(|t| {
            t.category == TokenCategory::Comment
                || !comment_spans
                    .iter()
                    .any(|&(cs, ce)| t.start >= cs && t.end <= ce)
        });
    }

    FileTokens::new(tokens)
}

/// Map an ElementKind to a TokenCategory.
pub fn element_kind_to_category(kind: &ElementKind) -> TokenCategory {
    match kind {
        // Namespace types
        ElementKind::Package | ElementKind::LibraryPackage => TokenCategory::Namespace,

        // Class-like definitions
        ElementKind::PartDefinition
        | ElementKind::ItemDefinition
        | ElementKind::OccurrenceDefinition
        | ElementKind::ConnectionDefinition
        | ElementKind::AllocationDefinition
        | ElementKind::AttributeDefinition
        | ElementKind::ViewDefinition
        | ElementKind::RenderingDefinition
        | ElementKind::MetadataDefinition
        | ElementKind::StateDefinition => TokenCategory::Class,

        // Behavioral definitions → Function (calculations are functions, cases invoke behavior)
        ElementKind::CalculationDefinition
        | ElementKind::AnalysisCaseDefinition
        | ElementKind::UseCaseDefinition
        | ElementKind::VerificationCaseDefinition => TokenCategory::Function,

        // Struct-like definitions
        ElementKind::RequirementDefinition
        | ElementKind::ConstraintDefinition
        | ElementKind::ConcernDefinition => TokenCategory::Struct,

        // State usages
        ElementKind::StateUsage => TokenCategory::Variable,

        // Action/behavior
        ElementKind::ActionDefinition
        | ElementKind::ActionUsage
        | ElementKind::PerformActionUsage
        | ElementKind::SendActionUsage
        | ElementKind::AcceptActionUsage
        | ElementKind::AssignmentActionUsage
        | ElementKind::IfActionUsage
        | ElementKind::WhileLoopActionUsage
        | ElementKind::ForLoopActionUsage
        | ElementKind::TerminateActionUsage
        | ElementKind::DecisionNode
        | ElementKind::MergeNode
        | ElementKind::ForkNode
        | ElementKind::JoinNode
        | ElementKind::TransitionUsage => TokenCategory::Function,

        // Port/interface
        ElementKind::PortDefinition
        | ElementKind::InterfaceDefinition
        | ElementKind::PortUsage
        | ElementKind::InterfaceUsage => TokenCategory::Interface,

        // Enum
        ElementKind::EnumerationDefinition | ElementKind::EnumerationUsage => TokenCategory::Enum,

        // Attribute usages
        ElementKind::AttributeUsage => TokenCategory::Property,

        // Typing/specialization relationships
        ElementKind::FeatureTyping
        | ElementKind::Specialization
        | ElementKind::Subsetting
        | ElementKind::Redefinition
        | ElementKind::ReferenceSubsetting => TokenCategory::Type,

        // Import relationships
        ElementKind::MembershipImport | ElementKind::NamespaceImport => TokenCategory::Namespace,

        // Constraint/requirement usages → Struct (matches their definitions)
        ElementKind::ConstraintUsage
        | ElementKind::RequirementUsage
        | ElementKind::ConcernUsage
        | ElementKind::AssertConstraintUsage => TokenCategory::Struct,

        // Behavioral usages → Function
        ElementKind::FlowUsage
        | ElementKind::CalculationUsage
        | ElementKind::AnalysisCaseUsage
        | ElementKind::UseCaseUsage
        | ElementKind::IncludeUseCaseUsage
        | ElementKind::VerificationCaseUsage => TokenCategory::Function,

        // Metadata → Property (annotation)
        ElementKind::MetadataUsage => TokenCategory::Property,

        // Explicit structural usages → Variable (documents intent, not catch-all)
        ElementKind::PartUsage
        | ElementKind::ItemUsage
        | ElementKind::ConnectionUsage
        | ElementKind::AllocationUsage
        | ElementKind::OccurrenceUsage
        | ElementKind::ViewUsage
        | ElementKind::RenderingUsage => TokenCategory::Variable,

        // Catch-all
        _ if kind.is_definition() => TokenCategory::Type,
        _ if kind.is_usage() => TokenCategory::Variable,
        _ => TokenCategory::Type,
    }
}

/// Extract model tokens from graph elements.
fn add_model_tokens(tokens: &mut Vec<RawToken>, graph: &sysml_core::ModelGraph, uri: &str) {
    for element in graph.elements.values() {
        if matches!(
            element.kind,
            ElementKind::Membership | ElementKind::OwningMembership
        ) {
            continue;
        }

        // Skip entry/do/exit subaction markers — they're ActionUsage elements whose
        // span covers only the keyword ("entry", "do", "exit"). Without this skip,
        // they'd emit a FUNCTION token that overrides the CST KEYWORD token.
        if element.get_prop("stateSubactionKind").is_some() {
            continue;
        }

        // Model tokens emit only when `name_span` is set. Tree-sitter's
        // ast_builder populates `name_span` from the identifier CST node
        // for every named declaration (Package, Definition, Usage). When
        // it is absent the element either has no name (anonymous
        // relationship, synthetic member) or is a kind not meant to be
        // coloured by the model walker — in either case we emit no token
        // here and let the CST walker handle the surface text.
        //
        // The historical `spans.first()` fallback painted whole-body
        // spans, which Monaco's overlap-rejection then silently dropped
        // along with the inner CST tokens. See
        let token_span = element
            .name_span
            .as_ref()
            .filter(|s| s.file == uri && s.start != s.end);

        if let Some(span) = token_span {
            let category = element_kind_to_category(&element.kind);
            let mut modifiers = 0u32;
            if element.kind.is_definition() && !element.kind.is_relationship() {
                modifiers |= token_modifiers::DEFINITION;
            }
            if element
                .props
                .get("isAbstract")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                modifiers |= token_modifiers::ABSTRACT;
            }
            if element
                .props
                .get("isReadOnly")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                modifiers |= token_modifiers::READONLY;
            }
            if element
                .props
                .get("isDerived")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                modifiers |= token_modifiers::DERIVED;
            }
            tokens.push(RawToken {
                start: span.start,
                end: span.end,
                category,
                modifiers,
            });
        }
    }
}

/// Emit resolution-backed reference tokens for `FeatureReferenceExpression`s.
///
/// An FRE carries no `name_span` (so `add_model_tokens` never emits it) — its
/// reference identifier span is `spans[0]` (the exact node span set at mint,
/// `expression_elements.rs`). The reference-resolution pass writes an additive
/// `resolved_props::FEATURE_REFERENCE` (`Value::Ref`) on success; here we emit a
/// token at `spans[0]` categorised by the **resolved target's** kind
/// (`element_kind_to_category`), per the design's rule that a reference's colour
/// derives from what it resolves to, never an invented category.
///
/// A bare (single-segment) FRE that the resolver *attempted but failed* gets an
/// honest `VARIABLE` + `UNRESOLVED` token. Without it the ref would inherit the
/// CST walker's wrong `feature_chain > identifier` TYPE guess — exactly the
/// miscolour the honesty invariant forbids (a reference we could not resolve
/// must not be painted as if it resolved to a type). Dotted / `::` refs are
/// deliberately deferred to B.1.2 (they need a Pass-1 chain root): they are
/// "not yet attempted", not "attempted and failed", so we leave them to the CST
/// guess rather than flag them unresolved.
fn collect_reference_tokens(graph: &sysml_core::ModelGraph, uri: &str) -> Vec<RawToken> {
    let mut out = Vec::new();
    for fre_id in graph.element_ids_by_kind(&ElementKind::FeatureReferenceExpression) {
        let Some(element) = graph.get_element(fre_id) else {
            continue;
        };
        // Reference site = the exact identifier span, in this file.
        let Some(span) = element
            .spans
            .first()
            .filter(|s| s.file == uri && s.start != s.end)
        else {
            continue;
        };
        match element.props.get(resolved_props::FEATURE_REFERENCE) {
            // Resolved: colour derives from the resolved target's kind, never
            // an invented category.
            Some(Value::Ref(target_id)) => {
                let category = graph
                    .get_element(target_id)
                    .map(|t| element_kind_to_category(&t.kind))
                    .unwrap_or(TokenCategory::Variable);
                out.push(RawToken {
                    start: span.start,
                    end: span.end,
                    category,
                    modifiers: 0,
                });
            }
            // Unresolved: only bare single-segment refs get the honest
            // UNRESOLVED token (see fn doc). Dotted/`::` refs (B.1.2) are left
            // to the CST guess.
            _ => {
                let name = element.name.as_deref().unwrap_or("");
                if name.is_empty() || name.contains('.') || name.contains("::") {
                    continue;
                }
                out.push(RawToken {
                    start: span.start,
                    end: span.end,
                    category: TokenCategory::Variable,
                    modifiers: token_modifiers::UNRESOLVED,
                });
            }
        }
    }
    out
}

/// Emit resolution-backed tokens for reference-site names the parser records as
/// `(start,end)` int companion props plus a resolved `Value::Ref` prop.
///
/// This is the sibling of `collect_reference_tokens` for references that don't
/// live on a `FeatureReferenceExpression` element's `spans[0]` — transition
/// `source`/`target` names (and, later, objectives). `sites` lists, per element
/// kind, the `(startProp, endProp, resolvedRefProp)` triples to emit. The colour
/// rule is identical to FRE: resolved → the target's kind; a name present but
/// unresolved → `VARIABLE + UNRESOLVED` (honest, never a fabricated colour).
///
/// The int offsets are into a single file's text, so we only emit for elements
/// that actually belong to `uri` (a workspace-merged graph mixes files).
fn collect_prop_ref_tokens(
    graph: &sysml_core::ModelGraph,
    uri: &str,
    kind: &ElementKind,
    sites: &[(&str, &str, &str)],
    out: &mut Vec<RawToken>,
) {
    for id in graph.element_ids_by_kind(kind) {
        let Some(el) = graph.get_element(id) else {
            continue;
        };
        // The name spans are byte offsets into THIS file — skip elements parsed
        // from a different file (workspace merge).
        let in_file = el.name_span.as_ref().is_some_and(|s| s.file == uri)
            || el.spans.iter().any(|s| s.file == uri);
        if !in_file {
            continue;
        }
        for (start_key, end_key, ref_key) in sites {
            let (Some(Value::Int(start)), Some(Value::Int(end))) =
                (el.props.get(*start_key), el.props.get(*end_key))
            else {
                continue;
            };
            let (start, end) = (*start as usize, *end as usize);
            if start >= end {
                continue;
            }
            match el.props.get(*ref_key) {
                Some(Value::Ref(target_id)) => {
                    let category = graph
                        .get_element(target_id)
                        .map(|t| element_kind_to_category(&t.kind))
                        .unwrap_or(TokenCategory::Variable);
                    out.push(RawToken {
                        start,
                        end,
                        category,
                        modifiers: 0,
                    });
                }
                // Name present but unresolved → honest UNRESOLVED (Phase D rule).
                _ => out.push(RawToken {
                    start,
                    end,
                    category: TokenCategory::Variable,
                    modifiers: token_modifiers::UNRESOLVED,
                }),
            }
        }
    }
}

/// Maximum recursion depth for CST traversal to prevent stack overflow.
const MAX_CST_DEPTH: usize = 128;

/// Extract CST tokens for lexical elements (comments, strings, numbers, keywords, operators).
fn add_cst_tokens(
    tokens: &mut Vec<RawToken>,
    cursor: &mut tree_sitter::TreeCursor<'_>,
    depth: usize,
) {
    if depth > MAX_CST_DEPTH {
        return;
    }
    loop {
        let node = cursor.node();
        let start = node.start_byte();
        let end = node.end_byte();

        if start < end {
            let kind = node.kind();
            let category = match kind {
                "comment" | "doc_comment" | "doc_string" | "comment_element" | "sl_note" | "ml_note" => {
                    Some(TokenCategory::Comment)
                }
                "string_literal" => Some(TokenCategory::String),
                "integer_literal" | "real_literal" => Some(TokenCategory::Number),
                _ if !node.is_named() => {
                    let text = node.kind();
                    if text.len() >= 2 && text.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
                    {
                        Some(TokenCategory::Keyword)
                    } else if text
                        .chars()
                        .all(|c| !c.is_alphanumeric() && !c.is_whitespace())
                        && !text.is_empty()
                    {
                        // Brackets are coloured by Monaco itself (language-config
                        // `brackets`), so emitting a token for them would only
                        // fight the client's bracket-pair colouring — skip. The
                        // structural separators `;` and `,` get no such client
                        // treatment, so without a token they hit the magenta
                        // hard-fail default; emit OPERATOR so they render as
                        // neutral punctuation.
                        match text {
                            "{" | "}" | "(" | ")" | "[" | "]" => None,
                            _ => Some(TokenCategory::Operator),
                        }
                    } else {
                        None
                    }
                }
                _ if node.is_named() && node.kind() == "identifier" => {
                    if let Some(parent) = node.parent() {
                        match parent.kind() {
                            "type_ref" | "qualified_name" | "feature_chain" => {
                                Some(TokenCategory::Type)
                            }
                            // Import/expose target names (`import ISQ::*`,
                            // `expose ProtectionCoreODE`). The `::`/`*` glyphs already
                            // get OPERATOR; without this arm the name segments
                            // hit the magenta hard-fail default. The target
                            // resolves to a namespace/package, so colour the
                            // whole path uniformly as NAMESPACE — a purely
                            // syntactic classification keyed on the CST parent,
                            // matching the `type_ref` precedent above (no
                            // resolution lookup). Both single- and multi-segment
                            // import targets funnel through these two rules
                            // (`import_target` in tree-sitter rules/namespaces.js).
                            "import_single_name" | "import_qualified_name" => {
                                Some(TokenCategory::Namespace)
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(cat) = category {
                tokens.push(RawToken {
                    start,
                    end,
                    category: cat,
                    modifiers: 0,
                });
                // Don't recurse into terminal token nodes
                if !cursor.goto_next_sibling() {
                    return;
                }
                continue;
            }
        }

        // Recurse into children
        if cursor.goto_first_child() {
            add_cst_tokens(tokens, cursor, depth + 1);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootDatabase;

    #[test]
    fn tokens_simple_package() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let tokens = file_semantic_tokens(&db, sf, None, None);
        assert!(!tokens.is_empty());
        // Should have at least a keyword token ("package") and a namespace token ("Foo")
        let has_keyword = tokens
            .tokens()
            .iter()
            .any(|t| t.category == TokenCategory::Keyword);
        assert!(has_keyword, "Should have keyword token for 'package'");
    }

    #[test]
    fn tokens_model_and_cst() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(
            &db,
            "test.sysml".to_string(),
            "// A comment\npackage Foo {\n  part def Engine;\n}".to_string(),
        );
        let tokens = file_semantic_tokens(&db, sf, None, None);
        let categories: Vec<_> = tokens.tokens().iter().map(|t| t.category).collect();
        assert!(
            categories.contains(&TokenCategory::Comment),
            "Should have comment token"
        );
        assert!(
            categories.contains(&TokenCategory::Keyword),
            "Should have keyword token"
        );
    }

    #[test]
    fn tokens_empty_file() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), String::new());
        let tokens = file_semantic_tokens(&db, sf, None, None);
        assert!(tokens.is_empty());
    }

    #[test]
    fn tokens_memoization() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let t1 = file_semantic_tokens(&db, sf, None, None);
        let t2 = file_semantic_tokens(&db, sf, None, None);
        assert_eq!(t1, t2, "Memoized results should be pointer-equal");
    }

    #[test]
    fn tokens_import_target_is_namespace_and_semicolon_is_operator() {
        // Bucket 1 + Bucket 2 regression: `import ISQ::*;` must colour the
        // target name `ISQ` as NAMESPACE and the trailing `;` as OPERATOR.
        // Neither is a declaration `name_span`, so both come from the CST
        // walker; a regression here reintroduces the magenta hard-fail in the
        // simulation-app editor (which has no Monarch fallback).
        let db = RootDatabase::default();
        let src = "package P {\n    private import ISQ::*;\n}";
        let sf = SourceFile::new(&db, "test.sysml".to_string(), src.to_string());
        let tokens = file_semantic_tokens(&db, sf, None, None);

        // `ISQ` occupies bytes for the identifier on line 2.
        let isq_start = src.find("ISQ").unwrap();
        let isq = tokens
            .tokens()
            .iter()
            .find(|t| t.start == isq_start && t.end == isq_start + 3)
            .expect("no token emitted for import target `ISQ`");
        assert_eq!(
            isq.category,
            TokenCategory::Namespace,
            "import target should be NAMESPACE, not magenta"
        );

        // The statement-terminating `;` must carry an OPERATOR token.
        let semi = src.rfind(';').unwrap();
        let semi_tok = tokens
            .tokens()
            .iter()
            .find(|t| t.start == semi && t.end == semi + 1)
            .expect("no token emitted for `;`");
        assert_eq!(semi_tok.category, TokenCategory::Operator);

        // Brackets stay untokenised (Monaco colours them itself).
        let brace = src.find('{').unwrap();
        assert!(
            !tokens.tokens().iter().any(|t| t.start == brace),
            "`{{` should not get a token (Monaco owns bracket colouring)"
        );
    }

    #[test]
    fn element_kind_mapping() {
        assert_eq!(
            element_kind_to_category(&ElementKind::Package),
            TokenCategory::Namespace
        );
        assert_eq!(
            element_kind_to_category(&ElementKind::PartDefinition),
            TokenCategory::Class
        );
        assert_eq!(
            element_kind_to_category(&ElementKind::ActionUsage),
            TokenCategory::Function
        );
        assert_eq!(
            element_kind_to_category(&ElementKind::PortDefinition),
            TokenCategory::Interface
        );
    }

    #[test]
    fn element_kind_explicit_usage_mappings() {
        // Constraint/requirement usages → Struct
        assert_eq!(
            element_kind_to_category(&ElementKind::ConstraintUsage),
            TokenCategory::Struct
        );
        assert_eq!(
            element_kind_to_category(&ElementKind::RequirementUsage),
            TokenCategory::Struct
        );
        assert_eq!(
            element_kind_to_category(&ElementKind::ConcernUsage),
            TokenCategory::Struct
        );
        assert_eq!(
            element_kind_to_category(&ElementKind::AssertConstraintUsage),
            TokenCategory::Struct
        );

        // Behavioral usages → Function
        assert_eq!(
            element_kind_to_category(&ElementKind::FlowUsage),
            TokenCategory::Function
        );
        assert_eq!(
            element_kind_to_category(&ElementKind::CalculationUsage),
            TokenCategory::Function
        );

        // Metadata → Property
        assert_eq!(
            element_kind_to_category(&ElementKind::MetadataUsage),
            TokenCategory::Property
        );

        // Enum
        assert_eq!(
            element_kind_to_category(&ElementKind::EnumerationDefinition),
            TokenCategory::Enum
        );
        assert_eq!(
            element_kind_to_category(&ElementKind::EnumerationUsage),
            TokenCategory::Enum
        );

        // Interface
        assert_eq!(
            element_kind_to_category(&ElementKind::InterfaceUsage),
            TokenCategory::Interface
        );

        // Explicit structural usages → Variable
        assert_eq!(
            element_kind_to_category(&ElementKind::PartUsage),
            TokenCategory::Variable
        );
        assert_eq!(
            element_kind_to_category(&ElementKind::ItemUsage),
            TokenCategory::Variable
        );
        assert_eq!(
            element_kind_to_category(&ElementKind::ConnectionUsage),
            TokenCategory::Variable
        );
    }
}
