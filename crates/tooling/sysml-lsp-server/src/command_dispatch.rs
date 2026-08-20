//! Command dispatch table for LSP executeCommand.
//!
//! Maps command name strings to async handler functions, replacing the
//! monolithic match block in `execute_command`. The `sysml.cache.rebuild`
//! command is excluded because it requires the server handle.

// LSP server: tower-lsp patterns use unwrap/expect for client sends,
// indexing is bounds-checked by protocol invariants, Arc cloning is intentional.
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::manual_let_else,
    clippy::arc_with_non_send_sync,
    clippy::clone_on_ref_ptr,
    clippy::map_err_ignore,
    clippy::needless_pass_by_value,
    clippy::panic
)]

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::commands::{self, CommandContext};

/// Async command handler: takes a context and arguments, returns a JSON value.
type CommandHandler = for<'a> fn(
    &'a CommandContext<'a>,
    &'a [Value],
) -> Pin<Box<dyn Future<Output = Value> + Send + 'a>>;

/// Build the command dispatch table.
///
/// Returns a map from command name to handler function. Every registered
/// command except `sysml.cache.rebuild` is included here.
pub(crate) fn build_dispatch_table() -> HashMap<&'static str, CommandHandler> {
    let mut table: HashMap<&'static str, CommandHandler> = HashMap::with_capacity(43);

    // Cache commands
    table.insert("sysml.cache.clear", |ctx, _args| {
        Box::pin(commands::handle_cache_clear(ctx))
    });
    table.insert("sysml.cache.status", |ctx, _args| {
        Box::pin(commands::handle_cache_status(ctx))
    });

    // Debug commands
    table.insert("sysml.debug.status", |ctx, _args| {
        Box::pin(commands::handle_debug_status(ctx))
    });
    table.insert("sysml.debug.bundle", |ctx, _args| {
        Box::pin(commands::handle_debug_bundle(ctx))
    });

    // Evaluation commands
    table.insert("sysml.evaluate", |ctx, args| {
        Box::pin(commands::handle_evaluate(ctx, args))
    });
    table.insert("sysml.evaluate.all", |ctx, args| {
        Box::pin(commands::handle_evaluate_all(ctx, args))
    });
    table.insert("sysml.verify", |ctx, args| {
        Box::pin(commands::handle_verify(ctx, args))
    });

    // Analysis commands
    table.insert("sysml.analysis.run", |ctx, args| {
        Box::pin(commands::handle_analysis_run(ctx, args))
    });

    // Simulation commands
    table.insert("sysml.simulate.start", |ctx, args| {
        Box::pin(commands::handle_simulate_start(ctx, args))
    });
    table.insert("sysml.simulate.step", |ctx, args| {
        Box::pin(commands::handle_simulate_step(ctx, args))
    });
    table.insert("sysml.simulate.stop", |ctx, args| {
        Box::pin(commands::handle_simulate_stop(ctx, args))
    });
    table.insert("sysml.simulate.reset", |ctx, args| {
        Box::pin(commands::handle_simulate_reset(ctx, args))
    });

    // Orchestrator commands
    table.insert("sysml.orchestrate.start", |ctx, args| {
        Box::pin(commands::handle_orchestrate_start(ctx, args))
    });
    table.insert("sysml.orchestrate.step", |ctx, args| {
        Box::pin(commands::handle_orchestrate_step(ctx, args))
    });
    table.insert("sysml.orchestrate.inject", |ctx, args| {
        Box::pin(commands::handle_orchestrate_inject(ctx, args))
    });
    table.insert("sysml.orchestrate.stop", |ctx, args| {
        Box::pin(commands::handle_orchestrate_stop(ctx, args))
    });

    // Scenario commands
    table.insert("sysml.scenario.run", |ctx, args| {
        Box::pin(commands::handle_scenario_run(ctx, args))
    });

    // Monte Carlo commands
    table.insert("sysml.montecarlo.run", |ctx, args| {
        Box::pin(commands::handle_montecarlo_run(ctx, args))
    });

    // Timeline commands
    table.insert("sysml.timeline.getTrace", |ctx, args| {
        Box::pin(commands::handle_timeline_get_trace(ctx, args))
    });
    table.insert("sysml.timeline.getSnapshot", |ctx, args| {
        Box::pin(commands::handle_timeline_get_snapshot(ctx, args))
    });

    // Action commands
    table.insert("sysml.action.run", |ctx, args| {
        Box::pin(commands::handle_action_run(ctx, args))
    });
    table.insert("sysml.action.start", |ctx, args| {
        Box::pin(commands::handle_action_start(ctx, args))
    });
    table.insert("sysml.action.step", |ctx, args| {
        Box::pin(commands::handle_action_step(ctx, args))
    });
    table.insert("sysml.action.stop", |ctx, args| {
        Box::pin(commands::handle_action_stop(ctx, args))
    });
    table.insert("sysml.action.reset", |ctx, args| {
        Box::pin(commands::handle_action_reset(ctx, args))
    });
    table.insert("sysml.action.visualize", |ctx, args| {
        Box::pin(commands::handle_action_visualize(ctx, args))
    });

    // Unified session commands (session backend contract)
    table.insert("sysml.sessions.step", |ctx, args| {
        Box::pin(commands::handle_sessions_step(ctx, args))
    });
    table.insert("sysml.sessions.inject", |ctx, args| {
        Box::pin(commands::handle_sessions_inject(ctx, args))
    });

    // Flow visualization
    table.insert("sysml.flow.visualize", |ctx, args| {
        Box::pin(commands::handle_flow_visualize(ctx, args))
    });

    // What-if analysis
    table.insert("sysml.whatif", |ctx, args| {
        Box::pin(commands::handle_whatif(ctx, args))
    });
    table.insert("sysml.whatif.sweep", |ctx, args| {
        Box::pin(commands::handle_whatif_sweep(ctx, args))
    });
    table.insert("sysml.diagram.whatif", |ctx, args| {
        Box::pin(commands::handle_diagram_whatif(ctx, args))
    });

    // Salsa query stats
    table.insert("sysml.salsa.stats", |ctx, _args| {
        Box::pin(commands::handle_salsa_stats(ctx))
    });
    table.insert("sysml.salsa.stats.reset", |ctx, _args| {
        Box::pin(commands::handle_salsa_stats_reset(ctx))
    });

    // Workspace verification
    table.insert("sysml.project.info", |ctx, _args| {
        Box::pin(commands::handle_project_info(ctx))
    });
    table.insert("sysml.workspace.info", |ctx, _args| {
        Box::pin(commands::handle_workspace_info(ctx))
    });
    table.insert("sysml.dependency.status", |ctx, _args| {
        Box::pin(commands::handle_dependency_status(ctx))
    });
    table.insert("sysml.workspace.verify", |ctx, _args| {
        Box::pin(commands::handle_workspace_verify(ctx))
    });

    // Diagram commands
    table.insert("sysml.diagram.open", |ctx, args| {
        Box::pin(commands::handle_diagram_open(ctx, args))
    });
    table.insert("sysml.diagram.view", |ctx, args| {
        Box::pin(commands::handle_diagram_view(ctx, args))
    });
    table.insert("sysml.diagram.export", |ctx, args| {
        Box::pin(commands::handle_diagram_export(ctx, args))
    });
    table.insert("sysml.diagram.expand", |ctx, args| {
        Box::pin(commands::handle_diagram_expand(ctx, args))
    });
    table.insert("sysml.diagram.edit", |ctx, args| {
        Box::pin(commands::handle_diagram_edit(ctx, args))
    });

    // Model tree
    table.insert(
        "sysml.model.tree",
        (|ctx, args| Box::pin(commands::handle_model_tree(ctx, args))) as CommandHandler,
    );

    table
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_table_has_all_commands() {
        let table = build_dispatch_table();

        // Enumerate the full expected set rather than asserting on len() —
        // adding a command no longer rots the test, and a regression
        // surfaces with the missing-or-extra command named in the diff.
        // `sysml.cache.rebuild` is intentionally excluded (special-cased
        // in lib.rs, not part of the dispatch table).
        let expected: std::collections::BTreeSet<&str> = [
            "sysml.cache.clear",
            "sysml.cache.status",
            "sysml.debug.status",
            "sysml.debug.bundle",
            "sysml.evaluate",
            "sysml.evaluate.all",
            "sysml.verify",
            "sysml.analysis.run",
            "sysml.simulate.start",
            "sysml.simulate.step",
            "sysml.simulate.stop",
            "sysml.simulate.reset",
            "sysml.orchestrate.start",
            "sysml.orchestrate.step",
            "sysml.orchestrate.inject",
            "sysml.orchestrate.stop",
            "sysml.scenario.run",
            "sysml.montecarlo.run",
            "sysml.timeline.getTrace",
            "sysml.timeline.getSnapshot",
            "sysml.action.run",
            "sysml.action.start",
            "sysml.action.step",
            "sysml.action.stop",
            "sysml.action.reset",
            "sysml.action.visualize",
            "sysml.sessions.step",
            "sysml.sessions.inject",
            "sysml.flow.visualize",
            "sysml.whatif",
            "sysml.whatif.sweep",
            "sysml.diagram.whatif",
            "sysml.salsa.stats",
            "sysml.salsa.stats.reset",
            "sysml.project.info",
            "sysml.workspace.info",
            "sysml.dependency.status",
            "sysml.workspace.verify",
            "sysml.diagram.open",
            "sysml.diagram.view",
            "sysml.diagram.export",
            "sysml.diagram.expand",
            "sysml.diagram.edit",
            "sysml.model.tree",
        ]
        .into_iter()
        .collect();

        let actual: std::collections::BTreeSet<&str> =
            table.keys().copied().collect();

        assert_eq!(actual, expected);
        assert!(!table.contains_key("sysml.cache.rebuild"));
    }
}
