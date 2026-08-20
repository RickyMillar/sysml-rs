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

/// Look up information for a diagnostic error code.
pub fn lookup(code: &str) -> Option<ErrorCodeInfo> {
    match code {
        // === Structural errors (E-series) ===
        "E001" => Some(ErrorCodeInfo {
            code: "E001",
            short_description: "orphan element without an owner",
            category: ErrorCategory::Structural,
        }),
        "E002" => Some(ErrorCodeInfo {
            code: "E002",
            short_description: "ownership cycle detected",
            category: ErrorCategory::Structural,
        }),
        "E003" => Some(ErrorCodeInfo {
            code: "E003",
            short_description: "dangling membership reference",
            category: ErrorCategory::Structural,
        }),
        "E004" => Some(ErrorCodeInfo {
            code: "E004",
            short_description: "relationship source type mismatch",
            category: ErrorCategory::Structural,
        }),
        "E005" => Some(ErrorCodeInfo {
            code: "E005",
            short_description: "relationship target type mismatch",
            category: ErrorCategory::Structural,
        }),
        "E006" => Some(ErrorCodeInfo {
            code: "E006",
            short_description: "dangling relationship reference",
            category: ErrorCategory::Structural,
        }),
        "E007" => Some(ErrorCodeInfo {
            code: "E007",
            short_description: "dangling owning membership reference",
            category: ErrorCategory::Structural,
        }),
        "E008" => Some(ErrorCodeInfo {
            code: "E008",
            short_description: "invalid owning membership type",
            category: ErrorCategory::Structural,
        }),

        // === Resolution errors ===
        "E200" => Some(ErrorCodeInfo {
            code: "E200",
            short_description: "unresolved name reference",
            category: ErrorCategory::Resolution,
        }),
        "E201" => Some(ErrorCodeInfo {
            code: "E201",
            short_description: "ambiguous reference (requires qualification)",
            category: ErrorCategory::Resolution,
        }),

        // === Import / file-loading hints (IM-series) ===
        // IM001 lives in import_health.rs (kept for backwards compat).
        "IM010" => Some(ErrorCodeInfo {
            code: "IM010",
            short_description: "name not in local scope but defined elsewhere — add import or qualify",
            category: ErrorCategory::Resolution,
        }),
        "IM012" => Some(ErrorCodeInfo {
            code: "IM012",
            short_description: "file opened in strict single-file mode; cross-file imports cannot resolve",
            category: ErrorCategory::Resolution,
        }),

        // === Semantic errors (S-series) ===
        "S001" => Some(ErrorCodeInfo {
            code: "S001",
            short_description: "duplicate member name in namespace",
            category: ErrorCategory::Semantic,
        }),
        "S005" => Some(ErrorCodeInfo {
            code: "S005",
            short_description: "same top-level package name declared in multiple files (workspace)",
            category: ErrorCategory::Semantic,
        }),
        "S015" => Some(ErrorCodeInfo {
            code: "S015",
            short_description: "invalid typing for usage element",
            category: ErrorCategory::Semantic,
        }),
        "S030" => Some(ErrorCodeInfo {
            code: "S030",
            short_description: "invalid specialization across type boundaries",
            category: ErrorCategory::Semantic,
        }),
        "S041" => Some(ErrorCodeInfo {
            code: "S041",
            short_description: "ReturnParameterMembership in non-function/expression context",
            category: ErrorCategory::Semantic,
        }),
        "S042" => Some(ErrorCodeInfo {
            code: "S042",
            short_description: "membership in wrong ownership context",
            category: ErrorCategory::Semantic,
        }),
        "S043" => Some(ErrorCodeInfo {
            code: "S043",
            short_description: "SubjectMembership in non-requirement/case context",
            category: ErrorCategory::Semantic,
        }),
        "S044" => Some(ErrorCodeInfo {
            code: "S044",
            short_description: "ObjectiveMembership in non-case context",
            category: ErrorCategory::Semantic,
        }),
        "S045" => Some(ErrorCodeInfo {
            code: "S045",
            short_description: "ActorMembership in non-requirement/case context",
            category: ErrorCategory::Semantic,
        }),
        "S046" => Some(ErrorCodeInfo {
            code: "S046",
            short_description: "StakeholderMembership in non-requirement context",
            category: ErrorCategory::Semantic,
        }),
        "S047" => Some(ErrorCodeInfo {
            code: "S047",
            short_description: "RequirementConstraintMembership in non-requirement context",
            category: ErrorCategory::Semantic,
        }),
        "S048" => Some(ErrorCodeInfo {
            code: "S048",
            short_description: "ViewRenderingMembership in non-view context",
            category: ErrorCategory::Semantic,
        }),
        "S051" => Some(ErrorCodeInfo {
            code: "S051",
            short_description: "ResultExpressionMembership in non-function/expression context",
            category: ErrorCategory::Semantic,
        }),
        "S060" => Some(ErrorCodeInfo {
            code: "S060",
            short_description: "member cardinality violation",
            category: ErrorCategory::Semantic,
        }),
        "S066" => Some(ErrorCodeInfo {
            code: "S066",
            short_description: "function has more than one ReturnParameterMembership",
            category: ErrorCategory::Semantic,
        }),
        "S067" => Some(ErrorCodeInfo {
            code: "S067",
            short_description: "expression has more than one ReturnParameterMembership",
            category: ErrorCategory::Semantic,
        }),
        "S068" => Some(ErrorCodeInfo {
            code: "S068",
            short_description: "state definition has duplicate subaction",
            category: ErrorCategory::Semantic,
        }),
        "S069" => Some(ErrorCodeInfo {
            code: "S069",
            short_description: "state usage has duplicate subaction",
            category: ErrorCategory::Semantic,
        }),
        "S070" => Some(ErrorCodeInfo {
            code: "S070",
            short_description: "ViewDefinition has more than one ViewRenderingMembership",
            category: ErrorCategory::Semantic,
        }),
        "S071" => Some(ErrorCodeInfo {
            code: "S071",
            short_description: "ViewUsage has more than one ViewRenderingMembership",
            category: ErrorCategory::Semantic,
        }),

        // Requirement/case constraint and cardinality rules
        "S130" => Some(ErrorCodeInfo {
            code: "S130",
            short_description: "RequirementDefinition constraint not composite",
            category: ErrorCategory::Semantic,
        }),
        "S131" => Some(ErrorCodeInfo {
            code: "S131",
            short_description: "RequirementUsage constraint not composite",
            category: ErrorCategory::Semantic,
        }),
        "S132" => Some(ErrorCodeInfo {
            code: "S132",
            short_description: "SatisfyRequirementUsage typed by more than one requirement",
            category: ErrorCategory::Semantic,
        }),
        "S133" => Some(ErrorCodeInfo {
            code: "S133",
            short_description: "ConcernDefinition has more than one SubjectMembership",
            category: ErrorCategory::Semantic,
        }),
        "S134" => Some(ErrorCodeInfo {
            code: "S134",
            short_description: "ConcernUsage has more than one SubjectMembership",
            category: ErrorCategory::Semantic,
        }),
        "S135" => Some(ErrorCodeInfo {
            code: "S135",
            short_description: "VerificationCaseDefinition has more than one SubjectMembership",
            category: ErrorCategory::Semantic,
        }),
        "S136" => Some(ErrorCodeInfo {
            code: "S136",
            short_description: "VerificationCaseUsage has more than one SubjectMembership",
            category: ErrorCategory::Semantic,
        }),
        "S137" => Some(ErrorCodeInfo {
            code: "S137",
            short_description: "AnalysisCaseDefinition has more than one SubjectMembership",
            category: ErrorCategory::Semantic,
        }),
        "S138" => Some(ErrorCodeInfo {
            code: "S138",
            short_description: "AnalysisCaseUsage has more than one SubjectMembership",
            category: ErrorCategory::Semantic,
        }),
        "S139" => Some(ErrorCodeInfo {
            code: "S139",
            short_description: "UseCaseDefinition has more than one SubjectMembership",
            category: ErrorCategory::Semantic,
        }),
        "S140" => Some(ErrorCodeInfo {
            code: "S140",
            short_description: "UseCaseUsage has more than one SubjectMembership",
            category: ErrorCategory::Semantic,
        }),

        // === Connector ownership context (S106-S108) ===
        "S106" => Some(ErrorCodeInfo {
            code: "S106",
            short_description: "connection owned by package instead of type",
            category: ErrorCategory::Semantic,
        }),
        "S107" => Some(ErrorCodeInfo {
            code: "S107",
            short_description: "interface owned by package instead of type",
            category: ErrorCategory::Semantic,
        }),
        "S108" => Some(ErrorCodeInfo {
            code: "S108",
            short_description: "flow owned by package instead of type",
            category: ErrorCategory::Semantic,
        }),

        // === Structural property constraints (S090-S091) ===
        "S090" => Some(ErrorCodeInfo {
            code: "S090",
            short_description: "AttributeUsage must not be composite",
            category: ErrorCategory::Semantic,
        }),
        "S091" => Some(ErrorCodeInfo {
            code: "S091",
            short_description: "AttributeDefinition must not be composite",
            category: ErrorCategory::Semantic,
        }),

        // === Physics diagnostics (PH-series) ===
        "PH001" => Some(ErrorCodeInfo {
            code: "PH001",
            short_description: "domain mismatch on flow connection",
            category: ErrorCategory::Semantic,
        }),
        "PH002" => Some(ErrorCodeInfo {
            code: "PH002",
            short_description: "conservation imbalance — all ports same direction",
            category: ErrorCategory::Semantic,
        }),
        "PH003" => Some(ErrorCodeInfo {
            code: "PH003",
            short_description: "incomplete physics port — missing effort or flow feature",
            category: ErrorCategory::Semantic,
        }),
        "PH004" => Some(ErrorCodeInfo {
            code: "PH004",
            short_description: "direction conflict on flow connection",
            category: ErrorCategory::Semantic,
        }),
        "PH005" => Some(ErrorCodeInfo {
            code: "PH005",
            short_description: "R/C/I element detected but not wired with constraint",
            category: ErrorCategory::Semantic,
        }),
        "PH006" => Some(ErrorCodeInfo {
            code: "PH006",
            short_description: "Real-typed attribute could use ISQ type for physics features",
            category: ErrorCategory::Semantic,
        }),

        // === Flow / exchange-plane diagnostics (FL-series) ===
        "FL010" => Some(ErrorCodeInfo {
            code: "FL010",
            short_description: "port type mismatch on flow connection",
            category: ErrorCategory::Semantic,
        }),
        "FL011" => Some(ErrorCodeInfo {
            code: "FL011",
            short_description: "target port expects a feature the source does not provide",
            category: ErrorCategory::Semantic,
        }),
        "FL012" => Some(ErrorCodeInfo {
            code: "FL012",
            short_description: "conjugation incompatibility on flow connection",
            category: ErrorCategory::Semantic,
        }),
        "FL013" => Some(ErrorCodeInfo {
            code: "FL013",
            short_description: "unconnected output port / open terminal",
            category: ErrorCategory::Semantic,
        }),
        "FL014" => Some(ErrorCodeInfo {
            code: "FL014",
            short_description: "direction conflict on flow connection",
            category: ErrorCategory::Semantic,
        }),
        "FL015" => Some(ErrorCodeInfo {
            code: "FL015",
            short_description: "port multiplicity detected (informational)",
            category: ErrorCategory::Semantic,
        }),
        "FL016" => Some(ErrorCodeInfo {
            code: "FL016",
            short_description: "structural payload incompatibility on flow connection",
            category: ErrorCategory::Semantic,
        }),
        "FL017" => Some(ErrorCodeInfo {
            code: "FL017",
            short_description: "link class unresolved — routing as message channel",
            category: ErrorCategory::Semantic,
        }),
        "FL018" => Some(ErrorCodeInfo {
            code: "FL018",
            short_description:
                "transfer between ports not connected by any declared interface or \
                 connection (Ports.sysml interfacingPorts constraint)",
            category: ErrorCategory::Semantic,
        }),
        "FL019" => Some(ErrorCodeInfo {
            code: "FL019",
            short_description:
                "transfer direction violation — pick-up at an in-direction port or \
                 drop-off into an out-direction port (post-conjugation)",
            category: ErrorCategory::Semantic,
        }),
        "FL020" => Some(ErrorCodeInfo {
            code: "FL020",
            short_description:
                "payload type does not conform to the flow's source-output / \
                 target-input typing (Transfers.kerml payload subsetting)",
            category: ErrorCategory::Semantic,
        }),

        // === Variability / runtime lints (VR-series) ===
        "VR001" => Some(ErrorCodeInfo {
            code: "VR001",
            short_description:
                "assignment to configuration attribute (defaulted part attribute) at runtime",
            category: ErrorCategory::Semantic,
        }),

        // === Runtime semantic core (RS-series, ADR-017) ===
        "RS001" => Some(ErrorCodeInfo {
            code: "RS001",
            short_description:
                "multiple runtime writers — two executors claim the same runtime variable slot",
            category: ErrorCategory::Semantic,
        }),
        "RS002" => Some(ErrorCodeInfo {
            code: "RS002",
            short_description:
                "unknown override target — session override names neither a runtime \
                 slot alias nor an existing context variable",
            category: ErrorCategory::Semantic,
        }),
        "RS003" => Some(ErrorCodeInfo {
            code: "RS003",
            short_description:
                "unresolved runtime name — expression reference resolves to neither a \
                 runtime slot nor a model feature (hard compile error since RSC-2.5)",
            category: ErrorCategory::Semantic,
        }),
        "RS014" => Some(ErrorCodeInfo {
            code: "RS014",
            short_description:
                "time-accurate zero-crossing re-step failed hard — a located crossing could \
                 not be re-stepped without silently corrupting the run: the per-tick crossing \
                 bound was exceeded, a sub-interval integration did not perform (non-RK45 \
                 solver on the re-step path is a Wave-2b deferral), the target state machine \
                 is not slot-attached (its mode/drive writeback would be dropped — the L44 raw \
                 add_state_machine shape), or a non-crossing due event raced the crossing for \
                 the same SM in the same tick (FIFO ordering unresolved)",
            category: ErrorCategory::Semantic,
        }),

        // === Quantity / dimensional-conformance errors (UQ-series, RSC-5.2 / D-5.0.8) ===
        "UQ001" => Some(ErrorCodeInfo {
            code: "UQ001",
            short_description:
                "quantity dimension mismatch at a binding connector — endpoints carry \
                 incompatible ISQ dimensions (error), or a dimensioned endpoint is bound to \
                 an untyped attribute (warning)",
            category: ErrorCategory::Semantic,
        }),
        "UQ002" => Some(ErrorCodeInfo {
            code: "UQ002",
            short_description:
                "quantity dimension mismatch across a signal link — the source and target \
                 port-feature slots carry incompatible ISQ dimensions, so the boundary has no \
                 meaningful conversion (same-dimension scale differences are converted, not \
                 flagged)",
            category: ErrorCategory::Semantic,
        }),
        "UQ003" => Some(ErrorCodeInfo {
            code: "UQ003",
            short_description:
                "cross-dimension comparison in a constraint expression — an ordering \
                 comparison (<, <=, >, >=) between operands with incompatible ISQ dimensions \
                 (the static twin of the RSC-5.1b eval-time error)",
            category: ErrorCategory::Semantic,
        }),
        "UQ004" => Some(ErrorCodeInfo {
            code: "UQ004",
            short_description:
                "dimensioned argument to a dimensionless-only function — a transcendental \
                 (sin, cos, exp, ln, …) requires a pure-number argument",
            category: ErrorCategory::Semantic,
        }),

        // === Validation errors (V-series) ===
        "V001" => Some(ErrorCodeInfo {
            code: "V001",
            short_description: "missing required property",
            category: ErrorCategory::Validation,
        }),
        "V002" => Some(ErrorCodeInfo {
            code: "V002",
            short_description: "property has wrong type",
            category: ErrorCategory::Validation,
        }),
        "V003" => Some(ErrorCodeInfo {
            code: "V003",
            short_description: "property requires at least one value",
            category: ErrorCategory::Validation,
        }),
        "V004" => Some(ErrorCodeInfo {
            code: "V004",
            short_description: "property allows at most one value",
            category: ErrorCategory::Validation,
        }),
        "V005" => Some(ErrorCodeInfo {
            code: "V005",
            short_description: "read-only property modified",
            category: ErrorCategory::Validation,
        }),

        _ => None,
    }
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
}
