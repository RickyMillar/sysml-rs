//! Regenerates the derived spec-reference indexes.
//!
//! Usage: `cargo run -p spec-index [-- <output-dir>]`
//! Default output dir: `references/sysmlv2/derived/` under the repo root.
// Developer-run regeneration CLI: panicking with the failing path IS the
// error UX, and stdout is the progress report (same pattern as
// generate_ts_tokens).
#![allow(clippy::print_stdout, clippy::panic)]

use std::path::{Path, PathBuf};

const KERML_HTML: &str = "SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/KerML-spec-r2025-04_REF.html";
const SYSML_HTML: &str = "SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/SysML-spec-r2025-04_REF.html";
const KERML_XTEXT: &str =
    "SysML-v2-Pilot-Implementation/org.omg.kerml.xtext/src/org/omg/kerml/xtext/KerML.xtext";
const SYSML_XTEXT: &str =
    "SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/src/org/omg/sysml/xtext/SysML.xtext";
const EXPR_XTEXT: &str = "SysML-v2-Pilot-Implementation/org.omg.kerml.expressions.xtext/src/org/omg/kerml/expressions/xtext/KerMLExpressions.xtext";

fn refs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../references/sysmlv2")
}

fn read(rel: &str) -> String {
    let path = refs_dir().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn main() {
    // `language-pack` subcommand: generate the derived language knowledge pack.
    // The default run (no subcommand) is byte-for-byte unchanged.
    let mut args = std::env::args().skip(1);
    let first = args.next();
    if first.as_deref() == Some("language-pack") {
        let repo_root = spec_index::language_pack::repo_root();
        let sub = args.next();
        // `language-pack info` prints the discovery/status JSON.
        if sub.as_deref() == Some("info") {
            let pack_dir = spec_index::language_pack::default_output_dir(&repo_root);
            match spec_index::language_pack::info::info_json(&repo_root, &pack_dir) {
                Ok(json) => {
                    print!("{json}");
                    return;
                }
                Err(e) => panic!("language-pack info failed: {e}"),
            }
        }
        // `language-pack render-mdbook <output-dir>` renders the generated
        // pack (at the default/SYSML_LP_PACK_DIR location) as mdBook pages.
        if sub.as_deref() == Some("render-mdbook") {
            let out_dir = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(|| panic!("usage: language-pack render-mdbook <output-dir>"));
            let pack_dir = spec_index::language_pack::default_output_dir(&repo_root);
            match spec_index::language_pack::render_mdbook::render(&pack_dir, &out_dir) {
                Ok(()) => return,
                Err(e) => panic!("language-pack render-mdbook failed: {e}"),
            }
        }
        // Otherwise regenerate the pack (optional explicit output dir).
        let out_dir = sub
            .map_or_else(|| spec_index::language_pack::default_output_dir(&repo_root), PathBuf::from);
        match spec_index::language_pack::run(&repo_root, &out_dir) {
            Ok(hash) => {
                println!("wrote language pack to {} (tree {hash})", out_dir.display());
                return;
            }
            Err(e) => panic!("language-pack generation failed: {e}"),
        }
    }

    // `diagnostics-registry --json`: dump the sysml-core diagnostic error-code
    // registry as JSON on stdout (consumed by website/scripts/generate-reference.mjs).
    // Runtime health families (AX/SM/FL/VC/CN/RQ/PH runtime codes) are not in
    // this registry and are deliberately not synthesized here.
    if first.as_deref() == Some("diagnostics-registry") {
        let json_flag = args.next();
        if json_flag.as_deref() != Some("--json") {
            panic!("usage: diagnostics-registry --json");
        }
        let codes: Vec<serde_json::Value> = sysml_core::error_codes::all()
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "code": entry.code,
                    "short_description": entry.short_description,
                    "category": match entry.category {
                        sysml_core::error_codes::ErrorCategory::Structural => "Structural",
                        sysml_core::error_codes::ErrorCategory::Resolution => "Resolution",
                        sysml_core::error_codes::ErrorCategory::Semantic => "Semantic",
                        sysml_core::error_codes::ErrorCategory::Validation => "Validation",
                    },
                })
            })
            .collect();
        let doc = serde_json::json!({ "codes": codes });
        match serde_json::to_string_pretty(&doc) {
            Ok(s) => {
                println!("{s}");
                return;
            }
            Err(e) => panic!("diagnostics-registry serialization failed: {e}"),
        }
    }

    let out_dir = first.map_or_else(|| refs_dir().join("derived"), PathBuf::from);
    std::fs::create_dir_all(&out_dir)
        .unwrap_or_else(|e| panic!("cannot create {}: {e}", out_dir.display()));

    for (rel, out_name) in [
        (KERML_HTML, "KerML-spec-r2025-04.txt"),
        (SYSML_HTML, "SysML-spec-r2025-04.txt"),
    ] {
        let artifact = spec_index::spec_text_artifact(rel, &read(rel));
        let out_path = out_dir.join(out_name);
        std::fs::write(&out_path, artifact)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", out_path.display()));
        println!("wrote {}", out_path.display());
    }

    let kerml = read(KERML_XTEXT);
    let sysml = read(SYSML_XTEXT);
    let expr = read(EXPR_XTEXT);
    let artifact = spec_index::xtext_rules_artifact(&[
        ("kerml", KERML_XTEXT, kerml.as_str()),
        ("sysml", SYSML_XTEXT, sysml.as_str()),
        ("kerml_expressions", EXPR_XTEXT, expr.as_str()),
    ]);
    let out_path = out_dir.join("xtext-rules.toml");
    std::fs::write(&out_path, artifact)
        .unwrap_or_else(|e| panic!("cannot write {}: {e}", out_path.display()));
    println!("wrote {}", out_path.display());
}
