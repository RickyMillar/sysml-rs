use std::path::PathBuf;

use clap::Subcommand;
use sysml_core::ElementKind;
use sysml_service::SysmlService;

use crate::common::CliError;

#[derive(Subcommand)]
pub enum QueryCommand {
    /// Find elements by name pattern
    Find {
        /// Path to the SysML file
        file: PathBuf,
        /// Name pattern to search for (substring match)
        #[arg(long)]
        name: String,
        /// Filter by element kind (e.g. PartUsage, RequirementUsage)
        #[arg(long)]
        kind: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show element statistics
    Stats {
        /// Path to the SysML file
        file: PathBuf,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show traceability matrix (requirements to parts via Satisfy)
    Trace {
        /// Path to the SysML file
        file: PathBuf,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show unverified requirements
    Unverified {
        /// Path to the SysML file
        file: PathBuf,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

pub fn run(command: QueryCommand) -> Result<(), CliError> {
    match command {
        QueryCommand::Find {
            file,
            name,
            kind,
            json,
        } => run_find(&file, &name, kind.as_deref(), json),
        QueryCommand::Stats { file, json } => run_stats(&file, json),
        QueryCommand::Trace { file, json } => run_trace(&file, json),
        QueryCommand::Unverified { file, json } => run_unverified(&file, json),
    }
}

fn run_find(
    file: &std::path::Path,
    name: &str,
    kind: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    let kind_filter = if let Some(k) = kind {
        let ek = ElementKind::from_str(k)
            .ok_or_else(|| CliError::user(format!("unknown element kind: '{k}'")))?;
        Some(ek)
    } else {
        None
    };

    let results = service.find(&uri, name, kind_filter.as_ref())?;

    if json {
        let items: Vec<serde_json::Value> = results
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id.as_str(),
                    "name": e.name.as_deref().unwrap_or(""),
                    "kind": e.kind.as_str(),
                })
            })
            .collect();
        let output = serde_json::json!({
            "count": items.len(),
            "results": items,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        if results.is_empty() {
            println!("no elements matching '{name}' found");
            return Ok(());
        }
        println!("{:<40} {:<30} {}", "ID", "KIND", "NAME");
        println!("{}", "-".repeat(90));
        for e in &results {
            println!(
                "{:<40} {:<30} {}",
                e.id.as_str(),
                e.kind.as_str(),
                e.name.as_deref().unwrap_or("(unnamed)"),
            );
        }
        println!("\n{} element(s) found", results.len());
    }

    Ok(())
}

fn run_stats(file: &std::path::Path, json: bool) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    let stats = service.stats(&uri)?;

    if json {
        let output = serde_json::json!({
            "total_elements": stats.total_elements,
            "total_relationships": stats.total_relationships,
            "elements_by_kind": stats.elements_by_kind,
            "relationships_by_kind": stats.relationships_by_kind,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        println!("Elements: {}", stats.total_elements);
        println!("Relationships: {}", stats.total_relationships);

        if !stats.elements_by_kind.is_empty() {
            println!("\nElements by kind:");
            let mut sorted: Vec<_> = stats.elements_by_kind.iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(a.1));
            for (kind, count) in sorted {
                println!("  {:<35} {}", kind, count);
            }
        }

        if !stats.relationships_by_kind.is_empty() {
            println!("\nRelationships by kind:");
            let mut sorted: Vec<_> = stats.relationships_by_kind.iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(a.1));
            for (kind, count) in sorted {
                println!("  {:<35} {}", kind, count);
            }
        }
    }

    Ok(())
}

fn run_trace(file: &std::path::Path, json: bool) -> Result<(), CliError> {
    use sysml_core::RelationshipKind;

    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    let rows = service.trace_matrix(
        &uri,
        &ElementKind::PartUsage,
        &RelationshipKind::Satisfy,
        &ElementKind::RequirementUsage,
    )?;

    if json {
        let items: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "source": r.source_name.as_deref().unwrap_or(""),
                    "target": r.target_name.as_deref().unwrap_or(""),
                    "source_id": r.source.as_str(),
                    "target_id": r.target.as_str(),
                })
            })
            .collect();
        let output = serde_json::json!({
            "count": items.len(),
            "rows": items,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        if rows.is_empty() {
            println!("no satisfy relationships found (PartUsage -> RequirementUsage)");
            return Ok(());
        }
        println!("{:<35} {:<10} {}", "SOURCE (Part)", "REL", "TARGET (Requirement)");
        println!("{}", "-".repeat(80));
        for r in &rows {
            println!(
                "{:<35} {:<10} {}",
                r.source_name.as_deref().unwrap_or("(unnamed)"),
                "satisfy",
                r.target_name.as_deref().unwrap_or("(unnamed)"),
            );
        }
        println!("\n{} trace(s) found", rows.len());
    }

    Ok(())
}

fn run_unverified(file: &std::path::Path, json: bool) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    let results = service.unverified(&uri)?;

    if json {
        let items: Vec<serde_json::Value> = results
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id.as_str(),
                    "name": e.name.as_deref().unwrap_or(""),
                })
            })
            .collect();
        let output = serde_json::json!({
            "count": items.len(),
            "requirements": items,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        if results.is_empty() {
            println!("all requirements are verified (or no requirements found)");
            return Ok(());
        }
        println!("{:<40} {}", "ID", "NAME");
        println!("{}", "-".repeat(60));
        for e in &results {
            println!(
                "{:<40} {}",
                e.id.as_str(),
                e.name.as_deref().unwrap_or("(unnamed)"),
            );
        }
        println!("\n{} unverified requirement(s)", results.len());
    }

    Ok(())
}
