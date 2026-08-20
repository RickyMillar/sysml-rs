//! Emits `editors/simulation-app/src/types/element-kind-classification.generated.ts`.
//!
//! Run explicitly when `archetype_rules.toml` changes:
//!
//! ```sh
//! cargo run -p sysml-codegen --bin emit-ts-classification
//! ```
//!
//! The generated TS file is checked into git so the FE doesn't need a Rust
//! toolchain at install time. The Rust side regenerates on every build via
//! `sysml-service/build.rs`, so the two outputs stay tied to the same rules.

use std::path::{Path, PathBuf};

#[allow(clippy::print_stdout, clippy::print_stderr)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = find_workspace_root()?;
    let rules_path = workspace_root
        .join("crates")
        .join("lang")
        .join("codegen")
        .join("src")
        .join("archetype_rules.toml");
    let out_path = workspace_root
        .join("editors")
        .join("simulation-app")
        .join("src")
        .join("types")
        .join("element-kind-classification.generated.ts");

    if !rules_path.exists() {
        return Err(format!(
            "archetype_rules.toml not found at {} — is the workspace layout intact?",
            rules_path.display()
        )
        .into());
    }
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let ts_code = sysml_codegen::archetype_generator::generate_classification_ts(&rules_path)?;
    std::fs::write(&out_path, ts_code)?;
    println!("wrote {}", out_path.display());
    Ok(())
}

/// Walk upward looking for a directory that contains both
/// `crates/lang/codegen/src/archetype_rules.toml` and
/// `editors/simulation-app/`. This is the workspace layout — works whether
/// the binary is invoked from the repo root or from any nested crate dir.
fn find_workspace_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cwd = std::env::current_dir()?;
    let mut cur: Option<&Path> = Some(cwd.as_path());
    for _ in 0..10 {
        let Some(c) = cur else { break };
        let rules = c
            .join("crates")
            .join("lang")
            .join("codegen")
            .join("src")
            .join("archetype_rules.toml");
        let editors = c.join("editors").join("simulation-app");
        if rules.exists() && editors.exists() {
            return Ok(c.to_path_buf());
        }
        cur = c.parent();
    }
    Err(format!(
        "could not locate workspace root from cwd {}; \
         expected crates/lang/codegen/src/archetype_rules.toml + editors/simulation-app/",
        cwd.display()
    )
    .into())
}
