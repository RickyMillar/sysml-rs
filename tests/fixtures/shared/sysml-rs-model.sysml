package SysmlRs {

// =============================================================
// Shared metadata
// =============================================================

abstract part def Crate;

// Status categories
part def ImplementedCrate :> Crate;
part def PartialCrate :> Crate;
part def PlaceholderCrate :> Crate;

// Layer categories
part def FoundationsCrate :> Crate;
part def CoreCrate :> Crate;
part def TextCrate :> Crate;
part def IdeCrate :> Crate;
part def FeaturesCrate :> Crate;
part def StorageCrate :> Crate;
part def ApiCrate :> Crate;
part def ToolingCrate :> Crate;
part def TestingCrate :> Crate;

// =============================================================
// Foundations (implemented)
// =============================================================

part def sysml_id :> FoundationsCrate, ImplementedCrate;

part def sysml_span :> FoundationsCrate, ImplementedCrate {
part def Span;
part def Diagnostic;
}

part def sysml_meta :> FoundationsCrate, ImplementedCrate;

// =============================================================
// Core (implemented/partial)
// =============================================================

part def sysml_core :> CoreCrate, ImplementedCraate {
abstract part def Resolver;
part def ModelGraph ;
part def NameResolver :> Resolver;
part def ModelGraph;
}

part def sysml_query :> CoreCrate, PartialCrate;

part def sysml_canon :> CoreCrate, PartialCrate;

// =============================================================
// Text frontend (implemented + sidecars)
// =============================================================

part def sysml_text :> TextCrate, ImplementedCrate {
abstract part def Parserpackage SysmlRs {

part def SysmlFile;
}

part def sysml_text_pest :> TextCrate, ImplementedCrate {
part def PestParser :> sysml_text::Parser
}

part def sysml_text_pilot_sidecar :> TextCrate, PlaceholderCrate {
part def PilotSidecarParser :> sysml_text::Parser;
}

part def sysml_text_monticore_sidecar :> TextCrate, PlaceholderCrate {
part def MonticoreSidecarParser :> sysml_text::Parser;
}

part def sysml_text_syside_sidecar :> TextCrate, PlaceholderCrate {
part def SysideSidecarParser :> sysml_text::Parser;
}

// =============================================================
// IDE (partial)
// =============================================================

part def sysml_ts :> IdeCrate, ImplementedCrate;

part def sysml_lsp :> IdeCrate, PartialCrate;

part def sysml_lsp_server :> IdeCrate, PartialCrate;

// =============================================================
// Visualization (implemented)
// =============================================================

part def sysml_vis :> FeaturesCrate, ImplementedCrate;

// =============================================================
// Execution (partial/placeholder)
// =============================================================

part def sysml_run :> FeaturesCrate, PartialCrate {
abstract part def Runner;
}

part def sysml_run_statemachine :> FeaturesCrate, PartialCrate {
part def StateMachineRunner :> sysml_run::Runner;
}

part def sysml_run_constraints :> FeaturesCrate, PartialCrate {
part def ConstraintRunner :> sysml_run::Runner;
}

// =============================================================
// Storage + API (placeholder)
// =============================================================

part def sysml_store :> StorageCrate, PlaceholderCrate {
abstract part def Store;
}

part def sysml_store_postgres :> StorageCrate, PlaceholderCrate {
part def PostgresStore :> sysml_store::Store;
}

part def sysml_api :> ApiCrate, PlaceholderCrate;

// =============================================================
// Tooling + testing
// =============================================================

part def sysml_codegen :> ToolingCrate, ImplementedCrate;

part def sysml_spec_tests :> TestingCrate, ImplementedCrate;

// =============================================================
// Demo pipeline (parse -> resolve -> diagnostics)
// =============================================================

part def ParseResolvePipeline {
    part parser_trait : sysml_text::Parser;
    part parser_impl : sysml_text_pest::PestParser;
    part graph : sysml_core::ModelGraph;
    part resolver : sysml_core::Resolver;
    part diagnostics : sysml_span::Diagnostic;
}

// =============================================================
// Crate dependencies (key edges for demos)
// =============================================================

dependency sysml_core to sysml_id;
dependency sysml_core to sysml_span;
dependency sysml_core to sysml_meta;

dependency sysml_query to sysml_core;
dependency sysml_canon to sysml_core;

dependency sysml_text to sysml_core;
dependency sysml_text to sysml_span;

dependency sysml_text_pest to sysml_text;
dependency sysml_text_pest to sysml_core;
dependency sysml_text_pest to sysml_span;
dependency sysml_text_pest to sysml_id;

dependency sysml_text_pilot_sidecar to sysml_text;
dependency sysml_text_monticore_sidecar to sysml_text;
dependency sysml_text_syside_sidecar to sysml_text;

dependency sysml_ts to sysml_span;

dependency sysml_lsp to sysml_span;
dependency sysml_lsp to sysml_id;
dependency sysml_lsp to sysml_core;

dependency sysml_lsp_server to sysml_lsp;
dependency sysml_lsp_server to sysml_text;
dependency sysml_lsp_server to sysml_ts;
dependency sysml_lsp_server to sysml_span;

dependency sysml_vis to sysml_core;

dependency sysml_run to sysml_core;
dependency sysml_run to sysml_span;

dependency sysml_run_statemachine to sysml_run;
dependency sysml_run_statemachine to sysml_core;
dependency sysml_run_statemachine to sysml_query;
dependency sysml_run_statemachine to sysml_span;

dependency sysml_run_constraints to sysml_run;
dependency sysml_run_constraints to sysml_core;
dependency sysml_run_constraints to sysml_query;
dependency sysml_run_constraints to sysml_span;

dependency sysml_store to sysml_core;
dependency sysml_store to sysml_canon;
dependency sysml_store to sysml_id;

dependency sysml_store_postgres to sysml_store;
dependency sysml_store_postgres to sysml_core;
dependency sysml_store_postgres to sysml_id;
dependency sysml_store_postgres to sysml_canon;

dependency sysml_api to sysml_store;
dependency sysml_api to sysml_query;
dependency sysml_api to sysml_core;
dependency sysml_api to sysml_canon;
dependency sysml_api to sysml_id;

dependency sysml_spec_tests to sysml_text_pest;
dependency sysml_spec_tests to sysml_text;
dependency sysml_spec_tests to sysml_core;
dependency sysml_spec_tests to sysml_codegen;
}
