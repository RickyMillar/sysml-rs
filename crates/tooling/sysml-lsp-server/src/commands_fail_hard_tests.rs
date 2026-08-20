//! Phase 0 — Fail-hard contract for the LSP-shell evaluate/whatif handlers.
//!
//! Locks the post-P4 contract before `handle_evaluate`,
//! `handle_whatif`, and `handle_whatif_sweep` get migrated onto
//! `ctx.get_resolved(uri)` (which fail-hards on a missing workspace).
//!
//! Today these handlers either fall back to the per-file library-only
//! resolve (open-coded match in `commands.rs`) or return
//! `error_json("document not found")`. Post-P4 the handler short-circuits
//! when no workspace is loaded and returns
//! `error_json("document not in a workspace; call load_workspace first")`.
//!
//! All tests are `#[ignore]`-tagged. They are inline (not in `tests/`)
//! because `test_harness::TestServer` is crate-private and the
//! `workspace/executeCommand` LSP surface is the right place to exercise
//! these handlers end-to-end. Phase P4 removes the ignore.
//!
//! Run opt-in:
//!   cargo test --release -p sysml-lsp-server commands_fail_hard -- --ignored --test-threads=2

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use serde_json::json;

use crate::test_harness::TestServer;

/// Substring the rewritten handler error must contain. Pins the post-P4
/// message body. If wording changes during P4, update here and re-run.
const FAIL_HARD_MSG_HINT: &str = "workspace";

fn extract_error_message(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("error")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned())
}

/// Assert that an executeCommand payload is a fail-hard error JSON whose
/// message hints at a missing workspace.
fn assert_workspace_fail_hard(command: &str, payload: Option<serde_json::Value>) {
    let payload = payload.unwrap_or_else(|| panic!("{command} should return a payload"));
    let msg = extract_error_message(&payload).unwrap_or_else(|| {
        panic!(
            "{command} should return {{\"error\": ...}} on missing workspace, got {payload:?}"
        )
    });
    assert!(
        msg.to_lowercase().contains(FAIL_HARD_MSG_HINT),
        "{command} error must hint at missing workspace (contain {FAIL_HARD_MSG_HINT:?}), got {msg:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_evaluate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn handle_evaluate_without_workspace_errors_cleanly() {
    let server = TestServer::new();
    server.initialize_full().await;
    // NB: no did_open — handler must fail-hard on a URI that was never
    // registered in any workspace.
    let bogus_uri = "file:///does/not/exist.sysml";
    let payload = server
        .execute_command(
            "sysml.evaluate",
            vec![json!(bogus_uri), json!(0_u32), json!(0_u32)],
        )
        .await;
    assert_workspace_fail_hard("sysml.evaluate", payload);
}

#[tokio::test]
async fn handle_whatif_without_workspace_errors_cleanly() {
    let server = TestServer::new();
    server.initialize_full().await;
    let bogus_uri = "file:///does/not/exist.sysml";
    // handle_whatif signature: uri, line, character, variable_name, override_value
    let payload = server
        .execute_command(
            "sysml.whatif",
            vec![
                json!(bogus_uri),
                json!(0_u32),
                json!(0_u32),
                json!("synthetic_variable"),
                json!(1.0_f64),
            ],
        )
        .await;
    assert_workspace_fail_hard("sysml.whatif", payload);
}

#[tokio::test]
async fn handle_whatif_sweep_without_workspace_errors_cleanly() {
    let server = TestServer::new();
    server.initialize_full().await;
    let bogus_uri = "file:///does/not/exist.sysml";
    // handle_whatif_sweep signature: uri, line, character, variable_name, start, end, steps
    let payload = server
        .execute_command(
            "sysml.whatif.sweep",
            vec![
                json!(bogus_uri),
                json!(0_u32),
                json!(0_u32),
                json!("synthetic_variable"),
                json!(0.0_f64),
                json!(1.0_f64),
                json!(5_u32),
            ],
        )
        .await;
    assert_workspace_fail_hard("sysml.whatif.sweep", payload);
}
