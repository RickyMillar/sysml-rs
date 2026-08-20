//! Generator for model-tree classification (Archetype + user-facing noise).
//!
//! Reads `archetype_rules.toml` (single source of truth) and the SysML/KerML
//! type hierarchy (same TTL files `hierarchy_generator` consumes), emits a
//! Rust source string for `sysml-service` and a TypeScript source string for
//! the simulation FE test fixtures.
//!
//! Validation is strict: any `ElementKind` variant that doesn't classify
//! into an archetype, isn't noise, and isn't in the explicit `Other`
//! allowlist fails the build with a listing of every uncategorized kind.
//! Spec extensions that add new variants therefore force a conscious
//! routing decision rather than silently drifting into `Other`.
//!
//! Usage:
//! ```ignore
//! use sysml_codegen::archetype_generator;
//! let rust_code = archetype_generator::generate_classification_rust(
//!     "crates/lang/codegen/src/archetype_rules.toml",
//! )?;
//! ```
//!
//! The Rust output declares two free functions:
//! ```ignore
//! pub(crate) fn archetype_for(kind: &ElementKind) -> Archetype { ... }
//! pub(crate) fn is_user_facing_noise_for(kind: &ElementKind) -> bool { ... }
//! ```
//! Both are flat `match` arms — runtime cost is a single jump.

use crate::hierarchy_generator::TypeHierarchy;
use crate::ttl_parser::{parse_ttl_vocab, TypeInfo};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Errors produced by archetype rule loading / validation.
#[derive(Debug, thiserror::Error)]
pub enum ArchetypeGenError {
    #[error("failed to read archetype rules at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse archetype rules: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("failed to read TTL file at {path}: {source}")]
    TtlRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse TTL: {0}")]
    Ttl(String),
    #[error(
        "archetype rules validation failed: {count} ElementKind variant(s) are uncategorized.\n\
         These variants are NOT classified into any archetype, NOT noise, and NOT in `[other].explicit_other`.\n\
         Either: (a) add an archetype anchor that catches them, (b) add them to `[noise].explicit_hide`, \
         or (c) add them to `[other].explicit_other`.\n\
         Uncategorized kinds: {kinds:?}"
    )]
    Uncategorized { count: usize, kinds: Vec<String> },
    #[error("archetype rules reference unknown anchor `{anchor}` in archetype `{archetype}`")]
    UnknownAnchor { archetype: String, anchor: String },
    #[error("could not locate references/sysmlv2 directory; set SYSML_REFS_DIR or run from repo root")]
    RefsNotFound,
}

/// Top-level rules schema (matches archetype_rules.toml).
#[derive(Debug, Deserialize)]
struct RulesFile {
    #[serde(default)]
    archetypes: Vec<ArchetypeRule>,
    #[serde(default)]
    noise: NoiseRules,
    #[serde(default)]
    other: OtherRules,
}

#[derive(Debug, Deserialize)]
struct ArchetypeRule {
    name: String,
    #[serde(default)]
    anchors: Vec<String>,
    #[serde(default)]
    exact: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct NoiseRules {
    #[serde(default)]
    predicates: Vec<String>,
    #[serde(default)]
    explicit_hide: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OtherRules {
    #[serde(default)]
    explicit_other: Vec<String>,
}

/// Final classification of a kind for emission. `archetype` and `is_noise` are
/// orthogonal: a kind that matches an archetype anchor AND is in
/// `explicit_hide` carries both labels. The two emitted functions
/// (`archetype_for` / `is_user_facing_noise_for`) are queried independently
/// at runtime — `is_user_facing_noise_for` filters first, so noise kinds
/// never reach `archetype_for` in `user_facing` view; in `full` view they
/// surface with their archetype intact.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KindLabel {
    /// Archetype name (e.g. `"Part"`), if any anchor matched.
    archetype: Option<String>,
    /// True if any noise predicate or `explicit_hide` matched.
    is_noise: bool,
    /// True if the kind is in `[other].explicit_other`.
    is_explicit_other: bool,
}

impl KindLabel {
    fn is_categorized(&self) -> bool {
        self.archetype.is_some() || self.is_noise || self.is_explicit_other
    }
}

/// Public Rust-emit entry point. Internally loads TTL hierarchy.
pub fn generate_classification_rust(rules_path: impl AsRef<Path>) -> Result<String, ArchetypeGenError> {
    let (rules, kerml, sysml) = load_inputs(rules_path.as_ref())?;
    let kinds = collect_all_kinds(&kerml, &sysml);
    let hierarchy = TypeHierarchy::new(&kerml, &sysml);
    validate_anchors(&rules, &hierarchy)?;
    let classification = classify_all(&rules, &kinds, &hierarchy)?;
    Ok(emit_rust(&archetype_names(&rules), &classification))
}

/// Public TypeScript-emit entry point. Internally loads TTL hierarchy.
pub fn generate_classification_ts(rules_path: impl AsRef<Path>) -> Result<String, ArchetypeGenError> {
    let (rules, kerml, sysml) = load_inputs(rules_path.as_ref())?;
    let kinds = collect_all_kinds(&kerml, &sysml);
    let hierarchy = TypeHierarchy::new(&kerml, &sysml);
    validate_anchors(&rules, &hierarchy)?;
    let classification = classify_all(&rules, &kinds, &hierarchy)?;
    Ok(emit_ts(&archetype_names(&rules), &classification))
}

/// Archetype names in rules-file order — the ONE ordering both emitters
/// consume. Emission previously iterated a hardcoded name list, which
/// SILENTLY dropped every kind classified under a rules-file archetype
/// the list didn't know (the kinds fell through to `Other`/`'other'`
/// with no error) — adding a new `[[archetypes]]` entry looked like it
/// worked while producing a wrong classifier. Deriving the order from
/// the rules file removes the duplicate source of truth entirely.
fn archetype_names(rules: &RulesFile) -> Vec<String> {
    rules.archetypes.iter().map(|a| a.name.clone()).collect()
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

fn load_inputs(
    rules_path: &Path,
) -> Result<(RulesFile, Vec<TypeInfo>, Vec<TypeInfo>), ArchetypeGenError> {
    let rules_text = std::fs::read_to_string(rules_path).map_err(|e| ArchetypeGenError::Read {
        path: rules_path.to_path_buf(),
        source: e,
    })?;
    let rules: RulesFile = toml::from_str(&rules_text)?;

    let refs_dir = find_references_dir()?;
    let kerml_path = refs_dir.join("Kerml-Vocab.ttl");
    let sysml_path = refs_dir.join("SysML-vocab.ttl");

    let kerml_text =
        std::fs::read_to_string(&kerml_path).map_err(|e| ArchetypeGenError::TtlRead {
            path: kerml_path.clone(),
            source: e,
        })?;
    let sysml_text =
        std::fs::read_to_string(&sysml_path).map_err(|e| ArchetypeGenError::TtlRead {
            path: sysml_path.clone(),
            source: e,
        })?;

    let kerml = parse_ttl_vocab(&kerml_text).map_err(|e| ArchetypeGenError::Ttl(e.to_string()))?;
    let sysml = parse_ttl_vocab(&sysml_text).map_err(|e| ArchetypeGenError::Ttl(e.to_string()))?;

    Ok((rules, kerml, sysml))
}

/// Discover `references/sysmlv2/` using the same logic sysml-core's build.rs uses.
fn find_references_dir() -> Result<PathBuf, ArchetypeGenError> {
    if let Ok(dir) = std::env::var("SYSML_REFS_DIR") {
        let path = PathBuf::from(dir);
        if path.exists() {
            return Ok(path);
        }
    }

    // Walk upward from CARGO_MANIFEST_DIR (build.rs case) or cwd (binary case)
    // looking for `references/sysmlv2`. Try up to 6 ancestors so callers from
    // any nesting depth in the workspace reach the repo root.
    let starts: Vec<PathBuf> = [
        std::env::var("CARGO_MANIFEST_DIR").ok().map(PathBuf::from),
        std::env::current_dir().ok(),
    ]
    .into_iter()
    .flatten()
    .collect();

    for start in starts {
        let mut cur: Option<&Path> = Some(start.as_path());
        for _ in 0..8 {
            if let Some(c) = cur {
                let candidate = c.join("references").join("sysmlv2");
                if candidate.exists() {
                    return Ok(candidate);
                }
                cur = c.parent();
            } else {
                break;
            }
        }
    }

    Err(ArchetypeGenError::RefsNotFound)
}

// ---------------------------------------------------------------------------
// Hierarchy
// ---------------------------------------------------------------------------

fn collect_all_kinds(kerml: &[TypeInfo], sysml: &[TypeInfo]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for t in kerml.iter().chain(sysml.iter()) {
        if seen.insert(t.name.clone()) {
            out.push(t.name.clone());
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Anchor validation
// ---------------------------------------------------------------------------

fn validate_anchors(
    rules: &RulesFile,
    hierarchy: &TypeHierarchy,
) -> Result<(), ArchetypeGenError> {
    for arch in &rules.archetypes {
        for anchor in arch.anchors.iter().chain(arch.exact.iter()) {
            if !hierarchy.contains_type(anchor) {
                return Err(ArchetypeGenError::UnknownAnchor {
                    archetype: arch.name.clone(),
                    anchor: anchor.clone(),
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Predicate hooks driven by `[noise].predicates`. Mirror the boolean
/// expressions on `ElementKind` in the generated module.
fn matches_noise_predicate(
    predicate: &str,
    kind: &str,
    hierarchy: &TypeHierarchy,
    syntax_keyword_map: &HashSet<&str>,
) -> bool {
    match predicate {
        "membership_subtree" => kind == "Membership" || hierarchy.is_subtype_of(kind, "Membership"),
        "type_binding" => {
            // is_relationship && !is_usage && !is_definition
            let is_rel =
                kind == "Relationship" || hierarchy.is_subtype_of(kind, "Relationship");
            let is_def = kind.ends_with("Definition");
            let is_use = kind.ends_with("Usage");
            is_rel && !is_def && !is_use
        }
        "control_node" => matches!(kind, "ForkNode" | "JoinNode" | "DecisionNode" | "MergeNode"),
        "no_syntax_keyword" => !syntax_keyword_map.contains(kind),
        // Unknown predicates are conservatively false; rules-file owner can
        // notice via the uncategorized validation if needed.
        _ => false,
    }
}

fn classify_kind(
    kind: &str,
    rules: &RulesFile,
    hierarchy: &TypeHierarchy,
    syntax_keywords: &HashSet<&str>,
    explicit_hide: &HashSet<&str>,
    explicit_other: &HashSet<&str>,
) -> KindLabel {
    // Archetype: walk top-to-bottom, first match wins.
    let mut archetype: Option<String> = None;
    for arch in &rules.archetypes {
        for anchor in &arch.anchors {
            if kind == anchor || hierarchy.is_subtype_of(kind, anchor) {
                archetype = Some(arch.name.clone());
                break;
            }
        }
        if archetype.is_some() {
            break;
        }
        for exact in &arch.exact {
            if kind == exact {
                archetype = Some(arch.name.clone());
                break;
            }
        }
        if archetype.is_some() {
            break;
        }
    }

    // Noise: any predicate or explicit_hide membership marks the kind.
    // Independent of archetype — `archetype_for` and `is_user_facing_noise_for`
    // are queried separately at runtime, so a kind can be both.
    let mut is_noise = false;
    for pred in &rules.noise.predicates {
        if matches_noise_predicate(pred, kind, hierarchy, syntax_keywords) {
            is_noise = true;
            break;
        }
    }
    if !is_noise && explicit_hide.contains(kind) {
        is_noise = true;
    }

    let is_explicit_other = explicit_other.contains(kind);

    KindLabel {
        archetype,
        is_noise,
        is_explicit_other,
    }
}

/// Classify every kind. Returns one (kind, label) per input.
/// Returns Err with the full list of uncategorized variants if validation fails.
fn classify_all(
    rules: &RulesFile,
    kinds: &[String],
    hierarchy: &TypeHierarchy,
) -> Result<Vec<(String, KindLabel)>, ArchetypeGenError> {
    // The one keyword table lives in hierarchy_generator (it also feeds the
    // generated ElementKind::syntax_keyword()); build the lookup from it so
    // the two consumers can never drift.
    let syntax_keywords: HashSet<&str> = crate::hierarchy_generator::SYNTAX_KEYWORDS
        .iter()
        .map(|(kind, _)| *kind)
        .collect();
    let explicit_hide: HashSet<&str> =
        rules.noise.explicit_hide.iter().map(String::as_str).collect();
    let explicit_other: HashSet<&str> = rules
        .other
        .explicit_other
        .iter()
        .map(String::as_str)
        .collect();

    let mut out: Vec<(String, KindLabel)> = Vec::with_capacity(kinds.len());
    let mut uncategorized: Vec<String> = Vec::new();

    for kind in kinds {
        let label = classify_kind(
            kind,
            rules,
            hierarchy,
            &syntax_keywords,
            &explicit_hide,
            &explicit_other,
        );
        if !label.is_categorized() {
            uncategorized.push(kind.clone());
        }
        out.push((kind.clone(), label));
    }

    if !uncategorized.is_empty() {
        uncategorized.sort();
        return Err(ArchetypeGenError::Uncategorized {
            count: uncategorized.len(),
            kinds: uncategorized,
        });
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Rust emission
// ---------------------------------------------------------------------------

fn emit_rust(archetype_order: &[String], classification: &[(String, KindLabel)]) -> String {
    let mut out = String::new();
    out.push_str("// AUTO-GENERATED by sysml-codegen — do not edit. Source:\n");
    out.push_str("// crates/lang/codegen/src/archetype_rules.toml\n");
    out.push_str("//\n");
    out.push_str("// Two free functions, both flat `match` arms:\n");
    out.push_str("//   - archetype_for(&ElementKind) -> Archetype\n");
    out.push_str("//   - is_user_facing_noise_for(&ElementKind) -> bool\n");
    out.push_str("//\n");
    out.push_str("// The hierarchy walks happen at codegen time so runtime cost is one jump.\n");
    out.push_str("// `archetype_for` and `is_user_facing_noise_for` are independent — a\n");
    out.push_str("// kind can match an archetype anchor AND be in `[noise].explicit_hide`.\n");
    out.push_str("// In `user_facing` view the noise filter runs first; in `full` view both\n");
    out.push_str("// surface together.\n");
    out.push_str("//\n");
    out.push_str("// Designed to be `include!`d into a module that already has `ElementKind`\n");
    out.push_str("// and `Archetype` in scope; no `use` statements are emitted.\n\n");

    // Group archetype kinds (independent of noise).
    let mut by_archetype: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut noise: Vec<&str> = Vec::new();
    for (kind, label) in classification {
        if let Some(name) = &label.archetype {
            by_archetype
                .entry(name.as_str())
                .or_default()
                .push(kind.as_str());
        }
        if label.is_noise {
            noise.push(kind.as_str());
        }
    }

    out.push_str("pub(crate) fn archetype_for(kind: &ElementKind) -> Archetype {\n");
    out.push_str("    match kind {\n");

    // Stable order — rules-file order, threaded in by the caller so a
    // new `[[archetypes]]` entry can never be silently skipped. Every
    // classified name comes from the same rules file, so this loop is
    // exhaustive over `by_archetype` by construction; the debug_assert
    // backstops that invariant.
    debug_assert!(
        by_archetype
            .keys()
            .all(|k| archetype_order.iter().any(|n| n == k)),
        "classified archetype missing from rules-file order: {:?}",
        by_archetype.keys().collect::<Vec<_>>()
    );
    for name in archetype_order {
        if let Some(kinds) = by_archetype.get(name.as_str()) {
            let mut kinds = kinds.clone();
            kinds.sort();
            for k in kinds {
                out.push_str(&format!(
                    "        ElementKind::{} => Archetype::{},\n",
                    k, name
                ));
            }
        }
    }
    // Everything else (noise-only, explicit-other, anything not anchored)
    // lands in Archetype::Other.
    out.push_str("        _ => Archetype::Other,\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    out.push_str("pub(crate) fn is_user_facing_noise_for(kind: &ElementKind) -> bool {\n");
    if noise.is_empty() {
        out.push_str("    let _ = kind;\n");
        out.push_str("    false\n");
    } else {
        let mut sorted = noise.clone();
        sorted.sort();
        out.push_str("    matches!(kind,\n");
        for (i, k) in sorted.iter().enumerate() {
            let sep = if i + 1 == sorted.len() { "" } else { " |" };
            out.push_str(&format!("        ElementKind::{}{}\n", k, sep));
        }
        out.push_str("    )\n");
    }
    out.push_str("}\n");

    out
}

// ---------------------------------------------------------------------------
// TS emission
// ---------------------------------------------------------------------------

fn archetype_lower(name: &str) -> &'static str {
    match name {
        "Part" => "part",
        "Port" => "port",
        "Attribute" => "attribute",
        "Sm" => "sm",
        "Action" => "action",
        "Case" => "case",
        "Constraint" => "constraint",
        "Calc" => "calc",
        "Connection" => "connection",
        "Section" => "section",
        // Fail hard at codegen time. The old `_ => "other"` fallback
        // silently mislabelled kinds whose archetype this map didn't
        // know — the FE mirror then disagreed with the Rust enum.
        other => panic!(
            "archetype `{other}` has no TS lowercase mapping — add it to \
             archetype_lower (and an `Archetype` enum variant in \
             sysml-service::types)"
        ),
    }
}

fn emit_ts(archetype_order: &[String], classification: &[(String, KindLabel)]) -> String {
    let mut out = String::new();
    out.push_str("// AUTO-GENERATED by sysml-codegen — do not edit. Source:\n");
    out.push_str("// crates/lang/codegen/src/archetype_rules.toml\n");
    out.push_str("//\n");
    out.push_str("// Run `cargo run -p sysml-codegen --bin emit-ts-classification` to regenerate.\n\n");
    // The union is derived from the rules-file archetypes (in rules
    // order) plus the two names that never appear in the rules:
    // `section` (FE-injected split rows) and `other` (the fallback).
    out.push_str("export type Archetype =\n");
    for name in archetype_order {
        out.push_str(&format!("  | '{}'\n", archetype_lower(name)));
    }
    out.push_str("  | 'section'\n");
    out.push_str("  | 'other';\n\n");

    // ARCHETYPE_FOR_KIND — rules-file order, threaded in by the caller
    // so a new `[[archetypes]]` entry can never be silently skipped
    // (the old hardcoded list dropped unknown archetypes' kinds to
    // 'other' with no error).
    out.push_str("export const ARCHETYPE_FOR_KIND: Record<string, Archetype> = {\n");
    let mut by_archetype: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut noise_kinds: Vec<&str> = Vec::new();
    for (kind, label) in classification {
        if let Some(name) = &label.archetype {
            by_archetype
                .entry(name.as_str())
                .or_default()
                .push(kind.as_str());
        }
        if label.is_noise {
            noise_kinds.push(kind.as_str());
        }
    }
    debug_assert!(
        by_archetype
            .keys()
            .all(|k| archetype_order.iter().any(|n| n == k)),
        "classified archetype missing from rules-file order: {:?}",
        by_archetype.keys().collect::<Vec<_>>()
    );
    for name in archetype_order {
        if let Some(kinds) = by_archetype.get(name.as_str()) {
            let mut kinds = kinds.clone();
            kinds.sort();
            let lower = archetype_lower(name);
            for k in kinds {
                out.push_str(&format!("  {}: '{}',\n", k, lower));
            }
        }
    }
    out.push_str("};\n\n");

    // IS_USER_FACING_NOISE
    out.push_str("export const IS_USER_FACING_NOISE: Record<string, true> = {\n");
    noise_kinds.sort();
    for k in &noise_kinds {
        out.push_str(&format!("  {}: true,\n", k));
    }
    out.push_str("};\n\n");

    out.push_str("export function archetypeForKind(kind: string): Archetype {\n");
    out.push_str("  return ARCHETYPE_FOR_KIND[kind] ?? 'other';\n");
    out.push_str("}\n\n");

    out.push_str("export function isUserFacingNoise(kind: string): boolean {\n");
    out.push_str("  return IS_USER_FACING_NOISE[kind] === true;\n");
    out.push_str("}\n");

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_type(name: &str, supertypes: &[&str]) -> TypeInfo {
        TypeInfo {
            name: name.to_owned(),
            supertypes: supertypes.iter().map(|s| (*s).to_owned()).collect(),
            comment: None,
        }
    }
    use std::path::PathBuf;

    fn rules_path() -> PathBuf {
        // tests run from CARGO_MANIFEST_DIR = crates/lang/codegen
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        PathBuf::from(manifest).join("src/archetype_rules.toml")
    }

    #[test]
    fn rules_file_loads_and_validates_against_current_hierarchy() {
        // Smoke test: bundled rules must classify every variant in the
        // current TTL hierarchy. Adding an ElementKind that doesn't fit
        // anywhere should fail this test.
        let rust_code = generate_classification_rust(rules_path())
            .expect("archetype rules must validate against current hierarchy");

        // Sanity: emitted Rust contains both functions.
        assert!(rust_code.contains("pub(crate) fn archetype_for("));
        assert!(rust_code.contains("pub(crate) fn is_user_facing_noise_for("));
        // And at least one well-known variant per archetype.
        assert!(rust_code.contains("ElementKind::PartUsage => Archetype::Part"));
        assert!(rust_code.contains("ElementKind::PortUsage => Archetype::Port"));
        assert!(rust_code.contains("ElementKind::ConnectionUsage => Archetype::Connection"));
        // Case family — regression pin for the silently-dropped-archetype
        // bug: emission used to iterate a hardcoded name list, so kinds
        // classified under a newer rules-file archetype (`Case`) fell to
        // `_ => Other` with no error.
        assert!(rust_code.contains("ElementKind::CaseUsage => Archetype::Case"));
        assert!(rust_code.contains("ElementKind::VerificationCaseUsage => Archetype::Case"));
        assert!(rust_code.contains("ElementKind::AnalysisCaseDefinition => Archetype::Case"));
        // …and the Case rule must not swallow plain actions.
        assert!(rust_code.contains("ElementKind::ActionUsage => Archetype::Action"));
    }

    #[test]
    fn ts_emit_works() {
        let ts_code = generate_classification_ts(rules_path()).expect("ts emit");
        assert!(ts_code.contains("export type Archetype"));
        assert!(ts_code.contains("PartUsage: 'part'"));
        assert!(ts_code.contains("PortUsage: 'port'"));
        assert!(ts_code.contains("ConnectionUsage: 'connection'"));
        // Case family present in the TS mirror + its union entry.
        assert!(ts_code.contains("VerificationCaseUsage: 'case'"));
        assert!(ts_code.contains("| 'case'"));
        assert!(ts_code.contains("export function archetypeForKind"));
    }

    #[test]
    fn every_rules_archetype_reaches_emission() {
        // Regression for the hardcoded-order bug: every archetype named
        // in the rules file must contribute at least one match arm to
        // the emitted Rust (a name whose kinds all vanish means the
        // emitter skipped it).
        let rules_text = std::fs::read_to_string(rules_path()).unwrap();
        let rules: RulesFile = toml::from_str(&rules_text).unwrap();
        let rust_code = generate_classification_rust(rules_path()).unwrap();
        for arch in &rules.archetypes {
            assert!(
                rust_code.contains(&format!("=> Archetype::{},", arch.name)),
                "rules-file archetype `{}` emitted no match arms",
                arch.name
            );
        }
    }

    #[test]
    fn uncategorized_kind_fails_with_clear_message() {
        // Synthesise a tiny hierarchy where one kind is unreachable.
        let rules: RulesFile = toml::from_str(
            r#"
[[archetypes]]
name = "Part"
anchors = ["ItemUsage"]

[noise]
predicates = []
explicit_hide = []

[other]
explicit_other = []
"#,
        )
        .unwrap();

        let types = vec![
            make_type("ItemUsage", &[]),
            make_type("PartUsage", &["ItemUsage"]),
            // Stranded: not a subtype of ItemUsage, not in any list.
            make_type("Stranded", &[]),
        ];
        let hierarchy = TypeHierarchy::new(&types, &[]);

        let kinds = vec![
            "ItemUsage".to_owned(),
            "PartUsage".to_owned(),
            "Stranded".to_owned(),
        ];
        let err = classify_all(&rules, &kinds, &hierarchy).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Stranded"), "message: {msg}");
        assert!(msg.contains("uncategorized"), "message: {msg}");
    }

    #[test]
    fn unknown_anchor_in_rules_is_reported() {
        let rules: RulesFile = toml::from_str(
            r#"
[[archetypes]]
name = "Bogus"
anchors = ["DoesNotExist"]
"#,
        )
        .unwrap();

        let types = vec![make_type("ItemUsage", &[])];
        let hierarchy = TypeHierarchy::new(&types, &[]);

        let err = validate_anchors(&rules, &hierarchy).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("DoesNotExist"), "message: {msg}");
        assert!(msg.contains("Bogus"), "message: {msg}");
    }
}
