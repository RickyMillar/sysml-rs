//! Inspect command implementation.
//!
//! Service-side replacement for the CLI's full-bypass parse → resolve →
//! elaborate → validate → health pipeline. Routes through the
//! salsa-backed `compute_full_diagnostics` (same pipeline the LSP uses)
//! and `Analysis::semantic_tokens` so every transport sees the same
//! result without inlining parser/resolver/elaborator state.

use std::sync::Mutex;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sysml_ide_db::tokens::{token_modifiers, FileTokens, TokenCategory};
use sysml_ide_db::{AnalysisHost, SourceFile};
use sysml_span::Diagnostic as SysmlDiagnostic;

use crate::diagnostics::compute_full_diagnostics;

/// One file's inspect-pipeline result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectFileResult {
    pub uri: String,
    pub diagnostics: Vec<SysmlDiagnostic>,
    pub tokens: Vec<InspectToken>,
}

/// A semantic token rendered for transport.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectToken {
    pub start: usize,
    pub end: usize,
    /// `Class`, `Function`, `Namespace`, ... — `TokenCategory` rendered as
    /// its enum variant name. CLI text output uppercases this for display.
    pub token_type: String,
    pub modifiers: Vec<String>,
}

/// Result of `SysmlService::inspect`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectResponse {
    pub files: Vec<InspectFileResult>,
}

/// Compute the inspect result for a single loaded URI.
///
/// `host` is the service's `AnalysisHost` mutex. The caller must guarantee
/// `source_file` is a salsa input registered under `uri`.
pub fn compute_inspect_file(
    host: &Mutex<AnalysisHost>,
    uri: &str,
    source_file: SourceFile,
) -> InspectFileResult {
    let diagnostics = compute_full_diagnostics(host, uri);
    let (analysis, project_id) = {
        let guard = host.lock().unwrap();
        let project_id = guard
            .file_id(uri)
            .and_then(|file_id| guard.files().project_id(file_id));
        (guard.analysis(), project_id)
    };
    let raw_tokens = analysis.semantic_tokens(source_file, project_id);
    InspectFileResult {
        uri: uri.to_owned(),
        diagnostics,
        tokens: raw_tokens_to_inspect(&raw_tokens),
    }
}

fn raw_tokens_to_inspect(raw: &FileTokens) -> Vec<InspectToken> {
    raw.tokens()
        .iter()
        .map(|t| InspectToken {
            start: t.start,
            end: t.end,
            token_type: token_category_name(t.category).to_owned(),
            modifiers: modifier_flags_to_strings(t.modifiers),
        })
        .collect()
}

fn token_category_name(category: TokenCategory) -> &'static str {
    match category {
        TokenCategory::Namespace => "Namespace",
        TokenCategory::Type => "Type",
        TokenCategory::Class => "Class",
        TokenCategory::Struct => "Struct",
        TokenCategory::Property => "Property",
        TokenCategory::Variable => "Variable",
        TokenCategory::Parameter => "Parameter",
        TokenCategory::Function => "Function",
        TokenCategory::Keyword => "Keyword",
        TokenCategory::Comment => "Comment",
        TokenCategory::String => "String",
        TokenCategory::Number => "Number",
        TokenCategory::Operator => "Operator",
        TokenCategory::Interface => "Interface",
        TokenCategory::Enum => "Enum",
    }
}

fn modifier_flags_to_strings(flags: u32) -> Vec<String> {
    let mut out = Vec::new();
    if flags & token_modifiers::DEFINITION != 0 {
        out.push("definition".to_owned());
    }
    if flags & token_modifiers::ABSTRACT != 0 {
        out.push("abstract".to_owned());
    }
    if flags & token_modifiers::READONLY != 0 {
        out.push("readonly".to_owned());
    }
    if flags & token_modifiers::DERIVED != 0 {
        out.push("derived".to_owned());
    }
    if flags & token_modifiers::UNRESOLVED != 0 {
        out.push("unresolved".to_owned());
    }
    out
}

/// Map a `TokenCategory` variant name (as emitted by `compute_inspect_file`)
/// to the legacy uppercase token name used in CLI text/JSON output.
pub fn category_to_legacy_token_name(category: &str) -> &'static str {
    match category {
        "Namespace" => "NAMESPACE",
        "Type" => "TYPE",
        "Class" => "CLASS",
        "Struct" => "STRUCT",
        "Property" => "PROPERTY",
        "Variable" => "VARIABLE",
        "Parameter" => "PARAMETER",
        "Function" => "FUNCTION",
        "Keyword" => "KEYWORD",
        "Comment" => "COMMENT",
        "String" => "STRING",
        "Number" => "NUMBER",
        "Operator" => "OPERATOR",
        "Interface" => "INTERFACE",
        "Enum" => "ENUM",
        _ => "TYPE",
    }
}
