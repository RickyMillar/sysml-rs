// CLI binary — stdout/stderr is the user interface.
// Session lookups, argument parsing, and structured data access use
// expect/unwrap/indexing that are infallible in context.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process;

mod add;
mod analysis;
mod cache;
mod check;
mod common;
mod eval;
mod export;
mod flow;
mod fetch;
mod info;
mod init;
mod inspect;
mod lock;
mod progress;
mod package;
mod project;
mod query;
mod remove;
mod run;
#[cfg(feature = "server")]
mod serve;
mod simulate;
mod solve;
mod trace;
mod trade_study;
mod tree;
mod update;
mod verify;
mod why;

#[derive(Parser)]
#[command(name = "sysml", about = "SysML v2 execution tool", version)]
struct Cli {
    /// Suppress progress output on stderr (also silences the progress
    /// subscriber even when stderr is a TTY). Place before the subcommand.
    #[arg(long)]
    quiet: bool,
    /// Force progress output to stderr even when stderr is not a TTY.
    /// Useful in CI / integration tests where `is_terminal()` is false.
    /// Also enabled via `SYSML_FORCE_PROGRESS=1`.
    #[arg(long, hide = true)]
    force_progress: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new SysML project
    Init {
        /// Project name (creates a new directory)
        #[arg(long)]
        name: Option<String>,
    },
    /// Show project information
    Info {
        /// Path to sysml.toml (auto-discovered if omitted)
        #[arg(long)]
        manifest_path: Option<PathBuf>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Add a dependency to sysml.toml
    Add {
        /// Dependency name
        name: String,
        /// Local path dependency
        #[arg(long)]
        path: Option<String>,
        /// Git repository URL
        #[arg(long)]
        git: Option<String>,
        /// Git tag
        #[arg(long)]
        tag: Option<String>,
        /// Git branch
        #[arg(long)]
        branch: Option<String>,
        /// Git revision (commit hash)
        #[arg(long)]
        rev: Option<String>,
        /// KPAR archive URL
        #[arg(long)]
        kpar: Option<String>,
    },
    /// Remove a dependency from sysml.toml
    Remove {
        /// Dependency name to remove
        name: String,
    },
    /// Resolve dependencies and update sysml.lock
    Lock {
        /// Force re-resolve even if lock file is up to date
        #[arg(long)]
        force: bool,
        /// Suppress non-error output
        #[arg(long)]
        quiet: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Resolve dependencies and fetch/cache all sources (without writing sysml.lock)
    Fetch {
        /// Suppress non-error output
        #[arg(long)]
        quiet: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Force dependency update and rewrite sysml.lock
    Update {
        /// Suppress non-error output
        #[arg(long)]
        quiet: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show dependency graph
    Tree {
        /// Suppress non-error output
        #[arg(long)]
        quiet: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show why a dependency exists in the resolved graph
    Why {
        /// Dependency package name
        name: String,
        /// Suppress non-error output
        #[arg(long)]
        quiet: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage local dependency cache
    Cache {
        #[command(subcommand)]
        command: cache::CacheCommand,
    },
    /// Build a .kpar distribution archive
    Package {
        /// Path to sysml.toml (auto-discovered if omitted)
        #[arg(long)]
        manifest_path: Option<PathBuf>,
        /// Output directory (default: target/package/)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Evaluate a SysML expression
    Eval {
        /// The expression to evaluate (e.g. "2 + 3")
        expr: String,
    },
    /// Check constraints in a SysML file
    Check {
        /// Path to the SysML file
        file: PathBuf,
        /// Override attribute values (e.g. --set mass=2600)
        #[arg(long = "set", value_parser = common::parse_key_val)]
        overrides: Vec<(String, String)>,
        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Run a verification case
    Verify {
        /// Name of the verification case to run
        case_name: String,
        /// Path to the SysML file
        file: PathBuf,
        /// Override attribute values (e.g. --set speed=85)
        #[arg(long = "set", value_parser = common::parse_key_val)]
        overrides: Vec<(String, String)>,
        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Run an analysis case
    Analysis {
        /// Name of the analysis case to run
        case_name: String,
        /// Path to the SysML file
        file: PathBuf,
        /// Override attribute values (e.g. --set temperature=350)
        #[arg(long = "set", value_parser = common::parse_key_val)]
        overrides: Vec<(String, String)>,
        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Run a trade study (evaluate alternatives against an objective)
    TradeStudy {
        /// Name of the trade study analysis case
        study_name: String,
        /// Path to the SysML file
        file: PathBuf,
        /// Override attribute values (e.g. --set mass=100)
        #[arg(long = "set", value_parser = common::parse_key_val)]
        overrides: Vec<(String, String)>,
        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Simulate a state machine
    Simulate {
        /// Name of the state machine
        sm_name: String,
        /// Path to the SysML file
        file: PathBuf,
        /// Comma-separated list of events (e.g. "timer,timer,reset")
        #[arg(long)]
        events: Option<String>,
        /// Interactive mode: read events from stdin
        #[arg(long)]
        interactive: bool,
        /// Auto-demo mode: walk all transitions automatically
        #[arg(long)]
        auto: bool,
        /// Show detailed execution trace
        #[arg(long)]
        trace: bool,
        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Run an action
    Run {
        /// Name of the action to run
        action_name: String,
        /// Path to the SysML file
        file: PathBuf,
        /// Show detailed execution trace
        #[arg(long)]
        trace: bool,
        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Solve constraint network via binding propagation
    Solve {
        /// Path to the SysML file
        file: PathBuf,
        /// Override attribute values (e.g. --set mass=2600)
        #[arg(long = "set", value_parser = common::parse_key_val)]
        overrides: Vec<(String, String)>,
        /// Compute rollup for a named property (e.g. --rollup mass)
        #[arg(long)]
        rollup: Option<String>,
        /// Sweep a parameter across a range (format: param:lo:hi, e.g. --sweep speed:0:200)
        #[arg(long)]
        sweep: Option<String>,
        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Generate a sequence trace from flow simulation
    Trace {
        /// Path to the SysML file
        file: PathBuf,
        /// Inject messages to simulate flow (format: source.port:value, repeatable)
        #[arg(long)]
        inject: Vec<String>,
        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Inspect and test port flows
    Flow {
        /// Path to the SysML file
        file: PathBuf,
        /// Name of a specific flow (optional — shows all if omitted)
        #[arg(long)]
        flow_name: Option<String>,
        /// Inject a payload into the flow source (JSON or simple value)
        #[arg(long)]
        inject: Option<String>,
        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage SysML projects (legacy)
    Project {
        #[command(subcommand)]
        command: project::ProjectCommand,
    },
    /// Query elements in a SysML model
    Query {
        #[command(subcommand)]
        command: query::QueryCommand,
    },
    /// Export a SysML model to various formats
    Export {
        #[command(subcommand)]
        command: export::ExportCommand,
    },
    /// Start the SysML REST API server
    #[cfg(feature = "server")]
    Serve {
        /// Port to listen on
        #[arg(long, default_value_t = 3000)]
        port: u16,
        /// Host address to bind to. Defaults to loopback: the API has
        /// unauthenticated writes unless SYSML_API_TOKEN is set, so reaching
        /// beyond this machine is opt-in.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
    /// Inspect semantic tokens and diagnostics for a SysML file
    Inspect {
        /// Path to the SysML file (required unless --workspace is used)
        file: Option<PathBuf>,
        /// Show only semantic tokens
        #[arg(long)]
        tokens: bool,
        /// Show only diagnostics
        #[arg(long)]
        diagnostics: bool,
        /// Show raw CST (tree-sitter parse tree)
        #[arg(long)]
        cst: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Disable loading the standard library for inspect diagnostics
        #[arg(long)]
        no_stdlib: bool,
        /// Override standard library path (directory containing library.kernel/library.systems)
        #[arg(long)]
        library_path: Option<PathBuf>,
        /// Inspect all files in a workspace directory with cross-file resolution
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Focus diagnostics on a specific file within workspace mode
        #[arg(long, requires = "workspace")]
        focus: Option<String>,
        /// Disable dependency source hydration in workspace mode
        #[arg(long, requires = "workspace")]
        no_workspace_deps: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init { name } => init::run(name.as_deref(), None),
        Commands::Info {
            manifest_path,
            json,
        } => info::run(manifest_path.as_deref(), json),
        Commands::Add {
            name,
            path,
            git,
            tag,
            branch,
            rev,
            kpar,
        } => add::run(
            &name,
            &add::DepSource {
                path,
                git,
                tag,
                branch,
                rev,
                kpar,
            },
        ),
        Commands::Remove { name } => remove::run(&name),
        Commands::Lock { force, quiet, json } => lock::run_with_options(force, quiet, json),
        Commands::Fetch { quiet, json } => fetch::run(quiet, json),
        Commands::Update { quiet, json } => update::run(quiet, json),
        Commands::Tree { quiet, json } => tree::run(quiet, json),
        Commands::Why { name, quiet, json } => why::run(&name, quiet, json),
        Commands::Cache { command } => cache::run(command),
        Commands::Package {
            manifest_path,
            output,
        } => package::run(manifest_path.as_deref(), output.as_deref()),
        Commands::Eval { expr } => eval::run(&expr),
        Commands::Check {
            file,
            overrides,
            json,
        } => check::run(&file, &overrides, json),
        Commands::Verify {
            case_name,
            file,
            overrides,
            json,
        } => verify::run(&case_name, &file, &overrides, json),
        Commands::Analysis {
            case_name,
            file,
            overrides,
            json,
        } => analysis::run(&case_name, &file, &overrides, json),
        Commands::TradeStudy {
            study_name,
            file,
            overrides,
            json,
        } => trade_study::run(&study_name, &file, &overrides, json),
        Commands::Simulate {
            sm_name,
            file,
            events,
            interactive,
            auto,
            trace,
            json,
        } => simulate::run(&sm_name, &file, &events, interactive, auto, trace, json),
        Commands::Run {
            action_name,
            file,
            trace,
            json,
        } => run::run(&action_name, &file, trace, json),
        Commands::Solve {
            file,
            overrides,
            rollup,
            sweep,
            json,
        } => solve::run(&file, &overrides, rollup.as_deref(), sweep.as_deref(), json),
        Commands::Trace {
            file,
            inject,
            json,
        } => trace::run(&file, &inject, json),
        Commands::Flow {
            flow_name,
            file,
            inject,
            json,
        } => flow::run(flow_name.as_deref(), &file, inject.as_deref(), json),
        Commands::Project { command } => project::run(command),
        Commands::Query { command } => query::run(command),
        Commands::Export { command } => export::run(command),
        #[cfg(feature = "server")]
        Commands::Serve { port, host } => serve::run(port, &host),
        Commands::Inspect {
            file,
            tokens,
            diagnostics,
            cst,
            json,
            no_stdlib,
            library_path,
            workspace,
            focus,
            no_workspace_deps,
        } => {
            let mode = if tokens {
                inspect::InspectMode::Tokens
            } else if diagnostics {
                inspect::InspectMode::Diagnostics
            } else if cst {
                inspect::InspectMode::Cst
            } else {
                inspect::InspectMode::All
            };
            let options = inspect::InspectOptions {
                use_stdlib: !no_stdlib,
                library_path,
                quiet: cli.quiet,
                force_progress: cli.force_progress
                    || std::env::var("SYSML_FORCE_PROGRESS").ok().as_deref() == Some("1"),
            };
            if let Some(ws_root) = workspace {
                inspect::run_workspace(
                    &ws_root,
                    focus.as_deref(),
                    mode,
                    json,
                    options,
                    !no_workspace_deps,
                )
            } else if let Some(file) = file {
                inspect::run(&file, mode, json, options)
            } else {
                Err(common::CliError::user(
                    "either a file path or --workspace is required",
                ))
            }
        }
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        process::exit(e.exit_code as i32);
    }
}
