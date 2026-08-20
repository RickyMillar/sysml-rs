//! Shared analysis pipeline for the language-pack eval gates (LPFIX-7).
//!
//! One home for "run a SysML snippet through the real pipeline and report what
//! each phase did" — parse errors, unresolved references, and the fired
//! semantic validator ids. Both the reference-answer gate
//! (`tests/language_pack_evals.rs`) and the candidate-report harness
//! (`examples/candidate_report.rs`) score answers through this exact path, so a
//! generation/repair verdict means the same thing in both places.

use std::collections::BTreeSet;

use sysml_core::elaborate::elaborate;
use sysml_core::ElementKind;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_span::Severity;

/// The outcome of running a snippet through parse → resolve → elaborate →
/// validate.
#[derive(Debug, Clone)]
pub struct Analysis {
    /// Count of error-severity parse diagnostics (0 == parses clean).
    pub parse_errors: usize,
    /// Count of unresolved references after the resolve phase.
    pub unresolved: usize,
    /// Semantic validator rule ids (e.g. `S001`) that fired.
    pub sem_codes: BTreeSet<String>,
}

impl Analysis {
    /// Clean through the parse phase.
    pub fn parses(&self) -> bool {
        self.parse_errors == 0
    }
    /// Clean through the resolve phase (implies `parses`).
    pub fn resolves(&self) -> bool {
        self.parse_errors == 0 && self.unresolved == 0
    }
}

/// Run `source` through the real tree-sitter parser, resolver, elaborator, and
/// semantic validator. Isolated (no workspace / stdlib load), so references to
/// standard-library types stay unresolved by design — a self-contained snippet
/// must define what it uses.
pub fn analyze(source: &str) -> Analysis {
    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new("eval.sysml", source)];
    let mut result = parser.parse(&files);
    let parse_errors = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let res = result.resolve();
    let _ = elaborate(&mut result.graph);
    let sem_codes: BTreeSet<String> = sysml_core::validate_semantic(&result.graph)
        .into_iter()
        .map(|e| e.rule_id.to_string())
        .collect();
    // Touch ElementKind so the analysis mirrors the card gate's pipeline shape.
    let _ = ElementKind::iter().count();
    Analysis {
        parse_errors,
        unresolved: res.unresolved_count,
        sem_codes,
    }
}
