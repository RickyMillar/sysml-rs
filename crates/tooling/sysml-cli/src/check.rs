use std::path::Path;

use sysml_service::{ConstraintResult, SysmlService, VerdictKind};

use crate::common::CliError;

/// Run `sysml check file.sysml [--set key=val] [--json]`.
pub fn run(file: &Path, overrides: &[(String, String)], json: bool) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    let results = service.check_constraints(&uri, overrides)?;

    if results.is_empty() {
        if json {
            println!("{{\"constraints\": 0, \"results\": []}}");
        } else {
            println!("no constraints found in {}", file.display());
        }
        return Ok(());
    }

    if json {
        print_check_json(&results);
    } else {
        print_check_text(&results);
    }

    // Exit policy (spec: inconclusive is NOT a verification failure):
    //   any Fail  -> 3 (verification failure)
    //   any Error -> 2 (internal — evaluation could not run)
    //   else (Pass / Inconclusive only) -> 0
    if results.iter().any(|r| r.verdict == VerdictKind::Fail) {
        return Err(CliError::verification("one or more constraints failed"));
    }
    if results.iter().any(|r| r.verdict == VerdictKind::Error) {
        return Err(CliError::internal("a constraint could not be evaluated"));
    }

    Ok(())
}

fn verdict_icon(verdict: VerdictKind) -> &'static str {
    match verdict {
        VerdictKind::Pass => "PASS",
        VerdictKind::Fail => "FAIL",
        VerdictKind::Inconclusive => "SKIP",
        VerdictKind::Error => "ERROR",
    }
}

fn print_check_text(results: &[ConstraintResult]) {
    let total = results.len();
    let passed = results
        .iter()
        .filter(|r| r.verdict == VerdictKind::Pass)
        .count();
    let inconclusive = results
        .iter()
        .filter(|r| r.verdict == VerdictKind::Inconclusive)
        .count();
    let failed = total - passed - inconclusive;

    for result in results {
        let icon = verdict_icon(result.verdict);
        let name = &result.name;
        // Show expression alongside description when available
        if let Some(ref expr) = result.expression {
            if expr != name {
                println!("[{icon}] {name}: {expr}");
            } else {
                println!("[{icon}] {name}");
            }
        } else {
            println!("[{icon}] {name}");
        }
    }

    if inconclusive > 0 {
        println!(
            "\n{passed}/{total} constraints passed, {failed} failed, {inconclusive} inconclusive"
        );
    } else {
        println!("\n{passed}/{total} constraints passed, {failed} failed");
    }
}

fn print_check_json(results: &[ConstraintResult]) {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "description": r.name,
                "verdict": r.verdict,
                "satisfied": r.verdict == VerdictKind::Pass,
                "inconclusive": r.verdict == VerdictKind::Inconclusive,
                "instance": r.instance_path,
            })
        })
        .collect();

    let output = serde_json::json!({
        "constraints": results.len(),
        "results": items,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
