//! Query functions for SysML v2 ModelGraph.
//!
//! Higher-level query functions built on top of the core ModelGraph type.
//! (Formerly the standalone `sysml-query` crate.)

use crate::{Element, ElementId, ElementKind, ModelGraph, RelationshipKind, Value};

/// Find elements by name, optionally filtered by kind.
///
/// # Arguments
///
/// * `graph` - The model graph to search
/// * `kind` - Optional element kind filter
/// * `name` - The name to search for (exact match)
///
/// # Returns
///
/// An iterator over matching elements.
pub fn find_by_name<'a>(
    graph: &'a ModelGraph,
    kind: Option<&'a ElementKind>,
    name: &'a str,
) -> impl Iterator<Item = &'a Element> {
    graph.elements.values().filter(move |e| {
        let name_matches = e.name.as_deref() == Some(name);
        let kind_matches = kind.is_none_or(|k| &e.kind == k);
        name_matches && kind_matches
    })
}

/// Find elements by name pattern (contains).
///
/// # Arguments
///
/// * `graph` - The model graph to search
/// * `kind` - Optional element kind filter
/// * `pattern` - The pattern to search for (substring match)
pub fn find_by_name_contains<'a>(
    graph: &'a ModelGraph,
    kind: Option<&'a ElementKind>,
    pattern: &'a str,
) -> impl Iterator<Item = &'a Element> {
    graph.elements.values().filter(move |e| {
        let name_matches = e.name.as_ref().is_some_and(|n| n.contains(pattern));
        let kind_matches = kind.is_none_or(|k| &e.kind == k);
        name_matches && kind_matches
    })
}

/// Find all requirements that are applicable.
///
/// Looks for requirements with an "applicability" property set to "applicable".
pub fn requirements_applicable(graph: &ModelGraph) -> impl Iterator<Item = &Element> {
    graph
        .elements_by_kind(&ElementKind::RequirementUsage)
        .filter(|e| {
            e.get_prop("applicability")
                .and_then(|v| v.as_str())
                .is_none_or(|s| s == "applicable" || s == "Applicable")
        })
}

/// Find all requirements that are not yet verified.
///
/// "Verified" is the same rollup [`elements_verifying`] answers — both are
/// built on [`verify_effective_targets`], so the two can never disagree: a
/// requirement is verified by a direct incoming Verify edge OR through a
/// membership-owned check-usage typed by it.
pub fn requirements_unverified(graph: &ModelGraph) -> impl Iterator<Item = &Element> {
    let mut verified_ids: std::collections::HashSet<ElementId> =
        std::collections::HashSet::new();
    for rel in graph.relationships_by_kind(&RelationshipKind::Verify) {
        verified_ids.extend(verify_effective_targets(graph, rel));
    }

    graph
        .elements_by_kind(&ElementKind::RequirementUsage)
        .filter(move |e| !verified_ids.contains(&e.id))
}

/// The requirements a single Verify edge effectively verifies — THE one seam
/// for the def-rollup opinion; every "is R verified / who verifies R"
/// consumer must route through this (via [`elements_verifying`] or
/// [`requirements_unverified`]), never re-derive it from raw edges.
///
/// Edge convention: source = verification case, target = the element the
/// `RequirementVerificationMembership` identifies (the spec's
/// `referencedConstraint` — see
/// `elaborate::requirements::verification_membership_target`). For the
/// declaration form `verify requirement check : ReqDef;` that target is the
/// LOCAL check-usage; the requirements the check is typed by are verified
/// THROUGH it. So an edge covers: its direct target always, plus — when the
/// target is a membership-owned check-usage — every requirement the check is
/// typed by.
fn verify_effective_targets(graph: &ModelGraph, rel: &crate::Relationship) -> Vec<ElementId> {
    let mut out = vec![rel.target.clone()];
    if is_verification_check_usage(graph, &rel.target) {
        out.extend(crate::resolution::scoping::chaining::find_feature_types(
            graph,
            &rel.target,
        ));
    }
    out
}

/// A membership-owned verification check-usage: a `RequirementUsage` owned by
/// a `RequirementVerificationMembership` (the `verify requirement …`
/// declaration form).
///
/// This is the ONE role predicate separating requirement CONTENT from
/// verification BOOKKEEPING (the normative library calls these "a record
/// of the evaluations", VerificationCases.sysml) — shared by the verify
/// rollup, `requirement_rows`' default exclusion, and `requirement_detail`'s
/// `instantiated_by` (steward ruling 2026-07-16: one taxonomy, never two).
pub fn is_verification_check_usage(graph: &ModelGraph, id: &ElementId) -> bool {
    graph.get_element(id).is_some_and(|e| {
        e.kind == ElementKind::RequirementUsage
            && e.owner
                .as_ref()
                .and_then(|o| graph.get_element(o))
                .is_some_and(|owner| {
                    owner.kind == ElementKind::RequirementVerificationMembership
                })
    })
}

/// A row in a trace matrix.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TraceMatrixRow {
    /// The source element.
    pub source: ElementId,
    /// The source element name.
    pub source_name: Option<String>,
    /// The target element.
    pub target: ElementId,
    /// The target element name.
    pub target_name: Option<String>,
    /// The relationship id.
    pub relationship: ElementId,
}

/// Generate a trace matrix between two element kinds via a relationship kind.
///
/// # Arguments
///
/// * `graph` - The model graph
/// * `source_kind` - The kind of source elements
/// * `rel_kind` - The relationship kind to trace
/// * `target_kind` - The kind of target elements
///
/// # Returns
///
/// A vector of trace matrix rows.
pub fn trace_matrix(
    graph: &ModelGraph,
    source_kind: &ElementKind,
    rel_kind: &RelationshipKind,
    target_kind: &ElementKind,
) -> Vec<TraceMatrixRow> {
    let mut rows = Vec::new();

    for rel in graph.relationships_by_kind(rel_kind) {
        let source = graph.get_element(&rel.source);
        let target = graph.get_element(&rel.target);

        if let (Some(src), Some(tgt)) = (source, target) {
            let src_match = &src.kind == source_kind || src.kind.is_subtype_of(source_kind.clone());
            let tgt_match = &tgt.kind == target_kind || tgt.kind.is_subtype_of(target_kind.clone());
            if src_match && tgt_match {
                rows.push(TraceMatrixRow {
                    source: src.id.clone(),
                    source_name: src.name.clone(),
                    target: tgt.id.clone(),
                    target_name: tgt.name.clone(),
                    relationship: rel.id.clone(),
                });
            }
        }
    }

    rows
}

/// Find elements that satisfy a given requirement.
pub fn elements_satisfying<'a>(
    graph: &'a ModelGraph,
    requirement_id: &'a ElementId,
) -> impl Iterator<Item = &'a Element> {
    graph
        .incoming(requirement_id)
        .filter(|r| matches!(r.kind, RelationshipKind::Satisfy))
        .filter_map(move |r| graph.get_element(&r.source))
}

/// Find the verification cases verifying a given requirement.
///
/// The one home for "cases verifying R" — every consumer (requirement rows,
/// unverified queries, requirement health) routes through this so no two of
/// them can disagree. Covers both contribution paths of
/// [`verify_effective_targets`]: the direct edge (reference form `verify R;`)
/// and the def rollup (declaration form `verify requirement check : R;`,
/// where the edge targets the membership-owned check-usage and R is verified
/// through it). Deduplicated by case; edge-iteration order preserved.
pub fn elements_verifying<'a>(
    graph: &'a ModelGraph,
    requirement_id: &ElementId,
) -> Vec<&'a Element> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for rel in graph.relationships_by_kind(&RelationshipKind::Verify) {
        if verify_effective_targets(graph, rel).contains(requirement_id)
            && seen.insert(rel.source.clone())
        {
            if let Some(case) = graph.get_element(&rel.source) {
                out.push(case);
            }
        }
    }
    out
}

/// Resolve the requirement a feature is typed by (`: Req`) — SINGLE hop,
/// never a specialization closure.
///
/// The ONE home of requirement feature-typing resolution (steward ruling
/// 2026-07-16), shared by the verification evaluator
/// (`sysml_runtime::cases`) and the workbench contract display
/// (`sysml_query::requirement_detail`) — the display must show exactly
/// what the evaluator reads, so both MUST call this function; a second
/// independent walk would silently drift.
///
/// Prefers the resolved `type` (`Value::Ref`) prop set during reference
/// resolution; falls back to a by-name lookup against `unresolved_type`
/// for graphs that have not been resolved. Only requirement kinds are
/// accepted from the name fallback (a bare name match against arbitrary
/// kinds would be a guess).
pub fn resolve_requirement_typing_target<'g>(
    elem: &Element,
    graph: &'g ModelGraph,
) -> Option<&'g Element> {
    graph.children_of(&elem.id).find_map(|child| {
        if child.kind != ElementKind::FeatureTyping {
            return None;
        }
        if let Some(Value::Ref(id)) = child.props.get("type") {
            if let Some(target) = graph.get_element(id) {
                return Some(target);
            }
        }
        if let Some(name) = child.props.get("unresolved_type").and_then(|v| v.as_str()) {
            return graph.elements.values().find(|e| {
                e.name.as_deref() == Some(name)
                    && matches!(
                        e.kind,
                        ElementKind::RequirementDefinition | ElementKind::RequirementUsage
                    )
            });
        }
        None
    })
}

/// The requirement usages typed by `def_id` — the REVERSE of
/// [`resolve_requirement_typing_target`], and single-hop-consistent with
/// it by construction (a direct `FeatureTyping.type` reverse walk, never
/// a specialization closure; edit the two together). Kind-indexed scan,
/// called per detail request — mint a reverse index only if profiling
/// warrants (the existing graph indexes' own precedent).
///
/// Returns ALL typed requirement usages; role-based consumers (e.g.
/// `instantiated_by`, content-only per the 2026-07-16 ruling) filter with
/// [`is_verification_check_usage`] themselves.
pub fn requirement_usages_typed_by<'g>(
    graph: &'g ModelGraph,
    def_id: &ElementId,
) -> Vec<&'g Element> {
    let mut out = Vec::new();
    for typing in graph.elements_by_kind(&ElementKind::FeatureTyping) {
        let targets = match typing.props.get("type") {
            Some(Value::Ref(id)) => id == def_id,
            _ => false,
        };
        if !targets {
            continue;
        }
        let usage = typing
            .owner
            .as_ref()
            .and_then(|o| graph.get_element(o))
            .filter(|e| e.kind == ElementKind::RequirementUsage);
        if let Some(usage) = usage {
            out.push(usage);
        }
    }
    out
}

/// The redefined feature's name from a child `Redefinition` member
/// (`:>> gap`), used as a display/binding key when the redefining usage
/// carries no name of its own (`attribute :>> gap = 8.0;` in a template
/// instantiation). The parser stamps the (possibly dotted) target name as
/// `unresolved_redefinedFeature`. ONE home — shared by the verification
/// evaluator's binding keys and `requirement_detail`'s attribute names.
pub fn redefined_feature_name(usage: &Element, graph: &ModelGraph) -> Option<String> {
    graph.children_of(&usage.id).find_map(|c| {
        if c.kind != ElementKind::Redefinition {
            return None;
        }
        c.get_prop("unresolved_redefinedFeature")
            .and_then(|v| v.as_str().map(str::to_owned))
    })
}

/// Does this requirement OWN any constraint source the verification
/// evaluator would read? (`ConstraintUsage` / `RequirementConstraintMembership`
/// children, or the legacy `constraint` string prop used by hand-built
/// test graphs.)
///
/// This is the single-hop typing fallback's GATE (steward ruling
/// 2026-07-16): only a requirement that owns nothing evaluates — and
/// displays — its typing target's constraints. Shared by the evaluator
/// and `sysml_query::requirement_detail`; keep them on this one predicate.
pub fn requirement_owns_constraints(elem: &Element, graph: &ModelGraph) -> bool {
    elem.props.contains_key("constraint")
        || graph.children_of(&elem.id).any(|c| {
            matches!(
                c.kind,
                ElementKind::ConstraintUsage | ElementKind::RequirementConstraintMembership
            )
        })
}

/// How an ancestor entered a requirement's inheritance chain (full-chain
/// aggregation ruling 2026-07-17, requirements-workbench-design.md §2.1a).
///
/// Per-row provenance is BINDING for display consumers: a row that
/// travelled two hops must never be misreported as one, so every
/// ancestor records the edge kind that reached it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum RequirementChainHop {
    /// `usage : Def` — a `FeatureTyping` edge.
    Typing,
    /// `def A :> B` — a `Subclassification` edge (def specialization).
    Specialization,
}

/// One ancestor in a requirement's inheritance chain, nearest-first.
#[derive(Debug, Clone, Copy)]
pub struct RequirementChainAncestor<'g> {
    /// The ancestor requirement def/usage.
    pub element: &'g Element,
    /// The edge kind that reached this ancestor from its predecessor.
    pub via: RequirementChainHop,
}

/// The UNCONDITIONAL inheritance chain of a requirement (excluding the
/// element itself): its `FeatureTyping` target, then transitive def
/// specialization (`:>`), breadth-first so nearer ancestors come first
/// (redefinition suppression resolves nearest-wins), cycle-guarded.
///
/// This is the closure ruled by the full-chain aggregation consult
/// (2026-07-17): KerML `Type::featureMembership` = owned ∪ inherited,
/// unconditionally — the earlier "owns no constraints" gate modeled the
/// spec's owned-only introspection properties (`assumedConstraint` /
/// `requiredConstraint`), not the evaluation feature, and is superseded.
/// Scope note (recorded, not an oversight): usage-to-usage subsetting
/// (`r :> other`) is NOT walked — the ruling names typing + def
/// specialization; reference subsetting has its own ruled semantics
/// (reference form = one nested obligation).
///
/// ONE home for the walk — the verification evaluator and
/// `requirement_detail` consume this same function; a second independent
/// walk would drift.
pub fn requirement_inheritance_chain<'g>(
    origin: &Element,
    graph: &'g ModelGraph,
) -> Vec<RequirementChainAncestor<'g>> {
    let mut out: Vec<RequirementChainAncestor<'g>> = Vec::new();
    let mut visited: std::collections::HashSet<ElementId> = std::collections::HashSet::new();
    visited.insert(origin.id.clone());

    // Breadth-first frontier of elements whose outgoing inheritance edges
    // are still to be expanded.
    let mut frontier: Vec<&Element> = vec![origin];
    while !frontier.is_empty() {
        let mut next: Vec<&'g Element> = Vec::new();
        for elem in frontier {
            for (target, via) in inheritance_targets(elem, graph) {
                if !matches!(
                    target.kind,
                    ElementKind::RequirementDefinition | ElementKind::RequirementUsage
                ) {
                    continue;
                }
                if !visited.insert(target.id.clone()) {
                    continue;
                }
                out.push(RequirementChainAncestor { element: target, via });
                next.push(target);
            }
        }
        frontier = next;
    }
    out
}

/// Direct inheritance edges out of one element: `FeatureTyping` targets
/// and `Subclassification` generals, resolved-`Ref`-first with the
/// unresolved-name fallback (same dual-read pattern as
/// [`resolve_requirement_typing_target`]).
fn inheritance_targets<'g>(
    elem: &Element,
    graph: &'g ModelGraph,
) -> Vec<(&'g Element, RequirementChainHop)> {
    let mut out = Vec::new();
    for child in graph.children_of(&elem.id) {
        let (ref_prop, name_prop, hop) = match child.kind {
            ElementKind::FeatureTyping => ("type", "unresolved_type", RequirementChainHop::Typing),
            ElementKind::Subclassification => (
                "superclassifier",
                "unresolved_superclassifier",
                RequirementChainHop::Specialization,
            ),
            _ => continue,
        };
        let target = match child.props.get(ref_prop) {
            Some(Value::Ref(id)) => graph.get_element(id),
            _ => child
                .get_prop(name_prop)
                .and_then(|v| v.as_str())
                .and_then(|name| {
                    let terminal = name.rsplit("::").next().unwrap_or(name);
                    graph.elements.values().find(|e| {
                        e.name.as_deref() == Some(terminal)
                            && matches!(
                                e.kind,
                                ElementKind::RequirementDefinition | ElementKind::RequirementUsage
                            )
                    })
                }),
        };
        if let Some(target) = target {
            out.push((target, hop));
        }
    }
    out
}

/// The role of one requirement constraint member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementConstraintRole {
    /// `assume constraint { … }`
    Assume,
    /// `require constraint { … }` (and bare `ConstraintUsage` children).
    Require,
}

/// One member of a requirement's EFFECTIVE constraint set — owned or
/// inherited, redefinition-suppressed.
#[derive(Debug, Clone, Copy)]
pub struct EffectiveRequirementConstraint<'g> {
    /// The `ConstraintUsage` / `RequirementConstraintMembership` element.
    pub element: &'g Element,
    pub role: RequirementConstraintRole,
    /// `None` = owned by the origin requirement; `Some` = inherited from
    /// that chain ancestor.
    pub origin: Option<&'g Element>,
    /// The edge kind that reached `origin` (`None` for owned members).
    pub via: Option<RequirementChainHop>,
}

/// The effective constraint members of a requirement: owned members
/// first, then each [`requirement_inheritance_chain`] ancestor's members
/// nearest-first, with KerML redefinition suppression — a member whose
/// (terminal) name is redefined by a NEARER member is excluded
/// (`Type::removeRedefinedFeatures`; inheritance is DEFINED through this
/// suppression, so the closure must never ship without it — §2.1a ruling
/// (b)). Suppression matches on redefinition targets only, never bare
/// name shadowing (KerML has no name-shadow semantics).
pub fn effective_requirement_constraints<'g>(
    origin: &'g Element,
    graph: &'g ModelGraph,
) -> Vec<EffectiveRequirementConstraint<'g>> {
    let mut out: Vec<EffectiveRequirementConstraint<'g>> = Vec::new();
    let mut suppressed: std::collections::HashSet<String> = std::collections::HashSet::new();

    let collect_level =
        |elem: &'g Element,
         origin_of_level: Option<&'g Element>,
         via: Option<RequirementChainHop>,
         suppressed: &mut std::collections::HashSet<String>,
         out: &mut Vec<EffectiveRequirementConstraint<'g>>| {
            for child in graph.children_of(&elem.id) {
                let role = match child.kind {
                    ElementKind::ConstraintUsage => RequirementConstraintRole::Require,
                    ElementKind::RequirementConstraintMembership => {
                        if child.get_prop("role").and_then(|v| v.as_str()) == Some("assume") {
                            RequirementConstraintRole::Assume
                        } else {
                            RequirementConstraintRole::Require
                        }
                    }
                    _ => continue,
                };
                // A nearer level's redefinition suppresses this member.
                if let Some(name) = child.name.as_deref() {
                    if suppressed.contains(name) {
                        continue;
                    }
                }
                // This member's own redefinition target suppresses farther
                // levels (same-level redefinition is spec-invalid — the
                // Redefinition constraint requires differing owningTypes —
                // and is simply not special-cased here). Bookkeeping is
                // UNCONDITIONAL: a skipped collector slot still suppresses.
                if let Some(target) = constraint_redefinition_target(child, graph) {
                    suppressed.insert(target);
                }
                // Bare collector SLOTS are not obligations (steward ruling
                // 2026-07-17): a chain that enters the standard library
                // reaches `RequirementConstraintCheck::assumptions/
                // constraints` (and their `:>>` shells on RequirementCheck)
                // — bodyless [0..*] collection features whose semantics
                // live in the def's own `allTrue(assumptions()) implies
                // allTrue(constraints())` result expression. Emitting the
                // slot itself hands the evaluator a phantom compile error.
                if is_vacuous_collector_slot(child, graph) {
                    continue;
                }
                out.push(EffectiveRequirementConstraint {
                    element: child,
                    role,
                    origin: origin_of_level,
                    via,
                });
            }
        };

    collect_level(origin, None, None, &mut suppressed, &mut out);
    for ancestor in requirement_inheritance_chain(origin, graph) {
        collect_level(
            ancestor.element,
            Some(ancestor.element),
            Some(ancestor.via),
            &mut suppressed,
            &mut out,
        );
    }
    out
}

/// The terminal name a requirement-constraint member redefines, if any —
/// read from a `Redefinition` child of the member itself or of its first
/// The element that owns a requirement-constraint body's expression AST:
/// the membership's owned `ConstraintUsage` child when present — the spec
/// shape (`ownedConstraint`, SysML §8.3.21.7: a RequirementConstraintMembership
/// owns exactly one ConstraintUsage, and result expressions live on
/// function-like Types, rule S051) — else the membership itself
/// (hand-crafted test graphs that hang props directly on the membership).
/// ONE home for the hop; the evaluator, requirement_detail, and any other
/// body reader must route through this rather than re-deriving it.
pub fn requirement_constraint_body_owner<'g>(
    membership: &'g Element,
    graph: &'g ModelGraph,
) -> &'g Element {
    graph
        .children_of(&membership.id)
        .find(|c| c.kind == ElementKind::ConstraintUsage)
        .unwrap_or(membership)
}

/// The reference relationship a requirement-constraint / framed-concern
/// reference form carries on its `ownedConstraint`, if any: its declared
/// (unresolved) target name plus the resolved target id once resolution has
/// run.
///
/// Grammar (SysML.xtext:2061-2076 — `RequirementConstraintUsage` /
/// `FramedConcernUsage`: `ownedRelationship += OwnedReferenceSubsetting
/// FeatureSpecialization*`) admits two reference forms, both lowered onto the
/// membership's owned `ConstraintUsage` ([`requirement_constraint_body_owner`]):
///  * bare-name (`require existingUsage;`) → an `OwnedReferenceSubsetting` →
///    `ReferenceSubsetting` (SysML.xtext:448), target in `referencedFeature`;
///  * `: Def` (`require constraint : Def;` / `frame concern : Def;`) → a
///    `FeatureTyping`, target in `type`.
///
/// Returns `None` for inline-body forms (which carry neither relationship).
fn referenced_constraint_relationship<'g>(
    membership: &Element,
    graph: &'g ModelGraph,
) -> Option<(&'g str, Option<&'g ElementId>)> {
    use crate::resolution::{resolved_props, unresolved_props};
    let owned = requirement_constraint_body_owner(membership, graph);
    graph.children_of(&owned.id).find_map(|child| {
        let (unresolved_key, resolved_key) = match child.kind {
            ElementKind::ReferenceSubsetting => (
                unresolved_props::REFERENCED_FEATURE,
                resolved_props::REFERENCED_FEATURE,
            ),
            ElementKind::FeatureTyping => (unresolved_props::TYPE, resolved_props::TYPE),
            _ => return None,
        };
        let name = child.get_prop(unresolved_key).and_then(|v| v.as_str())?;
        let resolved = child.get_prop(resolved_key).and_then(|v| v.as_ref());
        Some((name, resolved))
    })
}

/// The RESOLVED feature a requirement-constraint / framed-concern reference
/// form points at — the derived `referencedConstraint` target
/// (SysML-vocab.ttl:2576, "the referencedFeature of the ownedReferenceSubsetting
/// of the ownedConstraint").
///
/// For the bare-name form this is exactly the vocabulary's branch 1 (the
/// `ReferenceSubsetting`'s resolved `referencedFeature`); for the `: Def` form
/// it is the `FeatureTyping`'s resolved `type` (the definition the local owned
/// constraint specializes — what consumers display and evaluate). Returns
/// `None` for inline-body forms and for references whose target did not resolve
/// (fail-hard: a dangling reference yields nothing — it is never papered over
/// with a name string). Identity only, never a re-stringified name.
///
/// This is the ONE home for the derivation; every consumer that read the old
/// parse-time `referencedConstraint` string prop routes through here (or through
/// [`referenced_constraint_ref_name`] when it needs the declared name for a
/// diagnostic).
pub fn referenced_constraint_target<'g>(
    membership: &Element,
    graph: &'g ModelGraph,
) -> Option<&'g Element> {
    referenced_constraint_relationship(membership, graph)
        .and_then(|(_, resolved)| resolved)
        .and_then(|id| graph.get_element(id))
}

/// The declared (source-text) name of a requirement-constraint reference form,
/// regardless of whether it resolved. `Some` iff the membership carries a
/// reference form (bare-name or `: Def`); used to distinguish a genuine
/// reference from an inline body and to name an unresolved target in a
/// diagnostic. Read the resolved identity via [`referenced_constraint_target`].
pub fn referenced_constraint_ref_name<'g>(
    membership: &Element,
    graph: &'g ModelGraph,
) -> Option<&'g str> {
    referenced_constraint_relationship(membership, graph).map(|(name, _)| name)
}

/// constraint child (the parser hangs the `:>>` clause off whichever
/// element carries the declaration).
fn constraint_redefinition_target(member: &Element, graph: &ModelGraph) -> Option<String> {
    let direct = redefined_feature_name(member, graph);
    let via_child = || {
        graph
            .children_of(&member.id)
            .filter(|c| c.kind == ElementKind::ConstraintUsage)
            .find_map(|c| redefined_feature_name(c, graph))
    };
    direct.or_else(via_child).map(|name| terminal_segment(&name).to_owned())
}

/// Is this constraint member a bare collector SLOT rather than an
/// obligation? (Steward ruling 2026-07-17, filed off the v2 baseline
/// re-bless.)
///
/// The standard library declares `RequirementConstraintCheck::
/// assumptions[0..*]` / `::constraints[0..*]` (Requirements.sysml:27,34)
/// as bodyless collection features subsetting `constraintChecks`/
/// `subperformances`; `RequirementCheck` redefines them "solely to
/// simplify" (ibid.:84-88). Their VALUE is the set of concrete checks
/// bound at usage sites — the check semantics are the def's own
/// `allTrue(assumptions()) implies allTrue(constraints())` result
/// expression, vacuously true over an empty collection. They are
/// aggregation points, not evaluable obligations.
///
/// ALL THREE conditions must hold to skip — no single one may swallow a
/// genuine user constraint:
/// 1. structurally bodyless: no expression AST, no `constraint`/`expr`
///    prop, no `referencedConstraint` reference form, on the member or
///    its body owner;
/// 2. every specialization edge (subsets/redefines) targets a collector
///    marker (`constraintChecks`/`subperformances`/`assumptions`/
///    `constraints`), with at least one such edge and no feature typing
///    (a typed bodyless usage is the reference form, which stays on the
///    evaluator's fail-hard path);
/// 3. the declared multiplicity lower bound is 0 (the declaration's own
///    admission that emptiness is legal) — or, for the `:>>` shells that
///    declare no multiplicity, the member redefines a collector marker
///    and thereby inherits its `[0..*]`.
fn is_vacuous_collector_slot(member: &Element, graph: &ModelGraph) -> bool {
    const COLLECTOR_MARKERS: [&str; 4] =
        ["constraintChecks", "subperformances", "assumptions", "constraints"];

    let owner = requirement_constraint_body_owner(member, graph);
    let carriers: &[&Element] = if owner.id == member.id { &[member] } else { &[member, owner] };

    // A reference form (`require existingUsage;` / `require constraint : Def;`)
    // is an obligation that points elsewhere — never a vacuous collector slot.
    // (Replaces the old `referencedConstraint` string-prop check now that the
    // reference is a real ReferenceSubsetting / FeatureTyping relationship.)
    if referenced_constraint_ref_name(member, graph).is_some() {
        return false;
    }

    // (1) structurally bodyless.
    for e in carriers {
        if e.get_prop("constraint").is_some() || e.get_prop("expr").is_some() {
            return false;
        }
    }
    if graph
        .children_of(&owner.id)
        .any(|c| c.kind == ElementKind::ResultExpressionMembership)
    {
        return false;
    }

    // (2) specialization edges: all onto collector markers, at least one.
    let mut marker_edges = 0usize;
    let mut redefines_marker = false;
    for e in carriers {
        for c in graph.children_of(&e.id) {
            let target = match c.kind {
                ElementKind::Subsetting => c.get_prop("unresolved_subsettedFeature"),
                ElementKind::Redefinition => c.get_prop("unresolved_redefinedFeature"),
                // Typed / referencing bodyless usage = reference form, not a slot.
                ElementKind::FeatureTyping | ElementKind::ReferenceSubsetting => return false,
                _ => continue,
            };
            let Some(name) = target.and_then(|v| v.as_str()) else {
                continue;
            };
            if COLLECTOR_MARKERS.contains(&terminal_segment(name)) {
                marker_edges += 1;
                if c.kind == ElementKind::Redefinition {
                    redefines_marker = true;
                }
            } else {
                return false;
            }
        }
    }
    if marker_edges == 0 {
        return false;
    }

    // (3) lower bound 0 declared, or inherited via a marker redefinition.
    let lower_bound = carriers
        .iter()
        .find_map(|e| e.get_prop("multiplicity_lower").and_then(|v| v.as_int()));
    match lower_bound {
        Some(n) => n == 0,
        None => redefines_marker,
    }
}

/// Last segment of a possibly-qualified (`Base::maxMass`) or dotted
/// (`s.maxMass`) name.
fn terminal_segment(name: &str) -> &str {
    let after_qualifier = name.rsplit("::").next().unwrap_or(name);
    after_qualifier.rsplit('.').next().unwrap_or(after_qualifier)
}

/// Find requirements satisfied by a given element.
pub fn requirements_satisfied_by<'a>(
    graph: &'a ModelGraph,
    element_id: &'a ElementId,
) -> impl Iterator<Item = &'a Element> {
    graph
        .outgoing(element_id)
        .filter(|r| matches!(r.kind, RelationshipKind::Satisfy))
        .filter_map(move |r| graph.get_element(&r.target))
}

/// Find all ancestors of an element (owner chain).
pub fn ancestors<'a>(graph: &'a ModelGraph, element_id: &'a ElementId) -> Vec<&'a Element> {
    let mut result = Vec::new();
    let mut current_id = element_id;

    while let Some(element) = graph.get_element(current_id) {
        if let Some(owner_id) = &element.owner {
            if let Some(owner) = graph.get_element(owner_id) {
                result.push(owner);
                current_id = owner_id;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    result
}

/// Find all descendants of an element (recursive children).
pub fn descendants<'a>(graph: &'a ModelGraph, element_id: &'a ElementId) -> Vec<&'a Element> {
    let mut result = Vec::new();
    let mut stack = vec![element_id.clone()];

    while let Some(id) = stack.pop() {
        for child in graph.children_of(&id) {
            result.push(child);
            stack.push(child.id.clone());
        }
    }

    result
}

/// Find elements by property value.
pub fn find_by_property<'a>(
    graph: &'a ModelGraph,
    key: &'a str,
    value: &'a Value,
) -> impl Iterator<Item = &'a Element> {
    graph
        .elements
        .values()
        .filter(move |e| e.get_prop(key) == Some(value))
}

/// Count relationships by kind.
pub fn count_relationships_by_kind(graph: &ModelGraph) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();

    for rel in graph.relationships.values() {
        *counts.entry(rel.kind.as_str().to_owned()).or_insert(0) += 1;
    }

    counts
}

/// Count elements by kind.
pub fn count_elements_by_kind(graph: &ModelGraph) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();

    for elem in graph.elements.values() {
        *counts.entry(elem.kind.as_str().to_owned()).or_insert(0) += 1;
    }

    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Element, Relationship};

    fn create_test_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();

        // Package
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPackage");
        let pkg_id = graph.add_element(pkg);

        // Requirements
        let req1 = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("SafetyReq")
            .with_owner(pkg_id.clone())
            .with_prop("applicability", "applicable");
        let req1_id = graph.add_element(req1);

        let req2 = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("PerformanceReq")
            .with_owner(pkg_id.clone())
            .with_prop("applicability", "not_applicable");
        let _req2_id = graph.add_element(req2);

        // Parts
        let part1 = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(pkg_id.clone());
        let part1_id = graph.add_element(part1);

        // Verification case
        let vc = Element::new_with_kind(ElementKind::VerificationCaseUsage)
            .with_name("SafetyTest")
            .with_owner(pkg_id.clone());
        let vc_id = graph.add_element(vc);

        // Relationships
        let satisfy =
            Relationship::new(RelationshipKind::Satisfy, part1_id.clone(), req1_id.clone());
        graph.add_relationship(satisfy);

        let verify = Relationship::new(RelationshipKind::Verify, vc_id, req1_id);
        graph.add_relationship(verify);

        graph
    }

    #[test]
    fn test_find_by_name() {
        let graph = create_test_graph();
        let results: Vec<_> = find_by_name(&graph, None, "SafetyReq").collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, Some("SafetyReq".to_string()));
    }

    #[test]
    fn test_find_by_name_with_kind() {
        let graph = create_test_graph();
        let results: Vec<_> =
            find_by_name(&graph, Some(&ElementKind::RequirementUsage), "SafetyReq").collect();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_find_by_name_contains() {
        let graph = create_test_graph();
        let results: Vec<_> = find_by_name_contains(&graph, None, "Req").collect();
        assert_eq!(results.len(), 2); // SafetyReq and PerformanceReq
    }

    #[test]
    fn test_requirements_applicable() {
        let graph = create_test_graph();
        let results: Vec<_> = requirements_applicable(&graph).collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, Some("SafetyReq".to_string()));
    }

    #[test]
    fn test_requirements_unverified() {
        let graph = create_test_graph();
        let results: Vec<_> = requirements_unverified(&graph).collect();
        assert_eq!(results.len(), 1); // PerformanceReq is not verified
        assert_eq!(results[0].name, Some("PerformanceReq".to_string()));
    }

    #[test]
    fn elements_verifying_rolls_up_through_membership_owned_check_usage() {
        // Declaration form: Verify edge targets the membership-owned
        // check-usage; the def the check is typed by is verified THROUGH it.
        // `elements_verifying` and `requirements_unverified` must agree.
        let mut graph = ModelGraph::new();

        let pkg = crate::Element::new_with_kind(ElementKind::Package).with_name("P");
        let pkg_id = graph.add_element(pkg);

        let req_def = crate::Element::new_with_kind(ElementKind::RequirementDefinition)
            .with_name("TripReq")
            .with_owner(pkg_id.clone());
        let req_def_id = graph.add_element(req_def);

        let case = crate::Element::new_with_kind(ElementKind::VerificationCaseDefinition)
            .with_name("T1")
            .with_owner(pkg_id.clone());
        let case_id = graph.add_element(case);

        let membership =
            crate::Element::new_with_kind(ElementKind::RequirementVerificationMembership)
                .with_owner(case_id.clone());
        let membership_id = graph.add_element(membership);

        let check = crate::Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("tripCheck")
            .with_owner(membership_id);
        let check_id = graph.add_element(check);

        // Resolved typing: FeatureTyping child with a `type` Ref (the shape
        // `find_feature_types` reads via the typed_feature_to_typings index).
        let typing = crate::Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(check_id.clone())
            .with_prop("type", crate::Value::Ref(req_def_id.clone()))
            .with_prop("typedFeature", crate::Value::Ref(check_id.clone()));
        graph.add_element(typing);

        graph.add_relationship(crate::Relationship::new(
            RelationshipKind::Verify,
            case_id.clone(),
            check_id.clone(),
        ));

        // Direct target: the check-usage itself.
        let via_check = elements_verifying(&graph, &check_id);
        assert_eq!(via_check.len(), 1);
        assert_eq!(via_check[0].id, case_id);

        // Rollup: the def the check is typed by.
        let via_def = elements_verifying(&graph, &req_def_id);
        assert_eq!(
            via_def.len(),
            1,
            "def is verified through its membership-owned check-usage"
        );
        assert_eq!(via_def[0].id, case_id);

        // requirements_unverified agrees: the check-usage is verified, so no
        // RequirementUsage in this graph is unverified.
        assert!(
            requirements_unverified(&graph).next().is_none(),
            "check-usage carries a direct Verify edge"
        );
    }

    #[test]
    fn test_trace_matrix() {
        let graph = create_test_graph();
        let matrix = trace_matrix(
            &graph,
            &ElementKind::PartUsage,
            &RelationshipKind::Satisfy,
            &ElementKind::RequirementUsage,
        );
        assert_eq!(matrix.len(), 1);
        assert_eq!(matrix[0].source_name, Some("Engine".to_string()));
        assert_eq!(matrix[0].target_name, Some("SafetyReq".to_string()));
    }

    #[test]
    fn test_ancestors() {
        let graph = create_test_graph();
        let part = find_by_name(&graph, Some(&ElementKind::PartUsage), "Engine")
            .next()
            .unwrap();
        let ancestors = ancestors(&graph, &part.id);
        assert_eq!(ancestors.len(), 1);
        assert_eq!(ancestors[0].name, Some("TestPackage".to_string()));
    }

    #[test]
    fn test_descendants() {
        let graph = create_test_graph();
        let pkg = find_by_name(&graph, Some(&ElementKind::Package), "TestPackage")
            .next()
            .unwrap();
        let descendants = descendants(&graph, &pkg.id);
        assert_eq!(descendants.len(), 4); // 2 requirements, 1 part, 1 verification case
    }

    #[test]
    fn test_count_elements_by_kind() {
        let graph = create_test_graph();
        let counts = count_elements_by_kind(&graph);
        assert_eq!(counts.get("Package"), Some(&1));
        assert_eq!(counts.get("RequirementUsage"), Some(&2));
        assert_eq!(counts.get("PartUsage"), Some(&1));
    }

    #[test]
    fn test_count_relationships_by_kind() {
        let graph = create_test_graph();
        let counts = count_relationships_by_kind(&graph);
        assert_eq!(counts.get("Satisfy"), Some(&1));
        assert_eq!(counts.get("Verify"), Some(&1));
    }

    // -- full-chain walker (§2.1a ruling 2026-07-17) ----------------------

    /// Build: `def Base { require baseC }` · `def Derived :> Base { require derivedC }`
    /// · `requirement r : Derived { require ownC }`. Returns (graph, r_id).
    fn chain_graph() -> (ModelGraph, ElementId) {
        let mut graph = ModelGraph::new();

        let base = Element::new_with_kind(ElementKind::RequirementDefinition).with_name("Base");
        let base_id = graph.add_element(base);
        let base_c = Element::new_with_kind(ElementKind::RequirementConstraintMembership)
            .with_name("baseC")
            .with_owner(base_id.clone())
            .with_prop("role", "require")
            .with_prop("constraint", "1 < 2");
        graph.add_element(base_c);

        let derived =
            Element::new_with_kind(ElementKind::RequirementDefinition).with_name("Derived");
        let derived_id = graph.add_element(derived);
        let subcls = Element::new_with_kind(ElementKind::Subclassification)
            .with_owner(derived_id.clone())
            .with_prop("superclassifier", Value::Ref(base_id.clone()));
        graph.add_element(subcls);
        let derived_c = Element::new_with_kind(ElementKind::RequirementConstraintMembership)
            .with_name("derivedC")
            .with_owner(derived_id.clone())
            .with_prop("role", "require")
            .with_prop("constraint", "2 < 3");
        graph.add_element(derived_c);

        let usage = Element::new_with_kind(ElementKind::RequirementUsage).with_name("r");
        let usage_id = graph.add_element(usage);
        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(usage_id.clone())
            .with_prop("type", Value::Ref(derived_id.clone()));
        graph.add_element(typing);
        let own_c = Element::new_with_kind(ElementKind::RequirementConstraintMembership)
            .with_name("ownC")
            .with_owner(usage_id.clone())
            .with_prop("role", "require")
            .with_prop("constraint", "3 < 4");
        graph.add_element(own_c);

        (graph, usage_id)
    }

    #[test]
    fn inheritance_chain_walks_typing_then_specialization_transitively() {
        let (graph, usage_id) = chain_graph();
        let usage = graph.get_element(&usage_id).unwrap();
        let chain = requirement_inheritance_chain(usage, &graph);
        let names: Vec<(&str, RequirementChainHop)> = chain
            .iter()
            .map(|a| (a.element.name.as_deref().unwrap(), a.via))
            .collect();
        assert_eq!(
            names,
            vec![
                ("Derived", RequirementChainHop::Typing),
                ("Base", RequirementChainHop::Specialization),
            ],
            "nearest-first: typing target, then its specialization general"
        );
    }

    #[test]
    fn inheritance_chain_survives_cycles() {
        let mut graph = ModelGraph::new();
        let a = graph
            .add_element(Element::new_with_kind(ElementKind::RequirementDefinition).with_name("A"));
        let b = graph
            .add_element(Element::new_with_kind(ElementKind::RequirementDefinition).with_name("B"));
        graph.add_element(
            Element::new_with_kind(ElementKind::Subclassification)
                .with_owner(a.clone())
                .with_prop("superclassifier", Value::Ref(b.clone())),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::Subclassification)
                .with_owner(b.clone())
                .with_prop("superclassifier", Value::Ref(a.clone())),
        );
        let a_elem = graph.get_element(&a).unwrap();
        let chain = requirement_inheritance_chain(a_elem, &graph);
        assert_eq!(chain.len(), 1, "cycle must terminate, visiting B once");
        assert_eq!(chain[0].element.name.as_deref(), Some("B"));
    }

    /// The whole point of the §2.1a supersession: a usage that OWNS
    /// constraints STILL aggregates its chain's — the owns-none gate is gone.
    #[test]
    fn effective_constraints_are_unconditional_owned_plus_full_chain() {
        let (graph, usage_id) = chain_graph();
        let usage = graph.get_element(&usage_id).unwrap();
        let effective = effective_requirement_constraints(usage, &graph);
        let names: Vec<(&str, Option<&str>)> = effective
            .iter()
            .map(|c| {
                (
                    c.element.name.as_deref().unwrap(),
                    c.origin.and_then(|o| o.name.as_deref()),
                )
            })
            .collect();
        assert_eq!(
            names,
            vec![
                ("ownC", None),
                ("derivedC", Some("Derived")),
                ("baseC", Some("Base")),
            ],
            "owned first, then chain nearest-first — the usage owning \
             constraints must NOT suppress inheritance"
        );
        assert_eq!(effective[1].via, Some(RequirementChainHop::Typing));
        assert_eq!(effective[2].via, Some(RequirementChainHop::Specialization));
    }

    /// KerML `removeRedefinedFeatures`: a nearer member redefining a named
    /// inherited constraint EXCLUDES the inherited one — and only
    /// redefinition suppresses, never bare name shadowing.
    #[test]
    fn effective_constraints_apply_redefinition_suppression() {
        let (mut graph, usage_id) = chain_graph();
        // Give the usage a member redefining Base::baseC.
        let redefining = Element::new_with_kind(ElementKind::RequirementConstraintMembership)
            .with_name("baseC")
            .with_owner(usage_id.clone())
            .with_prop("role", "require")
            .with_prop("constraint", "9 < 10");
        let redefining_id = graph.add_element(redefining);
        graph.add_element(
            Element::new_with_kind(ElementKind::Redefinition)
                .with_owner(redefining_id.clone())
                .with_prop("unresolved_redefinedFeature", "Base::baseC"),
        );

        let usage = graph.get_element(&usage_id).unwrap();
        let effective = effective_requirement_constraints(usage, &graph);
        let names: Vec<&str> =
            effective.iter().map(|c| c.element.name.as_deref().unwrap()).collect();
        assert!(
            names.contains(&"derivedC") && names.contains(&"ownC"),
            "unrelated members survive: {names:?}"
        );
        let base_c_rows: Vec<_> = effective
            .iter()
            .filter(|c| c.element.name.as_deref() == Some("baseC"))
            .collect();
        assert_eq!(base_c_rows.len(), 1, "exactly one baseC — the redefining member");
        assert!(
            base_c_rows[0].origin.is_none(),
            "the surviving baseC is the usage's own redefining member, \
             the inherited one is suppressed"
        );
    }

    /// Name shadowing WITHOUT a redefinition clause does NOT suppress —
    /// KerML has no name-shadow semantics; both members stay effective.
    #[test]
    fn same_name_without_redefinition_does_not_suppress() {
        let (mut graph, usage_id) = chain_graph();
        let shadowing = Element::new_with_kind(ElementKind::RequirementConstraintMembership)
            .with_name("baseC")
            .with_owner(usage_id.clone())
            .with_prop("role", "require")
            .with_prop("constraint", "9 < 10");
        graph.add_element(shadowing);

        let usage = graph.get_element(&usage_id).unwrap();
        let effective = effective_requirement_constraints(usage, &graph);
        let base_c_count = effective
            .iter()
            .filter(|c| c.element.name.as_deref() == Some("baseC"))
            .count();
        assert_eq!(base_c_count, 2, "no redefinition clause → no suppression");
    }
}
