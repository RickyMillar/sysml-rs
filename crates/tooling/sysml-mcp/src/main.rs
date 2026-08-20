//! MCP server binary for SysML v2 model intelligence.
//!
//! Communicates via JSON-RPC over stdio. All logging goes to stderr.

use std::sync::Arc;
use sysml_service::SysmlService;

#[tokio::main]
async fn main() {
    // All logging to stderr — stdout is the MCP transport.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                // Infallible: parsing a static directive string
                .add_directive({
                    #[allow(clippy::unwrap_used)]
                    let d = "sysml_mcp=info".parse().unwrap();
                    d
                }),
        )
        .init();

    tracing::info!("sysml-mcp server starting");

    let service = Arc::new(SysmlService::empty());

    // Periodic reaper for expired sessions. Lives in sysml-service (see
    // S2.T15) so all transports share the same cadence + drop semantics.
    // Holds a Weak on the service, so the task self-terminates when this
    // binary's strong Arc drops (e.g. MCP client disconnects). The
    // returned AbortHandle is not retained — the reaper runs for the
    // lifetime of the process.
    let _reaper = sysml_service::session_reaper::spawn_session_reaper(&service);

    if let Err(e) = sysml_mcp::serve(service).await {
        tracing::error!("server error: {e}");
        std::process::exit(1);
    }
}
