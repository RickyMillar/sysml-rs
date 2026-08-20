use std::collections::HashMap;

use crate::info::ProjectInfo;
use crate::meta::{ProjectMeta, SymbolIndex};

/// A standard library project definition with embedded manifests.
struct StdlibDef {
    /// Short name used in `urn:kpar:<name>` IRIs.
    urn_name: &'static str,
    /// HTTPS IRI form.
    https_iri: &'static str,
    /// Embedded `.project.json` content.
    project_json: &'static str,
    /// Embedded `.meta.json` content.
    meta_json: &'static str,
}

/// The 10 standard library project definitions.
static STDLIB_DEFS: &[StdlibDef] = &[
    // Kernel libraries (3)
    StdlibDef {
        urn_name: "semantic-library",
        https_iri: "https://www.omg.org/spec/KerML/20250201/Kernel-Semantic-Library",
        project_json: include_str!("stdlib_assets/semantic-library.project.json"),
        meta_json: include_str!("stdlib_assets/semantic-library.meta.json"),
    },
    StdlibDef {
        urn_name: "data-type-library",
        https_iri: "https://www.omg.org/spec/KerML/20250201/Kernel-Data-Type-Library",
        project_json: include_str!("stdlib_assets/data-type-library.project.json"),
        meta_json: include_str!("stdlib_assets/data-type-library.meta.json"),
    },
    StdlibDef {
        urn_name: "function-library",
        https_iri: "https://www.omg.org/spec/KerML/20250201/Kernel-Function-Library",
        project_json: include_str!("stdlib_assets/function-library.project.json"),
        meta_json: include_str!("stdlib_assets/function-library.meta.json"),
    },
    // SysML domain libraries (7)
    StdlibDef {
        urn_name: "systems-library",
        https_iri: "https://www.omg.org/spec/SysML/20250201/SysML-Systems-Library",
        project_json: include_str!("stdlib_assets/systems-library.project.json"),
        meta_json: include_str!("stdlib_assets/systems-library.meta.json"),
    },
    StdlibDef {
        urn_name: "analysis-library",
        https_iri: "https://www.omg.org/spec/SysML/20250201/SysML-Analysis-Library",
        project_json: include_str!("stdlib_assets/analysis-library.project.json"),
        meta_json: include_str!("stdlib_assets/analysis-library.meta.json"),
    },
    StdlibDef {
        urn_name: "cause-and-effect-library",
        https_iri: "https://www.omg.org/spec/SysML/20250201/SysML-Cause-and-Effect-Library",
        project_json: include_str!("stdlib_assets/cause-and-effect-library.project.json"),
        meta_json: include_str!("stdlib_assets/cause-and-effect-library.meta.json"),
    },
    StdlibDef {
        urn_name: "geometry-library",
        https_iri: "https://www.omg.org/spec/SysML/20250201/SysML-Geometry-Library",
        project_json: include_str!("stdlib_assets/geometry-library.project.json"),
        meta_json: include_str!("stdlib_assets/geometry-library.meta.json"),
    },
    StdlibDef {
        urn_name: "metadata-library",
        https_iri: "https://www.omg.org/spec/SysML/20250201/SysML-Metadata-Library",
        project_json: include_str!("stdlib_assets/metadata-library.project.json"),
        meta_json: include_str!("stdlib_assets/metadata-library.meta.json"),
    },
    StdlibDef {
        urn_name: "quantities-and-units-library",
        https_iri: "https://www.omg.org/spec/SysML/20250201/SysML-Quantities-and-Units-Library",
        project_json: include_str!("stdlib_assets/quantities-and-units-library.project.json"),
        meta_json: include_str!("stdlib_assets/quantities-and-units-library.meta.json"),
    },
    StdlibDef {
        urn_name: "requirement-derivation-library",
        https_iri: "https://www.omg.org/spec/SysML/20250201/SysML-Requirement-Derivation-Library",
        project_json: include_str!("stdlib_assets/requirement-derivation-library.project.json"),
        meta_json: include_str!("stdlib_assets/requirement-derivation-library.meta.json"),
    },
];

/// URN names of the 3 KerML kernel libraries that have circular dependencies.
pub const KERNEL_PROJECT_URNS: &[&str] =
    &["semantic-library", "data-type-library", "function-library"];

/// Parsed entry in the stdlib registry.
#[derive(Debug, Clone)]
pub struct StdlibProject {
    /// Short URN name (e.g. `"systems-library"`).
    pub urn_name: String,
    /// Parsed project manifest.
    pub info: ProjectInfo,
    /// Parsed project metadata.
    pub meta: ProjectMeta,
}

/// Registry of the 10 SysML v2 standard library projects.
///
/// Provides lookup by `urn:kpar:<name>` or HTTPS IRI, and builds
/// a combined [`SymbolIndex`] across all stdlib projects.
pub struct StdlibRegistry {
    projects: Vec<StdlibProject>,
    by_urn: HashMap<String, usize>,
    by_https: HashMap<String, usize>,
    by_name: HashMap<String, usize>,
}

impl StdlibRegistry {
    /// Create a new registry, parsing all embedded manifests.
    pub fn new() -> crate::Result<Self> {
        let mut projects = Vec::with_capacity(STDLIB_DEFS.len());
        let mut by_urn = HashMap::new();
        let mut by_https = HashMap::new();
        let mut by_name = HashMap::new();

        for (i, def) in STDLIB_DEFS.iter().enumerate() {
            let info: ProjectInfo = serde_json::from_str(def.project_json)?;
            let meta: ProjectMeta = serde_json::from_str(def.meta_json)?;

            let urn_iri = format!("urn:kpar:{}", def.urn_name);
            by_urn.insert(urn_iri, i);
            by_https.insert(def.https_iri.to_owned(), i);
            by_name.insert(info.name.clone(), i);

            projects.push(StdlibProject {
                urn_name: def.urn_name.to_owned(),
                info,
                meta,
            });
        }

        Ok(Self {
            projects,
            by_urn,
            by_https,
            by_name,
        })
    }

    /// Look up a project by its full `urn:kpar:` IRI.
    #[allow(clippy::indexing_slicing)] // Indices are populated by new(); always valid
    pub fn get_by_iri(&self, iri: &str) -> Option<&StdlibProject> {
        self.by_urn
            .get(iri)
            .or_else(|| self.by_https.get(iri))
            .map(|&i| &self.projects[i])
    }

    /// Look up a project by its human-readable name.
    #[allow(clippy::indexing_slicing)] // Indices are populated by new(); always valid
    pub fn get_by_name(&self, name: &str) -> Option<&StdlibProject> {
        self.by_name.get(name).map(|&i| &self.projects[i])
    }

    /// Get the 3 kernel library projects.
    pub fn kernel_projects(&self) -> Vec<&StdlibProject> {
        KERNEL_PROJECT_URNS
            .iter()
            .filter_map(|urn| {
                let full = format!("urn:kpar:{urn}");
                self.get_by_iri(&full)
            })
            .collect()
    }

    /// Build a combined [`SymbolIndex`] from all stdlib projects.
    pub fn symbol_index(&self) -> SymbolIndex {
        let mut index = SymbolIndex::new();
        for proj in &self.projects {
            let proj_index = proj.meta.symbol_index(&proj.info.name);
            index.merge(&proj_index);
        }
        index
    }

    /// Iterate over all stdlib projects.
    pub fn iter(&self) -> impl Iterator<Item = &StdlibProject> {
        self.projects.iter()
    }

    /// Number of stdlib projects (always 10).
    pub fn len(&self) -> usize {
        self.projects.len()
    }

    /// Always false (there are always 10 stdlib projects).
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn all_projects_parse() {
        let registry = StdlibRegistry::new().unwrap();
        assert_eq!(registry.len(), 10);
    }

    #[test]
    fn urn_lookup() {
        let registry = StdlibRegistry::new().unwrap();
        let proj = registry.get_by_iri("urn:kpar:systems-library").unwrap();
        assert_eq!(proj.info.name, "SysML Systems Library");
    }

    #[test]
    fn https_lookup() {
        let registry = StdlibRegistry::new().unwrap();
        let proj = registry
            .get_by_iri("https://www.omg.org/spec/SysML/20250201/SysML-Systems-Library")
            .unwrap();
        assert_eq!(proj.info.name, "SysML Systems Library");
    }

    #[test]
    fn name_lookup() {
        let registry = StdlibRegistry::new().unwrap();
        let proj = registry.get_by_name("Kernel Semantic Library").unwrap();
        assert_eq!(proj.urn_name, "semantic-library");
    }

    #[test]
    fn kernel_projects_count() {
        let registry = StdlibRegistry::new().unwrap();
        assert_eq!(registry.kernel_projects().len(), 3);
    }

    #[test]
    fn symbol_index_has_expected_symbols() {
        let registry = StdlibRegistry::new().unwrap();
        let index = registry.symbol_index();

        // systems library symbols
        assert!(!index.lookup("Parts").is_empty());
        assert!(!index.lookup("Actions").is_empty());
        assert!(!index.lookup("States").is_empty());

        // semantic library symbols
        assert!(!index.lookup("Base").is_empty());
        assert!(!index.lookup("Occurrences").is_empty());

        // data type library symbols
        assert!(!index.lookup("ScalarValues").is_empty());

        // function library symbols
        assert!(!index.lookup("BaseFunctions").is_empty());

        // Should have many entries total
        assert!(
            index.len() > 50,
            "expected >50 symbols, got {}",
            index.len()
        );
    }

    #[test]
    fn no_stdlib_conflicts() {
        let registry = StdlibRegistry::new().unwrap();
        let index = registry.symbol_index();
        let conflicts = index.conflicts();
        assert!(conflicts.is_empty(), "unexpected conflicts: {conflicts:?}");
    }
}
