//! Re-export of the canonical expression pretty-printer from `sysml-core`.
//!
//! The original Phase 6B.2 implementation lived here as a diagram-local
//! mirror of `sysml_service::expression_ast::pretty_print` (avoiding an
//! upward dependency on sysml-service). Post-Phase-6D the canonical
//! pretty-printer lives in `sysml_core::expression_pretty`, and both
//! mirrors delegate to it.

pub(crate) use sysml_core::expression_pretty::pretty_print_owner;
