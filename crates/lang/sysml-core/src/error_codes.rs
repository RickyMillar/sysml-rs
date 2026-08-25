//! Error code registry for SysML diagnostics.
//!
//! Central mapping of all diagnostic codes to descriptions.

/// Category of a diagnostic error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Structural integrity errors (E-series).
    Structural,
    /// Name resolution errors (R-series, E200).
    Resolution,
    /// Semantic validation errors (S-series).
    Semantic,
    /// Property validation errors (V-series).
    Validation,
}

/// Information about a diagnostic error code.
#[derive(Debug, Clone)]
pub struct ErrorCodeInfo {
    /// The error code string (e.g., "E001").
    pub code: &'static str,
    /// A short human-readable description.
    pub short_description: &'static str,
    /// The category this code belongs to.
    pub category: ErrorCategory,
}

/// Shorthand constructor keeping the registry table readable.
const fn info(
    code: &'static str,
    short_description: &'static str,
    category: ErrorCategory,
) -> ErrorCodeInfo {
    ErrorCodeInfo {
        code,
        short_description,
        category,
    }
}

use ErrorCategory::{Resolution, Semantic, Structural, Validation};

/// Every registered diagnostic error code, in registry order.
///
/// [`lookup`] searches this table; [`all`] exposes it for iteration
/// (e.g. generated reference documentation).
static REGISTRY: &[ErrorCodeInfo] = &[
    // === Structural errors (E-series) ===
    info("E001", "orphan element without an owner", Structural),
    info("E002", "ownership cycle detected", Structural),
    info("E003", "dangling membership reference", Structural),
    info("E004", "relationship source type mismatch", Structural),
    info("E005", "relationship target type mismatch", Structural),
    info("E006", "dangling relationship reference", Structural),
    info("E007", "dangling owning membership reference", Structural),
    info("E008", "invalid owning membership type", Structural),
    // === Resolution errors ===
    info("E200", "unresolved name reference", Resolution),
    info(
        "E201",
        "ambiguous reference (requires qualification)",
        Resolution,
    ),
    // === Import / file-loading hints (IM-series) ===
    // IM001 lives in import_health.rs (kept for backwards compat).
    info(
        "IM010",
        "name not in local scope but defined elsewhere — add import or qualify",
        Resolution,
    ),
    info(
        "IM012",
        "file opened in strict single-file mode; cross-file imports cannot resolve",
        Resolution,
    ),
    // === Semantic errors (S-series) ===
    info("S001", "duplicate member name in namespace", Semantic),
    info(
        "S005",
        "same top-level package name declared in multiple files (workspace)",
        Semantic,
    ),
    info("S015", "invalid typing for usage element", Semantic),
    info(
        "S030",
        "invalid specialization across type boundaries",
        Semantic,
    ),
    info(
        "S041",
        "ReturnParameterMembership in non-function/expression context",
        Semantic,
    ),
    info("S042", "membership in wrong ownership context", Semantic),
    info(
        "S043",
        "SubjectMembership in non-requirement/case context",
        Semantic,
    ),
    info("S044", "ObjectiveMembership in non-case context", Semantic),
    info(
        "S045",
        "ActorMembership in non-requirement/case context",
        Semantic,
    ),
    info(
        "S046",
        "StakeholderMembership in non-requirement context",
        Semantic,
    ),
    info(
        "S047",
        "RequirementConstraintMembership in non-requirement context",
        Semantic,
    ),
    info(
        "S048",
        "ViewRenderingMembership in non-view context",
        Semantic,
    ),
    info(
        "S051",
        "ResultExpressionMembership in non-function/expression context",
        Semantic,
    ),
    info("S060", "member cardinality violation", Semantic),
    info(
        "S066",
        "function has more than one ReturnParameterMembership",
        Semantic,
    ),
    info(
        "S067",
        "expression has more than one ReturnParameterMembership",
        Semantic,
    ),
    info("S068", "state definition has duplicate subaction", Semantic),
    info("S069", "state usage has duplicate subaction", Semantic),
    info(
        "S070",
        "ViewDefinition has more than one ViewRenderingMembership",
        Semantic,
    ),
    info(
        "S071",
        "ViewUsage has more than one ViewRenderingMembership",
        Semantic,
    ),
    // Requirement/case constraint and cardinality rules
    info(
        "S130",
        "RequirementDefinition constraint not composite",
        Semantic,
    ),
    info(
        "S131",
        "RequirementUsage constraint not composite",
        Semantic,
    ),
    info(
        "S132",
        "SatisfyRequirementUsage typed by more than one requirement",
        Semantic,
    ),
    info(
        "S133",
        "ConcernDefinition has more than one SubjectMembership",
        Semantic,
    ),
    info(
        "S134",
        "ConcernUsage has more than one SubjectMembership",
        Semantic,
    ),
    info(
        "S135",
        "VerificationCaseDefinition has more than one SubjectMembership",
        Semantic,
    ),
    info(
        "S136",
        "VerificationCaseUsage has more than one SubjectMembership",
        Semantic,
    ),
    info(
        "S137",
        "AnalysisCaseDefinition has more than one SubjectMembership",
        Semantic,
    ),
    info(
        "S138",
        "AnalysisCaseUsage has more than one SubjectMembership",
        Semantic,
    ),
    info(
        "S139",
        "UseCaseDefinition has more than one SubjectMembership",
        Semantic,
    ),
    info(
        "S140",
        "UseCaseUsage has more than one SubjectMembership",
        Semantic,
    ),
    // === Connector ownership context (S106-S108) ===
    info(
        "S106",
        "connection owned by package instead of type",
        Semantic,
    ),
    info(
        "S107",
        "interface owned by package instead of type",
        Semantic,
    ),
    info("S108", "flow owned by package instead of type", Semantic),
    // === Structural property constraints (S090-S091) ===
    info("S090", "AttributeUsage must not be composite", Semantic),
    info("S091", "AttributeDefinition must not be composite", Semantic),
    // === Physics diagnostics (PH-series) ===
    info("PH001", "domain mismatch on flow connection", Semantic),
    info(
        "PH002",
        "conservation imbalance — all ports same direction",
        Semantic,
    ),
    info(
        "PH003",
        "incomplete physics port — missing effort or flow feature",
        Semantic,
    ),
    info("PH004", "direction conflict on flow connection", Semantic),
    info(
        "PH005",
        "R/C/I element detected but not wired with constraint",
        Semantic,
    ),
    info(
        "PH006",
        "Real-typed attribute could use ISQ type for physics features",
        Semantic,
    ),
    // === Flow / exchange-plane diagnostics (FL-series) ===
    info("FL010", "port type mismatch on flow connection", Semantic),
    info(
        "FL011",
        "target port expects a feature the source does not provide",
        Semantic,
    ),
    info(
        "FL012",
        "conjugation incompatibility on flow connection",
        Semantic,
    ),
    info(
        "FL013",
        "unconnected output port / open terminal",
        Semantic,
    ),
    info("FL014", "direction conflict on flow connection", Semantic),
    info(
        "FL015",
        "port multiplicity detected (informational)",
        Semantic,
    ),
    info(
        "FL016",
        "structural payload incompatibility on flow connection",
        Semantic,
    ),
    info(
        "FL017",
        "link class unresolved — routing as message channel",
        Semantic,
    ),
    info(
        "FL018",
        "transfer between ports not connected by any declared interface or \
         connection (Ports.sysml interfacingPorts constraint)",
        Semantic,
    ),
    info(
        "FL019",
        "transfer direction violation — pick-up at an in-direction port or \
         drop-off into an out-direction port (post-conjugation)",
        Semantic,
    ),
    info(
        "FL020",
        "payload type does not conform to the flow's source-output / \
         target-input typing (Transfers.kerml payload subsetting)",
        Semantic,
    ),
    // === Variability / runtime lints (VR-series) ===
    info(
        "VR001",
        "assignment to configuration attribute (defaulted part attribute) at runtime",
        Semantic,
    ),
    // === Runtime semantic core (RS-series, ADR-017) ===
    info(
        "RS001",
        "multiple runtime writers — two executors claim the same runtime variable slot",
        Semantic,
    ),
    info(
        "RS002",
        "unknown override target — session override names neither a runtime \
         slot alias nor an existing context variable",
        Semantic,
    ),
    info(
        "RS003",
        "unresolved runtime name — expression reference resolves to neither a \
         runtime slot nor a model feature (hard compile error since RSC-2.5)",
        Semantic,
    ),
    info(
        "RS014",
        "time-accurate zero-crossing re-step failed hard — a located crossing could \
         not be re-stepped without silently corrupting the run: the per-tick crossing \
         bound was exceeded, a sub-interval integration did not perform (non-RK45 \
         solver on the re-step path is a Wave-2b deferral), the target state machine \
         is not slot-attached (its mode/drive writeback would be dropped — the L44 raw \
         add_state_machine shape), or a non-crossing due event raced the crossing for \
         the same SM in the same tick (FIFO ordering unresolved)",
        Semantic,
    ),
    // === Quantity / dimensional-conformance errors (UQ-series, RSC-5.2 / D-5.0.8) ===
    info(
        "UQ001",
        "quantity dimension mismatch at a binding connector — endpoints carry \
         incompatible ISQ dimensions (error), or a dimensioned endpoint is bound to \
         an untyped attribute (warning)",
        Semantic,
    ),
    info(
        "UQ002",
        "quantity dimension mismatch across a signal link — the source and target \
         port-feature slots carry incompatible ISQ dimensions, so the boundary has no \
         meaningful conversion (same-dimension scale differences are converted, not \
         flagged)",
        Semantic,
    ),
    info(
        "UQ003",
        "cross-dimension comparison in a constraint expression — an ordering \
         comparison (<, <=, >, >=) between operands with incompatible ISQ dimensions \
         (the static twin of the RSC-5.1b eval-time error)",
        Semantic,
    ),
    info(
        "UQ004",
        "dimensioned argument to a dimensionless-only function — a transcendental \
         (sin, cos, exp, ln, …) requires a pure-number argument",
        Semantic,
    ),
    // === Validation errors (V-series) ===
    info("V001", "missing required property", Validation),
    info("V002", "property has wrong type", Validation),
    info("V003", "property requires at least one value", Validation),
    info("V004", "property allows at most one value", Validation),
    info("V005", "read-only property modified", Validation),
];

/// All registered diagnostic error codes, in registry order.
pub fn all() -> &'static [ErrorCodeInfo] {
    REGISTRY
}

/// Look up information for a diagnostic error code.
pub fn lookup(code: &str) -> Option<ErrorCodeInfo> {
    REGISTRY.iter().find(|info| info.code == code).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_structural_codes() {
        for code in &[
            "E001", "E002", "E003", "E004", "E005", "E006", "E007", "E008",
        ] {
            let info = lookup(code).unwrap_or_else(|| panic!("missing code {}", code));
            assert_eq!(info.code, *code);
            assert_eq!(info.category, ErrorCategory::Structural);
        }
    }

    #[test]
    fn lookup_resolution_codes() {
        for code in &["E200", "E201", "IM010", "IM012"] {
            let info = lookup(code).unwrap_or_else(|| panic!("missing code {}", code));
            assert_eq!(info.code, *code);
            assert_eq!(info.category, ErrorCategory::Resolution);
        }
    }

    #[test]
    fn lookup_semantic_codes() {
        for code in &[
            "S001", "S015", "S030", "S041", "S042", "S043", "S044", "S045", "S046", "S047", "S048",
            "S051", "S060", "S066", "S067", "S068", "S069", "S070", "S071", "S106", "S107", "S108",
            "S130", "S131", "S132", "S133", "S134", "S135", "S136", "S137", "S138", "S139", "S140",
            "S090", "S091", "PH001", "PH002", "PH003", "PH004", "PH005", "PH006", "VR001", "RS001",
            "RS002", "RS003", "RS014", "FL010", "FL011", "FL012", "FL013", "FL014", "FL015", "FL016",
            "FL017", "FL018", "FL019", "FL020", "UQ001", "UQ002", "UQ003", "UQ004",
        ] {
            let info = lookup(code).unwrap_or_else(|| panic!("missing code {}", code));
            assert_eq!(info.code, *code);
            assert_eq!(info.category, ErrorCategory::Semantic);
        }
    }

    #[test]
    fn lookup_validation_codes() {
        for code in &["V001", "V002", "V003", "V004", "V005"] {
            let info = lookup(code).unwrap_or_else(|| panic!("missing code {}", code));
            assert_eq!(info.code, *code);
            assert_eq!(info.category, ErrorCategory::Validation);
        }
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup("Z999").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn registry_codes_unique_and_lookup_round_trips() {
        let mut seen = std::collections::HashSet::new();
        for entry in all() {
            assert!(seen.insert(entry.code), "duplicate code {}", entry.code);
            let found =
                lookup(entry.code).unwrap_or_else(|| panic!("lookup missed {}", entry.code));
            assert_eq!(found.code, entry.code);
            assert_eq!(found.short_description, entry.short_description);
            assert_eq!(found.category, entry.category);
        }
        assert!(!all().is_empty());
    }
}
