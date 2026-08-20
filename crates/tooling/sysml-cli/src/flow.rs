use std::path::Path;

use sysml_service::{FlowResult, SysmlService};

use crate::common::CliError;

/// Run `sysml flow [flow_name] file.sysml [--inject payload] [--json]`.
pub fn run(
    flow_name: Option<&str>,
    file: &Path,
    inject: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    // Determine inject_source from flow_name or the first flow.
    // We need to know the source key before calling the service, so we do
    // a preliminary inspect without injection first, then inject if needed.
    let (inject_source, inject_payload) = if let Some(payload_str) = inject {
        // Validate JSON objects strictly before passing to service.
        let trimmed = payload_str.trim();
        if trimmed.starts_with('{') {
            serde_json::from_str::<serde_json::Value>(trimmed)
                .map_err(|e| CliError::user(format!("invalid JSON payload: {e}")))?;
        }

        // First, get the flow topology to determine the source key.
        let topology = service.flow_inspect(&uri, None, None)?;

        let source_key = if let Some(name) = flow_name {
            topology
                .flows
                .iter()
                .find(|f| f.id == name)
                .map(|f| f.source.clone())
                .ok_or_else(|| CliError::user(format!("flow '{name}' not found")))?
        } else if topology.flows.len() == 1 {
            topology.flows[0].source.clone()
        } else if topology.flows.is_empty() {
            return Err(CliError::user(
                "no flows found in model".to_owned(),
            ));
        } else {
            return Err(CliError::user(
                "multiple flows found — specify flow name as first argument".to_owned(),
            ));
        };

        (Some(source_key), Some(payload_str.to_owned()))
    } else {
        (None, None)
    };

    let result = service.flow_inspect(
        &uri,
        inject_source.as_deref(),
        inject_payload.as_deref(),
    )?;

    if json {
        print_flow_json(&result);
    } else {
        print_flow_text(&result, inject_source.as_deref());
    }

    Ok(())
}

fn print_flow_text(result: &FlowResult, inject_source: Option<&str>) {
    println!("Ports: {} registered", result.ports.len());
    for port in &result.ports {
        let conj = if port.conjugated { " (conjugated)" } else { "" };
        let def = port.definition.as_deref().unwrap_or("untyped");
        println!("  {} : {} ({}){conj}", port.key, def, port.direction);
    }

    println!("\nFlows: {} compiled", result.flows.len());
    for flow in &result.flows {
        let succ = if flow.succession { " [succession]" } else { "" };
        let ptype = flow
            .payload_type
            .as_deref()
            .map(|t| format!(" <{t}>"))
            .unwrap_or_default();
        println!(
            "  {} : {} -> {}{succ}{ptype}",
            flow.id, flow.source, flow.target
        );
    }

    // Show delivery results if injection was performed
    if !result.delivery.is_empty() {
        if let Some(source) = inject_source {
            println!("\nInjecting payload into '{source}':");
        }
        for msg in &result.delivery {
            println!("  -> delivered to '{}' (seq={})", msg.target, msg.sequence);
        }
    } else if inject_source.is_some() {
        println!("\n  -> no messages delivered (check flow topology)");
    }

    // Show port health diagnostics (FL001-FL015)
    if !result.diagnostics.is_empty() {
        println!("\nDiagnostics: {} issue(s)", result.diagnostics.len());
        for diag in &result.diagnostics {
            let prefix = match diag.severity.as_str() {
                "error" => "ERROR",
                "warning" => "WARN ",
                _ => "INFO ",
            };
            println!("  [{}] {}: {}", prefix, diag.code, diag.message);
        }
    }
}

fn print_flow_json(result: &FlowResult) {
    let ports: Vec<serde_json::Value> = result
        .ports
        .iter()
        .map(|port| {
            serde_json::json!({
                "key": port.key,
                "owner": port.owner,
                "name": port.name,
                "definition": port.definition,
                "direction": port.direction,
                "conjugated": port.conjugated,
            })
        })
        .collect();

    let flow_items: Vec<serde_json::Value> = result
        .flows
        .iter()
        .map(|f| {
            serde_json::json!({
                "id": f.id,
                "source": f.source,
                "target": f.target,
                "succession": f.succession,
                "payloadType": f.payload_type,
            })
        })
        .collect();

    let mut output = serde_json::json!({
        "ports": ports,
        "flows": flow_items,
    });

    if !result.delivery.is_empty() {
        let delivery_items: Vec<serde_json::Value> = result
            .delivery
            .iter()
            .map(|msg| {
                serde_json::json!({
                    "flow_id": msg.flow_id,
                    "source": msg.source,
                    "target": msg.target,
                    "sequence": msg.sequence,
                })
            })
            .collect();
        output["delivery"] = serde_json::json!(delivery_items);
    }

    if !result.diagnostics.is_empty() {
        let diag_items: Vec<serde_json::Value> = result
            .diagnostics
            .iter()
            .map(|d| {
                serde_json::json!({
                    "code": d.code,
                    "severity": d.severity,
                    "message": d.message,
                    "port": d.port,
                })
            })
            .collect();
        output["diagnostics"] = serde_json::json!(diag_items);
    }

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
