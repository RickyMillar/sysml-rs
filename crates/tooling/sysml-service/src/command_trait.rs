//! ServiceCommand trait and inventory-based command registration.
//!
//! The `#[service_command]` proc macro generates implementations of this trait
//! for each annotated service method. The `CommandRegistration` struct is
//! submitted to `inventory` for compile-time collection.

use crate::command_meta::CommandMeta;
use crate::ServiceError;
use crate::SysmlService;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Trait binding a command's metadata, wire types, and execution logic.
///
/// Implemented by the proc macro for each annotated method. Not intended
/// for manual implementation.
pub trait ServiceCommand: Send + Sync + 'static {
    /// Compile-time metadata (name, category, params, etc.).
    const META: CommandMeta;

    /// Deserialized request body (generated struct).
    type Request: DeserializeOwned + Send;

    /// Serializable response body.
    type Response: Serialize + Send;

    /// Execute the command against a service instance.
    fn execute(
        service: &SysmlService,
        req: Self::Request,
    ) -> Result<Self::Response, ServiceError>;
}

/// A registered command entry collected by `inventory`.
///
/// Each `#[service_command]` generates an `inventory::submit!` call
/// that adds one of these to the global registry.
pub struct CommandRegistration {
    /// Static reference to the command metadata.
    pub meta: &'static CommandMeta,
    /// Type-erased handler: deserialize JSON body, execute, serialize result.
    pub handler: fn(&SysmlService, serde_json::Value) -> Result<serde_json::Value, ServiceError>,
}

// Tell inventory how to collect these
inventory::collect!(CommandRegistration);

/// Iterate all registered commands (from `#[service_command]` annotations).
pub fn registered_commands() -> impl Iterator<Item = &'static CommandRegistration> {
    inventory::iter::<CommandRegistration>()
}

/// Get metadata for all registered commands.
pub fn registered_command_metas() -> Vec<&'static CommandMeta> {
    registered_commands().map(|r| r.meta).collect()
}

/// Execute a command by name with a JSON body.
pub fn execute_command(
    service: &SysmlService,
    command_name: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, ServiceError> {
    for reg in registered_commands() {
        if reg.meta.name == command_name {
            return (reg.handler)(service, body);
        }
    }
    Err(ServiceError::NotFound(format!(
        "command '{}' not found",
        command_name
    )))
}
