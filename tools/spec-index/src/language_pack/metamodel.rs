//! Metamodel (TTL vocabulary) + SHACL (OSLC shapes) integration.
//!
//! Motivation: the 182 metamodel classes and 257 SHACL shapes were
//! surfaced only as opaque `block`-mapped slots (metamodel 0/182, shacl 0/257 in
//! `coverage_by_source_kind`). This module turns each into a REVIEWED denominator
//! record with an auditable disposition and, for the common case, ENRICHES the
//! existing concept card with its metamodel facet instead of minting a duplicate
//! grammar+metamodel card (the review explicitly warns against duplicate cards —
//! `sysml.structure.part-usage` carries its `PartUsage` metaclass facet, it does
//! not get a second card).
//!
//! Sources are the allowlisted, hashed vocabulary + shapes TTLs, parsed with the
//! codegen parsers that already have one home for this job (`parse_ttl_vocab`,
//! `shapes_parser::parse_oslc_shapes`) — no second TTL parser is introduced.
//!
//! Disposition of each metaclass / shape:
//! - **fold-and-enrich** (the common case): a concept card already lowers to the
//!   metaclass (`semantic_types` names it) — the metamodel facet folds onto that
//!   ONE card and a `duplicate`-mapped record points at it.
//! - **card**: a metamodel-only concept an LLM authoring SysML needs (the graph
//!   memberships — `Membership`/`OwningMembership`/`FeatureMembership`/…) with no
//!   grammar-lowered card yet gets its own `metadata`-facet card.
//! - **abstract-only**: an abstract base or a metamodel enumeration with no
//!   textual notation whose semantics live in its concrete carded subtypes — a
//!   reviewed `abstract-only`/`exclusion` record (never a card).
//! - **fold-into-uncarded / known-gap**: a metaclass whose written form our
//!   tree-sitter lowering does not materialize (documented in `known_gaps`) —
//!   folds into the closest carded concept with the gap rationale.

use std::collections::BTreeMap;

use serde::Serialize;
use sysml_codegen::shapes_parser::{self, Cardinality, PropertyType, ShapeInfo};
use sysml_codegen::parse_ttl_vocab;

use super::cards::{ClauseRef, ExamplesRef, GrammarRuleRef, LanguageCard, Provenance};
use super::citations;
use super::concepts::{slugify, Classification, DenominatorRecord, Mapping};
use super::support::SupportAxes;
use super::{manifest, LpError};

// --- Facet data structures (schema `metamodelFacet` / `metamodelProperty`) ---

/// One property constraint of a metaclass, read from its OSLC shape.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MetamodelProperty {
    /// Property name (e.g. `owningType`, `general`, `isComposite`).
    pub name: String,
    /// Multiplicity as `min..max` (`0..1`, `1`, `0..*`, `1..*`) from the OSLC
    /// cardinality — the owned/reference-property multiplicity the review names.
    pub multiplicity: String,
    /// `reference` (an element endpoint — the relationship-endpoint / owned or
    /// referenced element), `boolean`, `string`, `datetime`, or `any`.
    pub kind: String,
    /// For a `reference` property, the target element type (an endpoint type).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_type: Option<String>,
    /// A read-only (derived-style) property. The pinned OSLC shapes mark
    /// `read_only`; they do not separately encode UML `isDerived`/`redefinedBy`,
    /// so `read_only` is the honest signal the sources carry.
    #[serde(skip_serializing_if = "is_false")]
    pub read_only: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The metamodel facet folded onto a concept card. Every element is grounded in
/// the allowlisted, hashed vocabulary + shapes TTLs.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MetamodelFacet {
    /// The metaclass (abstract-syntax type) this concept lowers to.
    pub metaclass: String,
    /// Which vocabulary declares it: `KerML` (base) or `SysML` (domain).
    pub authority: String,
    /// Direct superclasses (`rdfs:subClassOf`) — the inheritance the review names.
    pub supertypes: Vec<String>,
    /// The OSLC shape carrying this metaclass's property constraints, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shacl_shape: Option<String>,
    /// Owned / reference properties + multiplicities + relationship endpoints,
    /// from the OSLC shape (sorted by name for determinism).
    pub properties: Vec<MetamodelProperty>,
    /// Repo-relative pointers into the pinned vocabulary + shapes sources.
    pub source_pointers: Vec<String>,
}

// --- Parsed index over the four TTL sources -------------------------------

/// The metamodel/SHACL universe, parsed once from the four allowlisted TTLs.
pub struct MetamodelIndex {
    /// metaclass -> (authority, direct supertypes). KerML declaration wins when a
    /// class is re-declared in the SysML vocab (SysML re-declares every KerML
    /// base), matching the dedup in `source_concept_slots`.
    pub classes: BTreeMap<String, ClassInfo>,
    /// metaclass -> its OSLC shape (property constraints). Keyed by element type.
    pub shapes: BTreeMap<String, ShapeInfo>,
    /// Repo-relative source pointers, for provenance.
    pub kerml_vocab: &'static str,
    pub sysml_vocab: &'static str,
    pub kerml_shapes: &'static str,
    pub sysml_shapes: &'static str,
}

/// A metaclass's declaration facts.
#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub authority: String,
    pub supertypes: Vec<String>,
}

impl MetamodelIndex {
    /// Parse the four TTL sources. `*_vocab`/`*_shapes` are the already-read file
    /// contents (the caller reads them through the allowlist); the `*_path`
    /// arguments are the repo-relative pointers recorded in provenance.
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        kerml_vocab_src: &str,
        sysml_vocab_src: &str,
        kerml_shapes_src: &str,
        sysml_shapes_src: &str,
        kerml_vocab: &'static str,
        sysml_vocab: &'static str,
        kerml_shapes: &'static str,
        sysml_shapes: &'static str,
    ) -> Result<Self, String> {
        let kerml_types =
            parse_ttl_vocab(kerml_vocab_src).map_err(|e| format!("KerML vocab: {e}"))?;
        let sysml_types =
            parse_ttl_vocab(sysml_vocab_src).map_err(|e| format!("SysML vocab: {e}"))?;

        let mut classes: BTreeMap<String, ClassInfo> = BTreeMap::new();
        // KerML first so its declaration wins on a shared name (first-insert).
        for (auth, types) in [("KerML", &kerml_types), ("SysML", &sysml_types)] {
            for t in types {
                classes.entry(t.name.clone()).or_insert_with(|| ClassInfo {
                    authority: auth.to_owned(),
                    supertypes: dedup_sorted(&t.supertypes),
                });
            }
        }

        let (kerml_shapes_v, _kerml_shared) =
            shapes_parser::parse_oslc_shapes(kerml_shapes_src).map_err(|e| format!("KerML shapes: {e}"))?;
        let (sysml_shapes_v, _sysml_shared) =
            shapes_parser::parse_oslc_shapes(sysml_shapes_src).map_err(|e| format!("SysML shapes: {e}"))?;
        let mut shapes: BTreeMap<String, ShapeInfo> = BTreeMap::new();
        // SysML shapes are the richer domain surface; let them win on a shared
        // element type (the SysML shape carries the full property set).
        for sh in kerml_shapes_v.into_iter().chain(sysml_shapes_v.into_iter()) {
            shapes.insert(sh.element_type.clone(), sh);
        }

        Ok(MetamodelIndex {
            classes,
            shapes,
            kerml_vocab,
            sysml_vocab,
            kerml_shapes,
            sysml_shapes,
        })
    }

    /// Build the metamodel facet for a metaclass (inheritance + property
    /// constraints + shape), or `None` if the class is unknown.
    pub fn facet_for(&self, metaclass: &str) -> Option<MetamodelFacet> {
        let info = self.classes.get(metaclass)?;
        let vocab_path = if info.authority == "KerML" {
            self.kerml_vocab
        } else {
            self.sysml_vocab
        };
        let mut source_pointers = vec![format!("{vocab_path}#{metaclass}")];

        let (shacl_shape, properties) = match self.shapes.get(metaclass) {
            Some(sh) => {
                let shapes_path = if info.authority == "KerML" {
                    self.kerml_shapes
                } else {
                    self.sysml_shapes
                };
                source_pointers.push(format!("{shapes_path}#{}", sh.shape_name));
                (Some(sh.shape_name.clone()), shape_properties(sh))
            }
            None => (None, Vec::new()),
        };

        Some(MetamodelFacet {
            metaclass: metaclass.to_owned(),
            authority: info.authority.clone(),
            supertypes: info.supertypes.clone(),
            shacl_shape,
            properties,
            source_pointers,
        })
    }
}

/// Convert an OSLC shape's properties into the facet's property list (sorted).
fn shape_properties(sh: &ShapeInfo) -> Vec<MetamodelProperty> {
    let mut props: Vec<MetamodelProperty> = sh
        .properties
        .iter()
        .map(|p| {
            let (kind, reference_type) = match &p.property_type {
                PropertyType::ElementRef(t) => ("reference", Some(t.clone())),
                PropertyType::Bool => ("boolean", None),
                PropertyType::String => ("string", None),
                PropertyType::DateTime => ("datetime", None),
                PropertyType::Any => ("any", None),
            };
            MetamodelProperty {
                name: p.name.clone(),
                multiplicity: cardinality_str(p.cardinality).to_owned(),
                kind: kind.to_owned(),
                reference_type,
                read_only: p.read_only,
            }
        })
        .collect();
    props.sort_by(|a, b| a.name.cmp(&b.name));
    props.dedup_by(|a, b| a.name == b.name);
    props
}

fn cardinality_str(c: Cardinality) -> &'static str {
    match c {
        Cardinality::ZeroOrMany => "0..*",
        Cardinality::ZeroOrOne => "0..1",
        Cardinality::ExactlyOne => "1",
        Cardinality::OneOrMany => "1..*",
    }
}

fn dedup_sorted(v: &[String]) -> Vec<String> {
    let mut out: Vec<String> = v.to_vec();
    out.sort();
    out.dedup();
    out
}

// --- Reviewed disposition of the residual metamodel classes ---------------
//
// 129 of the 182 metamodel classes are already a concept card's `semantic_types`
// (fold-and-enrich, mechanical); 3 more slug-match a card that just omitted the
// metaclass (folded + the omission fixed). The remaining 50 are metamodel-only:
// each is REVIEWED here (the review's "justify each"). No entry mints a duplicate
// grammar+metamodel card.

/// The reviewed disposition for a metamodel-only class.
#[derive(Debug, Clone, Copy)]
pub enum ResidualDisposition {
    /// Mint a metamodel-only concept card (the graph memberships an LLM authoring
    /// SysML needs — the review's explicit `Membership`/`OwningMembership`
    /// example: no dedicated syntax, but how the model graph is wired).
    Card {
        facet: &'static str,
        clause: &'static str,
    },
    /// The written form exists but our tree-sitter lowering does not materialize
    /// the distinct kind — folds into the existing tooling known-gap card.
    KnownGapFold { tooling_card: &'static str },
    /// An abstract base or metamodel enumeration with no textual notation of its
    /// own; its semantics live in concrete carded subtypes / keyword modifiers.
    AbstractOnly,
}

/// Every metamodel-only class, reviewed. The generator asserts this table
/// exhausts the residual set (fail-hard if the pinned metamodel drop introduces
/// a new class), so no metamodel concept is ever silently undispositioned.
pub const RESIDUAL_METAMODEL: &[(&str, ResidualDisposition, &str)] = &[
    // Graph memberships → their own metadata-free structure cards (KerML). These
    // have no dedicated concrete syntax but are how ownership/features/ends are
    // wired into the model graph; an LLM authoring or reading SysML needs them.
    (
        "Membership",
        ResidualDisposition::Card { facet: "structure", clause: "8.3.2.4.3" },
        "the KerML Membership relationship — how a namespace exposes a member with \
         a visibility; no dedicated concrete syntax but load-bearing for the model \
         graph, so it is carded (the review's Membership/OwningMembership example)",
    ),
    (
        "OwningMembership",
        ResidualDisposition::Card { facet: "structure", clause: "8.3.2.4.8" },
        "the KerML OwningMembership — a membership that also owns its member \
         element; the graph-ownership relationship, carded like Membership",
    ),
    (
        "FeatureMembership",
        ResidualDisposition::Card { facet: "structure", clause: "8.3.3.1.6" },
        "the KerML FeatureMembership — an owning membership whose member is a \
         feature of the owning type; carded as a graph-structural concept",
    ),
    (
        "EndFeatureMembership",
        ResidualDisposition::Card { facet: "structure", clause: "8.3.3.3.3" },
        "the KerML EndFeatureMembership — a feature membership marking a connector \
         end feature; carded as a graph-structural concept",
    ),
    // Written forms our lowering does not materialize as a distinct kind — fold
    // into the existing tooling known-gap cards (targets exist in the registry).
    (
        "ConstructorExpression",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.constructor-expression-generic-lowering",
        },
        "constructor expression `Type(args)` parses but lowers to a generic \
         InvocationExpression (documented tree-sitter lowering gap); folds into \
         the tooling limitation card rather than being carded as a distinct kind",
    ),
    (
        "IfActionUsage",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.control-node-if-terminate-generic-lowering",
        },
        "the `if` control node parses but lowers to a generic TransitionUsage, not \
         a distinct IfActionUsage (documented tree-sitter lowering gap)",
    ),
    (
        "TerminateActionUsage",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.control-node-if-terminate-generic-lowering",
        },
        "the `terminate` action node parses but surfaces no distinct \
         TerminateActionUsage kind (documented tree-sitter lowering gap)",
    ),
    (
        "TextualRepresentation",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.textual-representation-generic-lowering",
        },
        "a textual-representation (`rep language …`) parses but lowers to a generic \
         ReferenceUsage, not a distinct TextualRepresentation (documented gap)",
    ),
    (
        "Unioning",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.type-relationship-fragment-generic-lowering",
        },
        "a type-relationship operator (`unions`) our lowering does not materialize \
         as a distinct kind (documented tree-sitter type-relationship-fragment gap)",
    ),
    (
        "Intersecting",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.type-relationship-fragment-generic-lowering",
        },
        "a type-relationship operator (`intersects`) our lowering does not \
         materialize as a distinct kind (documented type-relationship-fragment gap)",
    ),
    (
        "Differencing",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.type-relationship-fragment-generic-lowering",
        },
        "a type-relationship operator (`differences`) our lowering does not \
         materialize as a distinct kind (documented type-relationship-fragment gap)",
    ),
    (
        "Disjoining",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.type-relationship-fragment-generic-lowering",
        },
        "a type-relationship operator (`disjoint from`) our lowering does not \
         materialize as a distinct kind (documented type-relationship-fragment gap)",
    ),
    (
        "FeatureInverting",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.type-relationship-fragment-generic-lowering",
        },
        "the feature-inversion relationship (`inverse of`) our lowering does not \
         materialize as a distinct kind (documented type-relationship-fragment gap)",
    ),
    (
        "TypeFeaturing",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.type-relationship-fragment-generic-lowering",
        },
        "the type-featuring relationship (`featured by`) our lowering does not \
         materialize as a distinct kind (documented type-relationship-fragment gap)",
    ),
    (
        "Conjugation",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.type-relationship-fragment-generic-lowering",
        },
        "the type-level conjugation relationship (`conjugate ~`) does not parse to \
         a distinct kind (documented type-relationship-fragment gap); NB port \
         conjugation via a conjugated port definition is separately carded",
    ),
    (
        "PortConjugation",
        ResidualDisposition::KnownGapFold {
            tooling_card: "tooling.implementation.type-relationship-fragment-generic-lowering",
        },
        "the port-conjugation relationship metaclass; the conjugated-port syntax is \
         carded via conjugated-port-definition, the relationship itself is not \
         materialized as a distinct kind (documented type-relationship-fragment gap)",
    ),
    // Abstract bases: no textual notation of their own; semantics carried by the
    // concrete written forms, which are carded.
    (
        "Element",
        ResidualDisposition::AbstractOnly,
        "the root abstract metamodel base of every model element; never written \
         directly — its concrete subtypes carry the notation and the cards",
    ),
    (
        "Feature",
        ResidualDisposition::AbstractOnly,
        "the abstract KerML Feature base; the concrete written forms (attribute / \
         part / reference usages, `feature x`) lower to their own carded kinds",
    ),
    (
        "Relationship",
        ResidualDisposition::AbstractOnly,
        "the abstract base of every relationship; its concrete forms \
         (membership/specialization/typing/connection/…) are carded or folded",
    ),
    (
        "ControlNode",
        ResidualDisposition::AbstractOnly,
        "the abstract control-node base; concrete nodes (fork, join, merge, \
         decision) are carded",
    ),
    (
        "AnnotatingElement",
        ResidualDisposition::AbstractOnly,
        "the abstract annotating-element base; its concrete forms — comment, \
         documentation, metadata, textual representation — are carded or folded",
    ),
    (
        "Annotation",
        ResidualDisposition::AbstractOnly,
        "the abstract annotation relationship base; concrete comment/documentation/\
         metadata forms are carded",
    ),
    (
        "Expose",
        ResidualDisposition::AbstractOnly,
        "the abstract expose base; concrete namespace-expose / membership-expose \
         forms are carded",
    ),
    (
        "LiteralExpression",
        ResidualDisposition::AbstractOnly,
        "the abstract literal-expression base; concrete integer/real/string/\
         boolean/infinity literals are carded",
    ),
    (
        "OperatorExpression",
        ResidualDisposition::AbstractOnly,
        "the abstract operator-expression base; the concrete operators are carded \
         by the kerml.expression operator batch",
    ),
    (
        "BooleanExpression",
        ResidualDisposition::AbstractOnly,
        "the abstract boolean-expression base (an expression asserted as a model \
         constraint); concrete boolean operators/invariants are carded",
    ),
    (
        "LoopActionUsage",
        ResidualDisposition::AbstractOnly,
        "the abstract loop-action base; the concrete `while`/`for` loop forms are \
         carded (sysml.behavior.while-loop / for-loop)",
    ),
    // Metamodel enumerations: closed literal sets, not written constructs. Their
    // literals are keyword modifiers on carded concepts (in/out, public/private…).
    (
        "FeatureDirectionKind",
        ResidualDisposition::AbstractOnly,
        "a metamodel enumeration (in/out/inout); its literals are the feature \
         direction keyword on a carded feature, not a written construct",
    ),
    (
        "VisibilityKind",
        ResidualDisposition::AbstractOnly,
        "a metamodel enumeration (public/private/protected); its literals are the \
         visibility keyword on a carded membership, not a written construct",
    ),
    (
        "PortionKind",
        ResidualDisposition::AbstractOnly,
        "a metamodel enumeration (snapshot/timeslice); its literals are the portion \
         keyword on a carded occurrence usage, not a written construct",
    ),
    (
        "TriggerKind",
        ResidualDisposition::AbstractOnly,
        "a metamodel enumeration (when/at/after); its literals are the trigger \
         keyword on a carded accept/transition, not a written construct",
    ),
    (
        "RequirementConstraintKind",
        ResidualDisposition::AbstractOnly,
        "a metamodel enumeration (assumption/requirement); its literals are the \
         require/assume keyword on a carded requirement constraint, not a construct",
    ),
    (
        "StateSubactionKind",
        ResidualDisposition::AbstractOnly,
        "a metamodel enumeration (entry/do/exit); its literals are the state \
         subaction keyword on a carded state, not a written construct",
    ),
    (
        "TransitionFeatureKind",
        ResidualDisposition::AbstractOnly,
        "a metamodel enumeration (trigger/guard/effect); its literals are the \
         transition-feature keyword on a carded transition, not a written construct",
    ),
    // Relationship / feature fragments with no distinct notation: the written
    // concept they belong to (flow, succession flow, subsetting, metadata, …) is
    // carded; the metamodel fragment type is never written on its own.
    (
        "Flow",
        ResidualDisposition::AbstractOnly,
        "the metamodel flow connector type; the written `flow` form is carded as \
         flow usage / flow definition — the bare connector type is not written",
    ),
    (
        "FlowEnd",
        ResidualDisposition::AbstractOnly,
        "a flow-end fragment (the payload feature at a flow endpoint); the written \
         concept is the flow it belongs to, which is carded",
    ),
    (
        "SuccessionFlow",
        ResidualDisposition::AbstractOnly,
        "the metamodel succession-flow connector type; the written `succession \
         flow` form is carded as succession-flow usage",
    ),
    (
        "Step",
        ResidualDisposition::AbstractOnly,
        "the abstract KerML Step (a feature typed by a behavior); the concrete \
         written forms (action/state/calc usages) are carded",
    ),
    (
        "CrossSubsetting",
        ResidualDisposition::AbstractOnly,
        "a cross-subsetting fragment of the flow-crossing syntax; the written \
         crossing/subsetting concept is carded — this metamodel fragment is not \
         written on its own",
    ),
    (
        "ReferenceSubsetting",
        ResidualDisposition::AbstractOnly,
        "the reference-subsetting relationship (`::>`, a redefining reference); the \
         written subsetting form is carded, this metamodel relationship fragment is \
         not written directly on its own",
    ),
    (
        "FeatureChaining",
        ResidualDisposition::AbstractOnly,
        "the feature-chaining relationship behind a feature chain `a.b.c`; the \
         written feature-chain form is carded, the relationship metaclass is not \
         written directly",
    ),
    (
        "ConjugatedPortTyping",
        ResidualDisposition::AbstractOnly,
        "a typing fragment produced by the conjugated-port syntax; the written \
         concept is the conjugated port definition, which is carded",
    ),
    (
        "PayloadFeature",
        ResidualDisposition::AbstractOnly,
        "the payload feature of accept/send action nodes; the written concept is \
         the action node, which is carded — the payload feature is a fragment",
    ),
    (
        "MetadataFeature",
        ResidualDisposition::AbstractOnly,
        "a metadata feature fragment; the written concept is the metadata usage / \
         definition, which is carded",
    ),
    // Expression metaclasses whose concrete notation lowers to a generic
    // OperatorExpression/InvocationExpression in our engine; the written forms
    // (operators, `[]` index, `.` collect/select, chains) are carded via the
    // operator/expression batch, and no distinct subtype kind is materialized.
    (
        "CollectExpression",
        ResidualDisposition::AbstractOnly,
        "the collect-expression metamodel subtype (`col.collect{...}`); the written \
         form lowers via the carded operator/feature-chain expressions, no distinct \
         subtype kind is materialized",
    ),
    (
        "SelectExpression",
        ResidualDisposition::AbstractOnly,
        "the select-expression metamodel subtype (`col.select{...}`); the written \
         form lowers via the carded operator expressions, no distinct subtype kind",
    ),
    (
        "IndexExpression",
        ResidualDisposition::AbstractOnly,
        "the index-expression metamodel subtype (`seq#(i)`); the written form lowers \
         via the carded operator expressions, no distinct subtype kind",
    ),
    (
        "InstantiationExpression",
        ResidualDisposition::AbstractOnly,
        "the abstract instantiation-expression base of invocation/constructor \
         expressions; the concrete invocation form is carded",
    ),
    (
        "FeatureChainExpression",
        ResidualDisposition::AbstractOnly,
        "the feature-chain-expression metamodel subtype; the written feature \
         reference / chain forms are carded via the expression batch",
    ),
    (
        "TriggerInvocationExpression",
        ResidualDisposition::AbstractOnly,
        "the trigger-invocation-expression metamodel subtype behind an accept \
         trigger (`at`/`after`/`when`); the written trigger form folds into the \
         carded accept/transition concepts",
    ),
];

/// The tooling known-gap cards a residual `KnownGapFold` may target, so a fold is
/// rejected at generation time if the target card was not minted.
fn residual_disposition(name: &str) -> Option<(&'static ResidualDisposition, &'static str)> {
    RESIDUAL_METAMODEL
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, d, r)| (d, *r))
}

// --- Integration: records + card enrichment -------------------------------

/// The result of folding the metamodel/SHACL surface into the pack.
pub struct Integration {
    /// One reviewed denominator record per metamodel class + per SHACL shape.
    pub records: Vec<DenominatorRecord>,
    /// Per-source disposition tallies (for the completeness report).
    pub metamodel_disposition: BTreeMap<String, usize>,
    pub shacl_disposition: BTreeMap<String, usize>,
    /// The raw OSLC ResourceShape declaration count that collapsed into the
    /// distinct constrained element types (auditable dedup, mirrors the metamodel
    /// KerML/SysML re-declaration dedup).
    pub raw_shape_declarations: usize,
}

/// The disposition of one metaclass, resolved once and shared by its metamodel
/// record, its SHACL-shape record, and the facet enrichment.
enum Decision {
    /// Fold into an existing concept card (grammar-carded) and enrich its facet.
    FoldEnrich { card_id: String, fixed_semtype: bool },
    /// A newly-minted metamodel-only card (memberships).
    Carded { card_id: String },
    /// Fold into a tooling known-gap card (written form, non-distinct lowering).
    KnownGap { tooling_card: String },
    /// Abstract-only / metamodel enumeration; a reviewed exclusion, never a card.
    Abstract,
}

/// Deterministic owner card for a metaclass: among cards whose `semantic_types`
/// names it, prefer the one whose id slug equals the kebab-cased metaclass (the
/// concept card, not a multi-type validation card); otherwise the smallest id.
fn owner_card(metaclass: &str, cards: &[LanguageCard]) -> Option<String> {
    let slug = slugify(metaclass);
    let mut owners: Vec<&str> = cards
        .iter()
        .filter(|c| c.semantic_types.iter().any(|s| s == metaclass))
        .map(|c| c.id.as_str())
        .collect();
    owners.sort();
    owners
        .iter()
        .find(|id| id.rsplit('.').next() == Some(slug.as_str()))
        .or_else(|| owners.first())
        .map(|s| (*s).to_owned())
}

/// A card whose id slug equals the kebab-cased metaclass even though it did not
/// list the metaclass in `semantic_types` (the omission is fixed on fold).
fn slug_card(metaclass: &str, cards: &[LanguageCard]) -> Option<String> {
    let slug = slugify(metaclass);
    let mut hits: Vec<&str> = cards
        .iter()
        .filter(|c| c.id.rsplit('.').next() == Some(slug.as_str()))
        .map(|c| c.id.as_str())
        .collect();
    hits.sort();
    hits.first().map(|s| (*s).to_owned())
}

/// Fold the metamodel + SHACL surface into the pack: enrich the concept cards
/// that lower to each metaclass with its metamodel facet, mint the reviewed
/// metamodel-only cards (memberships), and emit one reviewed denominator record
/// per metamodel class (182) and per distinct constrained element type (the 257
/// raw OSLC shape declarations dedup to 175 distinct constrained types, mirroring
/// the metamodel's KerML/SysML re-declaration dedup). Replaces the opaque
/// `block`-mapped slots the review flagged (metamodel 0/182, shacl 0/257).
pub fn integrate(
    idx: &MetamodelIndex,
    cards: &mut Vec<LanguageCard>,
    kerml_headings: &BTreeMap<String, String>,
    repo_root: &std::path::Path,
) -> Result<Integration, LpError> {
    // Fail-hard: every metamodel-only (residual) class must be reviewed. A new
    // class in a future metamodel drop that is neither carded/folded nor in the
    // residual table is a hard error, never a silent omission.
    let carded_metaclasses: std::collections::BTreeSet<String> = cards
        .iter()
        .flat_map(|c| c.semantic_types.iter().cloned())
        .collect();
    for name in idx.classes.keys() {
        let owned = carded_metaclasses.contains(name);
        let slugged = slug_card(name, cards).is_some();
        let reviewed = residual_disposition(name).is_some();
        if !owned && !slugged && !reviewed {
            return Err(LpError::Other(format!(
                "metamodel class {name} is undispositioned: not a card's semantic_types, \
                 no slug-matching card, and not in RESIDUAL_METAMODEL"
            )));
        }
    }

    let card_ids: std::collections::BTreeSet<String> =
        cards.iter().map(|c| c.id.clone()).collect();

    // Membership cards are minted first so they are owners of their own class for
    // the SHACL pass. Provenance = the vocab + shapes TTLs (both allowlisted).
    let mut minted: BTreeMap<String, String> = BTreeMap::new(); // metaclass -> new card id
    for (name, disp, _rat) in RESIDUAL_METAMODEL {
        if let ResidualDisposition::Card { facet, clause } = disp {
            let card = build_metamodel_card(idx, name, facet, clause, kerml_headings, repo_root)?;
            minted.insert((*name).to_owned(), card.id.clone());
            cards.push(card);
        }
    }

    let mut records: Vec<DenominatorRecord> = Vec::new();
    let mut metamodel_disposition: BTreeMap<String, usize> = BTreeMap::new();
    let mut shacl_disposition: BTreeMap<String, usize> = BTreeMap::new();

    // Resolve every metaclass's decision once (metamodel + SHACL share it).
    let mut decisions: BTreeMap<String, Decision> = BTreeMap::new();
    for name in idx.classes.keys() {
        let decision = decide(name, cards, &minted, &card_ids)?;
        decisions.insert(name.clone(), decision);
    }

    // Enrich cards (fold-enrich owners/slug-cards; memberships already carry a
    // facet from minting). Deterministic: idx.classes is sorted, first owner wins
    // a card's single facet slot.
    for (name, decision) in &decisions {
        if let Decision::FoldEnrich { card_id, fixed_semtype } = decision {
            if let Some(card) = cards.iter_mut().find(|c| &c.id == card_id) {
                if *fixed_semtype && !card.semantic_types.iter().any(|s| s == name) {
                    card.semantic_types.push(name.clone());
                    card.semantic_types.sort();
                    card.semantic_types.dedup();
                }
                if card.metamodel_facet.is_none() {
                    card.metamodel_facet = idx.facet_for(name);
                }
            }
        }
    }

    // Metamodel records (182).
    for (name, decision) in &decisions {
        let (mapping, target, classification, kind) = record_shape(decision, name);
        *metamodel_disposition.entry(kind.to_owned()).or_insert(0) += 1;
        let vocab_path = match idx.classes.get(name) {
            Some(info) if info.authority == "KerML" => idx.kerml_vocab,
            _ => idx.sysml_vocab,
        };
        records.push(source_record(
            "metamodel",
            name,
            &format!("{vocab_path}#{name}"),
            classification,
            mapping,
            target,
            &metamodel_rationale(decision, name),
        ));
    }

    // SHACL records: one per distinct constrained element type (the OSLC shape).
    // Shares the metaclass decision; a shape for a type with no metaclass is a
    // reviewed exclusion.
    let mut shape_types: Vec<&String> = idx.shapes.keys().collect();
    shape_types.sort();
    for et in shape_types {
        let Some(shape) = idx.shapes.get(et) else { continue };
        let shapes_path = if idx
            .classes
            .get(et)
            .map(|c| c.authority == "KerML")
            .unwrap_or(false)
        {
            idx.kerml_shapes
        } else {
            idx.sysml_shapes
        };
        let pointer = format!("{shapes_path}#{}", shape.shape_name);
        let (mapping, target, classification, kind, rationale) = match decisions.get(et) {
            Some(decision) => {
                let (mut m, t, mut c, mut k) = record_shape(decision, et);
                // A SHACL shape never MINTS a card — it is a constraint bundle that
                // folds onto the concept card. When the concept was carded as a
                // metamodel-only card (memberships), the shape still folds in.
                if m == Mapping::Card {
                    m = Mapping::Duplicate;
                    c = Classification::Excluded;
                    k = "fold-enriched";
                }
                (m, t, c, k, shacl_rationale(decision, et, &shape.shape_name))
            }
            None => (
                Mapping::Exclusion,
                None,
                Classification::AbstractOnly,
                "abstract-only",
                format!(
                    "OSLC property-constraint shape {} for the metamodel type {et}, which has no \
                     distinct written concept (an abstract/non-vocabulary type); reviewed \
                     exclusion — its constraints apply through the concrete carded subtypes",
                    shape.shape_name
                ),
            ),
        };
        *shacl_disposition.entry(kind.to_owned()).or_insert(0) += 1;
        records.push(source_record(
            "shacl", et, &pointer, classification, mapping, target, &rationale,
        ));
    }

    let raw_shape_declarations = 257; // 82 KerML + 175 SysML ResourceShape decls.
    Ok(Integration {
        records,
        metamodel_disposition,
        shacl_disposition,
        raw_shape_declarations,
    })
}

/// Resolve a metaclass's decision.
fn decide(
    name: &str,
    cards: &[LanguageCard],
    minted: &BTreeMap<String, String>,
    card_ids: &std::collections::BTreeSet<String>,
) -> Result<Decision, LpError> {
    if let Some(id) = minted.get(name) {
        return Ok(Decision::Carded { card_id: id.clone() });
    }
    if let Some(id) = owner_card(name, cards) {
        return Ok(Decision::FoldEnrich { card_id: id, fixed_semtype: false });
    }
    if let Some(id) = slug_card(name, cards) {
        return Ok(Decision::FoldEnrich { card_id: id, fixed_semtype: true });
    }
    match residual_disposition(name) {
        Some((ResidualDisposition::KnownGapFold { tooling_card }, _)) => {
            if !card_ids.contains(*tooling_card) {
                return Err(LpError::Other(format!(
                    "metamodel {name} folds into known-gap card {tooling_card}, which was not minted"
                )));
            }
            Ok(Decision::KnownGap { tooling_card: (*tooling_card).to_owned() })
        }
        Some((ResidualDisposition::AbstractOnly, _)) => Ok(Decision::Abstract),
        Some((ResidualDisposition::Card { .. }, _)) => {
            // Handled by the minting pass; if we reach here the card is missing.
            Err(LpError::Other(format!("metamodel card {name} was not minted")))
        }
        None => Err(LpError::Other(format!("metamodel {name} has no disposition"))),
    }
}

/// (mapping, mapping_target, classification, disposition-kind) for a decision.
fn record_shape(
    decision: &Decision,
    _name: &str,
) -> (Mapping, Option<String>, Classification, &'static str) {
    match decision {
        Decision::Carded { card_id } => (
            Mapping::Card,
            Some(card_id.clone()),
            Classification::SemanticOnly,
            "carded",
        ),
        Decision::FoldEnrich { card_id, .. } => (
            Mapping::Duplicate,
            Some(card_id.clone()),
            Classification::Excluded, // "inherited-duplicate" via schema below
            "fold-enriched",
        ),
        Decision::KnownGap { tooling_card } => (
            Mapping::Duplicate,
            Some(tooling_card.clone()),
            Classification::Excluded,
            "known-gap-fold",
        ),
        Decision::Abstract => (Mapping::Exclusion, None, Classification::AbstractOnly, "abstract-only"),
    }
}

fn metamodel_rationale(decision: &Decision, name: &str) -> String {
    match decision {
        Decision::Carded { card_id } => format!(
            "metamodel-only concept carded as {card_id} (no dedicated concrete syntax but \
             load-bearing for the model graph); enriched with its inheritance + property \
             constraints from the vocabulary + shapes TTLs"
        ),
        Decision::FoldEnrich { card_id, fixed_semtype } => {
            let fix = if *fixed_semtype {
                " (the card's semantic_types omitted this metaclass; the omission is fixed on fold)"
            } else {
                ""
            };
            format!(
                "the {name} metaclass is the metamodel facet of the carded concept {card_id}{fix}; \
                 folded onto that ONE card and enriched with inheritance + owned/reference \
                 properties + multiplicities + SHACL constraints, never given a duplicate \
                 grammar+metamodel card"
            )
        }
        Decision::KnownGap { tooling_card } => residual_disposition(name)
            .map(|(_, r)| r.to_owned())
            .unwrap_or_else(|| format!("folds into known-gap card {tooling_card}")),
        Decision::Abstract => residual_disposition(name)
            .map(|(_, r)| r.to_owned())
            .unwrap_or_else(|| format!("abstract-only metamodel class {name}")),
    }
}

fn shacl_rationale(decision: &Decision, et: &str, shape_name: &str) -> String {
    match decision {
        Decision::Carded { card_id } | Decision::FoldEnrich { card_id, .. } => format!(
            "the OSLC shape {shape_name} is the property-constraint bundle for {et}; its \
             constraints are materialized as the metamodel_facet.properties of {card_id} \
             (multiplicities, reference endpoints, read-only flags) — folded there, not a card"
        ),
        Decision::KnownGap { tooling_card } => format!(
            "the OSLC shape {shape_name} constrains {et}, whose written form our lowering does not \
             materialize as a distinct kind; folds into the tooling limitation card {tooling_card}"
        ),
        Decision::Abstract => format!(
            "the OSLC shape {shape_name} constrains the abstract/enumeration metamodel type {et}; \
             its constraints apply through the concrete carded subtypes — reviewed exclusion"
        ),
    }
}

/// Build a metamodel/SHACL source-concept denominator record. The `classification`
/// string is refined here: fold/known-gap use `inherited-duplicate`, which the
/// schema enumerates and which reads correctly for a metamodel view collapsing
/// into an existing concept.
fn source_record(
    source_kind: &str,
    name: &str,
    pointer: &str,
    classification: Classification,
    mapping: Mapping,
    target: Option<String>,
    rationale: &str,
) -> DenominatorRecord {
    // Map the coarse Classification to the schema string, preferring
    // `inherited-duplicate` for a fold (a metamodel/SHACL view of an existing
    // concept) over the generic `excluded`.
    let classification_str = match (mapping, classification) {
        (Mapping::Duplicate, _) => "inherited-duplicate",
        (_, c) => c.as_str(),
    };
    let normalized_concept_id = if mapping == Mapping::Card { target.clone() } else { None };
    let mapping_target = if mapping == Mapping::Card { None } else { target };
    DenominatorRecord {
        source_id: format!("{source_kind}:{name}"),
        source_kind: source_kind.to_owned(),
        raw_name: name.to_owned(),
        normalized_concept_id,
        classification: classification_str.to_owned(),
        classification_rationale: rationale.to_owned(),
        review_state: "reviewed".to_owned(),
        source_pointer: pointer.to_owned(),
        mapping: mapping.as_str().to_owned(),
        mapping_target,
        merged_from: Vec::new(),
    }
}

/// Mint a metamodel-only concept card (memberships) with its facet.
fn build_metamodel_card(
    idx: &MetamodelIndex,
    metaclass: &str,
    facet: &str,
    clause: &str,
    kerml_headings: &BTreeMap<String, String>,
    repo_root: &std::path::Path,
) -> Result<LanguageCard, LpError> {
    let id = format!("kerml.{facet}.{}", slugify(metaclass));
    let (clause_str, anchor, resolution) =
        citations::resolve_clause_or_ancestor(kerml_headings, clause)?;
    let mm_facet = idx.facet_for(metaclass);
    // Provenance: the KerML vocab + shapes (both allowlisted, hashed).
    let source_paths: Vec<String> = vec![
        idx.kerml_vocab.to_owned(),
        idx.kerml_shapes.to_owned(),
    ];
    let source_hashes: Vec<String> = source_paths
        .iter()
        .map(|p| manifest::source_hash(repo_root, p))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LanguageCard {
        schema_version: super::cards::SCHEMA_VERSION.to_owned(),
        id,
        title: metaclass.to_owned(),
        language: "KerML".to_owned(),
        category: vec![facet.to_owned()],
        summary: format!(
            "The {metaclass} metamodel relationship (KerML abstract syntax). It has no dedicated \
             concrete notation — it is how the model graph is wired — so it is documented as a \
             metamodel concept with its inheritance and property constraints, not a grammar card."
        ),
        keywords: vec![metaclass.to_owned(), "membership".to_owned(), "metamodel".to_owned()],
        aliases: vec![format!("metamodel:{metaclass}")],
        normative_rules: Vec::<GrammarRuleRef>::new(),
        normative_clauses: vec![ClauseRef {
            document: "KerML".to_owned(),
            clause: clause_str,
            anchor,
            resolution: resolution.to_owned(),
        }],
        normalized_grammar: None,
        rule_dependencies: Vec::new(),
        semantic_types: vec![metaclass.to_owned()],
        validation_rules: Vec::new(),
        examples: ExamplesRef { positive: Vec::new(), negative: Vec::new(), composed: Vec::new() },
        support: SupportAxes::all_unknown(),
        known_gaps: Vec::new(),
        related_cards: Vec::new(),
        provenance: Provenance {
            spec_drop: manifest::SPEC_DROP.to_owned(),
            source_paths,
            source_hashes,
            generated_by: manifest::GENERATED_BY.to_owned(),
        },
        metamodel_facet: mm_facet,
    })
}

#[cfg(test)]
#[allow(clippy::print_stdout, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn load_index() -> MetamodelIndex {
        let repo = super::super::repo_root();
        let rd = |p: &str| std::fs::read_to_string(repo.join(p)).unwrap();
        MetamodelIndex::load(
            &rd("references/sysmlv2/Kerml-Vocab.ttl"),
            &rd("references/sysmlv2/SysML-vocab.ttl"),
            &rd("references/sysmlv2/KerML-shapes.ttl"),
            &rd("references/sysmlv2/SysML-shapes.ttl"),
            "references/sysmlv2/Kerml-Vocab.ttl",
            "references/sysmlv2/SysML-vocab.ttl",
            "references/sysmlv2/KerML-shapes.ttl",
            "references/sysmlv2/SysML-shapes.ttl",
        )
        .unwrap()
    }

    #[test]
    fn parses_metamodel_and_shapes() {
        if !super::super::fetched_sources_present(&super::super::repo_root()) {
            eprintln!("SKIP: references not fetched (run tools/fetch-references/fetch.sh fetch)");
            return;
        }
        let idx = load_index();
        println!("classes={} shapes={}", idx.classes.len(), idx.shapes.len());
        assert_eq!(idx.classes.len(), 182, "182 unique metamodel classes");
        // PartUsage facet: inheritance + property constraints from its shape.
        let f = idx.facet_for("PartUsage").expect("PartUsage facet");
        println!(
            "PartUsage: authority={} supertypes={:?} shape={:?} props={}",
            f.authority, f.supertypes, f.shacl_shape, f.properties.len()
        );
        assert_eq!(f.authority, "SysML");
        assert!(!f.supertypes.is_empty(), "PartUsage has supertypes");
        assert!(f.shacl_shape.is_some(), "PartUsage has an OSLC shape");
        assert!(!f.properties.is_empty(), "PartUsage shape has properties");
        // A KerML base class resolves to the KerML authority.
        let el = idx.facet_for("Element").expect("Element facet");
        assert_eq!(el.authority, "KerML");
    }
}
