#![allow(clippy::print_stdout)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use sysml_codegen::{parse_ttl_vocab, parse_xtext_rules, XtextRule};

/// Paths to xtext specification files relative to the references directory.
const SYSML_XTEXT_PATH: &str =
    "SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/src/org/omg/sysml/xtext/SysML.xtext";
const KERML_XTEXT_PATH: &str =
    "SysML-v2-Pilot-Implementation/org.omg.kerml.xtext/src/org/omg/kerml/xtext/KerML.xtext";
const KERML_EXPRESSIONS_PATH: &str = "SysML-v2-Pilot-Implementation/org.omg.kerml.expressions.xtext/src/org/omg/kerml/expressions/xtext/KerMLExpressions.xtext";

const GRAMMAR_PATH: &str = "tree-sitter/grammar.js";
const GRAMMAR_JSON_PATH: &str = "tree-sitter/src/grammar.json";

fn main() -> Result<(), Box<dyn Error>> {
    let refs_dir = find_references_dir().ok_or("Could not find references/sysmlv2 directory")?;

    let sysml_xtext = fs::read_to_string(refs_dir.join(SYSML_XTEXT_PATH))?;
    let kerml_xtext = fs::read_to_string(refs_dir.join(KERML_XTEXT_PATH))?;
    let kerml_expr = fs::read_to_string(refs_dir.join(KERML_EXPRESSIONS_PATH))?;

    let mut xtext_rules_full = Vec::new();
    xtext_rules_full.extend(parse_xtext_rules(&sysml_xtext));
    xtext_rules_full.extend(parse_xtext_rules(&kerml_xtext));
    xtext_rules_full.extend(parse_xtext_rules(&kerml_expr));

    let xtext_element_rules = build_xtext_element_rules(&xtext_rules_full);

    let element_kind_names = load_element_kinds(&refs_dir)?;

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let tree_rules = load_tree_sitter_rule_names(&manifest_dir)?;

    report_coverage(&tree_rules, &xtext_element_rules, &element_kind_names);

    Ok(())
}

fn load_tree_sitter_rule_names(manifest_dir: &Path) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let grammar_json_path = manifest_dir.join(GRAMMAR_JSON_PATH);
    if grammar_json_path.exists() {
        let grammar_json = fs::read_to_string(&grammar_json_path)?;
        if let Some(rules) = extract_tree_sitter_rule_names_from_json(&grammar_json) {
            return Ok(rules);
        }
    }

    let grammar_path = manifest_dir.join(GRAMMAR_PATH);
    let grammar = fs::read_to_string(&grammar_path)?;
    Ok(extract_tree_sitter_rule_names(&grammar))
}

fn report_coverage(
    tree_rules: &BTreeSet<String>,
    xtext_element_rules: &BTreeMap<String, String>,
    element_kind_names: &BTreeSet<String>,
) {
    let xtext_snake: BTreeMap<String, String> = xtext_element_rules
        .keys()
        .map(|name| (to_snake_case(name), name.clone()))
        .collect();

    let element_snake: BTreeMap<String, String> = element_kind_names
        .iter()
        .map(|name| (to_snake_case(name), name.clone()))
        .collect();

    let xtext_stats = analyze_coverage(&xtext_snake, tree_rules);
    let element_stats = analyze_coverage(&element_snake, tree_rules);

    println!("Tree-sitter coverage report");
    println!("  grammar rules: {}", tree_rules.len());
    println!(
        "  xtext element rules: {} covered / {} total",
        xtext_stats.covered(),
        xtext_stats.total
    );
    println!(
        "    breakdown: direct={}, alias={}, merged={}",
        xtext_stats.direct, xtext_stats.alias, xtext_stats.merged
    );
    println!(
        "    likely semantic-only unmatched: {}",
        xtext_stats.likely_semantic_only.len()
    );
    println!(
        "  element kinds: {} covered / {} total",
        element_stats.covered(),
        element_stats.total
    );
    println!(
        "    breakdown: direct={}, alias={}, merged={}",
        element_stats.direct, element_stats.alias, element_stats.merged
    );
    println!(
        "    likely semantic-only unmatched: {}",
        element_stats.likely_semantic_only.len()
    );
    println!(
        "  note: coverage is name-matched with alias/merge heuristics, not full semantic parity"
    );

    let show_missing = env::var("SYSML_TS_SHOW_MISSING").is_ok();

    if show_missing {
        if !xtext_stats.missing.is_empty() {
            println!(
                "\nMissing tree-sitter rules for Xtext element rules (after alias/merge mapping):"
            );
            for name in &xtext_stats.missing {
                println!("  - {}", name);
            }
        }
        if !xtext_stats.likely_semantic_only.is_empty() {
            println!("\nLikely semantic-only Xtext element rules (not expected as direct grammar rules):");
            for name in &xtext_stats.likely_semantic_only {
                println!("  - {}", name);
            }
        }
        if !element_stats.missing.is_empty() {
            println!("\nMissing tree-sitter rules for ElementKinds (after alias/merge mapping):");
            for name in &element_stats.missing {
                println!("  - {}", name);
            }
        }
        if !element_stats.likely_semantic_only.is_empty() {
            println!("\nLikely semantic-only ElementKinds (not expected as direct grammar rules):");
            for name in &element_stats.likely_semantic_only {
                println!("  - {}", name);
            }
        }
    } else {
        if !xtext_stats.missing.is_empty() {
            println!(
                "  note: {} xtext element rules still missing after alias/merge mapping (set SYSML_TS_SHOW_MISSING=1 to list)",
                xtext_stats.missing.len()
            );
        }
        if !xtext_stats.likely_semantic_only.is_empty() {
            println!(
                "  note: {} xtext element rules are likely semantic-only (set SYSML_TS_SHOW_MISSING=1 to list)",
                xtext_stats.likely_semantic_only.len()
            );
        }
        if !element_stats.missing.is_empty() {
            println!(
                "  note: {} element kinds still missing after alias/merge mapping (set SYSML_TS_SHOW_MISSING=1 to list)",
                element_stats.missing.len()
            );
        }
        if !element_stats.likely_semantic_only.is_empty() {
            println!(
                "  note: {} element kinds are likely semantic-only (set SYSML_TS_SHOW_MISSING=1 to list)",
                element_stats.likely_semantic_only.len()
            );
        }
    }
}

#[derive(Default)]
struct CoverageStats {
    total: usize,
    direct: usize,
    alias: usize,
    merged: usize,
    likely_semantic_only: Vec<String>,
    missing: Vec<String>,
}

impl CoverageStats {
    fn covered(&self) -> usize {
        self.direct + self.alias + self.merged
    }
}

fn analyze_coverage(
    source_names: &BTreeMap<String, String>,
    tree_rules: &BTreeSet<String>,
) -> CoverageStats {
    let mut stats = CoverageStats {
        total: source_names.len(),
        ..Default::default()
    };

    for (snake_name, original_name) in source_names {
        if tree_rules.contains(snake_name) {
            stats.direct += 1;
            continue;
        }

        if alias_match_for_name(snake_name, tree_rules).is_some() {
            stats.alias += 1;
            continue;
        }

        if let Some(merged_rule) = merged_rule_for_name(snake_name) {
            if tree_rules.contains(merged_rule) {
                stats.merged += 1;
                continue;
            }
        }

        if is_likely_semantic_only_name(snake_name) {
            stats.likely_semantic_only.push(original_name.clone());
        } else {
            stats.missing.push(original_name.clone());
        }
    }

    stats
}

#[allow(clippy::manual_find)]
fn alias_match_for_name(name: &str, tree_rules: &BTreeSet<String>) -> Option<String> {
    let mut seen = BTreeSet::new();

    for candidate in explicit_alias_candidates(name) {
        if alias_candidate_hits(candidate, name, tree_rules, &mut seen) {
            return Some((*candidate).to_owned());
        }
    }

    for candidate in generic_alias_candidates(name) {
        if alias_candidate_hits(&candidate, name, tree_rules, &mut seen) {
            return Some(candidate);
        }
    }

    None
}

fn alias_candidate_hits(
    candidate: &str,
    source_name: &str,
    tree_rules: &BTreeSet<String>,
    seen: &mut BTreeSet<String>,
) -> bool {
    if candidate == source_name {
        return false;
    }
    if candidate.starts_with('_') && candidate != "_expression" {
        return false;
    }
    if !seen.insert(candidate.to_owned()) {
        return false;
    }
    tree_rules.contains(candidate)
}

fn explicit_alias_candidates(name: &str) -> &'static [&'static str] {
    match name {
        "action_definition" => &["action_def"],
        "state_definition" => &["state_def"],
        "requirement_definition" => &["requirement_def"],
        "constraint_definition" => &["constraint_def"],
        "enumeration_definition" => &["enum_def"],
        "for_loop_action_usage" => &["for_action"],
        "while_loop_action_usage" => &["while_action"],
        "if_action_usage" => &["if_action"],
        "send_action_usage" => &["send_action"],
        "accept_action_usage" => &["accept_action"],
        "assignment_action_usage" => &["assignment_action"],
        "instantiation_expression" => &["new_expression"],
        "feature_chain_expression" => &["feature_chain"],
        "null_expression" => &["null_literal"],
        "literal_expression" => &["literal"],
        "literal_boolean" => &["boolean_literal"],
        "literal_integer" => &["integer_literal"],
        "literal_real" => &["real_literal"],
        "literal_rational" => &["real_literal"],
        "literal_string" => &["string_literal"],
        "literal_infinity" => &["infinity_literal"],
        "type_reference" => &["type_ref"],
        "visibility_kind" => &["visibility_indicator"],
        "import" => &["import_decl"],
        "expose" => &["expose_decl"],
        "alias" => &["alias_decl"],
        "package" => &["package_decl"],
        "namespace" => &["namespace_decl"],
        "membership_import" => &["import_decl"],
        "namespace_import" => &["import_decl"],
        "membership_expose" => &["expose_decl"],
        "namespace_expose" => &["expose_decl"],
        "filter_package_import" => &["filter_package"],
        "filter_package_membership_import" => &["filter_package"],
        "filter_package_namespace_import" => &["filter_package"],
        "transition_usage_member" => &["transition_usage"],
        "target_transition_usage_member" => &["target_transition_usage"],
        "trigger_action_member" => &["trigger_action"],
        "entry_action_kind" => &["entry_action"],
        "do_action_kind" => &["do_action"],
        "exit_action_kind" => &["exit_action"],
        "trigger_invocation_expression" => &["trigger_action", "trigger_accept"],
        "metadata_access_expression" => &["member_access"],
        "operator_expression" => &["_expression"],
        "feature_reference_expression" => &["feature_chain", "qualified_name", "member_access"],
        "invariant" => &["inv_constraint"],
        _ => &[],
    }
}

fn generic_alias_candidates(name: &str) -> Vec<String> {
    let mut out = Vec::new();

    if let Some(stem) = name.strip_suffix("_definition") {
        out.push(format!("{}_def", stem));
    }
    if let Some(stem) = name.strip_suffix("_usage") {
        out.push(stem.to_owned());
        out.push(format!("{}_decl", stem));
        out.push(format!("{}_def", stem));
    }
    if let Some(stem) = name.strip_suffix("_member") {
        out.push(stem.to_owned());
        out.push(format!("{}_usage", stem));
        out.push(format!("{}_def", stem));
    }
    if let Some(stem) = name.strip_suffix("_element") {
        out.push(stem.to_owned());
        out.push(format!("{}_usage", stem));
        out.push(format!("{}_def", stem));
    }
    if let Some(stem) = name.strip_suffix("_membership") {
        out.push(stem.to_owned());
        out.push(format!("{}_usage", stem));
        out.push(format!("{}_decl", stem));
    }
    if let Some(stem) = name.strip_suffix("_kind") {
        out.push(stem.to_owned());
    }
    if let Some(stem) = name.strip_suffix("_reference") {
        out.push(stem.to_owned());
        out.push(format!("{}_ref", stem));
    }
    if let Some(stem) = name.strip_suffix("_parameter") {
        out.push(stem.to_owned());
    }
    if let Some(stem) = name.strip_prefix("owned_") {
        out.push(stem.to_owned());
    }
    if let Some(stem) = name.strip_prefix("empty_") {
        out.push(stem.to_owned());
    }
    if let Some(stem) = name.strip_prefix("literal_") {
        out.push(format!("{}_literal", stem));
        if stem == "expression" {
            out.push("literal".to_owned());
        }
    }
    if name == "expression" {
        out.push("_expression".to_owned());
    }

    out
}

fn merged_rule_for_name(name: &str) -> Option<&'static str> {
    if let Some(stem) = name.strip_suffix("_definition") {
        if matches!(
            stem,
            "part"
                | "attribute"
                | "port"
                | "connection"
                | "interface"
                | "item"
                | "allocation"
                | "occurrence"
                | "flow"
        ) {
            return Some("standard_def");
        }
        if matches!(
            stem,
            "calculation"
                | "analysis_case"
                | "verification_case"
                | "view"
                | "viewpoint"
                | "rendering"
                | "metadata"
                | "concern"
                | "stakeholder"
        ) {
            return Some("definition");
        }
    }

    if let Some(stem) = name.strip_suffix("_usage") {
        if matches!(
            stem,
            "part" | "attribute" | "item" | "occurrence" | "reference"
        ) {
            return Some("standard_usage");
        }
        if matches!(
            stem,
            "analysis_case"
                | "calculation"
                | "verification_case"
                | "view"
                | "viewpoint"
                | "rendering"
                | "concern"
                | "stakeholder"
                | "metadata"
        ) {
            return Some("usage");
        }
        if matches!(stem, "flow" | "succession_flow") {
            return Some("flow_connection_usage");
        }
        if stem == "perform_action" {
            return Some("control_flow_node");
        }
    }

    match name {
        "class"
        | "classifier"
        | "data_type"
        | "datatype"
        | "function"
        | "behavior"
        | "interaction"
        | "metaclass"
        | "association"
        | "association_structure"
        | "type"
        | "predicate"
        | "structure" => Some("kerml_definition"),
        "step" | "message" => Some("kerml_usage"),
        "connector_as_usage" => Some("connector_usage"),
        "binding_connector" | "binding_connector_as_usage" => Some("binding_usage"),
        "fork_node" | "join_node" | "merge_node" | "decision_node" => Some("control_flow_node"),
        "use_case_definition" => Some("case_def"),
        "use_case_usage" => Some("case_usage"),
        "succession_flow" => Some("flow_connection_usage"),
        _ => None,
    }
}

fn is_likely_semantic_only_name(name: &str) -> bool {
    name.ends_with("_member")
        || name.ends_with("_membership")
        || name.ends_with("_kind")
        || name.ends_with("_element")
        || name.starts_with("owned_")
        || name.starts_with("empty_")
        || name.starts_with("default_")
        || name.starts_with("prefix_")
}

fn build_xtext_element_rules(xtext_rules: &[XtextRule]) -> BTreeMap<String, String> {
    let mut mapping = BTreeMap::new();

    for rule in xtext_rules {
        if rule.is_fragment || rule.is_terminal {
            continue;
        }

        if let Some(ref returns_type) = rule.returns_type {
            if returns_type.starts_with("SysML::") || returns_type.starts_with("KerML::") {
                mapping.insert(rule.name.clone(), returns_type.clone());
            }
        }
    }

    mapping
}

fn load_element_kinds(refs_dir: &Path) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let kerml_vocab_path = refs_dir.join("Kerml-Vocab.ttl");
    let sysml_vocab_path = refs_dir.join("SysML-vocab.ttl");

    let mut names = BTreeSet::new();

    if kerml_vocab_path.exists() {
        let content = fs::read_to_string(&kerml_vocab_path)?;
        for t in parse_ttl_vocab(&content).unwrap_or_default() {
            names.insert(t.name);
        }
    }

    if sysml_vocab_path.exists() {
        let content = fs::read_to_string(&sysml_vocab_path)?;
        for t in parse_ttl_vocab(&content).unwrap_or_default() {
            names.insert(t.name);
        }
    }

    Ok(names)
}

fn extract_tree_sitter_rule_names(grammar: &str) -> BTreeSet<String> {
    let mut rules = BTreeSet::new();

    for line in grammar.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("=>") {
            continue;
        }
        if let Some((name, _rest)) = trimmed.split_once(':') {
            let name = name.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                rules.insert(name.to_owned());
            }
        }
    }

    rules
}

fn extract_tree_sitter_rule_names_from_json(grammar_json: &str) -> Option<BTreeSet<String>> {
    let mut rules = BTreeSet::new();
    let mut in_rules = false;
    let mut rules_depth = 0_i32;

    for line in grammar_json.lines() {
        let trimmed = line.trim();

        if !in_rules {
            if trimmed.starts_with("\"rules\"") {
                rules_depth += brace_delta(trimmed);
                if rules_depth > 0 {
                    in_rules = true;
                }
            }
            continue;
        }

        if rules_depth == 1 {
            if let Some(name) = extract_json_object_key(trimmed) {
                rules.insert(name.to_owned());
            }
        }

        rules_depth += brace_delta(trimmed);
        if rules_depth <= 0 {
            break;
        }
    }

    if rules.is_empty() {
        None
    } else {
        Some(rules)
    }
}

fn brace_delta(input: &str) -> i32 {
    let mut delta = 0_i32;
    let mut in_string = false;
    let mut escaped = false;

    for ch in input.chars() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => delta += 1,
            '}' => delta -= 1,
            _ => {}
        }
    }

    delta
}

fn extract_json_object_key(trimmed: &str) -> Option<&str> {
    if !trimmed.starts_with('"') {
        return None;
    }

    let rest = &trimmed[1..];
    let key_end = rest.find('"')?;
    let key = &rest[..key_end];
    let after = rest[(key_end + 1)..].trim_start();

    if !after.starts_with(':') {
        return None;
    }

    if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some(key)
    } else {
        None
    }
}

fn to_snake_case(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;

    for ch in name.chars() {
        if ch.is_ascii_uppercase() {
            if prev_lower {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower = false;
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_lower = true;
        } else {
            out.push('_');
            prev_lower = false;
        }
    }

    while out.contains("__") {
        out = out.replace("__", "_");
    }

    out.trim_matches('_').to_owned()
}

/// Find the sysmlv2 references directory by searching upward from the crate directory.
fn find_references_dir() -> Option<PathBuf> {
    if let Ok(refs_dir) = env::var("SYSML_REFS_DIR") {
        let path = PathBuf::from(refs_dir);
        if path.exists() {
            return Some(path);
        }
    }

    if let Ok(refs_dir) = env::var("SYSMLV2_REFS_DIR") {
        let path = PathBuf::from(refs_dir);
        if path.exists() {
            return Some(path);
        }
    }

    let mut current = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);

    for _ in 0..5 {
        let refs_path = current.join("references").join("sysmlv2");
        if refs_path.exists() && refs_path.is_dir() {
            return Some(refs_path);
        }

        if !current.pop() {
            break;
        }
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    if let Some(parent) = manifest_dir.parent() {
        if let Some(grandparent) = parent.parent() {
            let refs_path = grandparent.join("references").join("sysmlv2");
            if refs_path.exists() && refs_path.is_dir() {
                return Some(refs_path);
            }
        }
    }

    None
}
