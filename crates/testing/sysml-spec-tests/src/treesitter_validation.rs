//! Tree-sitter grammar validation against SysML v2 specification.
//!
//! This module validates that the tree-sitter grammar (sysml-ts) correctly
//! represents SysML v2 element types and enum values by cross-referencing
//! against the authoritative spec files using `sysml-codegen` parsers.
//!
//! ## Validation Levels
//!
//! 1. **Element type coverage**: All spec Definition/Usage types have tree-sitter rules
//! 2. **Enum value correctness**: Grammar enum values match spec exactly
//! 3. **Structural validation**: Required properties are parseable (future)

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use sysml_codegen::ttl_parser::{EnumInfo, TypeInfo};
use sysml_codegen::{merge_enum_info, parse_ttl_enums, parse_ttl_vocab};
use sysml_codegen::{parse_xtext_enums, XtextEnumInfo};

// ---------------------------------------------------------------------------
// Node-types.json parsing
// ---------------------------------------------------------------------------

/// A node type entry from tree-sitter's generated node-types.json.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NodeType {
    #[serde(rename = "type")]
    pub type_name: String,
    pub named: bool,
    #[serde(default)]
    pub fields: HashMap<String, FieldInfo>,
    #[serde(default)]
    pub children: Option<ChildrenInfo>,
    #[serde(default)]
    pub subtypes: Option<Vec<TypeRef>>,
}

/// Field information for a node type.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct FieldInfo {
    pub multiple: bool,
    pub required: bool,
    pub types: Vec<TypeRef>,
}

/// Children information for a node type.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ChildrenInfo {
    pub multiple: bool,
    pub required: bool,
    pub types: Vec<TypeRef>,
}

/// A type reference within fields or children.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TypeRef {
    #[serde(rename = "type")]
    pub type_name: String,
    pub named: bool,
}

/// Load and parse tree-sitter's node-types.json.
pub fn load_node_types(path: &Path) -> Result<Vec<NodeType>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse node-types.json: {}", e))
}

/// Get all named node type names from the tree-sitter grammar.
pub fn named_node_types(node_types: &[NodeType]) -> HashSet<String> {
    node_types
        .iter()
        .filter(|n| n.named)
        .map(|n| n.type_name.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Spec name ↔ tree-sitter name mapping
// ---------------------------------------------------------------------------

/// Convert a PascalCase spec type name to the expected tree-sitter snake_case name.
///
/// Rules:
/// - PascalCase → snake_case
/// - `_definition` suffix → `_def`
///
/// Examples:
/// - `PartDefinition` → `part_def`
/// - `ActionUsage` → `action_usage`
/// - `FlowConnectionUsage` → `flow_connection_usage`
pub fn spec_to_treesitter_name(spec_name: &str) -> String {
    // Merged standard usages: 5 spec types share one grammar rule (standard_usage)
    // with a `keyword` field distinguishing them.
    match spec_name {
        "PartUsage" | "AttributeUsage" | "ItemUsage" | "OccurrenceUsage" | "ReferenceUsage" => {
            return "standard_usage".to_string()
        }
        _ => {}
    }

    let mut result = String::new();
    for (i, ch) in spec_name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(ch.to_ascii_lowercase());
    }
    // Abbreviate "definition" → "def" to match tree-sitter conventions
    result = result.replace("_definition", "_def");
    result
}

// ---------------------------------------------------------------------------
// Type classification
// ---------------------------------------------------------------------------

/// How a spec type maps to grammar constructs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCategory {
    /// Concrete definition type (e.g., PartDefinition)
    Definition,
    /// Concrete usage type (e.g., PartUsage)
    Usage,
    /// Relationship type (e.g., Specialization) — not a grammar rule
    Relationship,
    /// Abstract/base type (e.g., Element, Feature) — no direct syntax
    Abstract,
}

/// Classify a spec type based on its name and supertypes.
pub fn classify_type(info: &TypeInfo) -> TypeCategory {
    let name = &info.name;

    // Relationships
    if name.ends_with("Relationship") || name == "Relationship" {
        return TypeCategory::Relationship;
    }
    if info
        .supertypes
        .iter()
        .any(|s| s == "Relationship" || s.ends_with("Relationship"))
    {
        return TypeCategory::Relationship;
    }

    // Definitions and usages
    if name.ends_with("Definition") {
        return TypeCategory::Definition;
    }
    if name.ends_with("Usage") {
        return TypeCategory::Usage;
    }

    TypeCategory::Abstract
}

// ---------------------------------------------------------------------------
// Element type validation
// ---------------------------------------------------------------------------

/// A spec type that matched an explicit tree-sitter rule.
#[derive(Debug, Clone)]
pub struct TypeMatch {
    pub spec_name: String,
    pub treesitter_name: String,
}

/// Result of validating element type coverage.
#[derive(Debug)]
pub struct ElementTypeCoverage {
    /// Spec types with explicit tree-sitter rules (best coverage).
    pub explicit_rules: Vec<TypeMatch>,
    /// Spec types covered only by the generic `definition`/`usage` fallback.
    pub generic_coverage: Vec<String>,
    /// Spec types with no tree-sitter coverage at all.
    pub missing: Vec<String>,
    /// Tree-sitter node types that don't map to any spec Definition/Usage.
    pub extra_rules: Vec<String>,
    /// Total spec Definition/Usage types checked.
    pub total_spec_types: usize,
    /// Percentage of spec types covered (explicit + generic).
    pub coverage_percent: f64,
}

/// Well-known tree-sitter node types that are structural/expression rules,
/// not direct mappings to spec element types.
const STRUCTURAL_RULES: &[&str] = &[
    // Top-level
    "source_file",
    // Namespace/package
    "package_body",
    "package_decl",
    "namespace_body",
    "namespace_decl",
    "library_package",
    "filter_package",
    // Bodies
    "definition_body",
    "usage_body",
    "action_body",
    "state_body",
    "constraint_body",
    "requirement_body",
    "enum_body",
    "relationship_body",
    // Generic fallbacks
    "definition",
    "usage",
    // Names and references
    "qualified_name",
    "identifier",
    "quoted_name",
    "type_ref",
    "typing",
    "supertype_list",
    "multiplicity",
    "default_value",
    "visibility_indicator",
    "usage_prefix",
    // Imports and aliases
    "import_decl",
    "alias_decl",
    // Comments and docs
    "comment",
    "comment_element",
    "doc_comment",
    "doc_string",
    // Literals
    "literal",
    "number",
    "string_literal",
    "integer_literal",
    "real_literal",
    "boolean_literal",
    "null_literal",
    // Expressions
    "conditional_expression",
    "null_coalesce_expression",
    "implies_expression",
    "or_expression",
    "xor_expression",
    "and_expression",
    "equality_expression",
    "classification_expression",
    "relational_expression",
    "range_expression",
    "additive_expression",
    "multiplicative_expression",
    "exponentiation_expression",
    "unary_expression",
    "primary_expression",
    "parenthesized_expression",
    "bracket_expression",
    "invocation_expression",
    "feature_chain",
    "argument_list",
    "select_expression",
    "collect_expression",
    // Clauses
    "redefines_clause",
    "redefinition",
    "subsets_clause",
    "references_clause",
    // Connectors/endpoints
    "connection_ends",
    "flow_ends",
    "allocation_ends",
    // State machine sub-elements
    "guard_expression",
    "transition_source",
    "transition_target",
    "trigger_action",
    "effect_action",
    "entry_action",
    "exit_action",
    "do_action",
    // Action sub-elements
    "accept_action",
    "send_action",
    "if_action",
    "while_action",
    "for_action",
    "assignment_action",
    // Requirement sub-elements
    "assume_constraint",
    "require_constraint",
    "subject_requirement",
    // Enum sub-elements
    "enum_member",
];

/// Validate tree-sitter element type coverage against the SysML v2 spec.
///
/// Uses `sysml-codegen::parse_ttl_vocab()` to extract all spec types, then
/// checks whether each Definition/Usage type has a tree-sitter grammar rule.
pub fn validate_element_types(
    kerml_types: &[TypeInfo],
    sysml_types: &[TypeInfo],
    node_types: &[NodeType],
) -> ElementTypeCoverage {
    let ts_named = named_node_types(node_types);

    let structural: HashSet<&str> = STRUCTURAL_RULES.iter().copied().collect();

    let all_types: Vec<&TypeInfo> = kerml_types.iter().chain(sysml_types.iter()).collect();

    let mut explicit_rules = Vec::new();
    let mut generic_coverage = Vec::new();
    let mut missing = Vec::new();
    let mut def_usage_count = 0;

    for type_info in &all_types {
        let category = classify_type(type_info);
        match category {
            TypeCategory::Definition | TypeCategory::Usage => {
                def_usage_count += 1;
                let expected_name = spec_to_treesitter_name(&type_info.name);

                if ts_named.contains(&expected_name) {
                    explicit_rules.push(TypeMatch {
                        spec_name: type_info.name.clone(),
                        treesitter_name: expected_name,
                    });
                } else {
                    // Check if covered by generic "definition" or "usage" rule
                    let has_generic = match category {
                        TypeCategory::Definition => ts_named.contains("definition"),
                        TypeCategory::Usage => ts_named.contains("usage"),
                        _ => false,
                    };
                    if has_generic {
                        generic_coverage.push(type_info.name.clone());
                    } else {
                        missing.push(type_info.name.clone());
                    }
                }
            }
            _ => {} // Skip relationships, abstract types
        }
    }

    // Find tree-sitter rules with no spec counterpart
    let spec_names: HashSet<String> = all_types
        .iter()
        .filter(|t| {
            matches!(
                classify_type(t),
                TypeCategory::Definition | TypeCategory::Usage
            )
        })
        .map(|t| spec_to_treesitter_name(&t.name))
        .collect();

    let mut extra_rules: Vec<String> = ts_named
        .iter()
        .filter(|name| !spec_names.contains(name.as_str()) && !structural.contains(name.as_str()))
        .cloned()
        .collect();
    extra_rules.sort();

    // Sort for deterministic output
    explicit_rules.sort_by(|a, b| a.spec_name.cmp(&b.spec_name));
    generic_coverage.sort();
    missing.sort();

    let covered = explicit_rules.len() + generic_coverage.len();
    let coverage_percent = if def_usage_count > 0 {
        (covered as f64 / def_usage_count as f64) * 100.0
    } else {
        0.0
    };

    ElementTypeCoverage {
        explicit_rules,
        generic_coverage,
        missing,
        extra_rules,
        total_spec_types: def_usage_count,
        coverage_percent,
    }
}

// ---------------------------------------------------------------------------
// Enum validation
// ---------------------------------------------------------------------------

/// An enum that matched between spec and grammar.
#[derive(Debug, Clone)]
pub struct EnumMatch {
    pub name: String,
    pub values: Vec<String>,
}

/// An enum with value mismatches between spec and grammar.
#[derive(Debug, Clone)]
pub struct EnumMismatch {
    pub name: String,
    pub spec_values: Vec<String>,
    pub grammar_values: Vec<String>,
    pub missing_from_grammar: Vec<String>,
    pub extra_in_grammar: Vec<String>,
}

/// Result of validating enum coverage.
#[derive(Debug)]
pub struct EnumCoverage {
    /// Enums where spec and grammar values match exactly.
    pub matching: Vec<EnumMatch>,
    /// Enums in spec but not in grammar.
    pub spec_only: Vec<String>,
    /// Enums in grammar but not in spec.
    pub grammar_only: Vec<String>,
    /// Enums present in both but with different values.
    pub value_mismatches: Vec<EnumMismatch>,
}

/// Parse the enum definitions the tree-sitter grammar actually uses.
///
/// Historically the grammar declared enums inline in `grammar.js` as
/// `const enums = { ... };`. They are now generated into a modular asset
/// `generated/enums.js` (`module.exports = { ... };`), which `grammar.js`
/// `require()`s (`rules/common.js`). Reading only the inline block silently
/// returned an empty map after that move, which made every spec enum look
/// "spec-only" while the enum gate passed vacuously (SUP-02). This reads
/// **both**: the legacy inline block (if any remains) and the modular
/// `generated/enums.js` sibling, with the modular asset winning on collision
/// (it is the live source). A missing modular file is not an error — older
/// checkouts predate the split.
pub fn parse_grammar_enums(grammar_js_path: &Path) -> Result<HashMap<String, Vec<String>>, String> {
    let content = std::fs::read_to_string(grammar_js_path)
        .map_err(|e| format!("Failed to read grammar.js: {}", e))?;

    // Legacy inline `const enums = { ... };` block (empty/absent after the split).
    let mut result = parse_js_enum_object(&content, |t| t.starts_with("const enums = {"));

    // Modular generated asset: `<grammar dir>/generated/enums.js`.
    if let Some(dir) = grammar_js_path.parent() {
        let modular = dir.join("generated").join("enums.js");
        if modular.exists() {
            let modular_content = std::fs::read_to_string(&modular)
                .map_err(|e| format!("Failed to read {}: {e}", modular.display()))?;
            for (name, values) in
                parse_js_enum_object(&modular_content, |t| t.starts_with("module.exports = {"))
            {
                result.insert(name, values);
            }
        }
    }

    Ok(result)
}

/// Parse a `<start> ... };` JS object literal of `"Name": ["v", ...],` entries.
/// `is_start` matches the trimmed opening line (`const enums = {` /
/// `module.exports = {`); parsing runs until the closing `};`.
fn parse_js_enum_object(
    content: &str,
    is_start: impl Fn(&str) -> bool,
) -> HashMap<String, Vec<String>> {
    let mut result = HashMap::new();
    let mut in_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if !in_block {
            if is_start(trimmed) {
                in_block = true;
            }
            continue;
        }
        if trimmed == "};" {
            break;
        }
        // Parse: "EnumName": ["val1", "val2", ...],
        if let Some(colon_pos) = trimmed.find(':') {
            let name = trimmed[..colon_pos].trim().trim_matches('"');
            let rest = trimmed[colon_pos + 1..].trim();
            if rest.starts_with('[') {
                let bracket_end = rest.find(']').unwrap_or(rest.len());
                let values_str = &rest[1..bracket_end];
                let values: Vec<String> = values_str
                    .split(',')
                    .map(|v| v.trim().trim_matches('"').to_string())
                    .filter(|v| !v.is_empty())
                    .collect();
                result.insert(name.to_string(), values);
            }
        }
    }

    result
}

/// Validate grammar enum values against the TTL spec enums.
pub fn validate_enums_ttl(
    spec_enums: &[EnumInfo],
    grammar_enums: &HashMap<String, Vec<String>>,
) -> EnumCoverage {
    let spec_names: HashSet<String> = spec_enums.iter().map(|e| e.name.clone()).collect();
    let grammar_names: HashSet<String> = grammar_enums.keys().cloned().collect();

    let mut matching = Vec::new();
    let mut value_mismatches = Vec::new();

    for spec_enum in spec_enums {
        if let Some(grammar_values) = grammar_enums.get(&spec_enum.name) {
            let spec_values: HashSet<String> =
                spec_enum.values.iter().map(|v| v.name.clone()).collect();
            let grammar_value_set: HashSet<String> = grammar_values.iter().cloned().collect();

            if spec_values == grammar_value_set {
                matching.push(EnumMatch {
                    name: spec_enum.name.clone(),
                    values: grammar_values.clone(),
                });
            } else {
                let mut missing_from_grammar: Vec<String> = spec_values
                    .difference(&grammar_value_set)
                    .cloned()
                    .collect();
                let mut extra_in_grammar: Vec<String> = grammar_value_set
                    .difference(&spec_values)
                    .cloned()
                    .collect();
                missing_from_grammar.sort();
                extra_in_grammar.sort();

                value_mismatches.push(EnumMismatch {
                    name: spec_enum.name.clone(),
                    spec_values: spec_enum.values.iter().map(|v| v.name.clone()).collect(),
                    grammar_values: grammar_values.clone(),
                    missing_from_grammar,
                    extra_in_grammar,
                });
            }
        }
    }

    let mut spec_only: Vec<String> = spec_names.difference(&grammar_names).cloned().collect();
    let mut grammar_only: Vec<String> = grammar_names.difference(&spec_names).cloned().collect();
    spec_only.sort();
    grammar_only.sort();

    EnumCoverage {
        matching,
        spec_only,
        grammar_only,
        value_mismatches,
    }
}

/// Validate grammar enum values against the xtext grammar enums.
///
/// This is a separate check because xtext enums represent what the *parser*
/// should accept, which may differ from the full vocabulary.
pub fn validate_enums_xtext(
    xtext_enums: &[XtextEnumInfo],
    grammar_enums: &HashMap<String, Vec<String>>,
) -> EnumCoverage {
    // Build xtext enum map (grouped by return type, same as treesitter_generator)
    let mut xtext_map: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for enum_info in xtext_enums {
        let type_name = enum_info
            .returns_type
            .rsplit("::")
            .next()
            .unwrap_or(&enum_info.returns_type)
            .to_string();
        let values = xtext_map.entry(type_name).or_default();
        for (_, keyword) in &enum_info.values {
            if !keyword.is_empty() && keyword.chars().all(|c| c.is_ascii_alphabetic() || c == '_') {
                values.insert(keyword.clone());
            }
        }
    }

    let xtext_names: HashSet<String> = xtext_map.keys().cloned().collect();
    let grammar_names: HashSet<String> = grammar_enums.keys().cloned().collect();

    let mut matching = Vec::new();
    let mut value_mismatches = Vec::new();

    for (name, xtext_values) in &xtext_map {
        if let Some(grammar_values) = grammar_enums.get(name) {
            let grammar_value_set: HashSet<String> = grammar_values.iter().cloned().collect();

            if xtext_values == &grammar_value_set {
                matching.push(EnumMatch {
                    name: name.clone(),
                    values: grammar_values.clone(),
                });
            } else {
                let mut missing_from_grammar: Vec<String> = xtext_values
                    .difference(&grammar_value_set)
                    .cloned()
                    .collect();
                let mut extra_in_grammar: Vec<String> = grammar_value_set
                    .difference(xtext_values)
                    .cloned()
                    .collect();
                missing_from_grammar.sort();
                extra_in_grammar.sort();

                value_mismatches.push(EnumMismatch {
                    name: name.clone(),
                    spec_values: xtext_values.iter().cloned().collect(),
                    grammar_values: grammar_values.clone(),
                    missing_from_grammar,
                    extra_in_grammar,
                });
            }
        }
    }

    let mut spec_only: Vec<String> = xtext_names.difference(&grammar_names).cloned().collect();
    let mut grammar_only: Vec<String> = grammar_names.difference(&xtext_names).cloned().collect();
    spec_only.sort();
    grammar_only.sort();

    EnumCoverage {
        matching,
        spec_only,
        grammar_only,
        value_mismatches,
    }
}

// ---------------------------------------------------------------------------
// Top-level validation orchestrator
// ---------------------------------------------------------------------------

/// Full validation report.
#[derive(Debug)]
pub struct ValidationReport {
    pub element_type_coverage: ElementTypeCoverage,
    pub enum_coverage_ttl: EnumCoverage,
    pub enum_coverage_xtext: Option<EnumCoverage>,
}

/// Run all tree-sitter validations against the spec.
///
/// # Arguments
///
/// * `refs_dir` - Path to the references/sysmlv2 directory
/// * `treesitter_dir` - Path to sysml-ts/tree-sitter directory
pub fn validate_treesitter(
    refs_dir: &Path,
    treesitter_dir: &Path,
) -> Result<ValidationReport, String> {
    // --- Read spec files ---
    let kerml_vocab = std::fs::read_to_string(refs_dir.join("Kerml-Vocab.ttl"))
        .map_err(|e| format!("Failed to read Kerml-Vocab.ttl: {}", e))?;
    let sysml_vocab = std::fs::read_to_string(refs_dir.join("SysML-vocab.ttl"))
        .map_err(|e| format!("Failed to read SysML-vocab.ttl: {}", e))?;

    // --- Parse spec data via codegen ---
    let kerml_types =
        parse_ttl_vocab(&kerml_vocab).map_err(|e| format!("Failed to parse KerML vocab: {e}"))?;
    let sysml_types =
        parse_ttl_vocab(&sysml_vocab).map_err(|e| format!("Failed to parse SysML vocab: {e}"))?;

    let kerml_enums =
        parse_ttl_enums(&kerml_vocab).map_err(|e| format!("Failed to parse KerML enums: {e}"))?;
    let sysml_enums =
        parse_ttl_enums(&sysml_vocab).map_err(|e| format!("Failed to parse SysML enums: {e}"))?;
    let all_enums = merge_enum_info(kerml_enums, sysml_enums);

    // --- Load tree-sitter data ---
    let node_types_path = treesitter_dir.join("src/node-types.json");
    let node_types = load_node_types(&node_types_path)?;

    let grammar_js_path = treesitter_dir.join("grammar.js");
    let grammar_enums = parse_grammar_enums(&grammar_js_path)?;

    // --- Element type validation ---
    let element_type_coverage = validate_element_types(&kerml_types, &sysml_types, &node_types);

    // --- Enum validation (TTL) ---
    let enum_coverage_ttl = validate_enums_ttl(&all_enums, &grammar_enums);

    // --- Enum validation (xtext, optional) ---
    let xtext_path = refs_dir.join(
        "SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/src/org/omg/sysml/xtext/SysML.xtext",
    );
    let enum_coverage_xtext = if xtext_path.exists() {
        let xtext_content = std::fs::read_to_string(&xtext_path)
            .map_err(|e| format!("Failed to read SysML.xtext: {e}"))?;
        let xtext_enums = parse_xtext_enums(&xtext_content);
        Some(validate_enums_xtext(&xtext_enums, &grammar_enums))
    } else {
        None
    };

    Ok(ValidationReport {
        element_type_coverage,
        enum_coverage_ttl,
        enum_coverage_xtext,
    })
}

// ---------------------------------------------------------------------------
// Report formatting
// ---------------------------------------------------------------------------

/// Format the validation report for human-readable output.
pub fn format_report(report: &ValidationReport) -> String {
    let mut out = String::new();

    out.push_str("═══════════════════════════════════════════════════════════\n");
    out.push_str("  Tree-sitter Grammar Spec Validation Report\n");
    out.push_str("═══════════════════════════════════════════════════════════\n\n");

    // --- Element type coverage ---
    let etc = &report.element_type_coverage;
    out.push_str(&format!(
        "ELEMENT TYPE COVERAGE: {:.1}% ({}/{} spec types)\n",
        etc.coverage_percent,
        etc.explicit_rules.len() + etc.generic_coverage.len(),
        etc.total_spec_types,
    ));
    out.push_str(&format!(
        "  Explicit rules:   {} (best — distinct node types)\n",
        etc.explicit_rules.len()
    ));
    out.push_str(&format!(
        "  Generic fallback: {} (parsed but not distinguished)\n",
        etc.generic_coverage.len()
    ));
    out.push_str(&format!(
        "  Missing:          {} (no coverage)\n",
        etc.missing.len()
    ));
    out.push_str(&format!(
        "  Extra TS rules:   {} (not mapped to spec types)\n\n",
        etc.extra_rules.len()
    ));

    if !etc.explicit_rules.is_empty() {
        out.push_str("  Explicit matches:\n");
        for m in &etc.explicit_rules {
            out.push_str(&format!("    {} -> {}\n", m.spec_name, m.treesitter_name));
        }
        out.push('\n');
    }

    if !etc.generic_coverage.is_empty() {
        out.push_str("  Generic fallback (definition/usage):\n");
        for name in &etc.generic_coverage {
            out.push_str(&format!("    {}\n", name));
        }
        out.push('\n');
    }

    if !etc.missing.is_empty() {
        out.push_str("  MISSING from tree-sitter:\n");
        for name in &etc.missing {
            let expected = spec_to_treesitter_name(name);
            out.push_str(&format!("    {} (expected: {})\n", name, expected));
        }
        out.push('\n');
    }

    if !etc.extra_rules.is_empty() {
        out.push_str("  Extra tree-sitter rules (no spec counterpart):\n");
        for name in &etc.extra_rules {
            out.push_str(&format!("    {}\n", name));
        }
        out.push('\n');
    }

    // --- Enum coverage (TTL) ---
    out.push_str("───────────────────────────────────────────────────────────\n");
    out.push_str("ENUM COVERAGE (vs TTL vocabulary):\n");
    format_enum_coverage(&mut out, &report.enum_coverage_ttl);

    // --- Enum coverage (xtext) ---
    if let Some(ref xtext_cov) = report.enum_coverage_xtext {
        out.push_str("───────────────────────────────────────────────────────────\n");
        out.push_str("ENUM COVERAGE (vs xtext grammar):\n");
        format_enum_coverage(&mut out, xtext_cov);
    }

    out.push_str("═══════════════════════════════════════════════════════════\n");
    out
}

fn format_enum_coverage(out: &mut String, cov: &EnumCoverage) {
    out.push_str(&format!("  Matching: {}\n", cov.matching.len()));
    out.push_str(&format!("  Spec-only: {}\n", cov.spec_only.len()));
    out.push_str(&format!("  Grammar-only: {}\n", cov.grammar_only.len()));
    out.push_str(&format!(
        "  Value mismatches: {}\n\n",
        cov.value_mismatches.len()
    ));

    if !cov.matching.is_empty() {
        out.push_str("  Matching enums:\n");
        for m in &cov.matching {
            out.push_str(&format!("    {}: {:?}\n", m.name, m.values));
        }
        out.push('\n');
    }

    if !cov.spec_only.is_empty() {
        out.push_str("  MISSING from grammar (in spec only):\n");
        for name in &cov.spec_only {
            out.push_str(&format!("    {}\n", name));
        }
        out.push('\n');
    }

    if !cov.grammar_only.is_empty() {
        out.push_str("  Extra in grammar (not in spec):\n");
        for name in &cov.grammar_only {
            out.push_str(&format!("    {}\n", name));
        }
        out.push('\n');
    }

    if !cov.value_mismatches.is_empty() {
        out.push_str("  VALUE MISMATCHES:\n");
        for m in &cov.value_mismatches {
            out.push_str(&format!("    {}:\n", m.name));
            out.push_str(&format!("      spec:    {:?}\n", m.spec_values));
            out.push_str(&format!("      grammar: {:?}\n", m.grammar_values));
            if !m.missing_from_grammar.is_empty() {
                out.push_str(&format!(
                    "      missing from grammar: {:?}\n",
                    m.missing_from_grammar
                ));
            }
            if !m.extra_in_grammar.is_empty() {
                out.push_str(&format!(
                    "      extra in grammar:     {:?}\n",
                    m.extra_in_grammar
                ));
            }
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_name_conversion() {
        assert_eq!(spec_to_treesitter_name("PartDefinition"), "part_def");
        assert_eq!(spec_to_treesitter_name("PartUsage"), "standard_usage");
        assert_eq!(spec_to_treesitter_name("ActionDefinition"), "action_def");
        assert_eq!(
            spec_to_treesitter_name("FlowConnectionUsage"),
            "flow_connection_usage"
        );
        assert_eq!(
            spec_to_treesitter_name("TransitionUsage"),
            "transition_usage"
        );
        assert_eq!(
            spec_to_treesitter_name("AllocationDefinition"),
            "allocation_def"
        );
    }

    #[test]
    fn classify_types_basic() {
        let part_def = TypeInfo {
            name: "PartDefinition".to_string(),
            supertypes: vec!["ItemDefinition".to_string()],
            comment: None,
        };
        assert_eq!(classify_type(&part_def), TypeCategory::Definition);

        let part_usage = TypeInfo {
            name: "PartUsage".to_string(),
            supertypes: vec!["ItemUsage".to_string()],
            comment: None,
        };
        assert_eq!(classify_type(&part_usage), TypeCategory::Usage);

        let specialization = TypeInfo {
            name: "Specialization".to_string(),
            supertypes: vec!["Relationship".to_string()],
            comment: None,
        };
        assert_eq!(classify_type(&specialization), TypeCategory::Relationship);

        let element = TypeInfo {
            name: "Element".to_string(),
            supertypes: vec![],
            comment: None,
        };
        assert_eq!(classify_type(&element), TypeCategory::Abstract);
    }

    #[test]
    fn parse_grammar_enums_basic() {
        let content = r#"
const enums = {
  "FeatureDirectionKind": ["in", "inout", "out"],
  "VisibilityKind": ["expose", "private", "protected", "public"],
};
"#;
        let tmp = tempfile(content);
        let result = parse_grammar_enums(tmp.path()).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(
            result.get("FeatureDirectionKind").unwrap(),
            &vec!["in", "inout", "out"]
        );
        assert_eq!(
            result.get("VisibilityKind").unwrap(),
            &vec!["expose", "private", "protected", "public"]
        );
    }

    #[test]
    fn parse_modular_enums_object() {
        // The live grammar generates enums into generated/enums.js as a
        // `module.exports = { ... };` object (SUP-02). Extraction must read it.
        let content = r#"
module.exports = {
  "FeatureDirectionKind": ["inout", "out", "in"],
  "VisibilityKind": ["protected", "private", "public", "expose"],
};
"#;
        let result = parse_js_enum_object(content, |t| t.starts_with("module.exports = {"));
        assert_eq!(result.len(), 2);
        assert_eq!(
            result.get("FeatureDirectionKind").unwrap(),
            &vec!["inout", "out", "in"]
        );
    }

    /// Helper to create a temporary file for testing.
    fn tempfile(content: &str) -> TempFile {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "ts_validation_test_{}.js",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        TempFile(path)
    }

    struct TempFile(std::path::PathBuf);
    impl TempFile {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}
