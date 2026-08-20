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

/// View type for SModel export.
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum SmodelView {
    General,
    Interconnection,
    State,
    Action,
    Requirements,
    Browser,
    Sequence,
    Grid,
    Geometry,
    Parametric,
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
    /// Export model as Sprotty SModel JSON
    Smodel {
        /// Path to the SysML file
        file: PathBuf,
        /// Diagram view type
        #[arg(long, value_enum, default_value_t = SmodelView::General)]
        view: SmodelView,
        /// Expand all expandable nodes (show children as nested boxes)
        #[arg(long)]
        expand_all: bool,
    },
}

pub fn run(command: ExportCommand) -> Result<(), CliError> {
    match command {
        ExportCommand::Plantuml { file, view } => run_plantuml(&file, &view),
        ExportCommand::Json { file, pretty } => run_json(&file, pretty),
        ExportCommand::Smodel { file, view, expand_all } => run_smodel(&file, &view, expand_all),
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

fn run_smodel(file: &std::path::Path, view: &SmodelView, expand_all: bool) -> Result<(), CliError> {
    // Bucket B / B5 P1: SModel pipeline lives on the service. CLI is a thin
    // load → dispatch → print shell.
    let view_str = match view {
        SmodelView::General => "general",
        SmodelView::Interconnection => "interconnection",
        SmodelView::State => "state",
        SmodelView::Action => "action",
        SmodelView::Requirements => "requirements",
        SmodelView::Browser => "browser",
        SmodelView::Sequence => "sequence",
        SmodelView::Grid => "grid",
        SmodelView::Geometry => "geometry",
        SmodelView::Parametric => "parametric",
    };

    let service = SysmlService::empty();
    let uri = service.load_file(file)?;
    let value = service.export_smodel(&uri, view_str, expand_all)?;
    let output = serde_json::to_string_pretty(&value).unwrap_or_default();
    println!("{output}");
    Ok(())
}
