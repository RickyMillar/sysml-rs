use std::io::{self, BufRead, IsTerminal as _, Write as _};
use std::path::Path;

use sysml_service::execution::RuntimeSession;
use sysml_service::SysmlService;

use crate::common::CliError;

// ── RuntimeSession helpers ─────────────────────────────────────────
//
// These extract SM-level introspection data from the unified
// `RuntimeSession` (which wraps an `Orchestrator` with `Box<dyn Executor>`
// subsystems) without requiring a downcast to `StateMachineRunner`.

/// Get the current state name from the first subsystem.
fn session_current_state(session: &RuntimeSession) -> &str {
    session
        .orchestrator
        .subsystems()
        .first()
        .map(|s| s.executor.current_state_name())
        .unwrap_or("")
}

/// Get the SM name from the first subsystem.
fn session_name(session: &RuntimeSession) -> &str {
    session
        .orchestrator
        .subsystems()
        .first()
        .map(|s| s.name.as_str())
        .unwrap_or("sm")
}

/// Get available transitions `(event, target)` from the last snapshot's
/// subsystem state. Returns an empty vec if no history exists yet.
fn session_available_transitions(session: &RuntimeSession) -> Vec<(String, String)> {
    let sm_name = session_name(session);
    session
        .history()
        .back()
        .and_then(|snap| snap.subsystem_states.get(sm_name))
        .map(|st| st.available_transitions.clone())
        .unwrap_or_default()
}

/// Get all state names from the first subsystem's `TransitionIR` list.
fn session_all_states(session: &RuntimeSession) -> Vec<String> {
    let transitions = match session
        .orchestrator
        .subsystems()
        .first()
        .and_then(|s| s.executor.transitions())
    {
        Some(t) => t,
        None => return Vec::new(),
    };
    let mut states: Vec<String> = Vec::new();
    for t in transitions {
        if !states.iter().any(|s| s == &t.from) {
            states.push(t.from.clone());
        }
        if !states.iter().any(|s| s == &t.to) {
            states.push(t.to.clone());
        }
    }
    states
}

/// Get all transitions as `(from, event, guard, to)` from the first subsystem.
fn session_all_transitions(
    session: &RuntimeSession,
) -> Vec<(String, Option<String>, Option<String>, String)> {
    session
        .orchestrator
        .subsystems()
        .first()
        .and_then(|s| s.executor.transitions())
        .map(|trs| {
            trs.iter()
                .map(|t| {
                    (
                        t.from.clone(),
                        t.event.clone(),
                        t.guard.clone(),
                        t.to.clone(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Run `sysml simulate <sm_name> <file> [--events e1,e2] [--interactive] [--auto] [--trace] [--json]`.
pub fn run(
    sm_name: &str,
    file: &Path,
    events: &Option<String>,
    interactive: bool,
    auto: bool,
    trace: bool,
    json: bool,
) -> Result<(), CliError> {
    let service = SysmlService::from_file(file)?;
    let uri = file.to_string_lossy();

    // Use service to compile and start the simulation session.
    let (session_key_id, _initial) = service.simulate_start(&uri, sm_name)?;
    let session_key = session_key_id.to_string();

    let color = !json && std::io::stdout().is_terminal();

    // Access the simulation session for introspection (available transitions, states, etc.).
    let simulations = service.sessions();

    if !json {
        let session = simulations.get(&session_key_id).expect("just created");
        print_header(session_name(&session), session_current_state(&session), color);
        if interactive {
            print_machine_overview_from_session(&session, color);
            print_interactive_help(color);
            print_available_or_hint_from_session(&session, color);
        } else {
            print_available_from_session(&session, color);
        }
        drop(session);
    }

    if interactive {
        run_interactive(&service, &session_key, trace, json, color)
    } else if auto {
        run_auto(&service, &session_key, trace, json, color)
    } else if let Some(events_str) = events {
        let event_list: Vec<&str> = events_str.split(',').map(|s| s.trim()).collect();
        run_with_events(&service, &session_key, &event_list, trace, json, color)
    } else {
        // No events: just show initial state.
        if json {
            let session = simulations.get(&session_key_id).expect("session exists");
            let output = serde_json::json!({
                "state_machine": session_name(&session),
                "initial_state": session_current_state(&session),
                "steps": [],
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        } else {
            println!("(no events provided; use --events, --interactive, or --auto)");
        }
        Ok(())
    }
}

// ── Formatting helpers ──────────────────────────────────────────────

fn print_header(sm_name: &str, initial_state: &str, color: bool) {
    if color {
        println!(
            "\x1b[1m\x1b[36mState Machine:\x1b[0m {} \x1b[90m|\x1b[0m \x1b[1mInitial:\x1b[0m {}",
            sm_name, initial_state
        );
    } else {
        println!("State Machine: {} | Initial: {}", sm_name, initial_state);
    }
    println!();
}

fn print_available_from_session(
    session: &RuntimeSession,
    color: bool,
) {
    let transitions = session_available_transitions(session);
    if transitions.is_empty() {
        return;
    }
    let hints: Vec<String> = transitions
        .iter()
        .map(|(event, target)| {
            if color {
                format!("{event} \x1b[90m(-> {target})\x1b[0m")
            } else {
                format!("{event} (-> {target})")
            }
        })
        .collect();
    if color {
        println!("  \x1b[33mAvailable:\x1b[0m {}", hints.join(", "));
    } else {
        println!("  Available: {}", hints.join(", "));
    }
}

fn print_available_or_hint_from_session(
    session: &RuntimeSession,
    color: bool,
) {
    let available = session_available_transitions(session);
    if !available.is_empty() {
        print_available_from_session(session, color);
        return;
    }

    let total = session_all_transitions(session).len();
    if color {
        if total == 0 {
            println!("  \x1b[33mNo compiled transitions found.\x1b[0m");
        } else {
            println!(
                "  \x1b[33mNo transitions available from current state '{}'.\x1b[0m",
                session_current_state(session)
            );
        }
    } else if total == 0 {
        println!("  No compiled transitions found.");
    } else {
        println!(
            "  No transitions available from current state '{}'.",
            session_current_state(session)
        );
    }
}

fn print_machine_overview_from_session(
    session: &RuntimeSession,
    color: bool,
) {
    let states = session_all_states(session);
    let transitions = session_all_transitions(session);

    if color {
        println!(
            "  \x1b[33mStates ({})\x1b[0m: {}",
            states.len(),
            states.join(", ")
        );
    } else {
        println!("  States ({}): {}", states.len(), states.join(", "));
    }

    if transitions.is_empty() {
        if color {
            println!("  \x1b[33mTransitions (0)\x1b[0m");
        } else {
            println!("  Transitions (0)");
        }
        return;
    }

    if color {
        println!("  \x1b[33mTransitions ({})\x1b[0m:", transitions.len());
    } else {
        println!("  Transitions ({}):", transitions.len());
    }

    for (from, event, guard, to) in transitions.iter().take(12) {
        let trigger = event.as_deref().unwrap_or("auto");
        if let Some(g) = guard {
            println!("    {from} -[{trigger} if {g}]-> {to}");
        } else {
            println!("    {from} -[{trigger}]-> {to}");
        }
    }
    if transitions.len() > 12 {
        println!("    ...");
    }
}

fn print_interactive_help(color: bool) {
    if color {
        println!(
            "  \x1b[33mCommands\x1b[0m: help, events, states, transitions, status, step, quit"
        );
    } else {
        println!("  Commands: help, events, states, transitions, status, step, quit");
    }
}

fn print_prompt(current_state: &str, stdout: &mut io::Stdout, color: bool) {
    if color {
        print!("\x1b[36m{}\x1b[0m> ", current_state);
    } else {
        print!("{}> ", current_state);
    }
    stdout.flush().ok();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractiveInput<'a> {
    Event(&'a str),
    Help,
    Available,
    States,
    Transitions,
    Status,
    Step,
    Quit,
}

fn parse_interactive_input<'a>(input: &'a str, available: &[&str]) -> InteractiveInput<'a> {
    let token = input.trim();
    if token.is_empty() {
        return InteractiveInput::Quit;
    }

    // Prefer actual available events first, so event names can overlap with command words.
    if available.iter().any(|ev| *ev == token) {
        return InteractiveInput::Event(token);
    }

    match token {
        "?" | "help" | ":help" | ":h" => InteractiveInput::Help,
        "events" | "available" | ":events" | ":available" => InteractiveInput::Available,
        "states" | ":states" => InteractiveInput::States,
        "transitions" | "graph" | ":transitions" | ":graph" => InteractiveInput::Transitions,
        "status" | "where" | ":status" | ":where" => InteractiveInput::Status,
        "step" | ":step" => InteractiveInput::Step,
        "quit" | "exit" | ":quit" | ":q" => InteractiveInput::Quit,
        _ => InteractiveInput::Event(token),
    }
}

fn print_step(
    prev: &str,
    event: &str,
    new_state: &str,
    outputs: &[String],
    trace: bool,
    color: bool,
) {
    if color {
        println!("  \x1b[90m{prev}\x1b[0m \x1b[1m-[{event}]->\x1b[0m \x1b[32m{new_state}\x1b[0m");
    } else {
        println!("  {prev} -[{event}]-> {new_state}");
    }
    if trace {
        for output in outputs {
            if color {
                println!("    \x1b[90m{output}\x1b[0m");
            } else {
                println!("    {output}");
            }
        }
    }
}

// ── Run modes ───────────────────────────────────────────────────────

fn run_with_events(
    service: &SysmlService,
    session_key: &str,
    events: &[&str],
    trace: bool,
    json: bool,
    color: bool,
) -> Result<(), CliError> {
    let mut steps = Vec::new();
    let simulations = service.sessions();

    for event in events {
        let prev = {
            let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
            session_current_state(&session).to_owned()
        };

        let result = service.simulate_step(session_key, Some(event))?;

        if json {
            steps.push(serde_json::json!({
                "event": event,
                "from": prev,
                "to": result.state,
                "outputs": result.outputs,
                "completed": result.completed,
            }));
        } else {
            print_step(&prev, event, &result.state, &result.outputs, trace, color);
            if !result.completed {
                let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
                print_available_from_session(&session, color);
            }
        }

        if result.completed {
            if !json {
                if color {
                    println!("  \x1b[1;32m(completed)\x1b[0m");
                } else {
                    println!("  (completed)");
                }
            }
            break;
        }
    }

    if json {
        let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
        let output = serde_json::json!({
            "final_state": session_current_state(&session),
            "completed": session.orchestrator.is_completed(),
            "steps": steps,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    }

    Ok(())
}

fn run_interactive(
    service: &SysmlService,
    session_key: &str,
    trace: bool,
    json: bool,
    color: bool,
) -> Result<(), CliError> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let simulations = service.sessions();

    if !json {
        println!("interactive mode (type an event or command; empty line to quit)");
        println!();
        let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
        print_prompt(session_current_state(&session), &mut stdout, color);
        drop(session);
    }

    for line in stdin.lock().lines() {
        let line = line.map_err(|e| CliError::internal(format!("stdin error: {e}")))?;
        let input = line.trim();

        let (available_labels, current_state) = {
            let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
            let available = session_available_transitions(&session);
            let labels: Vec<String> = available.iter().map(|(ev, _)| ev.clone()).collect();
            let state = session_current_state(&session).to_owned();
            (labels, state)
        };
        let label_refs: Vec<&str> = available_labels.iter().map(|s| s.as_str()).collect();

        match parse_interactive_input(input, &label_refs) {
            InteractiveInput::Quit => break,
            InteractiveInput::Help => {
                if !json {
                    let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
                    print_interactive_help(color);
                    print_available_or_hint_from_session(&session, color);
                    print_prompt(session_current_state(&session), &mut stdout, color);
                }
                continue;
            }
            InteractiveInput::Available => {
                if !json {
                    let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
                    print_available_or_hint_from_session(&session, color);
                    print_prompt(session_current_state(&session), &mut stdout, color);
                }
                continue;
            }
            InteractiveInput::States => {
                if !json {
                    let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
                    let states = session_all_states(&session);
                    println!("  States ({}): {}", states.len(), states.join(", "));
                    print_prompt(session_current_state(&session), &mut stdout, color);
                }
                continue;
            }
            InteractiveInput::Transitions => {
                if !json {
                    let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
                    let transitions = session_all_transitions(&session);
                    if transitions.is_empty() {
                        println!("  Transitions (0)");
                    } else {
                        println!("  Transitions ({}):", transitions.len());
                        for (from, event, guard, to) in &transitions {
                            let trigger = event.as_deref().unwrap_or("auto");
                            if let Some(g) = guard {
                                println!("    {from} -[{trigger} if {g}]-> {to}");
                            } else {
                                println!("    {from} -[{trigger}]-> {to}");
                            }
                        }
                    }
                    print_prompt(session_current_state(&session), &mut stdout, color);
                }
                continue;
            }
            InteractiveInput::Status => {
                if !json {
                    let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
                    println!("  Current state: {}", session_current_state(&session));
                    println!("  Completed: {}", session.orchestrator.is_completed());
                    print_available_or_hint_from_session(&session, color);
                    print_prompt(session_current_state(&session), &mut stdout, color);
                }
                continue;
            }
            InteractiveInput::Step => {
                let prev = current_state.clone();
                let result = service.simulate_step(session_key, None)?;

                if json {
                    let step = serde_json::json!({
                        "event": null,
                        "from": prev,
                        "to": result.state,
                        "outputs": result.outputs,
                        "completed": result.completed,
                    });
                    println!("{}", serde_json::to_string_pretty(&step).unwrap());
                } else {
                    print_step(&prev, "auto", &result.state, &result.outputs, trace, color);
                }

                if result.completed {
                    if !json {
                        if color {
                            println!("  \x1b[1;32m(completed)\x1b[0m");
                        } else {
                            println!("  (completed)");
                        }
                    }
                    break;
                }

                if !json {
                    let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
                    print_available_or_hint_from_session(&session, color);
                    print_prompt(session_current_state(&session), &mut stdout, color);
                }
                continue;
            }
            InteractiveInput::Event(event) => {
                let prev = current_state.clone();
                let known_event = label_refs.iter().any(|ev| *ev == event);
                let result = service.simulate_step(session_key, Some(event))?;

                if json {
                    let step = serde_json::json!({
                        "event": event,
                        "from": prev,
                        "to": result.state,
                        "outputs": result.outputs,
                        "completed": result.completed,
                    });
                    println!("{}", serde_json::to_string_pretty(&step).unwrap());
                } else {
                    print_step(&prev, event, &result.state, &result.outputs, trace, color);
                    if !known_event && prev == result.state {
                        println!(
                            "  Unknown or inactive event '{event}' for current state '{}'.",
                            prev
                        );
                    }
                }

                if result.completed {
                    if !json {
                        if color {
                            println!("  \x1b[1;32m(completed)\x1b[0m");
                        } else {
                            println!("  (completed)");
                        }
                    }
                    break;
                }

                if !json {
                    let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
                    print_available_or_hint_from_session(&session, color);
                    print_prompt(session_current_state(&session), &mut stdout, color);
                }
            }
        }
    }

    Ok(())
}

fn run_auto(
    service: &SysmlService,
    session_key: &str,
    trace: bool,
    json: bool,
    color: bool,
) -> Result<(), CliError> {
    let mut steps = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let max_steps = 100; // safety limit
    let simulations = service.sessions();

    if !json {
        if color {
            println!("  \x1b[33mauto-demo mode\x1b[0m");
        } else {
            println!("  auto-demo mode");
        }
        println!();
    }

    for _ in 0..max_steps {
        let (current, event_owned) = {
            let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
            let current = session_current_state(&session).to_owned();
            let transitions = session_available_transitions(&session);

            if transitions.is_empty() {
                break;
            }

            // Pick the first unvisited transition, or the first one if all visited.
            let (event, _target) = transitions
                .iter()
                .find(|(ev, _)| !visited.contains(ev.as_str()))
                .or_else(|| transitions.first())
                .cloned()
                .unwrap();

            visited.insert(event.clone());
            (current, event)
        };

        let result = service.simulate_step(session_key, Some(&event_owned))?;

        if json {
            steps.push(serde_json::json!({
                "event": event_owned,
                "from": current,
                "to": result.state,
                "outputs": result.outputs,
                "completed": result.completed,
            }));
        } else {
            print_step(
                &current,
                &event_owned,
                &result.state,
                &result.outputs,
                trace,
                color,
            );
        }

        if result.completed {
            if !json {
                if color {
                    println!("  \x1b[1;32m(completed)\x1b[0m");
                } else {
                    println!("  (completed)");
                }
            }
            break;
        }

        // Stop when we've cycled back to a state we've already been in with no new transitions.
        let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
        let new_transitions = session_available_transitions(&session);
        if new_transitions.iter().all(|(ev, _)| visited.contains(ev.as_str())) {
            // One more step to show the cycle, then stop.
            if !json && !new_transitions.is_empty() {
                print_available_from_session(&session, color);
                if color {
                    println!("  \x1b[90m(cycle detected, stopping)\x1b[0m");
                } else {
                    println!("  (cycle detected, stopping)");
                }
            }
            break;
        }
    }

    if json {
        let session = simulations.get(&sysml_service::ElementId::from_string(session_key)).expect("session exists");
        let output = serde_json::json!({
            "final_state": session_current_state(&session),
            "completed": session.orchestrator.is_completed(),
            "steps": steps,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    }

    Ok(())
}
