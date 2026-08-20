//! Code lens handler.

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

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{CodeLensParams, CodeLens, Range, Position, Command};

use crate::aggregation;
use crate::evaluation;
use crate::utils::offset_to_position;
use crate::SysmlLanguageServer;

pub(crate) async fn code_lens(
    server: &SysmlLanguageServer,
    params: CodeLensParams,
) -> Result<Option<Vec<CodeLens>>> {
    let uri = params.text_document.uri.to_string();

    let Some(doc) = server.salsa_doc(&uri).await else {
        return Ok(None);
    };

    let mut lenses = Vec::new();

    // Always-visible debug lenses for quick triage in editor UX.
    let debug_anchor = Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 0,
        },
    };
    lenses.push(CodeLens {
        range: debug_anchor,
        command: Some(Command {
            title: "Debug Status".to_owned(),
            command: "sysml.debug.status".to_owned(),
            arguments: None,
        }),
        data: None,
    });
    lenses.push(CodeLens {
        range: debug_anchor,
        command: Some(Command {
            title: "Debug Bundle".to_owned(),
            command: "sysml.debug.bundle".to_owned(),
            arguments: None,
        }),
        data: None,
    });

    // Constraint code lenses (PASS/FAIL)
    for result in evaluation::evaluate_constraints(&doc.graph) {
        let span = match &result.span {
            Some(s) if s.file == uri => s,
            _ => continue,
        };

        let start = offset_to_position(span.start, &doc.content);
        let end = offset_to_position(span.end.min(doc.content.len()), &doc.content);

        // Stash the structured expression AST in `data` so the VS Code
        // client can render math (KaTeX) in the lens title via
        // `editors/expression-view`. Phase 6B.4.
        let data = doc
            .graph
            .get_element(&result.element_id)
            .and_then(|e| sysml_service::expression_ast::project_owner(e, &doc.graph))
            .and_then(|r| r.ast)
            .and_then(|ast| serde_json::to_value(ast).ok());

        lenses.push(CodeLens {
            range: Range { start, end },
            command: Some(Command {
                title: result.detail.clone(),
                command: String::new(),
                arguments: None,
            }),
            data,
        });
    }

    // Solver propagation lenses — show computed values from binding connectors
    {
        let mut network = sysml_runtime::solver::build_constraint_network(&doc.graph);
        let result = network.propagate();
        if !result.solved.is_empty() {
            // Show a summary lens at the top of the file
            let solved_summary: Vec<String> = result
                .solved
                .iter()
                .filter(|(k, _)| k.as_str() != "t_ms" && k.as_str() != "tick")
                .take(5)
                .map(|(k, v)| format!("{}={:?}", k, v))
                .collect();
            if !solved_summary.is_empty() {
                let display = format!("solved: {}", solved_summary.join(", "));
                lenses.push(CodeLens {
                    range: Range {
                        start: Position { line: 0, character: 0 },
                        end: Position { line: 0, character: 0 },
                    },
                    command: Some(Command {
                        title: display,
                        command: String::new(),
                        arguments: None,
                    }),
                    data: None,
                });
            }
        }
    }

    // Verification case code lenses (Run Verification / verdict)
    for result in evaluation::evaluate_verification_cases(&doc.graph) {
        let span = match &result.span {
            Some(s) if s.file == uri => s,
            _ => continue,
        };

        let start = offset_to_position(span.start, &doc.content);
        let end = offset_to_position(span.end.min(doc.content.len()), &doc.content);

        // "Run Verification" clickable lens
        lenses.push(CodeLens {
            range: Range { start, end },
            command: Some(Command {
                title: "Run Verification".to_owned(),
                command: "sysml.verify".to_owned(),
                arguments: Some(vec![
                    serde_json::json!(uri),
                    serde_json::json!(result.case_name),
                ]),
            }),
            data: None,
        });

        // Verdict lens (PASS/FAIL/etc.)
        lenses.push(CodeLens {
            range: Range { start, end },
            command: Some(Command {
                title: result.display,
                command: String::new(),
                arguments: None,
            }),
            data: None,
        });
    }

    // Analysis case code lenses (Run Analysis / solver output)
    for result in evaluation::evaluate_analysis_cases(&doc.graph) {
        let span = match &result.span {
            Some(s) if s.file == uri => s,
            _ => continue,
        };

        let start = offset_to_position(span.start, &doc.content);
        let end = offset_to_position(span.end.min(doc.content.len()), &doc.content);

        // "Run Analysis" clickable lens
        lenses.push(CodeLens {
            range: Range { start, end },
            command: Some(Command {
                title: "▶ Run Analysis".to_owned(),
                command: "sysml.analysis.run".to_owned(),
                arguments: Some(vec![
                    serde_json::json!(uri),
                    serde_json::json!(result.case_name),
                ]),
            }),
            data: None,
        });

        // Result status lens
        lenses.push(CodeLens {
            range: Range { start, end },
            command: Some(Command {
                title: result.display.clone(),
                command: String::new(),
                arguments: None,
            }),
            data: None,
        });
    }

    // Calculation code lenses (= value)
    for (_, span, display) in evaluation::evaluate_calculations(&doc.graph) {
        let span = match &span {
            Some(s) if s.file == uri => s,
            _ => continue,
        };

        let start = offset_to_position(span.start, &doc.content);
        let end = offset_to_position(span.end.min(doc.content.len()), &doc.content);

        lenses.push(CodeLens {
            range: Range { start, end },
            command: Some(Command {
                title: display,
                command: String::new(),
                arguments: None,
            }),
            data: None,
        });
    }

    // Physics domain code lenses on PortDefinitions
    {
        // Salsa-cached registry (ADR-011 §3 / S3.T11). Falls back to the
        // hardcoded baseline if the workspace lookup fails (e.g. file not
        // yet registered with the host).
        let physics_registry = server
            .service
            .workspace_physics_registry()
            .map(|arc| (*arc).clone())
            .unwrap_or_else(|_| sysml_core::physics::PhysicsDomainRegistry::new());
        for element in doc.graph.elements.values() {
            if element.kind != sysml_core::ElementKind::PortDefinition {
                continue;
            }
            let port_name = match &element.name {
                Some(n) => n.clone(),
                None => continue,
            };
            let Some(span) = element.spans.iter().find(|s| s.file == uri) else {
                continue;
            };

            let classification = sysml_core::physics::classify::classify_port_definition(
                &port_name,
                &doc.graph,
                &physics_registry,
            );
            let Some(domain) = classification.domain else {
                continue;
            };

            let effort_names: Vec<&str> = classification
                .features
                .iter()
                .filter(|f| f.role == sysml_core::physics::VariableRole::Effort)
                .map(|f| f.name.as_str())
                .collect();
            let flow_names: Vec<&str> = classification
                .features
                .iter()
                .filter(|f| f.role == sysml_core::physics::VariableRole::Flow)
                .map(|f| f.name.as_str())
                .collect();

            let effort_str = if effort_names.is_empty() {
                "none".to_owned()
            } else {
                effort_names.join(", ")
            };
            let flow_str = if flow_names.is_empty() {
                "none".to_owned()
            } else {
                flow_names.join(", ")
            };

            let title = format!(
                "Physics: {} (effort: {}, flow: {})",
                domain, effort_str, flow_str,
            );

            let start = offset_to_position(span.start, &doc.content);
            let end = offset_to_position(span.end.min(doc.content.len()), &doc.content);

            lenses.push(CodeLens {
                range: Range { start, end },
                command: Some(Command {
                    title,
                    command: String::new(),
                    arguments: None,
                }),
                data: None,
            });
        }
    }

    // State machine "Simulate" code lenses
    for element in doc.graph.elements.values() {
        if element.kind != sysml_core::ElementKind::StateDefinition {
            continue;
        }
        let sm_name = match &element.name {
            Some(n) => n.clone(),
            None => continue,
        };
        let Some(span) = element.spans.iter().find(|s| s.file == uri) else {
            continue;
        };

        let start = offset_to_position(span.start, &doc.content);
        let end = offset_to_position(span.end.min(doc.content.len()), &doc.content);

        lenses.push(CodeLens {
            range: Range { start, end },
            command: Some(Command {
                title: "Simulate".to_owned(),
                command: "sysml.simulate.start".to_owned(),
                arguments: Some(vec![serde_json::json!(uri), serde_json::json!(sm_name)]),
            }),
            data: None,
        });
    }

    // --- "Run Action" code lens on ActionDefinition / ActionUsage ---
    for element in doc.graph.elements.values() {
        if !matches!(
            element.kind,
            sysml_core::ElementKind::ActionDefinition | sysml_core::ElementKind::ActionUsage
        ) {
            continue;
        }
        let action_name = match &element.name {
            Some(n) => n.clone(),
            None => continue,
        };
        let Some(span) = element.spans.iter().find(|s| s.file == uri) else {
            continue;
        };

        let start = offset_to_position(span.start, &doc.content);
        let end = offset_to_position(span.end.min(doc.content.len()), &doc.content);

        lenses.push(CodeLens {
            range: Range { start, end },
            command: Some(Command {
                title: "Run Action".to_owned(),
                command: "sysml.action.run".to_owned(),
                arguments: Some(vec![serde_json::json!(uri), serde_json::json!(action_name)]),
            }),
            data: None,
        });
    }

    // Numeric attribute "what-if" exploration lenses.
    let whatif_defaults =
        |value: &sysml_core::Value| -> Option<(serde_json::Value, f64, f64, usize)> {
            match value {
                sysml_core::Value::Int(i) => {
                    let magnitude = i.abs();
                    let delta = ((magnitude as f64) * 0.1).round() as i64;
                    let delta = delta.max(1);
                    let override_value = if *i >= 0 {
                        i.saturating_add(delta)
                    } else {
                        i.saturating_sub(delta)
                    };

                    let center = *i as f64;
                    let spread = if magnitude == 0 {
                        50.0
                    } else {
                        center.abs() * 0.5
                    };
                    let mut sweep_start = center - spread;
                    let mut sweep_end = center + spread;
                    if (sweep_end - sweep_start).abs() < f64::EPSILON {
                        sweep_end = sweep_start + 1.0;
                    }
                    if sweep_start > sweep_end {
                        std::mem::swap(&mut sweep_start, &mut sweep_end);
                    }

                    Some((
                        serde_json::json!(override_value),
                        sweep_start,
                        sweep_end,
                        11,
                    ))
                }
                sysml_core::Value::Float(f) => {
                    let abs = f.abs();
                    let delta = if abs < f64::EPSILON { 10.0 } else { abs * 0.1 };
                    let override_value = if *f >= 0.0 { f + delta } else { f - delta };

                    let spread = if abs < f64::EPSILON { 50.0 } else { abs * 0.5 };
                    let mut sweep_start = f - spread;
                    let mut sweep_end = f + spread;
                    if (sweep_end - sweep_start).abs() < f64::EPSILON {
                        sweep_end = sweep_start + 1.0;
                    }
                    if sweep_start > sweep_end {
                        std::mem::swap(&mut sweep_start, &mut sweep_end);
                    }

                    Some((
                        serde_json::json!(override_value),
                        sweep_start,
                        sweep_end,
                        11,
                    ))
                }
                _ => None,
            }
        };

    for element in doc.graph.elements.values() {
        if element.kind != sysml_core::ElementKind::AttributeUsage {
            continue;
        }

        let variable_name = match &element.name {
            Some(n) => n.clone(),
            None => continue,
        };
        let Some(value) = element.get_prop("value") else {
            continue;
        };
        let (override_value, sweep_start, sweep_end, sweep_steps) = match whatif_defaults(value) {
            Some(v) => v,
            None => continue,
        };

        let Some(span) = element.spans.iter().find(|s| s.file == uri) else {
            continue;
        };

        let start = offset_to_position(span.start, &doc.content);
        let end = offset_to_position(span.end.min(doc.content.len()), &doc.content);
        let line = start.line;
        let character = start.character;
        let lens_range = Range { start, end };

        lenses.push(CodeLens {
            range: lens_range,
            command: Some(Command {
                title: "What-If (+10%)".to_owned(),
                command: "sysml.whatif".to_owned(),
                arguments: Some(vec![
                    serde_json::json!(uri),
                    serde_json::json!(line),
                    serde_json::json!(character),
                    serde_json::json!(variable_name.clone()),
                    override_value,
                ]),
            }),
            data: None,
        });

        lenses.push(CodeLens {
            range: lens_range,
            command: Some(Command {
                title: "Sweep (+/-50%)".to_owned(),
                command: "sysml.whatif.sweep".to_owned(),
                arguments: Some(vec![
                    serde_json::json!(uri),
                    serde_json::json!(line),
                    serde_json::json!(character),
                    serde_json::json!(variable_name),
                    serde_json::json!(sweep_start),
                    serde_json::json!(sweep_end),
                    serde_json::json!(sweep_steps),
                ]),
            }),
            data: None,
        });
    }

    // F5: Aggregate satisfaction matrix code lenses
    for status in aggregation::aggregate_all_statuses(&doc.graph) {
        let span = match &status.owner_span {
            Some(s) if s.file == uri => s,
            _ => continue,
        };

        let lens_text = aggregation::format_aggregate_lens(&status);
        if lens_text.is_empty() {
            continue;
        }

        let start = offset_to_position(span.start, &doc.content);
        let end = offset_to_position(span.end.min(doc.content.len()), &doc.content);

        lenses.push(CodeLens {
            range: Range { start, end },
            command: Some(Command {
                title: lens_text,
                command: String::new(),
                arguments: None,
            }),
            data: None,
        });
    }

    if lenses.is_empty() {
        Ok(None)
    } else {
        Ok(Some(lenses))
    }
}
