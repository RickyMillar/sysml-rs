use std::io::IsTerminal as _;
use std::path::Path;

use sysml_service::{SysmlService, VerifyResult};

use crate::common::CliError;

/// Run `sysml verify CaseName file.sysml [--set key=val] [--json]`.
pub fn run(
    case_name: &str,
    file: &Path,
    overrides: &[(String, String)],
    json: bool,
) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let result = service.verify(case_name, overrides)?;

    if json {
        print_verify_json(case_name, &result);
    } else {
        print_verify_text(case_name, &result);
    }

    // Exit with verification failure on non-pass verdict.
    if result.verdict != "Pass" {
        return Err(CliError::verification(format!(
            "verdict: {}",
            result.verdict
        )));
    }

    Ok(())
}

fn print_verify_text(case_name: &str, result: &VerifyResult) {
    let color = std::io::stdout().is_terminal();

    // Header
    println!("verification case: {case_name}");

    // Verdict with color
    let verdict_str = &result.verdict;
    if color {
        let code = match verdict_str.as_str() {
            "Pass" => "\x1b[32m",         // green
            "Fail" => "\x1b[1;31m",       // bold red
            "Inconclusive" => "\x1b[33m", // yellow
            _ => "\x1b[1;31m",            // bold red (Error or unknown)
        };
        println!("verdict: {code}{verdict_str}\x1b[0m");
    } else {
        println!("verdict: {verdict_str}");
    }

    println!();

    for req in &result.requirements {
        let (icon, icon_color) = match req.verdict.as_str() {
            "Pass" => ("PASS", "\x1b[32m"),
            "Fail" => ("FAIL", "\x1b[1;31m"),
            "Inconclusive" => ("SKIP", "\x1b[33m"),
            _ => ("ERR ", "\x1b[1;31m"),
        };

        if color {
            println!(
                "  {icon_color}[{icon}]\x1b[0m {}: {}",
                req.requirement_id, req.message
            );
        } else {
            println!("  [{icon}] {}: {}", req.requirement_id, req.message);
        }
    }

    // Summary line — backend pre-aggregates the rollup so the CLI
    // doesn't recompute pass/fail counts.
    let total = result.requirements.len();
    if total > 0 {
        let pass_count = result.summary.pass;
        println!();
        if color {
            let summary_color = if pass_count == total {
                "\x1b[32m"
            } else {
                "\x1b[33m"
            };
            println!("{summary_color}{pass_count}/{total} requirements passed\x1b[0m");
        } else {
            println!("{pass_count}/{total} requirements passed");
        }
    }

    if !result.diagnostics.is_empty() {
        println!("\ndiagnostics:");
        for diag in &result.diagnostics {
            println!("  {diag}");
        }
    }
}

fn print_verify_json(case_name: &str, result: &VerifyResult) {
    let reqs: Vec<serde_json::Value> = result
        .requirements
        .iter()
        .map(|r| {
            serde_json::json!({
                "requirement": r.requirement_id,
                "verdict": r.verdict,
                "message": r.message,
            })
        })
        .collect();

    let output = serde_json::json!({
        "case": case_name,
        "verdict": result.verdict,
        "summary": result.summary,
        "requirements": reqs,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
