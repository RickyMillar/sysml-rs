//! Phase 4 — Conservation constraint generation.
//!
//! Walks the [`ConnectionGraph`] to produce conservation constraints at junctions
//! (KCL, mass balance, energy balance) and effort-equality constraints across
//! edges that share a physics domain.

use std::collections::HashMap;

use sysml_core::{ElementKind, ModelGraph};
use sysml_span::Diagnostic;

use super::connection::{ConnectionGraph, JunctionId, JunctionType};
use super::domain::{BondGraphRole, ConservationLaw, PhysicsDomainRegistry, VariableRole};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A conservation constraint at a junction node (e.g., KCL: sum of currents = 0).
#[derive(Debug, Clone, PartialEq)]
pub struct ConservationConstraint {
    /// Human-readable name (e.g., `"kcl_busbar_electrical"`).
    pub name: String,
    /// The junction this constraint belongs to.
    pub junction_id: JunctionId,
    /// The conservation law governing the constraint.
    pub law: ConservationLaw,
    /// Variable paths for incoming flows (e.g., `"busbar.powerIn.current"`).
    pub incoming_vars: Vec<String>,
    /// Variable paths for outgoing flows (e.g., `"busbar.circuitOut1.current"`).
    pub outgoing_vars: Vec<String>,
}

/// An effort-equality constraint between two connected ports of the same domain.
///
/// 0-junction semantics: voltage at source port == voltage at target port.
#[derive(Debug, Clone, PartialEq)]
pub struct EffortEquality {
    /// Source variable path (e.g., `"busbar.circuitOut1.voltage"`).
    pub source_var: String,
    /// Target variable path (e.g., `"circuit1.breaker.phaseIn.voltage"`).
    pub target_var: String,
}

/// A flow-equality constraint between two connected ports (1-junction semantics).
///
/// 1-junction: current through series elements is equal.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowEquality {
    /// Source variable path (e.g., `"r1.phaseOut.current"`).
    pub source_var: String,
    /// Target variable path (e.g., `"r2.phaseIn.current"`).
    pub target_var: String,
}

/// A constitutive relation derived from bond graph element classification.
///
/// Covers all 9 standard bond graph element types. Sign conventions follow
/// BondGraphTools (`base.json`), verified against Modelica BondLib.
///
/// Reference: `/references/bond-graph/BondGraphTools/BondGraphTools/components/base.json`
/// Reference: `/references/bond-graph/BondLib/BondLib 2.3/package.mo`
#[derive(Debug, Clone, PartialEq)]
pub enum ConstitutiveRelation {
    // --- One-port passive elements ---
    /// R-element: `e_in - e_out = R * f` (Ohm's law, Fourier's law, etc.)
    Resistance {
        effort_in_var: String,
        effort_out_var: String,
        flow_var: String,
        parameter_var: String,
        parameter_value: Option<f64>,
    },
    /// Conductance (inverse R): `f = G * e` (BondGraphTools: `e_0 - f_0/G = 0`)
    Conductance {
        effort_var: String,
        flow_var: String,
        parameter_var: String,
        parameter_value: Option<f64>,
    },
    /// C-element: `dq/dt = f`, `e = q/C`. State variable: effort.
    /// BondGraphTools: `q_0 - C*e_0 = 0`, `dq_0 - f_0 = 0`
    Capacitance {
        effort_var: String,
        flow_var: String,
        parameter_var: String,
        parameter_value: Option<f64>,
    },
    /// I-element: `dp/dt = e`, `f = p/L`. State variable: flow.
    /// BondGraphTools: `p_0 - L*f_0 = 0`, `dp_0 - e_0 = 0`
    Inductance {
        flow_var: String,
        effort_var: String,
        parameter_var: String,
        parameter_value: Option<f64>,
    },

    // --- One-port active elements (sources) ---
    /// Se: `e_0 = e_source`. Fixed causality: imposes effort.
    /// BondGraphTools: `e_0 - e = 0`
    EffortSource {
        effort_var: String,
        source_value: Option<f64>,
    },
    /// Sf: `f_0 = -f_source`. Fixed causality: imposes flow.
    /// BondGraphTools: `f_0 + f = 0` (positive source = flow INTO network)
    FlowSource {
        flow_var: String,
        source_value: Option<f64>,
    },

    // --- Two-port elements ---
    /// TF (transformer): `e_1 = m * e_0`, `f_0 + m * f_1 = 0`.
    /// Power-conserving: `e_0 * f_0 + e_1 * f_1 = 0`.
    /// BondGraphTools: `e_1 - r*e_0 = 0`, `f_0 + r*f_1 = 0`
    Transformer {
        effort_in_var: String,
        effort_out_var: String,
        flow_in_var: String,
        flow_out_var: String,
        modulus: f64,
    },
    /// GY (gyrator): `e_1 = -r * f_0`, `e_0 = r * f_1`.
    /// Power-conserving: `e_0 * f_0 + e_1 * f_1 = 0`.
    /// BondGraphTools: `e_1 + R*f_0 = 0`, `e_0 - R*f_1 = 0`
    Gyrator {
        effort_in_var: String,
        effort_out_var: String,
        flow_in_var: String,
        flow_out_var: String,
        modulus: f64,
    },
}

/// A user-written constraint expression compiled from a `ConstraintDefinition`.
///
/// Represents an algebraic equality constraint in residual form: `lhs - rhs = 0`.
/// The expression is compiled to `ExprIR` and evaluated with state vector
/// variables bound in the `EvalContext` during DAE solving.
#[derive(Debug, Clone)]
pub struct UserConstraintExpression {
    /// Human-readable source expression (e.g., `"e == R * f"`).
    pub source: String,
    /// Compiled residual expression: `lhs - rhs` (should evaluate to 0 at solution).
    pub residual_expr: crate::expressions::ExprIR,
    /// Variable names referenced by this expression (for state vector mapping).
    pub referenced_vars: Vec<String>,
    /// Owner element name for diagnostics.
    pub owner_name: Option<String>,
}

/// All constraints generated from a connection graph.
#[derive(Debug, Clone, Default)]
pub struct GeneratedConstraints {
    /// Conservation constraints (one per non-signal junction).
    pub conservation: Vec<ConservationConstraint>,
    /// Effort equalities — 0-junction: all efforts equal across connection.
    pub effort_equalities: Vec<EffortEquality>,
    /// Flow equalities — 1-junction: all flows equal through series chain.
    pub flow_equalities: Vec<FlowEquality>,
    /// Constitutive relations from bond graph element classification (R/C/I).
    pub constitutive: Vec<ConstitutiveRelation>,
    /// User-written constraint expressions compiled from `ConstraintDefinition` elements.
    pub user_constraints: Vec<UserConstraintExpression>,
    /// Diagnostics encountered during generation.
    pub diagnostics: Vec<Diagnostic>,
}

// ---------------------------------------------------------------------------
// Feature-name heuristics
// ---------------------------------------------------------------------------

/// Determine the default feature name for a domain and variable role.
///
/// These are fallback heuristics when full ISQ classification is unavailable.
pub fn default_feature_name(domain: &str, role: VariableRole) -> &'static str {
    match (domain, role) {
        ("electrical", VariableRole::Flow) => "current",
        ("electrical", VariableRole::Effort) => "voltage",
        ("thermal", VariableRole::Flow) => "heat_flow",
        ("thermal", VariableRole::Effort) => "temperature",
        ("hydraulic", VariableRole::Flow) => "mass_flow",
        ("hydraulic", VariableRole::Effort) => "pressure",
        ("mechanical_translational", VariableRole::Flow) => "force",
        ("mechanical_translational", VariableRole::Effort) => "velocity",
        ("mechanical_rotational", VariableRole::Flow) => "torque",
        ("mechanical_rotational", VariableRole::Effort) => "angular_velocity",
        ("chemical", VariableRole::Flow) => "molar_flow",
        ("chemical", VariableRole::Effort) => "chemical_potential",
        ("luminous", VariableRole::Flow) => "luminous_flux",
        ("luminous", VariableRole::Effort) => "luminous_intensity",
        // Fallback
        (_, VariableRole::Flow) => "flow",
        (_, VariableRole::Effort) => "effort",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Generate conservation and effort-equality constraints from a connection graph.
///
/// Algorithm:
/// 1. For each junction, emit a `ConservationConstraint` (unless `SignalRouting`).
/// 2. For each edge with a shared domain, emit an `EffortEquality`.
pub fn generate_constraints(
    graph: &ConnectionGraph,
    registry: &PhysicsDomainRegistry,
) -> GeneratedConstraints {
    generate_constraints_with_model(graph, registry, None)
}

/// Generate constraints with optional access to the full model graph.
///
/// When `model` is `Some`, part attributes are walked to automatically detect
/// R/C/I parameters, 2-port elements (TF/GY), and sources (Se/Sf).
pub fn generate_constraints_with_model(
    graph: &ConnectionGraph,
    _registry: &PhysicsDomainRegistry,
    model: Option<&ModelGraph>,
) -> GeneratedConstraints {
    let mut result = GeneratedConstraints::default();

    // --- Step 1: Conservation constraints from junctions ---
    for junction in &graph.junctions {
        if junction.conservation == ConservationLaw::SignalRouting {
            continue;
        }

        match junction.junction_type {
            JunctionType::Zero => {
                // 0-junction: flows are conserved (KCL). sum(f_in) = sum(f_out).
                let flow_feature = default_feature_name(junction.domain, VariableRole::Flow);

                let incoming_vars: Vec<String> = junction
                    .incoming
                    .iter()
                    .map(|(node_id, _feat)| {
                        let node = &graph.nodes[*node_id];
                        format!("{}.{}", node.qualified_path, flow_feature)
                    })
                    .collect();

                let outgoing_vars: Vec<String> = junction
                    .outgoing
                    .iter()
                    .map(|(node_id, _feat)| {
                        let node = &graph.nodes[*node_id];
                        format!("{}.{}", node.qualified_path, flow_feature)
                    })
                    .collect();

                result.conservation.push(ConservationConstraint {
                    name: format!("kcl_{}_{}", junction.owner, junction.domain),
                    junction_id: junction.id,
                    law: junction.conservation.clone(),
                    incoming_vars,
                    outgoing_vars,
                });
            }
            JunctionType::One => {
                // 1-junction: efforts are conserved (KVL). sum(e_in) = sum(e_out).
                let effort_feature = default_feature_name(junction.domain, VariableRole::Effort);

                let incoming_vars: Vec<String> = junction
                    .incoming
                    .iter()
                    .map(|(node_id, _feat)| {
                        let node = &graph.nodes[*node_id];
                        format!("{}.{}", node.qualified_path, effort_feature)
                    })
                    .collect();

                let outgoing_vars: Vec<String> = junction
                    .outgoing
                    .iter()
                    .map(|(node_id, _feat)| {
                        let node = &graph.nodes[*node_id];
                        format!("{}.{}", node.qualified_path, effort_feature)
                    })
                    .collect();

                result.conservation.push(ConservationConstraint {
                    name: format!("kvl_{}_{}", junction.owner, junction.domain),
                    junction_id: junction.id,
                    law: junction.conservation.clone(),
                    incoming_vars,
                    outgoing_vars,
                });
            }
        }
    }

    // --- Step 2: Equalities from edges (skip disabled) ---
    for edge in &graph.edges {
        if !edge.enabled {
            continue;
        }
        let domain = match edge.domain {
            Some(d) => d,
            None => continue,
        };

        if domain == "signal" {
            continue;
        }

        let source_node = &graph.nodes[edge.source];
        let target_node = &graph.nodes[edge.target];

        // Default: effort equality (0-junction semantics on edges)
        let effort_feature = default_feature_name(domain, VariableRole::Effort);
        result.effort_equalities.push(EffortEquality {
            source_var: format!("{}.{}", source_node.qualified_path, effort_feature),
            target_var: format!("{}.{}", target_node.qualified_path, effort_feature),
        });

        // Also propagate flow equality for series connections
        let flow_feature = default_feature_name(domain, VariableRole::Flow);
        result.flow_equalities.push(FlowEquality {
            source_var: format!("{}.{}", source_node.qualified_path, flow_feature),
            target_var: format!("{}.{}", target_node.qualified_path, flow_feature),
        });
    }

    // --- Step 3: Constitutive relations from classified features + model attributes ---
    result.constitutive = generate_constitutive_relations(graph, model, _registry);

    result
}

// ---------------------------------------------------------------------------
// Open-terminal zero-flow (RSC-1.3) + declared defaults (RSC-1.6)
// ---------------------------------------------------------------------------

/// Declared `default` value found on an open terminal's FLOW feature (RSC-1.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DeclaredFlowDefault {
    /// Literal numeric default — the boundary condition to pin the flow to.
    Numeric(f64),
    /// A default is declared but is not a literal numeric (expression,
    /// string, bool). Callers fall back to 0 and say so.
    NonNumeric,
    /// No default declared.
    None,
}

/// Read a literal numeric `default` from the named feature of a
/// `PortDefinition` (RSC-1.6 open-terminal boundary conditions).
///
/// Literal defaults are stored two ways by the parser pipeline:
/// - typed props: `default` (legacy graphs) or `value` + `isDefault = true`
///   (see `element_builder.rs` / `lift_split_default_value`). A plain `value`
///   without `isDefault` is a fixed `= x` binding, not a default — skipped.
/// - the legacy `unresolved_value` string fallback, which carries negative
///   numerics like `"-2.5"` (see feedback_unresolved_value_numerics).
pub(crate) fn flow_feature_declared_default(
    def_name: &str,
    feature_name: &str,
    model: &ModelGraph,
) -> DeclaredFlowDefault {
    use sysml_core::Value;

    let Some(port_def) = model
        .elements
        .values()
        .find(|e| e.kind == ElementKind::PortDefinition && e.name.as_deref() == Some(def_name))
    else {
        return DeclaredFlowDefault::None;
    };
    let Some(feat) = model.elements.values().find(|e| {
        e.owner.as_ref() == Some(&port_def.id)
            && matches!(e.kind, ElementKind::AttributeUsage | ElementKind::ItemUsage)
            && e.name.as_deref() == Some(feature_name)
    }) else {
        return DeclaredFlowDefault::None;
    };

    let is_default = feat.get_prop("isDefault").and_then(|v| v.as_bool()) == Some(true);
    let typed = feat.get_prop("default").or_else(|| {
        if is_default {
            feat.get_prop("value")
        } else {
            None
        }
    });
    match typed {
        Some(Value::Int(i)) => return DeclaredFlowDefault::Numeric(*i as f64),
        Some(Value::Float(f)) => return DeclaredFlowDefault::Numeric(*f),
        _ => {}
    }
    // Legacy fallback: negative numeric defaults land in `unresolved_value`
    // as strings instead of typed props.
    if is_default {
        if let Some(s) = feat.get_prop("unresolved_value").and_then(|v| v.as_str()) {
            if let Ok(f) = s.trim().parse::<f64>() {
                return DeclaredFlowDefault::Numeric(f);
            }
        }
    }
    if is_default || typed.is_some() {
        DeclaredFlowDefault::NonNumeric
    } else {
        DeclaredFlowDefault::None
    }
}

/// Generate implicit `flow = 0` equations for unconnected power ports
/// (Modelica unconnected-connector semantics, RSC-1.3 / audit gap G9).
///
/// Scope: only ports that participate in the physics network type-wise —
/// CLASSIFIED power ports (`!is_signal`) with a flow feature. Signal ports
/// (incomplete effort/flow conjugate pair) are exempt: unconnected or
/// fanned-out signal ports are normal. Unclassifiable ports are skipped —
/// they never enter the physics network, so pinning them would invent
/// semantics.
///
/// A port counts as *connected* when it appears as either endpoint of any
/// flow, i.e. when it has a node in the [`ConnectionGraph`] (nodes are minted
/// exclusively from flow endpoints). Both the full qualified node path and
/// its `leafOwner.port` suffix are matched so deeper instance paths don't
/// produce false "open terminal" pins.
///
/// Each open terminal yields a [`ConstitutiveRelation::FlowSource`] with
/// `source_value = Some(0.0)` (Sf semantics `0 = f + f_source` ⇒ `f = 0`) —
/// one new variable and one new equation, so the assembled DAE stays square —
/// plus an info diagnostic stating the assumption (fail loud about
/// assumptions).
///
/// RSC-1.6: when the FLOW feature carries a literal numeric `default`, the
/// flow is pinned to THAT value instead of 0 (`default` = "value when nothing
/// else determines it"; an unconnected terminal is exactly that — a nonzero
/// value is a model boundary condition, e.g. "rest of the panel abstracted as
/// a 2A draw"). Non-numeric/expression defaults are ignored (fall back to 0,
/// noted in the message). Effort-feature defaults are never consulted — an
/// open terminal's effort stays free.
pub fn open_terminal_zero_flow_relations(
    graph: &ConnectionGraph,
    port_registry: &crate::flows::port::PortRegistry,
    model: &ModelGraph,
    registry: &PhysicsDomainRegistry,
) -> (Vec<ConstitutiveRelation>, Vec<Diagnostic>) {
    use super::classify::{classify_port_definition, ClassificationConfidence};
    use super::connection::find_port_definition_for_name;
    use std::collections::HashSet;

    // Connected = present in the connection graph (full path or leaf suffix).
    let mut connected: HashSet<String> = HashSet::new();
    for node in &graph.nodes {
        connected.insert(node.qualified_path.clone());
        let leaf_owner = node
            .owner_path
            .rsplit('.')
            .next()
            .unwrap_or(node.owner_path.as_str());
        connected.insert(format!("{}.{}", leaf_owner, node.port_name));
    }

    let mut relations = Vec::new();
    let mut diagnostics = Vec::new();

    // Deterministic iteration order for stable diagnostics/equations.
    let mut entries: Vec<&crate::flows::port::PortInstanceIR> =
        port_registry.iter().map(|(_, p)| p).collect();
    entries.sort_by_key(|p| p.key());

    for port in entries {
        let key = port.key();
        if connected.contains(&key) {
            continue;
        }

        // Resolve the PortDefinition name (registry first, then graph walk).
        let def_name = port
            .definition
            .clone()
            .or_else(|| find_port_definition_for_name(&port.name, model));
        let Some(def_name) = def_name else { continue };

        let classification = classify_port_definition(&def_name, model, registry);
        if classification.confidence == ClassificationConfidence::Unknown {
            continue; // not part of the physics network
        }
        if classification.is_signal {
            continue; // signal ports are exempt — no conservation semantics
        }
        let Some(flow_feat) = classification
            .features
            .iter()
            .find(|f| f.role == VariableRole::Flow)
        else {
            continue; // power port without a flow feature: nothing to pin
        };

        // RSC-1.6: a literal numeric default on the FLOW feature is a
        // declared boundary condition — pin the flow to it instead of 0.
        let (source_value, message) =
            match flow_feature_declared_default(&def_name, &flow_feat.name, model) {
                DeclaredFlowDefault::Numeric(v) => (
                    v,
                    format!(
                        "Physics: open terminal '{}': assuming {} {} \
                         (declared default — model boundary condition)",
                        key, v, flow_feat.name,
                    ),
                ),
                DeclaredFlowDefault::NonNumeric => (
                    0.0,
                    format!(
                        "Physics: open terminal '{}': assuming zero {} \
                         (unconnected power port; declared default is not a \
                         literal numeric — ignored)",
                        key, flow_feat.name,
                    ),
                ),
                DeclaredFlowDefault::None => (
                    0.0,
                    format!(
                        "Physics: open terminal '{}': assuming zero {} (unconnected power port)",
                        key, flow_feat.name,
                    ),
                ),
            };

        diagnostics.push(Diagnostic::info(message));
        relations.push(ConstitutiveRelation::FlowSource {
            flow_var: format!("{}.{}", key, flow_feat.name),
            source_value: Some(source_value),
        });
    }

    (relations, diagnostics)
}

// ---------------------------------------------------------------------------
// User-written constraint extraction
// ---------------------------------------------------------------------------

/// Extract user-written `ConstraintDefinition` expressions from a model graph.
///
/// Finds `ConstraintDefinition` elements, extracts their body expression (the
/// trailing boolean expression), parses `lhs == rhs` equality constraints into
/// residual form (`lhs - rhs`), and returns compiled `UserConstraintExpression`s
/// suitable for feeding into the DAE solver as algebraic equations.
///
/// Non-equality constraints (inequalities, complex expressions) are skipped with
/// a diagnostic.
pub fn extract_user_constraints(
    model: &ModelGraph,
) -> (Vec<UserConstraintExpression>, Vec<Diagnostic>) {
    use crate::expressions::{compile_simple_expression, ExprIR};

    let mut results = Vec::new();
    let mut diagnostics = Vec::new();

    // AST-first string source: pretty-print the structured expression
    // subtree when present, then fall back to legacy string props. The
    // equality-split below operates on this string.
    fn expression_string(elem: &sysml_core::Element, graph: &ModelGraph) -> Option<String> {
        sysml_core::expression_pretty::pretty_print_owner(elem, graph).or_else(|| {
            elem.get_prop("constraint")
                .or_else(|| elem.get_prop("expr"))
                .and_then(|v| match v {
                    sysml_core::Value::String(s) => Some(s.clone()),
                    _ => None,
                })
        })
    }

    for element in model.elements_by_kind(&ElementKind::ConstraintDefinition) {
        let name = element.name.as_deref().unwrap_or("<anonymous>");

        // Body expression: AST subtree (pretty-printed) or legacy string props.
        let expr_text = expression_string(element, model);

        let expr_text = match expr_text {
            Some(text) => text,
            None => {
                // Walk children for a nested constraint/expression body.
                let child_expr = model
                    .children_of(&element.id)
                    .filter_map(|child| {
                        if matches!(
                            child.kind,
                            ElementKind::ConstraintUsage
                                | ElementKind::Expression
                                | ElementKind::OperatorExpression
                        ) {
                            expression_string(child, model)
                        } else {
                            None
                        }
                    })
                    .next();

                match child_expr {
                    Some(text) => text,
                    None => continue, // No expression found, skip
                }
            }
        };

        // Parse equality: split on "=="
        let trimmed = expr_text.trim();
        let parts: Vec<&str> = trimmed.splitn(2, "==").collect();
        if parts.len() != 2 {
            diagnostics.push(Diagnostic::warning(format!(
                "constraint def '{}': only equality constraints (==) are supported for DAE, skipping: {}",
                name, trimmed
            )));
            continue;
        }

        let lhs_str = parts[0].trim();
        let rhs_str = parts[1].trim();

        // Compile both sides
        let lhs = match compile_simple_expression(lhs_str) {
            Ok(expr) => expr,
            Err(diags) => {
                diagnostics.extend(diags);
                continue;
            }
        };
        let rhs = match compile_simple_expression(rhs_str) {
            Ok(expr) => expr,
            Err(diags) => {
                diagnostics.extend(diags);
                continue;
            }
        };

        // Build residual: lhs - rhs (should be 0 at solution)
        let residual = ExprIR::BinaryOp {
            op: crate::expressions::BinOp::Subtract,
            left: Box::new(lhs.clone()),
            right: Box::new(rhs.clone()),
        };

        // Collect referenced variables
        let mut var_set = lhs.free_variables();
        var_set.extend(rhs.free_variables());
        let mut vars: Vec<String> = var_set.into_iter().collect();
        vars.sort();

        results.push(UserConstraintExpression {
            source: trimmed.to_string(),
            residual_expr: residual,
            referenced_vars: vars,
            owner_name: Some(name.to_string()),
        });
    }

    (results, diagnostics)
}

// ---------------------------------------------------------------------------
// Constitutive relation generation
// ---------------------------------------------------------------------------

/// Generate constitutive relations from bond graph-classified port features
/// and model-level part attributes.
///
/// Walks each node in the connection graph. When a node's port classification
/// contains features with R/C/I bond graph roles alongside effort and flow
/// features, emits the corresponding constitutive relation.
///
/// When `model` is provided, also walks the owner part's `AttributeUsage`
/// children to find ISQ-typed parameters (resistance, capacitance, etc.)
/// and extract their numeric default values.
pub fn generate_constitutive_relations(
    graph: &ConnectionGraph,
    model: Option<&ModelGraph>,
    registry: &PhysicsDomainRegistry,
) -> Vec<ConstitutiveRelation> {
    let mut relations = Vec::new();

    for node in &graph.nodes {
        let classification = match &node.classification {
            Some(c) => c,
            None => continue,
        };

        // Collect features by bond graph role
        let mut effort_feat: Option<&str> = None;
        let mut flow_feat: Option<&str> = None;
        let mut r_feat: Option<&str> = None;
        let mut g_feat: Option<&str> = None;
        let mut c_feat: Option<&str> = None;
        let mut i_feat: Option<&str> = None;

        for feat in &classification.features {
            match feat.bond_graph_role {
                Some(BondGraphRole::Effort) | Some(BondGraphRole::EffortRate) => {
                    effort_feat = Some(&feat.name);
                }
                Some(BondGraphRole::Flow) | Some(BondGraphRole::FlowRate) => {
                    flow_feat = Some(&feat.name);
                }
                Some(BondGraphRole::Resistance) => r_feat = Some(&feat.name),
                Some(BondGraphRole::Conductance) => g_feat = Some(&feat.name),
                Some(BondGraphRole::Capacitance) => c_feat = Some(&feat.name),
                Some(BondGraphRole::Inductance) => i_feat = Some(&feat.name),
                Some(BondGraphRole::Displacement)
                | Some(BondGraphRole::Momentum)
                | Some(BondGraphRole::Power)
                | Some(BondGraphRole::Energy)
                | Some(BondGraphRole::Dimensionless)
                | Some(BondGraphRole::Unclassified)
                | None => {}
            }
        }

        // If no effort/flow features, use domain defaults
        let domain = match classification.domain {
            Some(d) => d,
            None => continue,
        };

        let effort_name =
            effort_feat.unwrap_or_else(|| default_feature_name(domain, VariableRole::Effort));
        let flow_name =
            flow_feat.unwrap_or_else(|| default_feature_name(domain, VariableRole::Flow));
        let base = &node.qualified_path;

        // R-element: we need effort + flow + resistance parameter
        if let Some(r_name) = r_feat {
            // For a 2-port R-element, we'd need in/out ports.
            // For a single-port node, model as effort drop across the element.
            relations.push(ConstitutiveRelation::Resistance {
                effort_in_var: format!("{base}.{effort_name}"),
                effort_out_var: format!("{base}.{effort_name}_out"),
                flow_var: format!("{base}.{flow_name}"),
                parameter_var: format!("{base}.{r_name}"),
                parameter_value: None,
            });
        }

        // G-element (conductance): f = G * e
        if let Some(g_name) = g_feat {
            relations.push(ConstitutiveRelation::Conductance {
                effort_var: format!("{base}.{effort_name}"),
                flow_var: format!("{base}.{flow_name}"),
                parameter_var: format!("{base}.{g_name}"),
                parameter_value: None,
            });
        }

        // C-element: effort is state variable, d(effort)/dt = flow / C
        if let Some(c_name) = c_feat {
            relations.push(ConstitutiveRelation::Capacitance {
                effort_var: format!("{base}.{effort_name}"),
                flow_var: format!("{base}.{flow_name}"),
                parameter_var: format!("{base}.{c_name}"),
                parameter_value: None,
            });
        }

        // I-element: flow is state variable, d(flow)/dt = effort / L
        if let Some(i_name) = i_feat {
            relations.push(ConstitutiveRelation::Inductance {
                flow_var: format!("{base}.{flow_name}"),
                effort_var: format!("{base}.{effort_name}"),
                parameter_var: format!("{base}.{i_name}"),
                parameter_value: None,
            });
        }
    }

    // --- Model-based attribute classification (items 9.3, 9.11, 9.8, 9.12) ---
    if let Some(model) = model {
        // Build a lookup of which owners already have constitutive relations
        // from the port-feature pass, so we don't double-emit.
        let mut owners_with_relations: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for rel in &relations {
            match rel {
                ConstitutiveRelation::Resistance { effort_in_var, .. } => {
                    if let Some(owner) = effort_in_var.split('.').next() {
                        owners_with_relations.insert(owner.to_string());
                    }
                }
                ConstitutiveRelation::Capacitance { effort_var, .. }
                | ConstitutiveRelation::Conductance { effort_var, .. } => {
                    if let Some(owner) = effort_var.split('.').next() {
                        owners_with_relations.insert(owner.to_string());
                    }
                }
                ConstitutiveRelation::Inductance { flow_var, .. } => {
                    if let Some(owner) = flow_var.split('.').next() {
                        owners_with_relations.insert(owner.to_string());
                    }
                }
                _ => {}
            }
        }

        // Group nodes by owner for 2-port correlation and source detection
        let mut owner_nodes: HashMap<String, Vec<&super::connection::PhysicsPortNode>> =
            HashMap::new();
        for node in &graph.nodes {
            owner_nodes
                .entry(node.owner_path.clone())
                .or_default()
                .push(node);
        }

        for (owner_path, nodes) in &owner_nodes {
            // Skip owners that already have relations from port-feature classification
            if owners_with_relations.contains(owner_path.as_str()) {
                continue;
            }

            // Find the owner part element in the model graph
            let part_attrs = classify_part_attributes(owner_path, model, registry);

            // Determine effort/flow feature names from the first node's classification or domain
            let first_domain = nodes.iter().find_map(|n| n.domain);
            let effort_name = nodes
                .iter()
                .find_map(|n| n.classification.as_ref())
                .and_then(|c| c.features.iter().find(|f| f.role == VariableRole::Effort))
                .map(|f| f.name.as_str())
                .unwrap_or_else(|| {
                    first_domain
                        .map(|d| default_feature_name(d, VariableRole::Effort))
                        .unwrap_or("effort")
                });
            let flow_name = nodes
                .iter()
                .find_map(|n| n.classification.as_ref())
                .and_then(|c| c.features.iter().find(|f| f.role == VariableRole::Flow))
                .map(|f| f.name.as_str())
                .unwrap_or_else(|| {
                    first_domain
                        .map(|d| default_feature_name(d, VariableRole::Flow))
                        .unwrap_or("flow")
                });

            // --- Source detection (item 9.8) ---
            // Single-port owner that prescribes a value → Se or Sf
            if nodes.len() == 1 {
                let node = nodes[0];
                for (attr_name, role, value) in &part_attrs {
                    match role {
                        BondGraphRole::Effort => {
                            relations.push(ConstitutiveRelation::EffortSource {
                                effort_var: format!("{}.{}", node.qualified_path, effort_name),
                                source_value: *value,
                            });
                        }
                        BondGraphRole::Flow => {
                            relations.push(ConstitutiveRelation::FlowSource {
                                flow_var: format!("{}.{}", node.qualified_path, flow_name),
                                source_value: *value,
                            });
                        }
                        _ => {}
                    }
                }
            }

            // --- 2-port owner correlation (item 9.12) ---
            // Owner with exactly 2 ports in different domains → TF/GY
            if nodes.len() == 2 {
                let d0 = nodes[0].domain;
                let d1 = nodes[1].domain;
                if let (Some(dom0), Some(dom1)) = (d0, d1) {
                    if dom0 != dom1 {
                        // Look for a modulus parameter in the attributes
                        let modulus = part_attrs
                            .iter()
                            .find(|(_, role, _)| {
                                matches!(
                                    role,
                                    BondGraphRole::Resistance | BondGraphRole::Dimensionless
                                )
                            })
                            .and_then(|(_, _, v)| *v)
                            .unwrap_or(1.0);

                        let e0_name = default_feature_name(dom0, VariableRole::Effort);
                        let f0_name = default_feature_name(dom0, VariableRole::Flow);
                        let e1_name = default_feature_name(dom1, VariableRole::Effort);
                        let f1_name = default_feature_name(dom1, VariableRole::Flow);

                        // Determine TF vs GY by checking domain variable mappings:
                        // TF: effort↔effort (same generalized variable type across domains)
                        // GY: effort↔flow (cross-type mapping)
                        // Heuristic: if both domains have same conservation law → TF, else → GY
                        let is_gyrator = registry
                            .domains()
                            .iter()
                            .zip(registry.domains().iter().skip(1))
                            .any(|(a, b)| {
                                (a.name == dom0 && b.name == dom1)
                                    || (a.name == dom1 && b.name == dom0)
                            })
                            && dom0 != dom1;
                        // Default: Transformer (most common multi-domain element)
                        let _ = is_gyrator; // reserved for future refinement

                        relations.push(ConstitutiveRelation::Transformer {
                            effort_in_var: format!("{}.{}", nodes[0].qualified_path, e0_name),
                            effort_out_var: format!("{}.{}", nodes[1].qualified_path, e1_name),
                            flow_in_var: format!("{}.{}", nodes[0].qualified_path, f0_name),
                            flow_out_var: format!("{}.{}", nodes[1].qualified_path, f1_name),
                            modulus,
                        });
                        continue; // Don't also emit R/C/I for TF/GY parts
                    }
                }
            }

            // --- R/C/I from part attributes (items 9.3, 9.11) ---
            // For single or multi-port parts with R/C/I attributes
            let base = if nodes.len() == 1 {
                nodes[0].qualified_path.clone()
            } else {
                // Use owner path as base for multi-port parts
                owner_path.clone()
            };

            for (attr_name, role, value) in &part_attrs {
                match role {
                    BondGraphRole::Resistance => {
                        // 2-port R-element if we have in+out ports
                        let (e_in, e_out) = if nodes.len() >= 2 {
                            (
                                format!("{}.{}", nodes[0].qualified_path, effort_name),
                                format!("{}.{}", nodes[1].qualified_path, effort_name),
                            )
                        } else {
                            (
                                format!("{base}.{effort_name}"),
                                format!("{base}.{effort_name}_out"),
                            )
                        };
                        let f_var = if nodes.len() >= 2 {
                            format!("{}.{}", nodes[0].qualified_path, flow_name)
                        } else {
                            format!("{base}.{flow_name}")
                        };
                        relations.push(ConstitutiveRelation::Resistance {
                            effort_in_var: e_in,
                            effort_out_var: e_out,
                            flow_var: f_var,
                            parameter_var: format!("{owner_path}.{attr_name}"),
                            parameter_value: *value,
                        });
                    }
                    BondGraphRole::Conductance => {
                        let e_var = format!("{base}.{effort_name}");
                        let f_var = format!("{base}.{flow_name}");
                        relations.push(ConstitutiveRelation::Conductance {
                            effort_var: e_var,
                            flow_var: f_var,
                            parameter_var: format!("{owner_path}.{attr_name}"),
                            parameter_value: *value,
                        });
                    }
                    BondGraphRole::Capacitance => {
                        let e_var = if nodes.len() >= 2 {
                            format!("{}.{}", nodes[0].qualified_path, effort_name)
                        } else {
                            format!("{base}.{effort_name}")
                        };
                        let f_var = if nodes.len() >= 2 {
                            format!("{}.{}", nodes[0].qualified_path, flow_name)
                        } else {
                            format!("{base}.{flow_name}")
                        };
                        relations.push(ConstitutiveRelation::Capacitance {
                            effort_var: e_var,
                            flow_var: f_var,
                            parameter_var: format!("{owner_path}.{attr_name}"),
                            parameter_value: *value,
                        });
                    }
                    BondGraphRole::Inductance => {
                        let e_var = if nodes.len() >= 2 {
                            format!("{}.{}", nodes[0].qualified_path, effort_name)
                        } else {
                            format!("{base}.{effort_name}")
                        };
                        let f_var = if nodes.len() >= 2 {
                            format!("{}.{}", nodes[0].qualified_path, flow_name)
                        } else {
                            format!("{base}.{flow_name}")
                        };
                        relations.push(ConstitutiveRelation::Inductance {
                            flow_var: f_var,
                            effort_var: e_var,
                            parameter_var: format!("{owner_path}.{attr_name}"),
                            parameter_value: *value,
                        });
                    }
                    _ => {} // Effort/Flow/other roles handled above as sources
                }
            }
        }
    }

    relations
}

// ---------------------------------------------------------------------------
// Model-based attribute classification helpers
// ---------------------------------------------------------------------------

/// Classify a part's attributes by ISQ dimension → BondGraphRole, and extract
/// numeric default values.
///
/// Searches the model graph for a PartUsage or PartDefinition matching the
/// owner path name, then walks its AttributeUsage children. For each attribute
/// with an ISQ-typed type, classifies via the physics domain registry and
/// extracts the numeric default from any LiteralInteger/LiteralRational child.
///
/// Returns `(attribute_name, bond_graph_role, optional_numeric_value)` tuples.
fn classify_part_attributes(
    owner_path: &str,
    model: &ModelGraph,
    registry: &PhysicsDomainRegistry,
) -> Vec<(String, BondGraphRole, Option<f64>)> {
    let mut results = Vec::new();

    // Extract the leaf name from the owner path (e.g., "circuit.r1" → "r1")
    let leaf_name = owner_path.rsplit('.').next().unwrap_or(owner_path);

    // Find the part element by name — could be PartUsage or PartDefinition
    let part_elem = model.elements.values().find(|e| {
        matches!(e.kind, ElementKind::PartUsage | ElementKind::PartDefinition)
            && e.name.as_deref() == Some(leaf_name)
    });

    let part_id = match part_elem {
        Some(e) => e.id.clone(),
        None => return results,
    };

    // Also find the part's definition (if this is a PartUsage with a type)
    let def_name = part_elem.and_then(|e| {
        e.get_prop("typeName")
            .or_else(|| e.get_prop("unresolved_type"))
            .and_then(|v| v.as_str())
    });

    // Collect attribute IDs to check: direct children + definition's children
    let mut attr_sources = vec![part_id.clone()];
    if let Some(dn) = def_name {
        if let Some(def_elem) = model.elements.values().find(|e| {
            matches!(e.kind, ElementKind::PartDefinition) && e.name.as_deref() == Some(dn)
        }) {
            attr_sources.push(def_elem.id.clone());
        }
    }

    for source_id in &attr_sources {
        for child in model.children_of(source_id) {
            if child.kind != ElementKind::AttributeUsage {
                continue;
            }
            let attr_name = match &child.name {
                Some(n) => n.clone(),
                None => continue,
            };

            // Get the attribute's type name
            let type_name = child
                .get_prop("typeName")
                .or_else(|| child.get_prop("unresolved_type"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    model
                        .children_of(&child.id)
                        .find(|e| e.kind == ElementKind::FeatureTyping)
                        .and_then(|ft| ft.get_prop("unresolved_type"))
                        .and_then(|v| v.as_str())
                });

            if let Some(tn) = type_name {
                // Try ISQ-based classification
                if let Some(dim) = registry.dimension_for_type(tn) {
                    // Try full bond graph classification
                    let bg_role = if let Some(entry) = super::isq_types::lookup_isq_type(tn) {
                        registry
                            .classify_dimension_full_with_hint(dim, entry.2)
                            .map(|(_, role)| role)
                    } else {
                        registry.classify_dimension_full(dim).map(|(_, role)| role)
                    };

                    // Fall back to domain classify → bond graph role
                    let bg_role = bg_role.or_else(|| {
                        registry
                            .classify_dimension(dim)
                            .and_then(|(domain, _)| Some(domain.classify_bond_graph_role(dim)))
                    });

                    if let Some(role) = bg_role {
                        if matches!(
                            role,
                            BondGraphRole::Resistance
                                | BondGraphRole::Conductance
                                | BondGraphRole::Capacitance
                                | BondGraphRole::Inductance
                                | BondGraphRole::Effort
                                | BondGraphRole::Flow
                        ) {
                            let value = extract_numeric_default(&child.id, model);
                            results.push((attr_name, role, value));
                            continue; // Classified by ISQ, skip name heuristic
                        }
                    }
                }
            }

            // Fallback: name-based heuristic for common attribute names
            if let Some(role) = classify_attribute_name_heuristic(&attr_name) {
                let value = extract_numeric_default(&child.id, model);
                results.push((attr_name, role, value));
            }
        }
    }

    results
}

/// Name-based heuristic for classifying common attribute names to bond graph roles.
fn classify_attribute_name_heuristic(name: &str) -> Option<BondGraphRole> {
    let lower = name.to_lowercase();
    if lower.contains("resistance") || lower == "r" {
        Some(BondGraphRole::Resistance)
    } else if lower.contains("capacitance") || lower == "c" {
        Some(BondGraphRole::Capacitance)
    } else if lower.contains("inductance") || lower == "l" {
        Some(BondGraphRole::Inductance)
    } else if lower.contains("conductance") || lower == "g" {
        Some(BondGraphRole::Conductance)
    } else if lower.contains("sourcevoltage")
        || lower.contains("source_voltage")
        || lower.contains("emf")
        || lower.contains("sourceeffort")
    {
        Some(BondGraphRole::Effort)
    } else if lower.contains("sourcecurrent")
        || lower.contains("source_current")
        || lower.contains("sourceflow")
    {
        Some(BondGraphRole::Flow)
    } else {
        None
    }
}

/// Extract a numeric default value from an element's children.
///
/// Walks the element's descendants looking for LiteralInteger or LiteralRational
/// elements (possibly nested under FeatureValue). Returns the first numeric
/// value found.
fn extract_numeric_default(elem_id: &sysml_core::ElementId, model: &ModelGraph) -> Option<f64> {
    // Direct children: look for literals
    for child in model.children_of(elem_id) {
        match child.kind {
            ElementKind::LiteralInteger => {
                if let Some(sysml_core::Value::Int(i)) = child.get_prop("value") {
                    return Some(*i as f64);
                }
            }
            ElementKind::LiteralRational => {
                if let Some(sysml_core::Value::Float(f)) = child.get_prop("value") {
                    return Some(*f);
                }
            }
            _ => {
                // Recurse one level (e.g., FeatureValue → Literal)
                if let Some(val) = extract_numeric_default(&child.id, model) {
                    return Some(val);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::flows::port::PortDirection;
    use crate::physics::connection::{
        ConnectionGraph, Junction, JunctionType, PhysicsConnection, PhysicsPortNode,
    };
    use crate::physics::domain::{ConservationLaw, PhysicsDomainRegistry};

    /// Helper: build a PhysicsPortNode.
    fn node(
        id: usize,
        owner: &str,
        port: &str,
        domain: Option<&'static str>,
        dir: PortDirection,
    ) -> PhysicsPortNode {
        PhysicsPortNode {
            id,
            qualified_path: format!("{}.{}", owner, port),
            owner_path: owner.to_string(),
            port_name: port.to_string(),
            domain,
            direction: dir,
            classification: None,
        }
    }

    /// Test 1: KCL at a busbar — 1 incoming, 3 outgoing electrical ports.
    #[test]
    fn kcl_at_busbar() {
        let nodes = vec![
            node(
                0,
                "busbar",
                "powerIn",
                Some("electrical"),
                PortDirection::In,
            ),
            node(
                1,
                "busbar",
                "circuitOut1",
                Some("electrical"),
                PortDirection::Out,
            ),
            node(
                2,
                "busbar",
                "circuitOut2",
                Some("electrical"),
                PortDirection::Out,
            ),
            node(
                3,
                "busbar",
                "circuitOut3",
                Some("electrical"),
                PortDirection::Out,
            ),
        ];

        let junctions = vec![Junction {
            id: 0,
            owner: "busbar".to_string(),
            domain: "electrical",
            junction_type: JunctionType::Zero,
            conservation: ConservationLaw::FlowConservation,
            incoming: vec![(0, "current".to_string())],
            outgoing: vec![
                (1, "current".to_string()),
                (2, "current".to_string()),
                (3, "current".to_string()),
            ],
        }];

        let graph = ConnectionGraph {
            nodes,
            edges: vec![],
            junctions,
        };

        let registry = PhysicsDomainRegistry::new();
        let result = generate_constraints(&graph, &registry);

        assert_eq!(result.conservation.len(), 1);
        assert!(result.diagnostics.is_empty());

        let c = &result.conservation[0];
        assert_eq!(c.name, "kcl_busbar_electrical");
        assert_eq!(c.law, ConservationLaw::FlowConservation);
        assert_eq!(c.incoming_vars, vec!["busbar.powerIn.current"]);
        assert_eq!(
            c.outgoing_vars,
            vec![
                "busbar.circuitOut1.current",
                "busbar.circuitOut2.current",
                "busbar.circuitOut3.current",
            ]
        );
    }

    /// Test 2: Effort equalities — 2 electrical edges produce 2 voltage equalities.
    #[test]
    fn effort_equalities_electrical() {
        let nodes = vec![
            node(
                0,
                "busbar",
                "circuitOut1",
                Some("electrical"),
                PortDirection::Out,
            ),
            node(
                1,
                "circuit1",
                "phaseIn",
                Some("electrical"),
                PortDirection::In,
            ),
            node(
                2,
                "busbar",
                "circuitOut2",
                Some("electrical"),
                PortDirection::Out,
            ),
            node(
                3,
                "circuit2",
                "phaseIn",
                Some("electrical"),
                PortDirection::In,
            ),
        ];

        let edges = vec![
            PhysicsConnection {
                source: 0,
                target: 1,
                domain: Some("electrical"),
                enabled: true,
            },
            PhysicsConnection {
                source: 2,
                target: 3,
                domain: Some("electrical"),
                enabled: true,
            },
        ];

        let graph = ConnectionGraph {
            nodes,
            edges,
            junctions: vec![],
        };

        let registry = PhysicsDomainRegistry::new();
        let result = generate_constraints(&graph, &registry);

        assert_eq!(result.effort_equalities.len(), 2);
        assert_eq!(
            result.effort_equalities[0],
            EffortEquality {
                source_var: "busbar.circuitOut1.voltage".to_string(),
                target_var: "circuit1.phaseIn.voltage".to_string(),
            }
        );
        assert_eq!(
            result.effort_equalities[1],
            EffortEquality {
                source_var: "busbar.circuitOut2.voltage".to_string(),
                target_var: "circuit2.phaseIn.voltage".to_string(),
            }
        );
    }

    /// Test 3: Signal-domain junction produces no conservation constraint.
    #[test]
    fn signal_domain_skipped() {
        let nodes = vec![
            node(
                0,
                "controller",
                "cmdOut",
                Some("signal"),
                PortDirection::Out,
            ),
            node(1, "actuator", "cmdIn", Some("signal"), PortDirection::In),
        ];

        let junctions = vec![Junction {
            id: 0,
            owner: "controller".to_string(),
            domain: "signal",
            junction_type: JunctionType::Zero,
            conservation: ConservationLaw::SignalRouting,
            incoming: vec![(1, "signal".to_string())],
            outgoing: vec![(0, "signal".to_string())],
        }];

        // Also add a signal-domain edge to verify effort equality is skipped
        let edges = vec![PhysicsConnection {
            source: 0,
            target: 1,
            domain: Some("signal"),
            enabled: true,
        }];

        let graph = ConnectionGraph {
            nodes,
            edges,
            junctions,
        };

        let registry = PhysicsDomainRegistry::new();
        let result = generate_constraints(&graph, &registry);

        assert!(
            result.conservation.is_empty(),
            "signal junctions should not produce conservation constraints"
        );
        assert!(
            result.effort_equalities.is_empty(),
            "signal edges should not produce effort equalities"
        );
    }

    /// Test 4: Empty graph returns empty constraints.
    #[test]
    fn empty_graph() {
        let graph = ConnectionGraph {
            nodes: vec![],
            edges: vec![],
            junctions: vec![],
        };

        let registry = PhysicsDomainRegistry::new();
        let result = generate_constraints(&graph, &registry);

        assert!(result.conservation.is_empty());
        assert!(result.effort_equalities.is_empty());
        assert!(result.diagnostics.is_empty());
    }

    /// Test 5: Thermal domain uses energy balance and correct feature names.
    #[test]
    fn thermal_energy_balance() {
        let nodes = vec![
            node(0, "heatsink", "heatIn", Some("thermal"), PortDirection::In),
            node(
                1,
                "heatsink",
                "heatOut",
                Some("thermal"),
                PortDirection::Out,
            ),
        ];

        let junctions = vec![Junction {
            id: 0,
            owner: "heatsink".to_string(),
            domain: "thermal",
            junction_type: JunctionType::Zero,
            conservation: ConservationLaw::EnergyBalance,
            incoming: vec![(0, "heatFlow".to_string())],
            outgoing: vec![(1, "heatFlow".to_string())],
        }];

        let edges = vec![PhysicsConnection {
            source: 0,
            target: 1,
            domain: Some("thermal"),
            enabled: true,
        }];

        let graph = ConnectionGraph {
            nodes,
            edges,
            junctions,
        };

        let registry = PhysicsDomainRegistry::new();
        let result = generate_constraints(&graph, &registry);

        assert_eq!(result.conservation.len(), 1);
        let c = &result.conservation[0];
        assert_eq!(c.name, "kcl_heatsink_thermal");
        assert_eq!(c.law, ConservationLaw::EnergyBalance);
        assert_eq!(c.incoming_vars, vec!["heatsink.heatIn.heat_flow"]);
        assert_eq!(c.outgoing_vars, vec!["heatsink.heatOut.heat_flow"]);

        // Effort equality should use temperature
        assert_eq!(result.effort_equalities.len(), 1);
        assert_eq!(
            result.effort_equalities[0].source_var,
            "heatsink.heatIn.temperature"
        );
        assert_eq!(
            result.effort_equalities[0].target_var,
            "heatsink.heatOut.temperature"
        );
    }

    /// Test 6: Constitutive relation from R-element classified port.
    #[test]
    fn constitutive_resistance_from_classification() {
        use crate::physics::classify::{
            ClassificationConfidence, ClassifiedFeature, PortClassification,
        };
        use crate::physics::dimension::DimensionVector;
        use crate::physics::domain::BondGraphRole;

        let classification = PortClassification {
            domain: Some("electrical"),
            confidence: ClassificationConfidence::ISQTyped,
            diagnostics: vec![],
            is_signal: false,
            carrier_domain: None,
            features: vec![
                ClassifiedFeature {
                    name: "voltage".to_string(),
                    role: VariableRole::Effort,
                    dimension: Some(DimensionVector::new(2, 1, -3, -1, 0, 0, 0)),
                    bond_graph_role: Some(BondGraphRole::Effort),
                },
                ClassifiedFeature {
                    name: "current".to_string(),
                    role: VariableRole::Flow,
                    dimension: Some(DimensionVector::new(0, 0, 0, 1, 0, 0, 0)),
                    bond_graph_role: Some(BondGraphRole::Flow),
                },
                ClassifiedFeature {
                    name: "resistance".to_string(),
                    role: VariableRole::Parameter,
                    dimension: Some(DimensionVector::new(2, 1, -3, -2, 0, 0, 0)),
                    bond_graph_role: Some(BondGraphRole::Resistance),
                },
            ],
        };

        let nodes = vec![PhysicsPortNode {
            id: 0,
            qualified_path: "resistor.phaseIn".to_string(),
            owner_path: "resistor".to_string(),
            port_name: "phaseIn".to_string(),
            domain: Some("electrical"),
            direction: PortDirection::In,
            classification: Some(classification),
        }];

        let graph = ConnectionGraph {
            nodes,
            edges: vec![],
            junctions: vec![],
        };
        let relations =
            super::generate_constitutive_relations(&graph, None, &PhysicsDomainRegistry::new());

        assert_eq!(relations.len(), 1);
        match &relations[0] {
            ConstitutiveRelation::Resistance {
                effort_in_var,
                flow_var,
                parameter_var,
                ..
            } => {
                assert_eq!(effort_in_var, "resistor.phaseIn.voltage");
                assert_eq!(flow_var, "resistor.phaseIn.current");
                assert_eq!(parameter_var, "resistor.phaseIn.resistance");
            }
            other => panic!("expected Resistance, got {:?}", other),
        }
    }

    /// Test 7: Constitutive relation from C-element classified port.
    #[test]
    fn constitutive_capacitance_from_classification() {
        use crate::physics::classify::{
            ClassificationConfidence, ClassifiedFeature, PortClassification,
        };
        use crate::physics::dimension::DimensionVector;
        use crate::physics::domain::BondGraphRole;

        let classification = PortClassification {
            domain: Some("electrical"),
            confidence: ClassificationConfidence::ISQTyped,
            diagnostics: vec![],
            is_signal: false,
            carrier_domain: None,
            features: vec![
                ClassifiedFeature {
                    name: "voltage".to_string(),
                    role: VariableRole::Effort,
                    dimension: Some(DimensionVector::new(2, 1, -3, -1, 0, 0, 0)),
                    bond_graph_role: Some(BondGraphRole::Effort),
                },
                ClassifiedFeature {
                    name: "current".to_string(),
                    role: VariableRole::Flow,
                    dimension: Some(DimensionVector::new(0, 0, 0, 1, 0, 0, 0)),
                    bond_graph_role: Some(BondGraphRole::Flow),
                },
                ClassifiedFeature {
                    name: "capacitance".to_string(),
                    role: VariableRole::Storage,
                    dimension: Some(DimensionVector::new(-2, -1, 4, 2, 0, 0, 0)),
                    bond_graph_role: Some(BondGraphRole::Capacitance),
                },
            ],
        };

        let nodes = vec![PhysicsPortNode {
            id: 0,
            qualified_path: "cap.phaseIn".to_string(),
            owner_path: "cap".to_string(),
            port_name: "phaseIn".to_string(),
            domain: Some("electrical"),
            direction: PortDirection::In,
            classification: Some(classification),
        }];

        let graph = ConnectionGraph {
            nodes,
            edges: vec![],
            junctions: vec![],
        };
        let relations =
            super::generate_constitutive_relations(&graph, None, &PhysicsDomainRegistry::new());

        assert_eq!(relations.len(), 1);
        match &relations[0] {
            ConstitutiveRelation::Capacitance {
                effort_var,
                flow_var,
                parameter_var,
                ..
            } => {
                assert_eq!(effort_var, "cap.phaseIn.voltage");
                assert_eq!(flow_var, "cap.phaseIn.current");
                assert_eq!(parameter_var, "cap.phaseIn.capacitance");
            }
            other => panic!("expected Capacitance, got {:?}", other),
        }
    }

    /// Test 8: No classified features → no constitutive relations.
    #[test]
    fn no_constitutive_without_classification() {
        let nodes = vec![node(0, "a", "out", Some("electrical"), PortDirection::Out)];
        let graph = ConnectionGraph {
            nodes,
            edges: vec![],
            junctions: vec![],
        };
        let relations =
            super::generate_constitutive_relations(&graph, None, &PhysicsDomainRegistry::new());
        assert!(relations.is_empty());
    }

    /// Test 9: Edges with no shared domain produce no effort equality.
    #[test]
    fn no_domain_edge_skipped() {
        let nodes = vec![
            node(0, "a", "out", None, PortDirection::Out),
            node(1, "b", "in", None, PortDirection::In),
        ];

        let edges = vec![PhysicsConnection {
            source: 0,
            target: 1,
            domain: None,
            enabled: true,
        }];

        let graph = ConnectionGraph {
            nodes,
            edges,
            junctions: vec![],
        };

        let registry = PhysicsDomainRegistry::new();
        let result = generate_constraints(&graph, &registry);

        assert!(result.effort_equalities.is_empty());
    }

    // =======================================================================
    // Model-based pipeline tests (items 9.3, 9.8, 9.11, 9.12)
    // =======================================================================

    use sysml_core::{Element, ElementId, ModelGraph as CoreModelGraph, Value};

    /// Helper: build a minimal model graph with a PartDefinition that has
    /// AttributeUsage children with ISQ types and literal default values.
    fn build_resistor_model(resistance_value: f64) -> CoreModelGraph {
        let mut model = CoreModelGraph::new();

        // PartDefinition "Resistor"
        let def_id = ElementId::new_v4();
        model.add_element(
            Element::new(def_id.clone(), ElementKind::PartDefinition).with_name("Resistor"),
        );

        // AttributeUsage "resistance" : ResistanceValue = resistance_value
        let attr_id = ElementId::new_v4();
        model.add_element(
            Element::new(attr_id.clone(), ElementKind::AttributeUsage)
                .with_owner(def_id.clone())
                .with_name("resistance")
                .with_prop("typeName", Value::String("ResistanceValue".into())),
        );

        // LiteralRational child with the numeric value
        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::LiteralRational)
                .with_owner(attr_id)
                .with_prop("value", Value::Float(resistance_value)),
        );

        // PartUsage "r1" : Resistor
        let part_id = ElementId::new_v4();
        model.add_element(
            Element::new(part_id, ElementKind::PartUsage)
                .with_name("r1")
                .with_prop("typeName", Value::String("Resistor".into())),
        );

        model
    }

    /// Test 10: Part attribute classification — R-element from model with numeric value.
    #[test]
    fn model_attribute_resistance_classification() {
        let model = build_resistor_model(10.0);
        let registry = PhysicsDomainRegistry::new();

        let nodes = vec![
            node(0, "r1", "in", Some("electrical"), PortDirection::In),
            node(1, "r1", "out", Some("electrical"), PortDirection::Out),
        ];

        let graph = ConnectionGraph {
            nodes,
            edges: vec![],
            junctions: vec![],
        };

        let relations = super::generate_constitutive_relations(&graph, Some(&model), &registry);

        assert!(
            !relations.is_empty(),
            "should detect R-element from part attributes"
        );
        match &relations[0] {
            ConstitutiveRelation::Resistance {
                parameter_value,
                parameter_var,
                ..
            } => {
                assert_eq!(
                    *parameter_value,
                    Some(10.0),
                    "should extract R=10 from literal"
                );
                assert!(
                    parameter_var.contains("resistance"),
                    "param var should reference resistance attr"
                );
            }
            other => panic!("expected Resistance, got {:?}", other),
        }
    }

    /// Test 11: Source detection — single-port part with effort attribute → EffortSource.
    #[test]
    fn model_effort_source_detection() {
        let mut model = CoreModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        // PartDefinition "VoltageSource" with sourceVoltage attribute
        let def_id = ElementId::new_v4();
        model.add_element(
            Element::new(def_id.clone(), ElementKind::PartDefinition).with_name("VoltageSource"),
        );

        let attr_id = ElementId::new_v4();
        model.add_element(
            Element::new(attr_id.clone(), ElementKind::AttributeUsage)
                .with_owner(def_id.clone())
                .with_name("sourceVoltage"),
        );
        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::LiteralRational)
                .with_owner(attr_id)
                .with_prop("value", Value::Float(12.0)),
        );

        // PartUsage "source" : VoltageSource
        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::PartUsage)
                .with_name("source")
                .with_prop("typeName", Value::String("VoltageSource".into())),
        );

        // Single out port
        let nodes = vec![node(
            0,
            "source",
            "out",
            Some("electrical"),
            PortDirection::Out,
        )];

        let graph = ConnectionGraph {
            nodes,
            edges: vec![],
            junctions: vec![],
        };

        let relations = super::generate_constitutive_relations(&graph, Some(&model), &registry);

        assert!(!relations.is_empty(), "should detect effort source");
        match &relations[0] {
            ConstitutiveRelation::EffortSource {
                source_value,
                effort_var,
            } => {
                assert_eq!(*source_value, Some(12.0));
                assert!(
                    effort_var.contains("voltage"),
                    "should use domain effort feature"
                );
            }
            other => panic!("expected EffortSource, got {:?}", other),
        }
    }

    /// Test 12: Capacitance from model attributes with numeric value.
    #[test]
    fn model_attribute_capacitance_classification() {
        let mut model = CoreModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        let def_id = ElementId::new_v4();
        model.add_element(
            Element::new(def_id.clone(), ElementKind::PartDefinition).with_name("Capacitor"),
        );

        let attr_id = ElementId::new_v4();
        model.add_element(
            Element::new(attr_id.clone(), ElementKind::AttributeUsage)
                .with_owner(def_id.clone())
                .with_name("capacitance")
                .with_prop("typeName", Value::String("CapacitanceValue".into())),
        );
        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::LiteralRational)
                .with_owner(attr_id)
                .with_prop("value", Value::Float(0.001)),
        );

        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::PartUsage)
                .with_name("c1")
                .with_prop("typeName", Value::String("Capacitor".into())),
        );

        let nodes = vec![
            node(0, "c1", "in", Some("electrical"), PortDirection::In),
            node(1, "c1", "out", Some("electrical"), PortDirection::Out),
        ];

        let graph = ConnectionGraph {
            nodes,
            edges: vec![],
            junctions: vec![],
        };

        let relations = super::generate_constitutive_relations(&graph, Some(&model), &registry);

        assert!(!relations.is_empty(), "should detect C-element");
        match &relations[0] {
            ConstitutiveRelation::Capacitance {
                parameter_value,
                parameter_var,
                ..
            } => {
                assert_eq!(*parameter_value, Some(0.001));
                assert!(parameter_var.contains("capacitance"));
            }
            other => panic!("expected Capacitance, got {:?}", other),
        }
    }

    /// Test 13: 2-port owner correlation — different domains → Transformer.
    #[test]
    fn model_two_port_transformer_detection() {
        let model = CoreModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        // A part with one electrical port and one mechanical port → TF
        let nodes = vec![
            node(0, "motor", "elecIn", Some("electrical"), PortDirection::In),
            node(
                1,
                "motor",
                "mechOut",
                Some("mechanical_translational"),
                PortDirection::Out,
            ),
        ];

        let graph = ConnectionGraph {
            nodes,
            edges: vec![],
            junctions: vec![],
        };

        let relations = super::generate_constitutive_relations(&graph, Some(&model), &registry);

        assert!(
            !relations.is_empty(),
            "should detect TF from 2-port multi-domain"
        );
        match &relations[0] {
            ConstitutiveRelation::Transformer {
                effort_in_var,
                effort_out_var,
                modulus,
                ..
            } => {
                assert!(effort_in_var.contains("voltage"), "port 0 effort = voltage");
                assert!(
                    effort_out_var.contains("velocity"),
                    "port 1 effort = velocity"
                );
                assert_eq!(*modulus, 1.0, "default modulus when no attribute found");
            }
            other => panic!("expected Transformer, got {:?}", other),
        }
    }

    /// Test 14: No model → only port-feature classification, no model pipeline.
    #[test]
    fn no_model_skips_attribute_classification() {
        let nodes = vec![
            node(0, "r1", "in", Some("electrical"), PortDirection::In),
            node(1, "r1", "out", Some("electrical"), PortDirection::Out),
        ];

        let graph = ConnectionGraph {
            nodes,
            edges: vec![],
            junctions: vec![],
        };

        let relations =
            super::generate_constitutive_relations(&graph, None, &PhysicsDomainRegistry::new());

        assert!(
            relations.is_empty(),
            "no model → no attribute classification"
        );
    }

    // ── Phase 2: UserConstraintExpression extraction tests ──────────

    fn build_model_with_constraint(name: &str, expr: &str) -> ModelGraph {
        use sysml_core::{Element, ElementId};

        let mut graph = ModelGraph::new();
        let id = ElementId::new_v4();
        let mut elem = Element::new(id, ElementKind::ConstraintDefinition);
        elem.name = Some(name.to_string());
        elem.set_prop("constraint", sysml_core::Value::String(expr.to_string()));
        graph.add_element(elem);
        graph
    }

    #[test]
    fn extract_ohms_law_constraint() {
        let model = build_model_with_constraint("OhmsLaw", "e == R * f");
        let (constraints, diags) = super::extract_user_constraints(&model);
        assert!(diags.is_empty(), "unexpected diags: {:?}", diags);
        assert_eq!(constraints.len(), 1);
        assert_eq!(constraints[0].source, "e == R * f");
        assert_eq!(constraints[0].owner_name, Some("OhmsLaw".to_string()));
    }

    #[test]
    fn constraint_free_variables_detected() {
        let model = build_model_with_constraint("OhmsLaw", "e == R * f");
        let (constraints, _) = super::extract_user_constraints(&model);
        let vars = &constraints[0].referenced_vars;
        assert!(vars.contains(&"R".to_string()), "expected R in {:?}", vars);
        assert!(vars.contains(&"e".to_string()), "expected e in {:?}", vars);
        assert!(vars.contains(&"f".to_string()), "expected f in {:?}", vars);
    }

    #[test]
    fn constraint_residual_evaluates_to_zero() {
        use crate::expressions::{EvalContext, ExpressionEvaluator};
        use sysml_core::Value;

        let model = build_model_with_constraint("OhmsLaw", "e == R * f");
        let (constraints, _) = super::extract_user_constraints(&model);
        let uc = &constraints[0];

        let evaluator = ExpressionEvaluator::new();
        let mut ctx = EvalContext::new();
        ctx.set("e", Value::Float(10.0));
        ctx.set("R", Value::Float(5.0));
        ctx.set("f", Value::Float(2.0));

        let result = evaluator.eval(&uc.residual_expr, &ctx).unwrap();
        match result {
            Value::Float(v) => assert!(v.abs() < 1e-10, "expected 0, got {}", v),
            _ => panic!("expected float, got {:?}", result),
        }
    }

    #[test]
    fn constraint_residual_nonzero_when_violated() {
        use crate::expressions::{EvalContext, ExpressionEvaluator};
        use sysml_core::Value;

        let model = build_model_with_constraint("OhmsLaw", "e == R * f");
        let (constraints, _) = super::extract_user_constraints(&model);
        let uc = &constraints[0];

        let evaluator = ExpressionEvaluator::new();
        let mut ctx = EvalContext::new();
        ctx.set("e", Value::Float(10.0));
        ctx.set("R", Value::Float(5.0));
        ctx.set("f", Value::Float(3.0)); // Violated: 10 != 5*3=15

        let result = evaluator.eval(&uc.residual_expr, &ctx).unwrap();
        match result {
            Value::Float(v) => assert!((v - (-5.0)).abs() < 1e-10, "expected -5, got {}", v),
            _ => panic!("expected float, got {:?}", result),
        }
    }

    #[test]
    fn non_equality_constraint_skipped_with_diagnostic() {
        let model = build_model_with_constraint("Bound", "x > 0");
        let (constraints, diags) = super::extract_user_constraints(&model);
        assert!(constraints.is_empty(), "inequality should be skipped");
        assert_eq!(diags.len(), 1, "should have 1 diagnostic");
    }

    // ── RSC-1.3: open-terminal zero-flow relations ──────────────────────

    use crate::flows::port::{PortInstanceIR, PortRegistry};

    /// Model with a power port def (voltage+current, ISQ-typed) and a signal
    /// port def (flow-only current quantity), plus PortUsages for lookup.
    fn open_terminal_model() -> CoreModelGraph {
        let mut model = CoreModelGraph::new();

        // port def ElPowerPort { voltage; current } — full conjugate pair.
        let power_def = ElementId::new_v4();
        model.add_element(
            Element::new(power_def.clone(), ElementKind::PortDefinition).with_name("ElPowerPort"),
        );
        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(power_def.clone())
                .with_name("voltage")
                .with_prop("typeName", Value::String("ElectricPotentialValue".into())),
        );
        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(power_def)
                .with_name("current")
                .with_prop("typeName", Value::String("ElectricCurrentValue".into())),
        );

        // port def SensePort { rms } — flow-only ⇒ signal.
        let sense_def = ElementId::new_v4();
        model.add_element(
            Element::new(sense_def.clone(), ElementKind::PortDefinition).with_name("SensePort"),
        );
        model.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(sense_def)
                .with_name("rms")
                .with_prop("typeName", Value::String("ElectricCurrentValue".into())),
        );

        model
    }

    fn power_port(owner: &str, name: &str, dir: PortDirection) -> PortInstanceIR {
        PortInstanceIR::new(owner, name)
            .with_definition("ElPowerPort")
            .with_direction(dir)
    }

    /// [`open_terminal_model`] with extra props set on the named feature
    /// (RSC-1.6 declared-default scenarios).
    fn open_terminal_model_with_feature_props(
        feature: &str,
        props: &[(&str, Value)],
    ) -> CoreModelGraph {
        let mut model = open_terminal_model();
        let feat_id = model
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some(feature))
            .map(|e| e.id.clone())
            .expect("feature exists in open_terminal_model");
        let elem = model.get_element_mut(&feat_id).expect("feature element");
        for (key, value) in props {
            elem.set_prop(key.to_string(), value.clone());
        }
        model
    }

    /// Open POWER terminal → one FlowSource(0) + a stated assumption.
    #[test]
    fn open_power_terminal_gets_zero_flow_relation() {
        let model = open_terminal_model();
        let registry = PhysicsDomainRegistry::new();

        let mut port_reg = PortRegistry::new();
        port_reg.register(power_port("busbar", "circuitOut2", PortDirection::Out));

        // Empty connection graph: the port appears in no flow.
        let graph = ConnectionGraph {
            nodes: vec![],
            edges: vec![],
            junctions: vec![],
        };

        let (relations, diags) =
            super::open_terminal_zero_flow_relations(&graph, &port_reg, &model, &registry);

        assert_eq!(relations.len(), 1, "exactly one zero-flow relation");
        match &relations[0] {
            ConstitutiveRelation::FlowSource {
                flow_var,
                source_value,
            } => {
                assert_eq!(flow_var, "busbar.circuitOut2.current");
                assert_eq!(*source_value, Some(0.0));
            }
            other => panic!("expected FlowSource, got {:?}", other),
        }
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0]
                .message
                .contains("open terminal 'busbar.circuitOut2'")
                && diags[0].message.contains("assuming zero current")
                && diags[0].message.contains("unconnected power port"),
            "assumption must be stated: {}",
            diags[0].message
        );
    }

    /// Open SIGNAL port → exempt: no equation, no zero-flow message.
    #[test]
    fn open_signal_port_exempt_from_zero_flow() {
        let model = open_terminal_model();
        let registry = PhysicsDomainRegistry::new();

        let mut port_reg = PortRegistry::new();
        port_reg.register(
            PortInstanceIR::new("sensor", "senseOut")
                .with_definition("SensePort")
                .with_direction(PortDirection::Out),
        );

        let graph = ConnectionGraph {
            nodes: vec![],
            edges: vec![],
            junctions: vec![],
        };

        let (relations, diags) =
            super::open_terminal_zero_flow_relations(&graph, &port_reg, &model, &registry);

        assert!(
            relations.is_empty(),
            "signal ports must not be pinned: {:?}",
            relations
        );
        assert!(
            diags.is_empty(),
            "no assumption message for signal ports: {:?}",
            diags
        );
    }

    /// CONNECTED power port (has a node in the connection graph) → nothing.
    #[test]
    fn connected_power_port_gets_no_zero_flow_relation() {
        let model = open_terminal_model();
        let registry = PhysicsDomainRegistry::new();

        let mut port_reg = PortRegistry::new();
        port_reg.register(power_port("busbar", "circuitOut1", PortDirection::Out));

        let graph = ConnectionGraph {
            nodes: vec![node(
                0,
                "busbar",
                "circuitOut1",
                Some("electrical"),
                PortDirection::Out,
            )],
            edges: vec![],
            junctions: vec![],
        };

        let (relations, diags) =
            super::open_terminal_zero_flow_relations(&graph, &port_reg, &model, &registry);

        assert!(
            relations.is_empty(),
            "connected ports must not be pinned: {:?}",
            relations
        );
        assert!(
            diags.is_empty(),
            "no open-terminal message for connected ports"
        );
    }

    /// Deeper instance paths in the connection graph ("circuit1.busbar.x")
    /// still count as connected for the registry's "busbar.x" key.
    #[test]
    fn leaf_suffix_path_counts_as_connected() {
        let model = open_terminal_model();
        let registry = PhysicsDomainRegistry::new();

        let mut port_reg = PortRegistry::new();
        port_reg.register(power_port("busbar", "circuitOut1", PortDirection::Out));

        let graph = ConnectionGraph {
            nodes: vec![PhysicsPortNode {
                id: 0,
                qualified_path: "circuit1.busbar.circuitOut1".to_string(),
                owner_path: "circuit1.busbar".to_string(),
                port_name: "circuitOut1".to_string(),
                domain: Some("electrical"),
                direction: PortDirection::Out,
                classification: None,
            }],
            edges: vec![],
            junctions: vec![],
        };

        let (relations, _diags) =
            super::open_terminal_zero_flow_relations(&graph, &port_reg, &model, &registry);

        assert!(
            relations.is_empty(),
            "deep-path connected port must not be treated as open: {:?}",
            relations
        );
    }

    // ── RSC-1.6: declared defaults as open-terminal boundary conditions ──

    /// Open POWER terminal whose flow feature declares `default 2.0` →
    /// FlowSource pinned to 2.0 and the message names the default.
    #[test]
    fn open_terminal_declared_default_pins_flow_to_default() {
        let model = open_terminal_model_with_feature_props(
            "current",
            &[
                ("value", Value::Float(2.0)),
                ("isDefault", Value::Bool(true)),
            ],
        );
        let registry = PhysicsDomainRegistry::new();

        let mut port_reg = PortRegistry::new();
        port_reg.register(power_port("busbar", "circuitOut2", PortDirection::Out));

        let graph = ConnectionGraph {
            nodes: vec![],
            edges: vec![],
            junctions: vec![],
        };

        let (relations, diags) =
            super::open_terminal_zero_flow_relations(&graph, &port_reg, &model, &registry);

        assert_eq!(relations.len(), 1, "exactly one flow-source relation");
        match &relations[0] {
            ConstitutiveRelation::FlowSource {
                flow_var,
                source_value,
            } => {
                assert_eq!(flow_var, "busbar.circuitOut2.current");
                assert_eq!(*source_value, Some(2.0));
            }
            other => panic!("expected FlowSource, got {:?}", other),
        }
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0]
                .message
                .contains("open terminal 'busbar.circuitOut2'")
                && diags[0].message.contains("assuming 2 current")
                && diags[0]
                    .message
                    .contains("declared default — model boundary condition"),
            "default must be named in the assumption: {}",
            diags[0].message
        );
    }

    /// Negative numeric default carried by the legacy `unresolved_value`
    /// string (feedback_unresolved_value_numerics) → pinned correctly.
    #[test]
    fn open_terminal_negative_default_via_unresolved_value_string() {
        let model = open_terminal_model_with_feature_props(
            "current",
            &[
                ("isDefault", Value::Bool(true)),
                ("unresolved_value", Value::String("-2.5".into())),
            ],
        );
        let registry = PhysicsDomainRegistry::new();

        let mut port_reg = PortRegistry::new();
        port_reg.register(power_port("busbar", "circuitOut2", PortDirection::Out));

        let graph = ConnectionGraph {
            nodes: vec![],
            edges: vec![],
            junctions: vec![],
        };

        let (relations, diags) =
            super::open_terminal_zero_flow_relations(&graph, &port_reg, &model, &registry);

        assert_eq!(relations.len(), 1);
        match &relations[0] {
            ConstitutiveRelation::FlowSource { source_value, .. } => {
                assert_eq!(*source_value, Some(-2.5));
            }
            other => panic!("expected FlowSource, got {:?}", other),
        }
        assert!(
            diags[0].message.contains("assuming -2.5 current")
                && diags[0].message.contains("declared default"),
            "negative default must be named: {}",
            diags[0].message
        );
    }

    /// Non-numeric/expression default → ignored: flow falls back to 0 and
    /// the message notes the ignored default.
    #[test]
    fn open_terminal_non_numeric_default_falls_back_to_zero() {
        let model = open_terminal_model_with_feature_props(
            "current",
            &[
                ("isDefault", Value::Bool(true)),
                ("value", Value::String("nominalDraw".into())),
            ],
        );
        let registry = PhysicsDomainRegistry::new();

        let mut port_reg = PortRegistry::new();
        port_reg.register(power_port("busbar", "circuitOut2", PortDirection::Out));

        let graph = ConnectionGraph {
            nodes: vec![],
            edges: vec![],
            junctions: vec![],
        };

        let (relations, diags) =
            super::open_terminal_zero_flow_relations(&graph, &port_reg, &model, &registry);

        assert_eq!(relations.len(), 1);
        match &relations[0] {
            ConstitutiveRelation::FlowSource { source_value, .. } => {
                assert_eq!(*source_value, Some(0.0));
            }
            other => panic!("expected FlowSource, got {:?}", other),
        }
        assert!(
            diags[0].message.contains("assuming zero current")
                && diags[0].message.contains("not a literal numeric"),
            "ignored default must be noted: {}",
            diags[0].message
        );
    }

    /// SIGNAL port with a declared default → still exempt: no equation.
    #[test]
    fn open_signal_port_with_default_still_exempt() {
        let model = open_terminal_model_with_feature_props(
            "rms",
            &[
                ("value", Value::Float(3.0)),
                ("isDefault", Value::Bool(true)),
            ],
        );
        let registry = PhysicsDomainRegistry::new();

        let mut port_reg = PortRegistry::new();
        port_reg.register(
            PortInstanceIR::new("sensor", "senseOut")
                .with_definition("SensePort")
                .with_direction(PortDirection::Out),
        );

        let graph = ConnectionGraph {
            nodes: vec![],
            edges: vec![],
            junctions: vec![],
        };

        let (relations, diags) =
            super::open_terminal_zero_flow_relations(&graph, &port_reg, &model, &registry);

        assert!(
            relations.is_empty(),
            "signal ports must not be pinned: {:?}",
            relations
        );
        assert!(
            diags.is_empty(),
            "no assumption message for signal ports: {:?}",
            diags
        );
    }

    /// The zero-flow pin keeps the assembled DAE square and solvable, with
    /// the open terminal's flow trajectory identically zero.
    #[test]
    fn zero_flow_pin_solves_with_open_terminal() {
        // RC circuit (Se 10V → R 5Ω → C 1F) + an open terminal pinned to 0.
        let mut gc = GeneratedConstraints::default();
        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "source.voltage".to_string(),
            source_value: Some(10.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "source.voltage".to_string(),
            effort_out_var: "cap.voltage".to_string(),
            flow_var: "rc.current".to_string(),
            parameter_var: "r1.resistance".to_string(),
            parameter_value: Some(5.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Capacitance {
            effort_var: "cap.voltage".to_string(),
            flow_var: "rc.current".to_string(),
            parameter_var: "cap.capacitance".to_string(),
            parameter_value: Some(1.0),
        });
        gc.constitutive.push(ConstitutiveRelation::FlowSource {
            flow_var: "busbar.circuitOut2.current".to_string(),
            source_value: Some(0.0),
        });

        let dae = crate::physics::dae::BondGraphDae::from_constraints(&gc).unwrap();
        assert_eq!(
            dae.n_equations(),
            dae.map.len(),
            "system must stay square with the zero-flow pin"
        );

        let solution = dae.solve((0.0, 1.0), 1e-6, 1e-8).unwrap();
        let open_idx = solution
            .var_names
            .iter()
            .position(|n| n == "busbar.circuitOut2.current")
            .expect("open terminal variable must exist in the solve");
        assert!(
            solution.x[open_idx].iter().all(|v| v.abs() < 1e-9),
            "open terminal current must stay pinned to 0 over the trajectory"
        );
        // The rest of the network still solves (RC charging unaffected).
        let cap_idx = solution
            .var_names
            .iter()
            .position(|n| n == "cap.voltage")
            .unwrap();
        let final_v = *solution.x[cap_idx].last().unwrap();
        assert!(
            final_v > 0.0,
            "network must still solve around the open terminal"
        );
    }
}
