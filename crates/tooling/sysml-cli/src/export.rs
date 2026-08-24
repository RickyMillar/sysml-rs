use std::path::PathBuf;

use clap::Subcommand;
use sysml_service::SysmlService;

use crate::common::CliError;

/// View type for PlantUML export.
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum PlantUmlView {
    General,
    State,
    Action,
    Sequence,
}

impl PlantUmlView {
    /// Selector string matching the `sysml.export.plantuml` service command's
    /// `view` argument.
    fn as_str(&self) -> &'static str {
        match self {
            PlantUmlView::General => "general",
            PlantUmlView::State => "state",
            PlantUmlView::Action => "action",
            PlantUmlView::Sequence => "sequence",
        }
    }
}

#[derive(Subcommand)]
pub enum ExportCommand {
    /// Export model as PlantUML diagram
    Plantuml {
        /// Path to the SysML file
        file: PathBuf,
        /// Diagram view type
        #[arg(long, value_enum, default_value_t = PlantUmlView::General)]
        view: PlantUmlView,
    },
    /// Export model as canonical JSON
    Json {
        /// Path to the SysML file
        file: PathBuf,
        /// Pretty-print the JSON output
        #[arg(long)]
        pretty: bool,
    },
    /// Export a declared view's ViewModel JSON (scene + tokens + text-map +
    /// interactions + frame / non-graph payload), sidecars pruned to the
    /// view's referenced ids
    Viewmodel {
        /// Workspace directory to load (declared views render against the
        /// whole workspace)
        #[arg(long)]
        workspace: PathBuf,
        /// Qualified name of the declared view to export
        /// (e.g. ShowcaseViews::OverviewView; a unique bare name also resolves)
        #[arg(long)]
        view: String,
        /// Expand every expandable node
        #[arg(long, conflicts_with = "expand")]
        expand_all: bool,
        /// Element id to render expanded (repeatable)
        #[arg(long)]
        expand: Vec<String>,
        /// Write the JSON to this file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

pub fn run(command: ExportCommand) -> Result<(), CliError> {
    match command {
        ExportCommand::Plantuml { file, view } => run_plantuml(&file, &view),
        ExportCommand::Json { file, pretty } => run_json(&file, pretty),
        ExportCommand::Viewmodel {
            workspace,
            view,
            expand_all,
            expand,
            output,
        } => run_viewmodel(&workspace, &view, expand_all, &expand, output.as_deref()),
    }
}

fn run_plantuml(file: &std::path::Path, view: &PlantUmlView) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    // Single service call across all views — the per-view branching now
    // lives inside `sysml.export.plantuml` (see sysml-service::lib).
    let output = service.export_plantuml(&uri, Some(view.as_str()))?;

    println!("{output}");
    Ok(())
}

fn run_json(file: &std::path::Path, pretty: bool) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    let output = if pretty {
        // service.export_json() returns compact JSON; re-format for pretty output
        let compact = service.export_json(&uri)?;
        let value: serde_json::Value =
            serde_json::from_str(&compact).unwrap_or(serde_json::Value::Null);
        serde_json::to_string_pretty(&value).unwrap_or(compact)
    } else {
        service.export_json(&uri)?
    };

    println!("{output}");
    Ok(())
}

fn run_viewmodel(
    workspace: &std::path::Path,
    view: &str,
    expand_all: bool,
    expand: &[String],
    output: Option<&std::path::Path>,
) -> Result<(), CliError> {
    let service = SysmlService::from_workspace(workspace)?;
    let views = service.workspace_declared_views()?;
    let view_id = resolve_view_arg(&views, view)?.clone();

    let expanded: std::collections::HashSet<String> = expand.iter().cloned().collect();
    let value = service.export_view_model(&view_id, &expanded, expand_all)?;
    let json = serde_json::to_string_pretty(&value).unwrap_or_default();
    match output {
        Some(path) => std::fs::write(path, json)
            .map_err(|e| CliError::internal(format!("write {}: {e}", path.display())))?,
        None => println!("{json}"),
    }
    Ok(())
}

/// Resolve a `--view` argument against the workspace's declared views
/// (`(qualified_name, id)` pairs). An exact qualified-name match wins; a bare
/// name resolves when it names exactly one view; anything else is a user error
/// listing the alternatives.
fn resolve_view_arg<'a, T>(views: &'a [(String, T)], arg: &str) -> Result<&'a T, CliError> {
    let exact: Vec<&(String, T)> = views.iter().filter(|(qname, _)| qname == arg).collect();
    let matches = if exact.is_empty() {
        let suffix = format!("::{arg}");
        views
            .iter()
            .filter(|(qname, _)| qname.ends_with(&suffix))
            .collect()
    } else {
        exact
    };
    match matches.as_slice() {
        [(_, id)] => Ok(id),
        [] => {
            let available: Vec<&str> = views.iter().map(|(q, _)| q.as_str()).collect();
            Err(CliError::user(format!(
                "view '{arg}' not found in workspace. Declared views:\n  {}",
                available.join("\n  ")
            )))
        }
        many => {
            let names: Vec<&str> = many.iter().map(|(q, _)| q.as_str()).collect();
            Err(CliError::user(format!(
                "view '{arg}' is ambiguous; use a qualified name:\n  {}",
                names.join("\n  ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_view_arg;

    fn views() -> Vec<(String, u32)> {
        vec![
            ("Pkg::OverviewView".to_owned(), 1),
            ("Pkg::CatalogView".to_owned(), 2),
            ("Other::CatalogView".to_owned(), 3),
        ]
    }

    #[test]
    fn qualified_name_resolves_exactly() {
        assert_eq!(resolve_view_arg(&views(), "Pkg::OverviewView").unwrap(), &1);
    }

    #[test]
    fn unique_bare_name_resolves() {
        assert_eq!(resolve_view_arg(&views(), "OverviewView").unwrap(), &1);
    }

    #[test]
    fn ambiguous_bare_name_lists_candidates() {
        let err = resolve_view_arg(&views(), "CatalogView").unwrap_err();
        assert!(err.message.contains("ambiguous"), "{}", err.message);
        assert!(err.message.contains("Pkg::CatalogView"));
        assert!(err.message.contains("Other::CatalogView"));
    }

    #[test]
    fn unknown_name_lists_available_views() {
        let err = resolve_view_arg(&views(), "NoSuchView").unwrap_err();
        assert!(err.message.contains("not found"), "{}", err.message);
        assert!(err.message.contains("Pkg::OverviewView"));
    }

    #[test]
    fn bare_name_never_matches_mid_segment() {
        // "View" is a suffix of every name but a full segment of none.
        let err = resolve_view_arg(&views(), "View").unwrap_err();
        assert!(err.message.contains("not found"), "{}", err.message);
    }
}
