use std::path::Path;

use sysml_service::{SolveResult, SysmlService};

use crate::common::CliError;

/// Run `sysml solve file.sysml [--set key=val] [--rollup property] [--sweep param:lo:hi] [--json]`.
pub fn run(
    file: &Path,
    overrides: &[(String, String)],
    rollup_property: Option<&str>,
    sweep_spec: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    let result = service.solve(&uri, overrides, rollup_property, sweep_spec)?;

    if json {
        print_solve_json(&result);
    } else {
        print_solve_text(&result, rollup_property);
    }

    Ok(())
}

fn print_solve_text(result: &SolveResult, rollup_property: Option<&str>) {
    println!("Binding Propagation:");
    println!("  iterations: {}", result.iterations);
    println!("  solved: {} variables", result.solved.len());

    if !result.solved.is_empty() {
        let mut sorted: Vec<_> = result.solved.iter().collect();
        sorted.sort_by_key(|(k, _)| k.clone());
        for (name, value) in sorted {
            println!("    {name} = {value}");
        }
    }

    if !result.unsolved.is_empty() {
        println!("  unsolved: {:?}", result.unsolved);
    }

    println!("\nDOF Analysis:");
    println!(
        "  {} equations, {} variables ({} known, {} free)",
        result.dof.equations, result.dof.variables, result.dof.known_count, result.dof.free_count
    );
    println!("  DOF = {}, status: {}", result.dof.dof, result.dof.status);

    if !result.rollups.is_empty() {
        println!(
            "\nRollup (sum of '{}'):",
            rollup_property.unwrap_or("?")
        );
        let mut sorted: Vec<_> = result.rollups.iter().collect();
        sorted.sort_by_key(|(k, _)| k.clone());
        for (name, value) in sorted {
            println!("    {name} = {value}");
        }
    }

    if let Some(ref s) = result.sensitivity {
        println!("\nSensitivity sweep of '{}':", s.parameter);
        for effect in &s.effects {
            if let Some(flip_val) = effect.flip_value {
                let dir = effect
                    .flip_direction
                    .as_ref()
                    .map(|d| d.clone())
                    .unwrap_or_default();
                println!("  '{}' flips at {} ({dir})", effect.constraint_name, flip_val);
            } else {
                println!("  '{}' stable across range", effect.constraint_name);
            }
        }
    }
}

fn print_solve_json(result: &SolveResult) {
    let solved: serde_json::Map<String, serde_json::Value> = result
        .solved
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();

    let rollup_items: serde_json::Map<String, serde_json::Value> = result
        .rollups
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();

    let sweep_json = result.sensitivity.as_ref().map(|s| {
        let effects: Vec<serde_json::Value> = s
            .effects
            .iter()
            .map(|e| {
                serde_json::json!({
                    "constraint": e.constraint_name,
                    "flipValue": e.flip_value,
                    "flipDirection": e.flip_direction,
                })
            })
            .collect();
        serde_json::json!({
            "parameter": s.parameter,
            "steps": s.steps,
            "effects": effects,
        })
    });

    let output = serde_json::json!({
        "iterations": result.iterations,
        "solved": solved,
        "unsolved": result.unsolved,
        "dof": {
            "equations": result.dof.equations,
            "variables": result.dof.variables,
            "known": result.dof.known_count,
            "free": result.dof.free_count,
            "dof": result.dof.dof,
            "status": result.dof.status,
        },
        "rollups": rollup_items,
        "sensitivity": sweep_json,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
