use crate::common::CliError;

/// Run the SysML API server.
pub fn run(port: u16, host: &str) -> Result<(), CliError> {
    let addr = format!("{host}:{port}");

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::internal(format!("failed to create tokio runtime: {e}")))?;

    rt.block_on(async {
        eprintln!("starting SysML API server on {addr}");
        if !is_loopback_host(host) {
            eprintln!("WARNING: binding {host} — this server is reachable beyond this machine.");
            if std::env::var_os("SYSML_API_TOKEN").is_none() {
                eprintln!("WARNING: SYSML_API_TOKEN is not set, so writes are UNAUTHENTICATED.");
            }
        }
        sysml_api::run_server(&addr)
            .await
            .map_err(|e| CliError::internal(format!("server error: {e}")))
    })
}

/// Does this bind host keep the server on this machine?
///
/// `0.0.0.0` and `::` bind every interface; anything else that is not an
/// explicit loopback name is treated as exposed, so an unrecognised host warns
/// rather than passing silently.
fn is_loopback_host(host: &str) -> bool {
    matches!(
        host.trim_start_matches('[').trim_end_matches(']'),
        "127.0.0.1" | "::1" | "localhost"
    )
}

#[cfg(test)]
mod tests {
    use super::is_loopback_host;

    #[test]
    fn loopback_hosts_are_recognised() {
        for h in ["127.0.0.1", "::1", "[::1]", "localhost"] {
            assert!(is_loopback_host(h), "{h} should be loopback");
        }
    }

    #[test]
    fn wildcard_and_routable_hosts_are_not_loopback() {
        // These are the binds that must produce the exposure warning.
        for h in ["0.0.0.0", "::", "192.168.1.10", "example.com"] {
            assert!(!is_loopback_host(h), "{h} should not be loopback");
        }
    }
}
