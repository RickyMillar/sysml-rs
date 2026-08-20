//! SCAP — Sequential Causality Assignment Procedure for bond graphs.
//!
//! Determines the computation order for bond graph elements by assigning
//! causality (which end of each bond imposes effort vs flow). This is
//! important for detecting algebraic loops and high-index DAEs.

use std::collections::HashMap;

use super::connection::{ConnectionGraph, JunctionType, NodeId};
use super::constraints::{ConstitutiveRelation, GeneratedConstraints};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Causality type for a bond graph element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Causality {
    /// Integral causality — preferred for C/I elements. Produces clean ODEs.
    /// C: effort is output (integral of flow). I: flow is output (integral of effort).
    Integral,
    /// Derivative causality — forced by topology. Creates algebraic constraints.
    /// C: flow is output (derivative of effort). I: effort is output (derivative of flow).
    Derivative,
    /// Fixed causality — mandatory for sources.
    /// Se: effort is output. Sf: flow is output.
    Fixed,
    /// Not yet assigned.
    Unassigned,
}

/// Causality assignment for a single element/bond.
#[derive(Debug, Clone)]
pub struct ElementCausality {
    /// Variable name of the element (e.g., "cap.voltage" for a C-element).
    pub element_var: String,
    /// What type of element this is.
    pub element_type: ElementType,
    /// Assigned causality.
    pub causality: Causality,
}

/// Simplified element classification for causality purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementType {
    /// C or I — prefers integral causality.
    Storage,
    /// R or G — no preference.
    Resistive,
    /// Se or Sf — fixed causality.
    Source,
    /// TF or GY — propagates causality.
    TwoPort,
}

/// Result of SCAP analysis.
#[derive(Debug, Clone)]
pub struct CausalityAssignment {
    /// Causality for each element, keyed by primary variable name.
    pub elements: HashMap<String, ElementCausality>,
    /// Elements forced into derivative causality (potential issues).
    pub derivative_causality_warnings: Vec<String>,
    /// Whether the assignment is fully consistent (no unresolvable conflicts).
    pub consistent: bool,
    /// Diagnostics from the assignment process.
    pub diagnostics: Vec<String>,
}

impl CausalityAssignment {
    /// Check if any storage elements have derivative causality.
    pub fn has_derivative_causality(&self) -> bool {
        !self.derivative_causality_warnings.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Classification helpers
// ---------------------------------------------------------------------------

/// Classify a constitutive relation into an element type and primary variable.
fn classify_relation(rel: &ConstitutiveRelation) -> (String, ElementType) {
    match rel {
        ConstitutiveRelation::Capacitance { effort_var, .. } => {
            (effort_var.clone(), ElementType::Storage)
        }
        ConstitutiveRelation::Inductance { flow_var, .. } => {
            (flow_var.clone(), ElementType::Storage)
        }
        ConstitutiveRelation::Resistance { flow_var, .. } => {
            (flow_var.clone(), ElementType::Resistive)
        }
        ConstitutiveRelation::Conductance { flow_var, .. } => {
            (flow_var.clone(), ElementType::Resistive)
        }
        ConstitutiveRelation::EffortSource { effort_var, .. } => {
            (effort_var.clone(), ElementType::Source)
        }
        ConstitutiveRelation::FlowSource { flow_var, .. } => {
            (flow_var.clone(), ElementType::Source)
        }
        ConstitutiveRelation::Transformer { effort_in_var, .. } => {
            (effort_in_var.clone(), ElementType::TwoPort)
        }
        ConstitutiveRelation::Gyrator { effort_in_var, .. } => {
            (effort_in_var.clone(), ElementType::TwoPort)
        }
    }
}

/// Extract all variable names referenced by a constitutive relation.
fn relation_variables(rel: &ConstitutiveRelation) -> Vec<String> {
    match rel {
        ConstitutiveRelation::Capacitance {
            effort_var,
            flow_var,
            ..
        } => {
            vec![effort_var.clone(), flow_var.clone()]
        }
        ConstitutiveRelation::Inductance {
            flow_var,
            effort_var,
            ..
        } => {
            vec![flow_var.clone(), effort_var.clone()]
        }
        ConstitutiveRelation::Resistance {
            effort_in_var,
            effort_out_var,
            flow_var,
            ..
        } => {
            vec![
                effort_in_var.clone(),
                effort_out_var.clone(),
                flow_var.clone(),
            ]
        }
        ConstitutiveRelation::Conductance {
            effort_var,
            flow_var,
            ..
        } => {
            vec![effort_var.clone(), flow_var.clone()]
        }
        ConstitutiveRelation::EffortSource { effort_var, .. } => {
            vec![effort_var.clone()]
        }
        ConstitutiveRelation::FlowSource { flow_var, .. } => {
            vec![flow_var.clone()]
        }
        ConstitutiveRelation::Transformer {
            effort_in_var,
            effort_out_var,
            flow_in_var,
            flow_out_var,
            ..
        } => {
            vec![
                effort_in_var.clone(),
                effort_out_var.clone(),
                flow_in_var.clone(),
                flow_out_var.clone(),
            ]
        }
        ConstitutiveRelation::Gyrator {
            effort_in_var,
            effort_out_var,
            flow_in_var,
            flow_out_var,
            ..
        } => {
            vec![
                effort_in_var.clone(),
                effort_out_var.clone(),
                flow_in_var.clone(),
                flow_out_var.clone(),
            ]
        }
    }
}

// ---------------------------------------------------------------------------
// SCAP algorithm
// ---------------------------------------------------------------------------

/// Run the 6-step Sequential Causality Assignment Procedure.
///
/// Reference: Karnopp, Margolis & Rosenberg, "System Dynamics", Chapter 4.
///
/// For the MVP implementation:
/// 1. Classify each `ConstitutiveRelation` by `ElementType`.
/// 2. Assign `Fixed` to sources, `Integral` to C/I, `Unassigned` to R/G.
/// 3. Check for conflicts: if two C/I elements share a junction without an R
///    between them, flag derivative causality on one.
/// 4. Assign remaining R elements.
pub fn assign_causality(
    graph: &ConnectionGraph,
    constraints: &GeneratedConstraints,
) -> CausalityAssignment {
    let mut elements: HashMap<String, ElementCausality> = HashMap::new();
    let mut derivative_warnings: Vec<String> = Vec::new();
    let mut diagnostics: Vec<String> = Vec::new();

    // --- Step 1: Assign FIXED causality to all sources (Se, Sf) ---
    for rel in &constraints.constitutive {
        let (var, etype) = classify_relation(rel);
        if etype == ElementType::Source {
            elements.insert(
                var.clone(),
                ElementCausality {
                    element_var: var,
                    element_type: ElementType::Source,
                    causality: Causality::Fixed,
                },
            );
        }
    }

    // --- Step 2: Propagate through junctions (simplified) ---
    // In the MVP we don't do full junction propagation but record junction info
    // for conflict detection in step 6.

    // --- Step 3: Assign INTEGRAL causality to all C and I elements ---
    for rel in &constraints.constitutive {
        let (var, etype) = classify_relation(rel);
        if etype == ElementType::Storage {
            elements.insert(
                var.clone(),
                ElementCausality {
                    element_var: var,
                    element_type: ElementType::Storage,
                    causality: Causality::Integral,
                },
            );
        }
    }

    // --- Step 4: Propagate through junctions again (simplified — no-op in MVP) ---

    // --- Step 5: Assign causality to remaining R elements ---
    for rel in &constraints.constitutive {
        let (var, etype) = classify_relation(rel);
        if etype == ElementType::Resistive {
            elements.insert(
                var.clone(),
                ElementCausality {
                    element_var: var,
                    element_type: ElementType::Resistive,
                    causality: Causality::Unassigned,
                },
            );
        }
    }

    // TwoPort elements: record but leave Unassigned (propagation not yet impl)
    for rel in &constraints.constitutive {
        let (var, etype) = classify_relation(rel);
        if etype == ElementType::TwoPort && !elements.contains_key(&var) {
            elements.insert(
                var.clone(),
                ElementCausality {
                    element_var: var,
                    element_type: ElementType::TwoPort,
                    causality: Causality::Unassigned,
                },
            );
        }
    }

    // --- Step 6: Conflict detection ---
    // Check if any junction has >1 storage element connected directly.
    // Build a mapping from variable name to the set of junctions it participates in.
    let junction_storage_conflicts =
        detect_junction_storage_conflicts(graph, constraints, &elements);

    for (junction_desc, conflicting_vars) in &junction_storage_conflicts {
        // The first storage element keeps integral causality;
        // subsequent ones are forced to derivative.
        for var in conflicting_vars.iter().skip(1) {
            if let Some(elem) = elements.get_mut(var) {
                elem.causality = Causality::Derivative;
                derivative_warnings.push(var.clone());
                diagnostics.push(format!(
                    "Storage element '{}' forced to derivative causality at {}",
                    var, junction_desc,
                ));
            }
        }
    }

    let consistent = derivative_warnings.is_empty();

    CausalityAssignment {
        elements,
        derivative_causality_warnings: derivative_warnings,
        consistent,
        diagnostics,
    }
}

/// Detect junctions where multiple storage elements connect without an R between them.
///
/// Returns a list of `(junction_description, vec_of_conflicting_storage_vars)`.
fn detect_junction_storage_conflicts(
    graph: &ConnectionGraph,
    constraints: &GeneratedConstraints,
    elements: &HashMap<String, ElementCausality>,
) -> Vec<(String, Vec<String>)> {
    // Build a mapping: NodeId → set of variable names from constitutive relations.
    let mut node_vars: HashMap<NodeId, Vec<String>> = HashMap::new();
    for rel in &constraints.constitutive {
        let vars = relation_variables(rel);
        for var in &vars {
            // Try to find which node this variable belongs to by matching qualified_path prefix.
            for node in &graph.nodes {
                if var.starts_with(&node.qualified_path) {
                    node_vars.entry(node.id).or_default().push(var.clone());
                }
            }
        }
    }

    let mut conflicts = Vec::new();

    for junction in &graph.junctions {
        let mut storage_vars_at_junction: Vec<String> = Vec::new();
        let mut has_resistive = false;

        // Collect all node IDs at this junction.
        let junction_node_ids: Vec<NodeId> = junction
            .incoming
            .iter()
            .map(|(id, _)| *id)
            .chain(junction.outgoing.iter().map(|(id, _)| *id))
            .collect();

        for &nid in &junction_node_ids {
            if let Some(vars) = node_vars.get(&nid) {
                for var in vars {
                    if let Some(elem) = elements.get(var) {
                        match elem.element_type {
                            ElementType::Storage => {
                                if !storage_vars_at_junction.contains(var) {
                                    storage_vars_at_junction.push(var.clone());
                                }
                            }
                            ElementType::Resistive => {
                                has_resistive = true;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Conflict: >1 storage element without a resistive element at the same junction.
        if storage_vars_at_junction.len() > 1 && !has_resistive {
            let junction_desc = format!(
                "junction '{}' ({})",
                junction.owner,
                match junction.junction_type {
                    JunctionType::Zero => "0-junction",
                    JunctionType::One => "1-junction",
                },
            );
            conflicts.push((junction_desc, storage_vars_at_junction));
        }
    }

    conflicts
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::physics::connection::ConnectionGraph;
    use crate::physics::constraints::*;

    /// Helper: build an empty connection graph (no junctions to conflict).
    fn empty_graph() -> ConnectionGraph {
        ConnectionGraph {
            nodes: vec![],
            edges: vec![],
            junctions: vec![],
        }
    }

    /// Test 1: Se + R + C → all integral, consistent=true, no warnings.
    #[test]
    fn scap_rc_circuit_all_integral() {
        let mut gc = GeneratedConstraints::default();

        // Source: 10V
        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "source.voltage".to_string(),
            source_value: Some(10.0),
        });

        // Resistance: 5 ohm
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "source.voltage".to_string(),
            effort_out_var: "cap.voltage".to_string(),
            flow_var: "rc.current".to_string(),
            parameter_var: "r1.resistance".to_string(),
            parameter_value: Some(5.0),
        });

        // Capacitor: 1 Farad
        gc.constitutive.push(ConstitutiveRelation::Capacitance {
            effort_var: "cap.voltage".to_string(),
            flow_var: "rc.current".to_string(),
            parameter_var: "cap.capacitance".to_string(),
            parameter_value: Some(1.0),
        });

        let graph = empty_graph();
        let result = assign_causality(&graph, &gc);

        assert!(result.consistent, "RC circuit should be fully consistent");
        assert!(
            !result.has_derivative_causality(),
            "no derivative causality expected"
        );
        assert!(
            result.derivative_causality_warnings.is_empty(),
            "no warnings expected"
        );

        // Source should be Fixed
        let src = result.elements.get("source.voltage").unwrap();
        assert_eq!(src.causality, Causality::Fixed);
        assert_eq!(src.element_type, ElementType::Source);

        // Capacitor should be Integral
        let cap = result.elements.get("cap.voltage").unwrap();
        assert_eq!(cap.causality, Causality::Integral);
        assert_eq!(cap.element_type, ElementType::Storage);

        // Resistor should be Unassigned (no preference)
        let res = result.elements.get("rc.current").unwrap();
        assert_eq!(res.element_type, ElementType::Resistive);
    }

    /// Test 2: Se + R + I → all integral, consistent=true.
    #[test]
    fn scap_rl_circuit_all_integral() {
        let mut gc = GeneratedConstraints::default();

        // Source: 10V
        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "source.voltage".to_string(),
            source_value: Some(10.0),
        });

        // Resistance: 5 ohm
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "source.voltage".to_string(),
            effort_out_var: "ind.voltage".to_string(),
            flow_var: "rl.current".to_string(),
            parameter_var: "r1.resistance".to_string(),
            parameter_value: Some(5.0),
        });

        // Inductance: 0.1 Henry
        gc.constitutive.push(ConstitutiveRelation::Inductance {
            flow_var: "ind.current".to_string(),
            effort_var: "ind.voltage".to_string(),
            parameter_var: "ind.inductance".to_string(),
            parameter_value: Some(0.1),
        });

        let graph = empty_graph();
        let result = assign_causality(&graph, &gc);

        assert!(result.consistent, "RL circuit should be fully consistent");
        assert!(
            !result.has_derivative_causality(),
            "no derivative causality expected"
        );

        // Source should be Fixed
        let src = result.elements.get("source.voltage").unwrap();
        assert_eq!(src.causality, Causality::Fixed);

        // Inductor should be Integral (flow is state variable)
        let ind = result.elements.get("ind.current").unwrap();
        assert_eq!(ind.causality, Causality::Integral);
        assert_eq!(ind.element_type, ElementType::Storage);
    }

    /// Test 3: Se/Sf → Fixed causality.
    #[test]
    fn scap_sources_get_fixed() {
        let mut gc = GeneratedConstraints::default();

        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "battery.voltage".to_string(),
            source_value: Some(12.0),
        });

        gc.constitutive.push(ConstitutiveRelation::FlowSource {
            flow_var: "pump.flow".to_string(),
            source_value: Some(5.0),
        });

        let graph = empty_graph();
        let result = assign_causality(&graph, &gc);

        assert!(result.consistent);

        let se = result.elements.get("battery.voltage").unwrap();
        assert_eq!(se.causality, Causality::Fixed);
        assert_eq!(se.element_type, ElementType::Source);

        let sf = result.elements.get("pump.flow").unwrap();
        assert_eq!(sf.causality, Causality::Fixed);
        assert_eq!(sf.element_type, ElementType::Source);
    }

    /// Test 4: R-only circuit → consistent, R gets Unassigned (no preference).
    #[test]
    fn scap_r_only_no_preference() {
        let mut gc = GeneratedConstraints::default();

        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "a.voltage".to_string(),
            effort_out_var: "b.voltage".to_string(),
            flow_var: "r1.current".to_string(),
            parameter_var: "r1.resistance".to_string(),
            parameter_value: Some(10.0),
        });

        gc.constitutive.push(ConstitutiveRelation::Conductance {
            effort_var: "c.voltage".to_string(),
            flow_var: "g1.current".to_string(),
            parameter_var: "g1.conductance".to_string(),
            parameter_value: Some(0.1),
        });

        let graph = empty_graph();
        let result = assign_causality(&graph, &gc);

        assert!(result.consistent, "R-only should be consistent");
        assert!(
            !result.has_derivative_causality(),
            "no derivative warnings for R-only"
        );

        // Both R elements should be Unassigned (no causality preference)
        let r1 = result.elements.get("r1.current").unwrap();
        assert_eq!(r1.causality, Causality::Unassigned);
        assert_eq!(r1.element_type, ElementType::Resistive);

        let g1 = result.elements.get("g1.current").unwrap();
        assert_eq!(g1.causality, Causality::Unassigned);
        assert_eq!(g1.element_type, ElementType::Resistive);
    }
}
