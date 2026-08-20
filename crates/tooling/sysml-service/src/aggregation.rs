//! Satisfaction matrix aggregation (F5).
//!
//! Provides per-owner aggregate status of constraints, verifications,
//! and requirement satisfaction. Powers the aggregate code lens display
//! like `[3/4 constraints | PASS verification | 2/3 satisfied]`.

use std::collections::HashMap;
use sysml_core::{is_requirement_kind, is_verification_case_kind, ElementKind, ModelGraph};
use sysml_id::ElementId;
use sysml_span::Span;

use crate::evaluation::{self, EvalConstraintResult};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Aggregate status for one owner element (PartDef, PartUsage, Package).
#[derive(Debug, Clone)]
pub struct AggregateStatus {
    /// Owner element name.
    pub owner_name: Option<String>,
    /// Owner element span (for code lens positioning).
    pub owner_span: Option<Span>,
    /// Number of constraints that passed.
    pub constraints_passed: usize,
    /// Number of constraints that failed.
    pub constraints_failed: usize,
    /// Number of verification cases that passed.
    pub verifications_passed: usize,
    /// Number of verification cases that failed.
    pub verifications_failed: usize,
    /// Number of requirements satisfied (via Satisfy relationships).
    pub requirements_satisfied: usize,
    /// Number of requirements unsatisfied.
    pub requirements_unsatisfied: usize,
}

impl AggregateStatus {
    /// Total constraint count.
    pub fn total_constraints(&self) -> usize {
        self.constraints_passed + self.constraints_failed
    }

    /// Total verification count.
    pub fn total_verifications(&self) -> usize {
        self.verifications_passed + self.verifications_failed
    }

    /// Total requirement count.
    pub fn total_requirements(&self) -> usize {
        self.requirements_satisfied + self.requirements_unsatisfied
    }
}

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/// Compute aggregate statuses for all relevant owners in a graph (single-pass).
#[tracing::instrument(level = "debug", skip(graph))]
pub fn aggregate_all_statuses(graph: &ModelGraph) -> Vec<AggregateStatus> {
    // Step 1: Evaluate all constraints once and group by owner
    let constraint_results_grouped = evaluation::evaluate_constraints_grouped(graph);

    // Step 2: Evaluate all verification cases once
    let verification_results = evaluation::evaluate_verification_cases(graph);

    // Step 3: Index verification results by element_id for O(1) lookup
    let verification_map: HashMap<ElementId, &evaluation::VerificationCaseResult> =
        verification_results
            .iter()
            .map(|r| (r.element_id.clone(), r))
            .collect();

    let mut statuses = Vec::new();

    // Find all aggregatable elements (owners of constraints/verifications/requirements)
    for element in graph.elements.values() {
        let is_aggregatable = matches!(
            element.kind,
            ElementKind::PartDefinition
                | ElementKind::PartUsage
                | ElementKind::Package
                | ElementKind::RequirementDefinition
                | ElementKind::RequirementUsage
        );

        if !is_aggregatable {
            continue;
        }

        let status = aggregate_status_cached(
            &element.id,
            graph,
            &constraint_results_grouped,
            &verification_map,
        );

        // Filter out elements with nothing to show
        if status.total_constraints() == 0
            && status.total_verifications() == 0
            && status.total_requirements() == 0
        {
            continue;
        }

        statuses.push(status);
    }

    // Sort by element name for consistent output
    statuses.sort_by(|a, b| a.owner_name.cmp(&b.owner_name));

    statuses
}

/// Optimized version of aggregate_status that uses pre-computed results.
fn aggregate_status_cached(
    owner_id: &ElementId,
    graph: &ModelGraph,
    constraint_results_grouped: &HashMap<ElementId, Vec<EvalConstraintResult>>,
    verification_map: &HashMap<ElementId, &evaluation::VerificationCaseResult>,
) -> AggregateStatus {
    let owner = graph.get_element(owner_id);
    let owner_name = owner.and_then(|e| e.name.clone());
    let owner_span = owner.and_then(|e| e.spans.first().cloned());

    // Count constraints from pre-grouped results
    let mut constraints_passed = 0;
    let mut constraints_failed = 0;

    if let Some(results) = constraint_results_grouped.get(owner_id) {
        for result in results {
            if result.satisfied {
                constraints_passed += 1;
            } else {
                constraints_failed += 1;
            }
        }
    }

    // Count verification cases from pre-indexed results
    let mut verifications_passed = 0;
    let mut verifications_failed = 0;

    for child in graph.children_of(owner_id) {
        let is_verification = is_verification_case_kind(child.kind.clone());
        if is_verification {
            if let Some(result) = verification_map.get(&child.id) {
                use sysml_runtime::cases::VerdictKind;
                match result.verdict {
                    VerdictKind::Pass => verifications_passed += 1,
                    VerdictKind::Fail | VerdictKind::Inconclusive | VerdictKind::Error => {
                        verifications_failed += 1;
                    }
                }
            }
        }
    }

    // Count requirement satisfaction via Satisfy relationships
    let mut requirements_satisfied = 0;
    let mut requirements_unsatisfied = 0;

    // Collect requirement IDs owned by this element
    let requirement_ids: Vec<ElementId> = graph
        .children_of(owner_id)
        .filter(|e| is_requirement_kind(e.kind.clone()))
        .map(|e| e.id.clone())
        .collect();

    // Check for Satisfy relationships
    use sysml_core::RelationshipKind;
    for req_id in requirement_ids {
        let has_satisfy = graph
            .relationships
            .values()
            .any(|rel| matches!(rel.kind, RelationshipKind::Satisfy) && rel.target == req_id);

        if has_satisfy {
            requirements_satisfied += 1;
        } else {
            requirements_unsatisfied += 1;
        }
    }

    AggregateStatus {
        owner_name,
        owner_span,
        constraints_passed,
        constraints_failed,
        verifications_passed,
        verifications_failed,
        requirements_satisfied,
        requirements_unsatisfied,
    }
}

/// Format an aggregate status for code lens display.
///
/// Returns something like `[3/4 constraints | PASS verification | 2/3 satisfied]`
#[tracing::instrument(level = "debug", skip(status))]
pub fn format_aggregate_lens(status: &AggregateStatus) -> String {
    let mut parts = Vec::new();

    if status.total_constraints() > 0 {
        parts.push(format!(
            "{}/{} constraints",
            status.constraints_passed,
            status.total_constraints()
        ));
    }

    if status.total_verifications() > 0 {
        let verdict = if status.verifications_failed == 0 {
            "PASS"
        } else {
            "FAIL"
        };
        parts.push(format!("{} verification", verdict));
    }

    if status.total_requirements() > 0 {
        parts.push(format!(
            "{}/{} satisfied",
            status.requirements_satisfied,
            status.total_requirements()
        ));
    }

    if parts.is_empty() {
        return String::new();
    }

    format!("[{}]", parts.join(" | "))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, Value, VisibilityKind};

    fn make_element(kind: ElementKind, name: &str) -> Element {
        Element::new(ElementId::new_v4(), kind).with_name(name)
    }

    #[test]
    fn test_aggregate_all_statuses() {
        let mut graph = ModelGraph::new();

        // Create two parts with constraints
        let part1_id = ElementId::new_v4();
        let part1 = Element::new(part1_id.clone(), ElementKind::PartUsage).with_name("part1");
        graph.add_element(part1);

        let speed1 =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(50));
        graph.add_owned_element(speed1, part1_id.clone(), VisibilityKind::Public);

        let c1 = make_element(ElementKind::ConstraintUsage, "check1")
            .with_prop("constraint", Value::String("speed < 100".into()));
        graph.add_owned_element(c1, part1_id.clone(), VisibilityKind::Public);

        let part2_id = ElementId::new_v4();
        let part2 = Element::new(part2_id.clone(), ElementKind::PartUsage).with_name("part2");
        graph.add_element(part2);

        let speed2 =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(150));
        graph.add_owned_element(speed2, part2_id.clone(), VisibilityKind::Public);

        let c2 = make_element(ElementKind::ConstraintUsage, "check2")
            .with_prop("constraint", Value::String("speed < 100".into()));
        graph.add_owned_element(c2, part2_id.clone(), VisibilityKind::Public);

        // Get all statuses
        let statuses = aggregate_all_statuses(&graph);

        assert_eq!(statuses.len(), 2);
        assert!(statuses
            .iter()
            .any(|s| s.owner_name == Some("part1".into())));
        assert!(statuses
            .iter()
            .any(|s| s.owner_name == Some("part2".into())));
    }

    #[test]
    fn test_format_lens_constraints_only() {
        let status = AggregateStatus {
            owner_name: Some("test".into()),
            owner_span: None,
            constraints_passed: 3,
            constraints_failed: 1,
            verifications_passed: 0,
            verifications_failed: 0,
            requirements_satisfied: 0,
            requirements_unsatisfied: 0,
        };

        let formatted = format_aggregate_lens(&status);
        assert_eq!(formatted, "[3/4 constraints]");
    }

    #[test]
    fn test_format_lens_mixed() {
        let status = AggregateStatus {
            owner_name: Some("test".into()),
            owner_span: None,
            constraints_passed: 3,
            constraints_failed: 1,
            verifications_passed: 1,
            verifications_failed: 0,
            requirements_satisfied: 2,
            requirements_unsatisfied: 1,
        };

        let formatted = format_aggregate_lens(&status);
        assert_eq!(
            formatted,
            "[3/4 constraints | PASS verification | 2/3 satisfied]"
        );
    }

    #[test]
    fn test_format_lens_empty() {
        let status = AggregateStatus {
            owner_name: Some("test".into()),
            owner_span: None,
            constraints_passed: 0,
            constraints_failed: 0,
            verifications_passed: 0,
            verifications_failed: 0,
            requirements_satisfied: 0,
            requirements_unsatisfied: 0,
        };

        let formatted = format_aggregate_lens(&status);
        assert_eq!(formatted, "");
    }

    #[test]
    fn test_format_lens_verification_fail() {
        let status = AggregateStatus {
            owner_name: Some("test".into()),
            owner_span: None,
            constraints_passed: 0,
            constraints_failed: 0,
            verifications_passed: 0,
            verifications_failed: 1,
            requirements_satisfied: 0,
            requirements_unsatisfied: 0,
        };

        let formatted = format_aggregate_lens(&status);
        assert_eq!(formatted, "[FAIL verification]");
    }

}
