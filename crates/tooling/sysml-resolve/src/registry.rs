//! Registry backend abstraction and Sysand backend implementation.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use directories::{BaseDirs, ProjectDirs};
use semver::{Version, VersionReq};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::ResolveError;

pub(crate) const DEFAULT_REGISTRY_BACKEND: &str = "sysand";
const SYSAND_INDEX_ENV: &str = "SYSML_REGISTRY_SYSAND_INDEX";
const SYSAND_LOCAL_INDEX_RELATIVE: &str = ".sysml/registries/sysand/index.json";
const SYSAND_REMOTE_INDEX_CACHE_RELATIVE: &str = "dependencies/registry/sysand/index-cache";
#[cfg(test)]
const SYSAND_REMOTE_INDEX_CACHE_TTL_SECS: u64 = 1;
#[cfg(not(test))]
const SYSAND_REMOTE_INDEX_CACHE_TTL_SECS: u64 = 300;

/// Immutable resolved registry release.
#[derive(Debug, Clone)]
pub(crate) struct RegistryRelease {
    pub backend: String,
    pub package: String,
    pub requested: String,
    pub version: String,
    pub artifact_url: String,
    pub checksum: String,
}

/// Registry release metadata for tooling use without artifact fetch/extract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryReleaseMetadata {
    pub backend: String,
    pub package: String,
    pub requested_requirement: String,
    pub resolved_version: String,
    pub artifact_url: String,
    pub checksum: String,
}

/// Materialized artifact bytes fetched from a registry backend.
#[derive(Debug, Clone)]
pub(crate) struct ArtifactFetch {
    pub path: PathBuf,
    pub checksum: String,
}

/// Registry backend contract used by `RegistryProvider`.
pub(crate) trait RegistryBackend {
    fn backend_id(&self) -> &'static str;

    fn resolve_release(
        &self,
        dep_name: &str,
        requirement: &str,
        parent_dir: &Path,
    ) -> Result<RegistryRelease, ResolveError>;

    fn fetch_artifact(
        &self,
        release: &RegistryRelease,
        dest_path: &Path,
    ) -> Result<ArtifactFetch, ResolveError>;
}

/// In-process backend registry used by `RegistryProvider`.
pub(crate) struct RegistryBackendRegistry {
    backends: Vec<Box<dyn RegistryBackend + Send + Sync>>,
}

impl std::fmt::Debug for RegistryBackendRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&'static str> = self.backends.iter().map(|b| b.backend_id()).collect();
        f.debug_struct("RegistryBackendRegistry")
            .field("backends", &ids)
            .finish()
    }
}

impl RegistryBackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: vec![Box::new(SysandRegistryBackend)],
        }
    }

    pub fn resolve_release(
        &self,
        backend_id: &str,
        dep_name: &str,
        requirement: &str,
        parent_dir: &Path,
    ) -> Result<RegistryRelease, ResolveError> {
        let backend =
            self.backend_for(backend_id)
                .ok_or_else(|| ResolveError::UnsupportedSource {
                    name: dep_name.to_owned(),
                    dep_type: format!("registry-backend-{backend_id}"),
                })?;

        backend.resolve_release(dep_name, requirement, parent_dir)
    }

    pub fn fetch_artifact(
        &self,
        backend_id: &str,
        release: &RegistryRelease,
        dest_path: &Path,
    ) -> Result<ArtifactFetch, ResolveError> {
        let backend =
            self.backend_for(backend_id)
                .ok_or_else(|| ResolveError::UnsupportedSource {
                    name: release.package.clone(),
                    dep_type: format!("registry-backend-{backend_id}"),
                })?;

        backend.fetch_artifact(release, dest_path)
    }

    fn backend_for(&self, backend_id: &str) -> Option<&(dyn RegistryBackend + Send + Sync)> {
        self.backends
            .iter()
            .find(|backend| backend.backend_id() == backend_id)
            .map(Box::as_ref)
    }
}

/// Resolve registry metadata (requested requirement -> resolved release) without
/// downloading or extracting artifacts.
pub fn resolve_registry_release_metadata(
    backend_id: &str,
    package: &str,
    requirement: &str,
    parent_dir: &Path,
) -> Result<RegistryReleaseMetadata, ResolveError> {
    let backend = if backend_id.trim().is_empty() {
        DEFAULT_REGISTRY_BACKEND
    } else {
        backend_id.trim()
    };

    let registry = RegistryBackendRegistry::default();
    let release = registry.resolve_release(backend, package, requirement, parent_dir)?;
    Ok(RegistryReleaseMetadata {
        backend: release.backend,
        package: release.package,
        requested_requirement: release.requested,
        resolved_version: release.version,
        artifact_url: release.artifact_url,
        checksum: release.checksum,
    })
}

/// Resolve the latest available release metadata for a registry package.
pub fn resolve_latest_registry_release_metadata(
    backend_id: &str,
    package: &str,
    parent_dir: &Path,
) -> Result<RegistryReleaseMetadata, ResolveError> {
    resolve_registry_release_metadata(backend_id, package, "*", parent_dir)
}

impl Default for RegistryBackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Sysand index backend.
#[derive(Debug, Default)]
pub(crate) struct SysandRegistryBackend;

#[derive(Debug)]
enum IndexSource {
    Local(PathBuf),
    RemoteUrl(String),
}

#[derive(Debug, Deserialize)]
struct SysandIndex {
    #[serde(default)]
    packages: BTreeMap<String, BTreeMap<String, SysandIndexRelease>>,
}

#[derive(Debug, Deserialize)]
struct SysandIndexRelease {
    artifact: String,
    checksum: String,
}

impl RegistryBackend for SysandRegistryBackend {
    fn backend_id(&self) -> &'static str {
        DEFAULT_REGISTRY_BACKEND
    }

    fn resolve_release(
        &self,
        dep_name: &str,
        requirement: &str,
        parent_dir: &Path,
    ) -> Result<RegistryRelease, ResolveError> {
        let parsed_requirement = parse_registry_requirement(dep_name, requirement)?;
        let requested_requirement = requirement.trim().to_owned();

        let index_source = resolve_sysand_index_source(parent_dir)?;
        let index = load_sysand_index(&index_source)?;

        let package_versions = index.packages.get(dep_name).ok_or_else(|| {
            ResolveError::io(
                index_source_context(&index_source),
                io::Error::other(format!(
                    "registry package '{dep_name}' not found in Sysand index"
                )),
            )
        })?;

        let (version_text, release) = select_sysand_release(
            dep_name,
            &parsed_requirement,
            package_versions,
            &index_source,
        )?;

        let artifact_url = canonicalize_artifact_url(&release.artifact, &index_source)?;
        ensure_sha256_checksum_format(dep_name, &release.checksum)?;

        Ok(RegistryRelease {
            backend: self.backend_id().to_owned(),
            package: dep_name.to_owned(),
            requested: requested_requirement,
            version: version_text,
            artifact_url,
            checksum: release.checksum.clone(),
        })
    }

    fn fetch_artifact(
        &self,
        release: &RegistryRelease,
        dest_path: &Path,
    ) -> Result<ArtifactFetch, ResolveError> {
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent).map_err(|e| ResolveError::io(parent, e))?;
        }

        fetch_url_to_path(&release.package, &release.artifact_url, dest_path)?;

        let actual_hex = sha256_hex_file(dest_path)?;
        let actual = format!("sha256:{actual_hex}");
        if actual != release.checksum {
            return Err(ResolveError::checksum_mismatch(
                dest_path,
                release.checksum.clone(),
                actual,
                "Registry artifact bytes did not match checksum from index; retry or clear cache",
            ));
        }

        Ok(ArtifactFetch {
            path: dest_path.to_path_buf(),
            checksum: release.checksum.clone(),
        })
    }
}

#[derive(Debug, Clone)]
enum RegistryRequirement {
    Exact(Version),
    Range(VersionReq),
}

fn parse_registry_requirement(
    dep_name: &str,
    requirement: &str,
) -> Result<RegistryRequirement, ResolveError> {
    let trimmed = requirement.trim();
    if let Some(raw) = trimmed.strip_prefix('=') {
        return Version::parse(raw.trim())
            .map(RegistryRequirement::Exact)
            .map_err(|e| {
                ResolveError::io(
                    dep_name,
                    io::Error::other(format!(
                        "invalid exact registry version '{requirement}': {e}"
                    )),
                )
            });
    }

    if let Ok(version) = Version::parse(trimmed) {
        return Ok(RegistryRequirement::Exact(version));
    }

    if let Ok(range) = VersionReq::parse(trimmed) {
        return Ok(RegistryRequirement::Range(range));
    }

    Err(ResolveError::io(
        dep_name,
        io::Error::other(format!(
            "invalid registry version requirement '{requirement}'"
        )),
    ))
}

fn select_sysand_release<'a>(
    dep_name: &str,
    requirement: &RegistryRequirement,
    package_versions: &'a BTreeMap<String, SysandIndexRelease>,
    index_source: &IndexSource,
) -> Result<(String, &'a SysandIndexRelease), ResolveError> {
    match requirement {
        RegistryRequirement::Exact(version) => {
            let version_text = version.to_string();
            let release = package_versions.get(&version_text).ok_or_else(|| {
                ResolveError::io(
                    index_source_context(index_source),
                    io::Error::other(format!(
                        "registry package '{dep_name}' has no release '{version_text}' in Sysand index"
                    )),
                )
            })?;

            Ok((version_text, release))
        }
        RegistryRequirement::Range(range) => {
            let mut best: Option<(Version, &SysandIndexRelease)> = None;
            for (version_text, release) in package_versions {
                let parsed = Version::parse(version_text).map_err(|e| {
                    ResolveError::io(
                        index_source_context(index_source),
                        io::Error::other(format!(
                            "registry package '{dep_name}' contains malformed release version '{version_text}' in Sysand index: {e}"
                        )),
                    )
                })?;
                if !range.matches(&parsed) {
                    continue;
                }

                let replace = best
                    .as_ref()
                    .map(|(current, _)| parsed > *current)
                    .unwrap_or(true);
                if replace {
                    best = Some((parsed, release));
                }
            }

            let Some((selected_version, selected_release)) = best else {
                return Err(ResolveError::io(
                    index_source_context(index_source),
                    io::Error::other(format!(
                        "registry package '{dep_name}' has no compatible release for requirement '{range}' in Sysand index"
                    )),
                ));
            };

            Ok((selected_version.to_string(), selected_release))
        }
    }
}

fn resolve_sysand_index_source(parent_dir: &Path) -> Result<IndexSource, ResolveError> {
    if let Ok(value) = std::env::var(SYSAND_INDEX_ENV) {
        let raw = value.trim();
        if !raw.is_empty() {
            if raw.starts_with("http://") || raw.starts_with("https://") {
                return Ok(IndexSource::RemoteUrl(raw.to_owned()));
            }
            if raw.starts_with("file://") {
                let path = parse_file_url_path(raw).map_err(|e| ResolveError::io(raw, e))?;
                return Ok(IndexSource::Local(path));
            }

            let path = if Path::new(raw).is_absolute() {
                PathBuf::from(raw)
            } else {
                parent_dir.join(raw)
            };
            return Ok(IndexSource::Local(path));
        }
    }

    let local_index = parent_dir.join(SYSAND_LOCAL_INDEX_RELATIVE);
    if local_index.exists() {
        return Ok(IndexSource::Local(local_index));
    }

    Err(ResolveError::UnsupportedSource {
        name: "registry".to_owned(),
        dep_type: "registry-sysand-unconfigured".to_owned(),
    })
}

fn load_sysand_index(index_source: &IndexSource) -> Result<SysandIndex, ResolveError> {
    let json = match index_source {
        IndexSource::Local(path) => {
            fs::read_to_string(path).map_err(|e| ResolveError::io(path, e))?
        }
        IndexSource::RemoteUrl(url) => load_remote_sysand_index(url)?,
    };

    serde_json::from_str(&json).map_err(|e| {
        let remediation = match index_source {
            IndexSource::Local(path) => {
                format!("Fix malformed JSON in '{}' and retry.", path.display())
            }
            IndexSource::RemoteUrl(url) => format!(
                "Fix malformed JSON at '{}' (or clear cached index under '{}') and retry.",
                url,
                cache_root()
                    .join(SYSAND_REMOTE_INDEX_CACHE_RELATIVE)
                    .display()
            ),
        };
        ResolveError::io(
            index_source_context(index_source),
            io::Error::other(format!(
                "failed to parse Sysand registry index JSON: {e}. {remediation}"
            )),
        )
    })
}

fn load_remote_sysand_index(url: &str) -> Result<String, ResolveError> {
    let cache_path = sysand_remote_index_cache_path(url);
    let ttl = Duration::from_secs(SYSAND_REMOTE_INDEX_CACHE_TTL_SECS);

    if is_index_cache_fresh(&cache_path, ttl) {
        return fs::read_to_string(&cache_path).map_err(|e| ResolveError::io(&cache_path, e));
    }

    match fetch_remote_sysand_index(url) {
        Ok(json) => {
            write_text_atomically(&cache_path, &json)?;
            Ok(json)
        }
        Err(fetch_error) => {
            if cache_path.exists() {
                fs::read_to_string(&cache_path).map_err(|e| ResolveError::io(&cache_path, e))
            } else {
                Err(fetch_error)
            }
        }
    }
}

fn fetch_remote_sysand_index(url: &str) -> Result<String, ResolveError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(30))
        .build();

    let response = agent.get(url).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => {
            let hint = match code {
                401 | 403 => {
                    "Authentication/authorization failed. Verify registry credentials/token and retry."
                }
                404 => "Registry index URL was not found (HTTP 404). Verify SYSML_REGISTRY_SYSAND_INDEX.",
                _ => "Verify Sysand index URL/reachability and retry.",
            };
            ResolveError::io(
                url,
                io::Error::other(format!(
                    "failed to fetch Sysand index '{url}': HTTP {code}. {hint}"
                )),
            )
        }
        ureq::Error::Transport(transport) => ResolveError::io(
            url,
            io::Error::other(format!(
                "failed to fetch Sysand index '{url}': {transport}. Check network access or use a local/file index."
            )),
        ),
    })?;

    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    io::copy(&mut reader, &mut bytes).map_err(|e| ResolveError::io(url, e))?;
    String::from_utf8(bytes).map_err(|e| {
        ResolveError::io(
            url,
            io::Error::other(format!("invalid UTF-8 index payload: {e}")),
        )
    })
}

fn is_index_cache_fresh(path: &Path, ttl: Duration) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    match SystemTime::now().duration_since(modified) {
        Ok(age) => age <= ttl,
        Err(_) => true,
    }
}

fn write_text_atomically(path: &Path, content: &str) -> Result<(), ResolveError> {
    let Some(parent) = path.parent() else {
        return Err(ResolveError::io(
            path,
            io::Error::other("index cache path has no parent directory"),
        ));
    };
    fs::create_dir_all(parent).map_err(|e| ResolveError::io(parent, e))?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp = parent.join(format!(".tmp-sysand-index-{}-{nanos}", std::process::id()));
    fs::write(&temp, content).map_err(|e| ResolveError::io(&temp, e))?;
    if let Err(e) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(ResolveError::io(path, e));
    }
    Ok(())
}

fn sysand_remote_index_cache_path(url: &str) -> PathBuf {
    cache_root()
        .join(SYSAND_REMOTE_INDEX_CACHE_RELATIVE)
        .join(format!("{}.json", source_hash(url)))
}

fn cache_root() -> PathBuf {
    if let Some(project_dirs) = ProjectDirs::from("", "", "sysml-rs") {
        return project_dirs.cache_dir().to_path_buf();
    }

    if let Some(base_dirs) = BaseDirs::new() {
        return base_dirs.cache_dir().join("sysml-rs");
    }

    PathBuf::from("/tmp/sysml-rs-cache")
}

fn source_hash(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn index_source_context(index_source: &IndexSource) -> PathBuf {
    match index_source {
        IndexSource::Local(path) => path.clone(),
        IndexSource::RemoteUrl(url) => PathBuf::from(url),
    }
}

fn canonicalize_artifact_url(
    artifact_value: &str,
    index_source: &IndexSource,
) -> Result<String, ResolveError> {
    if artifact_value.starts_with("http://")
        || artifact_value.starts_with("https://")
        || artifact_value.starts_with("file://")
    {
        return Ok(artifact_value.to_owned());
    }

    if has_uri_scheme(artifact_value) {
        return Err(ResolveError::UnsupportedSource {
            name: "registry".to_owned(),
            dep_type: "registry-artifact-scheme".to_owned(),
        });
    }

    let path = match index_source {
        IndexSource::Local(index_path) => {
            let base = index_path.parent().unwrap_or_else(|| Path::new("."));
            let candidate = base.join(artifact_value);
            candidate
                .canonicalize()
                .map_err(|e| ResolveError::io(&candidate, e))?
        }
        IndexSource::RemoteUrl(url) => {
            return Err(ResolveError::io(
                url,
                io::Error::other(format!(
                    "relative registry artifact path '{artifact_value}' cannot be resolved against remote index"
                )),
            ));
        }
    };

    Ok(format!("file://{}", path.display()))
}

fn fetch_url_to_path(package: &str, source: &str, dest_path: &Path) -> Result<(), ResolveError> {
    if source.starts_with("file://") {
        let source_path = parse_file_url_path(source).map_err(|e| ResolveError::io(source, e))?;
        fs::copy(&source_path, dest_path).map_err(|e| ResolveError::io(dest_path, e))?;
        return Ok(());
    }

    if source.starts_with("http://") || source.starts_with("https://") {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(60))
            .timeout_write(Duration::from_secs(60))
            .build();
        let response = agent.get(source).call().map_err(|e| {
            ResolveError::io(
                source,
                io::Error::other(format!("failed to fetch registry artifact '{source}': {e}")),
            )
        })?;
        let mut reader = response.into_reader();
        let mut out = fs::File::create(dest_path).map_err(|e| ResolveError::io(dest_path, e))?;
        io::copy(&mut reader, &mut out).map_err(|e| ResolveError::io(dest_path, e))?;
        return Ok(());
    }

    if has_uri_scheme(source) {
        return Err(ResolveError::UnsupportedSource {
            name: package.to_owned(),
            dep_type: "registry-artifact-remote".to_owned(),
        });
    }

    let source_path = Path::new(source);
    fs::copy(source_path, dest_path).map_err(|e| ResolveError::io(dest_path, e))?;
    Ok(())
}

fn ensure_sha256_checksum_format(dep_name: &str, checksum: &str) -> Result<(), ResolveError> {
    if checksum
        .strip_prefix("sha256:")
        .filter(|hex| !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .is_some()
    {
        return Ok(());
    }

    Err(ResolveError::io(
        dep_name,
        io::Error::other(format!(
            "registry checksum must be in 'sha256:<hex>' format, got '{checksum}'"
        )),
    ))
}

fn has_uri_scheme(source: &str) -> bool {
    source.contains("://")
}

fn parse_file_url_path(url: &str) -> Result<PathBuf, io::Error> {
    let Some(raw) = url.strip_prefix("file://") else {
        return Err(io::Error::other(format!("invalid file URL: {url}")));
    };

    let path_part = if let Some(rest) = raw.strip_prefix("localhost/") {
        format!("/{rest}")
    } else {
        raw.to_owned()
    };

    if !path_part.starts_with('/') {
        return Err(io::Error::other(format!(
            "only absolute file URLs are supported: {url}"
        )));
    }

    Ok(PathBuf::from(percent_decode(path_part.as_str())?))
}

#[allow(clippy::indexing_slicing)] // Loop bounds ensure i, i+1, i+2 are in range
fn percent_decode(input: &str) -> Result<String, io::Error> {
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut out = Vec::with_capacity(bytes.len());
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(io::Error::other("incomplete percent-encoding"));
            }
            let hi = (bytes[i + 1] as char)
                .to_digit(16)
                .ok_or_else(|| io::Error::other("invalid percent-encoding"))?;
            let lo = (bytes[i + 2] as char)
                .to_digit(16)
                .ok_or_else(|| io::Error::other("invalid percent-encoding"))?;
            out.push(((hi << 4) | lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8(out).map_err(|_e| io::Error::other("decoded path was not UTF-8"))
}

fn sha256_hex_file(path: &Path) -> Result<String, ResolveError> {
    let bytes = fs::read(path).map_err(|e| ResolveError::io(path, e))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use tempfile::TempDir;

    fn spawn_one_shot_http_server(
        status_line: &str,
        body: &str,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local address");
        let status = status_line.to_string();
        let body = body.as_bytes().to_vec();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn parse_registry_requirement_accepts_exact_and_ranges() {
        match parse_registry_requirement("units", "1.4.2").unwrap() {
            RegistryRequirement::Exact(version) => assert_eq!(version.to_string(), "1.4.2"),
            other => panic!("expected exact requirement, got: {other:?}"),
        }

        match parse_registry_requirement("units", "^1.4").unwrap() {
            RegistryRequirement::Range(range) => {
                assert!(range.matches(&Version::parse("1.9.0").unwrap()))
            }
            other => panic!("expected range requirement, got: {other:?}"),
        }

        match parse_registry_requirement("units", "~1.4").unwrap() {
            RegistryRequirement::Range(range) => {
                assert!(range.matches(&Version::parse("1.4.9").unwrap()));
                assert!(!range.matches(&Version::parse("1.5.0").unwrap()));
            }
            other => panic!("expected range requirement, got: {other:?}"),
        }
    }

    #[test]
    fn sysand_backend_resolves_local_index_release() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let index_dir = root.join(".sysml/registries/sysand");
        fs::create_dir_all(index_dir.join("artifacts")).unwrap();

        let artifact_path = index_dir.join("artifacts/units-1.4.2.kpar");
        fs::write(&artifact_path, b"dummy-kpar-bytes").unwrap();
        let checksum = format!("sha256:{}", sha256_hex_file(&artifact_path).unwrap());

        fs::write(
            index_dir.join("index.json"),
            format!(
                "{{\"packages\":{{\"units\":{{\"1.4.2\":{{\"artifact\":\"artifacts/units-1.4.2.kpar\",\"checksum\":\"{checksum}\"}}}}}}}}"
            ),
        )
        .unwrap();

        let backend = SysandRegistryBackend;
        let release = backend.resolve_release("units", "1.4.2", root).unwrap();
        assert_eq!(release.backend, "sysand");
        assert_eq!(release.package, "units");
        assert_eq!(release.version, "1.4.2");
        assert_eq!(release.checksum, checksum);
        assert!(release.artifact_url.starts_with("file://"));
    }

    #[test]
    fn sysand_fetch_artifact_checks_checksum() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("artifact.kpar");
        fs::write(&source, b"artifact-bytes").unwrap();
        let checksum = format!("sha256:{}", sha256_hex_file(&source).unwrap());

        let release = RegistryRelease {
            backend: "sysand".to_string(),
            package: "units".to_string(),
            requested: "1.4.2".to_string(),
            version: "1.4.2".to_string(),
            artifact_url: format!("file://{}", source.display()),
            checksum: checksum.clone(),
        };

        let dest = temp.path().join("downloaded.kpar");
        let backend = SysandRegistryBackend;
        let fetched = backend.fetch_artifact(&release, &dest).unwrap();

        assert_eq!(fetched.path, dest);
        assert_eq!(fetched.checksum, checksum);
    }

    #[test]
    fn sysand_fetch_artifact_reports_checksum_mismatch() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("artifact.kpar");
        fs::write(&source, b"artifact-bytes").unwrap();

        let release = RegistryRelease {
            backend: "sysand".to_string(),
            package: "units".to_string(),
            requested: "1.4.2".to_string(),
            version: "1.4.2".to_string(),
            artifact_url: format!("file://{}", source.display()),
            checksum: "sha256:deadbeef".to_string(),
        };

        let dest = temp.path().join("downloaded.kpar");
        let backend = SysandRegistryBackend;
        let err = backend.fetch_artifact(&release, &dest).unwrap_err();
        assert!(matches!(err, ResolveError::ChecksumMismatch { .. }));
    }

    #[test]
    fn sysand_backend_reports_missing_package_with_actionable_message() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let index_dir = root.join(".sysml/registries/sysand");
        fs::create_dir_all(&index_dir).unwrap();
        fs::write(
            index_dir.join("index.json"),
            r#"{"packages":{"units":{"1.4.2":{"artifact":"artifacts/units-1.4.2.kpar","checksum":"sha256:deadbeef"}}}}"#,
        )
        .unwrap();

        let backend = SysandRegistryBackend;
        let err = backend
            .resolve_release("missing-units", "1.4.2", root)
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("missing-units") && msg.contains("not found"),
            "expected actionable missing package message, got: {msg}"
        );
    }

    #[test]
    fn sysand_backend_reports_missing_version_with_actionable_message() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let index_dir = root.join(".sysml/registries/sysand");
        fs::create_dir_all(&index_dir).unwrap();
        fs::write(
            index_dir.join("index.json"),
            r#"{"packages":{"units":{"1.4.2":{"artifact":"artifacts/units-1.4.2.kpar","checksum":"sha256:deadbeef"}}}}"#,
        )
        .unwrap();

        let backend = SysandRegistryBackend;
        let err = backend.resolve_release("units", "9.9.9", root).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("9.9.9") && msg.contains("no release"),
            "expected actionable missing version message, got: {msg}"
        );
    }

    #[test]
    fn sysand_backend_resolves_highest_compatible_range_release() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let index_dir = root.join(".sysml/registries/sysand");
        fs::create_dir_all(index_dir.join("artifacts")).unwrap();

        for version in ["1.2.0", "1.4.2", "1.6.0", "2.0.0"] {
            let artifact_path = index_dir.join(format!("artifacts/units-{version}.kpar"));
            fs::write(&artifact_path, format!("dummy-{version}")).unwrap();
            let checksum = format!("sha256:{}", sha256_hex_file(&artifact_path).unwrap());
            let line = format!(
                "\"{version}\":{{\"artifact\":\"artifacts/units-{version}.kpar\",\"checksum\":\"{checksum}\"}}"
            );
            fs::write(index_dir.join(format!("entry-{version}.json")), line).unwrap();
        }

        let mut entries = Vec::new();
        for version in ["1.2.0", "1.4.2", "1.6.0", "2.0.0"] {
            entries
                .push(fs::read_to_string(index_dir.join(format!("entry-{version}.json"))).unwrap());
        }
        fs::write(
            index_dir.join("index.json"),
            format!("{{\"packages\":{{\"units\":{{{}}}}}}}", entries.join(",")),
        )
        .unwrap();

        let backend = SysandRegistryBackend;
        let release = backend.resolve_release("units", "^1.4", root).unwrap();
        assert_eq!(release.backend, "sysand");
        assert_eq!(release.package, "units");
        assert_eq!(release.requested, "^1.4");
        assert_eq!(release.version, "1.6.0");
    }

    #[test]
    fn sysand_backend_reports_no_compatible_release_for_range() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let index_dir = root.join(".sysml/registries/sysand");
        fs::create_dir_all(index_dir.join("artifacts")).unwrap();
        fs::write(index_dir.join("artifacts/units-1.4.2.kpar"), b"dummy").unwrap();
        fs::write(
            index_dir.join("index.json"),
            r#"{"packages":{"units":{"1.4.2":{"artifact":"artifacts/units-1.4.2.kpar","checksum":"sha256:deadbeef"}}}}"#,
        )
        .unwrap();

        let backend = SysandRegistryBackend;
        let err = backend.resolve_release("units", "^2.0", root).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no compatible release") && msg.contains("^2.0"),
            "expected no-compatible-release message, got: {msg}"
        );
    }

    #[test]
    fn sysand_backend_reports_malformed_version_data_for_ranges() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let index_dir = root.join(".sysml/registries/sysand");
        fs::create_dir_all(index_dir.join("artifacts")).unwrap();
        fs::write(index_dir.join("artifacts/units-bad.kpar"), b"dummy").unwrap();
        fs::write(
            index_dir.join("index.json"),
            r#"{"packages":{"units":{"bad.version":{"artifact":"artifacts/units-bad.kpar","checksum":"sha256:deadbeef"}}}}"#,
        )
        .unwrap();

        let backend = SysandRegistryBackend;
        let err = backend.resolve_release("units", "^1.0", root).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("malformed release version") && msg.contains("bad.version"),
            "expected malformed version data message, got: {msg}"
        );
    }

    #[test]
    fn resolve_registry_release_metadata_returns_requested_and_resolved_versions() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let index_dir = root.join(".sysml/registries/sysand");
        fs::create_dir_all(index_dir.join("artifacts")).unwrap();

        for version in ["1.2.0", "1.4.2", "1.6.0"] {
            let artifact_path = index_dir.join(format!("artifacts/units-{version}.kpar"));
            fs::write(&artifact_path, format!("dummy-{version}")).unwrap();
            let checksum = format!("sha256:{}", sha256_hex_file(&artifact_path).unwrap());
            let line = format!(
                "\"{version}\":{{\"artifact\":\"artifacts/units-{version}.kpar\",\"checksum\":\"{checksum}\"}}"
            );
            fs::write(index_dir.join(format!("entry-{version}.json")), line).unwrap();
        }

        let mut entries = Vec::new();
        for version in ["1.2.0", "1.4.2", "1.6.0"] {
            entries
                .push(fs::read_to_string(index_dir.join(format!("entry-{version}.json"))).unwrap());
        }
        fs::write(
            index_dir.join("index.json"),
            format!("{{\"packages\":{{\"units\":{{{}}}}}}}", entries.join(",")),
        )
        .unwrap();

        let resolved = resolve_registry_release_metadata("sysand", "units", "^1.4", root).unwrap();
        assert_eq!(resolved.backend, "sysand");
        assert_eq!(resolved.package, "units");
        assert_eq!(resolved.requested_requirement, "^1.4");
        assert_eq!(resolved.resolved_version, "1.6.0");
    }

    #[test]
    fn resolve_latest_registry_release_metadata_uses_highest_available_release() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let index_dir = root.join(".sysml/registries/sysand");
        fs::create_dir_all(index_dir.join("artifacts")).unwrap();

        for version in ["2.0.0", "2.1.5", "3.0.0"] {
            let artifact_path = index_dir.join(format!("artifacts/core-{version}.kpar"));
            fs::write(&artifact_path, format!("dummy-{version}")).unwrap();
            let checksum = format!("sha256:{}", sha256_hex_file(&artifact_path).unwrap());
            let line = format!(
                "\"{version}\":{{\"artifact\":\"artifacts/core-{version}.kpar\",\"checksum\":\"{checksum}\"}}"
            );
            fs::write(index_dir.join(format!("entry-{version}.json")), line).unwrap();
        }

        let mut entries = Vec::new();
        for version in ["2.0.0", "2.1.5", "3.0.0"] {
            entries
                .push(fs::read_to_string(index_dir.join(format!("entry-{version}.json"))).unwrap());
        }
        fs::write(
            index_dir.join("index.json"),
            format!("{{\"packages\":{{\"core\":{{{}}}}}}}", entries.join(",")),
        )
        .unwrap();

        let latest = resolve_latest_registry_release_metadata("sysand", "core", root).unwrap();
        assert_eq!(latest.package, "core");
        assert_eq!(latest.requested_requirement, "*");
        assert_eq!(latest.resolved_version, "3.0.0");
    }

    #[test]
    fn remote_sysand_index_uses_stale_cache_when_refresh_fails() {
        let body = r#"{"packages":{"units":{"1.4.2":{"artifact":"https://example.com/units-1.4.2.kpar","checksum":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}}"#;
        let (url, handle) = spawn_one_shot_http_server("200 OK", body);
        let cache_path = sysand_remote_index_cache_path(&url);
        let _ = fs::remove_file(&cache_path);

        let first = load_sysand_index(&IndexSource::RemoteUrl(url.clone()))
            .expect("first remote load should succeed");
        assert!(
            first.packages.contains_key("units"),
            "expected package from remote index payload"
        );
        handle.join().expect("server thread should join");

        std::thread::sleep(Duration::from_secs(2));
        let second = load_sysand_index(&IndexSource::RemoteUrl(url.clone()))
            .expect("stale cached index should be used when refresh fails");
        assert!(
            second.packages.contains_key("units"),
            "expected package from stale cache payload"
        );

        let _ = fs::remove_file(cache_path);
    }

    #[test]
    fn remote_sysand_index_auth_error_is_actionable() {
        let (url, handle) =
            spawn_one_shot_http_server("401 Unauthorized", r#"{"error":"unauthorized"}"#);
        let cache_path = sysand_remote_index_cache_path(&url);
        let _ = fs::remove_file(&cache_path);

        let err = load_sysand_index(&IndexSource::RemoteUrl(url.clone()))
            .expect_err("unauthorized response should fail index fetch");
        handle.join().expect("server thread should join");
        let msg = err.to_string();
        assert!(
            msg.contains("HTTP 401") && msg.contains("credentials"),
            "expected actionable auth failure message, got: {msg}"
        );
    }

    #[test]
    fn local_sysand_index_parse_error_includes_remediation_hint() {
        let temp = TempDir::new().unwrap();
        let index_path = temp.path().join("index.json");
        fs::write(&index_path, "{bad-json").unwrap();

        let err = load_sysand_index(&IndexSource::Local(index_path.clone()))
            .expect_err("malformed local index JSON should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("Fix malformed JSON") && msg.contains(&index_path.display().to_string()),
            "expected remediation hint in parse error, got: {msg}"
        );
    }
}
