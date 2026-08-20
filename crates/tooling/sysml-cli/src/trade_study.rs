use std::path::Path;

use sysml_service::SysmlService;

use crate::common::CliError;

/// Run `sysml trade-study StudyName file.sysml [--set key=val] [--json]`.
pub fn run(
    study_name: &str,
    file: &Path,
    overrides: &[(String, String)],
    json: bool,
) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    let result = service.trade_study(study_name, overrides)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        print_text(&result, study_name);
    }

    Ok(())
}

fn print_text(result: &serde_json::Value, study_name: &str) {
    println!("Trade Study: {study_name}");
    println!("  Alternatives:");
    let best = result.get("best").and_then(|v| v.as_str()).unwrap_or("");
    let best_score = result
        .get("best_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    if let Some(alts) = result.get("alternatives").and_then(|v| v.as_array()) {
        for alt in alts {
            let name = alt.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let score = alt.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let marker = if name == best { " <-- BEST" } else { "" };
            println!("    {name}: {score:.4}{marker}");
        }
    }
    println!("  Best: {best} (score: {best_score:.4})");
}
