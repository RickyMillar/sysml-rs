use std::path::Path;

use sysml_service::SysmlService;

use crate::common::CliError;

/// Run `sysml run <action_name> <file> [--trace] [--json]`.
pub fn run(action_name: &str, file: &Path, trace: bool, json: bool) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    let session_key = service.action_start(&uri, action_name)?.to_string();

    if !json {
        println!("action: {action_name}");
    }

    let mut steps = Vec::new();
    let max_steps = 10_000;

    for step_num in 0..max_steps {
        let entry = service.action_step(&session_key)?;

        if trace || json {
            if json {
                steps.push(serde_json::json!({
                    "step": step_num,
                    "outputs": entry.outputs,
                    "diagnostics": entry.diagnostics,
                    "completed": entry.completed,
                }));
            } else if !entry.outputs.is_empty() {
                println!("  step {step_num}:");
                for output in &entry.outputs {
                    println!("    {output}");
                }
            }
        }

        // Show per-step diagnostics (errors/warnings from action execution).
        for diag in &entry.diagnostics {
            eprintln!("  diag: {diag}");
        }

        // Flow events from the service layer are logged as warnings
        // (they're informational, not diagnostics in the traditional sense).
        for fe in &entry.flow_events {
            eprintln!("  flow: {}", fe.description);
        }

        if entry.completed {
            if !json {
                println!("completed after {} step(s)", step_num + 1);
            }
            break;
        }
    }

    if json {
        let output = serde_json::json!({
            "action": action_name,
            "steps": steps,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    }

    Ok(())
}
