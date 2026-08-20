//! Layer 4 v2 — axis-aware spec-property conformance gate.
//!
//! Builds on Layer 4 v0 (`spec_kind_conformance.rs`, the
//! ElementKind ⊆ TTL vocab gate) and Layer 4 v1 (every TS-emitted
//! prop key reduces to a spec prop name) by checking, for every
//! shape-required spec prop on every observed kind, that TS
//! populates the spec contract on **the correct axis**.
//!
//! ## Why "axis"?
//!
//! The OSLC `ResourceShape` defines required props uniformly per kind
//! — e.g. `Specialization` requires `general` + `specific`. TS
//! encodes the same contract across **three storage axes**:
//!
//! * **Props axis** — `Element.props[<mapped name>]` (most semantic
//!   scalars: `declaredName`, `isAbstract`, etc.)
//! * **Relationship axis** — `Relationship.source` / `target` for
//!   the relationship-kind entries TS stores in `graph.relationships`
//!   (Specialization, Subsetting, FeatureTyping, Redefinition,
//!   Annotation, …). E.g. `general` ↔ `Relationship.target`,
//!   `specific` ↔ `Relationship.source`.
//! * **Ownership axis** — `Element.owning_membership` /
//!   `Element.owner` chains (e.g. `owningType`,
//!   `membershipOwningNamespace`). For Membership-kind entries,
//!   the *reverse* of this chain identifies the `memberElement`.
//!
//! Checking only the Props axis (as Layer 4 v1 did) yields a
//! structurally meaningless 3–5% coverage number because the spec's
//! required-prop set is dominated by reference props TS encodes
//! across axes 2 and 3. This gate consults a `SpecPropAxis` table to
//! ask the right question per prop, so 100% becomes a number that
//! actually means "TS conformance with the OSLC shape contract".
//!
//! ## Two gates
//!
//! **Gate 1 (strict, unchanged from v1)**: every TS prop key emitted
//! across the corpus, after mapping + TS-internal allowlist, must
//! appear in some shape's property list.
//!
//! **Gate 2 (advisory)**: per-corpus required-prop coverage with
//! the framework allowlist + default-elidable boolean allowlist
//! subtracted from the denominator, and per-axis satisfaction
//! evaluation. Printed only; not asserted. Pinning a floor here is
//! Phase 2c territory — first we get the metric honest, then we
//! pin the floor.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use sysml_codegen::shapes_parser::{
    parse_oslc_shapes, resolve_shared_properties, Cardinality, PropertyInfo, ShapeInfo,
};
use sysml_core::ElementKind;
use sysml_parser_incremental::{build_model_graph, TreeSitterParser};

// ---------------------------------------------------------------------------
// Paths (mirror spec_kind_conformance.rs)
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn references_root() -> PathBuf {
    workspace_root().join("references").join("sysmlv2")
}

fn pilot_examples_root() -> PathBuf {
    references_root()
        .join("SysML-v2-Pilot-Implementation")
        .join("sysml")
        .join("src")
        .join("examples")
}

fn kerml_examples_root() -> PathBuf {
    references_root()
        .join("SysML-v2-Pilot-Implementation")
        .join("kerml")
        .join("src")
        .join("examples")
}

fn local_examples_root() -> PathBuf {
    workspace_root().join("examples")
}

// ---------------------------------------------------------------------------
// Shape loading
// ---------------------------------------------------------------------------

fn load_shapes() -> HashMap<String, ShapeInfo> {
    let refs = references_root();
    let kerml_content =
        std::fs::read_to_string(refs.join("KerML-shapes.ttl")).expect("read KerML-shapes.ttl");
    let sysml_content =
        std::fs::read_to_string(refs.join("SysML-shapes.ttl")).expect("read SysML-shapes.ttl");

    let (mut kerml_shapes, kerml_shared) =
        parse_oslc_shapes(&kerml_content).expect("parse KerML shapes");
    let (mut sysml_shapes, sysml_shared) =
        parse_oslc_shapes(&sysml_content).expect("parse SysML shapes");

    resolve_shared_properties(&mut kerml_shapes, &kerml_shared);
    resolve_shared_properties(&mut sysml_shapes, &sysml_shared);

    let mut map: HashMap<String, ShapeInfo> = HashMap::new();
    // SysML wins on conflict because SysML kinds inherit + extend KerML.
    for shape in kerml_shapes {
        map.insert(shape.element_type.clone(), shape);
    }
    for shape in sysml_shapes {
        map.insert(shape.element_type.clone(), shape);
    }

    // OSLC TTL does not encode `oslc:extends` between shapes, but the
    // SysML/KerML metamodel hierarchy IS authoritative (via
    // `ElementKind::supertypes`). Walk every shape's supertype chain
    // and merge in required props that the descendant shape doesn't
    // already declare. This gives the conformance metric a complete
    // view of the contract for any given kind, mirroring what TS
    // implementations have to satisfy through inheritance.
    resolve_inherited_shape_properties(&mut map);

    map
}

/// Walk the metamodel supertype chain for each shape and merge in any
/// required spec properties declared by an ancestor shape that the
/// descendant doesn't already carry. This compensates for the absence
/// of `oslc:extends` in the OSLC TTL files: the shape inheritance
/// contract is implicit in the metamodel hierarchy via
/// `ElementKind::supertypes`.
///
/// Only props missing from the descendant (by name) are merged. We
/// keep all cardinalities intact so the downstream `is_required`
/// filter still drives the coverage denominator.
///
/// Empirically (as of this commit) this is a no-op on the current
/// SysML/KerML shape files: the OSLC TTL authors pre-flatten shape
/// inheritance, so every `oslc:ResourceShape` already redundantly
/// lists every required prop from its supertypes. The walk is kept
/// because:
///   1. It documents the inheritance contract in code (not just in
///      the authors' heads).
///   2. It is the right safety net for future OSLC TTL revisions
///      that may drop the redundant pre-flattening once `oslc:extends`
///      lands upstream.
fn resolve_inherited_shape_properties(shapes: &mut HashMap<String, ShapeInfo>) {
    // First, snapshot the supertype-prop contributions per shape so we
    // don't mutate while iterating. Each entry is `(descendant_kind,
    // ancestor_props)` — a list of `PropertyInfo` to merge into the
    // descendant.
    let mut to_merge: Vec<(String, Vec<PropertyInfo>)> = Vec::new();
    for (kind_name, shape) in shapes.iter() {
        let Some(kind) = ElementKind::from_str(kind_name) else {
            continue;
        };
        let existing_names: BTreeSet<String> =
            shape.properties.iter().map(|p| p.name.clone()).collect();
        let mut additions: Vec<PropertyInfo> = Vec::new();
        let mut added_names: BTreeSet<String> = BTreeSet::new();
        for sup in kind.supertypes() {
            let Some(sup_shape) = shapes.get(sup.as_str()) else {
                continue;
            };
            for prop in &sup_shape.properties {
                if existing_names.contains(&prop.name) || added_names.contains(&prop.name) {
                    continue;
                }
                additions.push(prop.clone());
                added_names.insert(prop.name.clone());
            }
        }
        if !additions.is_empty() {
            to_merge.push((kind_name.clone(), additions));
        }
    }
    for (kind_name, additions) in to_merge {
        if let Some(shape) = shapes.get_mut(&kind_name) {
            shape.properties.extend(additions);
        }
    }
}

fn is_required(p: &PropertyInfo) -> bool {
    matches!(
        p.cardinality,
        Cardinality::ExactlyOne | Cardinality::OneOrMany
    )
}

// ---------------------------------------------------------------------------
// TS-prop ↔ spec-prop mapping
// ---------------------------------------------------------------------------

/// Map a TS-emitted prop key onto a spec prop name. Returns the
/// canonical spec name. Order of resolution:
///
/// 1. `unresolved_<X>` markers strip the prefix — TS uses them to mean
///    "I saw a reference here but haven't resolved it to a canonical
///    path yet". Semantically identical to the underlying name.
/// 2. Exact aliases (TS shorthand vs spec canonical name).
/// 3. Returns the key unchanged otherwise.
fn map_ts_prop_to_spec(key: &str) -> &str {
    // Strip the unresolved_ marker first, then continue to the alias
    // lookup so unresolved_aliasTarget → aliasTarget → memberElement
    // resolves end-to-end.
    let key = key.strip_prefix("unresolved_").unwrap_or(key);
    match key {
        // TS emits inline bound expressions; shape expects a single
        // multiplicity reference to a MultiplicityRange element.
        "multiplicity_lower" | "multiplicity_upper" => "multiplicity",
        "multiplicity_lower_text" | "multiplicity_upper_text" => "multiplicity",
        // Expressions in calculations are inlined by TS; shape names the
        // containing relationship. (The former `guard`/`trigger`/`effect`
        // rows retired with task #81: transitions now own real
        // TransitionFeatureMembership-wrapped children instead of string
        // props, so those TS props no longer exist.)
        "expr" => "expression",
        // ConstraintUsage shorthand — shape uses ownedConstraint as
        // the containment name.
        "constraint" => "ownedConstraint",
        // RequirementConstraintMembership.role — shape uses
        // `kind` (RequirementConstraintKind enum). TS preserves
        // the source keyword (`assume`/`require`) under `role`.
        "role" => "kind",
        // Membership alias — TS calls the alias-pointed element
        // `aliasTarget`; spec calls it `memberElement`.
        "aliasTarget" => "memberElement",
        // Dependency lowers `dependency a, b to c, d` losslessly as LIST
        // props `unresolved_clients` / `unresolved_suppliers` (G27
        // cross-product contract) alongside the singular first-entry
        // markers. The spec's `client` / `supplier` are themselves
        // multi-valued (Dependency.client [1..*]), so the plural list is
        // the same spec property, not a TS invention.
        "clients" => "client",
        "suppliers" => "supplier",
        // SatisfyRequirementUsage shorthand — TS stores the satisfying
        // subject under `satisfiedBy`; spec names it `satisfyingFeature`.
        "satisfiedBy" => "satisfyingFeature",
        // MetadataUsage type reference. G16 emits the unresolved metadata
        // type name (the `@TypeRef` suffix) under `unresolvedTypeName`,
        // matching the Pest baseline and every runtime reader
        // (sysml-core::metadata, compiler.rs @DataSource/@ToolVariable
        // lookups). Note this is camelCase (no `unresolved_` underscore),
        // so it isn't handled by the strip_prefix path above. A
        // MetadataUsage is a MetadataFeature whose type must be a
        // Metaclass; the spec models that type reference as `metaclass`
        // (MetadataUsageShape / MetadataFeatureShape — SysML-shapes.ttl
        // oslc_sysml_shapes:metaclass: "The type of this MetadataFeature,
        // which must be a Metaclass"). Emitted only on MetadataUsage
        // (single set_prop site in ast_builder/dispatch.rs).
        "unresolvedTypeName" => "metaclass",
        // ElementFilterMembership filter expression. TS stores the filter
        // text under `filterExpression`; the spec models it as `condition`
        // (ElementFilterMembershipShape — "The condition Expression that
        // must be satisfied"). Surfaced for the first time by the Arc 1
        // nested-package grammar fix: filter declarations inside nested
        // library packages previously fell into the broken
        // package-in-package recovery path and never lowered.
        "filterExpression" => "condition",
        other => other,
    }
}

/// TS-internal prop keys that have no spec equivalent and are not
/// expected to appear in any shape. Justification per key:
///
/// - `argIndex`: TS-internal positional sort key for the operands of
///   `InvocationExpression`/`OperatorExpression`/`FeatureReferenceExpression`.
///   Pilot recovers operand order from the parent's `argument` list;
///   TS embeds it on the child for free-standing diffs.
/// - `annotationType`: TS shorthand for the `MetadataUsage` →
///   metaclass relationship; the spec resolves this through the
///   element's owned typing, not via a property.
/// - `importedReference`: TS attaches the source-text reference of an
///   `import` statement directly; the spec resolves via the
///   `importedNamespace` / `importedMemberName` references.
/// - `isNamespace`: TS marker for `import ... :: *` vs membership
///   import; the spec distinguishes by kind (`NamespaceImport` vs
///   `MembershipImport`), so the marker is redundant for the
///   element-kind axis but kept for fast TS-side dispatch.
/// - `isBodyParameter`: TS-internal flag for action-body-parameter
///   features (distinguishes a body's bound parameter from a regular
///   feature). Spec recovers via owning ParameterMembership.
/// - `isMessage`: TS-internal `FlowUsage` marker for `message` flows;
///   the spec recovers via the owning relationship kind.
/// - `satisfiedBy`: TS shorthand for `SatisfyRequirementUsage` target
///   reference; the spec routes this through the
///   `satisfyingFeature` / requirement parameter binding.
/// - `stateSubactionKind`: TS-side `entry`/`do`/`exit` marker for
///   ActionUsage; the spec uses `StateSubactionMembership.kind`.
// ---------------------------------------------------------------------------
// Per-corpus required-prop coverage floors.
//
// Pinned a few points below the observed value to allow natural corpus
// fluctuation but catch regressions. Bump these up when a future pass
// closes axis_misses (real TS gaps or new classifier entries).
// ---------------------------------------------------------------------------

// Coverage after the TS-2.14 ast_builder pass (2026-05-26):
//   Pilot 100.0% (285/285)  — TS-2.14 closed the remaining 6 axis-misses
//     surfaced by the trust-point audit (loopVariable, seqArgument,
//     ifArgument, whileArgument, payloadParameter, payloadArgument) by
//     emitting child elements and routing the spec slots through Derived
//     classification, matching the bodyAction/thenAction precedent.
//   KerML 100.0% (126/126)  — unchanged; no Pilot-only constructs.
//   local 100.0% (168/168)  — unchanged.
// Floors pinned at observed - 1pp cushion to catch regressions
// without false-positiving on corpus fluctuation.
const PILOT_COVERAGE_FLOOR_PCT: f64 = 99.0;
const KERML_COVERAGE_FLOOR_PCT: f64 = 99.0;
const LOCAL_COVERAGE_FLOOR_PCT: f64 = 99.0;

const TS_INTERNAL_ALLOWLIST: &[&str] = &[
    "argIndex",
    // `argName`: the named-argument feature name on a `new T(field = value)`
    // constructor argument (A-structured). The spec binds the argument to its
    // parameter via a ParameterRedefinition (KerMLExpressions.xtext:470-485);
    // TS stamps the name on the projected value element so the runtime can key
    // the payload `Value::Map` by feature name — a TS-internal sibling of
    // `argIndex`, no model-property home.
    "argName",
    "annotationType",
    // `isPrefixMetadata`: marker for the `#Name` prefix-annotation form on
    // the minted MetadataUsage (single set_prop site, ast_builder/usages.rs).
    // The spec distinguishes prefix from body metadata structurally (the
    // owning prefixMetadata relationship), not via a boolean on the
    // MetadataUsage shape — a TS-internal sibling of `annotationType`.
    "isPrefixMetadata",
    // `via_port`: TS-internal routing encoding for port messages
    // (`send <payload> via <port>`). The spec models the `via` target as an
    // expression bound to the trigger (TransitionPerformances) — not a model
    // PROPERTY. The runtime reads it to lower the port-message wire (ledger
    // L26). (`accept_param` retired with task #81: the accept parameter is
    // now a real ReferenceUsage payload child of the trigger
    // AcceptActionUsage — the spec payloadParameter shape.)
    "via_port",
    "importedReference",
    "isNamespace",
    "isBodyParameter",
    "isMessage",
    // `isVerify` retired 2026-07-16: `verify requirement …` now lowers to a
    // RequirementVerificationMembership owning a RequirementUsage (the spec
    // kinds), so no marker prop exists.
    "stateSubactionKind",
    // `unit`: the SysML measurement notation `num [unit]` (e.g. `5 [kg]`) lowers
    // the numeric literal with a `unit` prop carrying the unit token (commit
    // `13b6886f`, D-5.0.5). The KerML metamodel models units via a
    // MeasurementReference / the owning attribute's quantity type — NOT as a
    // property on the literal/attribute element — so `unit` has no model-property
    // home on any shape. TS stamps it inline for ergonomic runtime/quantity
    // access; a TS-internal sibling of `argIndex`/`via_port`.
    "unit",
    // `<role>NameStart` / `<role>NameEnd`: byte-offset companions for
    // reference-site name nodes (transition source/target, assignment
    // target, verification subject), stamped by the ast_builder so the
    // semantic-token emitter can colour the reference by its RESOLVED
    // target's kind (token arc Phase B.2, commit 863545e5; `Value` has no
    // Span variant, so start/end ride as int props). Pure source-geometry —
    // no spec shape models byte offsets. One family entry per role below.
    "sourceNameStart",
    "sourceNameEnd",
    "targetNameStart",
    "targetNameEnd",
    "subjectNameStart",
    "subjectNameEnd",
];

/// Per-kind TS-internal allowlist. Each entry is a `(ts_prop, kind)` pair
/// where the prop is TS-internal *only* on that kind (it would be a true
/// spec prop on some other shape). These are encoding choices the TS
/// ast_builder makes that the spec doesn't mirror on the inner element —
/// usually because the spec models the value on a wrapping membership
/// (FeatureValue / VariantMembership) but TS short-circuits by writing
/// the value onto the inner Usage for ergonomic downstream access.
///
/// Closed clusters G10 (isDefault on Usage subtypes), G11 (role/constraint
/// on Invariant), G13 (general/specific on FeatureValue) and a tail of
/// shorthand cases (value/multiplicity/expr/target/targetFeature/expression
/// /isConjugated/portionKind/isIndividual/constraint shorthand on usages
/// that lack the spec slot on their own shape) without touching the
/// parser. Pair-keyed so each entry justifies itself per kind, and so
/// adding a new kind keeps the surface tight (no global drift).
///
/// Format: `("<ts_prop>", "<kind>")` — both strings literal, sorted by
/// kind then prop for searchability.
const TS_INTERNAL_PER_KIND_ALLOWLIST: &[(&str, &str)] = &[
    // G11: role/constraint emitted by process_usage for ConstraintUsage
    // codepaths; Invariant's own shape covers neither (`role` maps to
    // `kind`, `constraint` maps to `ownedConstraint`, neither on
    // InvariantShape).
    ("role", "Invariant"),
    ("constraint", "Invariant"),
    // Shorthand: ConstraintUsage / AssertConstraintUsage embed the
    // result-expression text inline as `constraint`. The spec hangs the
    // expression off a child ResultExpressionMembership, not a prop on
    // the usage itself, so the mapped name `ownedConstraint` is absent
    // from the inner shape.
    ("constraint", "ConstraintUsage"),
    ("constraint", "AssertConstraintUsage"),
    // Shorthand: CalculationUsage stores the result-expression text
    // inline as `expr` (mapped to `expression`). Spec routes this via
    // ResultExpressionMembership child.
    ("expr", "CalculationUsage"),
    // Shorthand: ResultExpressionMembership carries the inline
    // expression text as `expression`; spec routes the expression via
    // the membership's `target` (reverse member element).
    ("expression", "ResultExpressionMembership"),
    // G10: `isDefault` lives on the wrapping FeatureValue /
    // VariantMembership per spec. TS sets it on the inner Usage as a
    // shortcut for compiler/elaboration consumers.
    ("isDefault", "ActionUsage"),
    ("isDefault", "ActorMembership"),
    ("isDefault", "AttributeUsage"),
    ("isDefault", "EventOccurrenceUsage"),
    ("isDefault", "ItemUsage"),
    ("isDefault", "OccurrenceUsage"),
    ("isDefault", "PartUsage"),
    ("isDefault", "PortUsage"),
    ("isDefault", "ReferenceUsage"),
    ("isDefault", "ReturnParameterMembership"),
    // G10 (TS-3.7b extension): the same FeatureValue/VariantMembership
    // `isDefault` shortcut now reaches more kinds after the G15/G17/G19
    // grammar passes widened where `=`/`default`/`:=` value bindings and
    // succession/transition lowering attach a FeatureValue. Spec still
    // models `isDefault` on the wrapping FeatureValue/VariantMembership,
    // not on these inner elements' own shapes.
    ("isDefault", "AssertConstraintUsage"),
    ("isDefault", "AttributeDefinition"),
    ("isDefault", "BindingConnectorAsUsage"),
    ("isDefault", "EnumerationDefinition"),
    ("isDefault", "RequirementUsage"),
    ("isDefault", "SuccessionAsUsage"),
    ("isDefault", "TransitionUsage"),
    // G10-analogue (TS-3.7b): `isInitial` is likewise a FeatureValue
    // property ("whether this FeatureValue specifies a bound value or an
    // initial value", SysML-shapes.ttl oslc_sysml_shapes:isInitial). TS
    // writes the initial-value marker onto the inner ActionUsage as a
    // shortcut for the elaborator; ActionUsageShape does not list it.
    ("isInitial", "ActionUsage"),
    // Same `isInitial` FeatureValue shortcut on the inner element where a
    // `:=` initial-value binding attaches to attribute/reference usages
    // (the G15/G17/G19 widening — same lineage as the isDefault block
    // above). Spec models isInitial on the wrapping FeatureValue.
    ("isInitial", "AttributeUsage"),
    ("isInitial", "ReferenceUsage"),
    // G13: TS encodes the bind operator as a FeatureValue with two
    // back-pointers (`specific` → owning Feature, `general` → operand
    // expression). Per OSLC vocab, FeatureValue subclasses
    // OwningMembership (not Specialization), so `general` / `specific`
    // are not on its shape. The bind result reaches the spec via the
    // FeatureValue → Expression `target` link; the back-pointers are
    // TS-internal for elaboration's convenience.
    ("specific", "FeatureValue"),
    ("general", "FeatureValue"),
    // Shorthand: AttributeUsage / ReferenceUsage / AssignmentActionUsage
    // store the literal value inline as `value` (TS-1.4 elaboration
    // path). The spec models the literal via FeatureValue.value (an
    // Expression child), so the inner Usage shape doesn't list `value`.
    ("value", "AttributeUsage"),
    ("value", "ReferenceUsage"),
    ("value", "AssignmentActionUsage"),
    // Same `value` shortcut on more kinds after the TS-3.7b grammar
    // passes (G15/G17/G19) widened inline value-binding lowering. The
    // spec models the literal via FeatureValue.value (an Expression
    // child); these inner shapes don't list `value`.
    ("value", "ActionUsage"),
    ("value", "EnumerationDefinition"),
    ("value", "SuccessionAsUsage"),
    // Task #85: the standalone `enum <name> = <literal>;` usage form (grammar
    // 884e7a61 enum_usage) rides the same inline-literal shortcut; the spec
    // models the literal via FeatureValue.value, so EnumerationUsage's own
    // shape doesn't list `value`.
    ("value", "EnumerationUsage"),
    // Shorthand: ActorMembership inlines multiplicity from the wrapping
    // ActorMember in grammar; spec puts the multiplicity on the
    // contained ReferenceUsage / wrapping Feature, not on the
    // ActorMembership.
    ("multiplicity", "ActorMembership"),
    ("multiplicity_lower", "ActorMembership"),
    ("multiplicity_upper", "ActorMembership"),
    // Shorthand: AssignmentActionUsage uses `target` / `targetFeature`
    // to record the LHS of `:=`. Spec routes the LHS through the
    // ParameterMembership/referent chain; the inline props are
    // TS-internal for the elaborator.
    ("target", "AssignmentActionUsage"),
    ("targetFeature", "AssignmentActionUsage"),
    // Shorthand: ConjugatedPortTyping carries the conjugation flag
    // inline as `isConjugated`; the spec models the conjugation through
    // a FeatureTyping subtype, not as a prop on the typing element.
    ("isConjugated", "ConjugatedPortTyping"),
    // Shorthand: ReferenceUsage may carry a `portionKind` /
    // `isIndividual` marker when the source-text uses an `individual`
    // / portion keyword on an embedded reference. Spec puts the marker
    // on the contained item usage / occurrence definition, not on the
    // wrapping ReferenceUsage.
    ("portionKind", "ReferenceUsage"),
    ("isIndividual", "ReferenceUsage"),
    // Shorthand: the verification subject-binding (`subject s : T = occ`, commit
    // `39b5e889`) stamps the subject's default value as `unresolved_value` on the
    // SubjectMembership for the runtime to resolve at verify time (gap-A). It
    // strips to `value`, which the spec models on the referenced subject Feature
    // / a FeatureValue — not on the SubjectMembership itself — so it has no slot
    // on the membership's own shape (same pattern as the `isDefault`-on-membership
    // entries above). (Separately, the raw runtime read of this prop trips the
    // `no_legacy_string_readers` guard — a pre-existing 39b5e889 concern, not
    // resolved here.)
    ("value", "SubjectMembership"),
    // Shorthand (task #81): a transition's trigger/guard/effect children
    // (AcceptActionUsage / Expression / ActionUsage wrapped in a
    // TransitionFeatureMembership) carry their textual form inline as `text`
    // — the canonical trigger string / guard expression text / raw effect
    // clause the runtime string-compiles (same pattern as `constraint` /
    // `expr` above). The spec models the equivalents structurally (payload
    // parameter + trigger expression / owned expression tree / owned action
    // body), which TS does not lower further yet — documented residual.
    ("text", "AcceptActionUsage"),
    ("text", "ActionUsage"),
    ("text", "Expression"),
];

/// Check whether `(prop, kind)` is in the per-kind TS-internal
/// allowlist. Linear scan keeps the list searchable by humans (sorted
/// by kind/prop) and avoids the cost of building a `HashSet` for a
/// table that stays small.
fn is_ts_internal_per_kind(prop: &str, kind: &str) -> bool {
    TS_INTERNAL_PER_KIND_ALLOWLIST
        .iter()
        .any(|(p, k)| *p == prop && *k == kind)
}

/// Kind-conditional version of `map_ts_prop_to_spec`. Returns a mapping
/// that depends on the owning element kind. Falls back to the
/// kind-agnostic mapping when no kind-conditional override applies.
///
/// Currently handles G12 (`unresolved_importedNamespace` on
/// MembershipImport / MembershipExpose → `importedMembership`,
/// matching the spec's MembershipImportShape).
fn map_ts_prop_to_spec_for_kind<'a>(key: &'a str, kind: &str) -> &'a str {
    // Strip unresolved_ prefix first so we can match on the bare name.
    let stripped = key.strip_prefix("unresolved_").unwrap_or(key);
    if stripped == "importedNamespace" && (kind == "MembershipImport" || kind == "MembershipExpose")
    {
        return "importedMembership";
    }
    map_ts_prop_to_spec(key)
}

/// OSLC framework / resource-identity properties required on every one
/// of the 257 shapes via shared property inheritance, but not modeled
/// by TS as per-element props. Spec consumers resolve these via OSLC
/// resource identity (`elementId`), dcterms metadata (`identifier`,
/// `title`), or library/implied-include indicators — not via
/// `Element.props`. Subtracted from the required-prop coverage
/// denominator so the metric measures *semantic* coverage.
const FRAMEWORK_ALLOWLIST: &[&str] = &[
    "elementId",
    "identifier",
    "title",
    "isImpliedIncluded",
    "isLibraryElement",
];

/// Boolean properties marked `oslc:occurs Exactly-one` that TS
/// legitimately elides when they hold their `false` default. The
/// shape's "required" semantics here mean "must be settable", not
/// "must always be present in serialised form" — TS encodes absence
/// as `false` for these via typed accessors. Subtracted from the
/// required-prop coverage denominator.
///
/// Sourced from the OSLC shape catalogue (Phase 1) by intersecting
/// "Exactly-one props with no explicit `oslc:range`" (i.e. inferred
/// booleans) against the spec-conventional `is*` / `may*` /
/// `has*` naming prefix.
const DEFAULT_ELIDABLE_BOOLEAN_ALLOWLIST: &[&str] = &[
    "isAbstract",
    "isComposite",
    "isConjugated",
    "isConstant",
    "isDerived",
    "isEnd",
    "isImplied",
    "isIndividual",
    "isModelLevelEvaluable",
    "isNegated",
    "isOrdered",
    "isParallel",
    "isPortion",
    "isReference",
    "isSufficient",
    "isUnique",
    "isVariable",
    "isVariation",
    "mayTimeVary",
    // FeatureValue / Membership default-elidable.
    "isDefault",
    "isInitial",
    // Import: TS only emits isImportAll=true for wildcard form (`import * from N`);
    // absence encodes `false`. Same pattern for isStandard on LibraryPackage
    // (only the ~10 stdlib packages get the `library` keyword).
    "isImportAll",
    "isRecursive",
    "isStandard",
];

// ---------------------------------------------------------------------------
// Spec-prop axis classification
// ---------------------------------------------------------------------------

/// Where TS stores the value satisfying a given spec required prop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // RelSource/RelTarget/OwningMembership reserved
                    // for v3 when TS migrates relationship-kind
                    // entries into graph.relationships.
enum SpecPropAxis {
    /// TS stores it in `Element.props[<mapped name>]`.
    Prop,
    /// TS stores it as the source of a relationship of this kind
    /// (i.e. for relationship-kind entries, `Relationship.source`).
    RelSource,
    /// Same but `Relationship.target`.
    RelTarget,
    /// TS stores it on `Element.owner` (cached owner pointer).
    Owner,
    /// TS stores it on `Element.owning_membership`.
    OwningMembership,
    /// Reverse lookup: this Membership-kind element's `memberElement`
    /// is whichever element points back via its `owning_membership`.
    /// Counts as satisfied if any element in the corpus targets this
    /// membership kind via `owning_membership`.
    ReverseMemberElement,
    /// Structurally satisfied by virtue of the element existing
    /// (e.g. `featureTarget` on a Feature defaults to self when the
    /// feature has no chaining list — always reachable through the
    /// generated typed accessor).
    Derived,
    /// Not yet classified. Treated as unsatisfied so it appears in the
    /// triage line. Add to `spec_prop_axis` to claim it.
    Unclassified,
}

/// Helper: does `kind_name` correspond to an `ElementKind` that is a
/// subtype of (or equal to) `supertype`? Falls back to false if either
/// name is not a recognized `ElementKind`.
fn kind_is_subtype_of(kind_name: &str, supertype: &str) -> bool {
    let Some(kind) = ElementKind::from_str(kind_name) else {
        return false;
    };
    let Some(sup) = ElementKind::from_str(supertype) else {
        return false;
    };
    kind == sup || kind.is_subtype_of(sup)
}

/// Classify a spec required-prop name by axis + ts-name aliases.
///
/// Returns `(axis, ts_names_to_probe)`. For `Prop` axis the test
/// checks whether any of `ts_names_to_probe` appears (after
/// `map_ts_prop_to_spec` mapping) in the kind's `props`. For non-Prop
/// axes the test consults the per-entry counters on `KindInventory`.
///
/// The classification is kind-aware because the OSLC shape inherits
/// `general`/`specific` from `Specialization` onto subkinds
/// (`FeatureTyping`, `Subsetting`, …) where TS uses subkind-specific
/// names (`type`, `subsettedFeature`, …). Likewise membership-family
/// kinds split into "wrapper" memberships (`OwningMembership`,
/// `FeatureMembership`) where a separate wrapped element reverses
/// `owning_membership` back, vs. "collapsed" memberships
/// (`ActorMembership`, `StakeholderMembership`) where the membership
/// element IS the member.
fn spec_prop_axis(kind: &str, spec_prop: &str) -> (SpecPropAxis, Vec<&'static str>) {
    use SpecPropAxis::*;

    // ----- Specialization family ----------------------------------------
    // FeatureTyping / Subsetting / Redefinition / CrossSubsetting /
    // ReferenceSubsetting / ConjugatedPortTyping all inherit
    // `general` + `specific` (and `source`/`target` from Relationship)
    // from `Specialization`, then add their own kind-specific endpoint
    // names. TS emits the kind-specific source-side name verbatim
    // (`typedFeature`, `subsettingFeature`, …) and the target-side
    // name as `unresolved_<X>`. All names within a role are synonyms;
    // any prop on either role-set satisfies any other within the set.
    if kind_is_subtype_of(kind, "Specialization") {
        const SOURCE_SIDE: &[&str] = &[
            "specific",
            "source",
            "typedFeature",
            "subsettingFeature",
            "redefiningFeature",
            "crossingFeature",
            "referencingFeature",
            "subclassifier",
        ];
        const TARGET_SIDE: &[&str] = &[
            "general",
            "target",
            "type",
            "subsettedFeature",
            "redefinedFeature",
            "crossedFeature",
            "referencedFeature",
            "superclassifier",
        ];
        if SOURCE_SIDE.contains(&spec_prop) {
            return (Prop, SOURCE_SIDE.to_vec());
        }
        if TARGET_SIDE.contains(&spec_prop) {
            return (Prop, TARGET_SIDE.to_vec());
        }
    }

    // ----- Annotation relationship --------------------------------------
    // The `Annotation` element wraps an annotating element +
    // annotated element. TS emits `annotatingElement` verbatim and
    // `unresolved_annotatedElement` for the target.
    if kind == "Annotation" {
        match spec_prop {
            "annotatedElement" | "target" => return (Prop, vec!["annotatedElement", "target"]),
            "annotatingElement" | "source" => return (Prop, vec!["annotatingElement", "source"]),
            _ => {}
        }
    }

    // ----- KerML type-relationship operators (§7.3) ---------------------
    // Unioning / Intersecting / Differencing / Disjoining / TypeFeaturing /
    // FeatureInverting each store the source Type/Feature as a Ref under the
    // vocab source role (`typeUnioned`, …) and the target as
    // `unresolved_<targetRole>` (→ `<targetRole>` after resolution). Both
    // ground via the Prop axis — the target-side check accepts the
    // `unresolved_` shorthand (stripped by the Prop probe). Source/target
    // (inherited from Relationship) are synonyms within each role-set.
    if matches!(
        kind,
        "Unioning" | "Intersecting" | "Differencing" | "Disjoining" | "TypeFeaturing"
            | "FeatureInverting" | "Conjugation"
    ) {
        const SOURCE_SIDE: &[&str] = &[
            "source",
            "typeUnioned",
            "typeIntersected",
            "typeDifferenced",
            "typeDisjoined",
            "featureOfType",
            "featureInverted",
            // Conjugation: the declaring type is the conjugatedType (source-side,
            // set as a resolved Ref); the ConjugationPart target is originalType.
            "conjugatedType",
        ];
        const TARGET_SIDE: &[&str] = &[
            "target",
            "unioningType",
            "intersectingType",
            "differencingType",
            "disjoiningType",
            "featuringType",
            "invertingFeature",
            "originalType",
        ];
        if SOURCE_SIDE.contains(&spec_prop) {
            return (Prop, SOURCE_SIDE.to_vec());
        }
        if TARGET_SIDE.contains(&spec_prop) {
            return (Prop, TARGET_SIDE.to_vec());
        }
    }
    // ----- Dependency ----------------------------------------------------
    // TS lowers `dependency a, b to c, d` with `unresolved_client(s)` /
    // `unresolved_supplier(s)` props (G27 lossless cross-product contract);
    // both the singular and the plural list map onto the spec's
    // multi-valued `client` / `supplier` via map_ts_prop_to_spec.
    if kind == "Dependency" {
        match spec_prop {
            "client" => return (Prop, vec!["client"]),
            "supplier" => return (Prop, vec!["supplier"]),
            _ => {}
        }
    }

    // AnnotatingElement subtypes (Comment, Documentation,
    // MetadataUsage, TextualRepresentation) hold an `annotatedElement`
    // reference that TS encodes either as a prop on themselves or via
    // the Annotation wrapper. Treat as Derived since the typed
    // accessor walks owned Annotations.
    if kind_is_subtype_of(kind, "AnnotatingElement") {
        match spec_prop {
            "annotatedElement" | "annotatingElement" | "documentedElement" => {
                return (Derived, vec![])
            }
            _ => {}
        }
    }

    // TextualRepresentation (Kerml-Vocab.ttl: an AnnotatingElement whose `body`
    // represents the representedElement in a named `language`; the
    // representedElement "must be the owner"). The TS lowering
    // (ast_builder::imports::process_textual_representation) stamps `language`
    // as a real prop and owns the rep under the element it represents, so:
    //   - `language`          -> a stamped Prop
    //   - `representedElement` -> Element.owner (the owner IS the represented element)
    // (`body` grounds through the generic Prop handler below.)
    if kind == "TextualRepresentation" {
        match spec_prop {
            "language" => return (Prop, vec!["language"]),
            "representedElement" => return (Owner, vec![]),
            _ => {}
        }
    }

    // ----- Membership family --------------------------------------------
    // OwningMembership / FeatureMembership / ParameterMembership /
    // ReturnParameterMembership / RequirementConstraintMembership /
    // FramedConcernMembership / RequirementVerificationMembership /
    // ResultExpressionMembership / ElementFilterMembership /
    // EndFeatureMembership wrap a separate member element via
    // `add_owned_element`, so reverse-lookup works.
    //
    // ActorMembership / StakeholderMembership / SubjectMembership are
    // collapsed: the membership IS the member element (process_usage
    // creates one element with the membership kind). For these we
    // treat memberElement as Derived (structurally self-referential).
    // "Collapsed" memberships: TS mints these as a single element of the
    // membership kind itself (no separate wrapped child). The membership
    // IS the member, so memberElement / ownedMember* are structurally
    // self-referential, and any "typed view" of the memberElement (e.g.
    // ownedSubjectParameter on SubjectMembership) is Derived from the
    // same self-ref.
    //
    // When the collapsed membership is added via `add_with_ownership_keyed`,
    // an outer wrapping OwningMembership carries `visibility`. We accept
    // that as Derived too rather than requiring the prop to be mirrored
    // onto every collapsed-membership element.
    //
    // Note `Membership` (the base kind) is collapsed too — it's used for
    // alias declarations whose target is stored under `unresolved_aliasTarget`
    // and mapped to `memberElement` via `map_ts_prop_to_spec`.
    let is_collapsed_membership = matches!(
        kind,
        "Membership"
            | "ActorMembership"
            | "StakeholderMembership"
            | "SubjectMembership"
            | "ObjectiveMembership"
            // VariantMembership is NOT collapsed: the enum-member lowering
            // mints it as a wrapper (a distinct EnumerationUsage member owned
            // through a separate VariantMembership element, exactly like
            // OwningMembership), so `memberElement`/`ownedVariantUsage` are
            // reachable via the reverse `owning_membership` index and
            // `membershipOwningNamespace` is set as a Prop (below).
            //
            // StateSubactionMembership is likewise NOT collapsed: the state
            // entry/do/exit lowering mints it as a wrapper (a distinct
            // ActionUsage member owned through a separate StateSubactionMembership
            // element carrying `kind`), so `memberElement`/`ownedMemberFeature`/
            // `action` reach via the reverse `owning_membership` index and
            // `membershipOwningNamespace`/`owningType` ground as Props (below).
            //
            // TransitionFeatureMembership is likewise NOT collapsed (task #81,
            // completing the F3 407db661 pattern): a transition's trigger/
            // guard/effect are real AcceptActionUsage/Expression/ActionUsage
            // members owned through a separate TransitionFeatureMembership
            // element carrying `kind` (SysML v2 §8.3.18.8), so `memberElement`/
            // `ownedMemberFeature`/`transitionFeature` reach via the reverse
            // `owning_membership` index and `membershipOwningNamespace`/
            // `owningType` ground as Props (below).
            | "MembershipExpose"
            | "MembershipImport"
            | "VariantUsageMembership"
            | "ViewRenderingMembership"
            | "RequirementConstraintMembership"
            | "RequirementVerificationMembership"
            | "FramedConcernMembership"
            | "ResultExpressionMembership"
            | "ReturnParameterMembership"
            | "FeatureValue"
    );
    if kind_is_subtype_of(kind, "Membership") {
        // Typed views of memberElement that derive from the collapsed
        // membership being the member itself.
        if is_collapsed_membership {
            match spec_prop {
                "ownedSubjectParameter"
                | "ownedActorParameter"
                | "ownedStakeholderParameter"
                | "ownedRequirement"
                | "ownedObjectiveRequirement"
                | "ownedRendering"
                | "ownedResultExpression"
                | "ownedConstraint"
                | "referencedConstraint"
                // FramedConcernMembership's ConcernUsage-typed views of the
                // same two members (SysML-vocab.ttl): `ownedConcern` IS "the
                // ConcernUsage that is the ownedConstraint of this
                // FramedConcernMembership"; `referencedConcern` IS "the
                // referencedConstraint … considered as a … ConcernUsage".
                // Same physical members as ownedConstraint/referencedConstraint
                // above, so they ground the same way (derived typed accessors).
                | "ownedConcern"
                | "referencedConcern"
                | "referencedRendering"
                | "visibility" => return (Derived, vec![]),
                _ => {}
            }
        }
        match spec_prop {
            "memberElement"
            | "ownedMemberElement"
            | "ownedMemberFeature"
            | "ownedMemberParameter"
            // VariantMembership.ownedVariantUsage (§8.3.6.5) is the typed
            // view of the membership's owned member element — the enumerated
            // value it wraps — so it grounds through the same reverse-member
            // index as the generic ownedMemberElement.
            | "ownedVariantUsage"
            // StateSubactionMembership.action (SysML-vocab.ttl): the ActionUsage
            // that IS the ownedMemberFeature of this membership — the same
            // reverse-member view as ownedMemberFeature.
            | "action"
            // TransitionFeatureMembership.transitionFeature (SysML-vocab.ttl:
            // "The Step that is the ownedMemberFeature of this
            // TransitionFeatureMembership"; §8.3.18.8 `/transitionFeature
            // {redefines ownedMemberFeature}`) — same reverse-member view.
            | "transitionFeature" => {
                if is_collapsed_membership {
                    return (Derived, vec![]);
                } else {
                    return (ReverseMemberElement, vec![]);
                }
            }
            // Only the wrapper kind `OwningMembership` itself gets
            // `membershipOwningNamespace` set as a Prop (via
            // MembershipBuilder). Membership SUBTYPES (Actor/Subject/
            // Result*/Return*/etc.) are minted as collapsed elements
            // whose `Element.owner` is the namespace, so the Owner axis
            // is the right check for them.
            // StateSubactionMembership / TransitionFeatureMembership are
            // FeatureMemberships minted wrapper-style (namespace on the
            // `membershipOwningNamespace` Prop, Element.owner left unset), so
            // their FeatureMembership-inherited `owningType`/`owningNamespace`
            // typed views of that same namespace ground through the Prop
            // rather than the (unset) Owner axis.
            // FeatureMembership / EndFeatureMembership / ParameterMembership
            // joined the wrapper family with the task #85 ends-nesting slice
            // (e3ee604c): a flow/message's end and payload children are owned
            // through these memberships minted by the same MembershipBuilder
            // path (SysML.xtext FlowEndMember:1309 / FlowFeatureMember:1330 /
            // PayloadFeatureMember:1289 / MessageEventMember:1254).
            "owningType" | "owningNamespace"
                if kind == "StateSubactionMembership"
                    || kind == "TransitionFeatureMembership"
                    || kind == "FeatureMembership"
                    || kind == "EndFeatureMembership"
                    || kind == "ParameterMembership" =>
            {
                return (Prop, vec!["membershipOwningNamespace"]);
            }
            "membershipOwningNamespace" => {
                // OwningMembership, VariantMembership, StateSubactionMembership
                // and TransitionFeatureMembership are minted as wrapper
                // memberships via `MembershipBuilder`, which sets
                // `membershipOwningNamespace` directly as a Prop (owner is
                // left unset on the membership itself). FeatureMembership /
                // EndFeatureMembership / ParameterMembership joined with the
                // task #85 ends-nesting slice (same builder path). The
                // collapsed subtypes below instead carry their namespace on
                // `Element.owner`.
                if kind == "OwningMembership"
                    || kind == "VariantMembership"
                    || kind == "StateSubactionMembership"
                    || kind == "TransitionFeatureMembership"
                    || kind == "FeatureMembership"
                    || kind == "EndFeatureMembership"
                    || kind == "ParameterMembership"
                {
                    return (Prop, vec!["membershipOwningNamespace"]);
                }
                return (Owner, vec![]);
            }
            "owningRelatedElement" => return (Owner, vec![]),
            "visibility" => return (Prop, vec!["visibility"]),
            "memberName" | "memberShortName" => return (Prop, vec![spec_prop_static(spec_prop)]),
            // ElementFilterMembership: TS stores the filter text as
            // `filterExpression`, mapped to the spec's `condition` by
            // map_ts_prop_to_spec.
            "condition" if kind == "ElementFilterMembership" => {
                return (Prop, vec!["condition"])
            }
            _ => {}
        }
    }

    // ----- Import family ------------------------------------------------
    if kind_is_subtype_of(kind, "Import") || kind == "Expose" || kind_is_subtype_of(kind, "Expose")
    {
        // TS uses a single shorthand `unresolved_importedNamespace` for
        // the import target across all Import variants (the spec splits
        // it into importedElement / importedMembership / importedNamespace
        // by import form). The shorthand maps to `importedNamespace`
        // after `unresolved_` stripping, so accept it as an alias for the
        // sibling spec prop names too.
        const IMPORT_TARGET_ALIASES: &[&str] =
            &["importedElement", "importedMembership", "importedNamespace"];
        match spec_prop {
            "importedElement" | "importedMembership" | "importedNamespace" => {
                return (Prop, IMPORT_TARGET_ALIASES.to_vec());
            }
            "importOwningNamespace" => return (Owner, vec![]),
            "isImportAll" | "isRecursive" => return (Prop, vec![spec_prop_static(spec_prop)]),
            // Import elements are added via `add_with_ownership_keyed`,
            // so the wrapping OwningMembership carries the visibility
            // attribute. Accept as Derived rather than requiring TS to
            // mirror the prop onto every Import element.
            "visibility" => return (Derived, vec![]),
            _ => {}
        }
    }

    // ----- Feature ownership chain (any Feature subtype) ----------------
    if kind_is_subtype_of(kind, "Feature") {
        if spec_prop == "owningType" || spec_prop == "owningNamespace" {
            return (Owner, vec![]);
        }
    }

    // ----- Expression family --------------------------------------------
    if kind_is_subtype_of(kind, "Expression") {
        match spec_prop {
            // Expression.result is a derived owned-feature accessor.
            "result" => return (Derived, vec![]),
            // featureTarget on Feature/Expression always reachable.
            "featureTarget" => return (Derived, vec![]),
            // instantiatedType: spec attaches a Type ref naming the
            // operator/function. TS encodes the same reference through
            // the typing chain (`unresolved_type` for InvocationExpression
            // / InstantiationExpression) or implicitly via the expression
            // kind (IndexExpression, OperatorExpression). Treat as Derived
            // so the metric reflects how TS represents the data.
            "instantiatedType" => return (Derived, vec![]),
            // referent on FeatureReferenceExpression points to the
            // referenced Feature. TS encodes via the typing chain
            // (`unresolved_type` resolves to the feature) rather than a
            // dedicated `referent` prop.
            "referent" => return (Derived, vec![]),
            // operator: only OperatorExpression carries an explicit
            // `operator` prop. IndexExpression's operator is implicit
            // in the kind itself (always `[`).
            "operator" => {
                if kind == "OperatorExpression" {
                    return (Prop, vec!["operator"]);
                }
                return (Derived, vec![]);
            }
            _ => {}
        }
    }
    // Feature-chain `featureTarget` outside Expression context.
    if spec_prop == "featureTarget" && kind_is_subtype_of(kind, "Feature") {
        return (Derived, vec![]);
    }

    // ----- Conjugation / port-conjugation --------------------------------
    if kind == "Conjugation" || kind == "PortConjugation" {
        match spec_prop {
            "originalType" => return (Prop, vec!["originalType", "unresolved_originalType"]),
            "conjugatedType" => return (Prop, vec!["conjugatedType", "unresolved_conjugatedType"]),
            _ => {}
        }
    }

    // ----- Feature value / case / verification roles ---------------------
    if kind == "FeatureValue" {
        match spec_prop {
            // FeatureValue is a FeatureMembership that wraps an
            // expression. TS sets `specific` (feature) + `general`
            // (value source) directly on the FeatureValue element
            // itself, so memberElement/value are derived.
            "value"
            | "memberElement"
            | "ownedMemberElement"
            | "ownedMemberFeature"
            | "ownedMemberParameter" => return (Derived, vec![]),
            "featureWithValue" => return (Owner, vec![]),
            "isDefault" | "isInitial" => return (Prop, vec![]),
            _ => {}
        }
    }
    // subjectParameter on Definition/Usage kinds: TS parses `subject` as
    // a `subject_requirement` node which mints a SubjectMembership
    // child wrapped in an OwningMembership. The subject parameter is
    // reachable via the parent's owned-children index, not via the
    // ReverseMemberElement direct back-pointer that the metric checks.
    // Classify as Derived to reflect how TS encodes structural members.
    if matches!(
        kind,
        "AnalysisCaseDefinition"
            | "AnalysisCaseUsage"
            | "CaseDefinition"
            | "CaseUsage"
            | "VerificationCaseDefinition"
            | "VerificationCaseUsage"
            | "UseCaseDefinition"
            | "UseCaseUsage"
            | "RequirementDefinition"
            | "RequirementUsage"
            | "ConcernDefinition"
            | "ConcernUsage"
            | "ViewpointDefinition"
            | "ViewpointUsage"
            | "IncludeUseCaseUsage"
            | "SatisfyRequirementUsage"
    ) && spec_prop == "subjectParameter"
    {
        return (Derived, vec![]);
    }

    // Loop / branch action body slot data: TS parses statements as
    // generic owned children of the For/While/If usage. The audit
    // (2026-05-26) confirmed `bodyAction` / `thenAction` reachable as
    // undifferentiated children. TS-2.14 (2026-05-26) extends the same
    // undifferentiated-child encoding to the remaining loop/branch slots:
    //   * for-loop `var` → ReferenceUsage child (named after var)
    //   * for-loop sequence expression → structured expression subtree
    //   * while-loop / if condition → structured expression subtree
    // Slot identity is collapsed (no ParameterMembership envelope) but
    // the data is reachable via owner_to_children, matching the
    // bodyAction precedent. The ast_builder dispatch arms for for_action
    // / while_action / if_action emit these children via
    // `emit_for_loop_variable` / `emit_for_seq_argument` /
    // `emit_while_argument` / `emit_if_argument` (usages.rs). The
    // for-loop also has G14 closed: the action element's `name` is
    // cleared post-`process_usage` so the var (or its error-recovered
    // tail) doesn't leak as the for-loop's name.
    if matches!(
        kind,
        "ForLoopActionUsage" | "WhileLoopActionUsage" | "IfActionUsage"
    ) {
        match spec_prop {
            "bodyAction" | "thenAction" => return (Derived, vec![]),
            "loopVariable" | "seqArgument" | "whileArgument" | "ifArgument" => {
                return (Derived, vec![]);
            }
            _ => {}
        }
    }

    // AcceptActionUsage.payloadParameter / SendActionUsage.payloadArgument.
    // TS-2.14 (2026-05-26): the accept_action / send_action dispatch arms
    // now emit child elements for these slots (`emit_accept_payload_parameter`
    // / `emit_send_payload_argument` in usages.rs):
    //   * accept name+typing → ReferenceUsage child (mirroring the accept's
    //     own name+typing so the AcceptActionUsage retains its name per
    //     `test_accept_action_dispatch` invariant)
    //   * send payload → structured expression subtree
    // Data lands as undifferentiated children of the action usage, matching
    // the bodyAction/loopVariable encoding. Mark as Derived.
    if matches!(kind, "AcceptActionUsage") && spec_prop == "payloadParameter" {
        return (Derived, vec![]);
    }
    if matches!(kind, "SendActionUsage") && spec_prop == "payloadArgument" {
        return (Derived, vec![]);
    }

    // ----- Requirement / Constraint roles --------------------------------
    if matches!(
        kind,
        "FramedConcernMembership"
            | "RequirementConstraintMembership"
            | "RequirementVerificationMembership"
            | "ActorMembership"
            | "StakeholderMembership"
    ) {
        if spec_prop == "ownedConstraint" || spec_prop == "referencedConstraint" {
            return (ReverseMemberElement, vec![]);
        }
        if spec_prop == "kind" {
            // For RequirementVerificationMembership the kind enum
            // (verifiedConstraint) is implicit in the membership being
            // RequirementVerificationMembership itself — TS doesn't
            // emit a `role`/`kind` prop on it.
            if kind == "RequirementVerificationMembership" {
                return (Derived, vec![]);
            }
            return (Prop, vec!["kind", "role"]);
        }
    }
    // ExhibitState / Perform / Include / EventOccurrence behaviour-reference usages.
    // TS encodes the referenced behaviour through the typing chain
    // (`unresolved_type` -> resolved Type). The spec names this same
    // reference via per-kind aliases (`exhibitedState`, `performedAction`,
    // `useCaseIncluded`, `eventOccurrence`). All point to the same
    // physical reference, so treat as Derived from the typing axis.
    if matches!(
        kind,
        "ExhibitStateUsage" | "IncludeUseCaseUsage" | "PerformActionUsage" | "EventOccurrenceUsage"
    ) {
        match spec_prop {
            "performedAction" | "bodyAction" | "eventOccurrence" | "exhibitedState"
            | "useCaseIncluded" => return (Derived, vec![]),
            _ => {}
        }
    }

    // SatisfyRequirementUsage: TS encodes the satisfied requirement via
    // `unresolved_type` (the requirement reference) and the satisfying
    // feature via `satisfiedBy` (mapped to `satisfyingFeature` above).
    // `assertedConstraint` inherits from AssertConstraintUsage but for
    // satisfy/verify usages it's the requirement being satisfied — same
    // physical reference as satisfiedRequirement.
    //
    // `satisfyingFeature` is Exactly-one per OSLC but the by-clause
    // (which produces an explicit TS `satisfiedBy`) is optional in the
    // grammar — the default-case satisfying feature is the enclosing
    // context. Treat as Derived so the implicit case doesn't penalise.
    if kind == "SatisfyRequirementUsage" {
        match spec_prop {
            "satisfiedRequirement" | "assertedConstraint" | "satisfyingFeature" => {
                return (Derived, vec![]);
            }
            _ => {}
        }
    }

    // AssertConstraintUsage: TS stores the asserted constraint expression
    // under the `constraint` shorthand (mapped to `ownedConstraint` via
    // map_ts_prop_to_spec). The spec uses `assertedConstraint`.
    if kind == "AssertConstraintUsage" && spec_prop == "assertedConstraint" {
        return (
            Prop,
            vec!["assertedConstraint", "ownedConstraint", "constraint"],
        );
    }

    // RequirementVerificationMembership: TS sets `verifiedRequirement`
    // directly as a string prop. The `kind` enum (verifiedConstraint /
    // checkedConstraint / etc.) is implicit in the membership kind itself.
    if kind == "RequirementVerificationMembership" {
        match spec_prop {
            "verifiedRequirement" => {
                return (Prop, vec!["verifiedRequirement"]);
            }
            "kind" => return (Derived, vec![]),
            _ => {}
        }
    }

    // TransitionUsage.succession — succession is a derived view over
    // the transition's source/target endpoints which TS emits as
    // structural state-machine wiring rather than a single prop.
    if kind == "TransitionUsage" && spec_prop == "succession" {
        return (Derived, vec![]);
    }

    // EnumerationUsage.enumerationDefinition (§8.3.8.3): a derived accessor
    // that redefines attributeDefinition — the single EnumerationDefinition
    // typing this enumerated value. The enum-member lowering mints a real
    // FeatureTyping to the owning enum def, so the derived accessor is
    // grounded through the typing chain.
    if kind == "EnumerationUsage" && spec_prop == "enumerationDefinition" {
        return (Derived, vec![]);
    }

    // ConjugatedPortTyping: portDefinition / conjugatedPortDefinition
    // are typed views of the typing's target. TS encodes the same
    // reference through `unresolved_type` (the original PortDefinition
    // name); name resolution later resolves to both the PortDefinition
    // and its implicit ConjugatedPortDefinition twin.
    if kind == "ConjugatedPortTyping" {
        match spec_prop {
            "portDefinition" | "conjugatedPortDefinition" => return (Derived, vec![]),
            _ => {}
        }
    }

    // ConjugatedPortDefinition.ownedPortConjugator: spec models the
    // conjugation as a separate `PortConjugation` child element. TS
    // collapses the relation into the `originalPortDefinition` Ref on
    // the ConjugatedPortDefinition itself (set by
    // `create_conjugated_port_definition_with_key`), so the same
    // information is reachable without needing a wrapper child.
    if kind == "ConjugatedPortDefinition" && spec_prop == "ownedPortConjugator" {
        return (Derived, vec![]);
    }

    // ----- Per-prop globals fallthrough ---------------------------------
    match spec_prop {
        // Always-on prop axis.
        "body" | "operator" | "visibility" | "isNegated" | "kind" | "isStandard" | "isDefault"
        | "isInitial" | "isImportAll" | "isRecursive" => (Prop, vec![]),
        // Derived (typed accessor walks). `*Id` versions of an
        // already-required ref prop are trivially derived from the
        // ref itself.
        "result" | "featureTarget" => (Derived, vec![]),
        "memberElementId"
        | "ownedMemberElementId"
        | "ownedMemberFeatureId"
        | "ownedMemberParameterId"
        | "documentedElement"
        | "annotatedElement" => (Derived, vec![]),
        // Ownership.
        "owningType" | "owningNamespace" | "owningMembership" | "membershipOwningNamespace" => {
            (Owner, vec![])
        }
        // Loop / branching action-body wiring. TS owns these as child
        // memberships rather than direct props.
        "loopVariable"
        | "bodyAction"
        | "ifArgument"
        | "ifBody"
        | "elseBody"
        | "ownedResultExpression"
        | "ownedSubjectParameter"
        | "ownedRendering"
        | "ownedTransition"
        | "assertedConstraint"
        | "assumedConstraint"
        | "requiredConstraint"
        | "satisfiedRequirement"
        | "verifiedRequirement"
        | "actualParameter"
        | "argumentValue"
        | "subjectParameter"
        | "ownedActorParameter"
        | "ownedObjectiveRequirement"
        | "ownedRequirement"
        | "payloadArgument"
        | "payloadParameter"
        | "seqArgument"
        | "referencedRendering"
        // TransitionUsage trigger/guard/effect (§8.3.18.9): real
        // AcceptActionUsage/Expression/ActionUsage children owned through
        // TransitionFeatureMembership(kind) wrappers (task #81) — reachable
        // via the reverse-member index like bodyAction/thenAction.
        | "triggerAction"
        | "guardExpression"
        | "effectAction"
        | "succession" => (ReverseMemberElement, vec![]),
        // Port conjugation: TS emits unresolved refs as props.
        "originalPortDefinition"
        | "ownedPortConjugator"
        | "conjugatedPortDefinition"
        | "portDefinition" => (Prop, vec![]),
        // Literal value, satisfy-feature, assignment referent — direct
        // props. Note: AssignmentActionUsage emits the assignment target
        // under `target`/`targetFeature` rather than `referent`, so we
        // include those as aliases. Other kinds with a `referent` shape
        // requirement fall through to the kind-specific branches above.
        "value" | "satisfyingFeature" => (Prop, vec![]),
        "referent" => (Prop, vec!["referent", "target", "targetFeature"]),
        // TransitionUsage source/target are ref props that TS emits as
        // unresolved_source/target.
        "source" | "target" => (Prop, vec![]),
        // IfActionUsage / WhileLoopActionUsage / IncludeUseCaseUsage
        // body wiring — owned children.
        "thenAction" | "elseAction" | "whileArgument" | "useCaseIncluded" => {
            (ReverseMemberElement, vec![])
        }
        _ => (Unclassified, vec![]),
    }
}

/// Map a runtime prop name to its `&'static str` form so we can return
/// it from the axis classifier (which yields `Vec<&'static str>`).
/// Centralised here so future entries don't need to repeat the
/// `&'static`-only literal list.
fn spec_prop_static(name: &str) -> &'static str {
    match name {
        "body" => "body",
        "operator" => "operator",
        "visibility" => "visibility",
        "isNegated" => "isNegated",
        "kind" => "kind",
        "memberName" => "memberName",
        "memberShortName" => "memberShortName",
        "isImportAll" => "isImportAll",
        "isRecursive" => "isRecursive",
        "isDefault" => "isDefault",
        "isInitial" => "isInitial",
        "performedAction" => "performedAction",
        "bodyAction" => "bodyAction",
        "eventOccurrence" => "eventOccurrence",
        "exhibitedState" => "exhibitedState",
        "instantiatedType" => "instantiatedType",
        "referent" => "referent",
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// Corpus discovery
// ---------------------------------------------------------------------------

fn collect_model_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_into(root, &mut out);
    out.sort();
    out
}

fn collect_into(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_into(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "sysml" || e == "kerml")
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

fn parse_ts_or_skip(path: &Path) -> Option<sysml_core::ModelGraph> {
    let text = std::fs::read_to_string(path).ok()?;
    let parser = TreeSitterParser::new();
    let tree = parser.parse_tree(&text)?;
    Some(build_model_graph(&tree, &text, &path.to_string_lossy()).graph)
}

// ---------------------------------------------------------------------------
// Inventory
// ---------------------------------------------------------------------------

#[derive(Default)]
struct KindInventory {
    /// # of corpus entries (Element-kind or Relationship-kind) observed.
    entry_count: usize,
    /// # of entries seen as `Element` (vs. `Relationship`).
    element_count: usize,
    /// # of entries seen as `Relationship`.
    relationship_count: usize,
    /// TS prop key → # of entries that emitted it.
    ts_prop_counts: BTreeMap<String, usize>,
    /// # of entries with a populated source endpoint
    /// (always 1 per Relationship-kind entry).
    has_source: usize,
    /// # of entries with a populated target endpoint.
    has_target: usize,
    /// # of entries with `Element.owner` set.
    has_owner: usize,
    /// # of entries with `Element.owning_membership` set.
    has_owning_membership: usize,
    /// # of times some other element in the corpus points back via
    /// `owning_membership` (used for Membership-kind reverse lookup).
    has_reverse_member: usize,
}

#[derive(Default)]
struct CorpusInventory {
    kinds: BTreeMap<String, KindInventory>,
    kinds_without_shape: BTreeSet<String>,
    parsed_files: usize,
    skipped_files: usize,
}

fn ingest_corpus(corpus: &[PathBuf], shapes: &HashMap<String, ShapeInfo>) -> CorpusInventory {
    let mut inv = CorpusInventory::default();
    for path in corpus {
        let Some(graph) = parse_ts_or_skip(path) else {
            inv.skipped_files += 1;
            continue;
        };
        inv.parsed_files += 1;

        // First pass: Element-axis evidence.
        for element in graph.elements.values() {
            let kind_name = element.kind.as_str().to_string();
            let entry = inv.kinds.entry(kind_name.clone()).or_default();
            entry.entry_count += 1;
            entry.element_count += 1;
            if element.owner.is_some() {
                entry.has_owner += 1;
            }
            if element.owning_membership.is_some() {
                entry.has_owning_membership += 1;
            }
            for key in element.props.keys() {
                *entry.ts_prop_counts.entry(key.to_string()).or_insert(0) += 1;
            }
            if !shapes.contains_key(&kind_name) {
                inv.kinds_without_shape.insert(kind_name);
            }
        }

        // Second pass: Relationship-axis evidence. (TS rarely uses
        // the `Relationship` container in the corpus path — most
        // relationship-kind entries are emitted as Elements via
        // `add_owned_element` — but we still count them for any case
        // that does land here.)
        for rel in graph.relationships.values() {
            let kind_name = rel.kind.as_str().to_string();
            let entry = inv.kinds.entry(kind_name.clone()).or_default();
            entry.entry_count += 1;
            entry.relationship_count += 1;
            entry.has_source += 1;
            entry.has_target += 1;
            for key in rel.props.keys() {
                *entry.ts_prop_counts.entry(key.to_string()).or_insert(0) += 1;
            }
            if !shapes.contains_key(&kind_name) {
                inv.kinds_without_shape.insert(kind_name);
            }
        }

        // Third pass: reverse member index. For each element whose
        // `owning_membership` points at some membership element M,
        // record that M has at least one reverse-pointing member.
        for element in graph.elements.values() {
            let Some(om_id) = &element.owning_membership else {
                continue;
            };
            let Some(membership) = graph.elements.get(om_id) else {
                continue;
            };
            let kind_name = membership.kind.as_str().to_string();
            let entry = inv.kinds.entry(kind_name).or_default();
            entry.has_reverse_member += 1;
        }
    }
    inv
}

// ---------------------------------------------------------------------------
// Gates
// ---------------------------------------------------------------------------

/// Strict gate (union variant): every TS prop key emitted across the
/// corpus, after mapping + allowlist, must appear in **some** shape's
/// property list. This is the original v1 behavior — kept as-is so
/// the three corpus tests stay green while the tighter per-kind
/// variant runs in advisory mode.
fn collect_orphan_props(
    inv: &CorpusInventory,
    shapes: &HashMap<String, ShapeInfo>,
) -> BTreeMap<String, BTreeSet<String>> {
    collect_orphan_props_union(inv, shapes)
}

/// Strict gate (union variant). See `collect_orphan_props` doc.
fn collect_orphan_props_union(
    inv: &CorpusInventory,
    shapes: &HashMap<String, ShapeInfo>,
) -> BTreeMap<String, BTreeSet<String>> {
    // Build the union of all spec prop names across all shapes.
    let mut spec_names: BTreeSet<String> = BTreeSet::new();
    for shape in shapes.values() {
        for p in &shape.properties {
            spec_names.insert(p.name.clone());
        }
    }
    let internal: BTreeSet<&str> = TS_INTERNAL_ALLOWLIST.iter().copied().collect();

    let mut orphans: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (kind, ki) in &inv.kinds {
        for prop in ki.ts_prop_counts.keys() {
            let mapped = map_ts_prop_to_spec(prop);
            if internal.contains(prop.as_str()) {
                continue;
            }
            if spec_names.contains(mapped) {
                continue;
            }
            orphans
                .entry(prop.clone())
                .or_default()
                .insert(kind.clone());
        }
    }
    orphans
}

/// Tighter gate (per-kind variant, advisory only): for each
/// (kind, TS prop) pair, the mapped spec name must appear in **the
/// kind's own shape**, not just in some shape across the union of all
/// shapes. Kinds with no shape are skipped (covered by Layer 4 v0,
/// `spec_kind_conformance.rs`).
///
/// Returns a map keyed by `(ts_prop, kind)` whose value is the
/// per-(prop,kind) emission count from the corpus inventory, so the
/// triage print can sort by frequency.
fn collect_orphan_props_per_kind(
    inv: &CorpusInventory,
    shapes: &HashMap<String, ShapeInfo>,
) -> BTreeMap<(String, String), usize> {
    let internal: BTreeSet<&str> = TS_INTERNAL_ALLOWLIST.iter().copied().collect();
    let mut orphans: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (kind, ki) in &inv.kinds {
        let Some(shape) = shapes.get(kind) else {
            continue;
        };
        for (prop, count) in &ki.ts_prop_counts {
            if internal.contains(prop.as_str()) {
                continue;
            }
            // Per-kind TS-internal allowlist: TS encodings that only
            // apply on this specific element kind. See
            // `TS_INTERNAL_PER_KIND_ALLOWLIST` for the rationale per
            // entry. We match on the bare prop name; the lookup table
            // owns both `unresolved_` and non-unresolved forms.
            if is_ts_internal_per_kind(prop, kind) {
                continue;
            }
            let stripped = prop.strip_prefix("unresolved_").unwrap_or(prop);
            if is_ts_internal_per_kind(stripped, kind) {
                continue;
            }
            let mapped = map_ts_prop_to_spec_for_kind(prop, kind);
            let in_own_shape = shape.properties.iter().any(|p| p.name == mapped);
            if in_own_shape {
                continue;
            }
            orphans.insert((prop.clone(), kind.clone()), *count);
        }
    }
    orphans
}

/// Assert the per-kind orphan map is empty, printing the same triage
/// block format the advisory mode used so a regression failure
/// surfaces the exact `(prop, kind, count)` rows that drifted.
///
/// Was advisory through TS-2.14b; promoted to assertion once
/// G10/G11/G12/G13 + the shorthand tail closed (commit 2c26ccb7).
/// Per-kind orphans floor is now 0 across all 3 corpora and any
/// future drift must either be a real spec mismatch in the parser or
/// a justified entry in `TS_INTERNAL_PER_KIND_ALLOWLIST`.
fn assert_no_per_kind_orphans(label: &str, orphans: &BTreeMap<(String, String), usize>) {
    if orphans.is_empty() {
        eprintln!("[{label}] [per-kind orphans] 0 (prop,kind) violations");
        return;
    }
    // Distinct TS prop names across the violations.
    let mut distinct_props: BTreeMap<String, usize> = BTreeMap::new();
    for ((prop, _kind), count) in orphans {
        *distinct_props.entry(prop.clone()).or_insert(0) += *count;
    }
    // Sort (prop,kind) entries by count descending so the worst
    // offenders appear first in the failure dump.
    let mut sorted: Vec<_> = orphans.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let mut lines = Vec::with_capacity(sorted.len() + 2);
    lines.push(format!(
        "[{label}] [per-kind orphans] {n} (prop,kind) violations across {d} distinct TS prop names:",
        n = orphans.len(),
        d = distinct_props.len()
    ));
    for ((prop, kind), count) in &sorted {
        lines.push(format!("    {prop:<40} on {kind:<40} ({count} emissions)"));
    }
    lines.push(
        "Add the (prop,kind) pair to TS_INTERNAL_PER_KIND_ALLOWLIST with a justification, \
         OR fix the TS ast_builder to emit the spec-correct prop name, \
         OR extend map_ts_prop_to_spec_for_kind with a kind-conditional rename."
            .to_owned(),
    );
    panic!("{}", lines.join("\n"));
}

/// Axis-aware required-prop coverage.
///
/// For each `(kind, required-prop)` pair the kind's shape defines:
/// * skip if the prop is in `FRAMEWORK_ALLOWLIST` (OSLC resource
///   identity / dcterms metadata — not modeled per-element),
/// * skip if the prop is in `DEFAULT_ELIDABLE_BOOLEAN_ALLOWLIST`
///   (TS elides `false` defaults),
/// * otherwise classify the prop into a `SpecPropAxis` and check the
///   matching evidence on the inventory entry.
///
/// Returns `(satisfied, total, unclassified)` where `unclassified` is
/// the count of (kind, prop) pairs whose axis has not been registered
/// in `spec_prop_axis` yet — printed in the report so future passes
/// can chip them down.
fn measure_required_prop_coverage(
    inv: &CorpusInventory,
    shapes: &HashMap<String, ShapeInfo>,
) -> CoverageReport {
    let framework: BTreeSet<&str> = FRAMEWORK_ALLOWLIST.iter().copied().collect();
    let elidable: BTreeSet<&str> = DEFAULT_ELIDABLE_BOOLEAN_ALLOWLIST.iter().copied().collect();
    let mut report = CoverageReport::default();
    for (kind, ki) in &inv.kinds {
        let Some(shape) = shapes.get(kind) else {
            continue;
        };
        // Pre-compute the set of spec-prop names TS emits via the Props
        // axis (after mapping) for this kind.
        let mut ts_prop_names: BTreeSet<&str> = BTreeSet::new();
        for ts_key in ki.ts_prop_counts.keys() {
            ts_prop_names.insert(map_ts_prop_to_spec(ts_key));
        }

        for prop in shape.properties.iter().filter(|p| is_required(p)) {
            let name = prop.name.as_str();
            if framework.contains(name) || elidable.contains(name) {
                continue;
            }
            let (axis, aliases) = spec_prop_axis(kind, name);
            if matches!(axis, SpecPropAxis::Unclassified) {
                report
                    .unclassified
                    .entry(name.to_string())
                    .or_default()
                    .insert(kind.clone());
                report.total += 1;
                continue;
            }
            report.total += 1;
            let satisfied = match axis {
                SpecPropAxis::Prop => {
                    // Probe the literal name AND any kind-specific
                    // aliases. The aliases include both the spec-side
                    // synonym (e.g. `general`) and the TS-side rename
                    // (e.g. `type`, `unresolved_type`).
                    let mut probe = std::iter::once(name).chain(aliases.iter().copied());
                    probe.any(|n| {
                        ts_prop_names.contains(n)
                            || ki.ts_prop_counts.keys().any(|k| {
                                map_ts_prop_to_spec(k) == n
                                    || k == n
                                    || k.strip_prefix("unresolved_") == Some(n)
                            })
                    })
                }
                SpecPropAxis::RelSource => ki.has_source > 0,
                SpecPropAxis::RelTarget => ki.has_target > 0,
                SpecPropAxis::Owner => ki.has_owner > 0,
                SpecPropAxis::OwningMembership => ki.has_owning_membership > 0,
                SpecPropAxis::ReverseMemberElement => ki.has_reverse_member > 0,
                SpecPropAxis::Derived => true,
                SpecPropAxis::Unclassified => unreachable!(),
            };
            if satisfied {
                report.satisfied += 1;
            } else {
                report
                    .axis_misses
                    .entry((kind.clone(), name.to_string()))
                    .or_insert(axis);
            }
        }
    }
    report
}

#[derive(Default)]
struct CoverageReport {
    satisfied: usize,
    total: usize,
    /// (kind, prop) → axis: spec required-prop classified but TS did
    /// not populate the expected axis for that kind. Signals either a
    /// real TS gap or a mis-classified axis.
    axis_misses: BTreeMap<(String, String), SpecPropAxis>,
    /// prop → set of kinds: spec required-prop has no axis entry yet.
    /// Surfaces props the table doesn't know about.
    unclassified: BTreeMap<String, BTreeSet<String>>,
}

impl CoverageReport {
    fn pct(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            100.0 * self.satisfied as f64 / self.total as f64
        }
    }
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

fn report(label: &str, inv: &CorpusInventory, shapes: &HashMap<String, ShapeInfo>) {
    let total_elements: usize = inv.kinds.values().map(|k| k.element_count).sum();
    let total_rels: usize = inv.kinds.values().map(|k| k.relationship_count).sum();
    eprintln!(
        "[{label}] parsed={p} skipped={s} elements={e} relationships={r} kinds={k} with_shape={w} without_shape={ws}",
        p = inv.parsed_files,
        s = inv.skipped_files,
        e = total_elements,
        r = total_rels,
        k = inv.kinds.len(),
        w = inv.kinds.len() - inv.kinds_without_shape.len(),
        ws = inv.kinds_without_shape.len(),
    );

    let cov = measure_required_prop_coverage(inv, shapes);
    eprintln!(
        "[{label}] required-prop coverage (axis-aware, advisory): {sat} / {total} = {pct:.1}%",
        sat = cov.satisfied,
        total = cov.total,
        pct = cov.pct(),
    );

    // Print all axis misses for classifier-sweep triage.
    if !cov.axis_misses.is_empty() {
        eprintln!(
            "[{label}]   axis misses ({n} total):",
            n = cov.axis_misses.len()
        );
        for ((kind, prop), axis) in cov.axis_misses.iter() {
            eprintln!("[{label}]     {kind}.{prop} expected via {axis:?}");
        }
    }

    // Print up to 10 unclassified spec props for triage.
    if !cov.unclassified.is_empty() {
        eprintln!(
            "[{label}]   unclassified spec props ({n} distinct names, {total} (kind,prop) pairs): first 10 ↓",
            n = cov.unclassified.len(),
            total = cov.unclassified.values().map(|s| s.len()).sum::<usize>(),
        );
        for (prop, kinds) in cov.unclassified.iter().take(10) {
            let sample: Vec<&String> = kinds.iter().take(3).collect();
            eprintln!(
                "[{label}]     {prop} ({n} kinds, sample: {sample:?})",
                n = kinds.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn ts_prop_keys_are_grounded_in_spec_pilot_examples() {
    let shapes = load_shapes();
    let corpus = collect_model_files(&pilot_examples_root());
    assert!(!corpus.is_empty());
    let inv = ingest_corpus(&corpus, &shapes);
    report("Pilot examples", &inv, &shapes);

    let orphans = collect_orphan_props(&inv, &shapes);
    assert!(
        orphans.is_empty(),
        "TS emits {} prop key(s) with no spec home (after mapping + allowlist):\n{}",
        orphans.len(),
        orphans
            .iter()
            .map(|(p, kinds)| {
                let sample: Vec<&String> = kinds.iter().take(4).collect();
                format!("    {} ({} kinds, sample: {:?})", p, kinds.len(), sample)
            })
            .collect::<Vec<_>>()
            .join("\n")
    );

    let per_kind_orphans = collect_orphan_props_per_kind(&inv, &shapes);
    assert_no_per_kind_orphans("Pilot examples", &per_kind_orphans);

    let cov = measure_required_prop_coverage(&inv, &shapes);
    assert!(
        cov.pct() >= PILOT_COVERAGE_FLOOR_PCT,
        "Pilot examples coverage {:.1}% below floor {:.1}% \
         ({} / {} satisfied)",
        cov.pct(),
        PILOT_COVERAGE_FLOOR_PCT,
        cov.satisfied,
        cov.total
    );
}

#[test]
fn ts_prop_keys_are_grounded_in_spec_kerml_examples() {
    let shapes = load_shapes();
    let corpus = collect_model_files(&kerml_examples_root());
    assert!(!corpus.is_empty());
    let inv = ingest_corpus(&corpus, &shapes);
    report("KerML examples", &inv, &shapes);

    let orphans = collect_orphan_props(&inv, &shapes);
    assert!(
        orphans.is_empty(),
        "TS emits {} prop key(s) with no spec home (after mapping + allowlist):\n{}",
        orphans.len(),
        orphans
            .iter()
            .map(|(p, kinds)| {
                let sample: Vec<&String> = kinds.iter().take(4).collect();
                format!("    {} ({} kinds, sample: {:?})", p, kinds.len(), sample)
            })
            .collect::<Vec<_>>()
            .join("\n")
    );

    let per_kind_orphans = collect_orphan_props_per_kind(&inv, &shapes);
    assert_no_per_kind_orphans("KerML examples", &per_kind_orphans);

    let cov = measure_required_prop_coverage(&inv, &shapes);
    assert!(
        cov.pct() >= KERML_COVERAGE_FLOOR_PCT,
        "KerML examples coverage {:.1}% below floor {:.1}% \
         ({} / {} satisfied)",
        cov.pct(),
        KERML_COVERAGE_FLOOR_PCT,
        cov.satisfied,
        cov.total
    );
}

#[test]
fn ts_prop_keys_are_grounded_in_spec_local_examples() {
    let shapes = load_shapes();
    let corpus = collect_model_files(&local_examples_root());
    assert!(!corpus.is_empty());
    let inv = ingest_corpus(&corpus, &shapes);
    report("local examples", &inv, &shapes);

    let orphans = collect_orphan_props(&inv, &shapes);
    assert!(
        orphans.is_empty(),
        "TS emits {} prop key(s) with no spec home (after mapping + allowlist):\n{}",
        orphans.len(),
        orphans
            .iter()
            .map(|(p, kinds)| {
                let sample: Vec<&String> = kinds.iter().take(4).collect();
                format!("    {} ({} kinds, sample: {:?})", p, kinds.len(), sample)
            })
            .collect::<Vec<_>>()
            .join("\n")
    );

    let per_kind_orphans = collect_orphan_props_per_kind(&inv, &shapes);
    assert_no_per_kind_orphans("local examples", &per_kind_orphans);

    let cov = measure_required_prop_coverage(&inv, &shapes);
    assert!(
        cov.pct() >= LOCAL_COVERAGE_FLOOR_PCT,
        "local examples coverage {:.1}% below floor {:.1}% \
         ({} / {} satisfied)",
        cov.pct(),
        LOCAL_COVERAGE_FLOOR_PCT,
        cov.satisfied,
        cov.total
    );
}

#[cfg(test)]
mod mapping_unit_tests {
    use super::*;

    #[test]
    fn unresolved_prefix_is_stripped() {
        assert_eq!(map_ts_prop_to_spec("unresolved_type"), "type");
        assert_eq!(
            map_ts_prop_to_spec("unresolved_subsettedFeature"),
            "subsettedFeature"
        );
        assert_eq!(
            map_ts_prop_to_spec("unresolved_redefinedFeature"),
            "redefinedFeature"
        );
    }

    #[test]
    fn multiplicity_variants_collapse_to_one_spec_name() {
        assert_eq!(map_ts_prop_to_spec("multiplicity_lower"), "multiplicity");
        assert_eq!(map_ts_prop_to_spec("multiplicity_upper"), "multiplicity");
        assert_eq!(
            map_ts_prop_to_spec("multiplicity_lower_text"),
            "multiplicity"
        );
    }

    #[test]
    fn expression_aliases() {
        assert_eq!(map_ts_prop_to_spec("expr"), "expression");
        // `guard`/`trigger`/`effect` rows retired with task #81 (real
        // TransitionFeatureMembership-wrapped children replaced the props).
        assert_eq!(map_ts_prop_to_spec("guard"), "guard");
        assert_eq!(map_ts_prop_to_spec("trigger"), "trigger");
        assert_eq!(map_ts_prop_to_spec("effect"), "effect");
    }

    #[test]
    fn membership_alias_target_maps_to_member_element() {
        assert_eq!(map_ts_prop_to_spec("aliasTarget"), "memberElement");
        assert_eq!(
            map_ts_prop_to_spec("unresolved_aliasTarget"),
            "memberElement"
        );
    }

    #[test]
    fn unknown_key_passes_through() {
        assert_eq!(
            map_ts_prop_to_spec("ownedRelationship"),
            "ownedRelationship"
        );
        assert_eq!(map_ts_prop_to_spec("name"), "name");
    }
}
