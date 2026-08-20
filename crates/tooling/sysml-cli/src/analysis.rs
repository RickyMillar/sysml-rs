use std::path::Path;

use sysml_service::{AnalysisResult, SysmlService};

use crate::common::CliError;

/// Run `sysml analysis CaseName file.sysml [--set key=val] [--json]`.
pub fn run(
    case_name: &str,
    file: &Path,
    overrides: &[(String, String)],
    json: bool,
) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    let result = service.analysis_run(case_name, overrides)?;

    if json {
        print_analysis_json(&result);
    } else {
        print_analysis_text(&result);
    }

    Ok(())
}

fn print_analysis_text(result: &AnalysisResult) {
    println!("Analysis: {}", result.case_name);
    if let Some(ref tool) = result.tool_name {
        println!("  Solver: {tool}");
    }
    if !result.input_parameters.is_empty() {
        println!("  Parameters:");
        for param in &result.input_parameters {
            let default = param
                .default_value
                .as_deref()
                .map(|v| format!(" = {v}"))
                .unwrap_or_default();
            println!(
                "    {} : {} ({}){default}",
                param.name, param.param_type, param.direction
            );
        }
    }
    println!("  Outputs:");
    let mut sorted_outputs: Vec<_> = result.outputs.iter().collect();
    sorted_outputs.sort_by_key(|(k, _)| k.clone());
    for (k, v) in &sorted_outputs {
        println!("    {k} = {v}");
    }
    println!(
        "  Status: {}",
        if result.converged {
            "CONVERGED"
        } else {
            "NOT CONVERGED"
        }
    );
    if let Some(iters) = result.iterations {
        println!("  Iterations: {iters}");
    }
}

fn print_analysis_json(result: &AnalysisResult) {
    let params: Vec<serde_json::Value> = result
        .input_parameters
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "type": p.param_type,
                "direction": p.direction,
                "default_value": p.default_value,
            })
        })
        .collect();
    let j = serde_json::json!({
        "case_name": result.case_name,
        "tool_name": result.tool_name,
        "input_parameters": params,
        "outputs": result.outputs,
        "converged": result.converged,
        "iterations": result.iterations,
    });
    println!("{}", serde_json::to_string_pretty(&j).unwrap());
}
