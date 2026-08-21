//! Spec-drop identity, allowlist enforcement, and source hashing.
//! The generator reads **only** the paths in [`allowlist`]; any other
//! path is a hard reject. Every pinned source's SHA-256 is
//! recomputed and cross-checked against `spec-drop.toml`.

use std::path::Path;

use serde::Serialize;

use super::LpError;

/// Spec-drop label copied from `spec-drop.toml [drop].omg_release`.
pub const SPEC_DROP: &str = "2025-04";
/// Metamodel drop label copied from `spec-drop.toml [drop].metamodel_drop`.
pub const METAMODEL_DROP: &str = "20250201";
/// Stable, non-volatile generator identifier embedded in card provenance.
/// Deliberately spec-drop-labelled (not a git OID): the pack is a tracked,
/// regen-diff-gated artifact, so its content must not change on every commit
/// (the same reasoning applies to wall-clock timestamps). The
/// volatile generator commit lives only in the support-evidence derivation.
pub const GENERATED_BY: &str = "tools/spec-index@2025-04";
/// citation-only rule.
pub const LICENSING_MODE: &str = "citation-only";

/// Kind of an allowlisted source (source tiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    SpecDropPin,
    XtextGrammar,
    DerivedXtextIndex,
    DerivedSpecText,
    Ttl,
    Shacl,
    Xmi,
    SemanticRules,
    Obligation,
    StdlibModel,
}

/// The explicit source allowlist. Repo-relative paths only.
/// Anything not in this list is a hard reject in [`assert_allowlisted`] — there
/// is no recursive `.sysml`/`.xtext` discovery.
pub fn allowlist() -> &'static [(&'static str, SourceKind)] {
    &[
        ("references/sysmlv2/spec-drop.toml", SourceKind::SpecDropPin),
        (
            "references/sysmlv2/derived/xtext-rules.toml",
            SourceKind::DerivedXtextIndex,
        ),
        (
            "references/sysmlv2/SysML-v2-Pilot-Implementation/org.omg.kerml.xtext/src/org/omg/kerml/xtext/KerML.xtext",
            SourceKind::XtextGrammar,
        ),
        (
            "references/sysmlv2/SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/src/org/omg/sysml/xtext/SysML.xtext",
            SourceKind::XtextGrammar,
        ),
        (
            "references/sysmlv2/SysML-v2-Pilot-Implementation/org.omg.kerml.expressions.xtext/src/org/omg/kerml/expressions/xtext/KerMLExpressions.xtext",
            SourceKind::XtextGrammar,
        ),
        (
            "references/sysmlv2/derived/SysML-spec-r2025-04.txt",
            SourceKind::DerivedSpecText,
        ),
        (
            "references/sysmlv2/derived/KerML-spec-r2025-04.txt",
            SourceKind::DerivedSpecText,
        ),
        ("references/sysmlv2/Kerml-Vocab.ttl", SourceKind::Ttl),
        ("references/sysmlv2/SysML-vocab.ttl", SourceKind::Ttl),
        ("references/sysmlv2/KerML-shapes.ttl", SourceKind::Shacl),
        ("references/sysmlv2/SysML-shapes.ttl", SourceKind::Shacl),
        ("references/sysmlv2/KerML/20250201/KerML.xmi", SourceKind::Xmi),
        ("references/sysmlv2/SysML/20250201/SysML.xmi", SourceKind::Xmi),
        (
            "crates/lang/codegen/src/semantic_rules.toml",
            SourceKind::SemanticRules,
        ),
        // Tier 5 obligation matrices. Enumerated explicitly
        // (no glob discovery per §4) so every consumed file is named; consumed
        // only after re-validation against the current gates, never
        // as prose authority.
        (OBLIGATION_ACTIONS, SourceKind::Obligation),
        (OBLIGATION_CALCULATIONS, SourceKind::Obligation),
        (OBLIGATION_CONSTRAINTS, SourceKind::Obligation),
        (OBLIGATION_FLOWS_PORTS, SourceKind::Obligation),
        (OBLIGATION_OCCURRENCES, SourceKind::Obligation),
        (OBLIGATION_ODE_PHYSICS, SourceKind::Obligation),
        (OBLIGATION_REQUIREMENTS, SourceKind::Obligation),
        (OBLIGATION_STATE_MACHINES, SourceKind::Obligation),
        (OBLIGATION_STRUCTURAL, SourceKind::Obligation),
        (OBLIGATION_VERIFICATION, SourceKind::Obligation),
        // Tier 4 normative library semantics. The *meaning* of
        // VerdictKind / RequirementCheck / ConstraintCheck / the case machinery
        // lives in these models (root CLAUDE.md precedence 2: the library models
        // ARE the semantics). Reached only from these named files, never by a
        // `sysml.library` glob. Adding a file here re-keys the evidence epoch.
        (STDLIB_VERIFICATION_CASES, SourceKind::StdlibModel),
        (STDLIB_CASES, SourceKind::StdlibModel),
        (STDLIB_ANALYSIS_CASES, SourceKind::StdlibModel),
        (STDLIB_REQUIREMENTS, SourceKind::StdlibModel),
        (STDLIB_CONSTRAINTS, SourceKind::StdlibModel),
        (STDLIB_PERFORMANCES, SourceKind::StdlibModel),
        // The three obligation-home library files (un-blocking the
        // calc-default-param / incoming-transition-trigger / state-sequencing
        // obligations, whose normative home is a standard-library symbol).
        (STDLIB_CALCULATIONS, SourceKind::StdlibModel),
        (STDLIB_STATES, SourceKind::StdlibModel),
        (STDLIB_STATE_PERFORMANCES, SourceKind::StdlibModel),
        // Member-level library cards for the highest-retrieval-value
        // constructs an LLM reaches for when authoring library-heavy models —
        // scalar/sequence functions (expressions), transfers (flows),
        // occurrences (timing), and the SI/ISQ quantity-and-unit anchors. Each
        // addition re-keys the evidence epoch (see the module note above).
        (STDLIB_SCALAR_FUNCTIONS, SourceKind::StdlibModel),
        (STDLIB_SEQUENCE_FUNCTIONS, SourceKind::StdlibModel),
        (STDLIB_TRANSFERS, SourceKind::StdlibModel),
        (STDLIB_OCCURRENCES, SourceKind::StdlibModel),
        (STDLIB_SI, SourceKind::StdlibModel),
        (STDLIB_ISQ_BASE, SourceKind::StdlibModel),
    ]
}

pub const OBLIGATION_ACTIONS: &str = "crates/testing/sysml-spec-tests/spec-obligations/actions.md";
pub const OBLIGATION_CALCULATIONS: &str =
    "crates/testing/sysml-spec-tests/spec-obligations/calculations.md";
pub const OBLIGATION_CONSTRAINTS: &str =
    "crates/testing/sysml-spec-tests/spec-obligations/constraints-expressions.md";
pub const OBLIGATION_FLOWS_PORTS: &str =
    "crates/testing/sysml-spec-tests/spec-obligations/flows-ports.md";
pub const OBLIGATION_OCCURRENCES: &str =
    "crates/testing/sysml-spec-tests/spec-obligations/occurrences-clocks.md";
pub const OBLIGATION_ODE_PHYSICS: &str =
    "crates/testing/sysml-spec-tests/spec-obligations/ode-physics.md";
pub const OBLIGATION_REQUIREMENTS: &str =
    "crates/testing/sysml-spec-tests/spec-obligations/requirements.md";
pub const OBLIGATION_STATE_MACHINES: &str =
    "crates/testing/sysml-spec-tests/spec-obligations/state-machines.md";
pub const OBLIGATION_STRUCTURAL: &str =
    "crates/testing/sysml-spec-tests/spec-obligations/structural.md";
pub const OBLIGATION_VERIFICATION: &str =
    "crates/testing/sysml-spec-tests/spec-obligations/verification-analysis-cases.md";

// Tier 4 normative standard-library files. The pilot
// implementation's Systems Library + Kernel Semantic Library. Paths contain
// spaces exactly as they exist on disk.
pub const STDLIB_VERIFICATION_CASES: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/VerificationCases.sysml";
pub const STDLIB_CASES: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Cases.sysml";
pub const STDLIB_ANALYSIS_CASES: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/AnalysisCases.sysml";
pub const STDLIB_REQUIREMENTS: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Requirements.sysml";
pub const STDLIB_CONSTRAINTS: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Constraints.sysml";
pub const STDLIB_PERFORMANCES: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Performances.kerml";
pub const STDLIB_CALCULATIONS: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Calculations.sysml";
pub const STDLIB_STATES: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/States.sysml";
pub const STDLIB_STATE_PERFORMANCES: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/StatePerformances.kerml";
// Member-level card sources.
pub const STDLIB_SCALAR_FUNCTIONS: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/ScalarFunctions.kerml";
pub const STDLIB_SEQUENCE_FUNCTIONS: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/SequenceFunctions.kerml";
pub const STDLIB_TRANSFERS: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Transfers.kerml";
pub const STDLIB_OCCURRENCES: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Occurrences.kerml";
pub const STDLIB_SI: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/SI.sysml";
pub const STDLIB_ISQ_BASE: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQBase.sysml";

/// The explicit aggregate-root manifest of the standard model library:
/// every `standard library package` in the pinned pilot
/// implementation's `sysml.library` tree, `(group, package, repo-relative path)`.
/// Enumerated explicitly (NOT discovered by a `sysml.library` glob) so the
/// reviewed library denominator is grounded, deterministic, and auditable. This
/// is the aggregate-root granularity: each package hosts many member concepts;
/// the load-bearing ones are carded selectively ([`super::stdlib`]).
pub const STDLIB_LIBRARY_PACKAGES: &[(&str, &str, &str)] = &[
    ("Domain Libraries", "AnalysisTooling", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Analysis/AnalysisTooling.sysml"),
    ("Domain Libraries", "CausationConnections", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Cause and Effect/CausationConnections.sysml"),
    ("Domain Libraries", "CauseAndEffect", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Cause and Effect/CauseAndEffect.sysml"),
    ("Domain Libraries", "DerivationConnections", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Requirement Derivation/DerivationConnections.sysml"),
    ("Domain Libraries", "ISQ", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQ.sysml"),
    ("Domain Libraries", "ISQAcoustics", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQAcoustics.sysml"),
    ("Domain Libraries", "ISQAtomicNuclear", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQAtomicNuclear.sysml"),
    ("Domain Libraries", "ISQBase", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQBase.sysml"),
    ("Domain Libraries", "ISQCharacteristicNumbers", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQCharacteristicNumbers.sysml"),
    ("Domain Libraries", "ISQChemistryMolecular", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQChemistryMolecular.sysml"),
    ("Domain Libraries", "ISQCondensedMatter", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQCondensedMatter.sysml"),
    ("Domain Libraries", "ISQElectromagnetism", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQElectromagnetism.sysml"),
    ("Domain Libraries", "ISQInformation", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQInformation.sysml"),
    ("Domain Libraries", "ISQLight", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQLight.sysml"),
    ("Domain Libraries", "ISQMechanics", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQMechanics.sysml"),
    ("Domain Libraries", "ISQSpaceTime", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQSpaceTime.sysml"),
    ("Domain Libraries", "ISQThermodynamics", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/ISQThermodynamics.sysml"),
    ("Domain Libraries", "ImageMetadata", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Metadata/ImageMetadata.sysml"),
    ("Domain Libraries", "MeasurementRefCalculations", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/MeasurementRefCalculations.sysml"),
    ("Domain Libraries", "MeasurementReferences", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/MeasurementReferences.sysml"),
    ("Domain Libraries", "ModelingMetadata", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Metadata/ModelingMetadata.sysml"),
    ("Domain Libraries", "ParametersOfInterestMetadata", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Metadata/ParametersOfInterestMetadata.sysml"),
    ("Domain Libraries", "Quantities", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/Quantities.sysml"),
    ("Domain Libraries", "QuantityCalculations", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/QuantityCalculations.sysml"),
    ("Domain Libraries", "RequirementDerivation", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Requirement Derivation/RequirementDerivation.sysml"),
    ("Domain Libraries", "RiskMetadata", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Metadata/RiskMetadata.sysml"),
    ("Domain Libraries", "SI", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/SI.sysml"),
    ("Domain Libraries", "SIPrefixes", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/SIPrefixes.sysml"),
    ("Domain Libraries", "SampledFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Analysis/SampledFunctions.sysml"),
    ("Domain Libraries", "ShapeItems", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Geometry/ShapeItems.sysml"),
    ("Domain Libraries", "SpatialItems", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Geometry/SpatialItems.sysml"),
    ("Domain Libraries", "StateSpaceRepresentation", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Analysis/StateSpaceRepresentation.sysml"),
    ("Domain Libraries", "TensorCalculations", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/TensorCalculations.sysml"),
    ("Domain Libraries", "Time", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/Time.sysml"),
    ("Domain Libraries", "TradeStudies", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Analysis/TradeStudies.sysml"),
    ("Domain Libraries", "VectorCalculations", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Domain Libraries/Quantities and Units/VectorCalculations.sysml"),
    ("Kernel Libraries", "Base", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Base.kerml"),
    ("Kernel Libraries", "BaseFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/BaseFunctions.kerml"),
    ("Kernel Libraries", "BooleanFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/BooleanFunctions.kerml"),
    ("Kernel Libraries", "Clocks", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Clocks.kerml"),
    ("Kernel Libraries", "CollectionFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/CollectionFunctions.kerml"),
    ("Kernel Libraries", "Collections", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Data Type Library/Collections.kerml"),
    ("Kernel Libraries", "ComplexFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/ComplexFunctions.kerml"),
    ("Kernel Libraries", "ControlFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/ControlFunctions.kerml"),
    ("Kernel Libraries", "ControlPerformances", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/ControlPerformances.kerml"),
    ("Kernel Libraries", "DataFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/DataFunctions.kerml"),
    ("Kernel Libraries", "FeatureReferencingPerformances", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/FeatureReferencingPerformances.kerml"),
    ("Kernel Libraries", "IntegerFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/IntegerFunctions.kerml"),
    ("Kernel Libraries", "KerML", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/KerML.kerml"),
    ("Kernel Libraries", "Links", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Links.kerml"),
    ("Kernel Libraries", "Metaobjects", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Metaobjects.kerml"),
    ("Kernel Libraries", "NaturalFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/NaturalFunctions.kerml"),
    ("Kernel Libraries", "NumericalFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/NumericalFunctions.kerml"),
    ("Kernel Libraries", "Objects", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Objects.kerml"),
    ("Kernel Libraries", "Observation", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Observation.kerml"),
    ("Kernel Libraries", "OccurrenceFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/OccurrenceFunctions.kerml"),
    ("Kernel Libraries", "Occurrences", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Occurrences.kerml"),
    ("Kernel Libraries", "Performances", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Performances.kerml"),
    ("Kernel Libraries", "RationalFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/RationalFunctions.kerml"),
    ("Kernel Libraries", "RealFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/RealFunctions.kerml"),
    ("Kernel Libraries", "ScalarFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/ScalarFunctions.kerml"),
    ("Kernel Libraries", "ScalarValues", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Data Type Library/ScalarValues.kerml"),
    ("Kernel Libraries", "SequenceFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/SequenceFunctions.kerml"),
    ("Kernel Libraries", "SpatialFrames", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/SpatialFrames.kerml"),
    ("Kernel Libraries", "StatePerformances", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/StatePerformances.kerml"),
    ("Kernel Libraries", "StringFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/StringFunctions.kerml"),
    ("Kernel Libraries", "Transfers", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Transfers.kerml"),
    ("Kernel Libraries", "TransitionPerformances", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/TransitionPerformances.kerml"),
    ("Kernel Libraries", "TrigFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/TrigFunctions.kerml"),
    ("Kernel Libraries", "Triggers", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Semantic Library/Triggers.kerml"),
    ("Kernel Libraries", "VectorFunctions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Function Library/VectorFunctions.kerml"),
    ("Kernel Libraries", "VectorValues", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Kernel Libraries/Kernel Data Type Library/VectorValues.kerml"),
    ("Systems Library", "Actions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Actions.sysml"),
    ("Systems Library", "Allocations", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Allocations.sysml"),
    ("Systems Library", "AnalysisCases", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/AnalysisCases.sysml"),
    ("Systems Library", "Attributes", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Attributes.sysml"),
    ("Systems Library", "Calculations", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Calculations.sysml"),
    ("Systems Library", "Cases", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Cases.sysml"),
    ("Systems Library", "Connections", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Connections.sysml"),
    ("Systems Library", "Constraints", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Constraints.sysml"),
    ("Systems Library", "Flows", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Flows.sysml"),
    ("Systems Library", "Interfaces", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Interfaces.sysml"),
    ("Systems Library", "Items", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Items.sysml"),
    ("Systems Library", "Metadata", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Metadata.sysml"),
    ("Systems Library", "Parts", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Parts.sysml"),
    ("Systems Library", "Ports", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Ports.sysml"),
    ("Systems Library", "Requirements", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Requirements.sysml"),
    ("Systems Library", "StandardViewDefinitions", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/StandardViewDefinitions.sysml"),
    ("Systems Library", "States", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/States.sysml"),
    ("Systems Library", "SysML", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/SysML.sysml"),
    ("Systems Library", "UseCases", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/UseCases.sysml"),
    ("Systems Library", "VerificationCases", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/VerificationCases.sysml"),
    ("Systems Library", "Views", "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/Systems Library/Views.sysml"),
];

/// The ten allowlisted obligation-matrix area files (Tier 5).
pub const OBLIGATION_FILES: &[&str] = &[
    OBLIGATION_ACTIONS,
    OBLIGATION_CALCULATIONS,
    OBLIGATION_CONSTRAINTS,
    OBLIGATION_FLOWS_PORTS,
    OBLIGATION_OCCURRENCES,
    OBLIGATION_ODE_PHYSICS,
    OBLIGATION_REQUIREMENTS,
    OBLIGATION_STATE_MACHINES,
    OBLIGATION_STRUCTURAL,
    OBLIGATION_VERIFICATION,
];

/// The allowlisted kind of `rel`, or `None` if it is not allowlisted.
pub fn allowlisted_kind(rel: &str) -> Option<SourceKind> {
    allowlist()
        .iter()
        .find(|(p, _)| *p == rel)
        .map(|(_, k)| *k)
}

/// Hard-reject any non-allowlisted path.
pub fn assert_allowlisted(rel: &str) -> Result<SourceKind, LpError> {
    allowlisted_kind(rel).ok_or_else(|| LpError::NotAllowlisted(rel.to_owned()))
}

/// Lowercase-hex SHA-256 of an allowlisted source file.
pub fn source_hash(repo_root: &Path, rel: &str) -> Result<String, LpError> {
    assert_allowlisted(rel)?;
    let path = repo_root.join(rel);
    let bytes = std::fs::read(&path)
        .map_err(|e| LpError::Io(format!("read {}: {e}", path.display())))?;
    Ok(crate::sha256_hex(&bytes))
}

/// The `sha256 = "..."` pin recorded for `rel` in `spec-drop.toml`, if any.
/// Parsed leniently (the full TOML shape is validated by the existing
/// `spec_drop_manifest` gate), mirroring `derived_indexes.rs`.
pub fn spec_drop_pin(repo_root: &Path, rel: &str) -> Option<String> {
    // spec-drop.toml keys are relative to references/sysmlv2/, so strip that.
    let key = rel.strip_prefix("references/sysmlv2/")?;
    let manifest = std::fs::read_to_string(repo_root.join("references/sysmlv2/spec-drop.toml")).ok()?;
    let mut current: Option<String> = None;
    for line in manifest.lines() {
        if let Some(rest) = line.strip_prefix("path = ") {
            current = Some(rest.trim().trim_matches('"').to_owned());
        } else if let Some(rest) = line.strip_prefix("sha256 = ") {
            if current.as_deref() == Some(key) {
                return Some(rest.trim().trim_matches('"').to_owned());
            }
        }
    }
    None
}

/// Verify an allowlisted source's hash against its `spec-drop.toml` pin. A
/// mismatch is a hard failure — the same posture as the `spec_drop_manifest`
/// gate.
pub fn verify_pinned_hash(repo_root: &Path, rel: &str) -> Result<String, LpError> {
    let actual = source_hash(repo_root, rel)?;
    if let Some(expected) = spec_drop_pin(repo_root, rel) {
        if actual != expected {
            return Err(LpError::HashMismatch {
                path: rel.to_owned(),
                expected,
                actual,
            });
        }
    }
    Ok(actual)
}

/// Verify an explicit expected hash for `rel` (used by AC1's negative test:
/// a wrong expected hash aborts hard).
pub fn verify_hash_against(
    repo_root: &Path,
    rel: &str,
    expected: &str,
) -> Result<(), LpError> {
    let actual = source_hash(repo_root, rel)?;
    if actual != expected {
        return Err(LpError::HashMismatch {
            path: rel.to_owned(),
            expected: expected.to_owned(),
            actual,
        });
    }
    Ok(())
}

/// One consumed source in the manifest.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestSource {
    pub path: String,
    pub sha256: String,
    pub kind: SourceKind,
}

/// The pack manifest. Content-stable: no git commit and
/// no wall-clock timestamp, so the committed pack is regen-diff-gatable across
/// commits. The volatile generator commit lives only in support evidence.
#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    pub spec_drop: String,
    pub metamodel_drop: String,
    pub generator_version: String,
    pub licensing_mode: String,
    pub sources: Vec<ManifestSource>,
}

/// Resolve the manifest: verify every allowlisted source's hash against its
/// `spec-drop.toml` pin (hard fail on mismatch), and record the recomputed
/// hashes in stable path order.
pub fn resolve_manifest(repo_root: &Path) -> Result<Manifest, LpError> {
    let mut sources = Vec::new();
    for (rel, kind) in allowlist() {
        if *kind == SourceKind::SpecDropPin {
            continue; // the pin file itself is identity, not a consumed source
        }
        let sha256 = verify_pinned_hash(repo_root, rel)?;
        sources.push(ManifestSource {
            path: (*rel).to_owned(),
            sha256,
            kind: *kind,
        });
    }
    sources.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Manifest {
        spec_drop: SPEC_DROP.to_owned(),
        metamodel_drop: METAMODEL_DROP.to_owned(),
        generator_version: "1".to_owned(),
        licensing_mode: LICENSING_MODE.to_owned(),
        sources,
    })
}
