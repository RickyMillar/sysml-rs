//! External solver plugin -- wraps a subprocess that communicates via JSON over stdio.
//!
//! Protocol: the parent writes a JSON request to the child's stdin, then reads
//! a JSON response from stdout. The child process is spawned fresh for each
//! invocation (stateless).
//!
//! # JSON Protocol
//!
//! **Request** (written to stdin):
//! ```json
//! {
//!   "solver": "external:python:thermal",
//!   "params": [
//!     {"name": "heaterPower", "value": 1400.0, "direction": "in"},
//!     {"name": "ambientTemp", "value": 20.0, "direction": "in"}
//!   ]
//! }
//! ```
//!
//! **Response** (read from stdout):
//! ```json
//! {
//!   "status": "converged",
//!   "iterations": 5,
//!   "outputs": {"steadyStateTemp": 93.5, "heatLoss": 42.0},
//!   "diagnostics": []
//! }
//! ```

// External solver protocol uses indexing for parameter arrays and matrix operations.
#![allow(clippy::indexing_slicing)]

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use sysml_core::Value;

use crate::constraints::EvalContext;
use crate::solver_plugin::{
    ParamDirection, SolverCapabilities, SolverError, SolverParam, SolverPlugin, SolverResult,
};
use crate::ConstraintIR;

/// An external solver plugin that communicates via JSON over stdio.
///
/// Each call to [`solve`](SolverPlugin::solve) spawns a fresh subprocess,
/// writes the request JSON to its stdin, and reads the response JSON from
/// stdout. The subprocess is expected to exit after producing its output.
pub struct ExternalSolverPlugin {
    /// Plugin name (e.g., "external:python:thermal_solver").
    name: String,
    /// Command to execute (e.g., "python3", "/path/to/solver").
    command: String,
    /// Arguments to pass to the command.
    args: Vec<String>,
    /// Timeout for the subprocess (default: 30 seconds).
    timeout: Duration,
}

impl ExternalSolverPlugin {
    /// Create a new external solver plugin.
    ///
    /// # Arguments
    /// * `name` - Plugin name matching `ToolExecution.toolName` (conventionally prefixed with `"external:"`).
    /// * `command` - Path or name of the executable to spawn.
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            timeout: Duration::from_secs(30),
        }
    }

    /// Set the command-line arguments passed to the subprocess.
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Set the subprocess timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Serialize a `Value` to a JSON string fragment.
    fn value_to_json(value: &Value) -> String {
        match value {
            Value::Bool(b) => {
                if *b {
                    "true".to_owned()
                } else {
                    "false".to_owned()
                }
            }
            Value::Int(i) => i.to_string(),
            Value::Float(f) => {
                // Ensure floats always have a decimal point for JSON compatibility.
                let s = f.to_string();
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    s
                } else {
                    format!("{s}.0")
                }
            }
            Value::Complex { re, im } => format!("{{\"re\":{},\"im\":{}}}", re, im),
            Value::Quantity {
                value,
                dimension,
                unit,
            } => {
                let unit_str = unit.as_deref().unwrap_or("");
                format!(
                    "{{\"value\":{},\"dimension\":\"{}\",\"unit\":\"{}\"}}",
                    value, dimension, unit_str
                )
            }
            Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            Value::Null => "null".to_owned(),
            // For types that don't have a natural JSON mapping, emit as strings.
            Value::Enum(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            Value::Ref(id) => format!("\"{id}\""),
            Value::List(items) => {
                let inner: Vec<String> = items.iter().map(Self::value_to_json).collect();
                format!("[{}]", inner.join(","))
            }
            Value::Map(map) => {
                let entries: Vec<String> = map
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "\"{}\":{}",
                            k.replace('\\', "\\\\").replace('"', "\\\""),
                            Self::value_to_json(v)
                        )
                    })
                    .collect();
                format!("{{{}}}", entries.join(","))
            }
        }
    }

    /// Build the JSON request string for the given parameters.
    fn build_request(&self, params: &[SolverParam]) -> String {
        let param_entries: Vec<String> = params
            .iter()
            .map(|p| {
                let name = p
                    .tool_name
                    .as_deref()
                    .unwrap_or(&p.sysml_name);
                let value_json = match &p.value {
                    Some(v) => Self::value_to_json(v),
                    None => "null".to_owned(),
                };
                let direction = match p.direction {
                    ParamDirection::In => "in",
                    ParamDirection::Out => "out",
                    ParamDirection::InOut => "inout",
                };
                let escaped_name = name.replace('\\', "\\\\").replace('"', "\\\"");
                format!(
                    "{{\"name\":\"{escaped_name}\",\"value\":{value_json},\"direction\":\"{direction}\"}}"
                )
            })
            .collect();

        let escaped_solver = self.name.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            "{{\"solver\":\"{escaped_solver}\",\"params\":[{}]}}",
            param_entries.join(",")
        )
    }

    /// Parse a JSON value token at `pos` within `s`, returning the parsed `Value`
    /// and the index just past the consumed token.
    fn parse_json_value(s: &str, pos: usize) -> Result<(Value, usize), SolverError> {
        let s_trimmed = &s[pos..];
        let trimmed_offset = s_trimmed.len() - s_trimmed.trim_start().len();
        let start = pos + trimmed_offset;

        if start >= s.len() {
            return Err(SolverError::Runtime("unexpected end of JSON".into()));
        }

        match s.as_bytes()[start] {
            b'"' => {
                // String value
                let mut end = start + 1;
                while end < s.len() {
                    if s.as_bytes()[end] == b'\\' {
                        end += 2; // skip escaped char
                    } else if s.as_bytes()[end] == b'"' {
                        let content = &s[start + 1..end];
                        // Unescape basic sequences
                        let unescaped = content
                            .replace("\\\"", "\"")
                            .replace("\\\\", "\\")
                            .replace("\\n", "\n")
                            .replace("\\t", "\t");
                        return Ok((Value::String(unescaped), end + 1));
                    } else {
                        end += 1;
                    }
                }
                Err(SolverError::Runtime("unterminated string in JSON".into()))
            }
            b't' => {
                if s[start..].starts_with("true") {
                    Ok((Value::Bool(true), start + 4))
                } else {
                    Err(SolverError::Runtime(format!(
                        "unexpected token at position {start}"
                    )))
                }
            }
            b'f' => {
                if s[start..].starts_with("false") {
                    Ok((Value::Bool(false), start + 5))
                } else {
                    Err(SolverError::Runtime(format!(
                        "unexpected token at position {start}"
                    )))
                }
            }
            b'n' => {
                if s[start..].starts_with("null") {
                    Ok((Value::Null, start + 4))
                } else {
                    Err(SolverError::Runtime(format!(
                        "unexpected token at position {start}"
                    )))
                }
            }
            b'-' | b'0'..=b'9' => {
                // Number
                let mut end = start;
                let mut is_float = false;
                if end < s.len() && s.as_bytes()[end] == b'-' {
                    end += 1;
                }
                while end < s.len() && s.as_bytes()[end].is_ascii_digit() {
                    end += 1;
                }
                if end < s.len() && s.as_bytes()[end] == b'.' {
                    is_float = true;
                    end += 1;
                    while end < s.len() && s.as_bytes()[end].is_ascii_digit() {
                        end += 1;
                    }
                }
                if end < s.len() && (s.as_bytes()[end] == b'e' || s.as_bytes()[end] == b'E') {
                    is_float = true;
                    end += 1;
                    if end < s.len() && (s.as_bytes()[end] == b'+' || s.as_bytes()[end] == b'-') {
                        end += 1;
                    }
                    while end < s.len() && s.as_bytes()[end].is_ascii_digit() {
                        end += 1;
                    }
                }
                let num_str = &s[start..end];
                if is_float {
                    let f: f64 = num_str.parse().map_err(|e| {
                        SolverError::Runtime(format!("invalid float '{num_str}': {e}"))
                    })?;
                    Ok((Value::Float(f), end))
                } else {
                    let i: i64 = num_str.parse().map_err(|e| {
                        SolverError::Runtime(format!("invalid integer '{num_str}': {e}"))
                    })?;
                    Ok((Value::Int(i), end))
                }
            }
            b'[' => {
                // Array
                let mut items = Vec::new();
                let mut cur = start + 1;
                cur += s[cur..].len() - s[cur..].trim_start().len();
                if cur < s.len() && s.as_bytes()[cur] == b']' {
                    return Ok((Value::List(items), cur + 1));
                }
                loop {
                    let (val, next) = Self::parse_json_value(s, cur)?;
                    items.push(val);
                    cur = next;
                    cur += s[cur..].len() - s[cur..].trim_start().len();
                    if cur >= s.len() {
                        return Err(SolverError::Runtime("unterminated array".into()));
                    }
                    if s.as_bytes()[cur] == b']' {
                        return Ok((Value::List(items), cur + 1));
                    }
                    if s.as_bytes()[cur] == b',' {
                        cur += 1;
                    }
                }
            }
            b'{' => {
                // Object -- parse as HashMap<String, Value> and convert later
                let mut map: Vec<(String, Value)> = Vec::new();
                let mut cur = start + 1;
                cur += s[cur..].len() - s[cur..].trim_start().len();
                if cur < s.len() && s.as_bytes()[cur] == b'}' {
                    return Ok((Value::Map(std::collections::BTreeMap::new()), cur + 1));
                }
                loop {
                    // Parse key (must be a string)
                    let (key_val, next) = Self::parse_json_value(s, cur)?;
                    let Value::String(key) = key_val else {
                        return Err(SolverError::Runtime(
                            "expected string key in JSON object".into(),
                        ));
                    };
                    cur = next;
                    cur += s[cur..].len() - s[cur..].trim_start().len();
                    if cur >= s.len() || s.as_bytes()[cur] != b':' {
                        return Err(SolverError::Runtime("expected ':' in JSON object".into()));
                    }
                    cur += 1; // skip ':'
                    let (val, next) = Self::parse_json_value(s, cur)?;
                    map.push((key, val));
                    cur = next;
                    cur += s[cur..].len() - s[cur..].trim_start().len();
                    if cur >= s.len() {
                        return Err(SolverError::Runtime("unterminated object".into()));
                    }
                    if s.as_bytes()[cur] == b'}' {
                        let btree: std::collections::BTreeMap<String, Value> =
                            map.into_iter().collect();
                        return Ok((Value::Map(btree), cur + 1));
                    }
                    if s.as_bytes()[cur] == b',' {
                        cur += 1;
                    }
                }
            }
            other => Err(SolverError::Runtime(format!(
                "unexpected byte '{}' at position {start}",
                other as char
            ))),
        }
    }

    /// Parse the JSON response string into a `SolverResult`.
    ///
    /// Expected format:
    /// ```json
    /// {
    ///   "status": "converged",
    ///   "iterations": 5,
    ///   "outputs": {"key": value, ...},
    ///   "diagnostics": [...]
    /// }
    /// ```
    fn parse_response(response: &str) -> Result<SolverResult, SolverError> {
        let (val, _) = Self::parse_json_value(response, 0)?;
        let Value::Map(map) = val else {
            return Err(SolverError::Runtime(
                "expected JSON object in response".into(),
            ));
        };

        // Extract "status" -> converged
        let converged = match map.get("status") {
            Some(Value::String(s)) => s == "converged",
            _ => false,
        };

        // Extract "iterations"
        let iterations = match map.get("iterations") {
            Some(Value::Int(n)) => Some(*n as usize),
            Some(Value::Float(f)) => Some(*f as usize),
            _ => None,
        };

        // Extract "outputs" -> HashMap<String, Value>
        let outputs: HashMap<String, Value> = match map.get("outputs") {
            Some(Value::Map(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => HashMap::new(),
        };

        Ok(SolverResult {
            outputs,
            diagnostics: Vec::new(),
            iterations,
            converged,
        })
    }
}

impl SolverPlugin for ExternalSolverPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            supports_constraints: true,
            supports_optimization: false,
            supports_sensitivity: false,
            max_variables: None,
        }
    }

    fn solve(
        &self,
        inputs: &[SolverParam],
        _constraints: &[ConstraintIR],
        _context: &EvalContext,
    ) -> Result<SolverResult, SolverError> {
        // 1. Build JSON request
        let request_json = self.build_request(inputs);

        // 2. Spawn subprocess with stdin/stdout piped
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                SolverError::Runtime(format!(
                    "failed to spawn external solver '{}': {e}",
                    self.command
                ))
            })?;

        // 3. Write JSON to stdin, then close it
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(request_json.as_bytes()) {
                // A solver that exits before reading stdin (e.g. an
                // immediate usage/config failure) closes the pipe; EPIPE
                // here is not the real failure — the exit status read
                // below is, and it carries the solver's stderr.
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(SolverError::Runtime(format!(
                        "failed to write to solver stdin: {e}"
                    )));
                }
            }
            // stdin is dropped here, closing the pipe
        }

        // 4. Wait for output with timeout.
        //    We use a background thread + channel to implement the timeout without
        //    requiring async or platform-specific APIs.
        let timeout = self.timeout;
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let result = child.wait_with_output();
            let _ = tx.send(result);
        });

        let output = match rx.recv_timeout(timeout) {
            Ok(result) => {
                let _ = handle.join();
                result.map_err(|e| {
                    SolverError::Runtime(format!("failed to read solver output: {e}"))
                })?
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // The child is still running; we can't easily kill it from here
                // because it was moved into the thread. The thread will eventually
                // complete (or be reaped when the process exits).
                return Err(SolverError::Runtime(format!(
                    "external solver '{}' timed out after {:.1}s",
                    self.name,
                    timeout.as_secs_f64()
                )));
            }
            Err(e) => {
                return Err(SolverError::Runtime(format!(
                    "channel error waiting for solver: {e}"
                )));
            }
        };

        // 5. Check exit code
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_owned());
            return Err(SolverError::Runtime(format!(
                "external solver '{}' exited with code {code}: {stderr}",
                self.name
            )));
        }

        // 6. Parse JSON response from stdout
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stdout_trimmed = stdout.trim();
        if stdout_trimmed.is_empty() {
            return Err(SolverError::Runtime(format!(
                "external solver '{}' produced no output",
                self.name
            )));
        }

        Self::parse_response(stdout_trimmed)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // JSON serialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn value_to_json_primitives() {
        assert_eq!(
            ExternalSolverPlugin::value_to_json(&Value::Bool(true)),
            "true"
        );
        assert_eq!(
            ExternalSolverPlugin::value_to_json(&Value::Bool(false)),
            "false"
        );
        assert_eq!(ExternalSolverPlugin::value_to_json(&Value::Int(42)), "42");
        assert_eq!(
            ExternalSolverPlugin::value_to_json(&Value::Float(3.14)),
            "3.14"
        );
        assert_eq!(ExternalSolverPlugin::value_to_json(&Value::Null), "null");
    }

    #[test]
    fn value_to_json_string_escaping() {
        let val = Value::String("hello \"world\"".to_string());
        assert_eq!(
            ExternalSolverPlugin::value_to_json(&val),
            "\"hello \\\"world\\\"\""
        );
    }

    #[test]
    fn value_to_json_list() {
        let val = Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        assert_eq!(ExternalSolverPlugin::value_to_json(&val), "[1,2,3]");
    }

    #[test]
    fn build_request_no_params() {
        let plugin = ExternalSolverPlugin::new("test:empty", "echo");
        let json = plugin.build_request(&[]);
        assert_eq!(json, r#"{"solver":"test:empty","params":[]}"#);
    }

    #[test]
    fn build_request_with_params() {
        let plugin = ExternalSolverPlugin::new("test:thermal", "solver");
        let params = vec![
            SolverParam {
                sysml_name: "heaterPower".to_string(),
                tool_name: None,
                value: Some(Value::Float(1400.0)),
                direction: ParamDirection::In,
            },
            SolverParam {
                sysml_name: "temperature".to_string(),
                tool_name: Some("T".to_string()),
                value: None,
                direction: ParamDirection::Out,
            },
        ];
        let json = plugin.build_request(&params);
        assert!(json.contains("\"heaterPower\""));
        assert!(json.contains("1400"));
        assert!(json.contains("\"T\"")); // uses tool_name over sysml_name
        assert!(json.contains("\"direction\":\"out\""));
    }

    // -----------------------------------------------------------------------
    // JSON parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_response_converged() {
        let json =
            r#"{"status":"converged","iterations":5,"outputs":{"temp":93.5},"diagnostics":[]}"#;
        let result = ExternalSolverPlugin::parse_response(json).unwrap();
        assert!(result.converged);
        assert_eq!(result.iterations, Some(5));
        assert_eq!(result.outputs.get("temp"), Some(&Value::Float(93.5)));
    }

    #[test]
    fn parse_response_not_converged() {
        let json = r#"{"status":"diverged","iterations":100,"outputs":{}}"#;
        let result = ExternalSolverPlugin::parse_response(json).unwrap();
        assert!(!result.converged);
        assert_eq!(result.iterations, Some(100));
    }

    #[test]
    fn parse_response_empty_outputs() {
        let json = r#"{"status":"converged","iterations":1,"outputs":{},"diagnostics":[]}"#;
        let result = ExternalSolverPlugin::parse_response(json).unwrap();
        assert!(result.converged);
        assert!(result.outputs.is_empty());
    }

    #[test]
    fn parse_response_missing_fields() {
        // Minimal response -- only outputs
        let json = r#"{"outputs":{"x":42}}"#;
        let result = ExternalSolverPlugin::parse_response(json).unwrap();
        assert!(!result.converged); // no status field
        assert_eq!(result.iterations, None);
        assert_eq!(result.outputs.get("x"), Some(&Value::Int(42)));
    }

    #[test]
    fn parse_response_invalid_json() {
        let result = ExternalSolverPlugin::parse_response("not json");
        assert!(result.is_err());
    }

    #[test]
    fn parse_json_value_types() {
        // Boolean
        let (v, _) = ExternalSolverPlugin::parse_json_value("true", 0).unwrap();
        assert_eq!(v, Value::Bool(true));

        // Null
        let (v, _) = ExternalSolverPlugin::parse_json_value("null", 0).unwrap();
        assert_eq!(v, Value::Null);

        // Negative number
        let (v, _) = ExternalSolverPlugin::parse_json_value("-42", 0).unwrap();
        assert_eq!(v, Value::Int(-42));

        // Float with exponent
        let (v, _) = ExternalSolverPlugin::parse_json_value("1.5e3", 0).unwrap();
        assert_eq!(v, Value::Float(1500.0));
    }

    // -----------------------------------------------------------------------
    // Subprocess integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn external_solver_echo() {
        // Use "echo" to produce a fixed JSON response (ignoring stdin).
        let plugin = ExternalSolverPlugin::new("test:echo", "echo").with_args(vec![
            r#"{"status":"converged","iterations":1,"outputs":{},"diagnostics":[]}"#.into(),
        ]);
        let ctx = EvalContext::new();
        let result = plugin.solve(&[], &[], &ctx);
        assert!(result.is_ok(), "echo solver failed: {result:?}");
        let result = result.unwrap();
        assert!(result.converged);
        assert_eq!(result.iterations, Some(1));
    }

    #[test]
    fn external_solver_timeout() {
        // "sleep 60" will not produce output within 100ms.
        let plugin = ExternalSolverPlugin::new("test:slow", "sleep")
            .with_args(vec!["60".into()])
            .with_timeout(Duration::from_millis(100));
        let ctx = EvalContext::new();
        let result = plugin.solve(&[], &[], &ctx);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("timed out"), "unexpected error: {err_msg}");
    }

    #[test]
    fn external_solver_bad_command() {
        let plugin = ExternalSolverPlugin::new("test:bad", "/nonexistent/command");
        let ctx = EvalContext::new();
        let result = plugin.solve(&[], &[], &ctx);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("failed to spawn"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn external_solver_nonzero_exit() {
        // "false" exits with code 1.
        let plugin = ExternalSolverPlugin::new("test:fail", "false");
        let ctx = EvalContext::new();
        let result = plugin.solve(&[], &[], &ctx);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("exited with code"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn external_solver_name_and_capabilities() {
        let plugin = ExternalSolverPlugin::new("external:python:thermal", "/usr/bin/python3");
        assert_eq!(plugin.name(), "external:python:thermal");

        let caps = plugin.capabilities();
        assert!(caps.supports_constraints);
        assert!(!caps.supports_optimization);
        assert!(!caps.supports_sensitivity);
        assert!(caps.max_variables.is_none());
    }

    #[test]
    fn external_solver_with_timeout_builder() {
        let plugin = ExternalSolverPlugin::new("test", "cmd")
            .with_args(vec!["--flag".into()])
            .with_timeout(Duration::from_secs(60));
        assert_eq!(plugin.timeout, Duration::from_secs(60));
        assert_eq!(plugin.args, vec!["--flag"]);
    }
}
