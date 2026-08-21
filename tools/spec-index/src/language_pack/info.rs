//! Stable pack discovery/status locator.
//!
//! `spec-index language-pack info` prints a single JSON object describing the
//! committed pack without any developer-absolute path: repo-relative pack
//! location, schema version, spec drop, generator identity, the pinned source
//! hashes, artifact counts, and freshness (whether the committed tree matches a
//! fresh regeneration). Agent instructions and the freshness gate
//! call this instead of hard-coding paths.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use super::{manifest, LpError};

/// One consumed source, path + hash.
#[derive(Debug, Clone, Serialize)]
pub struct SourceHash {
    pub path: String,
    pub sha256: String,
}

/// The discovery/status payload.
#[derive(Debug, Clone, Serialize)]
pub struct PackInfo {
    /// Repo-relative pack directory (never developer-absolute).
    pub pack_path: String,
    pub present: bool,
    pub schema_version: String,
    pub spec_drop: String,
    pub metamodel_drop: String,
    pub generator: String,
    pub generator_version: String,
    pub licensing_mode: String,
    pub card_count: usize,
    pub example_count: usize,
    pub chunk_count: usize,
    pub eval_datasets: Vec<String>,
    pub source_count: usize,
    pub source_hashes: Vec<SourceHash>,
    /// Tree hash recorded in the committed `report.json`.
    pub committed_tree_hash: String,
    /// Tree hash of a fresh in-process regeneration.
    pub regenerated_tree_hash: String,
    /// `clean` (committed == regenerated), `stale` (differs), or `absent`.
    pub freshness: String,
}

fn count_json_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0)
}

fn count_lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// Compute the pack info for the committed pack at `pack_dir`.
pub fn pack_info(repo_root: &Path, pack_dir: &Path) -> Result<PackInfo, LpError> {
    let rel_pack = pack_dir
        .strip_prefix(repo_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "references/sysmlv2/derived/language-pack".to_owned());

    let manifest = manifest::resolve_manifest(repo_root)?;
    let source_hashes: Vec<SourceHash> = manifest
        .sources
        .iter()
        .map(|s| SourceHash {
            path: s.path.clone(),
            sha256: s.sha256.clone(),
        })
        .collect();

    let present = pack_dir.join("report.json").exists();

    // Schema version as actually shipped (first card), else the schema default.
    let schema_version = std::fs::read_dir(pack_dir.join("cards"))
        .ok()
        .and_then(|mut rd| rd.find_map(|e| e.ok().map(|e| e.path())))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("schema_version").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| "1".to_owned());

    let card_count = count_json_files(&pack_dir.join("cards"));
    let example_count = count_json_files(&pack_dir.join("examples"));
    let chunk_count = count_lines(&pack_dir.join("retrieval/chunks.jsonl"));

    let mut eval_datasets: Vec<String> = std::fs::read_dir(pack_dir.join("evals"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    eval_datasets.sort();

    let committed_tree_hash = std::fs::read_to_string(pack_dir.join("report.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("tree_hash").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default();

    // Freshness: regenerate into a temp dir and compare the tree hash.
    let (regenerated_tree_hash, freshness) = if !present {
        (String::new(), "absent".to_owned())
    } else {
        let tmp = std::env::temp_dir().join(format!("lp-info-{}/language-pack", std::process::id()));
        if let Some(parent) = tmp.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let hash = super::run(repo_root, &tmp)?;
        let _ = std::fs::remove_dir_all(tmp.parent().unwrap_or(&tmp));
        let state = if hash == committed_tree_hash { "clean" } else { "stale" };
        (hash, state.to_owned())
    };

    Ok(PackInfo {
        pack_path: rel_pack,
        present,
        schema_version,
        spec_drop: manifest.spec_drop.clone(),
        metamodel_drop: manifest.metamodel_drop.clone(),
        generator: manifest::GENERATED_BY.to_owned(),
        generator_version: manifest.generator_version.clone(),
        licensing_mode: manifest.licensing_mode,
        card_count,
        example_count,
        chunk_count,
        eval_datasets,
        source_count: source_hashes.len(),
        source_hashes,
        committed_tree_hash,
        regenerated_tree_hash,
        freshness,
    })
}

/// Render [`pack_info`] as pretty JSON with a trailing newline.
pub fn info_json(repo_root: &Path, pack_dir: &Path) -> Result<String, LpError> {
    let info = pack_info(repo_root, pack_dir)?;
    let mut s = serde_json::to_string_pretty(&info)
        .map_err(|e| LpError::Other(format!("serialize info: {e}")))?;
    s.push('\n');
    Ok(s)
}
