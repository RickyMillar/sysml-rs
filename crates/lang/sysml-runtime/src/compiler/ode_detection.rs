//! ODE / SSR / composite dynamics detection and solver-selection metadata.

use std::collections::{HashMap, HashSet};

use sysml_core::element_ordering::sort_elements_by_source_order;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, Value};

use crate::expressions::ExprIR;
use crate::ode_builder;
use crate::orchestrator::SubsystemIndex;

use super::*;

/// Role of an ODE owner's child `AttributeUsage` in state-space detection.
///
/// The discriminant is the **value-child node kind** (see [`classify_ode_attr`]
/// and [`is_computed_value_child`]), NOT the parser's `isDefault` flag:
/// - `out`/`inout` with a literal IC, an `:=` initial, or no value → state var;
/// - a non-literal `= expr` binding (an `OperatorExpression`/`InvocationExpression`
///   value child) → a derived algebraic output recomputed each step;
/// - a numeric value → a fixed parameter.
///
/// Node-kind keying is deliberately robust to FeatureValue-flag semantics. It
/// predates and outlives grammar gap **G22** (now CLOSED — the ast_builder once
/// collapsed `=` and `default` into a single `isDefault=true`, contra
/// KerML.xtext:740-746; the parser now sets `isDefault`/`isInitial` per the
/// grammar). We never relied on that flag here, so the G22 fix needed no change
/// to this classifier. See
/// `Architectural-cleanup/tree-sitter-canonical-plan/grammar-gaps-inventory.md`
/// (G22, CLOSED).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OdeAttrRole {
    /// `out`/`inout` with a literal initial value, an `:=` initial, or no value
    /// at all — an ODE integration state variable. `initial` is the literal IC
    /// (0.0 when unspecified, or for a non-literal `:=` whose expression we do
    /// not yet evaluate at build time).
    StateVar { initial: f64 },
    /// A non-`in` attribute carrying a literal value — a fixed ODE parameter.
    Parameter { value: f64 },
    /// A `= expr` non-literal binding (possibly marked `out`) — a derived
    /// algebraic output. NOT integrated and NOT a parameter; recomputed every
    /// step via [`ModelCompiler::detect_computed_expressions`] →
    /// `Orchestrator::add_computed_expression`. This is the case that the old
    /// "any `out` attribute is a state variable" rule misclassified (it
    /// integrated `r = H_dc/H_ref` as a phantom state) and that the
    /// `isDefault`-keyed computed-expression filter wrongly dropped.
    DerivedBinding,
    /// `in` direction — a calc input. Collected into `input_names` by sites that
    /// track them; ignored by sites that don't.
    Input,
    /// Any attribute with no usable value — ignored by all collection.
    Ignore,
}

/// Classify an ODE owner's child `AttributeUsage` into the role that
/// state-space detection should assign it. See [`OdeAttrRole`] for the
/// spec-silent rationale behind the literal-vs-binding discriminant. This is
/// the single home for the decision — the per-detection-site collectors and
/// [`ModelCompiler::detect_computed_expressions`] all defer to it so the
/// classification can never drift between paths (RSC principle #5).
pub(crate) fn classify_ode_attr(child: &sysml_core::Element, graph: &ModelGraph) -> OdeAttrRole {
    let dir = child
        .get_prop("direction")
        .and_then(|v| v.as_str().map(|s| s.to_owned()));
    let numeric = child
        .get_prop("default")
        .or_else(|| child.get_prop("value"))
        .and_then(|v| v.as_float());
    let is_initial = child.get_prop("isInitial").and_then(|v| v.as_bool()) == Some(true);
    // A *binding* carries a genuine computed expression as its value child. The
    // value-child node KIND is the reliable signal (the parser's `isDefault`
    // flag is not — see the type doc): an `OperatorExpression` /
    // `InvocationExpression` is a computed equation (e.g. `H_dc / H_ref`,
    // `loadCurrent >= magneticThreshold * ratedCurrent`); a `Literal*` child is
    // a fixed value (`-0.597`, `"Bathroom"`, `BreakerCurveType::C` reaches us as a
    // literal-typed default); a bare `FeatureReferenceExpression` /
    // `FeatureChainExpression` (`config.x`, an enum literal) is a build-time
    // binding/parameter, never a per-step computed equation.
    let has_binding_expr = is_computed_value_child(child, graph);

    match dir.as_deref() {
        Some("out") | Some("inout") => {
            if has_binding_expr && numeric.is_none() && !is_initial {
                OdeAttrRole::DerivedBinding
            } else {
                OdeAttrRole::StateVar {
                    initial: numeric.unwrap_or(0.0),
                }
            }
        }
        Some("in") => OdeAttrRole::Input,
        _ => {
            if has_binding_expr && numeric.is_none() && !is_initial {
                OdeAttrRole::DerivedBinding
            } else if let Some(v) = numeric {
                OdeAttrRole::Parameter { value: v }
            } else {
                OdeAttrRole::Ignore
            }
        }
    }
}

/// Diagnostics for matching a dynamics subsystem's state variables to the calc
/// returns that compute them. ONE home for all five detection lanes.
///
/// The under-determined case is the one that mattered. Each lane wrote
/// `if calcs.len() == 1 { vec![the_one_expr; state_vars.len()] }` — broadcasting
/// a single result across EVERY state variable. For a one-state model that is
/// an identity and perfectly fine, which is why it survived: a census of the
/// example corpus (2026-08-19) found every dynamics subsystem has
/// `states == calcs`, so the broadcast only ever ran with N = 1.
///
/// `examples/damped-oscillator` is the exception — 2 state variables (`x`, `v`)
/// and one scalar `getNextState`. Broadcasting there silently gave both states
/// the same equation, and a zeta sweep returned five identical numbers.
///
/// The spec does not sanction that. `StateSpaceRepresentation` (Domain
/// Libraries/Analysis) declares `StateSpace :> VectorQuantityValue` and
/// `calc def GetNextState { …; return : StateSpace; }` — the result is the
/// WHOLE next-state vector, not one component to be replicated. A model
/// carrying N scalar state attributes against one scalar return is
/// under-determined, and inventing a broadcast to close the gap is exactly the
/// kind of made-up semantics that turns a modelling error into confident wrong
/// numbers. It is a hard error.
pub(crate) mod state_match {
    /// One result expression against N > 1 state variables — under-determined.
    pub(crate) fn under_determined(
        subsystem: &str,
        calc_kind: &str,
        state_vars: &[String],
    ) -> String {
        format!(
            "dynamics subsystem '{subsystem}' declares {} state variables ({}) but its \
             {calc_kind} returns 1 expression. StateSpaceRepresentation::{calc_kind} returns \
             the whole StateSpace vector; per-variable dynamics requires one return per state \
             variable. Give each state variable its own return, or model the state as a single \
             vector attribute.",
            state_vars.len(),
            state_vars.join(", "),
        )
    }

    /// A state variable that no return names — previously a silent constant-0
    /// difference, i.e. a state that just stops moving.
    pub(crate) fn unmatched(
        subsystem: &str,
        calc_kind: &str,
        var: &str,
        available: &[&str],
    ) -> String {
        format!(
            "state variable '{var}' of '{subsystem}' matches no {calc_kind} return. \
             Available returns: {available:?}"
        )
    }
}

/// The slot claim of a detected DISCRETE state-space subsystem.
///
/// The continuous lane carries this on [`OdeDetection`] and `mint_slot_store`
/// reads it to mint one `Continuous` slot per state variable, owned by the
/// registered solver. The discrete lane had no equivalent: its detectors
/// returned `(label, solver)` and nothing else, so `add_discrete` registered a
/// subsystem whose state vector was never minted. `prepare_slot_writeback`
/// then built a write-set of UNROUTED strict routes and the first tick
/// panicked in `WriteRoute::apply` — reachable on every checked-in discrete
/// fixture (`damped-oscillator`, `digital-filter`).
///
/// This is that missing claim. It is deliberately NOT an `OdeDetection`: a
/// discrete solver has no derivative expressions, and borrowing the ODE
/// carrier would put it in `ensure_derivatives_matched` and the continuous
/// solver-build loop, where it does not belong.
#[derive(Debug, Clone)]
pub struct DiscreteDetection {
    /// Subsystem name this detection was registered under — the same string
    /// passed to `Orchestrator::add_discrete`.
    pub label: String,
    /// Elements the state variables may be declared under, nearest first
    /// (the dynamics action, then its owning definition). Each state var is
    /// resolved to its declaring feature by searching these in order, so the
    /// minted slot carries a real declaration identity rather than a
    /// synthesised one.
    pub scope_ids: Vec<ElementId>,
    /// Names of the state variables, in solver order.
    pub state_vars: Vec<String>,
    /// Initial values, same order as `state_vars`.
    pub initial_values: Vec<f64>,
    /// [`SubsystemIndex`](crate::orchestrator::SubsystemIndex) captured at the
    /// `add_discrete` call site (RSC-4.2 L40) — never re-derived by name.
    /// `None` until registration; an unresolved index at mint time is a hard
    /// error, exactly as for [`OdeDetection::subsystem_index`].
    pub subsystem_index: Option<SubsystemIndex>,
    /// State-variable to calc-return matching failures recorded at detection.
    ///
    /// A *deferred* hard error, the same idiom as
    /// [`OdeDetection::derivative_match_errors`]: detection is a pure read that
    /// annotates the problem, and `build_workspace_orchestrator` fails the
    /// build. The continuous lane has had this since RSC ruling 1; the three
    /// discrete lanes never got it, so an unmatched state variable silently
    /// took a constant-`0` difference and a single return was silently
    /// broadcast across every state. See [`state_match`].
    pub match_errors: Vec<String>,
}

impl DiscreteDetection {
    /// Fail hard on any state-variable / calc-return mismatch recorded at
    /// detection. The discrete counterpart of
    /// [`OdeDetection::ensure_derivatives_matched`], called from
    /// `build_workspace_orchestrator` before any subsystem is registered.
    pub fn ensure_states_matched(&self) -> Result<(), CompileError> {
        if self.match_errors.is_empty() {
            return Ok(());
        }
        Err(CompileError::from_message(format!(
            "discrete dynamics '{}': could not resolve state variables to calc returns:\n  - {}",
            self.label,
            self.match_errors.join("\n  - ")
        )))
    }
}

/// Detected ODE configuration from spec-standard SSR types and/or `@ToolExecution` metadata.
///
/// The primary detection path is `calc def :> GetDerivative` (SSR).
/// `@ToolExecution { toolName }` on the enclosing part def provides solver selection.
#[derive(Debug, Clone)]
pub struct OdeDetection {
    /// Human-readable name derived from the owner element (e.g. `"ThermalProtectionModel"`).
    /// Used as the subsystem name in the orchestrator.
    pub name: Option<String>,
    /// The tool name, e.g. `"builtin:ode-rk4"`, `"builtin:ode-rk45"`, or `"ssr:GetDerivative"`.
    pub tool_name: String,
    /// Names of the state variables (direction out/inout).
    pub state_vars: Vec<String>,
    /// Initial values for each state variable (same order as `state_vars`).
    pub initial_values: Vec<f64>,
    /// Named ODE parameters (non-state numeric attributes).
    pub parameters: HashMap<String, f64>,
    /// Derivative expressions for each state variable (from `calc def :> GetDerivative`).
    pub derivative_exprs: Vec<String>,
    /// Signal expressions keyed by attribute name (from `calc def :> GetOutput`).
    pub signal_exprs: HashMap<String, String>,
    /// ElementId of the owner — the part/usage that holds the
    /// `calc def :> GetDerivative` definitions. Lets callers walk the
    /// graph to compute the canonical tree path for this ODE's state
    /// variables (container → instance → sub-part chain → var), which
    /// is required for runtime snapshot keys to match the frontend
    /// tree's `ownerPath`.
    pub owner_id: Option<ElementId>,
    /// [`SubsystemIndex`](crate::orchestrator::SubsystemIndex) of the
    /// registered ODE subsystem (RSC-4.2 L40). `None` at detection time;
    /// set by `&mut` mutation in the registration loop once this detection's
    /// solver is added to the orchestrator (never re-derived by name).
    /// Still `None` for a detection skipped at registration (e.g. it failed
    /// to build, or it's a per-instance template not itself registered) —
    /// `mint_slot_store` treats an unresolved index for a writer-claimed ODE
    /// as a `CompileError` (ruling 4 fail-hard), not a soft fallback.
    pub subsystem_index: Option<SubsystemIndex>,
    /// Per-state-variable derivative-matching failures recorded during
    /// detection (loose-SSR path). One message per state var that could not
    /// be mapped to exactly one `GetDerivative` return.
    ///
    /// Empty on a clean detection. This is a *deferred* hard error, following
    /// the same idiom as [`subsystem_index`](Self::subsystem_index): detection
    /// is a pure read that annotates the problem; the build entry points
    /// ([`ModelCompiler::prepare_single_ode`] and
    /// [`ModelCompiler::build_workspace_orchestrator`]) call
    /// [`ensure_derivatives_matched`](Self::ensure_derivatives_matched) and
    /// fail hard (ruling 1 fail-hard) — never a soft fallback to a constant-0
    /// derivative or a substring-collision first-match.
    pub derivative_match_errors: Vec<String>,
}

impl OdeDetection {
    /// Fail hard if derivative→state-var matching recorded any ambiguity or
    /// non-match during detection. Called by the orchestrator build paths so
    /// a malformed ODE aborts compilation consistently on every entry path.
    pub fn ensure_derivatives_matched(&self) -> Result<(), CompileError> {
        if self.derivative_match_errors.is_empty() {
            return Ok(());
        }
        let owner = self.name.as_deref().unwrap_or("<anonymous>");
        Err(CompileError::from_message(format!(
            "ODE '{owner}': could not resolve derivatives to state variables:\n  - {}",
            self.derivative_match_errors.join("\n  - ")
        )))
    }

    /// Whether the detected solver is the adaptive RK45 variant.
    pub fn is_rk45(&self) -> bool {
        self.tool_name.contains("rk45")
    }

    /// Whether the detected solver is the implicit BDF variant (R7.3).
    ///
    /// Matches `"builtin:ode-bdf"`, the alias `"bdf"`, or anything that
    /// contains `"bdf"` as a substring.
    pub fn is_bdf(&self) -> bool {
        let t = self.tool_name.to_ascii_lowercase();
        t == "bdf" || t.contains("bdf")
    }

    /// Whether the model *explicitly* requested the fixed-step explicit RK4
    /// solver (`@ToolExecution { toolName = "builtin:ode-rk4" }` or the `"rk4"`
    /// alias). This is a deliberate user assertion ("this problem is non-stiff,
    /// give me fixed-step explicit"), distinct from an UN-annotated ODE — which
    /// (WS-B2) defaults to the robust adaptive RK45, not RK4. Note `"rk45"`
    /// contains `"rk4"`, so this matches the exact RK4 spellings only.
    pub fn is_rk4(&self) -> bool {
        let t = self.tool_name.to_ascii_lowercase();
        t == "builtin:ode-rk4" || t == "rk4"
    }
}

/// Detect solver selections from `@ToolExecution { toolName }` metadata.
///
/// Scans the graph for `MetadataUsage` elements typed as `ToolExecution` whose
/// `toolName` starts with `"builtin:ode-"`. Returns a map from enclosing
/// definition name → tool name string (e.g., `"ProtectionCorePhysicsModel" → "builtin:ode-rk45"`).
///
/// This function only provides solver selection. Physics behavior (derivatives,
/// signals) comes from spec-standard SSR types (`GetDerivative`, `GetOutput`).
pub fn detect_solver_selections_from_metadata(graph: &ModelGraph) -> HashMap<String, String> {
    use sysml_core::ElementId;

    fn extract_string_child(
        graph: &ModelGraph,
        parent_id: &ElementId,
        child_name: &str,
    ) -> Option<String> {
        for child in graph.children_of(parent_id) {
            if child.name.as_deref() == Some(child_name) {
                return child
                    .get_prop("value")
                    .or_else(|| child.get_prop("default"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned())
                    .or_else(|| sysml_core::expression_pretty::pretty_print_owner(child, graph));
            }
        }
        None
    }

    let mut selections: HashMap<String, String> = HashMap::new();

    for element in graph.elements.values() {
        if element.kind != ElementKind::MetadataUsage {
            continue;
        }
        let is_tool_exec = element.name.as_deref() == Some("ToolExecution")
            || element
                .get_prop("unresolvedTypeName")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s == "ToolExecution" || s.ends_with("::ToolExecution"));
        if !is_tool_exec {
            continue;
        }

        // Extract toolName from children
        let Some(tn) = extract_string_child(graph, &element.id, "toolName") else {
            continue;
        };
        if !tn.starts_with("builtin:ode-") {
            continue;
        }

        // Walk up from @ToolExecution to find the enclosing definition.
        let Some(mut walk_id) = element.owner.clone() else {
            continue;
        };
        for _ in 0..20 {
            let Some(walk_elem) = graph.get_element(&walk_id) else {
                break;
            };
            if walk_elem.kind.is_definition() {
                if let Some(name) = &walk_elem.name {
                    selections.insert(name.clone(), tn.clone());
                }
                break;
            }
            match &walk_elem.owner {
                Some(parent) => walk_id = parent.clone(),
                None => break,
            }
        }
    }

    selections
}

impl ModelCompiler {
    /// Detect ODE configuration from the model (unified SSR + metadata path).
    ///
    /// Returns the first detected ODE group (backward-compatible).
    /// Prefers SSR-detected ODEs; merges solver selection from `@ToolExecution`.
    pub fn detect_ode(&self) -> Option<OdeDetection> {
        self.detect_all_odes_unified().into_iter().next()
    }

    /// Detect ALL ODE configurations from the model (unified SSR + metadata path).
    ///
    /// Returns one `OdeDetection` per distinct owner element. SSR detection
    /// provides derivative expressions; `@ToolExecution` provides solver selection.
    pub fn detect_all_odes(&self) -> Vec<OdeDetection> {
        self.detect_all_odes_unified()
    }

    // -- SampledFunction extraction ------------------------------------------

    /// Detect ODE systems defined via the SSR (State Space Representation) pattern.
    ///
    /// Finds `CalculationDefinition` elements that specialize `GetDerivative`
    /// (from `StateSpaceRepresentation.sysml`) and extracts their result expression
    /// as ODE derivative functions. This is the spec-standard alternative to
    /// `@ToolExecution` metadata.
    ///
    /// Returns ODE detections that can be merged with metadata-detected ODEs.
    pub fn detect_ode_from_ssr(&self) -> Vec<OdeDetection> {
        /// A named derivative: calc def name, return variable name, + expression text.
        struct NamedDerivative {
            calc_name: String,
            /// The return variable name (e.g., "dydt" from `return dydt = expr`).
            /// Used for matching derivatives to state variables.
            return_name: String,
            expr: String,
        }

        /// A named output: return variable name + expression text (from GetOutput).
        struct NamedOutput {
            return_name: String,
            expr: String,
        }

        // Phase 1: Collect all GetDerivative calc defs grouped by owner.
        let mut owner_calcs: HashMap<ElementId, Vec<NamedDerivative>> = HashMap::new();

        for element in self
            .graph
            .elements_by_kind(&ElementKind::CalculationDefinition)
        {
            if !self.specializes_name(&element.id, "GetDerivative") {
                continue;
            }

            let expr_text = extract_calc_result_expr(&self.graph, element);

            if let (Some(owner_id), Some(expr)) = (&element.owner, expr_text) {
                let calc_name = element.name.clone().unwrap_or_default();
                // Extract return variable name from children (e.g., "dydt" from `return dydt = expr`)
                let return_name = calc_return_name(&self.graph, element);
                owner_calcs
                    .entry(owner_id.clone())
                    .or_default()
                    .push(NamedDerivative {
                        calc_name,
                        return_name,
                        expr,
                    });
            }
        }

        // Phase 1b: Collect all GetOutput calc defs grouped by owner.
        // These produce algebraic signal expressions (e.g., i_drive from BH inverse).
        let mut owner_outputs: HashMap<ElementId, Vec<NamedOutput>> = HashMap::new();

        for element in self
            .graph
            .elements_by_kind(&ElementKind::CalculationDefinition)
        {
            if !self.specializes_name(&element.id, "GetOutput") {
                continue;
            }

            let expr_text = extract_calc_result_expr(&self.graph, element);

            if let (Some(owner_id), Some(expr)) = (&element.owner, expr_text) {
                let return_name = calc_return_name(&self.graph, element);
                owner_outputs
                    .entry(owner_id.clone())
                    .or_default()
                    .push(NamedOutput { return_name, expr });
            }
        }

        // Phase 2: For each owner with GetDerivative calcs, build an OdeDetection.
        let mut results = Vec::new();

        // `owner_calcs` is a `HashMap`, so iterating it directly makes the
        // order of `results` — and therefore, downstream in
        // `detect_all_odes_unified`, the registration order that determines
        // each ODE's `subsystem_index` — build-to-build nondeterministic
        // whenever a model has two or more ODE-owning parts. Sort owners by
        // source declaration order first (same idiom as the state-var sort
        // below).
        let mut owners: Vec<&Element> = owner_calcs
            .keys()
            .filter_map(|id| self.graph.get_element(id))
            .collect();
        sort_elements_by_source_order(&mut owners);

        for owner in owners {
            let owner_id = &owner.id;
            let named_derivs = &owner_calcs[owner_id];
            let owner_name = owner.name.clone();

            // Extract state variables (out/inout) and parameters from owner's children.
            let mut state_vars = Vec::new();
            let mut initial_values = Vec::new();
            let mut parameters = HashMap::new();

            // `children_of` is backed by `FxHashSet` (unordered) — with a
            // single state var this never mattered, but with two or more
            // (e.g. a fault-model part declaring both a fault-integral and a
            // flux state) it made `state_vars`'s element order — and every
            // downstream positional index into the ODE's state vector —
            // build-to-build nondeterministic. Sort to source declaration
            // order first, matching the idiom already used for the same
            // class of bug in quantity_health.rs/constraints.rs.
            //
            // Scope note (steward-reviewed): this fixes the loose-SSR path
            // (`detect_ode_from_ssr`) only. The identical unsorted-`children_of`
            // pattern also exists in `detect_discrete_from_ssr` and in the
            // `collect_attrs` closure used by the action-embedded ODE path
            // (`detect_composite_continuous_ssr` and friends) — same defect,
            // different call sites, intentionally not touched here. Tracked
            // as a separate follow-up; do not re-diagnose.
            let mut children: Vec<&Element> = self.graph.children_of(owner_id).collect();
            sort_elements_by_source_order(&mut children);

            for child in children {
                if child.kind != ElementKind::AttributeUsage {
                    continue;
                }
                let child_name = child.name.clone().unwrap_or_default();
                match classify_ode_attr(child, &self.graph) {
                    OdeAttrRole::StateVar { initial } => {
                        state_vars.push(child_name);
                        initial_values.push(initial);
                    }
                    OdeAttrRole::Parameter { value } => {
                        parameters.insert(child_name, value);
                    }
                    // Derived `= expr` bindings (e.g. `out attribute r = H_dc/H_ref`)
                    // are algebraic outputs recomputed via computed expressions,
                    // not integration states. This site does not track `in` inputs.
                    OdeAttrRole::DerivedBinding | OdeAttrRole::Input | OdeAttrRole::Ignore => {}
                }
            }

            if state_vars.is_empty() {
                continue;
            }

            // Phase 3: Match derivatives to state variables by name.
            //
            // A `GetDerivative` calc must map to exactly one state var. The
            // corpus uses two documented conventions, in precedence order:
            //   (A) exact — the return name is `d<state>dt` (e.g. `dstrokedt`
            //       for state `stroke`) or the calc name is `<State>Derivative`
            //       (e.g. `CircuitBayDerivative` for `T_circuitBay`, matched on
            //       the `t_`/`x_`-stripped stem);
            //   (B) substring — a legacy looser match (return/calc *contains*
            //       the stem) kept only for names that follow neither exact
            //       shape (e.g. `EnclosureWallDerivative` for `T_enclosure`).
            //
            // Exact wins first; substring is consulted only when no exact
            // match exists, and is required to be unambiguous. A state var that
            // matches zero derivatives, or that matches two or more under the
            // same tier, is a *hard error* recorded in `match_errors` and
            // enforced at build time — never a silent constant-`0` derivative
            // (the old `.unwrap_or("0")`) nor a substring-collision first-match
            // (the old `.find()`), both of which produced wrong physics
            // silently. This is deterministic regardless of iteration order.
            let mut match_errors: Vec<String> = Vec::new();
            let derivative_exprs: Vec<String> = if named_derivs.len() == 1
                && state_vars.len() == 1
            {
                // One derivative, one state: the only shape where a single
                // result IS the whole answer.
                vec![named_derivs[0].expr.clone()]
            } else if named_derivs.len() == 1 {
                // This arm used to be reached by the branch above, whose
                // comment ASSERTED "by construction, the single-state case"
                // without enforcing it — so a multi-state ODE with one
                // derivative would have broadcast silently. Now it is the
                // under-determined error.
                match_errors.push(state_match::under_determined(
                    owner_name.as_deref().unwrap_or("<unnamed>"),
                    "GetDerivative",
                    &state_vars,
                ));
                vec![named_derivs[0].expr.clone(); state_vars.len()]
            } else {
                // Multi-state: match by name, tier A then tier B.
                state_vars
                    .iter()
                    .map(|var_name| {
                        let var_lower = var_name.to_lowercase();
                        let var_stem = var_lower
                            .strip_prefix("t_")
                            .or_else(|| var_lower.strip_prefix("x_"))
                            .unwrap_or(&var_lower);

                        let is_exact = |nd: &NamedDerivative| {
                            let ret = nd.return_name.to_lowercase();
                            let calc = nd.calc_name.to_lowercase();
                            ret == format!("d{var_lower}dt")
                                || ret == format!("d{var_stem}dt")
                                || calc == format!("{var_lower}derivative")
                                || calc == format!("{var_stem}derivative")
                        };
                        let is_substr = |nd: &NamedDerivative| {
                            let ret = nd.return_name.to_lowercase();
                            let calc = nd.calc_name.to_lowercase();
                            // Guard single-char stems (e.g. "b") against the
                            // trivial "dt"/"derivative" substrings.
                            let by_return = ret.contains(var_stem)
                                && (var_stem.len() > 1
                                    || ret.starts_with(&format!("d{var_stem}")));
                            let by_calc = var_stem.len() > 1 && calc.contains(var_stem);
                            by_return || by_calc
                        };

                        let exact: Vec<&NamedDerivative> =
                            named_derivs.iter().filter(|nd| is_exact(nd)).collect();
                        let chosen: Vec<&NamedDerivative> = if exact.is_empty() {
                            named_derivs.iter().filter(|nd| is_substr(nd)).collect()
                        } else {
                            exact
                        };

                        match chosen.as_slice() {
                            [nd] => nd.expr.clone(),
                            [] => {
                                let candidates: Vec<String> = named_derivs
                                    .iter()
                                    .map(|nd| {
                                        format!(
                                            "{} (return {})",
                                            nd.calc_name, nd.return_name
                                        )
                                    })
                                    .collect();
                                match_errors.push(format!(
                                    "state variable '{var_name}' matches no GetDerivative; \
                                     expected a return named 'd{var_name}dt' or a calc named \
                                     '{var_name}Derivative'. Available derivatives: [{}]",
                                    candidates.join(", ")
                                ));
                                // Placeholder; never used — build fails first.
                                "0".to_string()
                            }
                            multiple => {
                                let collisions: Vec<String> = multiple
                                    .iter()
                                    .map(|nd| {
                                        format!(
                                            "{} (return {})",
                                            nd.calc_name, nd.return_name
                                        )
                                    })
                                    .collect();
                                match_errors.push(format!(
                                    "state variable '{var_name}' ambiguously matches \
                                     {} derivatives: [{}]. Disambiguate with the exact \
                                     'd{var_name}dt' return name or '{var_name}Derivative' \
                                     calc name.",
                                    collisions.len(),
                                    collisions.join(", ")
                                ));
                                "0".to_string()
                            }
                        }
                    })
                    .collect()
            };

            // Phase 4: Collect GetOutput expressions as signal_exprs.
            // Each GetOutput return variable name maps to its expression.
            let mut signal_exprs = HashMap::new();
            if let Some(outputs) = owner_outputs.get(owner_id) {
                for output in outputs {
                    if !output.return_name.is_empty() {
                        signal_exprs.insert(output.return_name.clone(), output.expr.clone());
                    }
                }
            }

            results.push(OdeDetection {
                name: owner_name,
                tool_name: "ssr:GetDerivative".to_string(),
                state_vars,
                initial_values,
                parameters,
                derivative_exprs,
                signal_exprs,
                owner_id: Some(owner_id.clone()),
                subsystem_index: None,
                derivative_match_errors: match_errors,
            });
        }

        results
    }

    /// Detect `calc def :> GetOutput` expressions from the model graph.
    ///
    /// Returns a map from owner element name to (return_var_name → expression)
    /// pairs. These can be merged into ODE detections as signal expressions,
    /// regardless of whether the ODE was detected via metadata or SSR.
    pub fn detect_output_calcs(&self) -> HashMap<String, HashMap<String, String>> {
        let mut result: HashMap<String, HashMap<String, String>> = HashMap::new();

        for element in self
            .graph
            .elements_by_kind(&ElementKind::CalculationDefinition)
        {
            if !self.specializes_name(&element.id, "GetOutput") {
                continue;
            }

            let expr_text = extract_calc_result_expr(&self.graph, element);

            if let (Some(owner_id), Some(expr)) = (&element.owner, expr_text) {
                let return_name = calc_return_name(&self.graph, element);

                if return_name.is_empty() {
                    continue;
                }

                let owner_name = self
                    .graph
                    .get_element(owner_id)
                    .and_then(|o| o.name.clone())
                    .unwrap_or_default();

                if !owner_name.is_empty() {
                    result
                        .entry(owner_name)
                        .or_default()
                        .insert(return_name, expr);
                }
            }
        }

        result
    }

    /// Detect `calc def :> GetDifference` from the model graph (spec-standard
    /// `DiscreteStateSpaceDynamics` pattern).
    ///
    /// For each owner (part def) that contains GetDifference calcs, builds a
    /// `DiscreteStateSolver` with the difference expressions. The update function
    /// evaluates `x_next = x + getDifference(input, stateSpace)`.
    ///
    /// Returns `(claim, solver)` pairs ready for `orchestrator.add_discrete()`;
    /// the [`DiscreteDetection`] carries what `mint_slot_store` needs to mint
    /// the solver's state vector as slots.
    pub fn detect_discrete_from_ssr(
        &self,
    ) -> Vec<(DiscreteDetection, crate::ode::DiscreteStateSolver)> {
        struct NamedDifference {
            _calc_name: String,
            return_name: String,
            expr: String,
        }

        let mut owner_calcs: HashMap<ElementId, Vec<NamedDifference>> = HashMap::new();

        for element in self
            .graph
            .elements_by_kind(&ElementKind::CalculationDefinition)
        {
            if !self.specializes_name(&element.id, "GetDifference") {
                continue;
            }

            let expr_text = extract_calc_result_expr(&self.graph, element);

            if let (Some(owner_id), Some(expr)) = (&element.owner, expr_text) {
                let calc_name = element.name.clone().unwrap_or_default();
                let return_name = calc_return_name(&self.graph, element);
                owner_calcs
                    .entry(owner_id.clone())
                    .or_default()
                    .push(NamedDifference {
                        _calc_name: calc_name,
                        return_name,
                        expr,
                    });
            }
        }

        let mut results = Vec::new();

        for (owner_id, diffs) in &owner_calcs {
            let owner = match self.graph.get_element(owner_id) {
                Some(o) => o,
                None => continue,
            };
            let owner_name = owner.name.clone().unwrap_or_else(|| "discrete".to_string());

            let mut state_vars = Vec::new();
            let mut initial_values = Vec::new();
            let mut input_names = Vec::new();
            let mut parameters = HashMap::new();

            for child in self.graph.children_of(owner_id) {
                if child.kind != ElementKind::AttributeUsage {
                    continue;
                }
                let child_name = child.name.clone().unwrap_or_default();
                match classify_ode_attr(child, &self.graph) {
                    OdeAttrRole::StateVar { initial } => {
                        state_vars.push(child_name);
                        initial_values.push(initial);
                    }
                    OdeAttrRole::Input => {
                        input_names.push(child_name);
                    }
                    OdeAttrRole::Parameter { value } => {
                        parameters.insert(child_name, value);
                    }
                    // Derived `= expr` bindings are algebraic outputs, not states.
                    OdeAttrRole::DerivedBinding | OdeAttrRole::Ignore => {}
                }
            }

            if state_vars.is_empty() {
                continue;
            }

            // Match difference expressions to state variables (same logic as derivatives)
            // See `state_match`: errors are recorded here and enforced at
            // build time, never silently absorbed.
            let mut match_errors: Vec<String> = Vec::new();
            let diff_exprs: Vec<String> = if diffs.len() == 1 && state_vars.len() == 1 {
                vec![diffs[0].expr.clone()]
            } else if diffs.len() == 1 {
                match_errors.push(state_match::under_determined(
                    &owner_name,
                    "GetDifference",
                    &state_vars,
                ));
                vec![diffs[0].expr.clone(); state_vars.len()]
            } else {
                state_vars
                    .iter()
                    .map(|var_name| {
                        let var_lower = var_name.to_lowercase();
                        match diffs.iter().find(|d| {
                            let ret = d.return_name.to_lowercase();
                            ret.contains(&var_lower)
                        }) {
                            Some(d) => d.expr.clone(),
                            None => {
                                let available: Vec<&str> =
                                    diffs.iter().map(|d| d.return_name.as_str()).collect();
                                match_errors.push(state_match::unmatched(
                                    &owner_name,
                                    "GetDifference",
                                    var_name,
                                    &available,
                                ));
                                "0".to_string()
                            }
                        }
                    })
                    .collect()
            };

            // Compile difference expressions to ExprIR
            let compiled_diffs: Vec<ExprIR> = diff_exprs
                .iter()
                .filter_map(|e| ode_builder::parse_derivative(e).ok())
                .collect();

            if compiled_diffs.len() != state_vars.len() {
                continue; // Failed to compile some expressions
            }

            // Build the discrete update function: x_next = x + diff(x, u, ctx)
            let sv_names = state_vars.clone();
            let in_names = input_names.clone();
            let params = parameters.clone();
            let evaluator = std::sync::Arc::new(crate::expressions::ExpressionEvaluator::new());

            let update_fn: crate::ode::DiscreteUpdateFn =
                std::sync::Arc::new(move |_k, x, u, ctx| {
                    let mut eval_ctx = ctx.scratch_snapshot();
                    for (name, &val) in &params {
                        eval_ctx.set(name.clone(), Value::Float(val));
                    }
                    for (i, name) in sv_names.iter().enumerate() {
                        eval_ctx.set(name.clone(), Value::Float(x[i]));
                    }
                    for (i, name) in in_names.iter().enumerate() {
                        if i < u.len() {
                            eval_ctx.set(name.clone(), Value::Float(u[i]));
                        }
                    }
                    compiled_diffs
                        .iter()
                        .enumerate()
                        .map(|(i, expr)| {
                            let diff = match evaluator.eval(expr, &eval_ctx) {
                                Ok(Value::Float(f)) => f,
                                Ok(Value::Int(n)) => n as f64,
                                _ => 0.0,
                            };
                            x[i] + diff // x_next = x + getDifference()
                        })
                        .collect()
                });

            let claim = DiscreteDetection {
                label: owner_name.clone(),
                // Loose `calc def :> GetDifference` members are collected per
                // OWNER, so the owner is the only declaration scope there is.
                scope_ids: vec![owner_id.clone()],
                state_vars: state_vars.clone(),
                initial_values: initial_values.clone(),
                subsystem_index: None,
                match_errors,
            };
            let solver = crate::ode::DiscreteStateSolver::new(
                &owner_name,
                state_vars,
                initial_values,
                input_names,
                update_fn,
            );

            results.push((claim, solver));
        }

        results
    }

    /// Detect `action def :> ContinuousStateSpaceDynamics` composite patterns.
    ///
    /// Unlike `detect_ode_from_ssr()` which finds loose `calc def :> GetDerivative`
    /// anywhere in the model, this function finds formal dynamics action defs that
    /// specialize the spec's `ContinuousStateSpaceDynamics` type and extracts their
    /// complete member structure:
    ///
    /// - `getDerivative : GetDerivative` → ODE RHS
    /// - `getOutput : GetOutput` → algebraic outputs
    /// - `stateSpace : StateSpace` → state variables (or `out` attributes)
    /// - `input : Input` → external inputs
    /// - `zeroCrossingEvents : ZeroCrossingEventDef` → event detectors
    ///
    /// The dynamics action may be nested inside a part def, in which case state
    /// variables and parameters from the enclosing part are also collected.
    pub fn detect_composite_continuous_ssr(&self) -> Vec<OdeDetection> {
        let mut results = Vec::new();

        for element in self.graph.elements.values() {
            if !matches!(
                element.kind,
                ElementKind::ActionDefinition | ElementKind::ActionUsage
            ) {
                continue;
            }
            if !self.specializes_name(&element.id, "ContinuousStateSpaceDynamics") {
                continue;
            }

            let action_name = element
                .name
                .clone()
                .unwrap_or_else(|| "dynamics".to_string());

            // Collect GetDerivative calc defs inside this action
            let mut derivative_calcs: Vec<(String, String)> = Vec::new(); // (return_name, expr)
                                                                          // Collect GetOutput calc defs inside this action
            let mut output_calcs: Vec<(String, String)> = Vec::new(); // (return_name, expr)

            for child in self.graph.children_of(&element.id) {
                if child.kind != ElementKind::CalculationDefinition
                    && child.kind != ElementKind::CalculationUsage
                {
                    continue;
                }

                let is_deriv = self.specializes_name(&child.id, "GetDerivative");
                let is_output = self.specializes_name(&child.id, "GetOutput");
                if !is_deriv && !is_output {
                    continue;
                }

                let expr = extract_calc_result_expr(&self.graph, child);

                let return_name = calc_return_name(&self.graph, child);

                if let Some(expr) = expr {
                    if is_deriv {
                        derivative_calcs.push((return_name, expr));
                    } else {
                        output_calcs.push((return_name, expr));
                    }
                }
            }

            if derivative_calcs.is_empty() {
                continue; // Not a valid dynamics action without derivatives
            }

            // Collect state variables and parameters.
            // Look in the action itself and its enclosing owner (part def).
            let mut state_vars = Vec::new();
            let mut initial_values = Vec::new();
            let mut parameters = HashMap::new();

            // Helper to collect attributes from an element
            let collect_attrs =
                |id: &ElementId,
                 state_vars: &mut Vec<String>,
                 initial_values: &mut Vec<f64>,
                 parameters: &mut HashMap<String, f64>| {
                    for child in self.graph.children_of(id) {
                        if child.kind != ElementKind::AttributeUsage {
                            continue;
                        }
                        let name = match &child.name {
                            Some(n) => n.clone(),
                            None => continue,
                        };
                        match classify_ode_attr(child, &self.graph) {
                            OdeAttrRole::StateVar { initial } => {
                                state_vars.push(name);
                                initial_values.push(initial);
                            }
                            OdeAttrRole::Parameter { value } => {
                                parameters.insert(name, value);
                            }
                            // Derived `= expr` bindings are algebraic outputs;
                            // this site does not track `in` inputs.
                            OdeAttrRole::DerivedBinding
                            | OdeAttrRole::Input
                            | OdeAttrRole::Ignore => {}
                        }
                    }
                };

            // First collect from the action itself
            collect_attrs(
                &element.id,
                &mut state_vars,
                &mut initial_values,
                &mut parameters,
            );

            // Also collect from enclosing owner (part def) if present
            if let Some(ref owner_id) = element.owner {
                collect_attrs(
                    owner_id,
                    &mut state_vars,
                    &mut initial_values,
                    &mut parameters,
                );
            }

            // An `out attribute` that a `GetOutput` calc RETURNS is an
            // algebraic output, not an integrated state. The normative library
            // separates the two roles on `StateSpaceDynamics` itself —
            // `attribute stateSpace: StateSpace` versus
            // `out attribute output: Output = getOutput(input, stateSpace)` —
            // but `classify_ode_attr` sees only a bare `out attribute` with a
            // literal default and calls it a state.
            //
            // Left in, such a name was handed a derivative it has no business
            // having: the `RampCross` test fixture (`out attribute sig` with
            // `calc def SigOut :> GetOutput { return sig = 3.0 * x; }`) had
            // `sig` INTEGRATED with `x`'s derivative before its signal sync
            // overwrote it. Invisible while a single derivative was silently
            // broadcast across every state; the under-determined check
            // surfaced it.
            {
                let output_names: HashSet<&str> =
                    output_calcs.iter().map(|(r, _)| r.as_str()).collect();
                let mut keep = state_vars.iter().map(|v| !output_names.contains(v.as_str()));
                initial_values.retain(|_| keep.next().unwrap_or(true));
                state_vars.retain(|v| !output_names.contains(v.as_str()));
            }

            if state_vars.is_empty() {
                continue;
            }

            // Match derivatives to state variables by return name. Same
            // fail-hard contract as the loose-SSR path
            // (`detect_ode_from_ssr`): exact `d<state>dt` return wins, else a
            // unique substring match; zero or ambiguous matches are recorded
            // in `match_errors` and enforced at build. The composite path
            // captures only return names (no calc-def name), so tier A here is
            // the exact `d<state>dt` return shape only.
            let mut match_errors: Vec<String> = Vec::new();
            let derivative_exprs: Vec<String> = if derivative_calcs.len() == 1
                && state_vars.len() == 1
            {
                vec![derivative_calcs[0].1.clone()]
            } else if derivative_calcs.len() == 1 {
                match_errors.push(state_match::under_determined(
                    &action_name,
                    "GetDerivative",
                    &state_vars,
                ));
                vec![derivative_calcs[0].1.clone(); state_vars.len()]
            } else {
                state_vars
                    .iter()
                    .map(|var_name| {
                        let var_lower = var_name.to_lowercase();
                        let var_stem = var_lower
                            .strip_prefix("t_")
                            .or_else(|| var_lower.strip_prefix("x_"))
                            .unwrap_or(&var_lower);

                        let exact: Vec<&(String, String)> = derivative_calcs
                            .iter()
                            .filter(|(ret, _)| {
                                let r = ret.to_lowercase();
                                r == format!("d{var_lower}dt") || r == format!("d{var_stem}dt")
                            })
                            .collect();
                        let chosen: Vec<&(String, String)> = if exact.is_empty() {
                            derivative_calcs
                                .iter()
                                .filter(|(ret, _)| {
                                    let r = ret.to_lowercase();
                                    r.contains(var_stem)
                                        && (var_stem.len() > 1
                                            || r.starts_with(&format!("d{var_stem}")))
                                })
                                .collect()
                        } else {
                            exact
                        };

                        match chosen.as_slice() {
                            [(_, expr)] => expr.clone(),
                            [] => {
                                let available: Vec<&str> =
                                    derivative_calcs.iter().map(|(r, _)| r.as_str()).collect();
                                match_errors.push(format!(
                                    "state variable '{var_name}' matches no GetDerivative return; \
                                     expected a return named 'd{var_name}dt'. Available returns: {:?}",
                                    available
                                ));
                                "0".to_string()
                            }
                            multiple => {
                                let collisions: Vec<&str> =
                                    multiple.iter().map(|(r, _)| r.as_str()).collect();
                                match_errors.push(format!(
                                    "state variable '{var_name}' ambiguously matches returns {:?}; \
                                     disambiguate with the exact 'd{var_name}dt' return name.",
                                    collisions
                                ));
                                "0".to_string()
                            }
                        }
                    })
                    .collect()
            };

            // Collect GetOutput expressions as signal_exprs
            let mut signal_exprs = HashMap::new();
            for (return_name, expr) in &output_calcs {
                if !return_name.is_empty() {
                    signal_exprs.insert(return_name.clone(), expr.clone());
                }
            }

            // Use the enclosing part def name if the action is a usage inside a part
            let detection_name = element
                .owner
                .as_ref()
                .and_then(|oid| self.graph.get_element(oid))
                .and_then(|o| {
                    if o.kind.is_definition() {
                        o.name.clone()
                    } else {
                        None
                    }
                })
                .unwrap_or(action_name);

            results.push(OdeDetection {
                name: Some(detection_name),
                tool_name: "ssr:ContinuousStateSpaceDynamics".to_string(),
                state_vars,
                initial_values,
                parameters,
                derivative_exprs,
                signal_exprs,
                owner_id: Some(element.id.clone()),
                subsystem_index: None,
                derivative_match_errors: match_errors,
            });
        }

        results
    }

    /// Detect `action def :> DiscreteStateSpaceDynamics` composite patterns.
    ///
    /// Finds formal discrete dynamics action defs with `getDifference : GetDifference`
    /// members and builds `DiscreteStateSolver` instances.
    pub fn detect_composite_discrete_ssr(
        &self,
    ) -> Vec<(DiscreteDetection, crate::ode::DiscreteStateSolver)> {
        let mut results = Vec::new();

        for element in self.graph.elements.values() {
            if !matches!(
                element.kind,
                ElementKind::ActionDefinition | ElementKind::ActionUsage
            ) {
                continue;
            }
            if !self.specializes_name(&element.id, "DiscreteStateSpaceDynamics") {
                continue;
            }

            let action_name = element
                .name
                .clone()
                .unwrap_or_else(|| "discrete".to_string());

            // Collect GetDifference calc defs inside this action
            let mut diff_calcs: Vec<(String, String)> = Vec::new(); // (return_name, expr)

            for child in self.graph.children_of(&element.id) {
                if child.kind != ElementKind::CalculationDefinition
                    && child.kind != ElementKind::CalculationUsage
                {
                    continue;
                }
                if !self.specializes_name(&child.id, "GetDifference") {
                    continue;
                }

                let expr = extract_calc_result_expr(&self.graph, child);

                let return_name = calc_return_name(&self.graph, child);

                if let Some(expr) = expr {
                    diff_calcs.push((return_name, expr));
                }
            }

            if diff_calcs.is_empty() {
                continue;
            }

            // Collect state variables, inputs, parameters from action + owner
            let mut state_vars = Vec::new();
            let mut initial_values = Vec::new();
            let mut input_names = Vec::new();
            let mut parameters = HashMap::new();

            let collect_from = |id: &ElementId,
                                sv: &mut Vec<String>,
                                iv: &mut Vec<f64>,
                                ins: &mut Vec<String>,
                                params: &mut HashMap<String, f64>| {
                for child in self.graph.children_of(id) {
                    if child.kind != ElementKind::AttributeUsage {
                        continue;
                    }
                    let name = match &child.name {
                        Some(n) => n.clone(),
                        None => continue,
                    };
                    match classify_ode_attr(child, &self.graph) {
                        OdeAttrRole::StateVar { initial } => {
                            sv.push(name);
                            iv.push(initial);
                        }
                        OdeAttrRole::Input => ins.push(name),
                        OdeAttrRole::Parameter { value } => {
                            params.insert(name, value);
                        }
                        // Derived `= expr` bindings are algebraic outputs, not states.
                        OdeAttrRole::DerivedBinding | OdeAttrRole::Ignore => {}
                    }
                }
            };

            collect_from(
                &element.id,
                &mut state_vars,
                &mut initial_values,
                &mut input_names,
                &mut parameters,
            );
            if let Some(ref owner_id) = element.owner {
                collect_from(
                    owner_id,
                    &mut state_vars,
                    &mut initial_values,
                    &mut input_names,
                    &mut parameters,
                );
            }

            if state_vars.is_empty() {
                continue;
            }

            // Match diff expressions to state vars
            // See `state_match`: errors are recorded here and enforced at
            // build time, never silently absorbed.
            let mut match_errors: Vec<String> = Vec::new();
            let disc_label = element
                .owner
                .as_ref()
                .and_then(|oid| self.graph.get_element(oid))
                .and_then(|o| o.name.clone())
                .unwrap_or_else(|| action_name.clone());
            let diff_exprs: Vec<String> = if diff_calcs.len() == 1 && state_vars.len() == 1 {
                vec![diff_calcs[0].1.clone()]
            } else if diff_calcs.len() == 1 {
                match_errors.push(state_match::under_determined(
                    &disc_label,
                    "GetDifference",
                    &state_vars,
                ));
                vec![diff_calcs[0].1.clone(); state_vars.len()]
            } else {
                state_vars
                    .iter()
                    .map(|var| {
                        let var_lower = var.to_lowercase();
                        match diff_calcs
                            .iter()
                            .find(|(ret, _)| ret.to_lowercase().contains(&var_lower))
                        {
                            Some((_, e)) => e.clone(),
                            None => {
                                let available: Vec<&str> =
                                    diff_calcs.iter().map(|(r, _)| r.as_str()).collect();
                                match_errors.push(state_match::unmatched(
                                    &disc_label,
                                    "GetDifference",
                                    var,
                                    &available,
                                ));
                                "0".to_string()
                            }
                        }
                    })
                    .collect()
            };

            // Compile to ExprIR
            let compiled: Vec<ExprIR> = diff_exprs
                .iter()
                .filter_map(|e| ode_builder::parse_derivative(e).ok())
                .collect();
            if compiled.len() != state_vars.len() {
                continue;
            }

            let sv = state_vars.clone();
            let ins = input_names.clone();
            let params_c = parameters.clone();
            let evaluator = std::sync::Arc::new(crate::expressions::ExpressionEvaluator::new());

            let update_fn: crate::ode::DiscreteUpdateFn =
                std::sync::Arc::new(move |_k, x, u, ctx| {
                    let mut eval_ctx = ctx.scratch_snapshot();
                    for (name, &val) in &params_c {
                        eval_ctx.set(name.clone(), Value::Float(val));
                    }
                    for (i, name) in sv.iter().enumerate() {
                        eval_ctx.set(name.clone(), Value::Float(x[i]));
                    }
                    for (i, name) in ins.iter().enumerate() {
                        if i < u.len() {
                            eval_ctx.set(name.clone(), Value::Float(u[i]));
                        }
                    }
                    compiled
                        .iter()
                        .enumerate()
                        .map(|(i, expr)| {
                            let diff = match evaluator.eval(expr, &eval_ctx) {
                                Ok(Value::Float(f)) => f,
                                Ok(Value::Int(n)) => n as f64,
                                _ => 0.0,
                            };
                            x[i] + diff
                        })
                        .collect()
                });

            let label = element
                .owner
                .as_ref()
                .and_then(|oid| self.graph.get_element(oid))
                .and_then(|o| {
                    if o.kind.is_definition() {
                        o.name.clone()
                    } else {
                        None
                    }
                })
                .unwrap_or(action_name);

            // Nearest scope first: a state var declared on the dynamics
            // action itself shadows a same-named one on the enclosing
            // definition, exactly as `collect_from` gathered them.
            let mut scope_ids = vec![element.id.clone()];
            if let Some(owner_id) = &element.owner {
                scope_ids.push(owner_id.clone());
            }
            let claim = DiscreteDetection {
                label: label.clone(),
                scope_ids,
                state_vars: state_vars.clone(),
                initial_values: initial_values.clone(),
                subsystem_index: None,
                match_errors,
            };
            let solver = crate::ode::DiscreteStateSolver::new(
                &label,
                state_vars,
                initial_values,
                input_names,
                update_fn,
            );

            results.push((claim, solver));
        }

        results
    }

    /// Detect `action def :> StateSpaceDynamics` (base form) with direct
    /// `getNextState : GetNextState` computation.
    ///
    /// The base `StateSpaceDynamics` is the simplest SSR form: the user provides
    /// a direct `getNextState` calculation that computes `x_next = f(x, u, dt)`.
    /// Unlike `ContinuousStateSpaceDynamics` (derivative + integration) or
    /// `DiscreteStateSpaceDynamics` (x + diff), this form computes the full
    /// next state directly.
    ///
    /// Skips action defs that already specialize the continuous or discrete subtypes,
    /// since those are handled by their own detectors.
    pub fn detect_composite_state_space_dynamics(
        &self,
    ) -> Vec<(DiscreteDetection, crate::ode::DiscreteStateSolver)> {
        let mut results = Vec::new();

        for element in self.graph.elements.values() {
            if !matches!(
                element.kind,
                ElementKind::ActionDefinition | ElementKind::ActionUsage
            ) {
                continue;
            }
            if !self.specializes_name(&element.id, "StateSpaceDynamics") {
                continue;
            }
            // Skip subtypes handled by dedicated detectors
            if self.specializes_name(&element.id, "ContinuousStateSpaceDynamics")
                || self.specializes_name(&element.id, "DiscreteStateSpaceDynamics")
            {
                continue;
            }

            let action_name = element
                .name
                .clone()
                .unwrap_or_else(|| "dynamics".to_string());

            // Collect GetNextState calc defs inside this action
            let mut next_state_calcs: Vec<(String, String)> = Vec::new(); // (return_name, expr)

            for child in self.graph.children_of(&element.id) {
                if child.kind != ElementKind::CalculationDefinition
                    && child.kind != ElementKind::CalculationUsage
                {
                    continue;
                }
                if !self.specializes_name(&child.id, "GetNextState") {
                    continue;
                }

                let expr = extract_calc_result_expr(&self.graph, child);

                let return_name = calc_return_name(&self.graph, child);

                if let Some(expr) = expr {
                    next_state_calcs.push((return_name, expr));
                }
            }

            if next_state_calcs.is_empty() {
                continue;
            }

            // Collect state variables, inputs, parameters
            let mut state_vars = Vec::new();
            let mut initial_values = Vec::new();
            let mut input_names = Vec::new();
            let mut parameters = HashMap::new();

            let collect_from = |id: &ElementId,
                                sv: &mut Vec<String>,
                                iv: &mut Vec<f64>,
                                ins: &mut Vec<String>,
                                params: &mut HashMap<String, f64>| {
                for child in self.graph.children_of(id) {
                    if child.kind != ElementKind::AttributeUsage {
                        continue;
                    }
                    let name = match &child.name {
                        Some(n) => n.clone(),
                        None => continue,
                    };
                    match classify_ode_attr(child, &self.graph) {
                        OdeAttrRole::StateVar { initial } => {
                            sv.push(name);
                            iv.push(initial);
                        }
                        OdeAttrRole::Input => ins.push(name),
                        OdeAttrRole::Parameter { value } => {
                            params.insert(name, value);
                        }
                        // Derived `= expr` bindings are algebraic outputs, not states.
                        OdeAttrRole::DerivedBinding | OdeAttrRole::Ignore => {}
                    }
                }
            };

            collect_from(
                &element.id,
                &mut state_vars,
                &mut initial_values,
                &mut input_names,
                &mut parameters,
            );
            if let Some(ref owner_id) = element.owner {
                collect_from(
                    owner_id,
                    &mut state_vars,
                    &mut initial_values,
                    &mut input_names,
                    &mut parameters,
                );
            }

            if state_vars.is_empty() {
                continue;
            }

            // Match next-state expressions to state variables. Errors are
            // recorded and enforced at build time (see `DiscreteDetection::
            // match_errors`) — never a silent broadcast or constant-0 state.
            let mut match_errors: Vec<String> = Vec::new();
            let ssd_label = element
                .owner
                .as_ref()
                .and_then(|oid| self.graph.get_element(oid))
                .and_then(|o| o.name.clone())
                .unwrap_or_else(|| action_name.clone());
            let next_exprs: Vec<String> = if next_state_calcs.len() == 1
                && state_vars.len() == 1
            {
                vec![next_state_calcs[0].1.clone()]
            } else if next_state_calcs.len() == 1 {
                match_errors.push(state_match::under_determined(
                    &ssd_label,
                    "GetNextState",
                    &state_vars,
                ));
                vec![next_state_calcs[0].1.clone(); state_vars.len()]
            } else {
                state_vars
                    .iter()
                    .map(|var| {
                        let var_lower = var.to_lowercase();
                        match next_state_calcs
                            .iter()
                            .find(|(ret, _)| ret.to_lowercase().contains(&var_lower))
                        {
                            Some((_, e)) => e.clone(),
                            None => {
                                let available: Vec<&str> = next_state_calcs
                                    .iter()
                                    .map(|(r, _)| r.as_str())
                                    .collect();
                                match_errors.push(state_match::unmatched(
                                    &ssd_label,
                                    "GetNextState",
                                    var,
                                    &available,
                                ));
                                "0".to_string()
                            }
                        }
                    })
                    .collect()
            };

            // Compile to ExprIR
            let compiled: Vec<ExprIR> = next_exprs
                .iter()
                .filter_map(|e| ode_builder::parse_derivative(e).ok())
                .collect();
            if compiled.len() != state_vars.len() {
                continue;
            }

            // Build update function: x_next = getNextState(x, u, dt)
            // Unlike DiscreteStateSpaceDynamics (x + diff), this directly computes the next state.
            let sv = state_vars.clone();
            let ins = input_names.clone();
            let params_c = parameters.clone();
            let evaluator = std::sync::Arc::new(crate::expressions::ExpressionEvaluator::new());

            let update_fn: crate::ode::DiscreteUpdateFn =
                std::sync::Arc::new(move |_k, x, u, ctx| {
                    let mut eval_ctx = ctx.scratch_snapshot();
                    for (name, &val) in &params_c {
                        eval_ctx.set(name.clone(), Value::Float(val));
                    }
                    for (i, name) in sv.iter().enumerate() {
                        eval_ctx.set(name.clone(), Value::Float(x[i]));
                    }
                    for (i, name) in ins.iter().enumerate() {
                        if i < u.len() {
                            eval_ctx.set(name.clone(), Value::Float(u[i]));
                        }
                    }
                    // Direct next-state computation (NOT x + diff)
                    compiled
                        .iter()
                        .map(|expr| match evaluator.eval(expr, &eval_ctx) {
                            Ok(Value::Float(f)) => f,
                            Ok(Value::Int(n)) => n as f64,
                            _ => 0.0,
                        })
                        .collect()
                });

            let label = element
                .owner
                .as_ref()
                .and_then(|oid| self.graph.get_element(oid))
                .and_then(|o| {
                    if o.kind.is_definition() {
                        o.name.clone()
                    } else {
                        None
                    }
                })
                .unwrap_or(action_name);

            // Nearest scope first: a state var declared on the dynamics
            // action itself shadows a same-named one on the enclosing
            // definition, exactly as `collect_from` gathered them.
            let mut scope_ids = vec![element.id.clone()];
            if let Some(owner_id) = &element.owner {
                scope_ids.push(owner_id.clone());
            }
            let claim = DiscreteDetection {
                label: label.clone(),
                scope_ids,
                state_vars: state_vars.clone(),
                initial_values: initial_values.clone(),
                subsystem_index: None,
                match_errors,
            };
            let solver = crate::ode::DiscreteStateSolver::new(
                &label,
                state_vars,
                initial_values,
                input_names,
                update_fn,
            );

            results.push((claim, solver));
        }

        results
    }

    /// Detect `calc integrate: Integrate` members inside `ContinuousStateSpaceDynamics`
    /// action defs and return solver hints.
    ///
    /// The spec says `Integrate` "should be given by a solver". When a model provides
    /// an Integrate calc, its name or metadata can hint at the solver to use. Returns
    /// a map from owner action name to solver tool name (e.g., "builtin:ode-rk45").
    pub fn detect_integrate_solver_hints(&self) -> HashMap<String, String> {
        let mut hints = HashMap::new();

        for element in self.graph.elements.values() {
            if !matches!(
                element.kind,
                ElementKind::ActionDefinition | ElementKind::ActionUsage
            ) {
                continue;
            }
            if !self.specializes_name(&element.id, "ContinuousStateSpaceDynamics") {
                continue;
            }

            let action_name = element
                .owner
                .as_ref()
                .and_then(|oid| self.graph.get_element(oid))
                .and_then(|o| {
                    if o.kind.is_definition() {
                        o.name.clone()
                    } else {
                        None
                    }
                })
                .or_else(|| element.name.clone())
                .unwrap_or_default();

            if action_name.is_empty() {
                continue;
            }

            // Walk children to find Integrate calc usage
            for child in self.graph.children_of(&element.id) {
                // Look for calc usages/defs that specialize Integrate, or
                // look inside getNextState for nested integrate calcs
                let check_integrate = |id: &ElementId| -> Option<String> {
                    for c in self.graph.children_of(id) {
                        if (c.kind == ElementKind::CalculationDefinition
                            || c.kind == ElementKind::CalculationUsage)
                            && self.specializes_name(&c.id, "Integrate")
                        {
                            // Check for solver hint in name or metadata
                            let name = c.name.as_deref().unwrap_or("integrate");
                            if name.to_lowercase().contains("rk45")
                                || name.to_lowercase().contains("adaptive")
                            {
                                return Some("builtin:ode-rk45".to_string());
                            }
                            // Check for @ToolExecution on the integrate calc
                            for meta_child in self.graph.children_of(&c.id) {
                                if meta_child.kind == ElementKind::MetadataUsage {
                                    if let Some(Value::String(tool)) =
                                        meta_child.get_prop("toolName")
                                    {
                                        return Some(tool.clone());
                                    }
                                }
                            }
                            // WS-B2: presence of an `Integrate` calc with no
                            // explicit solver hint takes the robust default
                            // (adaptive RK45), same as a bare SSR ODE. An
                            // explicit `builtin:ode-rk4` (caught above via the
                            // `@ToolExecution` toolName) still opts into RK4.
                            return Some("builtin:ode-rk45".to_string());
                        }
                    }
                    None
                };

                // Check direct children of the action
                if let Some(hint) = check_integrate(&element.id) {
                    hints.insert(action_name.clone(), hint);
                    break;
                }

                // Check inside getNextState calc (nested pattern from spec)
                if (child.kind == ElementKind::CalculationDefinition
                    || child.kind == ElementKind::CalculationUsage)
                    && self.specializes_name(&child.id, "GetNextState")
                {
                    if let Some(hint) = check_integrate(&child.id) {
                        hints.insert(action_name.clone(), hint);
                        break;
                    }
                }
            }
        }

        hints
    }

    /// Detect all ODE configurations — SSR is the primary path.
    ///
    /// Detection priority:
    /// 1. **Composite**: `action def :> ContinuousStateSpaceDynamics` (formal pattern)
    /// 2. **Individual**: `calc def :> GetDerivative` (loose calc defs in part defs)
    /// 3. **Solver selection**: `@ToolExecution { toolName }` on enclosing part/action
    /// 4. **Algebraic outputs**: `calc def :> GetOutput` merged into matching detections
    ///
    /// This is the single entry point for all ODE detection.
    pub fn detect_all_odes_unified(&self) -> Vec<OdeDetection> {
        // 1. Composite SSR: action def :> ContinuousStateSpaceDynamics
        let mut ode_detections = self.detect_composite_continuous_ssr();
        let composite_names: std::collections::HashSet<String> = ode_detections
            .iter()
            .filter_map(|o| o.name.clone())
            .collect();

        // 2. Individual SSR: loose calc def :> GetDerivative (skip duplicates)
        for ssr_ode in self.detect_ode_from_ssr() {
            if let Some(ref name) = ssr_ode.name {
                if composite_names.contains(name) {
                    continue; // Composite detection takes priority
                }
            }
            ode_detections.push(ssr_ode);
        }

        // 3. Merge solver selections from @ToolExecution metadata and Integrate hints.
        let solver_selections = detect_solver_selections_from_metadata(&self.graph);
        let integrate_hints = self.detect_integrate_solver_hints();
        for ode in &mut ode_detections {
            if let Some(ref name) = ode.name {
                // @ToolExecution takes precedence, then Integrate hint
                if let Some(tool_name) = solver_selections.get(name) {
                    ode.tool_name = tool_name.clone();
                } else if let Some(hint) = integrate_hints.get(name) {
                    ode.tool_name = hint.clone();
                }
            }
        }

        // 4. Merge GetOutput signal expressions into matching ODE detections.
        let output_calcs = self.detect_output_calcs();
        for ode in &mut ode_detections {
            if let Some(ref name) = ode.name {
                if let Some(outputs) = output_calcs.get(name) {
                    for (var_name, expr) in outputs {
                        ode.signal_exprs
                            .entry(var_name.clone())
                            .or_insert_with(|| expr.clone());
                    }
                }
            }
        }

        ode_detections
    }

    /// Declaring element for one of an ODE detection's variables: under the
    /// detection's owner first, falling back to the bare-binding walk.
    pub(crate) fn find_ode_feature_decl(
        &self,
        ode: &OdeDetection,
        name: &str,
        bare: &HashMap<String, (ElementId, Value)>,
    ) -> Option<ElementId> {
        ode.owner_id
            .as_ref()
            .and_then(|owner| self.find_feature_decl(owner, name))
            .or_else(|| bare.get(name).map(|(id, _)| id.clone()))
    }

}
