//! Source-provider architecture for dependency resolution.
//!
//! Providers encapsulate source-type specific behavior so the resolver
//! can stay source-agnostic.

use std::fs;
use std::io;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use directories::{BaseDirs, ProjectDirs};
use sha2::{Digest, Sha256};
use sysml_project::kpar::KparArchive;
use sysml_manifest::{load_manifest, save_manifest, Dependency, GitRef, SysmlManifest};
use tracing::{debug, trace, warn};

use crate::error::ResolveError;
use crate::graph::PackageSource;
use crate::registry::{RegistryBackendRegistry, RegistryRelease, DEFAULT_REGISTRY_BACKEND};

pub(crate) const KPAR_CHECKSUM_FILENAME: &str = ".sysml-kpar-checksum";

enum KparSourceSpec {
    Local {
        archive_path: PathBuf,
        source_key: String,
    },
    RemoteHttp {
        url: String,
        source_key: String,
    },
}

/// A dependency resolved by a source provider.
#[derive(Debug)]
pub(crate) struct ProviderResolution {
    /// Loaded manifest for the resolved dependency package.
    pub manifest: SysmlManifest,
    /// Canonical directory containing the package manifest and sources.
    pub source_dir: PathBuf,
    /// Source metadata used by lockfile generation.
    pub source: PackageSource,
}

impl ProviderResolution {
    /// Identity fingerprint used for deduplication.
    pub fn source_fingerprint(&self) -> String {
        self.source.to_lock_source()
    }
}

/// Resolves dependencies for one source family.
pub(crate) trait SourceProvider {
    fn supports(&self, dep: &Dependency) -> bool;

    fn resolve(
        &self,
        dep_name: &str,
        dep: &Dependency,
        parent_dir: &Path,
    ) -> Result<ProviderResolution, ResolveError>;
}

/// Registry of source providers used by the resolver.
pub(crate) struct SourceProviderRegistry {
    providers: Vec<Box<dyn SourceProvider + Send + Sync>>,
}

impl SourceProviderRegistry {
    pub fn new() -> Self {
        SourceProviderRegistry {
            providers: vec![
                Box::new(PathProvider),
                Box::new(GitProvider),
                Box::new(KparProvider),
                Box::new(RegistryProvider::default()),
            ],
        }
    }

    pub fn resolve(
        &self,
        dep_name: &str,
        dep: &Dependency,
        parent_dir: &Path,
    ) -> Result<ProviderResolution, ResolveError> {
        for provider in &self.providers {
            if provider.supports(dep) {
                trace!(
                    dependency = %dep_name,
                    source = dependency_source_kind(dep),
                    parent_dir = %parent_dir.display(),
                    "dispatching dependency to source provider"
                );
                return provider.resolve(dep_name, dep, parent_dir);
            }
        }

        Err(ResolveError::UnsupportedSource {
            name: dep_name.to_owned(),
            dep_type: "unknown".to_owned(),
        })
    }
}

impl Default for SourceProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Local path dependency provider.
#[derive(Debug, Default)]
pub(crate) struct PathProvider;

impl SourceProvider for PathProvider {
    fn supports(&self, dep: &Dependency) -> bool {
        dep.is_path()
    }

    fn resolve(
        &self,
        dep_name: &str,
        dep: &Dependency,
        parent_dir: &Path,
    ) -> Result<ProviderResolution, ResolveError> {
        let started = Instant::now();
        let rel_path = dep
            .as_path()
            .ok_or_else(|| ResolveError::UnsupportedSource {
                name: dep_name.to_owned(),
                dep_type: "path".to_owned(),
            })?;

        let unresolved_dir = parent_dir.join(rel_path);
        let source_dir =
            unresolved_dir
                .canonicalize()
                .map_err(|_e| ResolveError::MissingDependency {
                    name: dep_name.to_owned(),
                    path: unresolved_dir.clone(),
                })?;

        let dep_manifest_path = source_dir.join("sysml.toml");
        if !dep_manifest_path.exists() {
            return Err(ResolveError::MissingDependency {
                name: dep_name.to_owned(),
                path: source_dir,
            });
        }

        let manifest = load_manifest(&dep_manifest_path)?;
        debug!(
            dependency = %dep_name,
            source = "path",
            source_dir = %source_dir.display(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "resolved path dependency"
        );

        Ok(ProviderResolution {
            manifest,
            source_dir,
            source: PackageSource::Path(rel_path.to_owned()),
        })
    }
}

/// Git provider.
#[derive(Debug, Default)]
pub(crate) struct GitProvider;

impl GitProvider {
    fn ensure_mirror(&self, url: &str, objects_dir: &Path) -> Result<(), ResolveError> {
        if objects_dir.exists() {
            if objects_dir.is_file() {
                fs::remove_file(objects_dir).map_err(|e| ResolveError::io(objects_dir, e))?;
            } else {
                let is_bare = run_git_stdout(
                    objects_dir,
                    Some(objects_dir),
                    &["rev-parse", "--is-bare-repository"],
                )
                .map(|v| v == "true")
                .unwrap_or(false);

                if is_bare
                    && run_git_output(
                        objects_dir,
                        Some(objects_dir),
                        &["fetch", "--prune", "--tags", "origin"],
                    )
                    .is_ok()
                {
                    return Ok(());
                }

                fs::remove_dir_all(objects_dir).map_err(|e| ResolveError::io(objects_dir, e))?;
            }
        }

        let Some(parent) = objects_dir.parent() else {
            return Err(ResolveError::io(
                objects_dir,
                io::Error::other("git objects dir has no parent"),
            ));
        };
        fs::create_dir_all(parent).map_err(|e| ResolveError::io(parent, e))?;

        let objects = objects_dir.to_string_lossy().to_string();
        run_git_output(objects_dir, None, &["clone", "--mirror", url, &objects])?;
        Ok(())
    }

    fn resolve_commit(&self, objects_dir: &Path, git_ref: &GitRef) -> Result<String, ResolveError> {
        match git_ref {
            GitRef::Rev(rev) => self.rev_parse_commit(objects_dir, &format!("{rev}^{{commit}}")),
            GitRef::Tag(tag) => {
                self.rev_parse_commit(objects_dir, &format!("refs/tags/{tag}^{{commit}}"))
            }
            GitRef::Branch(branch) => {
                let local = format!("refs/heads/{branch}^{{commit}}");
                if let Ok(commit) = self.rev_parse_commit(objects_dir, &local) {
                    return Ok(commit);
                }

                self.rev_parse_commit(
                    objects_dir,
                    &format!("refs/remotes/origin/{branch}^{{commit}}"),
                )
            }
            GitRef::DefaultBranch => self.rev_parse_commit(objects_dir, "HEAD^{commit}"),
        }
    }

    fn rev_parse_commit(&self, objects_dir: &Path, revspec: &str) -> Result<String, ResolveError> {
        run_git_stdout(
            objects_dir,
            Some(objects_dir),
            &["rev-parse", "--verify", revspec],
        )
    }

    fn ensure_checkout(
        &self,
        objects_dir: &Path,
        checkout_dir: &Path,
        commit: &str,
    ) -> Result<(), ResolveError> {
        if checkout_dir.exists() {
            if checkout_dir.is_dir() {
                return Ok(());
            }
            fs::remove_file(checkout_dir).map_err(|e| ResolveError::io(checkout_dir, e))?;
        }

        let Some(checkouts_root) = checkout_dir.parent() else {
            return Err(ResolveError::io(
                checkout_dir,
                io::Error::other("checkout dir has no parent"),
            ));
        };
        fs::create_dir_all(checkouts_root).map_err(|e| ResolveError::io(checkouts_root, e))?;

        let commit_short = &commit[..commit.len().min(12)];
        let tmp_checkout = checkouts_root.join(format!(".tmp-{commit_short}-{}", unique_suffix()));

        if tmp_checkout.exists() {
            if tmp_checkout.is_dir() {
                let _ = fs::remove_dir_all(&tmp_checkout);
            } else {
                let _ = fs::remove_file(&tmp_checkout);
            }
        }

        let mirror = objects_dir.to_string_lossy().to_string();
        let tmp = tmp_checkout.to_string_lossy().to_string();
        run_git_output(
            checkouts_root,
            None,
            &["clone", "--no-checkout", &mirror, &tmp],
        )?;

        if let Err(err) = run_git_output(
            &tmp_checkout,
            Some(&tmp_checkout),
            &["checkout", "--detach", commit],
        ) {
            let _ = fs::remove_dir_all(&tmp_checkout);
            return Err(err);
        }

        match fs::rename(&tmp_checkout, checkout_dir) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && checkout_dir.is_dir() => {
                let _ = fs::remove_dir_all(&tmp_checkout);
                Ok(())
            }
            Err(err) => {
                let _ = fs::remove_dir_all(&tmp_checkout);
                Err(ResolveError::io(checkout_dir, err))
            }
        }
    }
}

impl SourceProvider for GitProvider {
    fn supports(&self, dep: &Dependency) -> bool {
        dep.is_git()
    }

    fn resolve(
        &self,
        dep_name: &str,
        dep: &Dependency,
        _parent_dir: &Path,
    ) -> Result<ProviderResolution, ResolveError> {
        let started = Instant::now();
        let url = dep
            .as_git_url()
            .ok_or_else(|| ResolveError::UnsupportedSource {
                name: dep_name.to_owned(),
                dep_type: "git".to_owned(),
            })?;

        let git_ref = dep.git_ref().unwrap_or(GitRef::DefaultBranch);
        debug!(
            dependency = %dep_name,
            source = "git",
            url_hash = %source_hash(url),
            git_ref = %git_ref_label(&git_ref),
            "resolving git dependency"
        );

        let cache_dir = git_cache_dir_for_url(url);
        let objects_dir = cache_dir.join("objects");
        self.ensure_mirror(url, &objects_dir)?;

        let commit = self.resolve_commit(&objects_dir, &git_ref)?;
        let checkout_dir = cache_dir.join("checkouts").join(&commit);
        self.ensure_checkout(&objects_dir, &checkout_dir, &commit)?;

        let manifest_path = checkout_dir.join("sysml.toml");
        let manifest = match load_manifest(&manifest_path) {
            Ok(manifest) => manifest,
            Err(_) => {
                // Recovery path for dirty/corrupted checkout contents.
                let _ = fs::remove_dir_all(&checkout_dir);
                self.ensure_checkout(&objects_dir, &checkout_dir, &commit)?;
                load_manifest(&manifest_path)?
            }
        };

        let source_dir = checkout_dir
            .canonicalize()
            .map_err(|e| ResolveError::io(&checkout_dir, e))?;
        debug!(
            dependency = %dep_name,
            source = "git",
            url_hash = %source_hash(url),
            commit = %commit,
            source_dir = %source_dir.display(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "resolved git dependency"
        );

        Ok(ProviderResolution {
            manifest,
            source_dir,
            source: PackageSource::Git {
                url: url.to_owned(),
                commit,
            },
        })
    }
}

/// KPAR provider.
#[derive(Debug, Default)]
pub(crate) struct KparProvider;

impl KparProvider {
    fn resolve_source_spec(
        &self,
        dep_name: &str,
        source: &str,
        parent_dir: &Path,
    ) -> Result<KparSourceSpec, ResolveError> {
        let unresolved_path = if source.starts_with("file://") {
            parse_file_url_path(source).map_err(|e| ResolveError::io(source, e))?
        } else if is_http_url(source) {
            return Ok(KparSourceSpec::RemoteHttp {
                url: source.to_owned(),
                source_key: source.to_owned(),
            });
        } else if has_uri_scheme(source) {
            return Err(ResolveError::UnsupportedSource {
                name: dep_name.to_owned(),
                dep_type: "kpar-remote".to_owned(),
            });
        } else {
            parent_dir.join(source)
        };

        let source_path =
            unresolved_path
                .canonicalize()
                .map_err(|_e| ResolveError::MissingDependency {
                    name: dep_name.to_owned(),
                    path: unresolved_path.clone(),
                })?;

        if !source_path.is_file() {
            return Err(ResolveError::MissingDependency {
                name: dep_name.to_owned(),
                path: source_path,
            });
        }

        let source_key = canonical_file_source(&source_path);
        Ok(KparSourceSpec::Local {
            archive_path: source_path,
            source_key,
        })
    }

    fn download_http_archive_to_temp(
        &self,
        url: &str,
        source_cache_dir: &Path,
    ) -> Result<PathBuf, ResolveError> {
        let archives_dir = source_cache_dir.join("archives");
        fs::create_dir_all(&archives_dir).map_err(|e| ResolveError::io(&archives_dir, e))?;
        let tmp_archive = archives_dir.join(format!(".tmp-download-{}.kpar", unique_suffix()));

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(60))
            .timeout_write(Duration::from_secs(60))
            .build();
        let response = agent.get(url).call().map_err(|e| {
            ResolveError::io(
                url,
                io::Error::other(format!("failed to download KPAR source '{url}': {e}")),
            )
        })?;

        let mut reader = response.into_reader();
        let mut out =
            fs::File::create(&tmp_archive).map_err(|e| ResolveError::io(&tmp_archive, e))?;
        io::copy(&mut reader, &mut out).map_err(|e| ResolveError::io(&tmp_archive, e))?;

        Ok(tmp_archive)
    }

    fn ensure_cached_archive(
        &self,
        source_archive: &Path,
        cached_archive: &Path,
        expected_checksum_hex: &str,
    ) -> Result<(), ResolveError> {
        if cached_archive.exists() {
            if cached_archive.is_file() {
                let actual = sha256_hex_file(cached_archive)?;
                if actual != expected_checksum_hex {
                    return Err(ResolveError::checksum_mismatch(
                        cached_archive,
                        format!("sha256:{expected_checksum_hex}"),
                        format!("sha256:{actual}"),
                        "Cached KPAR archive is corrupted; remove cache and retry",
                    ));
                }
                return Ok(());
            }

            fs::remove_dir_all(cached_archive).map_err(|e| ResolveError::io(cached_archive, e))?;
        }

        let Some(archives_dir) = cached_archive.parent() else {
            return Err(ResolveError::io(
                cached_archive,
                io::Error::other("kpar archive cache path has no parent"),
            ));
        };
        fs::create_dir_all(archives_dir).map_err(|e| ResolveError::io(archives_dir, e))?;

        let tmp = archives_dir.join(format!(".tmp-{}.kpar", unique_suffix()));
        fs::copy(source_archive, &tmp).map_err(|e| ResolveError::io(&tmp, e))?;

        let copied_checksum_hex = sha256_hex_file(&tmp)?;
        if copied_checksum_hex != expected_checksum_hex {
            let _ = fs::remove_file(&tmp);
            return Err(ResolveError::checksum_mismatch(
                &tmp,
                format!("sha256:{expected_checksum_hex}"),
                format!("sha256:{copied_checksum_hex}"),
                "Copied KPAR archive bytes changed unexpectedly during caching",
            ));
        }

        match fs::rename(&tmp, cached_archive) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && cached_archive.is_file() => {
                let _ = fs::remove_file(&tmp);
                Ok(())
            }
            Err(err) => {
                let _ = fs::remove_file(&tmp);
                Err(ResolveError::io(cached_archive, err))
            }
        }
    }

    fn ensure_extracted(
        &self,
        archive_path: &Path,
        extracted_dir: &Path,
        checksum: &str,
    ) -> Result<(), ResolveError> {
        if extracted_dir.exists() {
            if extracted_dir.is_dir() && extracted_cache_is_valid(extracted_dir, checksum) {
                return Ok(());
            }

            if extracted_dir.is_dir() {
                fs::remove_dir_all(extracted_dir)
                    .map_err(|e| ResolveError::io(extracted_dir, e))?;
            } else {
                fs::remove_file(extracted_dir).map_err(|e| ResolveError::io(extracted_dir, e))?;
            }
        }

        let Some(extracted_root) = extracted_dir.parent() else {
            return Err(ResolveError::io(
                extracted_dir,
                io::Error::other("kpar extracted cache path has no parent"),
            ));
        };
        fs::create_dir_all(extracted_root).map_err(|e| ResolveError::io(extracted_root, e))?;

        let tmp_dir = extracted_root.join(format!(".tmp-{}", unique_suffix()));
        if tmp_dir.exists() {
            if tmp_dir.is_dir() {
                let _ = fs::remove_dir_all(&tmp_dir);
            } else {
                let _ = fs::remove_file(&tmp_dir);
            }
        }

        materialize_kpar_extraction(archive_path, &tmp_dir, checksum)?;

        match fs::rename(&tmp_dir, extracted_dir) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && extracted_dir.is_dir() => {
                let _ = fs::remove_dir_all(&tmp_dir);
                Ok(())
            }
            Err(err) => {
                let _ = fs::remove_dir_all(&tmp_dir);
                Err(ResolveError::io(extracted_dir, err))
            }
        }
    }
}

impl SourceProvider for KparProvider {
    fn supports(&self, dep: &Dependency) -> bool {
        dep.is_kpar()
    }

    fn resolve(
        &self,
        dep_name: &str,
        dep: &Dependency,
        parent_dir: &Path,
    ) -> Result<ProviderResolution, ResolveError> {
        let started = Instant::now();
        let source = dep
            .as_kpar_url()
            .ok_or_else(|| ResolveError::UnsupportedSource {
                name: dep_name.to_owned(),
                dep_type: "kpar".to_owned(),
            })?;
        debug!(
            dependency = %dep_name,
            source = "kpar",
            source_key = %source_hash(source),
            "resolving kpar dependency"
        );

        let source_spec = self.resolve_source_spec(dep_name, source, parent_dir)?;
        let source_key = match &source_spec {
            KparSourceSpec::Local { source_key, .. } => source_key.clone(),
            KparSourceSpec::RemoteHttp { source_key, .. } => source_key.clone(),
        };
        let source_cache_dir = kpar_cache_dir_for_source(&source_key);
        let (source_archive, ephemeral_archive) = match &source_spec {
            KparSourceSpec::Local { archive_path, .. } => (archive_path.clone(), false),
            KparSourceSpec::RemoteHttp { url, .. } => (
                self.download_http_archive_to_temp(url, &source_cache_dir)?,
                true,
            ),
        };

        let archive_checksum_hex = sha256_hex_file(&source_archive)?;
        let archive_checksum = format!("sha256:{archive_checksum_hex}");
        let cached_archive = source_cache_dir
            .join("archives")
            .join(format!("{archive_checksum_hex}.kpar"));
        self.ensure_cached_archive(&source_archive, &cached_archive, &archive_checksum_hex)?;
        if ephemeral_archive {
            let _ = fs::remove_file(&source_archive);
        }

        let extracted_dir = source_cache_dir
            .join("extracted")
            .join(&archive_checksum_hex);
        self.ensure_extracted(&cached_archive, &extracted_dir, &archive_checksum)?;

        let manifest_path = extracted_dir.join("sysml.toml");
        let manifest = match load_manifest(&manifest_path) {
            Ok(manifest) => manifest,
            Err(_) => {
                // Recovery path for dirty/corrupted extraction contents.
                let _ = fs::remove_dir_all(&extracted_dir);
                self.ensure_extracted(&cached_archive, &extracted_dir, &archive_checksum)?;
                load_manifest(&manifest_path)?
            }
        };

        let source_dir = extracted_dir
            .canonicalize()
            .map_err(|e| ResolveError::io(&extracted_dir, e))?;
        debug!(
            dependency = %dep_name,
            source = "kpar",
            source_key = %source_hash(source),
            archive_checksum = %archive_checksum,
            source_dir = %source_dir.display(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "resolved kpar dependency"
        );

        Ok(ProviderResolution {
            manifest,
            source_dir,
            source: PackageSource::Kpar {
                url: source.to_owned(),
            },
        })
    }
}

/// Registry provider backed by pluggable registry backends (Sysand first).
#[derive(Debug, Default)]
pub(crate) struct RegistryProvider {
    backends: RegistryBackendRegistry,
    kpar: KparProvider,
}

impl RegistryProvider {
    fn resolve_dependency_spec<'a>(
        &self,
        dep_name: &str,
        dep: &'a Dependency,
    ) -> Result<(String, &'a str), ResolveError> {
        let requirement = dep
            .as_registry_requirement()
            .ok_or_else(|| ResolveError::UnsupportedSource {
                name: dep_name.to_owned(),
                dep_type: "registry".to_owned(),
            })?
            .trim()
            .to_owned();
        let backend = dep.registry_backend().unwrap_or(DEFAULT_REGISTRY_BACKEND);

        Ok((requirement, backend))
    }

    fn checksum_hex<'a>(&self, dep_name: &str, checksum: &'a str) -> Result<&'a str, ResolveError> {
        checksum.strip_prefix("sha256:").ok_or_else(|| {
            ResolveError::io(
                dep_name,
                io::Error::other(format!(
                    "registry checksum must start with 'sha256:', got '{checksum}'"
                )),
            )
        })
    }

    fn materialize_registry_release(
        &self,
        dep_name: &str,
        release: &RegistryRelease,
        cache_dir: &Path,
        backend_id: &str,
    ) -> Result<(PathBuf, String), ResolveError> {
        let checksum_hex = self.checksum_hex(dep_name, &release.checksum)?;
        let archives_dir = cache_dir.join("artifacts");
        fs::create_dir_all(&archives_dir).map_err(|e| ResolveError::io(&archives_dir, e))?;
        let tmp_artifact = archives_dir.join(format!(".tmp-registry-{}.kpar", unique_suffix()));

        let fetched = self
            .backends
            .fetch_artifact(backend_id, release, &tmp_artifact)?;
        if fetched.checksum != release.checksum {
            return Err(ResolveError::checksum_mismatch(
                &fetched.path,
                release.checksum.clone(),
                fetched.checksum.clone(),
                "Registry backend checksum proof diverged from release metadata",
            ));
        }

        let cached_archive = archives_dir.join(format!("{checksum_hex}.kpar"));
        self.kpar
            .ensure_cached_archive(&fetched.path, &cached_archive, checksum_hex)?;
        let _ = fs::remove_file(&fetched.path);

        let extracted_dir = cache_dir.join("extracted").join(checksum_hex);
        self.kpar
            .ensure_extracted(&cached_archive, &extracted_dir, &release.checksum)?;
        Ok((extracted_dir, release.checksum.clone()))
    }
}

impl SourceProvider for RegistryProvider {
    fn supports(&self, dep: &Dependency) -> bool {
        dep.is_registry()
    }

    fn resolve(
        &self,
        dep_name: &str,
        dep: &Dependency,
        parent_dir: &Path,
    ) -> Result<ProviderResolution, ResolveError> {
        let started = Instant::now();
        let (requirement, backend_id) = self.resolve_dependency_spec(dep_name, dep)?;
        debug!(
            dependency = %dep_name,
            source = "registry",
            backend = %backend_id,
            requirement = %requirement,
            "resolving registry dependency"
        );
        let release =
            self.backends
                .resolve_release(backend_id, dep_name, &requirement, parent_dir)?;

        let cache_dir = registry_cache_dir_for_request(backend_id, dep_name, &requirement);
        let (extracted_dir, _checksum) =
            self.materialize_registry_release(dep_name, &release, &cache_dir, backend_id)?;

        let manifest_path = extracted_dir.join("sysml.toml");
        let manifest = match load_manifest(&manifest_path) {
            Ok(manifest) => manifest,
            Err(_) => {
                // Recovery path for dirty/corrupted extraction contents.
                let _ = fs::remove_dir_all(&extracted_dir);
                let _ =
                    self.materialize_registry_release(dep_name, &release, &cache_dir, backend_id)?;
                load_manifest(&manifest_path)?
            }
        };

        let source_dir = extracted_dir
            .canonicalize()
            .map_err(|e| ResolveError::io(&extracted_dir, e))?;
        debug!(
            dependency = %dep_name,
            source = "registry",
            backend = %release.backend,
            package = %release.package,
            requested = %release.requested,
            version = %release.version,
            source_dir = %source_dir.display(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "resolved registry dependency"
        );

        Ok(ProviderResolution {
            manifest,
            source_dir,
            source: PackageSource::Registry {
                backend: release.backend.clone(),
                package: release.package.clone(),
                requested: release.requested.clone(),
                version: release.version,
            },
        })
    }
}

fn run_git_output(
    context: &Path,
    cwd: Option<&Path>,
    args: &[&str],
) -> Result<Output, ResolveError> {
    let started = Instant::now();
    let subcommand = args.first().copied().unwrap_or("unknown");
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let output = cmd.output().map_err(|e| ResolveError::io(context, e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit status {}", output.status)
        };
        warn!(
            context = %context.display(),
            command = %subcommand,
            elapsed_ms = started.elapsed().as_millis() as u64,
            detail = %detail,
            "git command failed"
        );

        return Err(ResolveError::io(
            context,
            io::Error::other(format!("git {} failed: {detail}", args.join(" "))),
        ));
    }

    trace!(
        context = %context.display(),
        command = %subcommand,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "git command completed"
    );

    Ok(output)
}

fn run_git_stdout(
    context: &Path,
    cwd: Option<&Path>,
    args: &[&str],
) -> Result<String, ResolveError> {
    let output = run_git_output(context, cwd, args)?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn cache_root() -> PathBuf {
    if let Ok(override_dir) = std::env::var("SYSML_RS_CACHE_DIR") {
        if !override_dir.trim().is_empty() {
            return PathBuf::from(override_dir);
        }
    }

    if let Some(project_dirs) = ProjectDirs::from("", "", "sysml-rs") {
        return project_dirs.cache_dir().to_path_buf();
    }

    if let Some(base_dirs) = BaseDirs::new() {
        return base_dirs.cache_dir().join("sysml-rs");
    }

    PathBuf::from("/tmp/sysml-rs-cache")
}

fn dependency_cache_root() -> PathBuf {
    cache_root().join("dependencies")
}

fn source_hash(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn git_cache_dir_for_url(url: &str) -> PathBuf {
    dependency_cache_root().join("git").join(source_hash(url))
}

fn kpar_cache_dir_for_source(source_key: &str) -> PathBuf {
    dependency_cache_root()
        .join("kpar")
        .join(source_hash(source_key))
}

fn registry_cache_dir_for_request(backend: &str, package: &str, requirement: &str) -> PathBuf {
    let request_key = format!("{backend}:{package}@{requirement}");
    dependency_cache_root()
        .join("registry")
        .join(backend)
        .join(source_hash(&request_key))
}

fn dependency_source_kind(dep: &Dependency) -> &'static str {
    if dep.is_path() {
        "path"
    } else if dep.is_git() {
        "git"
    } else if dep.is_kpar() {
        "kpar"
    } else if dep.is_registry() {
        "registry"
    } else {
        "unknown"
    }
}

fn git_ref_label(git_ref: &GitRef) -> String {
    match git_ref {
        GitRef::Rev(rev) => format!("rev:{rev}"),
        GitRef::Tag(tag) => format!("tag:{tag}"),
        GitRef::Branch(branch) => format!("branch:{branch}"),
        GitRef::DefaultBranch => "default-branch".to_owned(),
    }
}

fn has_uri_scheme(source: &str) -> bool {
    source.contains("://")
}

fn is_http_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
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

fn canonical_file_source(path: &Path) -> String {
    format!("file://{}", path.display())
}

#[allow(clippy::indexing_slicing)] // read <= buf.len(), so buf[..read] is always valid
fn sha256_hex_file(path: &Path) -> Result<String, ResolveError> {
    let mut file = fs::File::open(path).map_err(|e| ResolveError::io(path, e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 16 * 1024];

    loop {
        let read = file.read(&mut buf).map_err(|e| ResolveError::io(path, e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn extracted_cache_is_valid(extracted_dir: &Path, checksum: &str) -> bool {
    let marker_path = extracted_dir.join(KPAR_CHECKSUM_FILENAME);
    let marker_matches = fs::read_to_string(&marker_path)
        .map(|s| s.trim() == checksum)
        .unwrap_or(false);
    if !marker_matches {
        return false;
    }

    let manifest_path = extracted_dir.join("sysml.toml");
    manifest_path.exists() && load_manifest(&manifest_path).is_ok()
}

fn materialize_kpar_extraction(
    archive_path: &Path,
    out_dir: &Path,
    checksum: &str,
) -> Result<(), ResolveError> {
    fs::create_dir_all(out_dir).map_err(|e| ResolveError::io(out_dir, e))?;

    let archive = sysml_project::kpar::read_kpar(archive_path).map_err(|e| {
        ResolveError::io(
            archive_path,
            io::Error::other(format!("failed to parse KPAR archive: {e}")),
        )
    })?;

    for (relative, contents) in &archive.source_files {
        let rel_path = normalize_kpar_relative_path(relative).map_err(|e| {
            ResolveError::io(
                archive_path,
                io::Error::other(format!("invalid path '{relative}' in KPAR: {e}")),
            )
        })?;
        let target = out_dir.join(rel_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| ResolveError::io(parent, e))?;
        }
        fs::write(&target, contents).map_err(|e| ResolveError::io(&target, e))?;
    }

    let manifest = build_manifest_from_kpar(&archive);
    save_manifest(&out_dir.join("sysml.toml"), &manifest)?;
    fs::write(
        out_dir.join(KPAR_CHECKSUM_FILENAME),
        format!("{checksum}\n"),
    )
    .map_err(|e| ResolveError::io(out_dir.join(KPAR_CHECKSUM_FILENAME), e))?;

    Ok(())
}

fn normalize_kpar_relative_path(relative: &str) -> Result<PathBuf, io::Error> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty() {
        return Err(io::Error::other("empty path"));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(seg) => normalized.push(seg),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::other("path traversal is not allowed"));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(io::Error::other("empty normalized path"));
    }

    Ok(normalized)
}

fn build_manifest_from_kpar(archive: &KparArchive) -> SysmlManifest {
    let mut manifest = SysmlManifest::new(
        archive.project_info.name.clone(),
        archive.project_info.version.clone(),
    );
    manifest.project.description = archive.project_info.description.clone();
    manifest.project.license = archive.project_info.license.clone();

    for usage in &archive.project_info.usage {
        let Some(dep) = usage_to_dependency(usage.resource.as_str()) else {
            continue;
        };
        let dep_name = unique_dependency_name(
            &manifest,
            &dependency_name_hint_from_resource(usage.resource.as_str()),
        );
        manifest.dependencies.insert(dep_name, dep);
    }

    manifest
}

fn usage_to_dependency(resource: &str) -> Option<Dependency> {
    if resource.trim().is_empty() {
        return None;
    }

    if resource.starts_with("urn:") {
        return None;
    }

    if resource.ends_with(".kpar") || resource.contains(".kpar?") || resource.contains(".kpar#") {
        return Some(Dependency::kpar(resource.to_owned()));
    }

    if is_git_like_resource(resource) || (is_http_url(resource) && !resource.contains(".kpar")) {
        // Usage metadata doesn't carry git refs; use default branch semantics.
        return Some(Dependency::git_branch(resource.to_owned(), "main"));
    }

    None
}

fn is_git_like_resource(resource: &str) -> bool {
    resource.starts_with("git://")
        || resource.starts_with("ssh://")
        || resource.starts_with("git@")
        || resource.ends_with(".git")
}

fn unique_dependency_name(manifest: &SysmlManifest, base: &str) -> String {
    if !manifest.dependencies.contains_key(base) {
        return base.to_owned();
    }

    let mut idx = 2usize;
    loop {
        let candidate = format!("{base}-{idx}");
        if !manifest.dependencies.contains_key(&candidate) {
            return candidate;
        }
        idx += 1;
    }
}

fn dependency_name_hint_from_resource(resource: &str) -> String {
    let trimmed = resource.trim_end_matches('/');
    let segment = trimmed
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("dep");

    let stem = segment
        .split(['?', '#'])
        .next()
        .unwrap_or(segment)
        .trim_end_matches(".kpar")
        .trim_end_matches(".git");
    let mut out = String::new();
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }

    let normalized = out.trim_matches('-');
    if normalized.is_empty() {
        format!("dep-{}", &source_hash(resource)[..8])
    } else {
        normalized.to_owned()
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::thread;
    use sysml_project::kpar::{write_kpar, KparArchive, ProjectInfo, ProjectMetadata};
    use tempfile::TempDir;

    #[test]
    fn path_provider_loads_manifest() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let dep_dir = root.join("dep");
        fs::create_dir_all(&dep_dir).unwrap();
        fs::write(
            dep_dir.join("sysml.toml"),
            "[project]\nname = \"dep\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let provider = PathProvider;
        let dep = Dependency::path("./dep");

        let resolved = provider.resolve("dep", &dep, root).unwrap();
        assert_eq!(resolved.manifest.project.name, "dep");
        assert_eq!(resolved.manifest.project.version, "0.1.0");
        assert_eq!(resolved.source, PackageSource::Path("./dep".to_string()));
        assert_eq!(resolved.source_dir, dep_dir.canonicalize().unwrap());
    }

    #[test]
    fn registry_provider_is_selected_for_short_form() {
        let registry = SourceProviderRegistry::new();
        let dep = Dependency::registry("1.2.3");
        let tmp = TempDir::new().unwrap();

        let result = registry.resolve("remote", &dep, tmp.path());
        assert!(matches!(
            result,
            Err(ResolveError::UnsupportedSource { dep_type, .. }) if dep_type == "registry-sysand-unconfigured"
        ));
    }

    #[test]
    fn registry_provider_resolves_exact_version_from_local_sysand_index() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let archive_path = create_kpar_archive(root, "units-provider", "9.9.1");
        clean_registry_cache_for_request("sysand", "units-provider", "9.9.1");

        write_sysand_index_for_package(root, "units-provider", "9.9.1", &archive_path);

        let provider = RegistryProvider::default();
        let dep = Dependency::registry("9.9.1");
        let resolved = provider.resolve("units-provider", &dep, root).unwrap();

        assert_eq!(resolved.manifest.project.name, "units-provider");
        assert_eq!(
            resolved.source,
            PackageSource::Registry {
                backend: "sysand".to_string(),
                package: "units-provider".to_string(),
                requested: "9.9.1".to_string(),
                version: "9.9.1".to_string(),
            }
        );
        assert!(resolved.source_dir.join("sysml.toml").exists());
        assert!(resolved.source_dir.join(KPAR_CHECKSUM_FILENAME).exists());

        clean_registry_cache_for_request("sysand", "units-provider", "9.9.1");
    }

    #[test]
    fn registry_provider_resolves_caret_range_to_latest_compatible_release() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let archive_v1 = create_kpar_archive(root, "units-range", "1.4.0");
        let archive_v1_pinned = root.join("units-range-1.4.0.kpar");
        fs::copy(&archive_v1, &archive_v1_pinned).unwrap();
        let archive_v2 = create_kpar_archive(root, "units-range", "1.8.1");
        let archive_v2_pinned = root.join("units-range-1.8.1.kpar");
        fs::copy(&archive_v2, &archive_v2_pinned).unwrap();
        let archive_v3 = create_kpar_archive(root, "units-range", "2.0.0");
        let archive_v3_pinned = root.join("units-range-2.0.0.kpar");
        fs::copy(&archive_v3, &archive_v3_pinned).unwrap();
        clean_registry_cache_for_request("sysand", "units-range", "^1.0");
        write_sysand_index_with_releases(
            root,
            "units-range",
            &[
                ("1.4.0", &archive_v1_pinned),
                ("1.8.1", &archive_v2_pinned),
                ("2.0.0", &archive_v3_pinned),
            ],
        );

        let provider = RegistryProvider::default();
        let dep = Dependency::registry("^1.0");
        let resolved = provider.resolve("units-range", &dep, root).unwrap();

        assert_eq!(resolved.manifest.project.version, "1.8.1");
        assert_eq!(
            resolved.source,
            PackageSource::Registry {
                backend: "sysand".to_string(),
                package: "units-range".to_string(),
                requested: "^1.0".to_string(),
                version: "1.8.1".to_string(),
            }
        );
        clean_registry_cache_for_request("sysand", "units-range", "^1.0");
    }

    #[test]
    fn registry_provider_reports_unknown_backend() {
        let tmp = TempDir::new().unwrap();
        let manifest_path = tmp.path().join("sysml.toml");
        fs::write(
            &manifest_path,
            r#"
[project]
name = "root"
version = "0.1.0"

[dependencies]
units-custom = { version = "1.0.0", registry = "custom" }
"#,
        )
        .unwrap();
        let manifest = load_manifest(&manifest_path).unwrap();
        let dep = manifest.dependencies.get("units-custom").unwrap();

        let provider = RegistryProvider::default();
        let err = provider
            .resolve("units-custom", dep, tmp.path())
            .unwrap_err();
        assert!(matches!(
            err,
            ResolveError::UnsupportedSource { dep_type, .. } if dep_type == "registry-backend-custom"
        ));
    }

    #[test]
    fn unknown_dependency_source_returns_unknown_error() {
        let registry = SourceProviderRegistry::new();
        let tmp = TempDir::new().unwrap();
        let manifest_path = tmp.path().join("sysml.toml");
        fs::write(
            &manifest_path,
            r#"
[project]
name = "root"
version = "0.1.0"

[dependencies]
mystery = { tag = "v1.0.0" }
"#,
        )
        .unwrap();
        let manifest = load_manifest(&manifest_path).unwrap();
        let dep = manifest.dependencies.get("mystery").unwrap();

        let result = registry.resolve("mystery", dep, Path::new("."));
        assert!(matches!(
            result,
            Err(ResolveError::UnsupportedSource { dep_type, .. }) if dep_type == "unknown"
        ));
    }

    #[test]
    fn kpar_provider_resolves_local_path_and_file_url() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let archive_path = create_kpar_archive(root, "archive-lib", "1.2.3");
        clean_kpar_cache_for_path(&archive_path);

        let provider = KparProvider;

        let path_dep = Dependency::kpar("./archive-lib.kpar");
        let path_resolved = provider.resolve("archive-lib", &path_dep, root).unwrap();
        assert_eq!(path_resolved.manifest.project.name, "archive-lib");
        assert_eq!(path_resolved.manifest.project.version, "1.2.3");
        assert_eq!(
            path_resolved.source,
            PackageSource::Kpar {
                url: "./archive-lib.kpar".to_string(),
            }
        );
        assert!(path_resolved.source_dir.join("sysml.toml").exists());
        assert!(path_resolved
            .source_dir
            .join(KPAR_CHECKSUM_FILENAME)
            .exists());

        let file_url = format!("file://{}", archive_path.canonicalize().unwrap().display());
        let url_dep = Dependency::kpar(&file_url);
        let url_resolved = provider.resolve("archive-lib", &url_dep, root).unwrap();
        assert_eq!(url_resolved.source, PackageSource::Kpar { url: file_url });

        clean_kpar_cache_for_path(&archive_path);
    }

    #[test]
    fn kpar_provider_resolves_http_url() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let archive_path = create_kpar_archive(root, "archive-lib", "1.2.3");
        let bytes = fs::read(&archive_path).unwrap();
        let (url, join_handle) = serve_bytes_n(bytes, 1);

        let source_key = url.clone();
        clean_kpar_cache_for_source_key(&source_key);

        let provider = KparProvider;
        let dep = Dependency::kpar(&url);
        let resolved = provider.resolve("archive-lib", &dep, root).unwrap();
        assert_eq!(resolved.source, PackageSource::Kpar { url: url.clone() });
        assert_eq!(resolved.manifest.project.name, "archive-lib");
        assert!(resolved.source_dir.join("sysml.toml").exists());
        assert!(resolved.source_dir.join(KPAR_CHECKSUM_FILENAME).exists());

        join_handle.join().unwrap();
        clean_kpar_cache_for_source_key(&source_key);
    }

    #[test]
    fn kpar_manifest_build_includes_kpar_usage_dependencies() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let leaf = create_kpar_archive(root, "leaf-lib", "0.2.0");
        let leaf_url = format!("file://{}", leaf.canonicalize().unwrap().display());
        let archive_path =
            create_kpar_archive_with_usage(root, "archive-lib", "1.0.0", vec![leaf_url.clone()]);
        clean_kpar_cache_for_path(&archive_path);
        clean_kpar_cache_for_path(&leaf);

        let dep = Dependency::kpar("./archive-lib.kpar");
        let provider = KparProvider;
        let resolved = provider.resolve("archive-lib", &dep, root).unwrap();
        let usage_dep = resolved
            .manifest
            .dependencies
            .values()
            .find(|d| d.is_kpar())
            .expect("expected at least one kpar dependency from usage");
        assert_eq!(usage_dep.as_kpar_url(), Some(leaf_url.as_str()));

        clean_kpar_cache_for_path(&archive_path);
        clean_kpar_cache_for_path(&leaf);
    }

    #[test]
    fn kpar_provider_recovers_from_corrupt_extracted_cache() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let archive_path = create_kpar_archive(root, "archive-lib", "0.3.0");
        clean_kpar_cache_for_path(&archive_path);

        let dep = Dependency::kpar("./archive-lib.kpar");
        let provider = KparProvider;

        let first = provider.resolve("archive-lib", &dep, root).unwrap();
        let checksum_file = first.source_dir.join(KPAR_CHECKSUM_FILENAME);
        assert!(checksum_file.exists());

        fs::remove_file(first.source_dir.join("sysml.toml")).unwrap();
        fs::remove_file(&checksum_file).unwrap();

        let second = provider.resolve("archive-lib", &dep, root).unwrap();
        assert!(second.source_dir.join("sysml.toml").exists());
        assert!(second.source_dir.join(KPAR_CHECKSUM_FILENAME).exists());

        clean_kpar_cache_for_path(&archive_path);
    }

    #[test]
    fn kpar_provider_reports_checksum_mismatch_for_corrupt_archive_cache() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let archive_path = create_kpar_archive(root, "archive-lib", "0.4.0");
        clean_kpar_cache_for_path(&archive_path);

        let dep = Dependency::kpar("./archive-lib.kpar");
        let provider = KparProvider;

        let first = provider.resolve("archive-lib", &dep, root).unwrap();
        assert!(first.source_dir.join("sysml.toml").exists());

        let canonical = archive_path.canonicalize().unwrap();
        let source_key = canonical_file_source(&canonical);
        let expected_hex = sha256_hex_file(&canonical).unwrap();
        let cached_archive = kpar_cache_dir_for_source(&source_key)
            .join("archives")
            .join(format!("{expected_hex}.kpar"));
        assert!(cached_archive.exists());

        fs::write(&cached_archive, b"corrupt archive bytes").unwrap();

        let err = provider.resolve("archive-lib", &dep, root).unwrap_err();
        assert!(matches!(&err, ResolveError::ChecksumMismatch { .. }));
        let msg = err.to_string();
        assert!(msg.contains("checksum mismatch"));
        assert!(msg.contains("remove cache"));

        clean_kpar_cache_for_path(&archive_path);
    }

    #[test]
    fn git_provider_resolves_rev_to_commit() {
        if !git_available() {
            eprintln!("skipping git test: git binary unavailable");
            return;
        }

        let fixture = create_git_fixture();
        clean_git_cache(&fixture.url);

        let provider = GitProvider;
        let dep = Dependency::git_rev(&fixture.url, &fixture.main_commit);
        let resolved = provider.resolve("remote", &dep, Path::new(".")).unwrap();

        assert_eq!(resolved.manifest.project.name, "remote-lib");
        assert_eq!(
            resolved.source,
            PackageSource::Git {
                url: fixture.url.clone(),
                commit: fixture.main_commit.clone(),
            }
        );
        assert!(resolved.source_dir.join("sysml.toml").exists());

        clean_git_cache(&fixture.url);
    }

    #[test]
    fn git_provider_resolves_tag_and_branch() {
        if !git_available() {
            eprintln!("skipping git test: git binary unavailable");
            return;
        }

        let fixture = create_git_fixture();
        clean_git_cache(&fixture.url);

        git(&fixture.repo_dir, &["tag", "v1.0.0"]);
        git(&fixture.repo_dir, &["checkout", "-b", "release"]);
        fs::write(
            fixture.repo_dir.join("sysml.toml"),
            "[project]\nname = \"remote-lib\"\nversion = \"1.1.0\"\n",
        )
        .unwrap();
        git(&fixture.repo_dir, &["add", "sysml.toml"]);
        git(&fixture.repo_dir, &["commit", "-m", "release"]);
        let release_commit = git(&fixture.repo_dir, &["rev-parse", "HEAD"]);

        let provider = GitProvider;

        let tag_dep = Dependency::git_tag(&fixture.url, "v1.0.0");
        let tag_resolved = provider
            .resolve("remote", &tag_dep, Path::new("."))
            .unwrap();
        assert_eq!(
            tag_resolved.source,
            PackageSource::Git {
                url: fixture.url.clone(),
                commit: fixture.main_commit.clone(),
            }
        );

        let branch_dep = Dependency::git_branch(&fixture.url, "release");
        let branch_resolved = provider
            .resolve("remote", &branch_dep, Path::new("."))
            .unwrap();
        assert_eq!(
            branch_resolved.source,
            PackageSource::Git {
                url: fixture.url.clone(),
                commit: release_commit,
            }
        );

        clean_git_cache(&fixture.url);
    }

    #[test]
    fn git_provider_recovers_from_corrupt_cache_material() {
        if !git_available() {
            eprintln!("skipping git test: git binary unavailable");
            return;
        }

        let fixture = create_git_fixture();
        clean_git_cache(&fixture.url);

        let provider = GitProvider;
        let dep = Dependency::git_rev(&fixture.url, &fixture.main_commit);

        let first = provider.resolve("remote", &dep, Path::new(".")).unwrap();
        assert!(first.source_dir.join("sysml.toml").exists());

        let cache_dir = git_cache_dir_for_url(&fixture.url);
        let objects_dir = cache_dir.join("objects");
        fs::remove_dir_all(&objects_dir).unwrap();
        fs::write(&objects_dir, "corrupt").unwrap();

        let second = provider.resolve("remote", &dep, Path::new(".")).unwrap();
        assert!(second.source_dir.join("sysml.toml").exists());
        assert!(objects_dir.is_dir());

        let checkout_manifest = second.source_dir.join("sysml.toml");
        fs::remove_file(&checkout_manifest).unwrap();

        let third = provider.resolve("remote", &dep, Path::new(".")).unwrap();
        assert!(third.source_dir.join("sysml.toml").exists());

        clean_git_cache(&fixture.url);
    }

    struct GitFixture {
        _tmp: TempDir,
        repo_dir: PathBuf,
        url: String,
        main_commit: String,
    }

    fn create_git_fixture() -> GitFixture {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("remote-lib");
        fs::create_dir_all(&repo_dir).unwrap();

        git(&repo_dir, &["init", "--initial-branch", "main"]);
        git(
            &repo_dir,
            &["config", "user.email", "sysml-tests@example.com"],
        );
        git(&repo_dir, &["config", "user.name", "SysML Tests"]);

        fs::write(
            repo_dir.join("sysml.toml"),
            "[project]\nname = \"remote-lib\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        git(&repo_dir, &["add", "sysml.toml"]);
        git(&repo_dir, &["commit", "-m", "initial"]);
        let main_commit = git(&repo_dir, &["rev-parse", "HEAD"]);

        let canonical = repo_dir.canonicalize().unwrap();
        let url = format!("file://{}", canonical.display());

        GitFixture {
            _tmp: tmp,
            repo_dir,
            url,
            main_commit,
        }
    }

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    fn git(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {}: {e}", args.join(" ")));

        assert!(
            output.status.success(),
            "git {} failed in {}\nstdout: {}\nstderr: {}",
            args.join(" "),
            cwd.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn create_kpar_archive(root: &Path, name: &str, version: &str) -> PathBuf {
        create_kpar_archive_with_usage(root, name, version, Vec::new())
    }

    fn create_kpar_archive_with_usage(
        root: &Path,
        name: &str,
        version: &str,
        usage_resources: Vec<String>,
    ) -> PathBuf {
        let archive_path = root.join(format!("{name}.kpar"));
        let mut metadata = ProjectMetadata::new();
        metadata.created = Some("2026-03-01T00:00:00Z".to_string());
        metadata.add_index_entry("Root", "Root.sysml");

        let archive = KparArchive {
            root_dir: name.to_string(),
            project_info: ProjectInfo {
                name: name.to_string(),
                version: version.to_string(),
                description: Some("Fixture archive".to_string()),
                license: Some("MIT".to_string()),
                usage: usage_resources
                    .into_iter()
                    .map(|resource| sysml_project::kpar::UsageEntry {
                        resource,
                        version_constraint: None,
                    })
                    .collect(),
            },
            metadata,
            source_files: vec![(
                "Root.sysml".to_string(),
                b"package Root {\n  part def Sensor;\n}\n".to_vec(),
            )],
        };

        write_kpar(&archive_path, &archive).unwrap();
        archive_path
    }

    fn clean_kpar_cache_for_path(archive_path: &Path) {
        let canonical = archive_path.canonicalize().unwrap();
        let source_key = canonical_file_source(&canonical);
        clean_kpar_cache_for_source_key(&source_key);
    }

    fn clean_kpar_cache_for_source_key(source_key: &str) {
        let cache_dir = kpar_cache_dir_for_source(&source_key);
        let _ = fs::remove_dir_all(cache_dir);
    }

    fn write_sysand_index_for_package(
        root: &Path,
        package: &str,
        version: &str,
        artifact_path: &Path,
    ) {
        let index_dir = root.join(".sysml/registries/sysand");
        fs::create_dir_all(&index_dir).unwrap();
        let checksum = format!("sha256:{}", sha256_hex_file(artifact_path).unwrap());
        fs::write(
            index_dir.join("index.json"),
            format!(
                "{{\"packages\":{{\"{package}\":{{\"{version}\":{{\"artifact\":\"{}\",\"checksum\":\"{checksum}\"}}}}}}}}",
                artifact_path.display()
            ),
        )
        .unwrap();
    }

    fn write_sysand_index_with_releases(root: &Path, package: &str, releases: &[(&str, &Path)]) {
        let index_dir = root.join(".sysml/registries/sysand");
        fs::create_dir_all(&index_dir).unwrap();

        let entries = releases
            .iter()
            .map(|(version, artifact_path)| {
                let checksum = format!("sha256:{}", sha256_hex_file(artifact_path).unwrap());
                format!(
                    "\"{version}\":{{\"artifact\":\"{}\",\"checksum\":\"{checksum}\"}}",
                    artifact_path.display()
                )
            })
            .collect::<Vec<_>>()
            .join(",");

        fs::write(
            index_dir.join("index.json"),
            format!("{{\"packages\":{{\"{package}\":{{{entries}}}}}}}"),
        )
        .unwrap();
    }

    fn serve_bytes_n(bytes: Vec<u8>, requests: usize) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(&bytes).unwrap();
            }
        });

        (format!("http://{addr}/archive-lib.kpar"), handle)
    }

    fn clean_git_cache(url: &str) {
        let cache_dir = git_cache_dir_for_url(url);
        let _ = fs::remove_dir_all(cache_dir);
    }

    fn clean_registry_cache_for_request(backend: &str, package: &str, requirement: &str) {
        let cache_dir = registry_cache_dir_for_request(backend, package, requirement);
        let _ = fs::remove_dir_all(cache_dir);
    }
}
