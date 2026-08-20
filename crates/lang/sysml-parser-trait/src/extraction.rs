//! Parser-agnostic extraction data structures.
//!
//! These structs hold all data extracted from syntax nodes in a single traversal,
//! enabling both Pest and tree-sitter parsers to share the same element building
//! logic. The extraction structs are parser-agnostic - they don't depend on any
//! specific parser's types.
//!
//! ## Design
//!
//! The extraction structs capture all semantic information needed to build
//! `Element` instances. Each parser (Pest, tree-sitter) populates these structs
//! from their respective parse trees, then uses the shared `build_element()`
//! methods to create `Element` instances.
//!
//! This provides ~70% code reuse between parsers and ensures consistent
//! model construction regardless of the parsing backend.
//!
//! ## Canonical-key threading (ADR-009 / S1)
//!
//! Extraction structs carry an optional `parent_key` (and an optional
//! `sibling_index` for the anonymous case). When `parent_key` is set,
//! `build_element` derives a stable [`sysml_core::CanonicalKey`] from it
//! and routes through `Element::new_with_key`, so the resulting
//! `ElementId` is reparse-stable. When `parent_key` is `None` (today's
//! default), `build_element` falls through to `Element::new_with_kind`
//! and mints a fresh UUID — preserving today's behaviour for unmigrated
//! call sites.

/// All data extracted from a Usage element.
///
/// This struct captures all semantic fields needed to build a Usage element,
/// including flags, typing, and specialization relationships.
#[derive(Debug, Default, Clone)]
pub struct UsageExtraction {
    // === Flags from prefix ===
    pub is_abstract: bool,
    pub is_variation: bool,
    pub is_readonly: bool,
    pub is_derived: bool,
    pub is_end: bool,
    pub is_reference: bool,
    // KerML feature modifiers (for standard library parsing)
    pub is_composite: bool,
    pub is_portion: bool,
    pub is_variable: bool,
    pub is_constant: bool,
    pub is_individual: bool,
    /// Portion kind: "snapshot" or "timeslice" (set when snapshot/timeslice keyword is used)
    pub portion_kind: Option<String>,

    // === Direction ===
    /// Feature direction: "in", "out", or "inout"
    pub direction: Option<String>,

    // === Identification from declaration ===
    pub name: Option<String>,
    /// Declared short name (`<'REQ-001'>` → `REQ-001`). Stored as the
    /// `declaredShortName` prop — `Element::effective_short_name` reads it
    /// (KerML effectiveShortName(); for requirements this IS the requirement
    /// ID per SysML §7.21.2 / the API's `reqId`).
    pub short_name: Option<String>,

    // === Multiplicity ===
    /// (lower, upper) where upper=None means unbounded (*)
    pub multiplicity: Option<(i64, Option<i64>)>,
    /// Symbolic lower bound text (e.g., "min") when lower is not a literal integer
    pub multiplicity_lower_text: Option<String>,
    /// Symbolic upper bound text (e.g., "max") when upper is not a literal integer
    pub multiplicity_upper_text: Option<String>,
    /// Whether the multiplicity is ordered (default: false)
    pub is_ordered: bool,
    /// Whether the multiplicity is nonunique (default: false, spec uses isUnique = !isNonunique)
    pub is_nonunique: bool,

    // === Feature value ===
    pub value_expression: Option<String>,
    pub value_is_default: bool,
    pub value_is_initial: bool,
    /// Whether the value is a literal (not a reference that needs resolution)
    pub value_is_literal: bool,

    // === Feature specializations ===
    /// FeatureTyping targets (from `:` or `typed by` syntax)
    pub typings: Vec<String>,
    /// ConjugatedPortTyping targets (from `: ~Type` syntax).
    /// These are qualified names where the `~` prefix has been stripped.
    pub conjugated_typings: Vec<String>,
    /// Subsetting targets (from `:>` or `subsets` syntax)
    pub subsettings: Vec<String>,
    /// Redefinition targets (from `:>>` or `redefines` syntax)
    pub redefinitions: Vec<String>,
    /// ReferenceSubsetting targets (from `::>` or `references` syntax)
    pub references: Vec<String>,
    /// CrossSubsetting targets (from `crosses` syntax)
    pub crosses: Vec<String>,

    // === Connector endpoints ===
    /// Source endpoint name (e.g., "pathA.phaseIn") for connector-like usages
    pub connector_source: Option<String>,
    /// Target endpoint name for connector-like usages
    pub connector_target: Option<String>,

    // === Canonical-key threading (ADR-009 / S1) ===
    /// Parent canonical key. When `Some`, `build_element` mints a stable
    /// `ElementId` derived from this key; when `None`, falls through to
    /// the legacy fresh-UUID path (`Element::new_with_kind`).
    pub parent_key: Option<sysml_core::CanonicalKey>,
    /// Zero-based index among the parent's children of the same kind. Only
    /// used when `name` is `None` and `parent_key` is `Some`. Defaults to 0.
    pub sibling_index: Option<usize>,
}

impl UsageExtraction {
    /// Create a new empty usage extraction.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if text looks like a literal value (heuristic).
    pub fn text_looks_like_literal(text: &str) -> bool {
        // RSC-5.1 (D-5.0.5): a quantity literal `num [unit]` folds like its bare
        // magnitude — strip a trailing measurement reference first.
        let (s, _unit) = Self::split_unit_annotation(text);
        let s = s.trim();
        // Boolean literals
        if s == "true" || s == "false" {
            return true;
        }
        // Infinity literal
        if s == "*" {
            return true;
        }
        // String literal (quoted)
        if s.starts_with('"') && s.ends_with('"') {
            return true;
        }
        // Number literal (integer or real)
        if s.parse::<i64>().is_ok() || s.parse::<f64>().is_ok() {
            return true;
        }
        false
    }

    /// RSC-5.1 (D-5.0.5): split a literal value text into `(magnitude, unit)`,
    /// where `unit` is a trailing `[<measurement reference>]` annotation.
    /// `"273.15 [K]"` → `("273.15", Some("K"))`; `"100 [SI::kg]"` →
    /// `("100", Some("SI::kg"))`; `"105"` → `("105", None)`.
    ///
    /// Only splits when the magnitude side is itself a bare number — this is the
    /// spec measurement-reference operator `'['(num, mRef)`, distinct from
    /// indexing (`arr[2]`, whose source is a feature, not a number). Keeping the
    /// recogniser here gives the fold path one home shared with
    /// [`Self::text_looks_like_literal`] and `element_builder`.
    pub fn split_unit_annotation(text: &str) -> (&str, Option<&str>) {
        let s = text.trim();
        if let Some(open) = s.rfind('[') {
            if s.ends_with(']') {
                let mag = s[..open].trim();
                let unit = s[open + 1..s.len() - 1].trim();
                let mag_is_number = mag.parse::<i64>().is_ok() || mag.parse::<f64>().is_ok();
                if !mag.is_empty() && !unit.is_empty() && mag_is_number {
                    return (mag, Some(unit));
                }
            }
        }
        (s, None)
    }
}

/// All data extracted from a Definition element.
#[derive(Debug, Default, Clone)]
pub struct DefinitionExtraction {
    // === Flags from prefix ===
    pub is_abstract: bool,
    pub is_variation: bool,

    // === Identification from declaration ===
    pub name: Option<String>,
    /// Declared short name (`<'REQ-001'>` → `REQ-001`) — see
    /// [`UsageExtraction::short_name`].
    pub short_name: Option<String>,

    // === Subclassification targets ===
    pub subclassifications: Vec<String>,

    // === Canonical-key threading (ADR-009 / S1) ===
    /// Parent canonical key — see [`UsageExtraction::parent_key`].
    pub parent_key: Option<sysml_core::CanonicalKey>,
    /// Sibling index for the anonymous case — see
    /// [`UsageExtraction::sibling_index`].
    pub sibling_index: Option<usize>,
}

impl DefinitionExtraction {
    /// Create a new empty definition extraction.
    pub fn new() -> Self {
        Self::default()
    }
}

/// All data extracted from a Package element.
#[derive(Debug, Default, Clone)]
pub struct PackageExtraction {
    pub name: Option<String>,
    pub is_standard: bool,

    // === Canonical-key threading (ADR-009 / S1) ===
    /// Parent canonical key — see [`UsageExtraction::parent_key`].
    pub parent_key: Option<sysml_core::CanonicalKey>,
    /// Sibling index for the anonymous case — see
    /// [`UsageExtraction::sibling_index`].
    pub sibling_index: Option<usize>,
}

impl PackageExtraction {
    /// Create a new empty package extraction.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Parse multiplicity text like "[4]", "[0..10]", "[*]", "[1..*]".
///
/// Returns (lower, upper) where upper=None means unbounded.
/// For symbolic bounds (e.g., "[min..max]"), returns None.
/// Use [`parse_multiplicity_full`] to also capture symbolic bounds.
pub fn parse_multiplicity_text(text: &str) -> Option<(i64, Option<i64>)> {
    let content = text.trim_start_matches('[').trim_end_matches(']').trim();

    if content == "*" {
        return Some((0, None));
    }

    if let Some(dot_pos) = content.find("..") {
        let lower_str = content[..dot_pos].trim();
        let upper_str = content[dot_pos + 2..].trim();

        let lower: i64 = lower_str.parse().ok()?;
        let upper = if upper_str == "*" {
            None
        } else {
            Some(upper_str.parse().ok()?)
        };

        Some((lower, upper))
    } else {
        let value: i64 = content.parse().ok()?;
        Some((value, Some(value)))
    }
}

/// Result of full multiplicity parsing, including symbolic bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiplicityResult {
    /// Numeric bounds, if both lower and upper are literals
    pub numeric: Option<(i64, Option<i64>)>,
    /// Symbolic lower bound text (e.g., "min") when lower is not a literal integer
    pub lower_text: Option<String>,
    /// Symbolic upper bound text (e.g., "max") when upper is not a literal integer
    pub upper_text: Option<String>,
}

/// Parse multiplicity text, capturing symbolic bounds when numeric parsing fails.
///
/// For `[0..10]` → numeric = Some((0, Some(10))), no symbolic text.
/// For `[min..max]` → numeric = None, lower_text = Some("min"), upper_text = Some("max").
/// For `[0..n]` → numeric = None, lower_text = None, upper_text = Some("n").
pub fn parse_multiplicity_full(text: &str) -> Option<MultiplicityResult> {
    let content = text.trim_start_matches('[').trim_end_matches(']').trim();

    if content.is_empty() {
        return None;
    }

    if content == "*" {
        return Some(MultiplicityResult {
            numeric: Some((0, None)),
            lower_text: None,
            upper_text: None,
        });
    }

    if let Some(dot_pos) = content.find("..") {
        let lower_str = content[..dot_pos].trim();
        let upper_str = content[dot_pos + 2..].trim();

        let lower_num = lower_str.parse::<i64>().ok();
        let upper_num = if upper_str == "*" {
            // * means unbounded — represented as None in numeric
            Some(None)
        } else {
            upper_str.parse::<i64>().ok().map(Some)
        };

        let lower_text = if lower_num.is_none() && !lower_str.is_empty() {
            Some(lower_str.to_owned())
        } else {
            None
        };
        let upper_text = if upper_num.is_none() && !upper_str.is_empty() && upper_str != "*" {
            Some(upper_str.to_owned())
        } else {
            None
        };

        let numeric = match (lower_num, upper_num) {
            (Some(l), Some(u)) => Some((l, u)),
            _ => None,
        };

        Some(MultiplicityResult {
            numeric,
            lower_text,
            upper_text,
        })
    } else {
        let value_num = content.parse::<i64>().ok();
        if let Some(v) = value_num {
            Some(MultiplicityResult {
                numeric: Some((v, Some(v))),
                lower_text: None,
                upper_text: None,
            })
        } else {
            // Single symbolic bound (e.g., [n])
            Some(MultiplicityResult {
                numeric: None,
                lower_text: Some(content.to_owned()),
                upper_text: None,
            })
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_multiplicity_exact() {
        assert_eq!(parse_multiplicity_text("[4]"), Some((4, Some(4))));
    }

    #[test]
    fn test_parse_multiplicity_range() {
        assert_eq!(parse_multiplicity_text("[0..10]"), Some((0, Some(10))));
    }

    #[test]
    fn test_parse_multiplicity_unbounded() {
        assert_eq!(parse_multiplicity_text("[*]"), Some((0, None)));
        assert_eq!(parse_multiplicity_text("[1..*]"), Some((1, None)));
    }

    #[test]
    fn test_parse_multiplicity_symbolic_range() {
        let result = parse_multiplicity_full("[min..max]").unwrap();
        assert_eq!(result.numeric, None);
        assert_eq!(result.lower_text, Some("min".to_string()));
        assert_eq!(result.upper_text, Some("max".to_string()));
    }

    #[test]
    fn test_parse_multiplicity_mixed_symbolic() {
        let result = parse_multiplicity_full("[0..n]").unwrap();
        assert_eq!(result.numeric, None);
        assert_eq!(result.lower_text, None); // 0 is numeric, stored as None
        assert_eq!(result.upper_text, Some("n".to_string()));
    }

    #[test]
    fn test_parse_multiplicity_full_numeric() {
        let result = parse_multiplicity_full("[1..5]").unwrap();
        assert_eq!(result.numeric, Some((1, Some(5))));
        assert_eq!(result.lower_text, None);
        assert_eq!(result.upper_text, None);
    }

    #[test]
    fn test_parse_multiplicity_full_unbounded() {
        let result = parse_multiplicity_full("[0..*]").unwrap();
        assert_eq!(result.numeric, Some((0, None)));
        assert_eq!(result.lower_text, None);
        assert_eq!(result.upper_text, None);
    }

    #[test]
    fn test_parse_multiplicity_single_symbolic() {
        let result = parse_multiplicity_full("[n]").unwrap();
        assert_eq!(result.numeric, None);
        assert_eq!(result.lower_text, Some("n".to_string()));
        assert_eq!(result.upper_text, None);
    }

    #[test]
    fn test_text_looks_like_literal() {
        assert!(UsageExtraction::text_looks_like_literal("true"));
        assert!(UsageExtraction::text_looks_like_literal("false"));
        assert!(UsageExtraction::text_looks_like_literal("*"));
        assert!(UsageExtraction::text_looks_like_literal("\"hello\""));
        assert!(UsageExtraction::text_looks_like_literal("42"));
        assert!(UsageExtraction::text_looks_like_literal("3.14"));

        assert!(!UsageExtraction::text_looks_like_literal("someVariable"));
        assert!(!UsageExtraction::text_looks_like_literal("a + b"));
    }
}
