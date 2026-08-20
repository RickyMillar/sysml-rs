use std::path::Path;

use sysml_service::{SysmlService, TraceResult};

use crate::common::CliError;

/// Run `sysml trace file.sysml [--inject source:payload] [--json]`.
///
/// Demonstrates sequence trace generation from flow topology.
/// Simulates message flow through compiled flows and generates
/// a sequence diagram trace.
pub fn run(
    file: &Path,
    inject_specs: &[String],
    json: bool,
) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    // Parse inject specs from CLI format "source.port:value" into (key, value) pairs.
    let parsed_specs: Vec<(String, String)> = inject_specs
        .iter()
        .filter_map(|spec| {
            let parts: Vec<&str> = spec.splitn(2, ':').collect();
            if parts.len() != 2 {
                eprintln!("warning: --inject format is source.port:value (e.g. pump.waterOut:42)");
                None
            } else {
                Some((parts[0].to_owned(), parts[1].to_owned()))
            }
        })
        .collect();

    let result = service.trace_sequence(&uri, &parsed_specs)?;

    if result.lifelines.is_empty() && result.messages.is_empty() && inject_specs.is_empty() {
        if json {
            println!("{{\"lifelines\": [], \"messages\": []}}");
        } else {
            println!("No flows compiled. Use --inject source:payload to simulate messages.");
        }
        return Ok(());
    }

    if json {
        print_trace_json(&result);
    } else {
        print_trace_text(&result);
    }

    Ok(())
}

fn print_trace_text(trace: &TraceResult) {
    println!("Sequence Trace:");
    println!("  Lifelines: {}", trace.lifelines.len());
    for ll in &trace.lifelines {
        println!("    [{}] {} ({})", ll.index, ll.name, ll.kind);
    }

    println!("  Messages: {}", trace.messages.len());
    for msg in &trace.messages {
        let payload_str = msg
            .payload
            .as_ref()
            .map(|p| format!(" [{p}]"))
            .unwrap_or_default();
        println!(
            "    #{} @{:.0}ms: {} -> {} : {}{}",
            msg.sequence, msg.timestamp_ms, msg.from, msg.to, msg.label, payload_str
        );
    }
}

fn print_trace_json(trace: &TraceResult) {
    let lifelines: Vec<serde_json::Value> = trace
        .lifelines
        .iter()
        .map(|ll| {
            serde_json::json!({
                "index": ll.index,
                "name": ll.name,
                "kind": ll.kind,
            })
        })
        .collect();

    let messages: Vec<serde_json::Value> = trace
        .messages
        .iter()
        .map(|msg| {
            serde_json::json!({
                "sequence": msg.sequence,
                "from": msg.from,
                "to": msg.to,
                "label": msg.label,
                "timestamp_ms": msg.timestamp_ms,
                "payload": msg.payload,
            })
        })
        .collect();

    let output = serde_json::json!({
        "lifelines": lifelines,
        "messages": messages,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
