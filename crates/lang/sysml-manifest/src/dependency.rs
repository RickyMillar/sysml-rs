//! Dependency specification types.

use serde::{Deserialize, Serialize};

/// A dependency specification from `[dependencies]` in `sysml.toml`.
///
/// Supports multiple source types following Cargo conventions:
/// ```toml
/// [dependencies]
/// local-lib = { path = "../local-lib" }
/// remote-lib = { git = "https://github.com/org/repo", tag = "v1.0.0" }
/// archive-lib = { kpar = "https://example.com/lib.kpar" }
/// registry-lib = "1.0"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    /// Short form: `dep = "1.0"` (registry version constraint).
    Registry(String),
    /// Table form with explicit source.
    Detailed(DetailedDependency),
}

/// Detailed dependency with explicit source specification.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetailedDependency {
    /// Local path dependency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Git repository URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,

    /// Git tag reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,

    /// Git branch reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Git revision (commit hash).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,

    /// Direct KPAR archive URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kpar: Option<String>,

    /// Version constraint (for registry or git deps).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Optional registry backend identifier (registry deps only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

/// Resolved git reference type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRef {
    Tag(String),
    Branch(String),
    Rev(String),
    /// Default branch (no ref specified).
    DefaultBranch,
}

impl Dependency {
    /// Create a path dependency.
    pub fn path(path: impl Into<String>) -> Self {
        Dependency::Detailed(DetailedDependency {
            path: Some(path.into()),
            ..DetailedDependency::default()
        })
    }

    /// Create a git dependency with a tag.
    pub fn git_tag(url: impl Into<String>, tag: impl Into<String>) -> Self {
        Dependency::Detailed(DetailedDependency {
            git: Some(url.into()),
            tag: Some(tag.into()),
            ..DetailedDependency::default()
        })
    }

    /// Create a git dependency with a branch.
    pub fn git_branch(url: impl Into<String>, branch: impl Into<String>) -> Self {
        Dependency::Detailed(DetailedDependency {
            git: Some(url.into()),
            branch: Some(branch.into()),
            ..DetailedDependency::default()
        })
    }

    /// Create a git dependency with a specific revision.
    pub fn git_rev(url: impl Into<String>, rev: impl Into<String>) -> Self {
        Dependency::Detailed(DetailedDependency {
            git: Some(url.into()),
            rev: Some(rev.into()),
            ..DetailedDependency::default()
        })
    }

    /// Create a KPAR URL dependency.
    pub fn kpar(url: impl Into<String>) -> Self {
        Dependency::Detailed(DetailedDependency {
            kpar: Some(url.into()),
            ..DetailedDependency::default()
        })
    }

    /// Create a registry dependency with a version constraint.
    pub fn registry(version: impl Into<String>) -> Self {
        Dependency::Registry(version.into())
    }

    /// Returns true if this is a path dependency.
    pub fn is_path(&self) -> bool {
        matches!(self, Dependency::Detailed(d) if d.path.is_some())
    }

    /// Returns true if this is a git dependency.
    pub fn is_git(&self) -> bool {
        matches!(self, Dependency::Detailed(d) if d.git.is_some())
    }

    /// Returns true if this is a KPAR URL dependency.
    pub fn is_kpar(&self) -> bool {
        matches!(self, Dependency::Detailed(d) if d.kpar.is_some())
    }

    /// Returns true if this is a registry dependency.
    pub fn is_registry(&self) -> bool {
        matches!(self, Dependency::Registry(_))
            || matches!(self, Dependency::Detailed(d) if d.version.is_some())
    }

    /// Get the path for a path dependency.
    pub fn as_path(&self) -> Option<&str> {
        match self {
            Dependency::Detailed(d) => d.path.as_deref(),
            _ => None,
        }
    }

    /// Get the git URL for a git dependency.
    pub fn as_git_url(&self) -> Option<&str> {
        match self {
            Dependency::Detailed(d) => d.git.as_deref(),
            _ => None,
        }
    }

    /// Get the resolved git reference for a git dependency.
    pub fn git_ref(&self) -> Option<GitRef> {
        match self {
            Dependency::Detailed(d) if d.git.is_some() => {
                if let Some(tag) = &d.tag {
                    Some(GitRef::Tag(tag.clone()))
                } else if let Some(branch) = &d.branch {
                    Some(GitRef::Branch(branch.clone()))
                } else if let Some(rev) = &d.rev {
                    Some(GitRef::Rev(rev.clone()))
                } else {
                    Some(GitRef::DefaultBranch)
                }
            }
            _ => None,
        }
    }

    /// Get the KPAR URL for a KPAR dependency.
    pub fn as_kpar_url(&self) -> Option<&str> {
        match self {
            Dependency::Detailed(d) => d.kpar.as_deref(),
            _ => None,
        }
    }

    /// Get registry version requirement for a registry dependency.
    pub fn as_registry_requirement(&self) -> Option<&str> {
        match self {
            Dependency::Registry(version) => Some(version.as_str()),
            Dependency::Detailed(d) => d.version.as_deref(),
        }
    }

    /// Get optional registry backend override for a detailed dependency.
    pub fn registry_backend(&self) -> Option<&str> {
        match self {
            Dependency::Detailed(d) => d.registry.as_deref(),
            _ => None,
        }
    }
}

