//! Gated / signal expression tables and expression-owner helpers used by
//! `ModelCompiler`.

use std::collections::HashMap;
use std::sync::Arc;

use sysml_core::{ElementId, ElementKind, ModelGraph, Value};

use crate::expressions::ExprIR;
use crate::ode_builder;
use crate::orchestrator::Orchestrator;

use super::*;

/// AST-first lookup of an element's expression body as a pretty-printed
/// string. Falls back to the legacy `result` / `expr` string props for
/// hand-crafted (test) graphs and runtime-synthesized elements that bypass
/// the parser pipeline. (`unresolved_value` is no longer read here — neither
/// parser has written it since Phase 6D.)
pub(crate) fn owner_expression_text(
    element: &sysml_core::Element,
    graph: &ModelGraph,
) -> Option<String> {
    sysml_core::expression_pretty::pretty_print_owner(element, graph).or_else(|| {
        // Legacy fallback for test graphs that bypass the parser pipeline.
        // Note: `unresolved_value` is no longer written by either parser (Phase 6D).
        element
            .get_prop("result")
            .or_else(|| element.get_prop("expr"))
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
    })
}

/// True when the element carries a structured expression subtree.
pub(crate) fn has_expression_subtree(element: &sysml_core::Element, graph: &ModelGraph) -> bool {
    sysml_core::expression_pretty::pretty_print_owner(element, graph).is_some()
}

/// True when an attribute's value child is a *computed expression* (a binding to
/// evaluate each step) rather than a literal value or fixed default. The
/// value-child node kind is the reliable signal — `isDefault` is not (see
/// [`OdeAttrRole`]).
///
/// - `OperatorExpression` / `InvocationExpression` → always computed (e.g.
///   `a * b`, `x >= y`, `f(x)`). This is the arm that newly captures the
///   `out attribute r = H_dc/H_ref` oracle and the derived booleans
///   (`tripped`, `magneticTripped`) that the old `isDefault` filter dropped.
/// - `FeatureReferenceExpression` / `FeatureChainExpression` → a read of another
///   feature (`config.x`, a `someUnit`/`one` reference, or an enum literal
///   `BreakerCurveType::C`). NEVER computed: a bare feature read is a binding /
///   parameter resolved at build time (config overlays, unit aliases, enum
///   defaults), not a per-step ODE equation. This keeps config bindings like
///   `attribute ratedCurrent = config.ratedCurrent` as External parameters and
///   stops stdlib unit aliases (`byte = one`) being swept into the computed set.
///   (Pre-G22 the discriminant was `!isDefault`, but the parser stamped
///   `isDefault=true` on every `=` binding, so feature reads were never computed
///   in practice; G22 fixed the flag, so the node kind is now the honest signal.)
/// - `Literal*` child or no value child → never computed.
pub(crate) fn is_computed_value_child(element: &sysml_core::Element, graph: &ModelGraph) -> bool {
    for child in graph.children_of(&element.id) {
        match child.kind {
            ElementKind::OperatorExpression | ElementKind::InvocationExpression => return true,
            ElementKind::FeatureReferenceExpression | ElementKind::FeatureChainExpression => {
                // A bare feature read (`= config.x`, `= someUnit`, an enum
                // literal) is a binding/parameter resolved at build time — NOT a
                // per-step computed ODE equation. (Pre-G22 this rode on the buggy
                // `isDefault=true` that the parser stamped on every `=` binding,
                // so feature reads were never computed. With G22 fixed, `=`
                // correctly yields isDefault=false; key on the value-child node
                // kind — only an Operator/Invocation expression is a computed
                // equation — instead of the now-correct, and here irrelevant,
                // isDefault flag.)
                return false;
            }
            _ => {}
        }
    }
    // Legacy fallback: hand-crafted test graphs carry the expression as an
    // `expr`/`result` string prop with no AST subtree (mirrors
    // `owner_expression_text`'s fallback). Treat a non-numeric such string as a
    // computed binding so those graphs keep working without the parser.
    for key in ["expr", "result"] {
        if let Some(s) = element.get_prop(key).and_then(|v| v.as_str()) {
            if s.trim().parse::<f64>().is_err() {
                return true;
            }
        }
    }
    false
}

/// Test-visible wrapper over [`extract_calc_result_expr`], which is
/// `pub(crate)`. Exists so `tests/calc_result_selection.rs` can pin the
/// selection rule directly instead of inferring it from a simulated value.
pub fn extract_calc_result_expr_for_test(
    graph: &ModelGraph,
    element: &sysml_core::Element,
) -> Option<String> {
    extract_calc_result_expr(graph, element)
}

/// The `ReturnParameterMembership` child of a calc — the element that IS its
/// result.
///
/// KerML gives a calculation exactly one result, carried by this membership
/// (`return <name> = <expr>`). Everything else owned by the calc — bound input
/// parameters, local intermediates — is scaffolding, not the answer.
///
/// `children_of` yields an unordered hash set, so a calc with more than one
/// return membership would otherwise resolve arbitrarily; the lowest element
/// id makes the pick deterministic rather than layout-dependent. A well-formed
/// calc has exactly one, so the tie-break never fires in practice.
pub(crate) fn calc_return_member<'g>(
    graph: &'g ModelGraph,
    calc: &sysml_core::Element,
) -> Option<&'g sysml_core::Element> {
    graph
        .children_of(&calc.id)
        .filter(|c| c.kind == ElementKind::ReturnParameterMembership)
        .min_by(|a, b| a.id.as_str().cmp(&b.id.as_str()))
}

/// Name of a calc's return parameter (`"dTdt"` from `return dTdt = …`), used
/// to match a calc to the state variable it computes.
///
/// ONE home. This walk used to be open-coded at seven call sites in
/// `ode_detection`, each taking `children_of(...).find(has an expression)` —
/// a `find` over an UNORDERED hash set. Any calc owning more than one
/// expression-bearing child therefore matched on whichever the hasher happened
/// to visit first. See [`extract_calc_result_expr`] for what that cost.
pub(crate) fn calc_return_name(graph: &ModelGraph, calc: &sysml_core::Element) -> String {
    if let Some(ret) = calc_return_member(graph, calc) {
        if let Some(name) = ret.name.clone() {
            return name;
        }
    }
    // Legacy shape: hand-built test graphs carry the result on a plain child
    // with a `result`/`expr` prop and no return membership. Ordered by id so
    // the pick is at least stable.
    let mut candidates: Vec<&sysml_core::Element> = graph
        .children_of(&calc.id)
        .filter(|c| has_expression_subtree(c, graph) || c.get_prop("result").is_some())
        .collect();
    candidates.sort_by(|a, b| a.id.as_str().cmp(&b.id.as_str()));
    candidates
        .first()
        .and_then(|c| c.name.clone())
        .unwrap_or_default()
}

/// Extract a calc-def-style result expression from an element.
///
/// Order: the element's own expression subtree, then its
/// `ReturnParameterMembership`, then a legacy any-child walk.
///
/// **The middle step is the fix.** This function used to go straight from the
/// element to `children_of(...).find_map(has an expression)` — a `find_map`
/// over an unordered hash set. `examples/damped-oscillator`'s `getNextState`
/// owns FOUR expression-bearing children:
///
/// ```text
///   timeStep    0.001                                   <- what it picked
///   stateSpace  dynamics::stateSpace
///   result      x + timeStep * v_next                   <- the actual result
///   v_next      v + timeStep * (-2.0 * zeta * omega * v - omega * omega * x)
/// ```
///
/// so the "next state" of that model was the bound value of an input
/// parameter, and every sweep over it returned a constant 0.001 whatever the
/// damping ratio. Nothing failed; the numbers just were not the model's.
///
/// Every OTHER SSR fixture in the corpus writes a calc with exactly one
/// expression-bearing child, so the arbitrary pick landed correctly by luck —
/// which is why a green corpus never surfaced this. Binding an input
/// (`in timeStep = 0.001`) or declaring a local intermediate is ordinary,
/// legal SysML; the corpus simply never did it.
pub(crate) fn extract_calc_result_expr(graph: &ModelGraph, element: &sysml_core::Element) -> Option<String> {
    if let Some(text) = owner_expression_text(element, graph) {
        return Some(text);
    }
    if let Some(ret) = calc_return_member(graph, element) {
        if let Some(text) = owner_expression_text(ret, graph) {
            return Some(text);
        }
    }

    // Legacy fallback for graphs with no return membership (hand-built test
    // graphs, and calc shapes the parser does not yet lower). Ordered by id so
    // an ambiguous pick is at least reproducible.
    let mut candidates: Vec<&sysml_core::Element> = graph.children_of(&element.id).collect();
    candidates.sort_by(|a, b| a.id.as_str().cmp(&b.id.as_str()));
    candidates.into_iter().find_map(|child| {
        owner_expression_text(child, graph).or_else(|| {
            child
                .get_prop("default")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        })
    })
}

/// RSC-4.2 (C.4): one entry in the cached gated/computed-expression bundle.
///
/// Carries the instance `scope_prefix` alongside the parsed (BARE, i.e. not
/// text-prefixed) [`ExprIR`] so the orchestrator's `bind_expression_slots`
/// pass can bind each instance-scoped RHS to *its instance's* slots via
/// [`SlotBinder::for_subsystem`](crate::expressions::SlotBinder::for_subsystem).
/// This replaces the deleted string-level `prefix_expression_identifiers`
/// rewrite with collision-safe IR-level slot binding.
#[derive(Debug, Clone)]
pub struct GatedExprSpec {
    /// Fully-qualified target variable (`{prefix}.{attr}` for instance-scoped
    /// entries; the authored name for orchestrator-scope entries).
    pub target: String,
    /// Parsed RHS. For instance-scoped entries this is the BARE
    /// instance-local expression (`bimetalTemp >= config.tripTemperature`),
    /// bound to the instance's slots at orchestrator-assembly time.
    pub expr: ExprIR,
    /// `Some(prefix)` for instance-multiplied expressions; `None` for
    /// orchestrator-scope expressions (aggregates, top-level `= expr`).
    pub scope_prefix: Option<String>,
}

/// Build the cached list of gated/computed expressions for a model graph.
///
/// Pure graph derivative — combines `ModelCompiler::detect_computed_expressions`
/// with `extract_instance_scoped_pairs` and parses each entry to [`ExprIR`].
/// Entries whose RHS fails `ode_builder::parse_derivative` are dropped (the
/// in-place path drops them too). Instance-scoped entries carry their prefix
/// (RSC-4.2 C.4) instead of being text-prefixed.
pub fn build_gated_expressions(graph: &ModelGraph) -> Vec<GatedExprSpec> {
    let mut out = Vec::new();

    for (target, expr_str) in ModelCompiler::detect_computed_expressions(graph) {
        if let Ok(expr) = ode_builder::parse_derivative(&expr_str) {
            out.push(GatedExprSpec {
                target,
                expr,
                scope_prefix: None,
            });
        }
    }

    for (target, expr_str, prefix) in extract_instance_scoped_pairs(graph) {
        if let Ok(expr) = ode_builder::parse_derivative(&expr_str) {
            out.push(GatedExprSpec {
                target,
                expr,
                scope_prefix: Some(prefix),
            });
        }
    }

    out
}

/// Graph-only mirror of `expand_part_instances` + `detect_instance_scoped_expressions`
/// that omits the reachable-SM/ODE filter. See the module-level note on
/// gated-expression extraction for the divergence rationale.
fn extract_instance_scoped_pairs(graph: &ModelGraph) -> Vec<(String, String, String)> {
    use std::collections::HashMap;

    // Container scan: for each PartDefinition/PartUsage, find its named
    // PartUsage children, group by resolved type name, keep groups with ≥2
    // instances. Mirrors `expand_part_instances` minus the SM/ODE filter.
    let mut type_to_prefixes: HashMap<String, Vec<String>> = HashMap::new();
    for container in graph.elements.values() {
        if !matches!(
            container.kind,
            ElementKind::PartDefinition | ElementKind::PartUsage
        ) {
            continue;
        }
        // Standard-library structure is not user-model simulation content —
        // skip it for the same reason `detect_computed_expressions` does (a
        // multiplied library PartDefinition would otherwise leak its template
        // `= expr` attributes, whose RHS reference calc-locals / unit symbols
        // that resolve to no runtime slot → RS003).
        if graph.is_library_element(&container.id) {
            continue;
        }

        let named_children: Vec<(String, Option<String>)> = graph
            .children_of(&container.id)
            .filter(|c| c.kind == ElementKind::PartUsage && c.name.is_some())
            .map(|c| {
                let name = c.name.as_ref().expect("name checked").clone();
                let type_name = c
                    .get_prop("unresolvedTypeName")
                    .or_else(|| c.get_prop("unresolved_type"))
                    .and_then(|v| v.as_str().map(|s| s.to_owned()))
                    .or_else(|| {
                        graph
                            .children_of(&c.id)
                            .find(|ft| ft.kind == ElementKind::FeatureTyping)
                            .and_then(|ft| {
                                ft.get_prop("unresolved_type")
                                    .and_then(|v| v.as_str().map(|s| s.to_owned()))
                            })
                    });
                (name, type_name)
            })
            .collect();

        if named_children.len() < 2 {
            continue;
        }

        let mut local: HashMap<String, Vec<String>> = HashMap::new();
        for (name, type_name) in named_children {
            if let Some(tn) = type_name {
                local.entry(tn).or_default().push(name);
            }
        }

        for (type_name, prefixes) in local {
            if prefixes.len() < 2 {
                continue;
            }
            type_to_prefixes
                .entry(type_name)
                .or_default()
                .extend(prefixes);
        }
    }

    // For each multiplied type, find the PartDefinition and walk its
    // computed-attribute children. Mirrors `detect_instance_scoped_expressions`.
    let mut out = Vec::new();
    for (type_name, prefixes) in &type_to_prefixes {
        let type_def = graph.elements.values().find(|e| {
            e.name.as_deref() == Some(type_name.as_str())
                && matches!(e.kind, ElementKind::PartDefinition)
        });
        let Some(td) = type_def else { continue };
        if graph.is_library_element(&td.id) {
            continue;
        }

        let computed_attrs: Vec<(String, String)> = graph
            .children_of(&td.id)
            .filter_map(|child| {
                if child.kind != ElementKind::AttributeUsage {
                    return None;
                }
                let name = child.name.as_ref()?;
                let expr_str: String = owner_expression_text(child, graph)?;
                if child.get_prop("value").is_some() {
                    return None;
                }
                Some((name.to_owned(), expr_str))
            })
            .collect();

        for (attr_name, expr_str) in &computed_attrs {
            for prefix in prefixes {
                let target = format!("{}.{}", prefix, attr_name);
                // RSC-4.2 (C.4): keep the BARE instance-local expression and
                // carry the prefix; the orchestrator binds it to this
                // instance's slots (was: text-prefix every identifier).
                out.push((target, expr_str.clone(), prefix.clone()));
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Signal expression table (S3.T12 cached_signal_expr_table, ADR-011 §3 RT-35).
//
// `RuntimeSession.signal_exprs` holds parsed `(param_name, ExprIR)` pairs used
// to keep time-varying display values in sync with the orchestrator context.
// Today the service builds the table inline at three call sites: it constructs
// a `Snapshot` (which builds a `ModelCompiler`), calls `snap.detect_ode()` to
// get the first `OdeDetection`, then iterates `ode.signal_exprs`
// (HashMap<String, String>) and parses each value to `ExprIR` via
// `ode_builder::parse_derivative`. Both the ODE detection walk and the
// per-entry parse are pure graph derivatives.
//
// The cached upstream
// (`sysml_ide_db::signal_expr_table::workspace_signal_expr_table_with_library`
// and siblings) calls `build_signal_expr_table` once per elaborated-graph
// revision and stores the parsed `Vec<(String, ExprIR)>`. Subsequent
// session-start calls hit the salsa cache and skip both ODE detection and
// per-entry parsing.
//
// The first-call cost includes a `ModelCompiler::from_arc` (defensive
// re-elaborate, deep-clones the graph) plus `detect_all_odes_unified`
// (a walk over the elaborated graph). A future ADR-011 step lifts the
// defensive re-elaborate by routing through `Snapshot::compile_*`; after
// that the per-session-start cost is the salsa cache hit only.
//
// Mirrors the in-place behaviour at lib.rs `simulate.continuous.auto`
// exactly: returns the first ODE's signal expressions; an empty `Vec` when
// no ODE is detected (callers branch on `is_empty()`).
// ---------------------------------------------------------------------------

/// Extract the comparator expression text from an `accept when` trigger event
/// string, which the parser lowers to `"when <comparator>"` (space form) or
/// `"when(<comparator>)"` (parenthesized form). Returns `None` for any other
/// trigger (`after(...)`, `accept via ...`, plain event names). Used by
/// [`ModelCompiler::wire_zero_crossing_detectors`] to recognize candidate
/// crossing-located triggers.
pub(crate) fn when_comparator(event: &str) -> Option<&str> {
    let trimmed = event.trim();
    if let Some(rest) = trimmed.strip_prefix("when(") {
        return rest.strip_suffix(')').map(str::trim);
    }
    trimmed.strip_prefix("when ").map(str::trim)
}

/// Build a [`DutyCycleTracker`] from the comparator crossings registered for one
/// (ODE, SM) pair (WS-D Stage 2, SPEC-SILENT).
///
/// `cmp_meta` is `(event_name, direction, comparator_var, is_output_signal)` per
/// located crossing. A square-wave comparator is a variable carrying BOTH a
/// `Rising` (↑ upper threshold) and a `Falling` (↓ lower threshold) crossing.
/// When several qualify, an output signal wins over a bare state var — the
/// firmware measures the drive signal (e.g. `i_drive`), not the saturation
/// safety limit on the state (`B`). Returns `None` when no variable forms a
/// complete ±threshold comparator (no duty observable for that pair).
pub(crate) fn build_duty_tracker(
    cmp_meta: &[(String, crate::ode_events::CrossingDirection, String, bool)],
) -> Option<crate::ode_events::DutyCycleTracker> {
    use crate::ode_events::CrossingDirection;
    use std::collections::BTreeMap;

    // var -> (rising event, falling event, is_signal). BTreeMap for a
    // deterministic candidate order (build determinism, WS-C).
    let mut by_var: BTreeMap<&str, (Option<&str>, Option<&str>, bool)> = BTreeMap::new();
    for (event, dir, var, on_signal) in cmp_meta {
        let entry = by_var.entry(var.as_str()).or_insert((None, None, false));
        entry.2 |= *on_signal;
        match dir {
            CrossingDirection::Rising => entry.0 = Some(event.as_str()),
            CrossingDirection::Falling => entry.1 = Some(event.as_str()),
            CrossingDirection::Either => {}
        }
    }

    // Candidates with a complete comparator; output signals first.
    let mut chosen: Option<(&str, &str)> = None;
    let mut chosen_on_signal = false;
    for (rise, fall, on_signal) in by_var.values() {
        if let (Some(r), Some(f)) = (rise, fall) {
            if chosen.is_none() || (*on_signal && !chosen_on_signal) {
                chosen = Some((r, f));
                chosen_on_signal = *on_signal;
            }
        }
    }

    chosen.map(|(rise, fall)| crate::ode_events::DutyCycleTracker::new(rise, fall))
}

/// Build the cached signal-expression table for a model graph.
///
/// Pure graph derivative. Returns the parsed `(param_name, ExprIR)` pairs
/// for the first detected ODE's `signal_exprs`. Entries whose RHS fails
/// `ode_builder::parse_derivative` are dropped (the in-place path drops
/// them too). Returns an empty `Vec` when no ODE is detected.
pub fn build_signal_expr_table(graph: &ModelGraph) -> Vec<(String, ExprIR)> {
    // `ModelCompiler::from_arc` deep-clones the graph internally for its
    // defensive re-elaborate (the cost lifted by S3.T7). Wrapping the
    // borrowed reference in a fresh `Arc` here just gives `from_arc` an
    // owned handle to clone from — the per-cache-miss cost is one
    // graph clone + one elaborate, both memoised by the salsa cache.
    let compiler = ModelCompiler::from_arc(Arc::new(graph.clone()));
    let Some(ode) = compiler.detect_ode() else {
        return Vec::new();
    };
    ode.signal_exprs
        .iter()
        .filter_map(|(name, expr_str)| {
            ode_builder::parse_derivative(expr_str)
                .ok()
                .map(|ir| (name.clone(), ir))
        })
        .collect()
}

impl ModelCompiler {
    /// Detect computed expressions from SysML `= expr` bindings.
    ///
    /// Attributes with a structured expression subtree (parser-emitted from
    /// non-literal `= expr` bindings) are returned as (name, expression_string),
    /// with the expression text read AST-first via [`owner_expression_text`]
    /// (which falls back to the legacy `result` / `expr` string props for
    /// hand-crafted test graphs — never `unresolved_value`, which neither
    /// parser has written since Phase 6D).
    pub fn detect_computed_expressions(graph: &ModelGraph) -> Vec<(String, String)> {
        // A computed expression is an AttributeUsage classified as a
        // `DerivedBinding` by [`classify_ode_attr`] — i.e. a non-literal `= expr`
        // binding (possibly marked `out`), as opposed to a literal initial value
        // / parameter / state variable. Routing through the shared classifier is
        // what lets a derived `out` attribute (e.g. `out attribute r = H_dc/H_ref`)
        // be recomputed each step instead of being integrated as a phantom ODE
        // state. It also means this filter no longer keys on the parser's
        // `isDefault` flag, which is unreliable for `=` bindings (see the
        // SPEC-SILENT note on `OdeAttrRole`).
        graph
            .elements
            .values()
            .filter_map(|el| {
                if el.kind != ElementKind::AttributeUsage {
                    return None;
                }
                // Standard-library unit/quantity definitions are type-level
                // metadata, NOT runtime per-tick computed expressions. The SI
                // library declares derived units as non-literal `= expr`
                // bindings (e.g. `attribute <Pa> pascal = N/m^2;`,
                // `attribute <Wb> weber = V*s;`) which `classify_ode_attr`
                // correctly tags `DerivedBinding`. Without this gate they would
                // be registered as orchestrator computed expressions whose RHS
                // references unit symbols (N, m, V, …) that resolve to no
                // runtime slot — an RS003 hard error for any model that
                // `import SI::*`. Scope collection to user-model elements, the
                // same way `constraints.rs` and `cases/health.rs` do. This
                // exclusion is intentional: a derived attribute authored inside
                // a *library* package is definitional metadata by assumption,
                // never an executable per-tick binding.
                let name = el.name.as_ref()?;
                if !matches!(classify_ode_attr(el, graph), OdeAttrRole::DerivedBinding) {
                    return None;
                }
                if graph.is_library_element(&el.id) {
                    return None;
                }
                // Skip collection-literal RHS like `= ((0,0.5), (1,1.0), ...)`
                // — these are SampledFunction data payloads, not scalar
                // computed equations. The parser emits them as a root
                // OperatorExpression with operator "," (sequence literal). They
                // reach here because a sequence literal is non-numeric, so the
                // classifier (correctly) calls them a binding.
                let root_is_sequence = graph
                    .children_of(&el.id)
                    .filter(|c| c.kind == ElementKind::OperatorExpression)
                    .any(|c| c.get_prop("operator").and_then(|v| v.as_str()) == Some(","));
                if root_is_sequence {
                    return None;
                }
                // AST-first source for the expression text; falls back to
                // legacy string props for hand-crafted test graphs.
                let expr_str: String = owner_expression_text(el, graph)?;
                Some((name.to_owned(), expr_str))
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Generic model-driven wiring (replaces hardcoded per-model functions)
    // -----------------------------------------------------------------------

    /// Detect aggregate expressions from instance-scoped computed attributes.
    ///
    /// For each part definition that gets instance-multiplied, finds attributes
    /// with `= expr` bindings inside the definition. These are duplicated for
    /// each instance with the appropriate prefix.
    ///
    /// Example: `CircuitPath { attribute tripped = bimetalTemp >= config.tripTemperature; }`
    /// For instances circuit1..circuit10, generates:
    ///   `circuit1.tripped = circuit1.bimetalTemp >= circuit1.config.tripTemperature`
    ///   `circuit2.tripped = circuit2.bimetalTemp >= circuit2.config.tripTemperature`
    ///   ...
    /// Returns the target variable names of every expression added, for
    /// slot minting (RSC-2.1).
    pub(crate) fn detect_instance_scoped_expressions(
        &self,
        orchestrator: &mut Orchestrator,
        instances: &[InstanceSpec],
    ) -> Vec<String> {
        let mut added_targets: Vec<String> = Vec::new();
        // Collect type names that are instance-multiplied
        let mut type_to_prefixes: HashMap<String, Vec<&str>> = HashMap::new();
        for inst in instances {
            // Find the type of this instance
            for elem in self.graph.elements.values() {
                if elem.kind != ElementKind::PartUsage {
                    continue;
                }
                if elem.name.as_deref() != Some(inst.prefix.as_str()) {
                    continue;
                }
                let type_name = elem
                    .get_prop("unresolvedTypeName")
                    .or_else(|| elem.get_prop("unresolved_type"))
                    .and_then(|v| v.as_str().map(|s| s.to_owned()))
                    .or_else(|| {
                        self.graph
                            .children_of(&elem.id)
                            .find(|ft| ft.kind == ElementKind::FeatureTyping)
                            .and_then(|ft| {
                                ft.get_prop("unresolved_type")
                                    .and_then(|v| v.as_str().map(|s| s.to_owned()))
                            })
                    });
                if let Some(tn) = type_name {
                    type_to_prefixes.entry(tn).or_default().push(&inst.prefix);
                }
                break;
            }
        }

        // For each multiplied type, find computed expressions in the definition
        for (type_name, prefixes) in &type_to_prefixes {
            let type_def = self.graph.elements.values().find(|e| {
                e.name.as_deref() == Some(type_name.as_str())
                    && matches!(e.kind, ElementKind::PartDefinition)
            });
            let Some(td) = type_def else { continue };

            // Find attributes with computed expressions (AST-first).
            let computed_attrs: Vec<(String, String)> = self
                .graph
                .children_of(&td.id)
                .filter_map(|child| {
                    if child.kind != ElementKind::AttributeUsage {
                        return None;
                    }
                    let name = child.name.as_ref()?;
                    let expr_str: String = owner_expression_text(child, &self.graph)?;
                    // Skip if it has a literal value (literal takes precedence)
                    if child.get_prop("value").is_some() {
                        return None;
                    }
                    Some((name.to_owned(), expr_str))
                })
                .collect();

            // RSC-4.2 (C.4): register the BARE instance-local expression as an
            // instance-scoped computed expression. `bind_expression_slots`
            // resolves its local names (`bimetalTemp`, `config.tripTemperature`)
            // to THIS instance's slots via `SlotBinder::for_subsystem(prefix)`
            // and it is evaluated against a scoped read view — collision-safe
            // without text-prefixing the string (the deleted
            // `prefix_expression_identifiers`). Byte-identical: the same slots
            // are bound as the pre-cull `{prefix}.{ident}` global binding.
            for (attr_name, expr_str) in &computed_attrs {
                for prefix in prefixes {
                    let target = format!("{}.{}", prefix, attr_name);
                    if let Ok(expr) = ode_builder::parse_derivative(expr_str) {
                        orchestrator.add_instance_computed_expression(&target, expr, *prefix);
                        added_targets.push(target);
                    }
                }
            }
        }

        added_targets
    }

    /// Walk the AST of an `elements` attribute whose RHS is a sequence of
    /// `(domain, range)` tuples and append each pair to `domain` / `range`.
    ///
    /// Expected shape (emitted by sysml-parser-batch's SequenceExpression
    /// handler): the attribute carries a single child `OperatorExpression`
    /// with `operator = ","`, whose children are themselves
    /// `OperatorExpression(",")` nodes with exactly two numeric-literal
    /// operands. Single-pair assignments (`= (5, 10)`) skip the outer layer
    /// and appear directly as a 2-operand `OperatorExpression(",")`.
    pub(crate) fn extract_tuple_pairs_from_ast(
        graph: &ModelGraph,
        attr_id: &ElementId,
        domain: &mut Vec<Value>,
        range: &mut Vec<Value>,
    ) {
        fn read_literal_number(elem: &sysml_core::Element) -> Option<f64> {
            elem.get_prop("value")
                .and_then(|v| v.as_float().or_else(|| v.as_int().map(|i| i as f64)))
        }

        fn ast_children_sorted<'a>(
            graph: &'a ModelGraph,
            id: &ElementId,
        ) -> Vec<&'a sysml_core::Element> {
            let mut children: Vec<&sysml_core::Element> = graph.children_of(id).collect();
            children.sort_by_key(|c| {
                c.get_prop("argIndex")
                    .and_then(|v| v.as_int())
                    .unwrap_or(i64::MAX)
            });
            children
        }

        /// Try to read a `(domain, range)` pair rooted at `elem`. Succeeds
        /// when `elem` is an `OperatorExpression(",")` with two numeric
        /// literal children.
        fn try_read_pair(graph: &ModelGraph, elem: &sysml_core::Element) -> Option<(f64, f64)> {
            if elem.kind != ElementKind::OperatorExpression {
                return None;
            }
            if elem.get_prop("operator").and_then(|v| v.as_str()) != Some(",") {
                return None;
            }
            let operands = ast_children_sorted(graph, &elem.id);
            // Filter out non-expression children (e.g., FeatureTyping).
            let operands: Vec<&sysml_core::Element> = operands
                .into_iter()
                .filter(|c| {
                    matches!(
                        c.kind,
                        ElementKind::LiteralInteger
                            | ElementKind::LiteralRational
                            | ElementKind::OperatorExpression
                            | ElementKind::LiteralString
                            | ElementKind::LiteralBoolean
                            | ElementKind::NullExpression
                    )
                })
                .collect();
            if operands.len() != 2 {
                return None;
            }
            let d = read_literal_number(operands[0])?;
            let r = read_literal_number(operands[1])?;
            Some((d, r))
        }

        // Look for an OperatorExpression(",") child of the attribute. This is
        // the sequence-literal root for the RHS.
        let root = graph.children_of(attr_id).find(|c| {
            c.kind == ElementKind::OperatorExpression
                && c.get_prop("operator").and_then(|v| v.as_str()) == Some(",")
        });
        let Some(root) = root else { return };

        let inner = ast_children_sorted(graph, &root.id);

        // Case 1: `= ((d,r), (d,r), ...)` — root's children are each a pair.
        let mut pushed_any = false;
        for c in &inner {
            if let Some((d, r)) = try_read_pair(graph, c) {
                domain.push(Value::Float(d));
                range.push(Value::Float(r));
                pushed_any = true;
            }
        }
        if pushed_any {
            return;
        }

        // Case 2: `= (d, r)` — root is itself a single pair.
        if let Some((d, r)) = try_read_pair(graph, root) {
            domain.push(Value::Float(d));
            range.push(Value::Float(r));
        }
    }

}
