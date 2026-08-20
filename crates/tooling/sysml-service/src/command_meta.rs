//! Command metadata registry — the always-up-to-date spec.
//!
//! Every public service operation is cataloged here with its name, description,
//! parameters, and return type. This registry is the single source of truth
//! consumed by MCP tool definitions, REST API OpenAPI specs, and the
//! machine-readable `commands.json`.

use serde::Serialize;

/// Metadata describing a single service command.
#[derive(Debug, Clone, Serialize)]
pub struct CommandMeta {
    /// Dot-separated command name (e.g. "sysml.find", "sysml.simulate.start").
    pub name: &'static str,
    /// Command category for grouping.
    pub category: CommandCategory,
    /// Human-readable description of what the command does.
    pub description: &'static str,
    /// Input parameters.
    pub params: &'static [ParamMeta],
    /// Description of the return type.
    pub returns: &'static str,
    /// Whether this command manages session state.
    pub stateful: bool,
    /// Whether this command is superseded by another and should be hidden
    /// from user-facing command listings (the Cmd-K palette). It remains
    /// registered and dispatchable, so existing callers keep working.
    ///
    /// This exists so a deprecation note never has to be written into
    /// `description` — the palette renders descriptions verbatim, so
    /// "[Deprecated: prefer sessions.create]" was leaking an internal
    /// migration note into the product (punch-list finding 31).
    pub deprecated: bool,
}

/// Parameter metadata for a command.
#[derive(Debug, Clone, Serialize)]
pub struct ParamMeta {
    /// Parameter name.
    pub name: &'static str,
    /// Type description (e.g. "string", "ElementKind?", "[(String, String)]").
    pub ty: &'static str,
    /// Whether the parameter is required.
    pub required: bool,
    /// Human-readable description.
    pub description: &'static str,
}

/// Command categories matching the MCP tier structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CommandCategory {
    /// File loading and workspace management.
    FileManagement,
    /// Stateless model queries (Tier 1).
    Query,
    /// Cached analysis operations (Tier 2).
    Analysis,
    /// Session-based execution (Tier 3).
    Execution,
    /// Diagram and export operations (Tier 4).
    Visualization,
    /// Persistence operations (Tier 5).
    Storage,
}

/// Returns all commands from the inventory-based registry.
///
/// This is the canonical source — all commands are auto-registered via
/// `#[service_command]` annotations processed by `#[service_impl]`.
pub fn command_registry_all() -> Vec<&'static CommandMeta> {
    crate::command_trait::registered_command_metas()
}

/// Total number of registered commands.
pub fn command_count() -> usize {
    command_registry_all().len()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_not_empty() {
        assert!(command_count() >= 48, "expected at least 48 commands, got {}", command_count());
    }

    #[test]
    fn test_unique_names() {
        let registry = command_registry_all();
        let mut names: Vec<&str> = registry.iter().map(|c| c.name).collect();
        let original_len = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), original_len, "duplicate command names found");
    }

    #[test]
    fn test_all_have_descriptions() {
        for cmd in command_registry_all() {
            assert!(!cmd.description.is_empty(), "command {} has empty description", cmd.name);
            for param in cmd.params {
                assert!(!param.description.is_empty(), "param {}.{} has empty description", cmd.name, param.name);
            }
        }
    }

    /// A deprecation is metadata, not prose. The Cmd-K palette renders
    /// `description` verbatim to end users, so a note like
    /// "[Deprecated: prefer sessions.create]" shipped an internal migration
    /// instruction into the product (punch-list finding 31). Mark the command
    /// `deprecated = true` instead — listings filter on the flag.
    #[test]
    fn test_no_deprecation_notes_in_descriptions() {
        for cmd in command_registry_all() {
            let lowered = cmd.description.to_lowercase();
            assert!(
                !lowered.contains("deprecat"),
                "command {} writes its deprecation into the user-visible description \
                 ({:?}); use `deprecated = true` on #[service_command] instead",
                cmd.name,
                cmd.description
            );
        }
    }

    /// The flag has to actually reach the serialized metadata, since that JSON
    /// is what the palette filters on.
    #[test]
    fn test_deprecated_flag_reaches_metadata() {
        let orchestrate_start = command_registry_all()
            .into_iter()
            .find(|c| c.name == "sysml.orchestrate.start")
            .expect("sysml.orchestrate.start is registered");
        assert!(
            orchestrate_start.deprecated,
            "orchestrate.start forwards to orchestrate.workspace.start and is superseded \
             by sessions.create — it must be flagged so listings can hide it"
        );

        let json = serde_json::to_value(orchestrate_start).unwrap();
        assert_eq!(json.get("deprecated").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_serializes_to_json() {
        let json = serde_json::to_value(command_registry_all()).unwrap();
        assert!(json.is_array());
        let arr = json.as_array().unwrap();
        assert!(arr.len() >= 48);

        // Verify structure of first entry
        let first = &arr[0];
        assert!(first.get("name").is_some());
        assert!(first.get("description").is_some());
        assert!(first.get("category").is_some());
        assert!(first.get("params").is_some());
        assert!(first.get("returns").is_some());
    }

    #[test]
    fn test_execute_command_dispatch() {
        let service = crate::SysmlService::empty();
        service
            .load_source("test.sysml", "package P { part x; }")
            .unwrap();
        let result = crate::execute_command(
            &service,
            "sysml.loaded_uris",
            serde_json::json!({}),
        );
        assert!(result.is_ok(), "execute_command should succeed for sysml.loaded_uris");
        let uris = result.unwrap();
        let uris_array = uris.as_array().expect("loaded_uris should return an array");
        assert!(
            uris_array.iter().any(|v| v.as_str() == Some("test.sysml")),
            "loaded_uris should contain 'test.sysml', got: {uris:?}"
        );
    }
}
