---
title: API and MCP command catalogue
description: Every native service command exposed over the HTTP API and as MCP tools, from the live /commands catalogue.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 11bd751
source_of_truth:
  - website/src/generated/service-commands.json
  - crates/tooling/sysml-service
---

<!--
GENERATED — do not edit.
Regenerate (with the artifact it renders, src/generated/service-commands.json) via:
  cd website && node scripts/generate-reference.mjs
-->

These are the native service commands of `sysml-api` version 0.1.0, captured from the server's own `GET /commands` catalogue — the live catalogue is authoritative. Each command is callable three ways:

- `POST /api/commands/<name>` with the parameters as a JSON body;
- `POST /api/command` with `{ "command": "<name>", "params": { ... } }`;
- as an MCP tool, where the tool name is the command name with dots replaced by underscores (`sysml.sessions.step` → `sysml_sessions_step`).

Transport, security posture, and client setup are covered in [Integrations](/sysml-rs/use/integrations/). These native commands are a sysml-rs convention, not the OMG Systems Modeling API.

The catalogue currently carries 170 commands, of which 1 are flagged deprecated (marked below).

## Analysis

| Command | Description | Parameters |
|---|---|---|
| `sysml.diagnostics` | Get the full diagnostic pipeline (parse + resolve + validate + health) for a loaded URI | `uri` |
| `sysml.inspect` | Compute the inspect-pipeline result (diagnostics + semantic tokens) for one URI or every loaded user URI | `uri`?, `workspace`, `focus_file`? |
| `sysml.parse` | Parse a SysML file and return the model graph with diagnostics (does not store) | `path` |
| `sysml.readiness` | Per-URI readiness predicate: is this question answerable yet (library/project/file)? | `uri` |

## Execution

| Command | Description | Parameters |
|---|---|---|
| `sysml.action.run` | Compile and run a named action to completion, returning the execution trace | `uri`, `action_name` |
| `sysml.action.start` | Compile an action from the model and start an execution session *(stateful)* | `uri`, `action_name` |
| `sysml.action.step` | Step an action session forward, returning the trace entry for the executed node *(stateful)* | `session_key` |
| `sysml.aggregate` | Compute aggregate constraint/verification/requirement status for all owners in a model | `uri` |
| `sysml.analysis.run` | Run a named analysis case using the solver registry with optional parameter overrides | `case_name`, `overrides` |
| `sysml.batch.create` | Create a BatchSession with N child runtime sessions (sweep / monte_carlo / trade_study) *(stateful)* | `kind`, `uri`, `subsystem_name`?, `children_params`, `label`?, `outcomes`?, `dt_ms`?, `max_time_ms`? |
| `sysml.batch.results` | Return per-child descriptors for a batch, optionally including each child's verdicts *(stateful)* | `batch_id`, `include_verdicts` |
| `sysml.batch.slice` | Filter a batch's children by status, verdict, or parameter predicate *(stateful)* | `batch_id`, `filter` |
| `sysml.batch.status` | Fetch the current BatchSession snapshot (kind, children, rollup status) *(stateful)* | `batch_id` |
| `sysml.breakpoint.clear` | Remove a breakpoint from a session by id (idempotent) *(stateful)* | `session_id`, `breakpoint_id` |
| `sysml.breakpoint.list` | List registered breakpoints for a session as (id, breakpoint) pairs *(stateful)* | `session_id` |
| `sysml.breakpoint.set` | Register a breakpoint on a running session; returns an opaque id *(stateful)* | `session_id`, `breakpoint` |
| `sysml.causation.trace` | Walk backward through the causation graph from a root event; returns the root plus upstream chain *(stateful)* | `session_id`, `root_event_id`?, `root_tick`?, `root_target`?, `max_depth`? |
| `sysml.constraint.check` | Evaluate all constraints in a model with optional parameter overrides | `uri`, `overrides` |
| `sysml.evaluate` | Evaluate a single element's expression value by its ID | `element_id` |
| `sysml.evaluate.analysis_cases` | Evaluate all analysis case elements in a model, returning output summaries per case | — |
| `sysml.evaluate.calculations` | Evaluate all calculation elements in a model, returning computed values | — |
| `sysml.evaluate.constraints` | Evaluate all constraint elements in a model, returning pass/fail results with details | — |
| `sysml.evaluate.expression` | Evaluate one expression-bearing element with optional overrides, returning value, verdict, context, and diagnostics | `element_id`, `overrides` |
| `sysml.evaluate.verification_cases` | Evaluate all verification case elements in a model, returning verdicts per case | — |
| `sysml.expression.eval` | Evaluate a standalone expression with optional variable bindings | `expr`, `context` |
| `sysml.flow.inspect` | Inspect port connections, flow paths, and optionally inject a payload to test delivery | `uri`, `inject_source`?, `inject_payload`? |
| `sysml.montecarlo.run` | Run Monte Carlo constraint analysis with parameter distributions, statistics, and histograms | `config` |
| `sysml.orchestrate.inject` | Inject an event into a specific subsystem and advance the orchestrator *(stateful)* | `session_key`, `subsystem`, `event` |
| `sysml.orchestrate.start` | Start a multi-subsystem orchestrator session from the model *(**deprecated**, stateful)* | `uri` |
| `sysml.orchestrate.step` | Advance all subsystems in the orchestrator by one tick *(stateful)* | `session_key` |
| `sysml.orchestrate.stop` | Terminate an orchestrator session and release its resources *(stateful)* | `session_key` |
| `sysml.orchestrate.workspace.start` | Compile all subsystems from the workspace and start an orchestrator session *(stateful)* | `uri`, `dt_ms`?, `max_time_ms`?, `overrides`? |
| `sysml.scenario.run` | Run a verification case end-to-end (SM compile + event-script + auto-step + per-tick assertion eval). Returns verdict + trace + assertion_checkpoints. | `case_name`, `max_ticks`? |
| `sysml.sensitivity.analyze` | Compute per-parameter Morris (μ, σ) or Sobol (S_i, S_Ti) indices from a completed sensitivity batch *(stateful)* | `batch_id`, `method`, `parameters_of_interest`, `output_metric`, `morris_levels`? |
| `sysml.sessions.archive.get` | Fetch the full archived session record (metadata + snapshots + verdicts) by id | `id` |
| `sysml.sessions.archive.list` | List archived sessions (completed runs) matching optional workspace / origin / since / only_golden filters | `workspace_uri`?, `origin`?, `since`?, `only_golden`? |
| `sysml.sessions.archive.mark_golden` | Tag an archived session as a golden reference run — pins it against archive eviction | `id`, `label` |
| `sysml.sessions.archive.unmark_golden` | Remove the golden tag from an archived session — it becomes eligible for LRU eviction again | `id` |
| `sysml.sessions.create` | Create an execution session, inferring the kind (simulation / action / orchestrator) from the model and optional target. Unified entry point subsuming the *.start commands. *(stateful)* | `uri`, `target`?, `dt_ms`?, `max_time_ms`?, `overrides`? |
| `sysml.sessions.diff` | Diff two sessions' latest snapshots (subsystem states + context variables) | `a_id`, `b_id` |
| `sysml.sessions.diff_timeline` | Walk two sessions' histories tick-by-tick and report where they first diverged | `a_id`, `b_id` |
| `sysml.sessions.fork` | Fork a session at its current tick into an independent child *(stateful)* | `session_id` |
| `sysml.sessions.fork_with_overrides` | Fork a session (optionally rewound to a past tick) and atomically apply parameter overrides to the child *(stateful)* | `session_id`, `overrides`, `at_tick`? |
| `sysml.sessions.info` | Return full detail for a session, including subsystems and the latest snapshot *(stateful)* | `session_id`, `include_variables`? |
| `sysml.sessions.inject` | Inject an event into a named subsystem of any session and advance one tick, optionally applying context overrides first *(stateful)* | `session_id`, `subsystem`, `event`, `overrides`? |
| `sysml.sessions.list` | List all live runtime sessions (state-machine, action, orchestrator) as typed summaries *(stateful)* | — |
| `sysml.sessions.quota` | Report per-kind session budgets and current usage *(stateful)* | — |
| `sysml.sessions.reap` | Drop all sessions past the inactivity timeout; returns the count removed *(stateful)* | — |
| `sysml.sessions.rename` | Set the display label on any session *(stateful)* | `session_id`, `label` |
| `sysml.sessions.reset` | Reset any session to its initial state, clearing step history and restarting the expiry timer *(stateful)* | `session_id` |
| `sysml.sessions.resume` | Clear the pause flag on a session halted at a breakpoint so subsequent sessions.step calls advance again. Idempotent: a no-op success on a session that isn't paused. Does not itself advance any ticks. *(stateful)* | `session_id` |
| `sysml.sessions.step` | Advance any session, optionally injecting an event and applying context overrides ONCE before stepping. `ticks` (default 1) runs that many ticks server-side in one call — for fine-dt runs that need thousands of ticks to reach an event (avoids a per-tick round-trip); the run still stops early at a breakpoint pause or the session's configured tick/time limit. Every advanced tick is recorded to the time series, so chart data stays complete; poll sessions.timeseries_decimated for it. *(stateful)* | `session_id`, `event`?, `overrides`?, `ticks`? |
| `sysml.sessions.stop` | Terminate a session of any kind and release its resources *(stateful)* | `session_id` |
| `sysml.sessions.subsystems` | List the subsystems of any session without fetching a full snapshot | `session_id` |
| `sysml.sessions.timeseries` | Fetch (time_ms, value) points for a single variable from a session's canonical time-series buffer (optionally bounded) | `session_id`, `var`, `start_ms`?, `end_ms`? |
| `sysml.sessions.timeseries_decimated` | Fetch an LTTB-decimated time series for a single variable (preserves visual shape; ideal for chart rendering) | `session_id`, `var`, `target_points`, `start_ms`?, `end_ms`? |
| `sysml.sessions.timeseries_names` | List the variable names captured in a session's canonical time-series buffer | `session_id` |
| `sysml.sessions.topology` | Return the structural topology of a session (modules, subsystems, physics domains) *(stateful)* | `session_id` |
| `sysml.sessions.verify` | Verify a running session's declared verification cases against its LIVE final-tick state (reads simulation-produced attributes like tripped/trip_time from the session's orchestrator context, including its slot store), routing each case through the one VerificationRunner. Reflects live-injected overrides and advanced ticks with no re-run. Optional case_names filters to specific cases; omit for all declared cases in the session's workspace. *(stateful)* | `session_id`, `case_names`? |
| `sysml.simulate.continuous.auto` | Start a continuous simulation auto-discovering ODE config from model metadata *(stateful)* | `uri`, `sm_name`, `dt_ms`?, `max_time_ms`? |
| `sysml.simulate.start` | Compile a state machine from the model and start a simulation session *(stateful)* | `uri`, `sm_name` |
| `sysml.simulate.step` | Advance a simulation session by one step with an optional event *(stateful)* | `session_key`, `event`? |
| `sysml.simulate.stop` | Terminate a simulation session and release its resources *(stateful)* | `session_key` |
| `sysml.solve` | Run constraint network solving with binding propagation, DOF analysis, optional rollup and sensitivity sweep | `uri`, `overrides`, `rollup_property`?, `sweep_spec`? |
| `sysml.timeline.getSnapshot` | Return a single TickSnapshot from a runtime session's history at the given tick index. | `session_id`, `tick` |
| `sysml.timeline.getTrace` | Return the full execution trace for a runtime session — every TickSnapshot from history() serialized to JSON. | `session_id` |
| `sysml.trace` | Generate a sequence trace by simulating message flow through compiled flow topology | `uri`, `inject_specs` |
| `sysml.trade_study` | Run a trade study evaluating design alternatives against a minimize/maximize objective | `study_name`, `overrides` |
| `sysml.trade_study.ode_sweep` | Sweep an ODE parameter across a range, running full simulations per value | `uri`, `sm_name`, `parameter_name`, `min_value`, `max_value`, `steps`, `dt_ms`?, `max_time_ms`?, `baseline_overrides`? |
| `sysml.verify` | Run a named verification case against model requirements with optional parameter overrides | `case_name`, `overrides` |
| `sysml.verify_with_simulation` | Run ODE simulation with parameter overrides, then verify the model's verification cases against the result (verdict via VerificationRunner) | `uri`, `sm_name`, `overrides`, `dt_ms`?, `max_time_ms`? |
| `sysml.verify_with_simulation_trace` | Run ODE simulation and return full time-series trace for charting (sampled to max_points) | `uri`, `sm_name`, `overrides`, `dt_ms`?, `max_time_ms`?, `max_points`? |
| `sysml.verify.executions` | List verification executions (newest-first) as a projection over the session archive: each is an archived session (trajectory or external ingest) carrying &gt;=1 verdict, with origin, evaluation_mode, B6 provenance, external run identity, per-case results (verdict + case_changed_since stale flag), and verdict counts. Verdict-less simulation runs are not executions. Optional case_name filter keeps only executions touching that case. Scoped server-side via session provenance, like sysml.verify.timeline. | `case_name`? |
| `sysml.verify.latest_status` | Per-case latest verification status across executions, context-qualified by evaluation_mode: {trajectory?, external?} with verdict, execution_id, timestamp, case_changed_since, and mode-specific provenance (trajectory model_digest; external tool + matches_current_model). Execution-side only — the caller composes it with the static read. Scoped server-side via session provenance. | — |
| `sysml.verify.record_external` | Ingest externally produced verification verdicts (CI, pytest, HIL) as a synthetic archived session (origin 'external'). Requires the producing tool name and the model content digest the results were produced against (declared_digest); each verdict row is {case_id: &lt;verification case NAME in the current workspace&gt;, verdict: pass\|fail\|inconclusive\|error, artifacts?: [uri]}. Unknown case names or verdict strings reject the whole batch; a stale declared_digest is recorded and labeled, never rejected. Verdicts appear in sysml.verify.timeline with the external evidence block. *(stateful)* | `tool`, `declared_digest`, `verdicts`, `run_ref`?, `artifacts`?, `label`? |
| `sysml.verify.timeline` | Return verdict-flip history across past verification runs of the current workspace (scoped server-side via session provenance), with optional case and timestamp filters | `case_ids`?, `since_timestamp`? |
| `sysml.whatif` | Override a variable value and evaluate ALL constraints on the graph (baseline vs override), returning per-constraint flips plus an overlay payload (values, constraintResults, guardDiagnoses) suitable for the diagram overlay UI. Optional session_key selects an orchestrator session as the base context. | `variable_name`, `override_value`, `session_key`? |
| `sysml.whatif.sweep` | Sweep a parameter across a range and evaluate constraints at each step to find thresholds | `element_id`, `variable_name`, `start`, `end`, `steps` |
| `sysml.workspace.verify` | Run cross-file workspace verification, merging all loaded graphs and evaluating all verification cases | `timeout_secs`? |

## FileManagement

| Command | Description | Parameters |
|---|---|---|
| `sysml.load_file` | Parse and load a SysML file using the PEG batch parser | `path` |
| `sysml.load_source` | Parse and load SysML source text directly (no file I/O) | `uri`, `source` |
| `sysml.load_workspace` | Discover and load all .sysml files under a directory (recursive). Returns loaded URIs and any errors. | `root` |
| `sysml.loaded_uris` | List all currently loaded model URIs | — |
| `sysml.unload_file` | Remove a previously-loaded URI from the analysis host | `uri` |
| `sysml.workspace.refresh` | Rediscover projects across workspace roots, reset the shared host, re-register projects, and re-enable stdlib. Returns the discovered project list + stdlib status. Does NOT preserve open-document buffers (LSP shell handles that). | `roots`, `enable_stdlib`? |

## Query

| Command | Description | Parameters |
|---|---|---|
| `sysml.ancestors` | Walk the ownership chain upward from an element to the root | `uri`, `id` |
| `sysml.cache.clear` | Delete the library cache file from disk. | — |
| `sysml.cache.rebuild` | Compute a library cache rebuild payload: clears the cache, returns before/after snapshots and library state. Reload spawn stays on the transport. | — |
| `sysml.cache.status` | Library cache file snapshot (size_bytes, element_count, crate_version, exists). | — |
| `sysml.children` | Legacy wrapper over sysml.query: get direct children of an element (ownership hierarchy) | `uri`, `id` |
| `sysml.code_action.list` | Compute every code action (quick-fixes, refactorings, source actions) at a range | `uri`, `range_start_line`, `range_start_col`, `range_end_line`, `range_end_col`, `diagnostics` |
| `sysml.completion` | Compute completion candidates for the cursor position | `uri`, `line`, `col`, `trigger`?, `ctx_in_import`, `ctx_in_comment_or_string`, `ctx_in_feature_chain`, `ctx_in_type_ref` |
| `sysml.completion.resolve` | Enrich a completion item with documentation and type detail | `uri`?, `element_id` |
| `sysml.dependency.status` | Walk workspace roots, hydrate manifest dependencies, and report per-root resolution outcomes + summary counts. | `roots` |
| `sysml.descendants` | Recursively collect all descendant elements in the ownership tree | `uri`, `id` |
| `sysml.diagram.edit` | Compute the workspace edit for a diagram-driven create/delete/editLabel/addSequenceMessage/addSequenceLifeline action | `request` |
| `sysml.element` | Get a single element by its ID | `uri`, `id` |
| `sysml.expression.ast` | Project expression element subtrees as JSON ASTs for rendering (KaTeX) and inspection | `element_id`? |
| `sysml.find` | Legacy wrapper over sysml.query: find elements by name pattern (substring match), optionally filtered by element kind | `uri`, `pattern`, `kind`? |
| `sysml.format.document` | Compute whitespace-only formatting edits for a loaded document | `uri`, `tab_size`?, `insert_spaces`? |
| `sysml.get_source` | Get the source text and span covering one element's declaration | `uri`, `id` |
| `sysml.goto_definition` | Resolve the goto-definition target for the cursor position | `uri`, `line`, `col` |
| `sysml.hover` | Render hover content (markdown + range) for the cursor position | `uri`, `line`, `col` |
| `sysml.model.tree` | Build a hierarchical tree of root elements and their children (for tree views) | `uri`, `max_depth`?, `view`? |
| `sysml.outline` | Build the document outline (nested symbol tree) for a loaded URI | `uri` |
| `sysml.query` | Run a structured, paged query over model elements | `uri`, `spec` |
| `sysml.references` | Find every reference to the element at the given cursor position | `uri`, `line`, `col` |
| `sysml.rename` | Compute prepare-rename info or apply-rename workspace edits at a cursor position | `uri`, `line`, `col`, `new_name`? |
| `sysml.salsa.stats` | Salsa query execution statistics (executions, validations, hit ratio) | — |
| `sysml.salsa.stats.reset` | Reset salsa query execution statistics to zero | — |
| `sysml.stats` | Compute element and relationship count statistics for a model | `uri` |
| `sysml.system.capabilities` | Report service feature-flag capabilities (fork-at-tick, snapshot retention, etc.) | — |
| `sysml.trace_matrix` | Generate a traceability matrix between element kinds via a relationship kind | `uri`, `source_kind`, `rel_kind`, `target_kind` |
| `sysml.unverified` | Legacy wrapper over sysml.query: find all requirements that have no Verify relationship targeting them | `uri` |
| `sysml.viewpoints.by_stakeholder` | Legacy wrapper over sysml.query: list ViewpointDefinitions / ViewpointUsages whose StakeholderMembership references the given stakeholder PartUsage | `uri`, `stakeholder_id` |
| `sysml.views.by_viewpoint` | Legacy wrapper over sysml.query: list user-authored views that satisfy the given ViewpointDefinition / ViewpointUsage | `uri`, `viewpoint_id` |
| `sysml.views.list` | Legacy wrapper over sysml.query: list user-authored ViewUsage / ViewDefinition elements (id, name, exposed namespaces, render and filter members) | `uri` |
| `sysml.workflow.log` | The append-only workflow event log for a project, oldest-first; optionally filtered to one element. Events keyed on ids that no longer exist are still returned — history is never deleted or silently re-attached (ADR-009). | `project`, `element_id`? |
| `sysml.workflow.state` | Current workflow state of one element, derived by folding its event log (never authored): latest approval + assignee, sign-offs, suspect-clearing attestations (each flagged `superseded` when the requirement changed again after it), comment count, and `orphaned` (the id no longer exists in the current graph — history belongs to a prior identity). | `project`, `element_id` |
| `sysml.workspace.add_attribute` | Compute a guarded text edit adding an `attribute &lt;name&gt; [= &lt;value&gt;];` member to an element. Name must be a valid identifier; value (optional) is a single-line expression (no `;`). Fails when an attribute of that name already exists (edit its value instead). Buffer-writeback with an expected_old_text guard. | `element_id`, `name`, `value`? |
| `sysml.workspace.add_constraint` | Compute a guarded text edit adding an `assume/require constraint [name] { &lt;expr&gt; }` member to a requirement. kind is "assume" or "require"; expr is a single-line boolean expression (no braces or `;`); name (optional) is an identifier. Buffer-writeback with an expected_old_text guard. | `element_id`, `kind`, `expr`, `name`? |
| `sysml.workspace.add_derive_link` | Compute a guarded text edit inserting `#derivation connection { end #original ::&gt; &lt;original&gt;; end #derive ::&gt; &lt;derived&gt;; }` at the end of the derived requirement's owning package. Prepends `private import RequirementDerivation::*;` in the same insertion when the owning-package chain lacks one (the import is load-bearing for Derive elaboration). Fails hard when the derive link already exists. requirement_id is the DERIVED end; a 'derived to' add swaps the roles client-side. Buffer-writeback with an expected_old_text guard. | `requirement_id`, `original_id` |
| `sysml.workspace.add_rationale` | Compute a guarded text edit adding a @Rationale { text = "…" } metadata member to an element. Add-only (a requirement may carry several rationale annotations; the read side joins them). Text must be single-line non-blank; embedded quotes/backslashes are escaped. Buffer-writeback with an expected_old_text guard. | `element_id`, `text` |
| `sysml.workspace.add_refine_link` | Compute a guarded text edit inserting `dependency from &lt;refining&gt; to &lt;refined&gt; { @Refinement; }` at the end of the refining requirement's owning package. Prepends `private import ModelingMetadata::*;` in the same insertion when the owning-package chain lacks one (the import is load-bearing for Refine elaboration). requirement_id is the REFINING end (the row's outgoing `refines`). Fails hard when the refine link already exists. Buffer-writeback with an expected_old_text guard. | `requirement_id`, `refined_id` |
| `sysml.workspace.add_requirement_doc` | Compute a guarded text edit adding a doc comment to an element that has none. Fails when a doc comment already exists (use edit_requirement_doc). Buffer-writeback with an expected_old_text guard. | `element_id`, `new_text` |
| `sysml.workspace.add_requirement_maturity` | Compute a guarded text edit adding @StatusInfo { status = StatusKind::&lt;status&gt; } to an element that has none (closed vocabulary: open\|tbd\|tbr\|tbc\|done\|closed). Fails when @StatusInfo already exists (use edit_requirement_maturity). Buffer-writeback with an expected_old_text guard. | `element_id`, `status` |
| `sysml.workspace.add_requirement_role` | Compute a guarded text edit adding a `&lt;keyword&gt; &lt;name&gt; : &lt;Type&gt;;` member (role = "subject"\|"actor"\|"stakeholder"\|"concern") to a requirement. type_id references the definition: subject accepts any definition, actor/stakeholder a part definition, concern a concern definition. Subject is singleton (fails when one exists). name is an identifier. Buffer-writeback with an expected_old_text guard. | `requirement_id`, `role`, `type_id`, `name` |
| `sysml.workspace.add_satisfy_link` | Compute a guarded text edit inserting `satisfy &lt;requirement&gt;;` at the end of the picked subject element's body (the subject's file — possibly a different file than the requirement's). The reference is the requirement's simple name when it is a sibling scope member, its fully qualified name otherwise. Fails hard when the satisfy link already exists. Buffer-writeback with an expected_old_text guard. | `requirement_id`, `subject_id` |
| `sysml.workspace.add_verify_link` | Compute a guarded text edit inserting `verify &lt;requirement&gt;;` into the picked verification case's objective body (the case's file). A case with no objective gets the whole `objective { verify &lt;requirement&gt;; }` block. Fails hard when the case already verifies the requirement, or when the target is not a verification case. Buffer-writeback with an expected_old_text guard. | `requirement_id`, `case_id` |
| `sysml.workspace.capabilities` | Workspace-level model-content feature flags + name lists for the simulation app's UI gating. | — |
| `sysml.workspace.create_requirement` | Compute a guarded text edit inserting a new requirement (optional &lt;'short name'&gt; reqId and doc body) at the end of a package's or requirement's body. Buffer-writeback: the client applies the edit only if the buffer slice equals expected_old_text. The parent must be a package or requirement. | `parent_id`, `name`, `short_name`?, `doc`? |
| `sysml.workspace.edit_attribute_value` | Compute a guarded text edit replacing an attribute usage's inline `= value` expression. Buffer-writeback with an expected_old_text guard. Fails hard when the declaration has no inline value (adding one is a creation action). | `element_id`, `new_value` |
| `sysml.workspace.edit_requirement_doc` | Compute a guarded text edit replacing an element's doc-comment body (first doc in document order). Buffer-writeback: the client applies the edit only if the buffer slice equals expected_old_text, else fails loudly. Fails hard when no doc comment exists (adding one is a creation action). | `element_id`, `new_text` |
| `sysml.workspace.edit_requirement_maturity` | Compute a guarded text edit setting @StatusInfo status to StatusKind::&lt;status&gt; (closed vocabulary: open\|tbd\|tbr\|tbc\|done\|closed). Buffer-writeback with an expected_old_text guard. Fails hard when the element has no @StatusInfo metadata (adding one is a creation action). | `element_id`, `status` |
| `sysml.workspace.files` | Recursively list .sysml/.kerml files under a workspace directory; tree pruned to directories that contain such files. | `root`, `max_depth`? |
| `sysml.workspace.info` | Return tree + stats for every loaded user URI in one call (excludes __workspace__/__stdlib__). | `uris`? |
| `sysml.workspace.info_summary` | Workspace-level summary: per-root discovery + loaded host counts + transport-supplied telemetry counters. | `workspace_roots`, `telemetry_counters` |
| `sysml.workspace.model_tree` | Walk all loaded user files; emit per-URI tree projections with line/character ranges (LSP Position semantics). Deterministic URI ordering for cache stability. | `max_depth`?, `view`? |
| `sysml.workspace.requirement_detail` | Evaluated contract of one requirement: subject, assumed/required constraints (inline text is verbatim source; reference-form links resolve when unambiguous), owned attribute values, plus narrative buckets (actors, stakeholders, framed concerns, rationale). Verdict inputs vs narrative separation is binding — render constraints/values next to the verified chip, roles in a generic detail bucket. Fails on unknown ids and non-requirement elements. | `element_id` |
| `sysml.workspace.requirement_rows` | Requirements-workbench table rows: document-ordered Requirement{Definition,Usage} rows over the elaborated workspace, with statement text, outline depth, StatusInfo maturity, satisfy/verify/derive/refine links, and a three-state verification rollup. Paged via limit/cursor in the spec ({} for defaults). | `spec` |
| `sysml.workspace.requirement_suspects` | Requirement rows suspect against a baseline: diffs two stored snapshots (`from`/`to` accept a baseline name or commit id; omitted `to` = latest commit), attributes every change to its nearest owning requirement, and propagates downstream along Derive edges. Causes distinguish text edits (with before/after bodies), other content changes, added/removed children, identity-not-in-baseline (ADR-009: never name-matched), and upstream suspicion. Each record carries `cleared_by` (workflow event seq) when a non-superseded suspect-clearing attestation covers it — cleared rows are not suspect for display but stay listed. | `project`, `from`, `to`? |

## Storage

| Command | Description | Parameters |
|---|---|---|
| `sysml.store.baseline.create` | Create a named, immutable baseline pointing at a commit (default: the latest). Baselines can never be renamed or retargeted; the referenced commit becomes eviction-exempt. When the workspace root is a git work tree, git provenance (HEAD sha, dirty flag, branch) is recorded as corroborating metadata — a dirty tree is recorded honestly, never refused (the content-addressed commit digest is the identity; B6 steward ruling 2026-07-16). | `project`, `name`, `commit`? |
| `sysml.store.baseline.list` | List a project's baselines (most recently created first) | `project` |
| `sysml.store.diff` | Element-level diff between two stored snapshots. `from`/`to` each accept a baseline name (resolved first) or a commit id; omitted `to` means the latest commit. Optional element_ids narrows the result (the changed-since composition). Scope renames / anonymous-sibling shifts surface as removed+added per the ADR-009 identity contract. | `project`, `from`, `to`?, `element_ids` |
| `sysml.store.history` | List all commits for a project (most recent first) | `project` |
| `sysml.store.latest` | Get the latest commit ID for a project from the store | `project` |
| `sysml.store.load` | Load a stored model snapshot by project and commit ID | `project`, `commit` |
| `sysml.store.projects` | List all projects in the store | — |
| `sysml.store.save` | Store a model snapshot with version metadata | `project`, `meta`, `graph` |
| `sysml.store.save_workspace` | Snapshot the current elaborated workspace graph into the store under a content-addressed commit id (SHA-256 of the diff-compared content). Idempotent: an unchanged workspace returns the existing commit's metadata instead of minting a new one. The graph is read through the same workspace accessor `sysml.workspace.requirement_rows` uses, so element ids in stored snapshots correlate 1:1 with row ids. | `project`, `message`? |
| `sysml.workflow.assign` | Assign an engineer to an element (workflow sidecar). Folded state keeps the latest assignee; the log keeps every assignment. The element must exist in the current workspace; requires an explicit non-blank `actor` and `assignee`. | `project`, `element_id`, `assignee`, `actor` |
| `sysml.workflow.attest_suspect_clearing` | Record a suspect-clearing attestation: `actor` reviewed `element_id`'s changes since `baseline` (name or commit id) and vouches the intent still holds. Mints/resolves the current content commit first (idempotent) and pins it as `attested_commit` — any later content change supersedes the attestation and suspicion re-fires. Fails if the element is not actually suspect against the baseline, or if `actor` is blank (no silent default identity). | `project`, `element_id`, `baseline`, `rationale`, `actor` |
| `sysml.workflow.attest_verification` | Record a MANUAL verification act on an element (B10 layer 3, human leg): `actor` verified `element_id` by `method` (one of the spec's VerificationMethodKind: inspect \| analyze \| demo \| test — validated, closed set). An ATTESTATION in the append-only workflow sidecar — never a computed verdict; it must never render as a verdict chip or enter verdict rollups. Mints/resolves the current content commit and pins it as `attested_commit`; any later content change supersedes the attestation at display time. Requires a non-blank `actor` and `statement`. | `project`, `element_id`, `method`, `statement`, `actor` |
| `sysml.workflow.comment` | Record a review comment on an element in the append-only workflow sidecar. The element must exist in the current workspace (new writes against dead ids are rejected — history on later-churned ids is handled at read time). Requires an explicit non-blank `actor` and `body`. | `project`, `element_id`, `body`, `actor` |
| `sysml.workflow.relink` | Record a deliberate re-link of workflow history from a dead element id to its successor (ADR-009: identity changes are never auto-matched; a re-link is itself an audited event). The target must exist in the current workspace. | `project`, `from_element`, `to_element`, `rationale`, `actor` |
| `sysml.workflow.set_approval` | Transition an element's approval state (workflow sidecar; closed vocabulary: draft, in_review, approved, rejected — 'draft' is every element's initial state). The transition's `from` is derived server-side from the folded event log, never client-claimed; a no-op transition (target == current state) is rejected. The element must exist in the current workspace; requires an explicit non-blank `actor`. | `project`, `element_id`, `to`, `actor` |
| `sysml.workflow.sign_off` | Record a sign-off attestation statement against an element (workflow sidecar; all sign-offs are kept oldest-first in folded state — a sign-off is a statement of record, never overwritten). The element must exist in the current workspace; requires an explicit non-blank `actor` and `statement`. | `project`, `element_id`, `statement`, `actor` |

## Visualization

| Command | Description | Parameters |
|---|---|---|
| `sysml.action.visualize` | Render a named action's control flow as PlantUML activity text. | `action_id` |
| `sysml.diagram.diagnostic_overlay` | Return the diagnostics overlay (validation-diagnostic badges: worst-case severity + per-message tooltip detail) for the given DECLARED view's scene, joined by ElementId, as JSON. Companion to sysml.diagram.viewmodel — pass the SAME view_usage_id so the overlay joins the same scene. Reads workspace diagnostics (readiness-gated); needs no session. | `view_usage_id`, `expanded_ids` |
| `sysml.diagram.expand` | Re-project a diagram with the given expanded-node set, returning the renderer-neutral ViewModel. Replaces expanded_nodes state for the URI. | `uri`, `view_type`, `expanded_node_ids` |
| `sysml.diagram.open` | Open a diagram for the given URI and view type, returning the renderer-neutral ViewModel. Updates open_diagrams for auto-refresh. | `uri`, `view_type`? |
| `sysml.diagram.sim_overlay` | Return the per-tick simulation overlay (active-element highlights, live scalar badges, time-series channel directory) for a live session, joined to the given DECLARED view's scene by ElementId, as JSON. Companion to sysml.diagram.viewmodel — pass the SAME view_usage_id so the overlay joins the same scene. | `session_id`, `view_usage_id`, `expanded_ids` |
| `sysml.diagram.verdict_overlay` | Return the per-run verdict overlay (constraint solver pass/fail badges + solved scalar values) for a live session, joined to the given DECLARED view's scene by ElementId, as JSON. Companion verdict sidecar to sysml.diagram.viewmodel — pass the SAME view_usage_id so the overlay joins the same scene. | `session_id`, `view_usage_id`, `expanded_ids` |
| `sysml.diagram.view` | Switch a diagram's view type for the given URI, returning the renderer-neutral ViewModel. Updates open_diagrams. | `uri`, `view_type` |
| `sysml.diagram.viewmodel` | Return the renderer-agnostic ViewModel (scene + design tokens + text-map + interaction descriptors + frame) for a DECLARED view (ViewUsage / ViewDefinition), scoped by its Expose / filter memberships, as JSON. | `uri`, `view_usage_id`, `expanded_ids` |
| `sysml.export.json` | Export a loaded model as canonical SysML v2 JSON | `uri` |
| `sysml.export.plantuml` | Export a loaded model as PlantUML notation. `view` selects general \| state \| action \| sequence (default general). | `uri`, `view`? |
| `sysml.flow.visualize` | Render the flow connections for the given URI as PlantUML sequence text. | `_flow_id`? |
| `sysml.views.create_scratch` | Build a 'view scratch : InterconnectionView { expose ...; }' source snippet from a list of qualified names | `expose` |
| `sysml.views.render` | Render a user-authored ViewUsage as a diagram (composes ViewRequest from the view's Expose / filter / rendering memberships) | `uri`, `view_usage_id`, `expanded_ids` |

Parameter names marked `?` are optional. Full parameter types and per-parameter descriptions are in the committed artifact `src/generated/service-commands.json` and from the live `GET /commands` endpoint.

## How this page is generated

This page and its data artifact were generated by `node scripts/generate-reference.mjs` (run from `website/`) at sysml-rs commit `11bd751` on 2026-08-25. Input: `GET /commands` and `GET /health` on a locally started `target/release/sysml-api` (version 0.1.0).
Do not edit the page by hand — regenerate it. `npm run gen-check` reports drift between the committed artifacts and a fresh generation.
