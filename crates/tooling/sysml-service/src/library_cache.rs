//! Library caching for fast LSP startup.
//!
//! This module provides functionality to cache the pre-parsed standard library
//! as a binary file, dramatically reducing LSP initialization time.
//!
//! ## Cache Location
//!
//! The cache is stored at `~/.cache/sysml-rs/library-v{VERSION}.bin` where
//! VERSION is the current crate version.
//!
//! ## Cache Invalidation
//!
//! The cache is invalidated when:
//! - The cache file doesn't exist
//! - The library source files have been modified (checked via hash)
//! - The sysml-lsp-server version changes

// LSP server: tower-lsp patterns use unwrap/expect for client sends,
// indexing is bounds-checked by protocol invariants, Arc cloning is intentional.
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::manual_let_else,
    clippy::arc_with_non_send_sync,
    clippy::clone_on_ref_ptr,
    clippy::map_err_ignore,
    clippy::needless_pass_by_value,
    clippy::panic
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sysml_core::{ModelGraph, Value};
use sysml_id::ElementId;
use sysml_ide_db::AnalysisHost;
use sysml_parser_trait::library::{LibraryConfig, LibraryLoadError};

/// Environment variable that disables disk caching. Useful for benchmarks or
/// when the on-disk cache is suspected of being stale. When set to any
/// non-empty value, [`LibraryCache::load_via_host`] skips the cache read /
/// write and always invokes the salsa-tracked stdlib loader.
const CACHE_DISABLE_ENV: &str = "SYSML_LIBRARY_CACHE_DISABLE";

/// Current cache format version. Bump this when the serialization format changes.
/// v1/v2: bincode (no longer read — such a cache is treated as a miss)
/// v3: serde_json for graph, JSON sidecar for metadata (requires ref rehydration on load)
// TS-3.6: bumped 3 → 4 to invalidate any pre-existing on-disk caches that
// were written by a PestParser-backed `LibraryCache::load(...)` (pre-TS-3.2
// builds). After TS-3.2 the canonical write path is TS; mixed Pest/TS
// caches in `~/.cache/sysml-rs/` would otherwise silently survive across
// the parser flip because `compute_source_hash` keys on (path, mtime) only
// and does not include parser identity. Bumping the format version is the
// cheapest fix (per TS-3.5 closeout's "consider bumping CACHE_FORMAT_VERSION
// or mixing parser.name() into the hash"); subsequent loads will treat the
// old bytes as a miss and rebuild via the canonical TS parser.
//
// diag-triage Arc 1 (2026-06-10): bumped 4 → 5 for the nested-package
// grammar fix — `package`/`namespace`/`library package` declarations are
// now legal package/namespace-body members and lower as real Package
// elements instead of ReferenceUsages. Stdlib graphs cached by the old
// grammar would silently survive (same (path, mtime) keying gap).
//
// rtbugs (2026-08-04): bumped 5 → 6 for the ADR-009 checkout-independent
// root-key change (commit 6f7e7f8e). Element identity no longer embeds the
// absolute checkout path: the stdlib bundle is now keyed off a
// library-relative `root_scope` instead of `CanonicalKey::root(file_path)`,
// so the same stdlib source produces DIFFERENT ElementId values before and
// after that commit. `compute_source_hash` keys on (path, mtime) only and
// does NOT cover the id schema, so a cache written by a pre-6f7e7f8e binary
// (path-coupled ids) would be accepted as a valid hit and return stale ids —
// while a fresh parse produces the new checkout-independent ids. That makes a
// cache HIT observably different from a cache MISS, violating the cache's one
// invariant ("must change speed, never results"). Bumping the format version
// is the durable fix: pre-ADR-009 caches now read as `FormatMismatch` misses
// and rebuild. INVARIANT for future id/resolution/serialization changes that
// move ElementIds or the digest: bump this constant in the same commit — the
// `library_cache_cold_warm_digest_equivalence` regression test guards the
// within-version half (cold parse == warm hit), this version key guards the
// cross-version half.
const CACHE_FORMAT_VERSION: u32 = 6;

/// The crate version used for cache path.
const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cache metadata stored alongside the serialized graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheMetadata {
    /// Cache format version.
    format_version: u32,
    /// Crate version that created this cache.
    crate_version: String,
    /// SHA256 hash of the library source files.
    source_hash: String,
    /// Timestamp when the cache was created.
    created_at: u64,
}

/// Result of a cache load attempt.
#[derive(Debug)]
pub enum CacheLoadResult {
    /// Cache hit - library loaded from cache.
    Hit(Box<ModelGraph>),
    /// Cache miss - need to parse library.
    Miss(CacheMissReason),
}

/// Reason why the cache was missed.
#[derive(Debug)]
pub enum CacheMissReason {
    /// Cache file doesn't exist.
    NotFound,
    /// Cache file is corrupted or unreadable.
    Corrupted(String),
    /// Cache version mismatch.
    VersionMismatch { cached: String, current: String },
    /// Source files have changed.
    SourceChanged,
    /// Cache format version mismatch.
    FormatMismatch { cached: u32, current: u32 },
}

impl std::fmt::Display for CacheMissReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheMissReason::NotFound => write!(f, "cache not found"),
            CacheMissReason::Corrupted(msg) => write!(f, "cache corrupted: {}", msg),
            CacheMissReason::VersionMismatch { cached, current } => {
                write!(
                    f,
                    "version mismatch: cached={}, current={}",
                    cached, current
                )
            }
            CacheMissReason::SourceChanged => write!(f, "source files changed"),
            CacheMissReason::FormatMismatch { cached, current } => {
                write!(f, "format mismatch: cached={}, current={}", cached, current)
            }
        }
    }
}

/// Library cache manager.
pub struct LibraryCache {
    /// Path to the cache directory.
    cache_dir: PathBuf,
    /// Library configuration.
    config: LibraryConfig,
}

impl LibraryCache {
    /// Create a new library cache manager.
    pub fn new(config: LibraryConfig) -> Self {
        let cache_dir = get_cache_dir();
        LibraryCache { cache_dir, config }
    }

    /// Get the library configuration.
    pub fn config(&self) -> &LibraryConfig {
        &self.config
    }

    /// Get the path to the graph cache file.
    fn cache_path(&self) -> PathBuf {
        self.cache_dir
            .join(format!("library-v{}.bin", CRATE_VERSION))
    }

    /// Get the path to the metadata sidecar file (small, ~200 bytes).
    fn meta_path(&self) -> PathBuf {
        self.cache_dir
            .join(format!("library-v{}.meta", CRATE_VERSION))
    }

    /// Try to load the library from cache.
    ///
    /// Uses a two-phase approach:
    /// 1. Read the tiny metadata sidecar to validate version/hash (fast)
    /// 2. Only then read+deserialize the large graph file (slow)
    pub fn try_load(&self) -> CacheLoadResult {
        let cache_path = self.cache_path();
        let meta_path = self.meta_path();

        // Phase 1: Quick metadata check (tiny file, ~200 bytes)
        //
        // A graph file with no metadata sidecar is a pre-v3 cache. It reads as
        // a miss and is rebuilt: the sidecar-split format arrived three format
        // versions ago, and a rebuild is what a miss already costs.
        if !meta_path.exists() || !cache_path.exists() {
            return CacheLoadResult::Miss(CacheMissReason::NotFound);
        }

        let metadata = match self.read_metadata(&meta_path) {
            Ok(m) => m,
            Err(e) => return CacheLoadResult::Miss(CacheMissReason::Corrupted(e)),
        };

        // Check format version
        if metadata.format_version != CACHE_FORMAT_VERSION {
            return CacheLoadResult::Miss(CacheMissReason::FormatMismatch {
                cached: metadata.format_version,
                current: CACHE_FORMAT_VERSION,
            });
        }

        // Check crate version
        if metadata.crate_version != CRATE_VERSION {
            return CacheLoadResult::Miss(CacheMissReason::VersionMismatch {
                cached: metadata.crate_version,
                current: CRATE_VERSION.to_owned(),
            });
        }

        // Quick freshness check: if cache file is newer than all library dirs,
        // skip the expensive per-file hash computation
        let needs_hash_check = !self.cache_is_newer_than_sources(&cache_path);

        if needs_hash_check {
            let current_hash = self.compute_source_hash();
            if metadata.source_hash != current_hash {
                return CacheLoadResult::Miss(CacheMissReason::SourceChanged);
            }
        }

        // Phase 2: Read the large graph file (only reached on valid cache)
        match self.read_graph(&cache_path) {
            Ok(graph) => CacheLoadResult::Hit(Box::new(graph)),
            Err(e) => CacheLoadResult::Miss(CacheMissReason::Corrupted(e)),
        }
    }

    /// Load the standard library via the unified salsa loader on the host.
    ///
    /// This is the canonical entry point: every transport (LSP, MCP, CLI,
    /// REST) should converge on this method so the parsed graph is identical
    /// across paths.
    ///
    /// Flow:
    /// 1. Cache enabled (default) → try the on-disk cache. On hit, install
    ///    the deserialized graph into the salsa db via
    ///    [`AnalysisHost::set_library`] and return.
    /// 2. Cache miss (or `SYSML_LIBRARY_CACHE_DISABLE` set) →
    ///    [`AnalysisHost::enable_stdlib_with_path`] parses the stdlib through
    ///    salsa (no `strict=true` gate), then we read the graph back via
    ///    [`AnalysisHost::library_graph`] and write it to disk for next time.
    ///
    /// The salsa loader is the same code path used by
    /// [`crate::SysmlService::workspace_refresh`], so MCP/REST/CLI behaviour
    /// stays in lock-step with the LSP.
    pub fn load_via_host(
        &self,
        host: &mut AnalysisHost,
    ) -> Result<ModelGraph, LibraryLoadError> {
        let cache_disabled = std::env::var_os(CACHE_DISABLE_ENV).is_some_and(|v| !v.is_empty());

        if !cache_disabled {
            match self.try_load() {
                CacheLoadResult::Hit(graph) => {
                    tracing::info!(
                        cache_hit = true,
                        cache_path = %self.cache_path().display(),
                        meta_path = %self.meta_path().display(),
                        library_path = %self.config.library_path.display(),
                        "loaded library from cache (load_via_host)"
                    );
                    let graph = *graph;
                    host.set_library(graph.clone());
                    return Ok(graph);
                }
                CacheLoadResult::Miss(reason) => {
                    tracing::info!(
                        cache_hit = false,
                        cache_path = %self.cache_path().display(),
                        meta_path = %self.meta_path().display(),
                        library_path = %self.config.library_path.display(),
                        miss_reason = %reason,
                        "library cache miss; deferring to host::enable_stdlib"
                    );
                }
            }
        } else {
            tracing::info!(
                env = CACHE_DISABLE_ENV,
                "library cache disabled via env var; deferring to host::enable_stdlib"
            );
        }

        // Cache miss (or disabled) — run the salsa loader through the host.
        let path = self.config.library_path.clone();
        host.enable_stdlib_with_path(Some(path.clone())).map_err(|e| {
            LibraryLoadError::ReadError {
                path: path.clone(),
                source: std::io::Error::other(format!("enable_stdlib failed: {e}")),
            }
        })?;

        let lib = host.library_graph().ok_or_else(|| LibraryLoadError::ReadError {
            path: path.clone(),
            source: std::io::Error::other(
                "enable_stdlib completed but no LibraryGraph was registered (path missing or empty?)",
            ),
        })?;
        let graph = lib.data(host.db()).graph().clone();

        if !cache_disabled {
            if let Err(e) = self.save_cache(&graph) {
                tracing::warn!(
                    error = %e,
                    cache_path = %self.cache_path().display(),
                    meta_path = %self.meta_path().display(),
                    library_path = %self.config.library_path.display(),
                    "failed to save library cache after enable_stdlib"
                );
            }
        }

        Ok(graph)
    }

    /// Save the library to cache (split format: .meta JSON + .bin JSON).
    ///
    /// Uses serde_json (not bincode) because the `Value` enum uses
    /// `#[serde(untagged)]` which is incompatible with bincode's positional format.
    fn save_cache(&self, graph: &ModelGraph) -> Result<(), String> {
        // Ensure cache directory exists
        fs::create_dir_all(&self.cache_dir)
            .map_err(|e| format!("failed to create cache dir: {}", e))?;

        let metadata = CacheMetadata {
            format_version: CACHE_FORMAT_VERSION,
            crate_version: CRATE_VERSION.to_owned(),
            source_hash: self.compute_source_hash(),
            created_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };

        // Write metadata sidecar (tiny, ~200 bytes)
        let meta_bytes = serde_json::to_vec(&metadata)
            .map_err(|e| format!("meta serialization failed: {}", e))?;
        let meta_path = self.meta_path();
        fs::write(&meta_path, &meta_bytes).map_err(|e| format!("failed to write meta: {}", e))?;

        // Write graph data using serde_json (handles untagged Value correctly)
        let cache_path = self.cache_path();
        let file =
            fs::File::create(&cache_path).map_err(|e| format!("failed to create cache: {}", e))?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer(writer, graph)
            .map_err(|e| format!("graph serialization failed: {}", e))?;

        let graph_size = fs::metadata(&cache_path).map(|m| m.len()).unwrap_or(0);

        let source_hash_prefix: String = metadata.source_hash.chars().take(8).collect();
        tracing::info!(
            cache_path = %cache_path.display(),
            meta_path = %meta_path.display(),
            graph_bytes = graph_size,
            meta_bytes = meta_bytes.len(),
            format_version = metadata.format_version,
            source_hash_prefix = %source_hash_prefix,
            "saved library cache"
        );

        Ok(())
    }

    /// Read metadata sidecar file.
    fn read_metadata(&self, path: &Path) -> Result<CacheMetadata, String> {
        let bytes = fs::read(path).map_err(|e| format!("failed to read meta: {}", e))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("failed to parse meta: {}", e))
    }

    /// Read and deserialize just the graph data (JSON format).
    fn read_graph(&self, path: &Path) -> Result<ModelGraph, String> {
        let file = fs::File::open(path).map_err(|e| format!("failed to open: {}", e))?;
        let reader = std::io::BufReader::new(file);
        let mut graph: ModelGraph = serde_json::from_reader(reader)
            .map_err(|e| format!("failed to deserialize graph: {}", e))?;

        // Cached Value uses untagged serde; reference-like strings need restoring.
        // Then rebuild indexes so library-name lookups (Real, Integer, etc.) work on cache hits.
        rehydrate_cached_ref_values(&mut graph);
        graph.rebuild_indexes();
        if !graph.library_packages().is_empty() {
            graph.build_library_index();
        }

        Ok(graph)
    }

    /// Quick check: is the cache file newer than all library source directories?
    ///
    /// This avoids the expensive per-file hash computation when nothing has changed.
    /// Falls back to full hash check if mtime comparison is inconclusive.
    fn cache_is_newer_than_sources(&self, cache_path: &Path) -> bool {
        let cache_mtime = match fs::metadata(cache_path)
            .ok()
            .and_then(|m| m.modified().ok())
        {
            Some(t) => t,
            None => return false,
        };

        // Check each library subdirectory and its files
        for subdir in &["library.kernel", "library.systems", "library.domain"] {
            let dir = self.config.library_path.join(subdir);
            if !dir.exists() {
                continue;
            }
            // Check directory mtime (catches file additions/removals)
            if let Ok(meta) = fs::metadata(&dir) {
                if let Ok(dir_mtime) = meta.modified() {
                    if dir_mtime > cache_mtime {
                        return false;
                    }
                }
            }
        }

        true
    }

    /// Compute a hash of the library source files.
    ///
    /// Uses a sorted list of (path, mtime) to detect changes efficiently.
    fn compute_source_hash(&self) -> String {
        let mut hasher = Sha256::new();

        // Collect file info sorted by path for deterministic hash
        let mut file_info: BTreeMap<String, u64> = BTreeMap::new();

        // Walk library directories
        for subdir in &["library.kernel", "library.systems", "library.domain"] {
            let dir = self.config.library_path.join(subdir);
            if dir.exists() {
                collect_file_mtimes(&dir, &mut file_info);
            }
        }

        // Hash the sorted file info
        for (path, mtime) in &file_info {
            hasher.update(path.as_bytes());
            hasher.update(b":");
            hasher.update(mtime.to_le_bytes());
            hasher.update(b"\n");
        }

        // Include the library path itself
        hasher.update(self.config.library_path.to_string_lossy().as_bytes());

        format!("{:x}", hasher.finalize())
    }

    /// Clear the cache.
    pub fn clear(&self) -> Result<(), std::io::Error> {
        let cache_path = self.cache_path();
        if cache_path.exists() {
            fs::remove_file(&cache_path)?;
        }
        let meta_path = self.meta_path();
        if meta_path.exists() {
            fs::remove_file(&meta_path)?;
        }
        Ok(())
    }

    /// Get cache statistics.
    pub fn stats(&self) -> Option<CacheStats> {
        let cache_path = self.cache_path();
        let meta_path = self.meta_path();
        if !cache_path.exists() {
            return None;
        }

        let file_metadata = fs::metadata(&cache_path).ok()?;
        let size = file_metadata.len();

        // Try to read metadata from sidecar first (fast path)
        let cache_meta = if meta_path.exists() {
            self.read_metadata(&meta_path).ok()
        } else {
            None
        };

        if let Some(meta) = cache_meta {
            return Some(CacheStats {
                size_bytes: size,
                element_count: 0, // Don't deserialize graph just for stats
                crate_version: meta.crate_version,
            });
        }

        // No metadata sidecar: a pre-v3 cache, which try_load treats as a miss.
        // Report nothing rather than deserializing a graph that will be
        // discarded on the next load anyway.
        None
    }
}

/// JSON shape describing the on-disk cache file: `{exists, size_bytes,
/// element_count, crate_version}`. Mirrors the LSP-side helper that this
/// module supersedes; consumers compose it into larger payloads
/// (`sysml.cache.status`, `sysml.cache.rebuild`).
pub fn library_cache_stats_json(cache: &LibraryCache) -> serde_json::Value {
    match cache.stats() {
        Some(stats) => serde_json::json!({
            "exists": true,
            "size_bytes": stats.size_bytes,
            "element_count": stats.element_count,
            "crate_version": stats.crate_version,
        }),
        None => serde_json::json!({
            "exists": false,
            "size_bytes": 0u64,
            "element_count": 0usize,
            "crate_version": serde_json::Value::Null,
        }),
    }
}

/// Find library configuration from environment or default paths.
pub fn find_library_config() -> Option<LibraryConfig> {
    find_library_config_with_override(None)
}

/// Find library configuration, checking an override path first.
///
/// If `override_path` is provided and points to an existing directory,
/// it takes priority over environment variables and default paths.
pub fn find_library_config_with_override(
    override_path: Option<&std::path::Path>,
) -> Option<LibraryConfig> {
    if let Some(path) = override_path {
        if path.exists() {
            return Some(LibraryConfig::new(path.to_path_buf()));
        }
    }

    if let Ok(path) = std::env::var("SYSML_LIBRARY_PATH") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(LibraryConfig::new(path));
        }
    }

    if let Some(path) = LibraryConfig::default_library_path() {
        return Some(LibraryConfig::new(path));
    }

    if let Ok(exe) = std::env::current_exe() {
        for depth in 0..4 {
            let mut candidate = exe.clone();
            for _ in 0..=depth {
                candidate = match candidate.parent() {
                    Some(p) => p.to_path_buf(),
                    None => break,
                };
            }
            let lib_path = candidate.join("libraries").join("standard");
            if lib_path.exists() {
                return Some(LibraryConfig::new(lib_path));
            }
        }
    }

    None
}

/// Statistics about the cache.
#[derive(Debug)]
pub struct CacheStats {
    /// Size of the cache file in bytes.
    pub size_bytes: u64,
    /// Number of elements in the cached graph.
    pub element_count: usize,
    /// Crate version that created this cache.
    pub crate_version: String,
}

/// Get the cache directory path.
fn get_cache_dir() -> PathBuf {
    // Try XDG cache dir first
    if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "sysml-rs") {
        return proj_dirs.cache_dir().to_path_buf();
    }

    // Fallback to ~/.cache/sysml-rs
    if let Some(home) = directories::BaseDirs::new() {
        return home.cache_dir().join("sysml-rs");
    }

    // Last resort
    PathBuf::from("/tmp/sysml-rs-cache")
}

/// Recursively collect file modification times.
fn collect_file_mtimes(dir: &Path, result: &mut BTreeMap<String, u64>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_file_mtimes(&path, result);
        } else if path.is_file() {
            // Only include .sysml and .kerml files
            if let Some(ext) = path.extension() {
                if ext == "sysml" || ext == "kerml" {
                    if let Ok(meta) = fs::metadata(&path) {
                        let mtime = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        result.insert(path.to_string_lossy().to_string(), mtime);
                    }
                }
            }
        }
    }
}

/// Rehydrate cached `Value::String` entries into `Value::Ref` when they point
/// to known element IDs in this graph.
///
/// Cache files serialize `Value` with untagged serde, so reference values are
/// deserialized as plain strings. This pass restores them so index rebuild and
/// resolution can operate correctly.
///
/// NOTE (rtbugs 2026-08-04): untagged serialization also erases the
/// `Value::Enum` vs `Value::String` distinction, so enum-valued props (today
/// only `visibility`) deserialize as `String`. This is NOT restored here on
/// purpose: the fresh parse itself is non-uniform — `membership.rs` writes
/// `visibility` as `Value::Enum` while `ast_builder/imports.rs` writes it as
/// `Value::String` — so there is no single variant a key-based pass could
/// restore to that would match the cold graph element-for-element. The
/// degradation is invisible to `ModelGraph::content_digest` (which serializes
/// props through the same untagged `Value`), so content-addressed identity
/// (CommitIds) is unaffected. Making `visibility` a uniform `VisibilityKind`
/// enum at the parser is a separate, steward-gated model-hygiene change.
fn rehydrate_cached_ref_values(graph: &mut ModelGraph) {
    let known_ids: std::collections::HashSet<ElementId> = graph.elements.keys().cloned().collect();
    for element in graph.elements.values_mut() {
        for value in element.props.values_mut() {
            rehydrate_value(value, &known_ids);
        }
    }
}

fn rehydrate_value(value: &mut Value, known_ids: &std::collections::HashSet<ElementId>) {
    let parsed_ref = match value {
        Value::String(text) => text
            .parse::<ElementId>()
            .ok()
            .filter(|id| known_ids.contains(id)),
        _ => None,
    };

    if let Some(id) = parsed_ref {
        *value = Value::Ref(id);
        return;
    }

    match value {
        Value::List(items) => {
            for item in items {
                rehydrate_value(item, known_ids);
            }
        }
        Value::Map(map) => {
            for nested in map.values_mut() {
                rehydrate_value(nested, known_ids);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn test_cache_dir_unique(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "sysml-rs-cache-test-{}-{}-{}",
            label,
            std::process::id(),
            nanos
        ))
    }

    /// Guards the `float_roundtrip` serde_json feature (declared on the
    /// workspace `serde_json` dependency). Without it serde_json's default
    /// float parser is not correctly-rounded: `1e-24` serializes to "1e-24" but
    /// parses back one ULP higher, so an f64 does NOT survive a JSON write→read.
    /// That silently corrupted the library cache (warm hit ≠ cold parse on
    /// float props, moving `content_digest`). If this test regresses, the
    /// feature was dropped — restore it, don't relax the assertion.
    #[test]
    fn value_float_survives_json_round_trip() {
        for s in ["1e-24", "1e-28", "3.14159265358979", "6.022e23"] {
            let x: f64 = s.parse().unwrap();
            let v = Value::Float(x);
            let json = serde_json::to_string(&v).unwrap();
            let back: Value = serde_json::from_str(&json).unwrap();
            assert_eq!(
                v, back,
                "Value::Float({s}) did not round-trip through JSON (json={json}); \
                 the serde_json `float_roundtrip` feature is likely missing"
            );
        }
    }

    #[test]
    fn test_cache_dir() {
        let dir = get_cache_dir();
        // Should be a reasonable path
        assert!(dir.to_string_lossy().contains("sysml"));
    }

    #[test]
    fn test_cache_miss_display() {
        assert_eq!(format!("{}", CacheMissReason::NotFound), "cache not found");
        assert_eq!(
            format!("{}", CacheMissReason::SourceChanged),
            "source files changed"
        );
    }

    #[test]
    fn test_cache_miss_display_all_variants() {
        // NotFound
        let msg = format!("{}", CacheMissReason::NotFound);
        assert_eq!(msg, "cache not found");

        // Corrupted
        let msg = format!("{}", CacheMissReason::Corrupted("bad data".to_string()));
        assert_eq!(msg, "cache corrupted: bad data");

        // VersionMismatch
        let msg = format!(
            "{}",
            CacheMissReason::VersionMismatch {
                cached: "0.1.0".to_string(),
                current: "0.2.0".to_string(),
            }
        );
        assert_eq!(msg, "version mismatch: cached=0.1.0, current=0.2.0");

        // SourceChanged
        let msg = format!("{}", CacheMissReason::SourceChanged);
        assert_eq!(msg, "source files changed");

        // FormatMismatch
        let msg = format!(
            "{}",
            CacheMissReason::FormatMismatch {
                cached: 1,
                current: 2,
            }
        );
        assert_eq!(msg, "format mismatch: cached=1, current=2");
    }

    #[test]
    fn test_try_load_not_found() {
        // Use an isolated temp cache dir so tests never touch the user's real cache.
        let config = LibraryConfig::new("/tmp/sysml-rs-test-nonexistent-library-path");
        let cache_dir = test_cache_dir_unique("not-found");
        let cache = LibraryCache {
            cache_dir: cache_dir.clone(),
            config,
        };

        match cache.try_load() {
            CacheLoadResult::Miss(CacheMissReason::NotFound) => { /* expected */ }
            other => panic!("Expected Miss(NotFound), got: {:?}", other),
        }

        let _ = fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn test_cache_format_version_constant() {
        // Verify the constant is set to a reasonable value
        assert!(CACHE_FORMAT_VERSION >= 1);
    }

    #[test]
    fn test_cache_hit_rebuilds_indexes_for_library_lookup() {
        use sysml_core::{Element, ElementKind};

        let cache_dir = test_cache_dir_unique("library-index");
        let config = LibraryConfig::new("/tmp/sysml-rs-test-library-index");
        let cache = LibraryCache {
            cache_dir: cache_dir.clone(),
            config,
        };

        let mut graph = ModelGraph::new();
        let scalar_values =
            Element::new_with_kind(ElementKind::LibraryPackage).with_name("ScalarValues");
        let scalar_values_id = graph.add_element(scalar_values);
        assert!(graph.register_library_package(scalar_values_id.clone()));

        let integer = Element::new_with_kind(ElementKind::DataType)
            .with_name("Integer")
            .with_owner(scalar_values_id.clone());
        let integer_id = graph.add_element(integer);

        // Minimal explicit membership so rebuild_indexes()/build_library_index can discover
        // Integer from ScalarValues after deserialization.
        let membership = Element::new_with_kind(ElementKind::Membership)
            .with_prop("membershipOwningNamespace", scalar_values_id.clone())
            .with_prop("memberElement", integer_id.clone())
            .with_prop("memberName", "Integer")
            .with_prop("visibility", "public");
        graph.add_element(membership);

        let user_pkg = Element::new_with_kind(ElementKind::Package).with_name("UserPkg");
        graph.add_element(user_pkg);

        cache
            .save_cache(&graph)
            .expect("failed to save test cache graph");

        let loaded = match cache.try_load() {
            CacheLoadResult::Hit(graph) => *graph,
            miss => panic!("expected cache hit, got {:?}", miss),
        };

        // Indexes should have been rebuilt on cache load.
        assert!(!loaded.root_ids().is_empty(), "root_ids should be rebuilt");
        assert!(
            !loaded.lookup_by_name("Integer").is_empty(),
            "name_index should be rebuilt"
        );
        assert!(
            loaded.resolve_in_library("ScalarValues").is_some(),
            "library index should be built for package lookup"
        );
        assert!(
            loaded.resolve_in_library("Integer").is_some(),
            "library index should resolve stdlib member names on cache hits"
        );
        assert!(
            !loaded.library_index_needs_rebuild(),
            "library index should be up-to-date after cache hit load"
        );
        assert!(
            loaded.elements.get(&integer_id).is_some(),
            "sanity check: Integer element should still exist"
        );

        let _ = fs::remove_dir_all(cache_dir);
    }

    /// Profile each phase of library loading to find bottlenecks.
    ///
    /// Run with: cargo test -p sysml-lsp-server profile_library_loading -- --ignored --nocapture
    #[test]
    #[ignore]
    fn profile_library_loading() {
        use std::time::Instant;

        // Phase 0: find_library_config
        let t0 = Instant::now();
        let config = find_library_config();
        let find_config_elapsed = t0.elapsed();
        eprintln!("[0] find_library_config: {:?}", find_config_elapsed);

        let config = match config {
            Some(c) => c,
            None => {
                eprintln!("No library config found - skipping");
                return;
            }
        };
        eprintln!("    library_path: {}", config.library_path.display());

        // Phase 1: LibraryCache::new
        let t1 = Instant::now();
        let cache = LibraryCache::new(config.clone());
        let new_elapsed = t1.elapsed();
        eprintln!("[1] LibraryCache::new: {:?}", new_elapsed);

        let cache_path = cache.cache_path();
        let meta_path = cache.meta_path();
        eprintln!(
            "    cache_path: {} (exists={})",
            cache_path.display(),
            cache_path.exists()
        );
        eprintln!(
            "    meta_path: {} (exists={})",
            meta_path.display(),
            meta_path.exists()
        );
        if cache_path.exists() {
            if let Ok(m) = fs::metadata(&cache_path) {
                eprintln!(
                    "    cache size: {} bytes ({:.1} MB)",
                    m.len(),
                    m.len() as f64 / 1_048_576.0
                );
            }
        }

        // Phase 2a: read metadata sidecar
        if meta_path.exists() {
            let t2a = Instant::now();
            let meta = cache.read_metadata(&meta_path);
            let meta_elapsed = t2a.elapsed();
            eprintln!(
                "[2a] read_metadata: {:?} (ok={})",
                meta_elapsed,
                meta.is_ok()
            );
            if let Ok(m) = &meta {
                eprintln!(
                    "     format_version={}, crate_version={}",
                    m.format_version, m.crate_version
                );
                eprintln!("     source_hash={}", &m.source_hash[..16]);
            }
        } else {
            eprintln!("[2a] read_metadata: SKIPPED (no .meta file)");
        }

        // Phase 2b: cache_is_newer_than_sources (quick mtime check)
        let t2b = Instant::now();
        let is_newer = cache.cache_is_newer_than_sources(&cache_path);
        let mtime_elapsed = t2b.elapsed();
        eprintln!(
            "[2b] cache_is_newer_than_sources: {:?} (result={})",
            mtime_elapsed, is_newer
        );

        // Phase 2c: compute_source_hash (walk 94 files)
        let t2c = Instant::now();
        let hash = cache.compute_source_hash();
        let hash_elapsed = t2c.elapsed();
        eprintln!(
            "[2c] compute_source_hash: {:?} (hash={})",
            hash_elapsed,
            &hash[..16]
        );

        // Phase 3: read + deserialize graph (the big one)
        if cache_path.exists() {
            let t3 = Instant::now();
            let graph = cache.read_graph(&cache_path);
            let graph_elapsed = t3.elapsed();
            match &graph {
                Ok(g) => eprintln!(
                    "[3]  read_graph: {:?} ({} elements)",
                    graph_elapsed,
                    g.element_count()
                ),
                Err(e) => eprintln!("[3]  read_graph: {:?} (FAILED: {})", graph_elapsed, e),
            }
        } else {
            eprintln!("[3]  read_graph: SKIPPED (no .bin file)");
        }

        // Phase 4: full try_load (combines all phases)
        let t4 = Instant::now();
        let result = cache.try_load();
        let total_elapsed = t4.elapsed();
        match &result {
            CacheLoadResult::Hit(g) => eprintln!(
                "[4]  try_load (total): {:?} (HIT, {} elements)",
                total_elapsed,
                g.element_count()
            ),
            CacheLoadResult::Miss(reason) => eprintln!(
                "[4]  try_load (total): {:?} (MISS: {})",
                total_elapsed, reason
            ),
        }

        // Phase 5: full load via the unified salsa path
        let mut host = sysml_ide_db::AnalysisHost::new();
        let t5 = Instant::now();
        let load_result = cache.load_via_host(&mut host);
        let load_elapsed = t5.elapsed();
        match &load_result {
            Ok(g) => eprintln!(
                "[5]  load_via_host (total): {:?} ({} elements)",
                load_elapsed,
                g.element_count()
            ),
            Err(e) => eprintln!("[5]  load_via_host (total): {:?} (FAILED: {})", load_elapsed, e),
        }

        eprintln!("\n=== SUMMARY ===");
        eprintln!("find_config:  {:>10?}", find_config_elapsed);
        eprintln!("mtime_check:  {:>10?}", mtime_elapsed);
        eprintln!("source_hash:  {:>10?}", hash_elapsed);
        eprintln!("total_load:   {:>10?}", load_elapsed);
    }

    /// Verifies `load_via_host` produces a usable stdlib graph in both
    /// cache-miss and cache-hit modes, with the same element count and the
    /// same `ScalarValues::Real` reachable through `resolve_in_library`.
    ///
    /// Each phase uses an isolated cache dir and a fresh `AnalysisHost`, so
    /// the cache file written by phase 1 is the only state that crosses the
    /// boundary. Both paths must converge on the same observable graph; the
    /// LSP and MCP rely on that invariant for stdlib navigation parity.
    #[test]
    fn load_via_host_round_trip() {
        let Some(mut config) = find_library_config() else {
            eprintln!("skipping: no standard library installed");
            return;
        };
        // Force strict=false defensively so the legacy code path can't
        // resurrect the 58-errors bug if the helper ever falls back to it.
        config.strict = false;
        let cache_dir = test_cache_dir_unique("load-via-host");

        // Phase 1: cache miss — host runs enable_stdlib_with_path via salsa.
        let (miss_elems, miss_has_real) = {
            let mut host = sysml_ide_db::AnalysisHost::new();
            let cache = LibraryCache {
                cache_dir: cache_dir.clone(),
                config: config.clone(),
            };
            let graph = cache
                .load_via_host(&mut host)
                .expect("cache miss load_via_host failed");
            assert!(host.library_graph().is_some(), "host should have library");
            let has_real = graph.resolve_in_library("Real").is_some();
            (graph.element_count(), has_real)
        };
        assert!(miss_elems > 0, "miss path returned empty graph");
        assert!(miss_has_real, "miss path: ScalarValues::Real unreachable");

        // Phase 2: cache hit — same cache dir, fresh host. Must not re-parse.
        let (hit_elems, hit_has_real) = {
            let mut host = sysml_ide_db::AnalysisHost::new();
            let cache = LibraryCache {
                cache_dir: cache_dir.clone(),
                config: config.clone(),
            };
            let graph = cache
                .load_via_host(&mut host)
                .expect("cache hit load_via_host failed");
            assert!(host.library_graph().is_some(), "host should have library");
            let has_real = graph.resolve_in_library("Real").is_some();
            (graph.element_count(), has_real)
        };

        assert_eq!(
            miss_elems, hit_elems,
            "cache hit and miss should produce graphs with the same element count"
        );
        assert!(hit_has_real, "hit path: ScalarValues::Real unreachable");

        let _ = fs::remove_dir_all(cache_dir);
    }

    /// A cache HIT must be observably identical to a cache MISS: the cache may
    /// only change *speed*, never *results*. This pins the full
    /// [`ModelGraph::content_digest`] (which folds every ElementId, kind, name,
    /// owner, props, and relationship triple) across the cold-parse write and
    /// the warm-hit read, so any future change that lets the serialize →
    /// deserialize → rehydrate → rebuild-indexes round-trip alter the graph
    /// (ids, ordering, ref rehydration) fails here instead of silently drifting
    /// resolution results between a warm and a cold tree.
    ///
    /// The cross-version half of the same invariant (a cache written by an
    /// older binary with a different id schema) is guarded structurally by
    /// [`CACHE_FORMAT_VERSION`], not by this test — this binary always writes
    /// the current schema, so cold and warm here are same-version by
    /// construction.
    #[test]
    fn library_cache_cold_warm_digest_equivalence() {
        let Some(mut config) = find_library_config() else {
            eprintln!("skipping: no standard library installed");
            return;
        };
        config.strict = false;
        let cache_dir = test_cache_dir_unique("cold-warm-digest");

        // Phase 1: cold — cache miss parses the stdlib and writes the cache.
        let cold_graph = {
            let mut host = sysml_ide_db::AnalysisHost::new();
            let cache = LibraryCache {
                cache_dir: cache_dir.clone(),
                config: config.clone(),
            };
            cache
                .load_via_host(&mut host)
                .expect("cold load_via_host failed")
        };
        let cold_digest = cold_graph.content_digest();

        // Phase 2: warm — same cache dir, fresh host. Must hit the cache and
        // deserialize to a byte-for-byte equivalent graph.
        let warm_graph = {
            let mut host = sysml_ide_db::AnalysisHost::new();
            let cache = LibraryCache {
                cache_dir: cache_dir.clone(),
                config: config.clone(),
            };
            // Assert we actually took the hit path, otherwise this proves nothing.
            match cache.try_load() {
                CacheLoadResult::Hit(_) => {}
                CacheLoadResult::Miss(reason) => {
                    let _ = fs::remove_dir_all(&cache_dir);
                    panic!("phase 2 expected a cache hit, got miss: {reason}");
                }
            }
            let graph = cache
                .load_via_host(&mut host)
                .expect("warm load_via_host failed");
            graph
        };
        let warm_digest = warm_graph.content_digest();

        // The cache's coherence contract is `content_digest`: it is what
        // content-addressed CommitIds key on, and the diff invariant
        // (`content_digest(a) == content_digest(b)`) is the meaning of "the
        // cache changed speed, not results". Before the `float_roundtrip` fix
        // this failed — f64 props drifted one ULP through the JSON round-trip,
        // so a warm hit minted a different digest than the cold parse.
        //
        // This is deliberately NOT `diff_graphs(cold, warm).is_empty()`: the
        // untagged `Value` serialization erases `Enum` vs `String`, and the
        // fresh parse stores `visibility` non-uniformly (see
        // `rehydrate_cached_ref_values`), so a variant-sensitive diff reports
        // differences that `content_digest` (and therefore identity) does not
        // see. Tightening this to a full structural diff requires first making
        // `visibility` a uniform enum at the parser.
        assert_eq!(
            cold_digest, warm_digest,
            "cache hit produced a graph with a different content_digest than the \
             cold parse — the cache is changing content-addressed identity, not just speed"
        );

        let _ = fs::remove_dir_all(cache_dir);
    }
}
