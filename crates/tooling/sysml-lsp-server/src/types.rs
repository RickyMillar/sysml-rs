//! Core types and constants for the SysML LSP server.

// LSP server: tower-lsp patterns use unwrap/expect for client sends,
// indexing is bounds-checked by protocol invariants, Arc cloning is intentional.
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::manual_let_else,
    clippy::arc_with_non_send_sync,
    clippy::clone_on_ref_ptr,
    clippy::map_err_ignore,
    clippy::needless_pass_by_value,
    clippy::panic
)]

use tower_lsp::lsp_types::{SemanticTokenType, SemanticTokenModifier};

/// Semantic token types for SysML.
/// These map to indices in the legend.
pub(crate) const SEMANTIC_TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::NAMESPACE, // 0: Package, Namespace
    SemanticTokenType::TYPE,      // 1: Generic definition types
    SemanticTokenType::CLASS,     // 2: PartDef, ItemDef, OccurrenceDef, ConnectionDef
    SemanticTokenType::STRUCT,    // 3: RequirementDef, ConstraintDef, CaseDef
    SemanticTokenType::PROPERTY,  // 4: AttributeUsage, AttributeDef
    SemanticTokenType::VARIABLE,  // 5: PartUsage, ItemUsage, general usages
    SemanticTokenType::PARAMETER, // 6: Parameters (in/out/inout)
    SemanticTokenType::FUNCTION,  // 7: ActionDef, ActionUsage, StateDef, StateUsage
    SemanticTokenType::KEYWORD,   // 8: Keywords
    SemanticTokenType::COMMENT,   // 9: Comments
    SemanticTokenType::STRING,    // 10: Strings
    SemanticTokenType::NUMBER,    // 11: Numbers
    SemanticTokenType::OPERATOR,  // 12: Operators
    SemanticTokenType::INTERFACE, // 13: PortDef, InterfaceDef, PortUsage
    SemanticTokenType::ENUM,      // 14: EnumDef, EnumUsage
];

/// Semantic token modifiers for SysML.
/// These are bit flags that can be combined.
pub(crate) const SEMANTIC_TOKEN_MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DEFINITION,       // 0: Definition site
    SemanticTokenModifier::DECLARATION,      // 1: Declaration
    SemanticTokenModifier::READONLY,         // 2: Readonly
    SemanticTokenModifier::STATIC,           // 3: Derived
    SemanticTokenModifier::ABSTRACT,         // 4: Abstract
    SemanticTokenModifier::DEPRECATED,       // 5: Deprecated (not used yet)
    SemanticTokenModifier::new("unresolved"), // 6: Reference resolution failed
];

/// Semantic token modifier bit flags (indices into SEMANTIC_TOKEN_MODIFIERS).
///
/// These bit positions are the **wire legend** — they must match the index of
/// the corresponding entry in `SEMANTIC_TOKEN_MODIFIERS` above, NOT the
/// crate-internal `sysml_ide_db::tokens::token_modifiers` bits (which differ —
/// ABSTRACT is bit 1 there, bit 4 here). `remap_modifiers` in
/// `semantic_tokens.rs` is the single translation point between the two.
pub(crate) const MOD_DEFINITION: u32 = 1 << 0;
pub(crate) const MOD_READONLY: u32 = 1 << 2;
pub(crate) const MOD_DERIVED: u32 = 1 << 3; // Uses STATIC slot for "derived"
pub(crate) const MOD_ABSTRACT: u32 = 1 << 4;
pub(crate) const MOD_UNRESOLVED: u32 = 1 << 6;

/// Sentinel file name for compiler-generated elements with no source location.
pub(crate) const SYNTHETIC_FILE: &str = "<synthetic>";

/// Feature flags for the LSP server.
#[derive(Debug, Clone)]
pub(crate) struct FeatureFlags {
    /// Enable name resolution (T2+).
    pub resolution: bool,
    /// Enable validation (T3).
    pub validation: bool,
    /// Timeout for synchronous resolution (ms).
    pub resolution_timeout_ms: u64,
    /// Override path for the standard library (set via `sysml.libraryPath` config).
    pub library_path_override: Option<std::path::PathBuf>,
    /// Maximum number of files to index in the workspace (0 = unlimited).
    pub max_index_files: u32,
    /// Enable inlay hints.
    pub inlay_hints: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        FeatureFlags {
            resolution: true,
            validation: true,
            resolution_timeout_ms: 500,
            library_path_override: None,
            max_index_files: 500,
            inlay_hints: true,
        }
    }
}

// `LibraryState` was retired in P-RA4 (commit-pending). The four
// lifecycle states (Unloaded / Loading / Loaded / Failed) now live in
// `sysml_service::readiness::LibraryReadiness`, populated by the
// service's `ProgressBus`. Consumers that needed the graph or its
// element-id set read them directly from
// `AnalysisHost::library_graph()` (the graph) and derive the element-id
// set on demand from `graph.elements.keys()`.
