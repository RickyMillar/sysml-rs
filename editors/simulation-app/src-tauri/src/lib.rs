use std::net::SocketAddr;
use std::sync::Arc;

use sysml_api::{create_router, AppState};
use sysml_service::{execute_command, SysmlService};

/// Tauri managed state — the single `SysmlService` instance shared
/// between `invoke` command handlers and the axum sidecar.
pub struct DesktopState {
    service: Arc<SysmlService>,
}

/// Loopback port for the axum sidecar (WebSocket session events, SSE, LSP).
/// Must match `TAURI_SIDECAR_PORT` in `src/shared/api/tauri-transport.ts`.
pub const SIDECAR_PORT: u16 = 8081;

/// Generic command dispatch over the `SysmlService` inventory.
///
/// All `POST /api/command` traffic from the frontend is routed here via
/// `invoke('sysml_command', { command, params })`.  The handler is synchronous
/// because every registered service command is sync (the salsa host runs behind
/// `std::sync::Mutex`, not tokio's).
#[tauri::command]
fn sysml_command(
    state: tauri::State<'_, DesktopState>,
    command: String,
    params: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let body = params.unwrap_or(serde_json::Value::Null);
    execute_command(&state.service, &command, body).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let service = Arc::new(SysmlService::empty());
    let sidecar_service = Arc::clone(&service);

    if let Err(e) = tauri::Builder::default()
        .manage(DesktopState { service })
        .setup(move |_app| {
            // Spawn the axum sidecar for WebSocket session events, SSE progress,
            // and the Monaco LSP WebSocket — channels that `invoke` cannot serve.
            let app_state = Arc::new(AppState::with_service(sidecar_service));
            let router = create_router(app_state);
            let addr = SocketAddr::from(([127, 0, 0, 1], SIDECAR_PORT));
            tauri::async_runtime::spawn(async move {
                match tokio::net::TcpListener::bind(addr).await {
                    Ok(listener) => {
                        if let Err(e) = axum::serve(listener, router).await {
                            tracing::error!(error = %e, "axum sidecar serve error");
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "axum sidecar bind error"),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![sysml_command])
        .run(tauri::generate_context!())
    {
        tracing::error!(error = %e, "tauri runtime error");
    }
}
