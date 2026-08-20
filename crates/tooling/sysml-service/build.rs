// Build scripts use panic/expect as the standard way to report build failures.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
//! Build script for sysml-service.
//!
//! Generates `element_kind_classification.generated.rs`, which declares the
//! free functions `archetype_for(&ElementKind) -> Archetype` and
//! `is_user_facing_noise_for(&ElementKind) -> bool` used by the query module
//! to project model trees.
//!
//! The single source of truth for both functions (and the FE TypeScript
//! mirror at `editors/simulation-app/src/types/element-kind-classification.generated.ts`)
//! is `crates/lang/codegen/src/archetype_rules.toml`. Validation fails the
//! build if any `ElementKind` variant slips through uncategorized.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let rules_path = locate_rules_path(&manifest_dir);
    println!("cargo:rerun-if-changed={}", rules_path.display());
    println!("cargo:rerun-if-changed=build.rs");
    // The generator ALSO reads the spec vocab TTLs (via find_references_dir)
    // to build the kind universe + hierarchy — without these lines, editing
    // the vocab silently leaves the archetype classification stale.
    let refs = Path::new(&manifest_dir).join("../../../references/sysmlv2");
    for ttl in ["Kerml-Vocab.ttl", "SysML-vocab.ttl"] {
        println!("cargo:rerun-if-changed={}", refs.join(ttl).display());
    }

    let rust_code = sysml_codegen::archetype_generator::generate_classification_rust(&rules_path)
        .unwrap_or_else(|e| panic!("archetype rules validation failed: {e}"));

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_path = Path::new(&out_dir).join("element_kind_classification.generated.rs");
    fs::write(&out_path, rust_code)
        .unwrap_or_else(|e| panic!("Failed to write {:?}: {}", out_path, e));

    println!(
        "cargo:warning=Generated element-kind classification at {}",
        out_path.display()
    );
}

/// Resolve `archetype_rules.toml` relative to the sysml-service manifest dir.
/// `sysml-service` lives at `crates/tooling/sysml-service`, so the rules file
/// is three levels up + `lang/codegen/src/archetype_rules.toml`.
fn locate_rules_path(manifest_dir: &str) -> PathBuf {
    let candidate = Path::new(manifest_dir)
        .join("../../lang/codegen/src/archetype_rules.toml");
    if candidate.exists() {
        return candidate;
    }
    panic!(
        "could not locate archetype_rules.toml from sysml-service manifest dir: {}",
        candidate.display()
    );
}
