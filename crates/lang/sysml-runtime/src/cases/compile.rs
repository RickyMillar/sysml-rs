//! Case compilers (`ModelGraph` -> case IR) and their requirement-binding helpers.

use crate::expressions::{compile_expression, compile_simple_expression, EvalContext, ExpressionEvaluator, ExprIR};
use crate::solver_plugin::SolverParam;
use crate::ConstraintIR;
// Requirement feature-typing resolution lives in sysml-core — the ONE
// home shared with the workbench contract display (`sysml_query::
// requirement_detail`), per the 2026-07-16 steward ruling. Do not
// re-grow a local copy here.
use sysml_core::query::{self as core_query, redefined_feature_name, resolve_requirement_typing_target};
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, Value};
use sysml_span::Diagnostic;

#[allow(unused_imports)]
use super::*;

// ---------------------------------------------------------------------------
// Case compiler (S2d)
// ---------------------------------------------------------------------------

/// Compile a verification case from a ModelGraph by name.
///
/// Searches for a `VerificationCaseDefinition` or `VerificationCaseUsage` element
/// with the given name, extracts its owned requirements and subject, and produces
/// a `VerificationCaseIR`.
pub fn compile_verification_case(
    case_name: &str,
    graph: &ModelGraph,
) -> Result<VerificationCaseIR, Vec<Diagnostic>> {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        case = case_name,
        element_count = graph.element_count(),
        "compiling verification case"
    );

    // Find the verification case element by name
    let case_elem = graph
        .elements
        .values()
        .find(|e| {
            (e.kind == ElementKind::VerificationCaseDefinition
                || e.kind == ElementKind::VerificationCaseUsage)
                && e.name.as_deref() == Some(case_name)
        })
        .ok_or_else(|| {
            vec![Diagnostic::error(format!(
                "verification case '{}' not found in model",
                case_name
            ))]
        })?;

    let case_id = case_elem.id.clone();

    // Extract subject from props or children
    let subject = case_elem
        .props
        .get("subject")
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| {
            graph
                .children_of(&case_id)
                .find(|c| c.kind == ElementKind::SubjectMembership)
                .and_then(|m| m.name.clone())
        });

    // Extract requirements owned by this case.
    //
    // Two sources, both spec-relevant:
    //  1. Direct `RequirementUsage`/`RequirementDefinition` children (legacy /
    //     hand-built shape).
    //  2. The objective's verified requirements — `objective { verify requirement
    //     R : Req { … } }` lowers to `ObjectiveMembership →
    //     RequirementVerificationMembership → RequirementUsage → FeatureTyping →
    //     Req`, which the direct-child walk never reaches. Without this, every
    //     case using the spec/corpus `verify requirement` form verifies nothing
    //     and passes vacuously.
    let mut requirements = compile_requirement_checks_for_owner(&case_id, graph);
    let (objective_reqs, _objective_subjects) =
        discover_objective_requirements(&case_id, graph, ObjectiveSubjectSource::CaseSubject);
    requirements.extend(objective_reqs);

    // Extract sub-verification cases (nested verification within this case)
    let sub_cases = graph
        .children_of(&case_id)
        .filter(|e| {
            e.kind == ElementKind::VerificationCaseDefinition
                || e.kind == ElementKind::VerificationCaseUsage
        })
        .filter_map(|e| {
            let sub_name = e.name.as_deref()?;
            compile_verification_case(sub_name, graph).ok()
        })
        .collect::<Vec<_>>();

    // Extract the modeled verdict criterion, if any (§8.4.20.1 — "the criteria
    // for passing must be modeled explicitly"). The result feature is named
    // `verdict` (VerificationCases.sysml:22 `return verdict : VerdictKind :>>
    // result`); its value expression is the criterion.
    let verdict_expression = extract_verdict_expression(&case_id, graph);

    let ir = VerificationCaseIR {
        id: case_id.to_string(),
        name: case_name.to_owned(),
        subject,
        setup_actions: Vec::new(),
        requirements,
        sub_cases,
        verdict_expression,
        // The case's own owned-attribute literals — in scope for the
        // criterion and checks (same collector subject attributes use).
        bindings: collect_occurrence_attribute_values(case_elem, graph),
    };

    #[cfg(feature = "tracing")]
    tracing::debug!(
        case = case_name,
        requirements = ir.requirements.len(),
        sub_cases = ir.sub_cases.len(),
        "compiled verification case"
    );

    Ok(ir)
}

/// Map a modeled verdict criterion's evaluated value to a [`VerdictKind`].
///
/// A `Boolean` follows the `PassIf` contract (`VerificationCases.sysml:70-79`):
/// `true` → `Pass`, `false` → `Fail`. A `String` is read as a `VerdictKind`
/// literal (`pass`/`fail`/`inconclusive`/`error` — the library's lowercase enum
/// literals, matched case-insensitively; also what the built-in `PassIf`
/// returns). Any other value shape yields `None`, which the caller reports as a
/// fail-hard `Error` verdict.
pub(crate) fn verdict_from_value(value: &Value) -> Option<VerdictKind> {
    match value {
        Value::Bool(true) => Some(VerdictKind::Pass),
        Value::Bool(false) => Some(VerdictKind::Fail),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "pass" => Some(VerdictKind::Pass),
            "fail" => Some(VerdictKind::Fail),
            "inconclusive" => Some(VerdictKind::Inconclusive),
            "error" => Some(VerdictKind::Error),
            _ => None,
        },
        _ => None,
    }
}

/// Extract a verification case's modeled verdict criterion as an expression
/// string, or `None` when the case states no explicit criterion.
///
/// §8.4.20.1: "the criteria for passing must be modeled explicitly." The base
/// `Case` declares `return ref result` (`Cases.sysml:49`); a `VerificationCase`
/// redefines it as `return verdict : VerdictKind :>> result`
/// (`VerificationCases.sysml:22`). A concrete case states its criterion by giving
/// that result feature a value expression — `return verdict = PassIf(margin >
/// 2.0)` (the `PassIf` helper, `VerificationCases.sysml:70-79`) or any Boolean /
/// `VerdictKind`-valued expression.
///
/// Lowering (confirmed against the tree-sitter output): `return verdict = <expr>`
/// lowers to a `ReturnParameterMembership` child of the case whose value subtree
/// pretty-prints to `<expr>`. The `return attribute verdict` spelling instead
/// lands the expression on an `AttributeUsage`/`ReferenceUsage` named `verdict`
/// (with a sibling empty return membership), so a named-`verdict` fallback keeps
/// that form reachable too. Returns the pretty-printed expression text; evaluated
/// at verify time by [`VerificationRunner`] (one pretty-print → compile → eval
/// pattern, shared with [`AnalysisCaseIR::result_expression`]).
fn extract_verdict_expression(
    case_id: &sysml_core::ElementId,
    graph: &ModelGraph,
) -> Option<String> {
    // Canonical form: `return verdict = <expr>` → ReturnParameterMembership.
    if let Some(expr) = graph
        .children_of(case_id)
        .filter(|c| c.kind == ElementKind::ReturnParameterMembership)
        .find_map(|c| sysml_core::expression_pretty::pretty_print_owner(c, graph))
    {
        return Some(expr);
    }

    // `return attribute verdict = <expr>` / `verdict = <expr>` → a feature named
    // `verdict` (the result feature, VerificationCases.sysml:22) carrying the
    // value expression.
    graph
        .children_of(case_id)
        .filter(|c| {
            c.name.as_deref() == Some("verdict")
                && matches!(
                    c.kind,
                    ElementKind::AttributeUsage
                        | ElementKind::ReferenceUsage
                        | ElementKind::ReturnParameterMembership
                )
        })
        .find_map(|c| sysml_core::expression_pretty::pretty_print_owner(c, graph))
}

/// Compile a use case from a ModelGraph by name.
///
/// Searches for a `UseCaseDefinition` or `UseCaseUsage` element with the given
/// name, extracts actors, subject, and includes.
pub fn compile_use_case(case_name: &str, graph: &ModelGraph) -> Result<UseCaseIR, Vec<Diagnostic>> {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        case = case_name,
        element_count = graph.element_count(),
        "compiling use case"
    );

    // Find the use case element by name
    let case_elem = graph
        .elements
        .values()
        .find(|e| {
            (e.kind == ElementKind::UseCaseDefinition || e.kind == ElementKind::UseCaseUsage)
                && e.name.as_deref() == Some(case_name)
        })
        .ok_or_else(|| {
            vec![Diagnostic::error(format!(
                "use case '{}' not found in model",
                case_name
            ))]
        })?;

    let case_id = case_elem.id.clone();

    // Extract subject
    let subject = case_elem.props.get("subject").and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        _ => None,
    });

    // Extract actors from ActorMembership children
    let actors = compile_actors_for_owner(&case_id, graph);

    // Extract objective
    let objective = case_elem.props.get("objective").and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        _ => None,
    });

    // Extract includes (IncludeUseCaseUsage children)
    let includes = graph
        .children_of(&case_id)
        .filter(|c| c.kind == ElementKind::IncludeUseCaseUsage)
        .map(|inc| {
            let inc_name = inc.name.as_deref().unwrap_or("unnamed");
            UseCaseIR {
                id: inc.id.to_string(),
                name: inc_name.to_owned(),
                subject: None,
                actors: Vec::new(),
                objective: None,
                steps: Vec::new(),
                includes: Vec::new(),
            }
        })
        .collect();

    let ir = UseCaseIR {
        id: case_id.to_string(),
        name: case_name.to_owned(),
        subject,
        actors,
        objective,
        steps: Vec::new(),
        includes,
    };

    #[cfg(feature = "tracing")]
    tracing::debug!(
        case = case_name,
        actors = ir.actors.len(),
        includes = ir.includes.len(),
        "compiled use case"
    );

    Ok(ir)
}

/// Compile an analysis case from a ModelGraph by name.
///
/// Extracts ToolExecution metadata (tool name + URI), parameters (from
/// AttributeUsage children with direction), constraints (from ConstraintUsage
/// children), and result expressions.
pub fn compile_analysis_case(
    case_name: &str,
    graph: &ModelGraph,
) -> Result<AnalysisCaseIR, Vec<Diagnostic>> {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        case = case_name,
        element_count = graph.element_count(),
        "compiling analysis case"
    );

    let case_elem = graph
        .elements
        .values()
        .find(|e| {
            (e.kind == ElementKind::AnalysisCaseDefinition
                || e.kind == ElementKind::AnalysisCaseUsage)
                && e.name.as_deref() == Some(case_name)
        })
        .ok_or_else(|| {
            vec![Diagnostic::error(format!(
                "analysis case '{}' not found in model",
                case_name
            ))]
        })?;

    let case_id = case_elem.id.clone();

    // Extract ToolExecution metadata (tool name + URI)
    let tool_exec = sysml_core::metadata::get_tool_execution(graph, case_elem);
    let tool_name = tool_exec.as_ref().map(|t| t.tool_name.clone());
    let tool_uri = tool_exec.and_then(|t| t.uri);

    // Extract parameters from AttributeUsage children that have a direction
    let tool_vars = sysml_core::metadata::get_tool_variables(graph, case_elem);
    let mut parameters: Vec<SolverParam> = tool_vars
        .into_iter()
        .map(|tv| {
            let direction = match tv.direction {
                sysml_core::metadata::ParamDirection::In => {
                    crate::solver_plugin::ParamDirection::In
                }
                sysml_core::metadata::ParamDirection::Out => {
                    crate::solver_plugin::ParamDirection::Out
                }
                sysml_core::metadata::ParamDirection::InOut => {
                    crate::solver_plugin::ParamDirection::InOut
                }
            };
            SolverParam {
                sysml_name: tv.sysml_name,
                tool_name: Some(tv.tool_name),
                value: None,
                direction,
            }
        })
        .collect();

    // Also pick up AttributeUsage children with a direction but no ToolVariable metadata.
    // These are plain parameters whose sysml_name is used directly.
    for child in graph.children_of(&case_id) {
        if child.kind != ElementKind::AttributeUsage {
            continue;
        }
        let param_name = match &child.name {
            Some(n) => n.clone(),
            None => continue,
        };
        // Skip if already captured via ToolVariable
        if parameters.iter().any(|p| p.sysml_name == param_name) {
            continue;
        }
        let direction = child
            .get_prop("direction")
            .and_then(|v| v.as_str())
            .map(|s| match s {
                "out" => crate::solver_plugin::ParamDirection::Out,
                "inout" => crate::solver_plugin::ParamDirection::InOut,
                _ => crate::solver_plugin::ParamDirection::In,
            });
        // Only include if direction is explicitly specified
        if let Some(dir) = direction {
            let value = child
                .get_prop("default")
                .or_else(|| child.get_prop("value"))
                .and_then(|v| match v {
                    Value::Float(_) | Value::Int(_) | Value::Bool(_) | Value::String(_) => {
                        Some(v.clone())
                    }
                    _ => None,
                });
            parameters.push(SolverParam {
                sysml_name: param_name,
                tool_name: None,
                value,
                direction: dir,
            });
        }
    }

    // Extract constraints from ConstraintUsage children
    let constraints: Vec<ConstraintIR> = graph
        .children_of(&case_id)
        .filter(|c| {
            c.kind == ElementKind::ConstraintUsage || c.kind == ElementKind::ConstraintDefinition
        })
        .map(|c| {
            // AST-first: pretty-print the structured expression subtree if
            // present, else fall back to legacy `constraint` string prop.
            let expr = sysml_core::expression_pretty::pretty_print_owner(c, graph)
                .or_else(|| {
                    c.get_prop("constraint")
                        .and_then(|v| v.as_str().map(String::from))
                })
                .or_else(|| c.name.clone())
                .unwrap_or_default();
            let desc = c
                .get_prop("text")
                .and_then(|v| v.as_str().map(String::from));
            ConstraintIR {
                expr,
                description: desc,
                owner_id: None,
                is_negated: false,
            }
        })
        .collect();

    // Extract result expression from a "return" or "result" child, or from
    // the case element's own expression subtree.
    let result_expression = sysml_core::expression_pretty::pretty_print_owner(case_elem, graph)
        .or_else(|| {
            graph
                .children_of(&case_id)
                .find(|c| {
                    c.name.as_deref() == Some("return") || c.name.as_deref() == Some("result")
                })
                .and_then(|c| {
                    sysml_core::expression_pretty::pretty_print_owner(c, graph).or_else(|| {
                        c.get_prop("default")
                            .or_else(|| c.get_prop("value"))
                            .and_then(|v| v.as_str().map(String::from))
                    })
                })
        });

    // Objective → result binding (§7.23.2 / Cases.sysml:46). Discover the case's
    // `verify`'d objective requirement(s) and bind the analysis result to each one's
    // subject through the SAME discovery path verification cases use — the only
    // difference is the subject-value source (the result vs the case subject). A
    // value-less result leaves the subject unbound → honest Inconclusive at verify.
    // Discover the objective requirement(s) once, regardless of whether the result
    // resolves — the verdict surface must exist either way. A resolved literal binds
    // the objective subject; an absent one leaves it unbound → honest Inconclusive.
    let result_value = analysis_result_value(&case_id, graph);
    let (objective_requirements, objective_subject_names) = discover_objective_requirements(
        &case_id,
        graph,
        ObjectiveSubjectSource::AnalysisResult(result_value.as_ref()),
    );

    let ir = AnalysisCaseIR {
        id: case_elem.id.to_string(),
        name: case_name.to_owned(),
        subject: case_elem.props.get("subject").and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        }),
        objective: case_elem.props.get("objective").and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        }),
        objective_requirements,
        objective_subject_names,
        tool_name,
        tool_uri,
        parameters,
        constraints,
        result_expression,
    };

    #[cfg(feature = "tracing")]
    tracing::debug!(
        case = case_name,
        tool = ir.tool_name.as_deref().unwrap_or("(none)"),
        params = ir.parameters.len(),
        constraints = ir.constraints.len(),
        "compiled analysis case"
    );

    Ok(ir)
}

/// Extract requirement checks owned by a parent element.
///
/// Searches the ModelGraph for `RequirementUsage` or `RequirementDefinition`
/// elements that are children of the given owner, and compiles each into a
/// `RequirementCheck`. Constraint expressions stored in the `constraint` prop
/// are compiled via `compile_simple_expression`.
pub fn compile_requirement_checks(graph: &ModelGraph) -> Vec<RequirementCheck> {
    graph
        .elements
        .values()
        .filter(|e| {
            e.kind == ElementKind::RequirementDefinition || e.kind == ElementKind::RequirementUsage
        })
        .map(|e| requirement_element_to_check(e, graph))
        .collect()
}

/// Extract actor names from a parent element's children.
///
/// Searches for `ActorMembership` children whose names represent actors.
pub fn compile_actors(graph: &ModelGraph) -> Vec<String> {
    graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::ActorMembership)
        .filter_map(|e| e.name.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract requirement checks for a specific owner element.
fn compile_requirement_checks_for_owner(
    owner_id: &sysml_core::ElementId,
    graph: &ModelGraph,
) -> Vec<RequirementCheck> {
    let mut visited = std::collections::HashSet::new();
    compile_requirement_checks_for_owner_inner(owner_id, graph, &mut visited)
}

/// Visited-threading variant: `visited` guards REFERENCE-form recursion
/// (`require otherReq;` — a reference cycle A→B→A must terminate; ownership
/// nesting alone is acyclic but the guard must survive across nesting
/// boundaries, since a nested requirement can reference back up the tree).
fn compile_requirement_checks_for_owner_inner(
    owner_id: &sysml_core::ElementId,
    graph: &ModelGraph,
    visited: &mut std::collections::HashSet<sysml_core::ElementId>,
) -> Vec<RequirementCheck> {
    graph
        .children_of(owner_id)
        .filter(|e| {
            e.kind == ElementKind::RequirementDefinition || e.kind == ElementKind::RequirementUsage
        })
        .map(|e| requirement_element_to_check_inner(e, graph, visited))
        .collect()
}

/// Extract actor names for a specific owner element.
fn compile_actors_for_owner(owner_id: &sysml_core::ElementId, graph: &ModelGraph) -> Vec<String> {
    graph
        .children_of(owner_id)
        .filter(|e| e.kind == ElementKind::ActorMembership)
        .filter_map(|e| e.name.clone())
        .collect()
}

/// Reduce a requirement's documentation body to a single lead sentence, for use
/// as the non-pass verdict-message marker (see `check_requirement`).
///
/// Doc bodies are stored with their `doc /* … */` continuation formatting
/// (embedded newlines + leading `*` on wrapped lines). This collapses that to one
/// line and returns the text up to (and including) the first sentence-ending
/// `. ` boundary — enough to carry a "KNOWN MODEL GAP (task #N)" marker without
/// dragging a multi-paragraph body onto a one-line overlay. The full body stays
/// on the `requirement_text` response field for anyone who wants it.
pub(crate) fn marker_lead_sentence(text: &str) -> String {
    let flat = text
        .lines()
        .map(|l| l.trim().trim_start_matches('*').trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    match flat.find(". ") {
        Some(i) => flat[..=i].trim_end().to_owned(),
        None => flat.trim_end().to_owned(),
    }
}

/// Convert a requirement element to a `RequirementCheck`.
fn requirement_element_to_check(elem: &Element, graph: &ModelGraph) -> RequirementCheck {
    let mut visited = std::collections::HashSet::new();
    requirement_element_to_check_inner(elem, graph, &mut visited)
}

/// Build a [`RequirementCheck`] from a requirement element, aggregating its
/// FULL effective constraint set (owned ∪ inherited via typing + def
/// specialization, redefinition-suppressed) per the full-chain ruling
/// (requirements-workbench-design.md §2.1a, 2026-07-17) — the closure comes
/// from `sysml_core::query::effective_requirement_constraints`, the SAME
/// walker `requirement_detail` displays, so the verdict and the workbench
/// contract can never drift. `visited` guards reference-form recursion.
fn requirement_element_to_check_inner(
    elem: &Element,
    graph: &ModelGraph,
    visited: &mut std::collections::HashSet<sysml_core::ElementId>,
) -> RequirementCheck {
    visited.insert(elem.id.clone());
    let id = elem.name.clone().unwrap_or_else(|| elem.id.to_string());

    let text = elem
        .props
        .get("text")
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| {
            // Fall back to the requirement's own documentation (`doc /* … */`,
            // stored as a `Documentation` child's `body`). This is what carries
            // a modeled annotation — e.g. a "KNOWN MODEL GAP (task #N)" marker —
            // into the requirement's text so the verdict message can surface it
            // (see `check_requirement`'s non-pass message). The parser does not
            // set a `text` prop on requirements, so without this the text is
            // always empty and a modeled marker never reaches any verdict surface.
            graph
                .children_of(&elem.id)
                .find(|c| c.kind == ElementKind::Documentation)
                .and_then(|d| d.get_prop("body"))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.trim().to_owned()),
                    _ => None,
                })
                .filter(|s| !s.is_empty())
        });

    // AST-first: compile constraint children via compile_expression,
    // falling back to legacy `constraint` string prop for test graphs.
    let mut constraints = Vec::new();
    let mut constraint_element_ids = Vec::new();
    let mut compile_errors = Vec::new();

    let mut assumptions = Vec::new();

    // Primary path: the requirement's EFFECTIVE constraint members — owned
    // members plus every inheritance-chain ancestor's (typing + def
    // specialization, transitive, redefinition-suppressed), from the shared
    // sysml-core walker (§2.1a ruling: the closure is UNCONDITIONAL — a
    // usage owning constraints still aggregates its chain's).
    //
    // Per member: `ConstraintUsage` elements compile via AST, following
    // ReferenceSubsetting chains (e.g. `verify BrewTempReq` or
    // `require constraint : BrewTempConstraint`) to the terminal definition.
    // `RequirementConstraintMembership` members (the parser's lowering of
    // `require constraint { … }` / `assume constraint { … }` — a membership
    // carrying the expression as a `constraint` string prop plus a `role`)
    // pretty-print AST-first with prop fallbacks.
    // (Requirements.sysml:27-41: the requirement result is
    // `allTrue(assumptions()) implies allTrue(constraints())`.)
    let mut referenced_requirement_checks: Vec<RequirementCheck> = Vec::new();
    for member in core_query::effective_requirement_constraints(elem, graph) {
        let child = member.element;
        if child.kind == ElementKind::ConstraintUsage {
            let mut ref_visited = std::collections::HashSet::new();
            ref_visited.insert(child.id.clone());
            match compile_constraint_following_references(child, graph, &mut ref_visited) {
                Ok(exprs) => {
                    for (expr, source_id) in exprs {
                        constraints.push(expr);
                        constraint_element_ids.push(source_id.map(|id| id.to_string()));
                    }
                }
                Err(msg) => compile_errors.push(msg),
            }
        } else if child.kind == ElementKind::RequirementConstraintMembership {
            let is_assumption =
                member.role == core_query::RequirementConstraintRole::Assume;
            // Inline body (`require constraint { … }`): AST-FIRST via
            // `compile_expression` on the body OWNER — the membership's
            // owned ConstraintUsage (`ownedConstraint`, the spec shape the
            // parser mints; v2 unification, design doc §7.1) — falling back
            // to the membership itself for hand-crafted graphs. The string
            // prop is only re-parsed when no AST child exists. Reference
            // forms have neither AST children nor the prop and fall
            // through: a constraint-kind target compiles its definition's
            // expression; a requirement-kind target binds as ONE nested
            // obligation (§2.1a(c)).
            let body_owner = core_query::requirement_constraint_body_owner(child, graph);
            match compile_expression(body_owner, graph) {
                Ok(expr) => {
                    if is_assumption {
                        assumptions.push(expr);
                    } else {
                        constraints.push(expr);
                        constraint_element_ids.push(Some(child.id.to_string()));
                    }
                }
                Err(diags)
                    if child.get_prop("constraint").is_some()
                        || child.get_prop("expr").is_some() =>
                {
                    // A body existed and failed to compile — an honest
                    // error, never a fall-through to reference resolution.
                    compile_errors.push(
                        diags
                            .iter()
                            .map(|d| d.message.as_str())
                            .collect::<Vec<_>>()
                            .join("; "),
                    );
                }
                Err(_) => {
                    if let Some(text) = resolve_referenced_constraint_expr(child, graph) {
                        match compile_simple_expression(&text) {
                            Ok(expr) => {
                                if is_assumption {
                                    assumptions.push(expr);
                                } else {
                                    constraints.push(expr);
                                    constraint_element_ids.push(Some(child.id.to_string()));
                                }
                            }
                            Err(diags) => compile_errors.push(
                                diags
                                    .iter()
                                    .map(|d| d.message.as_str())
                                    .collect::<Vec<_>>()
                                    .join("; "),
                            ),
                        }
                    } else if let Some(target) = resolve_referenced_requirement(child, graph) {
                        // Reference form naming a REQUIREMENT (`require
                        // emcCompliance;` — §2.1a ruling (c)): spec-valid
                        // (RequirementUsage IS a ConstraintUsage), bound as
                        // ONE nested obligation — the referenced requirement's
                        // own aggregate result contributes as a single
                        // sub-check (mirrors `subrequirements` semantics),
                        // never a flattening of its constraint list. Cycles
                        // terminate as an honest compile error, not a silent
                        // skip.
                        if visited.contains(&target.id) {
                            compile_errors.push(format!(
                                "cyclic requirement reference: '{}' is already being evaluated",
                                target.name.as_deref().unwrap_or("<anonymous>")
                            ));
                        } else {
                            referenced_requirement_checks
                                .push(requirement_element_to_check_inner(target, graph, visited));
                        }
                    } else if let Some(name) =
                        sysml_core::query::referenced_constraint_ref_name(child, graph)
                    {
                        // A reference form that names something but resolves
                        // to NEITHER a constraint nor a requirement — a
                        // dangling `require someName;`. It must surface as an
                        // honest Error verdict, never contribute nothing.
                        compile_errors.push(format!(
                            "unresolved constraint reference `{name}` — names no \
                             constraint or requirement in the model"
                        ));
                    }
                }
            }
        }
    }

    // Legacy fallback: if no AST children were found, check the `constraint`
    // string prop (populated by hand-crafted test graphs only).
    if constraints.is_empty() && compile_errors.is_empty() {
        if let Some(val) = elem.props.get("constraint") {
            match val {
                Value::String(s) => match compile_simple_expression(s) {
                    Ok(expr) => {
                        constraints.push(expr);
                        constraint_element_ids.push(Some(elem.id.to_string()));
                    }
                    Err(diags) => {
                        let msg = diags
                            .iter()
                            .map(|d| d.message.as_str())
                            .collect::<Vec<_>>()
                            .join("; ");
                        compile_errors.push(msg);
                    }
                },
                Value::List(items) => {
                    for item in items {
                        if let Value::String(s) = item {
                            match compile_simple_expression(s) {
                                Ok(expr) => {
                                    constraints.push(expr);
                                    constraint_element_ids.push(Some(elem.id.to_string()));
                                }
                                Err(diags) => {
                                    let msg = diags
                                        .iter()
                                        .map(|d| d.message.as_str())
                                        .collect::<Vec<_>>()
                                        .join("; ");
                                    compile_errors.push(msg);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Extract nested sub-requirements (owned), then the reference-form
    // obligations (`require otherReq;`) — both conjoin per
    // Requirements.sysml:90-95 (`subrequirements :> requirementChecks,
    // constraints`).
    let mut subrequirements =
        compile_requirement_checks_for_owner_inner(&elem.id, graph, visited);
    subrequirements.extend(referenced_requirement_checks);

    RequirementCheck {
        id,
        source_element_id: Some(elem.id.to_string()),
        text,
        assumptions,
        constraints,
        constraint_element_ids,
        compile_errors,
        subrequirements,
        // Subject feature values bound as flat dotted keys (`<subject>.<attr>`) so a
        // constraint that references the requirement's subject (e.g. `w.mass < 10`)
        // can evaluate. Aggregated along the inheritance chain (nearest
        // first): an inherited constraint references ITS declaring level's
        // subject name, so the chain's subject-type defaults must be in
        // scope too — first binding per key wins (nearest level is most
        // specific). The objective-discovery path appends the verify
        // clause's redefinition bindings on top of these.
        bindings: Vec::new(),
        binding_specs: {
            // Layered later-wins: subject-type defaults first, then the
            // chain's declared attribute values farthest-first, then the
            // requirement's OWN declared values (`attribute :>> gap = 8.0;`)
            // — nearest declaration wins, and the objective-discovery path
            // appends the case-subject + verify-clause layers after these.
            let mut specs = collect_chain_subject_bindings(elem, graph);
            let chain = core_query::requirement_inheritance_chain(elem, graph);
            for ancestor in chain.iter().rev() {
                specs.extend(collect_attribute_bindings(
                    ancestor.element,
                    &ancestor.element.id,
                    graph,
                ));
            }
            specs.extend(collect_attribute_bindings(elem, &elem.id, graph));
            specs
        },
    }
}

/// Subject-type default bindings for a requirement, aggregated along its
/// inheritance chain (origin first, then chain ancestors nearest-first).
/// First binding per dotted key wins — a nearer level's subject default
/// shadows a farther one, matching redefinition direction.
fn collect_chain_subject_bindings(elem: &Element, graph: &ModelGraph) -> Vec<RequirementBinding> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let push_level = |level: &Element,
                      out: &mut Vec<RequirementBinding>,
                      seen: &mut std::collections::HashSet<String>| {
        for binding in collect_subject_bindings(level, graph) {
            let key = match &binding {
                RequirementBinding::Literal { name, .. }
                | RequirementBinding::FeaturePath { name, .. }
                | RequirementBinding::FeaturePathWithFallback { name, .. }
                | RequirementBinding::Expression { name, .. } => name.clone(),
            };
            if seen.insert(key) {
                out.push(binding);
            }
        }
    };
    push_level(elem, &mut out, &mut seen);
    for ancestor in core_query::requirement_inheritance_chain(elem, graph) {
        push_level(ancestor.element, &mut out, &mut seen);
    }
    out
}

/// The requirement's subject name, chain-walked: an empty typed usage's
/// subject is declared on its def (the superseded owns-none hop reached it
/// by swapping the whole element; the closure reaches it by inheritance —
/// origin first, then nearest ancestor).
fn requirement_subject_name(requirement: &Element, graph: &ModelGraph) -> Option<String> {
    std::iter::once(requirement)
        .chain(
            core_query::requirement_inheritance_chain(requirement, graph)
                .into_iter()
                .map(|a| a.element),
        )
        .find_map(|level| {
            graph
                .children_of(&level.id)
                .find(|c| c.kind == ElementKind::SubjectMembership)
                .and_then(|s| s.name.clone())
        })
}

/// Resolve the REQUIREMENT a reference-form membership names
/// (`require emcCompliance;` → `referencedConstraint` prop), mirroring
/// [`resolve_referenced_constraint_expr`] but for requirement kinds —
/// the §2.1a(c) nested-obligation path.
fn resolve_referenced_requirement<'g>(
    membership: &Element,
    graph: &'g ModelGraph,
) -> Option<&'g Element> {
    // The reference form's target, resolved from its ReferenceSubsetting
    // (SysML-vocab.ttl:2576) — identity, not a re-stringified name lookup.
    let target = sysml_core::query::referenced_constraint_target(membership, graph)?;
    matches!(
        target.kind,
        ElementKind::RequirementDefinition | ElementKind::RequirementUsage
    )
    .then_some(target)
}

/// Compile constraints for a ConstraintUsage, following ReferenceSubsetting chains.
///
/// Handles three cases:
/// 1. The usage has direct expression children — compile them (normal path).
/// 2. The usage references a ConstraintDefinition/Usage (e.g.
///    `require constraint : BrewTempConstraint;`) — compile the target's expression.
/// 3. The usage references a RequirementDefinition/Usage (e.g.
///    `verify BrewTempReq;` inside an objective) — recursively gather all
///    constraint expressions from that requirement's `require constraint :` children.
///
/// The `visited` set guards against cycles in pathological graphs.
fn compile_constraint_following_references(
    usage: &Element,
    graph: &ModelGraph,
    visited: &mut std::collections::HashSet<sysml_core::ElementId>,
) -> Result<Vec<(ExprIR, Option<ElementId>)>, String> {
    // Path 1: try direct expression compilation.
    if let Ok(expr) = compile_expression(usage, graph) {
        return Ok(vec![(expr, Some(usage.id.clone()))]);
    }

    // Path 2/3: follow the ReferenceSubsetting child to its target.
    let Some(target) = resolve_reference_subsetting_target(usage, graph) else {
        return Err(format!(
            "element `{}` has no compilable expression children",
            usage.name.as_deref().unwrap_or("<unnamed>")
        ));
    };

    if !visited.insert(target.id.clone()) {
        return Ok(Vec::new());
    }

    match target.kind {
        ElementKind::ConstraintDefinition | ElementKind::ConstraintUsage => {
            // `require constraint : X` — compile X's expression, or follow
            // X's own reference chain if X is itself a reference.
            match compile_expression(target, graph) {
                Ok(expr) => Ok(vec![(expr, Some(target.id.clone()))]),
                Err(_) => compile_constraint_following_references(target, graph, visited),
            }
        }
        ElementKind::RequirementDefinition | ElementKind::RequirementUsage => {
            // `verify ReqName` — walk the requirement's ConstraintUsage children
            // and follow each chain.
            let mut all = Vec::new();
            for child in graph.children_of(&target.id) {
                if child.kind == ElementKind::ConstraintUsage {
                    if let Ok(mut exprs) =
                        compile_constraint_following_references(child, graph, visited)
                    {
                        all.append(&mut exprs);
                    }
                }
            }
            Ok(all)
        }
        _ => Err(format!(
            "reference target `{}` has unsupported kind {:?}",
            target.name.as_deref().unwrap_or("<unnamed>"),
            target.kind
        )),
    }
}

/// Find the element referenced by an element's first ReferenceSubsetting child.
///
/// Prefers the resolved `referencedFeature` (Value::Ref) prop; falls back to
/// lookup by `unresolved_referencedFeature` name across the whole graph.
fn resolve_reference_subsetting_target<'g>(
    elem: &Element,
    graph: &'g ModelGraph,
) -> Option<&'g Element> {
    graph.children_of(&elem.id).find_map(|child| {
        if child.kind != ElementKind::ReferenceSubsetting {
            return None;
        }
        // Resolved path first.
        if let Some(Value::Ref(id)) = child.props.get("referencedFeature") {
            if let Some(target) = graph.get_element(id) {
                return Some(target);
            }
        }
        // Unresolved fallback: look up by name. Prefer a
        // definition-level element if multiple candidates share the name.
        if let Some(name) = child
            .props
            .get("unresolved_referencedFeature")
            .and_then(|v| v.as_str())
        {
            let matches: Vec<&Element> = graph
                .elements
                .values()
                .filter(|e| {
                    e.name.as_deref() == Some(name)
                        && matches!(
                            e.kind,
                            ElementKind::ConstraintDefinition
                                | ElementKind::ConstraintUsage
                                | ElementKind::RequirementDefinition
                                | ElementKind::RequirementUsage
                        )
                })
                .collect();
            // Prefer Definition kinds (typical target of `:>` subsetting).
            return matches
                .iter()
                .find(|e| {
                    matches!(
                        e.kind,
                        ElementKind::ConstraintDefinition | ElementKind::RequirementDefinition
                    )
                })
                .copied()
                .or_else(|| matches.first().copied());
        }
        None
    })
}

/// Discover the verified requirements reached through a verification case's
/// objective(s).
///
/// Spec chain (VerificationCases.sysml:21-27): a VerificationCase's objective is
/// a `RequirementCheck`; `verify requirement R : Req { … }` declares a
/// `RequirementVerificationMembership` whose verified requirement is `Req`. The
/// tree-sitter lowering produces:
///
/// ```text
/// VerificationCase(Definition|Usage)
///   └── ObjectiveMembership
///         └── RequirementVerificationMembership             // the `verify` clause
///               └── RequirementUsage                        // the check-usage
///                     ├── FeatureTyping → Req               // the verified requirement
///                     └── AttributeUsage { value }          // redefinition bindings
/// ```
///
/// For each verify clause we resolve the verified requirement, compile it into a
/// [`RequirementCheck`], stamp the clause's name as the check id, and attach the
/// clause's redefinition bindings (occurrence-scoped — see
/// [`RequirementCheck::bindings`]).
/// Where the objective's subject value comes from — the ONE axis on which analysis
/// and verification cases differ. The base `Case` declares
/// `objective obj : RequirementCheck { subject subj default Case::result }`
/// (`Cases.sysml:46`); the objective subject is then identified with each verified
/// requirement's subject, so its value drives that requirement's constraints. The two
/// case kinds supply that value differently, so [`discover_objective_requirements`]
/// is parameterized over the source rather than forked into two near-duplicate walks
/// (CLAUDE #4/#5). The chosen arm both SUPPLIES the analysis/verification binding and
/// SUPPRESSES the other kind's — they are mutually exclusive per spec.
enum ObjectiveSubjectSource<'a> {
    /// VerificationCase: objective subject ← the case subject-under-test
    /// (`VerificationCases.sysml:25` `subject subj = VerificationCase::subj`,
    /// overriding the base default). Uses [`case_subject_occurrence_bindings`].
    CaseSubject,
    /// AnalysisCase: objective subject ← the analysis result
    /// (`Cases.sysml:46` `subject subj default Case::result`, which `AnalysisCase`
    /// does NOT override). §7.23.2. Uses [`analysis_result_subject_bindings`]; the
    /// case-subject occurrence binding is deliberately NOT applied. `None` when the
    /// result has no model-declared literal yet (computed/sim result is a later tier)
    /// — the objective subject then stays unbound → honest Inconclusive, never bound
    /// to a placeholder.
    AnalysisResult(Option<&'a Value>),
}

/// Returns the discovered objective requirement-checks AND, index-aligned, each one's
/// objective-subject NAME (the verified requirement's subject, the key the result/
/// case-subject binds under). The names let a verify-time caller (e.g. an analysis
/// case's executed-result path, `AnalysisCaseIR::run_and_verify`) re-bind the subject
/// to a value not known at compile time — aligned by construction (same clause walk),
/// so callers that don't need the names simply drop the second element.
fn discover_objective_requirements(
    case_id: &sysml_core::ElementId,
    graph: &ModelGraph,
    subject_source: ObjectiveSubjectSource,
) -> (Vec<RequirementCheck>, Vec<Option<String>>) {
    let mut out = Vec::new();
    let mut subject_names = Vec::new();
    for objective in graph
        .children_of(case_id)
        .filter(|c| c.kind == ElementKind::ObjectiveMembership)
    {
        for clause in graph.children_of(&objective.id) {
            // Every verify clause is a `RequirementVerificationMembership`
            // (SysML.xtext:2257-2270). Two lowered shapes:
            //  * declaration form `verify requirement R : Req { … }` — the
            //    membership owns a plain `RequirementUsage` check (target by
            //    its FeatureTyping, optional redefinition bindings in its
            //    body). We re-point `clause` to that check-usage below so
            //    name/typing/bindings all read from the right element.
            //  * bare `verify Req;` — the parser flattens the reference onto
            //    the membership itself as the `verifiedRequirement` name prop.
            //
            // Deliberately NOT matched: a bare `RequirementUsage` child of
            // the objective. Elaboration injects the library-derived
            // objective machinery (`requirementVerifications :
            // RequirementCheck`, plus `constraints`/`assumptions` —
            // VerificationCases.sysml:27/35) as RequirementUsages under the
            // objective; those are aggregates, not user verify clauses, and
            // matching them produces phantom requirements.
            if clause.kind != ElementKind::RequirementVerificationMembership {
                continue;
            }
            let clause = graph
                .children_of(&clause.id)
                .find(|c| c.kind == ElementKind::RequirementUsage)
                .unwrap_or(clause);

            // Resolve the verified requirement: a `: Req` typing, a `:> Req`
            // reference-subsetting, or the `verifiedRequirement` name prop (bare
            // `verify Req;`). Build the check from the resolved target so we inherit
            // its constraints/assumptions; fall back to the clause itself for an
            // inline-bodied verify clause.
            let target = resolve_requirement_typing_target(clause, graph)
                .or_else(|| resolve_reference_subsetting_target(clause, graph))
                .or_else(|| resolve_verified_requirement_target(clause, graph));
            let requirement = match target {
                Some(t)
                    if matches!(
                        t.kind,
                        ElementKind::RequirementDefinition | ElementKind::RequirementUsage
                    ) =>
                {
                    t
                }
                _ => clause,
            };
            // No typing hop here anymore: `requirement_element_to_check`
            // aggregates the FULL inheritance chain unconditionally (§2.1a
            // ruling 2026-07-17 — the 2026-07-16 owns-none single hop is
            // superseded), so a bare `verify reqUsage;` of an empty typed
            // usage evaluates its def's constraints through the closure,
            // and a usage that owns constraints STILL aggregates its
            // chain's.
            let mut check = requirement_element_to_check(requirement, graph);

            // The verdict is reported against the verify clause occurrence.
            if let Some(name) = &clause.name {
                check.id = name.clone();
            }
            check.source_element_id = Some(clause.id.to_string());
            // Binding precedence (each layer overlaid later-wins in `check_requirement`):
            //   1. the verified requirement's own subject TYPE defaults
            //      (`requirement_element_to_check` → `collect_subject_bindings`);
            //   2. the OBJECTIVE subject, identified with the verified requirement's
            //      subject — its value comes from `subject_source`: a verification
            //      case's subject-under-test occurrence (§7.24 / VerificationCases.sysml:25)
            //      OR an analysis case's result (§7.23.2 / Cases.sysml:46). Either
            //      overrides the type defaults; the two are mutually exclusive per spec.
            //   3. the verify clause's own redefinitions (`verify R { attribute :>> x = v }`)
            //      — most specific, applied last so a per-clause override always wins.
            let objective_subject_bindings = match &subject_source {
                ObjectiveSubjectSource::CaseSubject => {
                    case_subject_occurrence_bindings(case_id, requirement, graph)
                }
                ObjectiveSubjectSource::AnalysisResult(result) => {
                    analysis_result_subject_bindings(requirement, *result, graph)
                }
            };
            check.binding_specs.extend(objective_subject_bindings);
            check
                .binding_specs
                .extend(collect_redefinition_bindings(clause, case_id, graph));
            // The objective-subject name (the verified requirement's subject) — the
            // binding key the result/case-subject value uses. Captured index-aligned
            // so a verify-time executed-result re-bind can target it.
            let subject_name = requirement_subject_name(requirement, graph);
            out.push(check);
            subject_names.push(subject_name);
        }
    }
    (out, subject_names)
}

/// Collect per-occurrence value bindings declared on a verify clause.
///
/// A `verify requirement R { attribute x = v; }` clause owns redefinition members
/// (e.g. `AttributeUsage { value }`) that bind feature values for this occurrence
/// only. Per spec the bound `=` is a `FeatureValue` equivalent to a
/// BindingConnector (SysML-vocab.ttl:423-425); we surface it as a name→value pair
/// overlaid onto the requirement's evaluation context.
///
/// Two surface forms are handled:
///  * Literal: `attribute x = 5;` — the parser names the usage `x` and stores the
///    literal on its `value` prop. Key = `x`, value = the literal.
///  * Feature-reference redefinition: `attribute :>> w.mass = massRun.massResult;`
///    — the redefining usage is unnamed; the `:>>` target lives on a child
///    `Redefinition` (`unresolved_redefinedFeature = "w.mass"`, matching the dotted
///    flat key the evaluator's feature-chain fast path looks up) and the RHS is a
///    child `FeatureReferenceExpression` naming a feature path. Key = the redefined
///    feature; value = the referenced feature's model-declared literal, resolved
///    within the enclosing verification case. The value flows ONLY through the
///    model's explicit `=`/redefinition bindings — never an injected key — so a
///    sim/analysis result reaches the verdict the same way (Inc2: the analysis
///    usage's `return … = …` supplies the literal). Unresolvable or value-less →
///    no binding, leaving the constraint honestly Inconclusive rather than masking
///    the missing value.
fn collect_redefinition_bindings(
    clause: &Element,
    case_id: &sysml_core::ElementId,
    graph: &ModelGraph,
) -> Vec<RequirementBinding> {
    collect_attribute_bindings(clause, case_id, graph)
}

/// Attribute-value bindings declared ON an element: each `AttributeUsage` /
/// `ReferenceUsage` child with a value binds its (redefined-else-own) name.
/// Shared by the verify-clause redefinition layer AND the requirement's own
/// declared values (`attribute :>> gap = 8.0;` on a template instantiation,
/// `attribute margin = 2.5;` on a plain requirement) — without the latter,
/// every instantiation evaluated Inconclusive because its own binding never
/// reached the check context.
fn collect_attribute_bindings(
    owner: &Element,
    scope_id: &sysml_core::ElementId,
    graph: &ModelGraph,
) -> Vec<RequirementBinding> {
    let case_id = scope_id;
    let clause = owner;
    let mut bindings = Vec::new();
    for child in graph.children_of(&clause.id) {
        if !matches!(
            child.kind,
            ElementKind::AttributeUsage | ElementKind::ReferenceUsage
        ) {
            continue;
        }
        // Key: a `:>>` redefinition target wins over the local label, because the
        // constraint references the redefined feature (`w.mass`), not the usage's
        // own name. Falls back to `child.name` for the literal `attribute x = v`
        // form (which carries no Redefinition member).
        let Some(key) = redefined_feature_name(child, graph).or_else(|| child.name.clone()) else {
            continue;
        };
        // Value: a literal `=`/default FeatureValue, else a feature-reference RHS
        // resolved lazily against the check-time context. The legacy literal-only
        // graph walk is kept as a fallback for tests/graphs that have no runtime
        // binding for the referenced path yet.
        if let Some(value) = subject_literal_value(child) {
            bindings.push(RequirementBinding::Literal { name: key, value });
        } else if let Some(path) = value_reference_child_path(child, graph) {
            if let Some(fallback) = resolve_feature_path_literal(&path, case_id, graph) {
                bindings.push(RequirementBinding::FeaturePathWithFallback {
                    name: key,
                    path,
                    fallback,
                });
            } else {
                bindings.push(RequirementBinding::FeaturePath { name: key, path });
            }
        } else if let Some(expr) = binding_expression_child(child, graph) {
            bindings.push(RequirementBinding::Expression { name: key, expr });
        }
    }
    bindings
}

// `redefined_feature_name` moved to sysml-core (`query::redefined_feature_name`)
// — shared with requirement_detail's attribute display; do not re-grow a copy.

/// Resolve a redefining usage whose RHS is a feature reference (`= massRun.massResult`)
/// rather than a literal. The parser lowers the RHS to a child
/// `FeatureReferenceExpression` whose `name` is the dotted feature path; we resolve
/// that path to the referenced feature's model-declared literal value.
fn value_reference_child_path(usage: &Element, graph: &ModelGraph) -> Option<String> {
    let fre = graph
        .children_of(&usage.id)
        .find(|c| c.kind == ElementKind::FeatureReferenceExpression)?;
    fre.name.clone()
}

fn binding_expression_child(usage: &Element, graph: &ModelGraph) -> Option<ExprIR> {
    let expr_text = sysml_core::expression_pretty::pretty_print_owner(usage, graph)?;
    compile_simple_expression(&expr_text).ok()
}

/// Walk a dotted feature path (`massRun.massResult`) from the verification-case
/// scope to a terminal literal value. The first segment resolves among the case's
/// own members (occurrence-scoped — a sibling case's same-named analysis can't leak
/// in); each subsequent segment descends into the prior element's children. Returns
/// None if any segment is unresolvable or the terminal carries no literal.
fn resolve_feature_path_literal(
    path: &str,
    case_id: &sysml_core::ElementId,
    graph: &ModelGraph,
) -> Option<Value> {
    let mut segments = path.split('.');
    let first = segments.next()?;
    let mut current = graph
        .children_of(case_id)
        .find(|c| c.name.as_deref() == Some(first))?;
    for seg in segments {
        current = graph
            .children_of(&current.id)
            .find(|c| c.name.as_deref() == Some(seg))?;
    }
    subject_literal_value(current)
}

/// Collect per-occurrence value bindings from a requirement's SUBJECT.
///
/// A requirement's constraints are written in terms of its subject parameter
/// (`subject w : Widget; require constraint { w.mass < 10 }`). Per spec the
/// verification case's subject is identified with the requirement's subject
/// (`subject subj = VerificationCase::subj`, VerificationCases.sysml:25), and
/// value equality flows only through that explicit binding — SysML v2 has no
/// implicit name-matching. We surface the subject's declared attribute values as
/// flat dotted keys `<subject_name>.<attr_name>`, matching the evaluator's
/// feature-chain fast path (so `w.mass` resolves directly). The subject's type is
/// captured by the parser as the `unresolved_type` prop on the SubjectMembership
/// (SPEC-SILENT representation simplification — the type formally lives on the
/// subject's ReferenceUsage child, which we don't yet lower).
///
/// This binds the requirement subject's own type attribute defaults. When the
/// verification case binds its subject to a concrete occurrence
/// (`subject s = anOccurrence;`) those occurrence values OVERRIDE these defaults —
/// see [`case_subject_occurrence_bindings`], overlaid later in
/// `discover_objective_requirements`.
fn collect_subject_bindings(req_elem: &Element, graph: &ModelGraph) -> Vec<RequirementBinding> {
    let mut bindings = Vec::new();
    for subject in graph
        .children_of(&req_elem.id)
        .filter(|c| c.kind == ElementKind::SubjectMembership)
    {
        let Some(subject_name) = subject.name.as_deref() else {
            continue;
        };
        let Some(type_elem) = resolve_subject_type(subject, graph) else {
            continue;
        };
        for attr in graph.children_of(&type_elem.id).filter(|c| {
            matches!(
                c.kind,
                ElementKind::AttributeUsage | ElementKind::ReferenceUsage
            )
        }) {
            if let (Some(attr_name), Some(value)) =
                (attr.name.as_deref(), subject_literal_value(attr))
            {
                bindings.push(RequirementBinding::Literal {
                    name: format!("{subject_name}.{attr_name}"),
                    value,
                });
            }
        }
    }
    bindings
}

/// Overlay bindings from the verification case's subject-under-test onto a verified
/// requirement. When the case binds its subject to a concrete occurrence
/// (`subject s = anOccurrence;`), the spec identifies the verified requirement's
/// subject with the case subject (§7.24 / VerificationCases.sysml:25), so that
/// occurrence's attribute values drive the requirement's constraints. They are
/// keyed under the REQUIREMENT subject's name (what its constraints reference) —
/// value equality flows through the model's explicit `=` binding, never implicit
/// name-matching. Absent binding / unresolvable occurrence → no override, leaving
/// the type defaults (or an honest Inconclusive) in place.
///
/// The bound occurrence's feature path is read AST-first: the parser lowers the
/// `subject s = anOccurrence;` RHS to a child `FeatureReferenceExpression` (same
/// `emit_default_value_expression` path regular usages use), and
/// [`value_reference_child_path`] recovers the (possibly dotted) path from it — no
/// legacy `unresolved_value` string.
fn case_subject_occurrence_bindings(
    case_id: &sysml_core::ElementId,
    requirement: &Element,
    graph: &ModelGraph,
) -> Vec<RequirementBinding> {
    let Some(req_subject_name) = requirement_subject_name(requirement, graph) else {
        return Vec::new();
    };
    let Some(occ_path) = graph
        .children_of(case_id)
        .find(|c| c.kind == ElementKind::SubjectMembership)
        .and_then(|s| value_reference_child_path(s, graph))
    else {
        return Vec::new();
    };
    let Some(occ) = resolve_occurrence_by_path(&occ_path, graph) else {
        return Vec::new();
    };
    collect_occurrence_attribute_values(occ, graph)
        .into_iter()
        .map(|(attr, value)| RequirementBinding::Literal {
            name: format!("{req_subject_name}.{attr}"),
            value,
        })
        .collect()
}

/// Bind an analysis case's RESULT to its objective's verified-requirement subject.
///
/// §7.23.2: "the subject of the objective is always bound to the result of the
/// analysis case." The base `Case` objective declares `subject subj default
/// Case::result` (`Cases.sysml:46`), which `AnalysisCase` does NOT override (unlike
/// a verification case, which binds the case subject — `VerificationCases.sysml:25`).
/// The objective subject is identified with the verified requirement's subject, so
/// the result value drives that requirement's constraints (written in terms of the
/// subject). We key the result under the requirement subject's name — the same
/// structural move as [`case_subject_occurrence_bindings`], with the result as the
/// value source instead of an occurrence-under-test. Value equality flows only
/// through the model-declared `return … = …` (Inc2b B1); a value-less result or a
/// subject-less requirement yields no binding, leaving the constraint honestly
/// Inconclusive (§7.24.1) rather than masking the gap.
///
/// Tier-1 (literal result) handles a SCALAR result bound to a scalar subject
/// (`subject measuredMass : Real`); a structured result (an object with attributes)
/// is a later tier, resolved the way `collect_occurrence_attribute_values` already
/// handles structured occurrences.
fn analysis_result_subject_bindings(
    requirement: &Element,
    result: Option<&Value>,
    graph: &ModelGraph,
) -> Vec<RequirementBinding> {
    let Some(result) = result else {
        return Vec::new();
    };
    let Some(req_subject_name) = graph
        .children_of(&requirement.id)
        .find(|c| c.kind == ElementKind::SubjectMembership)
        .and_then(|s| s.name.clone())
    else {
        return Vec::new();
    };
    vec![RequirementBinding::Literal {
        name: req_subject_name,
        value: result.clone(),
    }]
}

/// Resolve an analysis case's result to a model-declared value, when one exists.
/// The base `Case` declares `return ref result` (`Cases.sysml:49`); a concrete
/// analysis fills it via `return attribute result = <literal-or-expression>`. Two
/// result sources, in precedence order:
///
///  * **Tier-1 (literal):** `return attribute result = 0.3;` — read the literal
///    directly off the result feature.
///  * **Tier-2 (expression/calc):** `return attribute result = base + 2.0;` — the
///    RHS lowers to an expression subtree (OperatorExpression / FeatureReference,
///    `ResultExpressionMembership`). Pretty-print it, compile, and evaluate STATICALLY
///    against the case's input-attribute defaults (`analysis_input_context`). Per the
///    design plan §4.2 ("evaluate statically where possible") this resolves the
///    deterministic, model-declared result; a runtime/solver/sim-supplied input is a
///    later tier (T3a / Inc2b).
///
/// A value-less / non-statically-resolvable result yields `None`, leaving the
/// objective honestly Inconclusive (§7.24.1). The value flows only through the
/// model's `=` (Inc2b B1) — no injected keys.
///
// FIXME(T2 consolidation): `compile_analysis_case` separately pretty-prints the
// result EXPRESSION into `result_expression` (a display string). Both walks resolve
// the same result feature; a future cleanup can share one result-feature resolver.
fn analysis_result_value(case_id: &sysml_core::ElementId, graph: &ModelGraph) -> Option<Value> {
    let result_feature = graph
        .children_of(case_id)
        .filter(|c| {
            matches!(
                c.kind,
                ElementKind::AttributeUsage | ElementKind::ReferenceUsage
            )
        })
        .find(|c| c.name.as_deref() == Some("result"))?;

    // Tier-1: a model-declared literal result.
    if let Some(literal) = subject_literal_value(result_feature) {
        return Some(literal);
    }

    // Tier-2: a result expression evaluated statically over the case's inputs.
    let expr_text = sysml_core::expression_pretty::pretty_print_owner(result_feature, graph)?;
    let expr = compile_simple_expression(&expr_text).ok()?;
    let ctx = analysis_input_context(case_id, graph);
    match ExpressionEvaluator::new().eval(&expr, &ctx) {
        Ok(v @ (Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::String(_))) => Some(v),
        // Unbound input / non-scalar / eval error → no static result → Inconclusive.
        _ => None,
    }
}

/// Seed an `EvalContext` from an analysis case's input-attribute defaults — the
/// model-declared values its result expression is computed over (Inc2b B1). Each
/// direct `AttributeUsage`/`ReferenceUsage` child carrying a literal default
/// contributes a binding (e.g. `in attribute base = 3.0`). Inputs with no literal
/// default stay unbound, so a result expression over them resolves to `None`
/// (honest Inconclusive) rather than a fabricated value.
fn analysis_input_context(case_id: &sysml_core::ElementId, graph: &ModelGraph) -> EvalContext {
    let mut ctx = EvalContext::new();
    for child in graph.children_of(case_id).filter(|c| {
        matches!(
            c.kind,
            ElementKind::AttributeUsage | ElementKind::ReferenceUsage
        )
    }) {
        if let (Some(name), Some(value)) = (child.name.as_deref(), subject_literal_value(child)) {
            ctx.set(name.to_owned(), value);
        }
    }
    ctx
}

/// Resolve an occurrence reference (`lightWidget`, `vehicle_b.engine`) to its
/// element. The first segment resolves globally by name among structural
/// usages/definitions (occurrences are typically top-level parts); each subsequent
/// segment descends into the prior element's children.
fn resolve_occurrence_by_path<'g>(path: &str, graph: &'g ModelGraph) -> Option<&'g Element> {
    let mut segments = path.split('.');
    let first = segments.next()?;
    let mut current = graph.elements.values().find(|e| {
        e.name.as_deref() == Some(first)
            && (e.kind.as_str().ends_with("Definition") || e.kind.as_str().ends_with("Usage"))
    })?;
    for seg in segments {
        current = graph
            .children_of(&current.id)
            .find(|c| c.name.as_deref() == Some(seg))?;
    }
    Some(current)
}

/// Collect an occurrence's bound attribute literal values. A part instance binds
/// attributes either by name (`attribute mass = 3.0`) or by redefinition
/// (`attribute :>> mass = 3.0`, lowered to an unnamed usage + a `Redefinition`
/// member); both are surfaced as `(attr_name, value)`.
fn collect_occurrence_attribute_values(occ: &Element, graph: &ModelGraph) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for attr in graph.children_of(&occ.id).filter(|c| {
        matches!(
            c.kind,
            ElementKind::AttributeUsage | ElementKind::ReferenceUsage
        )
    }) {
        let Some(name) = attr
            .name
            .clone()
            .or_else(|| redefined_feature_name(attr, graph))
        else {
            continue;
        };
        if let Some(value) = subject_literal_value(attr) {
            out.push((name, value));
        }
    }
    out
}

/// Resolve a SubjectMembership's declared type element.
///
/// Prefers a resolved `type` Ref (set by reference resolution), falling back to
/// the parser-stamped `unresolved_type` name looked up among definition kinds.
fn resolve_subject_type<'g>(subject: &Element, graph: &'g ModelGraph) -> Option<&'g Element> {
    if let Some(Value::Ref(id)) = subject.props.get("type") {
        if let Some(target) = graph.get_element(id) {
            return Some(target);
        }
    }
    let name = subject
        .get_prop("unresolved_type")
        .and_then(|v| v.as_str())?;
    // `graph.elements` is a HashMap, so `.values().find(..)` picks a match in
    // hash-iteration order — nondeterministic per process when the name has
    // more than one structural candidate (e.g. a `def` and a same-named
    // `usage`). Collect every candidate and pick the lowest ElementId so the
    // subject-type binding is stable across runs. Prefer the resolver-stamped
    // `type` Ref above (deterministic); this fallback only runs when the type
    // reference never resolved.
    graph
        .elements
        .values()
        .filter(|e| {
            e.name.as_deref() == Some(name)
                // A subject may be typed by `Anything` (Cases.sysml:24) — accept any
                // structural definition/usage by name, not a membership/relationship.
                && (e.kind.as_str().ends_with("Definition") || e.kind.as_str().ends_with("Usage"))
        })
        .min_by(|a, b| a.id.cmp(&b.id))
}

/// Extract a concrete literal value from a subject attribute, ignoring
/// non-literal kinds (Ref/Null/List) so a value-less attribute leaves its name
/// unbound — keeping a subject-referencing constraint honestly Inconclusive
/// rather than masking the gap.
fn subject_literal_value(attr: &Element) -> Option<Value> {
    match attr.get_prop("value").or_else(|| attr.get_prop("default")) {
        Some(v @ (Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::String(_))) => {
            Some(v.clone())
        }
        _ => None,
    }
}

/// Resolve the constraint expression referenced by a `require constraint : Def;`
/// membership (the reference form, which carries no inline body).
///
/// The ast-builder stamps the referenced definition's name on the membership as
/// `referencedConstraint`; we look the definition up and return its expression
/// (pretty-printed structured form, else the legacy `constraint` string prop).
fn resolve_referenced_constraint_expr(membership: &Element, graph: &ModelGraph) -> Option<String> {
    // The reference form's target, resolved from its FeatureTyping (`: Def`) or
    // ReferenceSubsetting (bare-name) — SysML-vocab.ttl:2576, identity not a
    // name lookup. Only a constraint kind carries a compilable expression.
    let def = sysml_core::query::referenced_constraint_target(membership, graph)?;
    if !matches!(
        def.kind,
        ElementKind::ConstraintDefinition | ElementKind::ConstraintUsage
    ) {
        return None;
    }
    sysml_core::expression_pretty::pretty_print_owner(def, graph).or_else(|| {
        def.get_prop("constraint")
            .or_else(|| def.get_prop("expr"))
            .and_then(|v| v.as_str().map(str::to_owned))
    })
}

/// Resolve the requirement a bare `verify Req;` clause names.
///
/// `RequirementVerificationMembership` carries the verified requirement's name in
/// the `verifiedRequirement` prop (the ast-builder lowering of the `verify`
/// target). We look it up among requirement kinds.
fn resolve_verified_requirement_target<'g>(
    clause: &Element,
    graph: &'g ModelGraph,
) -> Option<&'g Element> {
    let name = clause
        .get_prop("verifiedRequirement")
        .and_then(|v| v.as_str())?;
    graph.elements.values().find(|e| {
        e.name.as_deref() == Some(name)
            && matches!(
                e.kind,
                ElementKind::RequirementDefinition | ElementKind::RequirementUsage
            )
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::cases::test_support::*;
    use sysml_core::{Element, ElementId, ElementKind, ModelGraph, Value};

    #[test]
    fn compile_verification_case_from_graph() {
        let graph = build_verification_graph();
        let result = compile_verification_case("SpeedCheck", &graph);
        assert!(result.is_ok(), "compilation should succeed");
        let vc = result.unwrap();
        assert_eq!(vc.name, "SpeedCheck");
        assert_eq!(vc.subject.as_deref(), Some("vehicle"));
        assert_eq!(vc.requirements.len(), 1);
        assert_eq!(vc.requirements[0].id, "speed-limit");
        assert_eq!(
            vc.requirements[0].text.as_deref(),
            Some("Speed must be under limit")
        );
        // The constraint should have been compiled from the expression string
        assert_eq!(vc.requirements[0].constraints.len(), 1);
    }

    #[test]
    fn compile_use_case_from_graph() {
        let mut graph = ModelGraph::new();

        // Add use case
        let uc_id = ElementId::new_v4();
        let uc = Element::new(uc_id.clone(), ElementKind::UseCaseDefinition)
            .with_name("DriveVehicle")
            .with_prop("subject", Value::String("vehicle".into()))
            .with_prop("objective", Value::String("Transport cargo".into()));
        graph.add_element(uc);

        // Add actor membership
        let actor = Element::new(ElementId::new_v4(), ElementKind::ActorMembership)
            .with_name("driver")
            .with_owner(uc_id.clone());
        graph.add_element(actor);

        // Add an included sub-use-case
        let inc = Element::new(ElementId::new_v4(), ElementKind::IncludeUseCaseUsage)
            .with_name("StartEngine")
            .with_owner(uc_id.clone());
        graph.add_element(inc);

        let result = compile_use_case("DriveVehicle", &graph);
        assert!(result.is_ok(), "compilation should succeed");
        let compiled = result.unwrap();
        assert_eq!(compiled.name, "DriveVehicle");
        assert_eq!(compiled.subject.as_deref(), Some("vehicle"));
        assert_eq!(compiled.objective.as_deref(), Some("Transport cargo"));
        assert_eq!(compiled.actors, vec!["driver"]);
        assert_eq!(compiled.includes.len(), 1);
        assert_eq!(compiled.includes[0].name, "StartEngine");
    }

    #[test]
    fn compile_requirement_checks_test() {
        let mut graph = ModelGraph::new();

        let req1 = Element::new(ElementId::new_v4(), ElementKind::RequirementDefinition)
            .with_name("req-safety")
            .with_prop("text", Value::String("System must be safe".into()))
            .with_prop("constraint", Value::String("error_rate < 1".into()));
        graph.add_element(req1);

        let req2 = Element::new(ElementId::new_v4(), ElementKind::RequirementUsage)
            .with_name("req-perf")
            .with_prop("text", Value::String("Must be fast".into()))
            .with_prop("constraint", Value::String("latency < 100".into()));
        graph.add_element(req2);

        let checks = compile_requirement_checks(&graph);
        assert_eq!(checks.len(), 2);

        let names: Vec<_> = checks.iter().map(|c| c.id.as_str()).collect();
        assert!(names.contains(&"req-safety"));
        assert!(names.contains(&"req-perf"));

        // Each should have one compiled constraint
        for check in &checks {
            assert_eq!(check.constraints.len(), 1);
        }
    }

    #[test]
    fn compile_actors_test() {
        let mut graph = ModelGraph::new();

        let a1 = Element::new(ElementId::new_v4(), ElementKind::ActorMembership).with_name("pilot");
        graph.add_element(a1);

        let a2 =
            Element::new(ElementId::new_v4(), ElementKind::ActorMembership).with_name("copilot");
        graph.add_element(a2);

        // Non-actor element should not be included
        let other = Element::new(ElementId::new_v4(), ElementKind::PartUsage).with_name("engine");
        graph.add_element(other);

        let actors = compile_actors(&graph);
        assert_eq!(actors.len(), 2);
        assert!(actors.contains(&"pilot".to_string()));
        assert!(actors.contains(&"copilot".to_string()));
    }

    #[test]
    fn compile_analysis_case_with_tool_metadata() {
        let mut graph = ModelGraph::new();

        // Create an analysis case
        let ac = Element::new_with_kind(ElementKind::AnalysisCaseDefinition)
            .with_name("ThermalAnalysis")
            .with_prop("subject", Value::String("engine".into()))
            .with_prop("objective", Value::String("compute heat".into()));
        let ac_id = graph.add_element(ac);

        // Add ToolExecution metadata
        let meta = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_name("ToolExecution")
            .with_owner(ac_id.clone())
            .with_prop("unresolvedTypeName", "ToolExecution");
        let meta_id = graph.add_element(meta);

        let tool_name_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("toolName")
            .with_owner(meta_id.clone())
            .with_prop("default", "builtin:bisection");
        graph.add_element(tool_name_attr);

        let uri_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("uri")
            .with_owner(meta_id)
            .with_prop("default", "https://example.com/solver");
        graph.add_element(uri_attr);

        // Add a parameter with direction
        let param = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("temperature")
            .with_owner(ac_id.clone())
            .with_prop("direction", "in")
            .with_prop("default", Value::Float(300.0));
        graph.add_element(param);

        // Add a constraint
        let constraint = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("temp_limit")
            .with_owner(ac_id.clone())
            .with_prop("constraint", "temperature < 500");
        graph.add_element(constraint);

        let result = compile_analysis_case("ThermalAnalysis", &graph);
        assert!(
            result.is_ok(),
            "compilation should succeed: {:?}",
            result.err()
        );
        let ir = result.unwrap();

        assert_eq!(ir.name, "ThermalAnalysis");
        assert_eq!(ir.tool_name.as_deref(), Some("builtin:bisection"));
        assert_eq!(ir.tool_uri.as_deref(), Some("https://example.com/solver"));
        assert_eq!(ir.parameters.len(), 1);
        assert_eq!(ir.parameters[0].sysml_name, "temperature");
        assert_eq!(ir.constraints.len(), 1);
        assert_eq!(ir.constraints[0].expr, "temperature < 500");
    }

    #[test]
    fn compile_analysis_case_without_metadata() {
        let mut graph = ModelGraph::new();

        // Bare analysis case with no metadata or parameters
        let ac =
            Element::new_with_kind(ElementKind::AnalysisCaseDefinition).with_name("SimpleAnalysis");
        graph.add_element(ac);

        let result = compile_analysis_case("SimpleAnalysis", &graph);
        assert!(result.is_ok());
        let ir = result.unwrap();
        assert_eq!(ir.name, "SimpleAnalysis");
        assert!(ir.tool_name.is_none());
        assert!(ir.tool_uri.is_none());
        assert!(ir.parameters.is_empty());
        assert!(ir.constraints.is_empty());
        assert!(ir.result_expression.is_none());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn verdict_from_value_converts_primitives() {
        let v = Verdict::from_value(VerdictKind::Pass, Value::Int(42));
        assert_eq!(v.actual, Some(serde_json::json!(42)));

        let v = Verdict::from_value(VerdictKind::Pass, Value::Bool(true));
        assert_eq!(v.actual, Some(serde_json::json!(true)));

        let v = Verdict::from_value(VerdictKind::Pass, Value::String("hi".into()));
        assert_eq!(v.actual, Some(serde_json::json!("hi")));

        let v = Verdict::from_value(VerdictKind::Pass, Value::Float(2.5));
        assert_eq!(v.actual, Some(serde_json::json!(2.5)));
    }

    #[test]
    fn verify_chain_compiles_constraint_from_referenced_requirement() {
        // B14: compile_verification_case must follow `verify BrewTempReq` →
        // `require constraint : BrewTempConstraint` → the actual expression,
        // so that evaluation produces a real verdict instead of
        // "no compilable expression children".
        let (graph, _vc_id) = build_verify_chain_graph();

        let ir =
            compile_verification_case("BrewTempTest", &graph).expect("verification case compiles");

        assert_eq!(ir.name, "BrewTempTest");
        assert_eq!(ir.requirements.len(), 1, "objective requirement extracted");

        let objective_req = &ir.requirements[0];
        assert!(
            objective_req.compile_errors.is_empty(),
            "verify chain must not leave compile errors, got {:?}",
            objective_req.compile_errors,
        );
        assert_eq!(
            objective_req.constraints.len(),
            1,
            "the referenced BrewTempConstraint's expression must be picked up",
        );
    }
}
