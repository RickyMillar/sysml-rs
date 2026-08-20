//! PUMP-DATA-01 (plan §10.1 layered gate; matrix RT-ODE-SF checksum clause):
//! the espresso-pump-hybrid characteristic generator is deterministic — a rerun
//! reproduces byte-identical CSVs — and every emitted file's SHA-256 matches the
//! value recorded in `fixture-provenance.toml`.
//!
//! (The complementary DATA-04 gate `fixture_provenance.rs` checks the manifest
//! SHA-256 against the committed bytes on disk; this test additionally proves
//! the *generator itself* reproduces those exact bytes.)

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn fixture_dir() -> PathBuf {
    repo_root().join("examples/espresso-pump-hybrid")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "data")]
    data: Vec<DataEntry>,
}
#[derive(Deserialize)]
struct DataEntry {
    path: String,
    sha256: String,
    row_count: usize,
}

fn load_manifest() -> Manifest {
    let text = std::fs::read_to_string(fixture_dir().join("fixture-provenance.toml"))
        .expect("read fixture-provenance.toml");
    toml::from_str(&text).expect("parse fixture-provenance.toml")
}

#[test]
fn committed_csv_checksums_match_manifest() {
    let manifest = load_manifest();
    assert_eq!(manifest.data.len(), 2, "opening + closing branches declared");
    for entry in &manifest.data {
        let bytes = std::fs::read(fixture_dir().join(&entry.path))
            .unwrap_or_else(|e| panic!("read {}: {e}", entry.path));
        assert_eq!(
            sha256_hex(&bytes),
            entry.sha256,
            "{}: committed SHA-256 matches the manifest",
            entry.path
        );
        // row_count = header + data rows.
        let rows = bytes.iter().filter(|b| **b == b'\n').count();
        assert_eq!(rows, entry.row_count + 1, "{}: row_count matches", entry.path);
    }
}

#[test]
fn generator_rerun_is_byte_identical() {
    // Regenerate into an isolated copy of the fixture layout so the committed
    // tree is never mutated, then diff the fresh CSVs against the committed ones.
    let scratch = std::env::temp_dir().join(format!("pump-regen-{}", std::process::id()));
    let scripts = scratch.join("scripts");
    std::fs::create_dir_all(&scripts).expect("mkdir scratch/scripts");
    std::fs::create_dir_all(scratch.join("data")).expect("mkdir scratch/data");
    std::fs::copy(
        fixture_dir().join("scripts/generate_characteristics.py"),
        scripts.join("generate_characteristics.py"),
    )
    .expect("copy generator script");

    let status = Command::new("python3")
        .arg(scripts.join("generate_characteristics.py"))
        .status()
        .expect("python3 must be available to run the generator");
    assert!(status.success(), "generator exited nonzero");

    for name in ["generated_pump_opening.csv", "generated_pump_closing.csv"] {
        let committed = std::fs::read(fixture_dir().join("data").join(name)).expect("read committed");
        let regenerated = std::fs::read(scratch.join("data").join(name)).expect("read regenerated");
        assert_eq!(
            committed, regenerated,
            "{name}: generator rerun must reproduce the committed bytes exactly"
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}
