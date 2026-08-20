//! `sysml.toml` manifest parsing and serialization.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::dependency::Dependency;
use crate::error::ManifestError;
use crate::stdlib::StdlibConfig;

/// A parsed `sysml.toml` manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SysmlManifest {
    /// Project metadata (required).
    pub project: ProjectConfig,

    /// Package identity for publishing / KPAR generation (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<PackageConfig>,

    /// Standard library selection configuration (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdlib: Option<StdlibConfig>,

    /// Project dependencies.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, Dependency>,

    /// Workspace configuration (only in root manifest).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceConfig>,
}

/// Project metadata from the `[project]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Project name (required).
    pub name: String,

    /// Project version (required, semver).
    pub version: String,

    /// Human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// SPDX license identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    /// SysML spec edition year (e.g., "2025"). Determines stdlib version.
    #[serde(
        default = "default_edition",
        rename = "sysml-edition",
        skip_serializing_if = "is_default_edition"
    )]
    pub sysml_edition: String,

    /// List of authors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
}

fn default_edition() -> String {
    "2025".to_owned()
}

fn is_default_edition(s: &str) -> bool {
    s == "2025"
}

/// Package identity for publishing and KPAR generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageConfig {
    /// Canonical IRI for this package (used in `.project.json` `usage[]` entries).
    /// Auto-generated as `urn:sysml:<name>` if not specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
}

/// Workspace configuration from `[workspace]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Workspace member directories (relative to workspace root).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,

    /// Workspace paths excluded from member expansion.
    ///
    /// Semantics mirror Cargo: exclusion is applied after `members`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,

    /// Default members for root-level commands.
    #[serde(
        default,
        rename = "default-members",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub default_members: Vec<String>,

    /// Shared project config that members can inherit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<WorkspaceProjectConfig>,
}

/// Shared project configuration that workspace members can inherit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceProjectConfig {
    #[serde(
        default,
        rename = "sysml-edition",
        skip_serializing_if = "Option::is_none"
    )]
    pub sysml_edition: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl SysmlManifest {
    /// Create a minimal manifest with just a name and version.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        SysmlManifest {
            project: ProjectConfig {
                name: name.into(),
                version: version.into(),
                description: None,
                license: None,
                sysml_edition: default_edition(),
                authors: Vec::new(),
            },
            package: None,
            stdlib: None,
            dependencies: BTreeMap::new(),
            workspace: None,
        }
    }

    /// Get the effective IRI for this package.
    /// Returns the explicit IRI if set, otherwise auto-generates `urn:sysml:<name>`.
    pub fn effective_iri(&self) -> String {
        self.package
            .as_ref()
            .and_then(|p| p.iri.as_deref())
            .map(|s| s.to_owned())
            .unwrap_or_else(|| format!("urn:sysml:{}", self.project.name))
    }

    /// Get the effective stdlib config, defaulting to all stdlib libraries enabled.
    pub fn effective_stdlib(&self) -> StdlibConfig {
        self.stdlib.clone().unwrap_or_default()
    }

    /// Returns true if this manifest defines a workspace.
    pub fn is_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    /// Add a dependency.
    pub fn add_dependency(&mut self, name: impl Into<String>, dep: Dependency) {
        self.dependencies.insert(name.into(), dep);
    }

    /// Remove a dependency. Returns the removed dependency if it existed.
    pub fn remove_dependency(&mut self, name: &str) -> Option<Dependency> {
        self.dependencies.remove(name)
    }
}

/// Load and parse a `sysml.toml` manifest from the given path.
pub fn load_manifest(path: &Path) -> Result<SysmlManifest, ManifestError> {
    let content = std::fs::read_to_string(path).map_err(|e| ManifestError::io(path, e))?;
    parse_manifest(&content, path)
}

/// Parse a `sysml.toml` manifest from a string.
pub fn parse_manifest(content: &str, path: &Path) -> Result<SysmlManifest, ManifestError> {
    toml::from_str(content).map_err(|e| ManifestError::parse(path, e.message()))
}

/// Save a manifest to a `sysml.toml` file.
pub fn save_manifest(path: &Path, manifest: &SysmlManifest) -> Result<(), ManifestError> {
    let content =
        toml::to_string_pretty(manifest).map_err(|e| ManifestError::Serialize(e.to_string()))?;
    std::fs::write(path, content).map_err(|e| ManifestError::io(path, e))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_minimal_manifest() {
        let toml = r#"
[project]
name = "my-project"
version = "0.1.0"
"#;
        let manifest = parse_manifest(toml, Path::new("test.toml")).unwrap();
        assert_eq!(manifest.project.name, "my-project");
        assert_eq!(manifest.project.version, "0.1.0");
        assert_eq!(manifest.project.sysml_edition, "2025");
        assert!(manifest.dependencies.is_empty());
        assert!(manifest.stdlib.is_none());
        assert!(manifest.package.is_none());
        assert!(manifest.workspace.is_none());
    }

    #[test]
    fn parse_full_manifest() {
        let toml = r#"
[project]
name = "coffee-machine"
version = "0.1.0"
description = "Smart coffee machine system model"
license = "MIT"
sysml-edition = "2025"
authors = ["Alice <alice@example.com>"]

[package]
iri = "urn:acme:coffee-machine"

[stdlib]
include_only = ["systems", "analysis", "geometry"]
exclude = ["analysis"]

[dependencies]
beverage-types = { path = "../beverage-types" }
thermal-model = { git = "https://github.com/acme/thermal-model", tag = "v1.0.0" }
sensor-defs = { kpar = "https://example.com/sensors-v2.kpar" }
base-types = "1.0"
"#;
        let manifest = parse_manifest(toml, Path::new("test.toml")).unwrap();
        assert_eq!(manifest.project.name, "coffee-machine");
        assert_eq!(
            manifest.project.description.as_deref(),
            Some("Smart coffee machine system model")
        );
        assert_eq!(manifest.project.license.as_deref(), Some("MIT"));
        assert_eq!(manifest.project.authors, vec!["Alice <alice@example.com>"]);

        // Package
        let pkg = manifest.package.as_ref().unwrap();
        assert_eq!(pkg.iri.as_deref(), Some("urn:acme:coffee-machine"));

        // Stdlib
        let stdlib = manifest.stdlib.as_ref().unwrap();
        assert_eq!(
            stdlib.include_only,
            vec![
                "systems".to_string(),
                "analysis".to_string(),
                "geometry".to_string()
            ]
        );
        assert_eq!(stdlib.exclude, vec!["analysis".to_string()]);

        // Dependencies
        assert_eq!(manifest.dependencies.len(), 4);
        assert!(manifest.dependencies["beverage-types"].is_path());
        assert!(manifest.dependencies["thermal-model"].is_git());
        assert!(manifest.dependencies["sensor-defs"].is_kpar());
        assert!(manifest.dependencies["base-types"].is_registry());
    }

    #[test]
    fn parse_workspace_manifest() {
        let toml = r#"
[project]
name = "beverage-workspace"
version = "0.1.0"

[workspace]
members = ["beverage-types", "coffee-machine"]
exclude = ["legacy"]
default-members = ["coffee-machine"]

[workspace.project]
sysml-edition = "2025"
license = "MIT"
"#;
        let manifest = parse_manifest(toml, Path::new("test.toml")).unwrap();
        assert!(manifest.is_workspace());
        let ws = manifest.workspace.as_ref().unwrap();
        assert_eq!(ws.members, vec!["beverage-types", "coffee-machine"]);
        assert_eq!(ws.exclude, vec!["legacy"]);
        assert_eq!(ws.default_members, vec!["coffee-machine"]);
        let ws_project = ws.project.as_ref().unwrap();
        assert_eq!(ws_project.sysml_edition.as_deref(), Some("2025"));
        assert_eq!(ws_project.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn parse_manifest_rejects_legacy_stdlib_boolean_keys() {
        let toml = r#"
[project]
name = "legacy-stdlib"
version = "0.1.0"

[stdlib]
systems = true
"#;

        let err = parse_manifest(toml, Path::new("test.toml"))
            .expect_err("legacy stdlib boolean keys should be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("unknown field") && msg.contains("systems"),
            "expected unknown field parse failure, got: {msg}"
        );
    }

    #[test]
    fn round_trip_manifest() {
        let original = SysmlManifest {
            project: ProjectConfig {
                name: "test-project".to_string(),
                version: "1.0.0".to_string(),
                description: Some("A test project".to_string()),
                license: Some("MIT".to_string()),
                sysml_edition: "2025".to_string(),
                authors: vec!["Bob".to_string()],
            },
            package: Some(PackageConfig {
                iri: Some("urn:test:project".to_string()),
            }),
            stdlib: Some(StdlibConfig {
                include_only: vec!["systems".to_string(), "analysis".to_string()],
                exclude: vec!["analysis".to_string()],
            }),
            dependencies: {
                let mut deps = BTreeMap::new();
                deps.insert("local-dep".to_string(), Dependency::path("../local-dep"));
                deps.insert(
                    "git-dep".to_string(),
                    Dependency::git_tag("https://github.com/org/repo", "v1.0.0"),
                );
                deps
            },
            workspace: None,
        };

        let serialized = toml::to_string_pretty(&original).unwrap();
        let deserialized: SysmlManifest = toml::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn effective_iri_explicit() {
        let mut manifest = SysmlManifest::new("my-project", "0.1.0");
        manifest.package = Some(PackageConfig {
            iri: Some("urn:custom:iri".to_string()),
        });
        assert_eq!(manifest.effective_iri(), "urn:custom:iri");
    }

    #[test]
    fn effective_iri_auto_generated() {
        let manifest = SysmlManifest::new("my-project", "0.1.0");
        assert_eq!(manifest.effective_iri(), "urn:sysml:my-project");
    }

    #[test]
    fn add_remove_dependency() {
        let mut manifest = SysmlManifest::new("test", "0.1.0");
        manifest.add_dependency("foo", Dependency::path("../foo"));
        assert!(manifest.dependencies.contains_key("foo"));
        assert!(manifest.remove_dependency("foo").is_some());
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn dependency_type_detection() {
        let path_dep = Dependency::path("../lib");
        assert!(path_dep.is_path());
        assert!(!path_dep.is_git());
        assert_eq!(path_dep.as_path(), Some("../lib"));

        let git_dep = Dependency::git_tag("https://example.com/repo", "v1.0");
        assert!(git_dep.is_git());
        assert!(!git_dep.is_path());
        assert_eq!(git_dep.as_git_url(), Some("https://example.com/repo"));

        let kpar_dep = Dependency::kpar("https://example.com/lib.kpar");
        assert!(kpar_dep.is_kpar());
        assert_eq!(kpar_dep.as_kpar_url(), Some("https://example.com/lib.kpar"));

        let reg_dep = Dependency::registry("^1.0");
        assert!(reg_dep.is_registry());
    }
}
