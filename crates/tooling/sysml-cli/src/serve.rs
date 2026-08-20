use crate::common::CliError;

/// Run the SysML API server.
pub fn run(port: u16, host: &str) -> Result<(), CliError> {
    let addr = format!("{host}:{port}");

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::internal(format!("failed to create tokio runtime: {e}")))?;

    rt.block_on(async {
        eprintln!("starting SysML API server on {addr}");
        sysml_api::run_server(&addr)
            .await
            .map_err(|e| CliError::internal(format!("server error: {e}")))
    })
}
