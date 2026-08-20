//! Cross-file workspace verification (F7).
//!
//! Merges all open document graphs with the standard library, runs
//! cross-file resolution and verification, and attributes diagnostics
//! back to source files.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use sysml_core::{is_verification_case_kind, ElementKind, ModelGraph};
use sysml_id::ElementId;
use sysml_span::Diagnostic;

use crate::evaluation;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Result of a workspace-wide verification run.
#[derive(Debug)]
pub struct WorkspaceVerifyResult {
    /// Total verification cases found.
    pub total_cases: usize,
    /// Verification cases that passed.
    pub passed: usize,
    /// Verification cases that failed.
    pub failed: usize,
    /// URIs of files that had diagnostics attributed to them.
    pub per_file: std::collections::BTreeSet<String>,
    /// Time taken for the verification run.
    pub elapsed: Duration,
}

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/// Flatten a verification run's per-requirement results into the per-element
/// `(ElementId, VerdictKind, value)` rows the canvas verdict sidecar joins
/// ([`sysml_diagram::VerificationVerdicts`]).
///
/// One row per requirement that carries a real `source_element_id` (the
/// requirement's verdict), plus one row per requirement-constraint whose
/// element id is known (its individual satisfaction, index-aligned
/// `constraints_met` × `constraint_element_ids`). Recurses into
/// sub-requirements, pairing result↔check by requirement id. Rows without a
/// hard id are skipped — no fabricated join targets. Ids are parsed to
/// `ElementId` HERE, once, at the producer (never re-stringified downstream).
pub fn collect_verdict_rows(
    results: &[sysml_runtime::cases::RequirementResult],
    checks: &[sysml_runtime::cases::RequirementCheck],
    out: &mut Vec<(ElementId, sysml_runtime::VerdictKind, Option<f64>)>,
) {
    for result in results {
        let check = checks.iter().find(|c| c.id == result.requirement_id);
        if let Some(id) = result.source_element_id.as_deref() {
            out.push((ElementId::from_string(id), result.verdict, None));
        }
        if let Some(check) = check {
            for (met, element_id) in result
                .constraints_met
                .iter()
                .zip(check.constraint_element_ids.iter())
            {
                let Some(id) = element_id.as_deref() else {
                    continue;
                };
                let verdict = if *met {
                    sysml_runtime::VerdictKind::Pass
                } else {
                    sysml_runtime::VerdictKind::Fail
                };
                out.push((ElementId::from_string(id), verdict, None));
            }
            collect_verdict_rows(&result.subrequirement_results, &check.subrequirements, out);
        }
    }
}

/// Build a merged ModelGraph from all document graphs + library.
pub fn build_merged_graph(
    doc_graphs: &[(String, ModelGraph)],
    library: Option<&ModelGraph>,
) -> ModelGraph {
    let refs: Vec<(&str, &ModelGraph)> = doc_graphs
        .iter()
        .map(|(uri, graph)| (uri.as_str(), graph))
        .collect();
    build_merged_graph_refs(&refs, library)
}

/// Build a merged ModelGraph from borrowed document graphs + library.
///
/// Reference-only variant: avoids deep-cloning source graphs at the call
/// site. Used by `SysmlService::merged_graph()` where source graphs live
/// behind `Arc<ModelGraph>` and only the merge target needs to be owned.
pub fn build_merged_graph_refs(
    doc_graphs: &[(&str, &ModelGraph)],
    library: Option<&ModelGraph>,
) -> ModelGraph {
    let mut merged = ModelGraph::new();

    // Merge library first (if available) with as_library=true
    if let Some(lib) = library {
        merged.merge_from_ref(lib, true);
    }

    // Merge each document graph with as_library=false
    for (_uri, graph) in doc_graphs {
        merged.merge_from_ref(graph, false);
    }

    merged
}

/// Discover verification cases in the merged graph.
///
/// Library-owned cases are excluded: the workspace graph merges the
/// standard library, whose verification-case TEMPLATES
/// (VerificationCases.sysml etc.) are not verifiable model content —
/// running them yields perpetual Inconclusive rows that pollute every
/// rollup (live-caught: a 2-case model reported 6 cases). The MODEL's
/// declared cases are the verification surface.
pub fn discover_verification_cases(graph: &ModelGraph) -> Vec<ElementId> {
    graph
        .elements
        .values()
        .filter(|element| is_verification_case_kind(element.kind.clone()))
        .filter(|element| !graph.is_library_element(&element.id))
        .map(|element| element.id.clone())
        .collect()
}

/// Run workspace-wide verification with timeout protection.
///
/// Accepts an optional library graph directly (callers convert from
/// their own library state representation).
pub fn run_workspace_verification(
    doc_graphs: &[(String, ModelGraph)],
    library: Option<&ModelGraph>,
    timeout: Duration,
) -> WorkspaceVerifyResult {
    let start = Instant::now();

    // Step 1: Build merged graph
    let merged_graph = build_merged_graph(&doc_graphs, library);

    // Step 2: Discover verification cases
    let case_ids = discover_verification_cases(&merged_graph);
    let total_cases = case_ids.len();

    // Step 3: Pre-evaluate ALL verification cases ONCE before the loop
    let all_verification_results = evaluation::evaluate_verification_cases(&merged_graph);

    // Step 4: Index results by element_id for O(1) lookup
    let result_map: HashMap<ElementId, &evaluation::VerificationCaseResult> =
        all_verification_results
            .iter()
            .map(|r| (r.element_id.clone(), r))
            .collect();

    // Step 5: Process each case using the pre-computed results
    let mut passed = 0;
    let mut failed = 0;
    let mut all_diagnostics = Vec::new();

    for case_id in &case_ids {
        // Check timeout before processing each case
        if start.elapsed() >= timeout {
            tracing::warn!(
                timeout_ms = timeout.as_millis(),
                elapsed_ms = start.elapsed().as_millis(),
                processed = passed + failed,
                total_cases,
                "workspace verification timed out before all cases completed"
            );
            break;
        }

        if let Some(element) = merged_graph.get_element(case_id) {
            if let Some(case_name) = &element.name {
                // Look up the pre-computed result (O(1))
                if let Some(result) = result_map.get(case_id) {
                    match result.verdict {
                        sysml_runtime::cases::VerdictKind::Pass => passed += 1,
                        sysml_runtime::cases::VerdictKind::Fail => failed += 1,
                        _ => {} // Inconclusive/Error don't count as pass or fail
                    }

                    // Convert verification results to diagnostics if there are failures
                    if !matches!(result.verdict, sysml_runtime::cases::VerdictKind::Pass) {
                        if let Some(span) = &result.span {
                            let diagnostic = Diagnostic::error(format!(
                                "Verification case '{}': {}",
                                case_name, result.display
                            ))
                            .with_span(span.clone());
                            all_diagnostics.push(diagnostic);
                        }
                    }
                }
            }
        }
    }

    // Step 6: Attribute diagnostics by file, keeping just the set of URIs
    // that had any diagnostic (the per-file payloads were never read).
    let per_file = attribute_diagnostics_by_file(all_diagnostics)
        .into_keys()
        .collect();

    WorkspaceVerifyResult {
        total_cases,
        passed,
        failed,
        per_file,
        elapsed: start.elapsed(),
    }
}

/// Group diagnostics by source file URI.
pub fn attribute_diagnostics_by_file(
    diagnostics: Vec<Diagnostic>,
) -> HashMap<String, Vec<Diagnostic>> {
    let mut by_file: HashMap<String, Vec<Diagnostic>> = HashMap::new();

    for diagnostic in diagnostics {
        if let Some(span) = &diagnostic.span {
            let file = span.file.clone();
            by_file
                .entry(file)
                .or_default()
                .push(diagnostic);
        }
    }

    by_file
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, Value, VisibilityKind};
    use sysml_span::Span;

    fn make_element(kind: ElementKind, name: &str) -> Element {
        Element::new(ElementId::new_v4(), kind).with_name(name)
    }

    #[test]
    fn test_build_merged_graph_combines_elements() {
        let mut graph1 = ModelGraph::new();
        let elem1 = make_element(ElementKind::PartDefinition, "Part1");
        graph1.add_element(elem1);

        let mut graph2 = ModelGraph::new();
        let elem2 = make_element(ElementKind::PartDefinition, "Part2");
        graph2.add_element(elem2);

        let doc_graphs = vec![
            ("file:///a.sysml".to_string(), graph1),
            ("file:///b.sysml".to_string(), graph2),
        ];

        let merged = build_merged_graph(&doc_graphs, None);
        assert_eq!(
            merged.elements.len(),
            2,
            "merged graph should have 2 elements"
        );
    }

    #[test]
    fn test_build_merged_graph_with_library() {
        let mut lib_graph = ModelGraph::new();
        let lib_elem = make_element(ElementKind::Package, "StandardLib");
        lib_graph.add_element(lib_elem);

        let mut user_graph = ModelGraph::new();
        let user_elem = make_element(ElementKind::PartDefinition, "MyPart");
        user_graph.add_element(user_elem);

        let doc_graphs = vec![("file:///user.sysml".to_string(), user_graph)];

        let merged = build_merged_graph(&doc_graphs, Some(&lib_graph));
        assert_eq!(
            merged.elements.len(),
            2,
            "merged graph should have library + user elements"
        );
    }

    #[test]
    fn test_discover_verification_cases() {
        let mut graph = ModelGraph::new();

        // Add regular element (should not be found)
        let part = make_element(ElementKind::PartDefinition, "Part");
        graph.add_element(part);

        // Add VerificationCaseDefinition (should be found)
        let vc_def = make_element(ElementKind::VerificationCaseDefinition, "VCDef");
        let vc_def_id = vc_def.id.clone();
        graph.add_element(vc_def);

        // Add VerificationCaseUsage (should be found)
        let vc_usage = make_element(ElementKind::VerificationCaseUsage, "VCUsage");
        let vc_usage_id = vc_usage.id.clone();
        graph.add_element(vc_usage);

        let cases = discover_verification_cases(&graph);
        assert_eq!(cases.len(), 2, "should find 2 verification cases");
        assert!(
            cases.contains(&vc_def_id),
            "should find VerificationCaseDefinition"
        );
        assert!(
            cases.contains(&vc_usage_id),
            "should find VerificationCaseUsage"
        );
    }

    #[test]
    fn test_attribute_diagnostics_groups_by_file() {
        let span1 = Span::new("file:///a.sysml".to_string(), 0, 10);
        let diag1 = Diagnostic::error("Error in a.sysml").with_span(span1);

        let span2 = Span::new("file:///b.sysml".to_string(), 0, 10);
        let diag2 = Diagnostic::warning("Warning in b.sysml").with_span(span2);

        let span3 = Span::new("file:///a.sysml".to_string(), 20, 30);
        let diag3 = Diagnostic::error("Another error in a.sysml").with_span(span3);

        let diagnostics = vec![diag1, diag2, diag3];
        let by_file = attribute_diagnostics_by_file(diagnostics);

        assert_eq!(by_file.len(), 2, "should group into 2 files");
        assert_eq!(
            by_file.get("file:///a.sysml").map(|v| v.len()),
            Some(2),
            "a.sysml should have 2 diagnostics"
        );
        assert_eq!(
            by_file.get("file:///b.sysml").map(|v| v.len()),
            Some(1),
            "b.sysml should have 1 diagnostic"
        );
    }

    #[test]
    fn test_run_workspace_verification_empty() {
        let doc_graphs: Vec<(String, ModelGraph)> = Vec::new();
        let timeout = Duration::from_secs(5);

        let result = run_workspace_verification(&doc_graphs, None, timeout);
        assert_eq!(result.total_cases, 0, "should have 0 cases");
        assert_eq!(result.passed, 0, "should have 0 passed");
        assert_eq!(result.failed, 0, "should have 0 failed");
    }

    #[test]
    fn test_run_workspace_verification_with_case() {
        // Create a graph with a verification case
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage).with_name("vehicle");
        graph.add_element(owner);

        // Add sibling attribute for context
        let speed =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(50));
        graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

        // Add verification case with passing requirement
        let vc_id = ElementId::new_v4();
        let vc = Element::new(vc_id.clone(), ElementKind::VerificationCaseDefinition)
            .with_name("SpeedCheck");
        let vc_id = graph.add_owned_element(vc, owner_id.clone(), VisibilityKind::Public);

        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementUsage)
            .with_name("speed-limit")
            .with_prop("constraint", Value::String("speed < 100".into()));
        graph.add_owned_element(req, vc_id.clone(), VisibilityKind::Public);

        let doc_graphs = vec![("file:///test.sysml".to_string(), graph)];

        let timeout = Duration::from_secs(5);

        let result = run_workspace_verification(&doc_graphs, None, timeout);
        assert_eq!(result.total_cases, 1, "should find 1 verification case");
    }

    /// B14 integration: parsing the real `requirements.sysml` + `calculations.sysml`
    /// fixtures and running workspace verification must NOT return a blanket
    /// verdict:error for every case. Before the fix, every verification case
    /// collapsed to "all constraints failed to compile: element <unnamed>
    /// has no compilable expression children". After, the verify chain
    /// (`objective { verify X; }` → `require constraint : C;` → C's expression)
    /// compiles cleanly; cases without bound variables become Inconclusive,
    /// not Error.
    #[test]
    fn b14_verify_chain_follows_require_constraint_reference() {
        let source = r#"
            package Model {
                constraint def BrewTempConstraint {
                    in temp : Real;
                    temp >= 90 and temp <= 96
                }
                requirement def BrewTempReq {
                    require constraint : BrewTempConstraint;
                }
                verification def BrewTempTest {
                    objective {
                        verify BrewTempReq;
                    }
                }
            }
        "#;

        // Test-only inline TS parse (TS-3.6: swapped from PestParser as part
        // of the test/bench/example callsite sweep). `crate::parse` was deleted
        // in S2.T3.
        let mut graph = {
            use sysml_parser_incremental::TreeSitterParser;
            use sysml_parser_trait::{Parser, SysmlFile};
            let result = TreeSitterParser::new().parse(&[SysmlFile::new("test.sysml", source)]);
            result.graph
        };
        sysml_core::resolution::resolve_references(&mut graph);
        sysml_core::elaborate::elaborate(&mut graph);

        let doc_graphs = vec![("test.sysml".to_string(), graph)];
        let result =
            run_workspace_verification(&doc_graphs, None, Duration::from_secs(5));

        // Probe the per-case result via the evaluation module directly so we
        // can assert on the verdict kind itself, not just the pass/fail
        // tallies that `WorkspaceVerifyResult` exposes.
        let merged = crate::workspace_verify::build_merged_graph(&doc_graphs, None);
        let verification_results =
            crate::evaluation::evaluate_verification_cases(&merged);
        assert_eq!(result.total_cases, 1, "one verification case");
        let case = verification_results
            .iter()
            .find(|r| r.case_name == "BrewTempTest")
            .expect("BrewTempTest case present");

        // B14 contract: the case must no longer collapse to Error because
        // the constraint chain is now followed. With `temp` unbound we
        // expect Inconclusive (UndefinedVariable path), never Error.
        assert!(
            !matches!(case.verdict, sysml_runtime::cases::VerdictKind::Error),
            "expected non-Error verdict, got {:?}: {}",
            case.verdict,
            case.display
        );
    }
}
