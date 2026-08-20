//! Standard library configuration.
//!
//! The standard library is enabled by default. Projects can customize the
//! active set using `include_only` and `exclude` in `[stdlib]`.

use serde::{Deserialize, Serialize};

/// Configuration for selecting standard library packages.
///
/// ```toml
/// [stdlib]
/// include_only = ["systems", "analysis"]
/// exclude = ["analysis"]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct StdlibConfig {
    /// If non-empty, restrict enabled libraries to this set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_only: Vec<String>,

    /// Exclude these libraries from the final enabled set.
    ///
    /// Special values: `"all"` or `"*"` disable all standard libraries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

const STDLIB_LIBRARIES: &[&str] = &[
    "semantic",
    "data-type",
    "function",
    "systems",
    "analysis",
    "cause-and-effect",
    "geometry",
    "metadata",
    "quantities-and-units",
    "requirement-derivation",
];

impl StdlibConfig {
    /// Returns the canonical library names accepted in `include_only` / `exclude`.
    pub fn known_library_names() -> &'static [&'static str] {
        STDLIB_LIBRARIES
    }

    /// Returns true if any domain library is enabled.
    pub fn has_domain_libraries(&self) -> bool {
        self.enabled_libraries().iter().any(|name| {
            matches!(
                *name,
                "analysis"
                    | "cause-and-effect"
                    | "geometry"
                    | "metadata"
                    | "quantities-and-units"
                    | "requirement-derivation"
            )
        })
    }

    /// Returns the list of enabled standard library names.
    ///
    /// Behavior:
    /// - default (`include_only = []`, `exclude = []`): all libraries enabled
    /// - `include_only`: allowlist
    /// - `exclude`: removal pass after allowlist
    /// - `exclude` contains `"all"` or `"*"`: disable all
    pub fn enabled_libraries(&self) -> Vec<&'static str> {
        let include_all = self.include_only.is_empty();
        let exclude_all = self
            .exclude
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case("all") || entry == "*");
        if exclude_all {
            return Vec::new();
        }

        let include_only: std::collections::HashSet<&str> =
            self.include_only.iter().map(String::as_str).collect();
        let exclude: std::collections::HashSet<&str> =
            self.exclude.iter().map(String::as_str).collect();

        let mut libs = Vec::new();
        for &name in STDLIB_LIBRARIES {
            if !include_all && !include_only.contains(name) {
                continue;
            }
            if exclude.contains(name) {
                continue;
            }
            libs.push(name);
        }
        libs
    }

    /// Returns the KPAR resource URL for a known standard library.
    pub fn library_kpar_url(name: &str) -> Option<&'static str> {
        match name {
            "semantic" => Some("https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar"),
            "data-type" => Some("https://www.omg.org/spec/KerML/20250201/Data-Type-Library.kpar"),
            "function" => Some("https://www.omg.org/spec/KerML/20250201/Function-Library.kpar"),
            "systems" => Some("https://www.omg.org/spec/SysML/20250201/Systems-Library.kpar"),
            "analysis" => {
                Some("https://www.omg.org/spec/SysML/20250201/Analysis-Domain-Library.kpar")
            }
            "geometry" => {
                Some("https://www.omg.org/spec/SysML/20250201/Geometry-Domain-Library.kpar")
            }
            "metadata" => {
                Some("https://www.omg.org/spec/SysML/20250201/Metadata-Domain-Library.kpar")
            }
            "quantities-and-units" => {
                Some("https://www.omg.org/spec/KerML/20250201/Quantities-and-Units-Library.kpar")
            }
            "cause-and-effect" => Some(
                "https://www.omg.org/spec/SysML/20250201/Cause-and-Effect-Domain-Library.kpar",
            ),
            "requirement-derivation" => Some(
                "https://www.omg.org/spec/SysML/20250201/Requirement-Derivation-Domain-Library.kpar",
            ),
            _ => None,
        }
    }

    /// Returns the KPAR version constraint used for a known standard library.
    pub fn library_version_constraint(name: &str) -> Option<&'static str> {
        match name {
            "semantic" | "data-type" | "function" | "systems" => Some("1.0.0"),
            "analysis"
            | "cause-and-effect"
            | "geometry"
            | "metadata"
            | "quantities-and-units"
            | "requirement-derivation" => Some("2.0.0"),
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::StdlibConfig;

    #[test]
    fn default_enables_all_stdlib() {
        let cfg = StdlibConfig::default();
        let enabled = cfg.enabled_libraries();
        assert_eq!(enabled.len(), StdlibConfig::known_library_names().len());
    }

    #[test]
    fn include_only_restricts_enabled_libraries() {
        let cfg = StdlibConfig {
            include_only: vec!["systems".to_string(), "analysis".to_string()],
            exclude: Vec::new(),
        };
        assert_eq!(cfg.enabled_libraries(), vec!["systems", "analysis"]);
    }

    #[test]
    fn exclude_filters_enabled_libraries() {
        let cfg = StdlibConfig {
            include_only: Vec::new(),
            exclude: vec!["analysis".to_string(), "geometry".to_string()],
        };
        let enabled = cfg.enabled_libraries();
        assert!(!enabled.contains(&"analysis"));
        assert!(!enabled.contains(&"geometry"));
        assert!(enabled.contains(&"systems"));
    }

    #[test]
    fn exclude_all_disables_everything() {
        let cfg = StdlibConfig {
            include_only: Vec::new(),
            exclude: vec!["all".to_string()],
        };
        assert!(cfg.enabled_libraries().is_empty());
    }
}
