use std::path::PathBuf;

use sysml_manifest::{find_manifest, SysmlManifest, MANIFEST_FILENAME};

/// Structured CLI exit codes.
///
/// - 0: success
/// - 1: user error (bad input, parse failure, missing element)
/// - 2: internal error (IO failure, unexpected state)
/// - 3: verification/constraint failure (model checked but failed)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// Bad input, parse failure, missing element.
    UserError = 1,
    /// IO failure, unexpected internal state.
    InternalError = 2,
    /// Constraint or verification check failed.
    VerificationFailure = 3,
}

/// CLI error carrying a message and structured exit code.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct CliError {
    pub message: String,
    pub exit_code: ExitCode,
}

impl CliError {
    pub fn user(msg: impl Into<String>) -> Self {
        CliError {
            message: msg.into(),
            exit_code: ExitCode::UserError,
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        CliError {
            message: msg.into(),
            exit_code: ExitCode::InternalError,
        }
    }

    pub fn verification(msg: impl Into<String>) -> Self {
        CliError {
            message: msg.into(),
            exit_code: ExitCode::VerificationFailure,
        }
    }
}

impl From<sysml_service::ServiceError> for CliError {
    fn from(e: sysml_service::ServiceError) -> Self {
        CliError::internal(e.to_string())
    }
}

/// Clap value parser for "key=value" pairs.
pub fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=VALUE: no '=' found in '{s}'"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

/// Discover and load the root manifest from the current working directory.
pub fn load_root_manifest_from_cwd() -> Result<(PathBuf, SysmlManifest, PathBuf), CliError> {
    let cwd = std::env::current_dir()
        .map_err(|e| CliError::internal(format!("failed to get current directory: {e}")))?;

    let (manifest_path, manifest) = find_manifest(&cwd)
        .map_err(|e| CliError::user(format!("failed to find manifest: {e}")))?
        .ok_or_else(|| {
            CliError::user(format!(
                "no {MANIFEST_FILENAME} found (searched from {})",
                cwd.display()
            ))
        })?;

    let manifest_dir = manifest_path
        .parent()
        .ok_or_else(|| CliError::internal("manifest has no parent directory"))?
        .to_path_buf();

    Ok((manifest_path, manifest, manifest_dir))
}

/// Parse a CLI string argument into a [`sysml_core::Value`].
///
/// Tries (in order): JSON object, integer, float, boolean, string fallback.
pub fn parse_cli_value(s: &str) -> sysml_core::Value {
    let trimmed = s.trim();

    // Try JSON object → Value::Map
    if trimmed.starts_with('{') {
        if let Ok(serde_json::Value::Object(obj)) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let mut map = std::collections::BTreeMap::new();
            for (k, v) in obj {
                map.insert(k, json_to_value(&v));
            }
            return sysml_core::Value::Map(map);
        }
    }

    // Try integer
    if let Ok(i) = trimmed.parse::<i64>() {
        return sysml_core::Value::Int(i);
    }
    // Try float
    if let Ok(f) = trimmed.parse::<f64>() {
        return sysml_core::Value::Float(f);
    }
    // Try boolean
    match trimmed {
        "true" => return sysml_core::Value::Bool(true),
        "false" => return sysml_core::Value::Bool(false),
        _ => {}
    }

    // Fallback: string
    sysml_core::Value::String(trimmed.to_owned())
}

/// Convert a [`serde_json::Value`] into a [`sysml_core::Value`].
fn json_to_value(v: &serde_json::Value) -> sysml_core::Value {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                sysml_core::Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                sysml_core::Value::Float(f)
            } else {
                sysml_core::Value::Null
            }
        }
        serde_json::Value::String(s) => sysml_core::Value::String(s.clone()),
        serde_json::Value::Bool(b) => sysml_core::Value::Bool(*b),
        serde_json::Value::Null => sysml_core::Value::Null,
        _ => sysml_core::Value::String(v.to_string()),
    }
}
