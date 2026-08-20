// This is a binary entry point: `--version` / `--help` must reach stdout and
// argument errors must reach stderr, before any logging layer exists.
#![allow(clippy::print_stderr, clippy::print_stdout)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sysml_lsp_server::run_stdio;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

static TRACING_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("rs", "sysml", "sysml-lsp")
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/tmp/sysml-rs"))
}

/// Confirm the log file can actually be created before handing the path to
/// `tracing_appender`, whose `rolling::never` panics on an unwritable target.
fn probe_log_file(dir: &Path, file_name: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(file_name))
        .map(|_| ())
}

/// Initialise tracing and return a human-readable description of where logs go.
fn init_tracing() -> String {
    let cache_dir = cache_dir();
    let log_path = cache_dir.join("lsp.log");

    let (writer, destination) = match probe_log_file(&cache_dir, "lsp.log") {
        Ok(()) => {
            let file_appender = tracing_appender::rolling::never(&cache_dir, "lsp.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
            let _ = TRACING_GUARD.set(guard);
            (
                BoxMakeWriter::new(non_blocking),
                log_path.display().to_string(),
            )
        }
        Err(err) => {
            // Degrade rather than die: an editor still gets a working server.
            // stderr is the only safe console here — stdout carries LSP frames.
            eprintln!(
                "sysml-lsp-server: cannot write log file {}: {err} — logging to stderr instead",
                log_path.display()
            );
            (
                BoxMakeWriter::new(std::io::stderr),
                "stderr (fallback)".to_owned(),
            )
        }
    };

    let env_filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|v| EnvFilter::try_new(v).ok())
        .or_else(|| {
            EnvFilter::try_new(
                "sysml_lsp_server=info,sysml_core=warn,sysml_text=warn,\
                 sysml_text_pest=warn,sysml_ts=warn,sysml_runtime=warn,tower_lsp=warn",
            )
            .ok()
        })
        .unwrap_or_else(|| EnvFilter::new("info"));

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_file(true)
                .with_line_number(true)
                .compact()
                .with_writer(writer),
        )
        .try_init();

    destination
}

fn install_panic_hook() {
    // Install panic hook that writes to a log file.
    // Without this, panics in an LSP server vanish because stderr is not
    // captured by most editors (including Zed).
    std::panic::set_hook(Box::new(|info| {
        let cache_dir = cache_dir();
        let log_path = cache_dir.join("lsp-panic.log");
        let _ = std::fs::create_dir_all(&cache_dir);
        let msg = format!("[panic] {info}\n");
        // Append to file (best-effort)
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = f.write_all(msg.as_bytes());
            let bt = std::backtrace::Backtrace::force_capture();
            let _ = writeln!(f, "{bt}");
        }
        tracing::error!(panic_info = %info, "sysml-lsp-server panicked");
        eprintln!("sysml-lsp-server panic: {info}");
    }));
}

fn help_text() -> String {
    format!(
        "sysml-lsp-server {version}
Language Server Protocol implementation for SysML v2 and KerML.

USAGE:
    sysml-lsp-server [--version | --help]

With no arguments the process speaks LSP over stdin/stdout and runs until the
client closes the connection. It is normally launched by an editor (VS Code,
Zed, …) rather than invoked by hand — running it in a terminal will look like
it has hung, because it is waiting for LSP frames on stdin.

OPTIONS:
    -V, --version    Print version and exit
    -h, --help       Print this help and exit

ENVIRONMENT:
    RUST_LOG         tracing-subscriber filter (default:
                     sysml_lsp_server=info, dependencies at warn)

LOGS:
    {log_path}
    Panics are appended to lsp-panic.log alongside it. If that directory is not
    writable the server logs to stderr instead and keeps serving.
",
        version = env!("CARGO_PKG_VERSION"),
        log_path = cache_dir().join("lsp.log").display(),
    )
}

/// Handle argv before anything else runs. Returns only when the process should
/// go on to serve LSP over stdio; every other case exits here.
fn handle_args() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(first) = args.first() else {
        return;
    };

    if args.len() > 1 {
        eprintln!(
            "sysml-lsp-server: expected at most one argument, got {}",
            args.len()
        );
        eprintln!("Run `sysml-lsp-server --help` for usage.");
        std::process::exit(2);
    }

    match first.as_str() {
        "--version" | "-V" => {
            println!("sysml-lsp-server {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        "--help" | "-h" => {
            print!("{}", help_text());
            std::process::exit(0);
        }
        // Fail hard: an unrecognised flag must never silently become a server.
        other => {
            eprintln!("sysml-lsp-server: unrecognised argument: {other}");
            eprintln!("Run `sysml-lsp-server --help` for usage.");
            std::process::exit(2);
        }
    }
}

#[tokio::main]
async fn main() {
    handle_args();
    let log_destination = init_tracing();
    install_panic_hook();
    let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "<default>".to_owned());
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        pid = std::process::id(),
        rust_log = %rust_log,
        log_path = %log_destination,
        "starting sysml-lsp-server"
    );
    run_stdio().await;
}
