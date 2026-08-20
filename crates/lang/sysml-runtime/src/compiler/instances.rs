//! `InstanceTree`, instance expansion, and reachability.

use std::collections::HashMap;

use sysml_core::{ElementId, ElementKind, Value};

use crate::expressions::EvalContext;
use crate::orchestrator::SubsystemIndex;
use crate::statemachine::StateMachineCompiler;

use super::*;

/// Specification for an instance that needs subsystem multiplication.
///
/// When the model has `part circuit1 : CircuitPath` and CircuitPath contains
/// SMs/ODEs, this struct describes what to duplicate for `circuit1`.
#[derive(Debug, Clone)]
pub struct InstanceSpec {
    /// Instance name used as variable prefix (e.g., "circuit1")
    pub prefix: String,
    /// State machine names to duplicate for this instance
    pub sm_names: Vec<String>,
    /// ODE detections to duplicate for this instance
    pub ode_detections: Vec<OdeDetection>,
    /// Name of the containing part (e.g. `"Panel"`). Prepended to
    /// `prefix` when building the canonical variable key so the
    /// runtime's scalar_vars keys match the frontend tree's
    /// `ownerPath`. `None` for instances whose container has no name.
    pub container_name: Option<String>,
    /// ElementId of the `PartDefinition` this instance is typed by
    /// (e.g. `CircuitPath`). Used to walk sub-part usage chains
    /// inside the type when computing canonical variable keys.
    pub type_def_id: Option<ElementId>,
    /// ElementId of the `PartUsage` element this instance corresponds to
    /// (e.g. the `part circuit1 : CircuitPath;` usage). Head of the
    /// `RuntimeId::instance_path` usage chain for this instance's
    /// variables (RSC-2.1). `None` only on legacy construction paths.
    pub usage_id: Option<ElementId>,
    /// RSC-3.4 / L32: per-instance config attribute defaults, extracted
    /// from `build_config_maps` for each ODE in `ode_detections`.
    /// Each entry is `(key, default_value)` where key is a simple attribute
    /// name (not prefixed). Minted as Parameter slots in step 3b of
    /// `mint_slot_store`.
    pub config_entries: Vec<(String, f64)>,
    /// RSC-4.2 (L39): `SubsystemIndex` of each entry in `sm_names`, captured
    /// at its `add_state_machine_prefixed_with_canonical` registration call
    /// site (instance multiplication, step 4). Empty until that step runs;
    /// consumed by `wire_zero_crossing_detectors` so `accept when` crossing
    /// wiring targets the exact registered SM instead of re-deriving it by
    /// name. A name present in `sm_names` that also compiled successfully
    /// during wiring is guaranteed to have an entry here — both steps call
    /// the same deterministic `compile_state_machine` over the same graph.
    pub sm_subsystem_indices: HashMap<String, SubsystemIndex>,
}

// ---------------------------------------------------------------------------
// Instance tree (RSC-2.1)
// ---------------------------------------------------------------------------

/// A node in the part-usage instance tree: one `PartDefinition` or
/// `PartUsage` element with its resolved type and its direct part-usage
/// containment children.
///
/// The tree covers EVERY part uniformly — single instances and parts
/// without SMs/ODEs included. Instance discovery (which parts get
/// subsystem multiplication) is a *derivation* on top of this tree
/// ([`ModelCompiler::instance_specs_from_tree`]), not a property of it.
#[derive(Debug, Clone)]
pub struct InstanceNode {
    /// The `PartDefinition` / `PartUsage` element this node represents.
    pub element_id: ElementId,
    /// Element kind (`PartDefinition` or `PartUsage`).
    pub kind: ElementKind,
    /// Declared name, if any.
    pub name: Option<String>,
    /// Resolved type name for usages (`part circuit1 : CircuitPath` →
    /// `"CircuitPath"`). Resolution order: `unresolvedTypeName` prop →
    /// `unresolved_type` prop → owned `FeatureTyping` child's
    /// `unresolved_type`. `None` for untyped usages and definitions.
    pub type_name: Option<String>,
    /// ElementId of the first graph element whose name matches
    /// `type_name` (the same lookup legacy instance discovery used).
    pub type_def_id: Option<ElementId>,
    /// Direct `PartUsage` containment children, in `children_of` order.
    pub children: Vec<ElementId>,
    /// Owning node when the owner is itself a part node; `None` for roots
    /// (parts owned by packages or nothing).
    pub parent: Option<ElementId>,
}

/// One constraint verdict for one occurrence of the constraint's owning
/// definition, produced by [`ModelCompiler::evaluate_constraints_per_instance`].
/// `instance_*` identify the usage occurrence the verdict was evaluated against;
/// both are `None` for a constraint with no instantiable owner (package-level).
#[derive(Debug, Clone)]
pub struct PerInstanceConstraintResult {
    /// The underlying evaluation result (satisfied / inconclusive, operands,
    /// diagnostics).
    pub result: crate::constraints::EvaluationResult,
    /// ElementId of the usage occurrence this verdict was evaluated against.
    pub instance_element_id: Option<ElementId>,
    /// Name of the usage occurrence (display + dedup-key component).
    pub instance_path: Option<String>,
}

/// The occurrences at which a constraint must be evaluated. See
/// [`ModelCompiler::evaluate_constraints_per_instance`].
enum ConstraintOccurrences {
    /// No instantiable owner — evaluate once with no instance identity.
    None,
    /// Owning definition has zero usages — omit (no evaluation performance).
    Zero,
    /// One verdict per listed (usage element id, usage name) occurrence.
    Occurrences(Vec<(ElementId, Option<String>)>),
}

/// Forest of part-usage containment trees for a model graph
/// (RSC-2.1 instance-tree pass). Built by
/// [`ModelCompiler::build_instance_tree`].
#[derive(Debug, Clone, Default)]
pub struct InstanceTree {
    nodes: HashMap<ElementId, InstanceNode>,
    roots: Vec<ElementId>,
}

impl InstanceTree {
    /// Look up a node by its element id.
    pub fn node(&self, id: &ElementId) -> Option<&InstanceNode> {
        self.nodes.get(id)
    }

    /// Root nodes: parts not contained in another part.
    pub fn roots(&self) -> &[ElementId] {
        &self.roots
    }

    /// Iterate all nodes (arbitrary order — pair with the graph's element
    /// iteration when deterministic container order matters).
    pub fn nodes(&self) -> impl Iterator<Item = &InstanceNode> {
        self.nodes.values()
    }

    /// Number of part nodes in the forest.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when the model has no part elements.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The chain of usage element ids from the root container down to
    /// `id`, outermost first, **excluding** definition nodes (a
    /// `PartDefinition` is type-level context, not an instantiating
    /// usage). Returns `None` when `id` is not in the tree.
    pub fn usage_path(&self, id: &ElementId) -> Option<Vec<ElementId>> {
        let mut path = Vec::new();
        let mut cursor = self.nodes.get(id)?;
        loop {
            if cursor.kind == ElementKind::PartUsage {
                path.push(cursor.element_id.clone());
            }
            match cursor.parent.as_ref().and_then(|p| self.nodes.get(p)) {
                Some(parent) => cursor = parent,
                None => break,
            }
        }
        path.reverse();
        Some(path)
    }
}

// ---------------------------------------------------------------------------
// Slot-table minting inputs (RSC-2.1)
// ---------------------------------------------------------------------------

impl ModelCompiler {
    /// Canonical tree-path prefix for an instance's ODE variables
    /// (`{container}.{instance}.{sub_parts…}`), plus the named usage chain
    /// inside the instance's type that leads to the ODE owner. Shared by
    /// subsystem creation (canonical writeback keys) and slot minting
    /// (RSC-2.1) so both derive identical names.
    pub(crate) fn instance_canonical_prefix(
        &self,
        inst: &InstanceSpec,
        ode: &OdeDetection,
    ) -> (String, Vec<(String, ElementId)>) {
        let sub_path = match (inst.type_def_id.as_ref(), ode.owner_id.as_ref()) {
            (Some(type_id), Some(owner_id)) => self.in_type_path_with_ids(type_id, owner_id),
            _ => Vec::new(),
        };
        let mut canonical_segments: Vec<&str> = Vec::new();
        if let Some(c) = inst.container_name.as_deref() {
            canonical_segments.push(c);
        }
        canonical_segments.push(inst.prefix.as_str());
        for (s, _) in &sub_path {
            canonical_segments.push(s.as_str());
        }
        (canonical_segments.join("."), sub_path)
    }

    // -----------------------------------------------------------------------
    // Slot-table minting (RSC-2.1)
    // -----------------------------------------------------------------------

    /// Build the part-usage instance tree (RSC-2.1): one node per
    /// `PartDefinition` / `PartUsage` element, children = direct
    /// `PartUsage` containment children, with the resolved type recorded
    /// on every node. EVERY part appears — single instances and parts
    /// without SMs/ODEs included. Instance discovery and RuntimeId
    /// minting both derive from this pass.
    pub fn build_instance_tree(&self) -> InstanceTree {
        let mut nodes: HashMap<ElementId, InstanceNode> = HashMap::new();
        let mut roots: Vec<ElementId> = Vec::new();

        for element in self.graph.elements.values() {
            if !matches!(
                element.kind,
                ElementKind::PartDefinition | ElementKind::PartUsage
            ) {
                continue;
            }

            let type_name = Self::resolve_usage_type_name(&self.graph, element);
            // Same lookup legacy discovery used: first element (any kind)
            // whose name matches the resolved type name.
            let type_def_id = type_name.as_deref().and_then(|tn| {
                self.graph
                    .elements
                    .values()
                    .find(|e| e.name.as_deref() == Some(tn))
                    .map(|e| e.id.clone())
            });
            let children: Vec<ElementId> = self
                .graph
                .children_of(&element.id)
                .filter(|c| c.kind == ElementKind::PartUsage)
                .map(|c| c.id.clone())
                .collect();
            let parent = element.owner.clone().filter(|owner_id| {
                self.graph.get_element(owner_id).is_some_and(|owner| {
                    matches!(
                        owner.kind,
                        ElementKind::PartDefinition | ElementKind::PartUsage
                    )
                })
            });
            if parent.is_none() {
                roots.push(element.id.clone());
            }
            nodes.insert(
                element.id.clone(),
                InstanceNode {
                    element_id: element.id.clone(),
                    kind: element.kind.clone(),
                    name: element.name.clone(),
                    type_name,
                    type_def_id,
                    children,
                    parent,
                },
            );
        }

        InstanceTree { nodes, roots }
    }

    /// Evaluate every precompiled constraint PER OCCURRENCE of its owning
    /// definition.
    ///
    /// Spec basis: a `ConstraintUsage` is a `BooleanEvaluation` performed in the
    /// context of each occurrence of its featuringType (Constraints.sysml:23,
    /// `constraintChecks :> booleanEvaluations`; Performances.kerml:94-102,
    /// `BooleanEvaluation` returns `Boolean[1]` per performance). Therefore:
    ///
    /// - A constraint declared on a **PartDefinition** with N part-usages typed
    ///   by it yields **N verdicts**, one per usage occurrence, each evaluated
    ///   against that usage's bound values.
    /// - A constraint declared inside a **PartUsage** yields **one** verdict for
    ///   that usage occurrence.
    /// - A constraint on a definition with **zero** usages is **omitted** (no
    ///   occurrence ⇒ no evaluation performance).
    /// - A constraint with no instantiable owner (package-level / other) yields
    ///   one verdict with no instance identity — the pre-per-instance behavior.
    ///
    /// The single-occurrence, package, and non-Part-definition paths build the
    /// context exactly as the legacy single-verdict path did (base context +
    /// concrete owner/ancestor overlay), so they are byte-identical; only the
    /// ≥2-usage case takes the new per-instance seed+overlay path. This is the
    /// one home for owner-scoped constraint context construction — service-layer
    /// consumers route through it rather than re-implementing the overlay.
    pub fn evaluate_constraints_per_instance(
        &self,
        precompiled: &PrecompiledConstraintSet,
        base_ctx: &EvalContext,
    ) -> Result<Vec<PerInstanceConstraintResult>, CompileError> {
        let tree = self.build_instance_tree();
        let mut out = Vec::new();
        for tc in &precompiled.compiled {
            let owner_id = tc.constraint.owner_id.clone();
            match self.constraint_occurrences(&tree, owner_id.as_ref()) {
                ConstraintOccurrences::None => {
                    let ctx = self.constraint_eval_ctx(base_ctx, owner_id.as_ref(), None)?;
                    out.push(PerInstanceConstraintResult {
                        result: tc.evaluate(&ctx),
                        instance_element_id: None,
                        instance_path: None,
                    });
                }
                // Zero occurrences of the owning definition: nothing to evaluate.
                ConstraintOccurrences::Zero => {}
                ConstraintOccurrences::Occurrences(usages) => {
                    let multi = usages.len() >= 2;
                    for (uid, name) in usages {
                        // Single occurrence reuses the legacy base+overlay context
                        // (byte-identical); multiple occurrences each get their
                        // own instance-seeded context so verdicts are independent.
                        let prefix = if multi { name.as_deref() } else { None };
                        let ctx = self.constraint_eval_ctx(base_ctx, owner_id.as_ref(), prefix)?;
                        out.push(PerInstanceConstraintResult {
                            result: tc.evaluate(&ctx),
                            instance_element_id: Some(uid),
                            instance_path: name,
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    /// Determine the occurrences (usage instances) at which a constraint owned
    /// by `owner_id` must be evaluated. See [`evaluate_constraints_per_instance`].
    fn constraint_occurrences(
        &self,
        tree: &InstanceTree,
        owner_id: Option<&ElementId>,
    ) -> ConstraintOccurrences {
        let Some(owner_id) = owner_id else {
            return ConstraintOccurrences::None;
        };
        let Some(owner) = self.graph.get_element(owner_id) else {
            return ConstraintOccurrences::None;
        };
        match owner.kind {
            // Constraint declared inside a usage occurrence: that usage IS the
            // single occurrence.
            ElementKind::PartUsage => {
                ConstraintOccurrences::Occurrences(vec![(owner_id.clone(), owner.name.clone())])
            }
            // Constraint declared on a definition: one occurrence per part-usage
            // typed by that definition. Sorted for deterministic output.
            ElementKind::PartDefinition => {
                let mut usages: Vec<(ElementId, Option<String>)> = tree
                    .nodes
                    .values()
                    .filter(|n| {
                        n.kind == ElementKind::PartUsage && n.type_def_id.as_ref() == Some(owner_id)
                    })
                    .map(|n| (n.element_id.clone(), n.name.clone()))
                    .collect();
                usages.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
                if usages.is_empty() {
                    ConstraintOccurrences::Zero
                } else {
                    ConstraintOccurrences::Occurrences(usages)
                }
            }
            // Package-level / other owners have no occurrence dimension.
            _ => ConstraintOccurrences::None,
        }
    }

    /// Build the evaluation context for one constraint occurrence.
    ///
    /// `prefix` is `Some(instance_name)` only for a multi-occurrence definition
    /// constraint; in that case the instance's prefixed config values are seeded
    /// and overlaid onto the owner-scoped bare names so the constraint resolves
    /// against THIS occurrence's values. For single-occurrence / package
    /// constraints `prefix` is `None` and the context is the base context plus
    /// the concrete owner/ancestor overlay — identical to the legacy path.
    fn constraint_eval_ctx(
        &self,
        base_ctx: &EvalContext,
        owner_id: Option<&ElementId>,
        prefix: Option<&str>,
    ) -> Result<EvalContext, CompileError> {
        let mut ctx = base_ctx.scratch_snapshot();
        if let Some(prefix) = prefix {
            self.seed_instance_config_into(&mut ctx, prefix, &[])?;
            if let Some(owner) = owner_id {
                for f in owner_scoped_features(&self.graph, owner) {
                    let prefixed = format!("{}.{}", prefix, f.name);
                    if let Some(v) = ctx.get(&prefixed).cloned() {
                        ctx.set(f.name.clone(), v);
                    }
                }
            }
        }
        if let Some(owner) = owner_id {
            self.overlay_concrete_owner_scope(&mut ctx, owner, true);
            for ancestor in sysml_core::query::ancestors(&self.graph, owner) {
                self.overlay_concrete_owner_scope(&mut ctx, &ancestor.id, false);
            }
        }
        Ok(ctx)
    }

    /// Overlay the concrete (value/default) attribute children of `scope_id`
    /// into `ctx`. The immediate owner force-overwrites; ancestor scopes fill
    /// only names not already bound. Value-LESS attributes are NOT overlaid (no
    /// `Value::Ref` clobber) — they fall through to the base context's bound
    /// instance value, else stay unbound so the constraint keeps an honest
    /// "undefined variable — inconclusive" verdict. This is the increment-1
    /// owner overlay, lifted here as the single home.
    fn overlay_concrete_owner_scope(
        &self,
        ctx: &mut EvalContext,
        scope_id: &ElementId,
        is_owner: bool,
    ) {
        for child in self.graph.children_of(scope_id) {
            if let Some(name) = &child.name {
                if !is_owner && ctx.get(name).is_some() {
                    continue;
                }
                if let Some(val) = child.get_prop("value") {
                    ctx.set(name.clone(), val.clone());
                } else if let Some(val) = child.get_prop("default") {
                    ctx.set(name.clone(), val.clone());
                }
            }
        }
    }

    /// Discover part usage instances that need subsystem multiplication.
    ///
    /// RSC-2.1: implemented as a derivation over [`build_instance_tree`]
    /// (uniform part-usage containment pass) instead of the former ad-hoc
    /// container scan. The selection semantics are unchanged and
    /// parity-pinned by `instance_discovery_matches_legacy_*` tests:
    /// a container part with ≥2 named same-type part-usage children whose
    /// type reaches SMs/ODEs yields one `InstanceSpec` per child.
    pub(crate) fn expand_part_instances(&self) -> Vec<InstanceSpec> {
        let tree = self.build_instance_tree();
        self.instance_specs_from_tree(&tree)
    }

    /// Derive subsystem-multiplication specs from the instance tree.
    ///
    /// Containers are visited in the graph's element iteration order
    /// (identical to the legacy scan) so the produced instance order —
    /// and therefore subsystem creation order — is byte-identical.
    fn instance_specs_from_tree(&self, tree: &InstanceTree) -> Vec<InstanceSpec> {
        let mut instances = Vec::new();

        // Collect all SM names and ODE owner names for matching (both paths)
        let sm_names: Vec<String> = StateMachineCompiler::list_state_machine_names(&self.graph);
        let ode_detections = self.detect_all_odes_unified();
        let ode_owner_names: Vec<String> = ode_detections
            .iter()
            .filter_map(|d| d.name.clone())
            .collect();

        for element in self.graph.elements.values() {
            let Some(container) = tree.node(&element.id) else {
                continue;
            };

            // Named part-usage children (the legacy ≥2 threshold counts
            // named children regardless of whether their type resolved).
            //
            // Standard-library part usages are vocabulary, not user
            // subsystems, and must be excluded from multiplication — exactly
            // as the SM path (`StateMachineCompiler::compile_all`) and the
            // root-action path (`build_workspace_orchestrator` §1b) already
            // filter `is_library_element`. Without this, a library
            // `StateAction`/`Performance` definition whose internal pseudo-state
            // features (`start`, `done`, both typed by the library `Part`) are
            // ≥2 same-typed usages gets mis-expanded into spurious prefixed
            // subsystems (`start.StateAction`, `done.Part`, …) that shadow the
            // real state machine + ODE. This filter is a pure classification
            // predicate: it changes only WHICH elements are user subsystems,
            // never how any subsystem is indexed, scheduled, or routed.
            let named_children: Vec<&InstanceNode> = container
                .children
                .iter()
                .filter_map(|cid| tree.node(cid))
                .filter(|c| c.name.is_some())
                .filter(|c| !self.graph.is_library_element(&c.element_id))
                .collect();
            if named_children.len() < 2 {
                continue;
            }

            // Group by resolved type name
            let mut type_groups: HashMap<&str, Vec<&InstanceNode>> = HashMap::new();
            for child in &named_children {
                if let Some(tn) = child.type_name.as_deref() {
                    type_groups.entry(tn).or_default().push(child);
                }
            }

            for (type_name, group) in &type_groups {
                if group.len() < 2 {
                    continue;
                }

                // Find the type definition and check if it (or its
                // descendants) contain SMs or ODEs.
                let Some(td) = self
                    .graph
                    .elements
                    .values()
                    .find(|e| e.name.as_deref() == Some(*type_name))
                else {
                    continue;
                };

                let reachable_sms = self.find_reachable_sms(td, &sm_names);
                let reachable_odes =
                    self.find_reachable_odes(td, &ode_owner_names, &ode_detections);

                if reachable_sms.is_empty() && reachable_odes.is_empty() {
                    continue;
                }

                // RSC-3.4 / L32: flatten config maps for all reachable ODEs.
                // The outer map key (e.g. "config") is preserved so that
                // 2-segment FeatureChain RHS reads like `config.bimetalResistance`
                // resolve through the slot binder's prefix-stripped local alias
                // (L32 debt-ledger item: was incorrectly discarding the outer key,
                // leaving slots named `{prefix}.bimetalResistance` while the RHS
                // reads `config.bimetalResistance` — FeatureChain never matched,
                // scoped_view_bypass stayed false for all 20 prefixed ODE instances).
                let config_entries: Vec<(String, f64)> = reachable_odes
                    .iter()
                    .filter_map(|ode| ode.name.as_deref())
                    .flat_map(|ode_name| self.build_config_maps(ode_name))
                    .flat_map(|(outer, val)| {
                        if let Value::Map(map) = val {
                            map.into_iter()
                                .filter_map(move |(k, v)| {
                                    v.as_float().map(|f| (format!("{outer}.{k}"), f))
                                })
                                .collect::<Vec<_>>()
                        } else {
                            vec![]
                        }
                    })
                    .collect();

                for child in group {
                    instances.push(InstanceSpec {
                        prefix: child.name.clone().unwrap_or_default(),
                        sm_names: reachable_sms.clone(),
                        ode_detections: reachable_odes.clone(),
                        container_name: container.name.clone(),
                        type_def_id: Some(td.id.clone()),
                        usage_id: Some(child.element_id.clone()),
                        config_entries: config_entries.clone(),
                        sm_subsystem_indices: HashMap::new(),
                    });
                }
            }
        }

        // WS-C build determinism: the instances are grouped via a `HashMap`
        // (`type_groups`), so their natural order is per-process random. Several
        // build steps overlay per-instance values onto shared (un-prefixed) keys
        // last-writer-wins (e.g. the flat `config.<attr>` fallback), so a
        // nondeterministic instance order makes those values flake run-to-run.
        // Sort by (container, prefix) for a stable, reproducible order — this
        // also makes subsystem registration order deterministic.
        instances.sort_by(|a, b| {
            (a.container_name.as_deref(), a.prefix.as_str())
                .cmp(&(b.container_name.as_deref(), b.prefix.as_str()))
        });
        instances
    }

    // `expand_part_instances_legacy` (the pre-RSC-2.1 container scan kept
    // as a test-only oracle) was deleted at RSC-2.2 after the tree-based
    // pass soaked — `instance_specs_from_tree` is the only discovery path.

    /// Find SM names reachable from a part definition's children (recursive).
    fn find_reachable_sms(
        &self,
        part_def: &sysml_core::Element,
        all_sm_names: &[String],
    ) -> Vec<String> {
        let mut found = Vec::new();

        for child in self.graph.children_of(&part_def.id) {
            // Direct child is a state machine definition
            if matches!(child.kind, ElementKind::StateDefinition) {
                if let Some(name) = &child.name {
                    if all_sm_names.contains(name) {
                        found.push(name.clone());
                    }
                }
            }

            // Child is a PartUsage or StateUsage typed to something that contains/is an SM
            if matches!(child.kind, ElementKind::PartUsage | ElementKind::StateUsage) {
                let type_name = child
                    .get_prop("unresolvedTypeName")
                    .or_else(|| child.get_prop("unresolved_type"))
                    .and_then(|v| v.as_str());

                if let Some(tn) = type_name {
                    // Direct match: the type itself is an SM definition
                    if all_sm_names.contains(&tn.to_owned()) && !found.contains(&tn.to_owned()) {
                        found.push(tn.to_owned());
                    }
                    if let Some(type_def) = self
                        .graph
                        .elements
                        .values()
                        .find(|e| e.name.as_deref() == Some(tn))
                    {
                        found.extend(self.find_reachable_sms(type_def, all_sm_names));
                    }
                }
                // Also check FeatureTyping children
                for ft in self.graph.children_of(&child.id) {
                    if ft.kind == ElementKind::FeatureTyping {
                        if let Some(ftn) = ft.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                            if all_sm_names.contains(&ftn.to_owned())
                                && !found.contains(&ftn.to_owned())
                            {
                                found.push(ftn.to_owned());
                            }
                        }
                    }
                }
            }
        }

        found
    }

    /// Find ODE detections reachable from a part definition's children (recursive).
    fn find_reachable_odes(
        &self,
        part_def: &sysml_core::Element,
        all_ode_names: &[String],
        all_detections: &[OdeDetection],
    ) -> Vec<OdeDetection> {
        let mut found = Vec::new();

        for child in self.graph.children_of(&part_def.id) {
            // Direct child is a PartDefinition/PartUsage that has an ODE
            if let Some(name) = &child.name {
                if all_ode_names.contains(name) {
                    if let Some(det) = all_detections
                        .iter()
                        .find(|d| d.name.as_deref() == Some(name.as_str()))
                    {
                        found.push(det.clone());
                    }
                }
            }

            // Check typed children: if type IS an ODE definition, or recurse into it
            if child.kind == ElementKind::PartUsage {
                let type_name = child
                    .get_prop("unresolvedTypeName")
                    .or_else(|| child.get_prop("unresolved_type"))
                    .and_then(|v| v.as_str());

                if let Some(tn) = type_name {
                    // Direct match: the type itself is an ODE definition
                    if all_ode_names.contains(&tn.to_owned()) {
                        if let Some(det) = all_detections
                            .iter()
                            .find(|d| d.name.as_deref() == Some(tn))
                        {
                            found.push(det.clone());
                        }
                    }
                    // Also check FeatureTyping children for the type name
                    if let Some(type_def) = self
                        .graph
                        .elements
                        .values()
                        .find(|e| e.name.as_deref() == Some(tn))
                    {
                        found.extend(self.find_reachable_odes(
                            type_def,
                            all_ode_names,
                            all_detections,
                        ));
                    }
                }
                // Also check FeatureTyping children of the part usage
                for ft in self.graph.children_of(&child.id) {
                    if ft.kind != ElementKind::FeatureTyping {
                        continue;
                    }
                    let ft_type = ft.get_prop("unresolved_type").and_then(|v| v.as_str());
                    if let Some(ftn) = ft_type {
                        if all_ode_names.contains(&ftn.to_owned()) {
                            if let Some(det) = all_detections
                                .iter()
                                .find(|d| d.name.as_deref() == Some(ftn))
                            {
                                if !found.iter().any(|f| f.name.as_deref() == Some(ftn)) {
                                    found.push(det.clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        found
    }

    // RSC-4.2 (C.4): `seed_config_defaults` + `seed_config_from_type` were
    // DELETED. They seeded per-ODE config attributes as a collapsed
    // `{prefix}.config` `Value::Map` in the master context so name-first
    // FeatureChain reads (`config.bimetalResistance`) resolved. Config
    // attributes are now minted as typed slots (`mint_slot_store` step 3b) and
    // every reader — instance-scoped computed expressions and ODE derivatives
    // — binds `config.X` chains to those slots via `SlotBinder::for_subsystem`.
    // The Map (and its union-merge, `merge_config_map_into`) was vestigial once
    // the slot plane served every read (proven byte-identical across the corpus
    // and all baselines).

    /// Seed instance-specific config overrides from inline PartUsage attributes.
    ///
    /// When a circuit instance like `part circuit1 : CircuitPath { attribute config : CircuitConfig {
    ///     attribute ratedCurrent = 16.0; ... } }` exists, this extracts the inline
    /// attribute values and seeds them as `circuit1.config.ratedCurrent`, etc.
    ///
    /// Also seeds config defaults for each ODE definition reachable from the instance,
    /// and promotes key config values (e.g., `ratedCurrent`) to the instance prefix level.
    pub(crate) fn seed_instance_config_into(
        &self,
        ctx: &mut EvalContext,
        instance_prefix: &str,
        ode_detections: &[OdeDetection],
    ) -> Result<(), CompileError> {
        // Find the PartUsage element for this instance (e.g., "circuit1")
        let instance_elem = self.graph.elements.values().find(|e| {
            e.kind == ElementKind::PartUsage && e.name.as_deref() == Some(instance_prefix)
        });
        let Some(inst) = instance_elem else { return Ok(()) };

        // Walk inline children to find config overrides
        for child in self.graph.children_of(&inst.id) {
            if child.kind != ElementKind::AttributeUsage {
                continue;
            }
            let child_name = match &child.name {
                Some(n) => n.clone(),
                None => continue,
            };

            // Check if this child has nested attribute overrides (inline config body)
            let nested_children: Vec<_> = self.graph.children_of(&child.id).collect();
            if nested_children
                .iter()
                .any(|c| c.kind == ElementKind::AttributeUsage)
            {
                // This is a config object — extract its children's values
                for nested in &nested_children {
                    if nested.kind != ElementKind::AttributeUsage {
                        continue;
                    }
                    if let Some(nc_name) = &nested.name {
                        let raw = nested
                            .get_prop("default")
                            .or_else(|| nested.get_prop("value"));
                        let num = raw.and_then(|v| match v {
                            Value::Float(f) => Some(*f),
                            Value::Int(i) => Some(*i as f64),
                            _ => None,
                        });
                        if let Some(f) = num {
                            let key = format!("{}.{}.{}", instance_prefix, child_name, nc_name);
                            ctx.set(key, Value::Float(f));

                            // Also promote key values to the instance's ODE namespace.
                            // E.g., circuit1.config.ratedCurrent → circuit1.ratedCurrent
                            // so the ODE expression `ratedCurrent` resolves in scoped context.
                            let promoted = format!("{}.{}", instance_prefix, nc_name);
                            ctx.set(promoted, Value::Float(f));
                        } else if let Some(raw) = raw {
                            // Non-numeric config value (bool/string/enum): preserve
                            // it at the storage key so per-instance constraint
                            // evaluation can reference it. NOT promoted to the ODE
                            // namespace, which is numeric-only.
                            let key = format!("{}.{}.{}", instance_prefix, child_name, nc_name);
                            ctx.set(key, raw.clone());
                        }
                    }
                }
            } else {
                // Simple attribute with a value. Numeric values keep their
                // existing Float seeding (byte-identical); non-numeric values
                // (bool/string/enum) are preserved too so per-instance
                // constraint evaluation can reference them by name.
                let raw = child
                    .get_prop("default")
                    .or_else(|| child.get_prop("value"));
                if let Some(raw) = raw {
                    let key = format!("{}.{}", instance_prefix, child_name);
                    if let Some(f) = raw.as_float() {
                        ctx.set(key, Value::Float(f));
                    } else {
                        ctx.set(key, raw.clone());
                    }
                }
            }
        }

        // RSC-4.2 (C.4): per-ODE config-default seeding as a context `Value::Map`
        // (`seed_config_defaults`) was DELETED. Config attributes are minted as
        // typed slots (`mint_slot_store` step 3b, from `config_entries` built by
        // `build_config_maps`), and instance-scoped expressions now bind
        // `config.X` chains to those slots via `SlotBinder::for_subsystem`. The
        // collapsed `{prefix}.config` Map was vestigial once every reader
        // resolved through the slot plane (proven byte-identical across the
        // corpus + baselines), so the Map seeding and its union-merge are gone.
        let _ = ode_detections;
        Ok(())
    }

    // RSC-4.2 (C.4): `merge_config_map_into` was DELETED. It union-merged
    // per-ODE config `Value::Map`s under a collapsed `{prefix}.config` key (a
    // SPEC-SILENT collapsed-addressing shortcut) so name-first FeatureChain
    // reads resolved and co-located config types didn't clobber each other.
    // With text-prefixing gone and every config reader binding `config.X`
    // chains to typed slots (`mint_slot_store` step 3b + `SlotBinder::
    // for_subsystem`), the collapsed Map is never read — the merge is dead.

}
