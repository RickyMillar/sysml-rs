//! Element kind to LSP type mappings.
//!
//! Consolidates all 4 element_kind_to_* functions into one module (was M2).

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

use tower_lsp::lsp_types::SymbolKind;

/// Map ElementKind to LSP SymbolKind for document/workspace symbols.
pub(crate) fn element_kind_to_symbol_kind(kind: &sysml_core::ElementKind) -> SymbolKind {
    use sysml_core::ElementKind;
    match kind {
        ElementKind::Package | ElementKind::LibraryPackage => SymbolKind::PACKAGE,
        ElementKind::PartDefinition => SymbolKind::CLASS,
        ElementKind::ActionDefinition => SymbolKind::FUNCTION,
        ElementKind::StateDefinition => SymbolKind::CLASS,
        ElementKind::AttributeDefinition => SymbolKind::PROPERTY,
        ElementKind::PortDefinition => SymbolKind::INTERFACE,
        ElementKind::ConnectionDefinition => SymbolKind::CLASS,
        ElementKind::InterfaceDefinition => SymbolKind::INTERFACE,
        ElementKind::ItemDefinition => SymbolKind::STRUCT,
        ElementKind::RequirementDefinition => SymbolKind::STRUCT,
        ElementKind::ConstraintDefinition => SymbolKind::STRUCT,
        ElementKind::ConcernDefinition => SymbolKind::STRUCT,
        ElementKind::AllocationDefinition => SymbolKind::CLASS,
        ElementKind::EnumerationDefinition => SymbolKind::ENUM,
        ElementKind::CalculationDefinition => SymbolKind::FUNCTION,
        ElementKind::PartUsage => SymbolKind::FIELD,
        ElementKind::ActionUsage => SymbolKind::METHOD,
        ElementKind::AttributeUsage => SymbolKind::PROPERTY,
        ElementKind::PortUsage => SymbolKind::INTERFACE,
        ElementKind::InterfaceUsage => SymbolKind::INTERFACE,
        ElementKind::StateUsage => SymbolKind::ENUM_MEMBER,
        ElementKind::ItemUsage => SymbolKind::VARIABLE,
        ElementKind::EnumerationUsage => SymbolKind::ENUM_MEMBER,
        ElementKind::ConstraintUsage => SymbolKind::STRUCT,
        ElementKind::RequirementUsage => SymbolKind::STRUCT,
        ElementKind::ConnectionUsage => SymbolKind::FIELD,
        ElementKind::FlowUsage => SymbolKind::METHOD,
        ElementKind::TransitionUsage => SymbolKind::METHOD,
        ElementKind::CalculationUsage => SymbolKind::FUNCTION,
        _ if kind.is_definition() => SymbolKind::CLASS,
        _ if kind.is_usage() => SymbolKind::FIELD,
        _ => SymbolKind::VARIABLE,
    }
}

// `element_kind_to_token_type`, `element_kind_to_completion_kind`, and the
// `element_kind_to_hover_label` re-export were removed here. Production no
// longer maps element kind → LSP index in this crate:
//   - semantic tokens come from `AnalysisHost::semantic_tokens` (ide-db raw
//     tokens, whose category is `sysml_ide_db::element_kind_to_category`) run
//     through `semantic_tokens::token_category_to_lsp`;
//   - completion item kinds come from `sysml_service::completion` (i32 wire
//     values decoded in the completion shell);
//   - hover labels come from `sysml_service::hover::element_kind_to_hover_label`.
// These were LSP-local duplicates kept alive only by unit tests; the tests now
// call the canonical functions directly.
