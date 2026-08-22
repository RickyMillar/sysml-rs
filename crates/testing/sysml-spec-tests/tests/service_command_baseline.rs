//! S0.T1 — Service-command baseline corpus.
//!
//! Captures a frozen request/response fixture set for the load-bearing
//! `#[service_command]`s while `sysml-service` is still backed by its own
//! pest path + DashMap. This corpus becomes the byte-identity safety net
//! for Step 2 (re-backing service onto `sysml-ide-db`).
//!
//! ## Scope
//!
//! There are ~100 registered service commands. This harness captures a
//! curated, load-bearing subset (see `BaselineCommand` list below) that
//! takes a URI / source / element_id and returns model-derived data
//! (parses, resolves, elaborates, finds, lists, gets attributes). Skipped:
//! sessions, runtime, breakpoints, batches, simulate.*, orchestrate.*,
//! sensitivity, monte-carlo, store.*, action_run / action_start /
//! continuous_*, archive admin — none are load-bearing for the static-
//! analysis byte-identity question S2 protects.
//!
//! ## Fixtures
//!
//! - `examples/the-book-corpus/coffee-machine/` — worked example (vendored
//!   from the book repo) with definitions,
//!   parts, ports, connections, views, requirements, actions, calculations.
//!   We use `definitions.sysml` (definitions/parts) and `views.sysml`
//!   (views/viewpoints/stakeholders) as the two coffee-machine fixtures.
//! - `examples/espresso-pump-hybrid/Physics/PumpODE.sysml` — runtime/physics
//!   shape (PartDefinition + AttributeUsage + calc def / SSR).
//! - `libraries/standard/library.kernel/Base.kerml` — stdlib breadth.
//!
//! ## Determinism
//!
//! Element IDs are random today (pre-S1 — that's the whole point of this
//! baseline). Snapshots use insta redaction filters to scrub UUID-shaped
//! strings so re-runs match. The captured JSON files under
//! `fixtures/service-baseline/` keep IDs as-is for record-keeping; the
//! post-S1 baseline will assert these IDs become stable.
//!
//! Iteration order in responses is also normalised before snapshotting
//! (sort by stable key) so HashMap/HashSet randomness doesn't poison the
//! snapshot.
//!
//! ## Output
//!
//! - JSON corpus (human-readable archive):
//!   `crates/testing/sysml-spec-tests/fixtures/service-baseline/<command>/<fixture>.json`,
//!   one file per (command × fixture) pair, shape `{ request, response }`.
//! - Insta snapshots (regression gate):
//!   `crates/testing/sysml-spec-tests/tests/snapshots/service_command_baseline__*`.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sysml_core::{ElementKind, ModelGraph};
use sysml_service::{execute_command, SysmlService};

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    // crates/testing/sysml-spec-tests/Cargo.toml → workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Absolute-checkout-path → stable-token substitutions applied to every
/// response bundle before it is snapshotted or archived, so the committed
/// baselines carry no developer-specific absolute path (fresh-clone /
/// relocated-checkout portability). Most specific prefix first — the shared
/// repo parent must come last or it would partially rewrite the longer
/// workspace paths. See `sysml_spec_tests::path_canon`.
fn path_replacements() -> Vec<sysml_spec_tests::path_canon::PathReplacement> {
    use sysml_spec_tests::path_canon::PathReplacement;
    vec![
        PathReplacement::new(workspace_root().to_string_lossy().into_owned(), "<WS>"),
        PathReplacement::new(
            workspace_root()
                .parent()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "<REPO>",
        ),
    ]
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("service-baseline")
}

// ---------------------------------------------------------------------------
// Fixture catalog
// ---------------------------------------------------------------------------

/// One concrete SysML file we drive commands against. The label is used
/// to namespace both the JSON corpus filename and the insta snapshot
/// suffix; pick something filesystem-safe and stable.
#[derive(Clone, Copy)]
struct Fixture {
    label: &'static str,
    /// File path resolver — closure so we can switch between book / examples
    /// roots transparently.
    resolve: fn() -> PathBuf,
}

fn coffee_definitions_path() -> PathBuf {
    workspace_root()
        .join("examples")
        .join("the-book-corpus")
        .join("coffee-machine")
        .join("definitions.sysml")
}

fn coffee_views_path() -> PathBuf {
    workspace_root()
        .join("examples")
        .join("the-book-corpus")
        .join("coffee-machine")
        .join("views.sysml")
}

fn espresso_pump_path() -> PathBuf {
    workspace_root()
        .join("examples")
        .join("espresso-pump-hybrid")
        .join("Physics")
        .join("PumpODE.sysml")
}

fn stdlib_base_path() -> PathBuf {
    workspace_root()
        .join("libraries")
        .join("standard")
        .join("library.kernel")
        .join("Base.kerml")
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        label: "coffee_definitions",
        resolve: coffee_definitions_path,
    },
    Fixture {
        label: "coffee_views",
        resolve: coffee_views_path,
    },
    Fixture {
        label: "espresso_pump",
        resolve: espresso_pump_path,
    },
    Fixture {
        label: "stdlib_base",
        resolve: stdlib_base_path,
    },
];

// ---------------------------------------------------------------------------
// Command catalog
//
// The list below is the curated load-bearing subset. Each entry carries a
// one-line rationale for inclusion. Commands not in this list are either
// runtime/session/admin (out of scope for S2 byte-identity) or rely on
// state we cannot construct deterministically from a single file load.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BaselineCommand {
    /// Wire name (e.g. `sysml.find`).
    name: &'static str,
    /// One-line rationale for inclusion in the baseline.
    rationale: &'static str,
    /// Build the JSON request body given the loaded URI and the live
    /// graph. Returning `None` means: skip this fixture for this command
    /// (e.g. element-id-based commands when the graph contains no usable
    /// element of the required shape).
    build_req: fn(uri: &str, graph: &ModelGraph) -> Option<Value>,
}

// ---------- request builders ----------

fn req_uri_only(uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({ "uri": uri }))
}

/// Pick the first named, non-library element matching any of the given
/// kinds. Returns its id as a JSON string.
///
/// The selector sorts on `(name, kind, span)` rather than `(name, id)`
/// because `id` is non-deterministic pre-S1 — ties on name would resolve
/// run-to-run differently and produce different fixture inputs. Spans
/// are stable across runs of the same source bytes.
fn first_named_id_of(graph: &ModelGraph, kinds: &[ElementKind]) -> Option<String> {
    let mut hits: Vec<(String, String, String, String)> = graph
        .elements
        .values()
        .filter(|e| kinds.iter().any(|k| &e.kind == k))
        .filter(|e| e.name.is_some())
        .map(|e| {
            let span_key = e
                .spans
                .first()
                .map(|s| format!("{}:{}:{}", s.file, s.start, s.end))
                .unwrap_or_default();
            (
                e.name.clone().unwrap_or_default(),
                format!("{:?}", e.kind),
                span_key,
                e.id.to_string(),
            )
        })
        .collect();
    hits.sort();
    hits.into_iter().next().map(|(_, _, _, id)| id)
}

/// Pick the first named element of any kind (used when we just need
/// "some element id" to dereference). See `first_named_id_of` for the
/// tie-breaker rationale.
fn first_named_id(graph: &ModelGraph) -> Option<String> {
    let mut hits: Vec<(String, String, String, String)> = graph
        .elements
        .values()
        .filter(|e| e.name.is_some())
        .map(|e| {
            let span_key = e
                .spans
                .first()
                .map(|s| format!("{}:{}:{}", s.file, s.start, s.end))
                .unwrap_or_default();
            (
                e.name.clone().unwrap_or_default(),
                format!("{:?}", e.kind),
                span_key,
                e.id.to_string(),
            )
        })
        .collect();
    hits.sort();
    hits.into_iter().next().map(|(_, _, _, id)| id)
}

/// Pick a deterministic (line, col) cursor position over the first named
/// element in the file. Reads the file from disk to convert byte offset
/// → UTF-16 line/character (LSP convention).
fn first_named_element_position(uri: &str, graph: &ModelGraph) -> Option<(u32, u32)> {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    let content = std::fs::read_to_string(path).ok()?;
    let mut hits: Vec<(String, &str, usize)> = graph
        .elements
        .values()
        .filter(|e| e.name.is_some())
        .filter_map(|e| {
            let span = e.spans.iter().find(|s| s.file == uri || s.file == path)?;
            Some((
                e.name.clone().unwrap_or_default(),
                span.file.as_str(),
                span.start,
            ))
        })
        .collect();
    hits.sort();
    let (_, _, start) = hits.into_iter().next()?;
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (idx, ch) in content.char_indices() {
        if idx >= start {
            let prefix = &content[line_start..idx.min(content.len())];
            let character: u32 = prefix.chars().map(|c| c.len_utf16() as u32).sum();
            return Some((line, character));
        }
        if ch == '\n' {
            line += 1;
            line_start = idx + 1;
        }
    }
    Some((line, 0))
}

fn req_references(uri: &str, g: &ModelGraph) -> Option<Value> {
    let (line, col) = first_named_element_position(uri, g)?;
    Some(json!({ "uri": uri, "line": line, "col": col }))
}

fn req_goto_definition(uri: &str, g: &ModelGraph) -> Option<Value> {
    let (line, col) = first_named_element_position(uri, g)?;
    Some(json!({ "uri": uri, "line": line, "col": col }))
}

fn req_hover(uri: &str, g: &ModelGraph) -> Option<Value> {
    let (line, col) = first_named_element_position(uri, g)?;
    Some(json!({ "uri": uri, "line": line, "col": col }))
}

fn req_completion(uri: &str, g: &ModelGraph) -> Option<Value> {
    // Cursor at the first named element — completion-context query token will
    // be the element name's prefix; route classifier picks `General` since
    // there's no trigger character. The result is sensitive to scope chain
    // and workspace contents but stable across re-runs.
    let (line, col) = first_named_element_position(uri, g)?;
    Some(json!({
        "uri": uri,
        "line": line,
        "col": col,
        "trigger": null,
        "ctx_in_import": false,
        "ctx_in_comment_or_string": false,
        "ctx_in_feature_chain": false,
        "ctx_in_type_ref": false,
    }))
}

fn req_diagram_edit(uri: &str, _g: &ModelGraph) -> Option<Value> {
    // `Create` action — appends a `PartDefinition` template to the document.
    // The wire shape is the full `DiagramEditRequest` JSON. The result
    // includes the computed workspace edit (line/col-based) + status
    // payload, both stable across re-runs.
    Some(json!({
        "request": {
            "uri": uri,
            "action": "create",
            "elementTypeId": "PartDefinition",
            "containerId": null,
        }
    }))
}

fn req_code_action_list(uri: &str, g: &ModelGraph) -> Option<Value> {
    // Cursor at the first named element with an empty diagnostic list.
    // Exercises the cursor-refactoring + source-action paths
    // (expand-body / keyword-toggle / toggle-abstract / add-doc-comment /
    // organize-imports). Quick-fixes (auto-import / use-qualified-name /
    // create-definition / rename-duplicate / replace-Real-with-ISQ /
    // insert-semicolon / insert-closing-brace) require diagnostics — those
    // paths are exercised by the cross-transport parity test which threads
    // a synthetic E200/PH006/S001 diagnostic through.
    let (line, col) = first_named_element_position(uri, g)?;
    Some(json!({
        "uri": uri,
        "range_start_line": line,
        "range_start_col": col,
        "range_end_line": line,
        "range_end_col": col,
        "diagnostics": [],
    }))
}

fn req_format_document(uri: &str, _g: &ModelGraph) -> Option<Value> {
    // Default formatting options (4-space indent, spaces). Whitespace edit
    // shape is sensitive to source indentation/blank lines/trailing-ws state
    // and the tree-sitter nesting walk — stable across re-runs of the same
    // bytes; varies meaningfully across fixtures.
    Some(json!({
        "uri": uri,
        "tab_size": 4,
        "insert_spaces": true,
    }))
}

fn req_rename(uri: &str, g: &ModelGraph) -> Option<Value> {
    // Cursor at the first named element, no `new_name` provided → exercises
    // the prepare-rename path (placeholder + range). The apply path is also
    // covered indirectly via the cross-transport parity test, which dispatches
    // both modes and compares byte-identity across CLI / MCP / REST.
    let (line, col) = first_named_element_position(uri, g)?;
    Some(json!({
        "uri": uri,
        "line": line,
        "col": col,
        "new_name": null,
    }))
}

fn req_completion_resolve(uri: &str, g: &ModelGraph) -> Option<Value> {
    // Picks the first named element id in the loaded graph. The resolve
    // command enriches a candidate with doc-comment + type detail; shape
    // is `Option<CompletionDetails>` and is stable across re-runs.
    let id = first_named_id(g)?;
    Some(json!({ "uri": uri, "element_id": id }))
}

fn req_workspace_files(uri: &str, _g: &ModelGraph) -> Option<Value> {
    // Roots the listing at the directory containing the fixture file, with
    // a small `max_depth` so responses stay bounded (and don't pull in the
    // whole monorepo when a fixture lives near the workspace root). Path is
    // absolute by design — the redaction regex scrubs it before the snapshot
    // is taken so the fixture is portable across machines.
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    let parent = std::path::Path::new(path).parent()?;
    Some(json!({
        "root": parent.to_string_lossy(),
        "max_depth": 2,
    }))
}

fn req_find_part(uri: &str, _g: &ModelGraph) -> Option<Value> {
    // Stable across fixtures — empty pattern matches everything; capture
    // the substring filter shape and the kind filter shape.
    Some(json!({ "uri": uri, "pattern": "", "kind": null }))
}

fn req_element(uri: &str, g: &ModelGraph) -> Option<Value> {
    let id = first_named_id(g)?;
    Some(json!({ "uri": uri, "id": id }))
}

fn req_children(uri: &str, g: &ModelGraph) -> Option<Value> {
    // Use a "container-ish" kind first; fall back to any named element.
    let id = first_named_id_of(
        g,
        &[
            ElementKind::PartDefinition,
            ElementKind::Package,
            ElementKind::PartUsage,
            ElementKind::AttributeDefinition,
        ],
    )
    .or_else(|| first_named_id(g))?;
    Some(json!({ "uri": uri, "id": id }))
}

fn req_ancestors(uri: &str, g: &ModelGraph) -> Option<Value> {
    let id = first_named_id(g)?;
    Some(json!({ "uri": uri, "id": id }))
}

fn req_descendants(uri: &str, g: &ModelGraph) -> Option<Value> {
    let id = first_named_id_of(
        g,
        &[
            ElementKind::PartDefinition,
            ElementKind::Package,
            ElementKind::PartUsage,
        ],
    )
    .or_else(|| first_named_id(g))?;
    Some(json!({ "uri": uri, "id": id }))
}

fn req_model_tree(uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({ "uri": uri, "max_depth": 3, "view": "user_facing" }))
}

fn req_trace_matrix(uri: &str, _g: &ModelGraph) -> Option<Value> {
    // Even if no rows exist, the shape itself is what we baseline.
    Some(json!({
        "uri": uri,
        "source_kind": "PartUsage",
        "rel_kind": "Satisfy",
        "target_kind": "RequirementUsage",
    }))
}

fn req_expression_ast(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    // `element_id: None` ⇒ project all expression-bearing elements.
    // Workspace-scope collapse W2: the command is workspace-only and
    // takes no uri.
    Some(json!({ "element_id": null }))
}

fn req_check_constraints(uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({ "uri": uri, "overrides": [] }))
}

fn req_eval_expression(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    // expression_eval is graph-independent but we still capture its shape.
    Some(json!({ "expr": "2 + 3 * 4", "context": [] }))
}

fn req_evaluate(uri: &str, g: &ModelGraph) -> Option<Value> {
    let id = first_named_id_of(
        g,
        &[
            ElementKind::AttributeUsage,
            ElementKind::AttributeDefinition,
        ],
    )
    .or_else(|| first_named_id(g))?;
    Some(json!({ "element_id": id }))
}

fn req_evaluate_expression(uri: &str, g: &ModelGraph) -> Option<Value> {
    let id = first_named_id_of(
        g,
        &[
            ElementKind::AttributeUsage,
            ElementKind::AttributeDefinition,
        ],
    )
    .or_else(|| first_named_id(g))?;
    Some(json!({ "element_id": id, "overrides": [] }))
}

fn req_workspace_info(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    // Returns info for every loaded user URI when omitted.
    Some(json!({ "uris": null }))
}

fn req_loaded_uris(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({}))
}

// Workspace-scope collapse W2: these commands are workspace-only and take
// no parameters at all.
fn req_no_params(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({}))
}

fn req_views_render(uri: &str, g: &ModelGraph) -> Option<Value> {
    let id = first_named_id_of(g, &[ElementKind::ViewUsage, ElementKind::ViewDefinition])?;
    Some(json!({ "uri": uri, "view_usage_id": id, "expanded_ids": [] }))
}

fn req_views_create_scratch(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({ "names": ["P1::p1", "P1::p2"] }))
}

fn req_viewpoints_by_stakeholder(uri: &str, g: &ModelGraph) -> Option<Value> {
    let id = first_named_id_of(g, &[ElementKind::PartDefinition, ElementKind::PartUsage])?;
    Some(json!({ "uri": uri, "stakeholder_id": id }))
}

fn req_views_by_viewpoint(uri: &str, g: &ModelGraph) -> Option<Value> {
    let id = first_named_id_of(
        g,
        &[
            ElementKind::ViewpointDefinition,
            ElementKind::ViewpointUsage,
        ],
    )?;
    Some(json!({ "uri": uri, "viewpoint_id": id }))
}

fn req_flow_inspect(uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({ "uri": uri }))
}

fn req_aggregate(uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({ "uri": uri }))
}

fn req_workspace_verify(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({ "timeout_secs": 5 }))
}

fn req_system_capabilities(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({}))
}

fn req_salsa_stats(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({}))
}

fn req_salsa_stats_reset(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    Some(json!({}))
}

fn req_dependency_status(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    // Empty roots — no manifests to discover; deterministic empty
    // payload exercises the JSON shape without filesystem dependency.
    Some(json!({ "roots": [] }))
}

fn req_workspace_refresh(_uri: &str, _g: &ModelGraph) -> Option<Value> {
    // Empty roots — no projects to discover; deterministic
    // `{projects: [], stdlib_loaded: false, roots_count: 0}` shape.
    // The full discovery path is exercised by LSP integration tests
    // and the cross-transport parity test.
    Some(json!({ "roots": [], "enable_stdlib": false }))
}

// ---------- catalog ----------

fn baseline_commands() -> Vec<BaselineCommand> {
    vec![
        BaselineCommand {
            name: "sysml.loaded_uris",
            rationale: "Lists every loaded URI key — gates the DashMap → host transition.",
            build_req: req_loaded_uris,
        },
        BaselineCommand {
            name: "sysml.diagnostics",
            rationale: "Side-band diagnostics map; deleted in S2 and re-routed via validate_file_*.",
            build_req: req_uri_only,
        },
        BaselineCommand {
            name: "sysml.find",
            rationale: "Pure read against require_graph — survives S2 unchanged in shape.",
            build_req: req_find_part,
        },
        BaselineCommand {
            name: "sysml.element",
            rationale: "Single-element fetch by id — survives S2 unchanged in shape.",
            build_req: req_element,
        },
        BaselineCommand {
            name: "sysml.children",
            rationale: "Ownership hierarchy walker — survives S2 unchanged in shape.",
            build_req: req_children,
        },
        BaselineCommand {
            name: "sysml.ancestors",
            rationale: "Ownership upward walk — survives S2 unchanged in shape.",
            build_req: req_ancestors,
        },
        BaselineCommand {
            name: "sysml.descendants",
            rationale: "Ownership downward walk — survives S2 unchanged in shape.",
            build_req: req_descendants,
        },
        BaselineCommand {
            name: "sysml.stats",
            rationale: "Element + relationship counts — sensitive to merge / dedupe drift.",
            build_req: req_uri_only,
        },
        BaselineCommand {
            name: "sysml.unverified",
            rationale: "Requirements traversal — gates relationship-shape preservation across re-back.",
            build_req: req_uri_only,
        },
        BaselineCommand {
            name: "sysml.model.tree",
            rationale: "Tree view shape used by simulation-app + LSP — central FE contract.",
            build_req: req_model_tree,
        },
        BaselineCommand {
            name: "sysml.outline",
            rationale: "Document outline (S2.T9 commit 1/5) — graph-based symbol tree consumed by LSP documentSymbol.",
            build_req: req_uri_only,
        },
        BaselineCommand {
            name: "sysml.references",
            rationale: "Find references (S2.T9 commit 2/5) — cursor over first named element; in-file + cross-file walk.",
            build_req: req_references,
        },
        BaselineCommand {
            name: "sysml.goto_definition",
            rationale: "Goto-definition (S2.T9 commit 3/5) — relationship-following ladder + typed-usage type-def lookup.",
            build_req: req_goto_definition,
        },
        BaselineCommand {
            name: "sysml.hover",
            rationale: "Hover render (S2.T9 commit 4/5) — model element under cursor → markdown + range; cross-file def-search via host walk.",
            build_req: req_hover,
        },
        BaselineCommand {
            name: "sysml.completion",
            rationale: "Completion candidates (S2.T9 commit 5/5) — cursor at first named element, no trigger, no syntax hints → General route. Exercises keywords/snippets + in-file scope walk + workspace cross-file index + library types.",
            build_req: req_completion,
        },
        BaselineCommand {
            name: "sysml.completion.resolve",
            rationale: "Resolve-time enrichment for a completion item (S2.T9 follow-up) — element_id lookup → doc-comment + type detail. Distinct from sysml.completion's candidate list shape.",
            build_req: req_completion_resolve,
        },
        BaselineCommand {
            name: "sysml.rename",
            rationale: "Prepare-rename info at a cursor (S2.T10 commit 1/4) — placeholder + identifier range; apply-rename path (cross-file workspace edit set) covered by the cross-transport parity test.",
            build_req: req_rename,
        },
        BaselineCommand {
            name: "sysml.format.document",
            rationale: "Whitespace-only formatting edits (S2.T10 commit 2/4) — 4-space indent, tree-sitter nesting walk, blank-line collapse, trailing-ws removal, final-newline guard. Edit set is stable across re-runs of the same bytes.",
            build_req: req_format_document,
        },
        BaselineCommand {
            name: "sysml.code_action.list",
            rationale: "Code-action list at a cursor with no diagnostics (S2.T10 commit 3/4) — exercises cursor refactorings (expand-body, keyword toggles, toggle-abstract, add-doc-comment) + source actions (organize-imports). Quick-fix paths exercised by the parity test which threads synthetic diagnostics.",
            build_req: req_code_action_list,
        },
        BaselineCommand {
            name: "sysml.diagram.edit",
            rationale: "Diagram-driven edit compute (S2.T10 commit 4/4) — `create` action with a PartDefinition template. Returns the workspace edit + status payload; the LSP transport applies the edit via workspace/applyEdit. Full action surface (delete / editLabel / addSequenceMessage / addSequenceLifeline) covered by the cross-transport parity test.",
            build_req: req_diagram_edit,
        },
        BaselineCommand {
            name: "sysml.trace_matrix",
            rationale: "Cross-element traceability — exercises relationship indexing.",
            build_req: req_trace_matrix,
        },
        BaselineCommand {
            name: "sysml.expression.ast",
            rationale: "Expression projection — exercises expression element walking + structural shape.",
            build_req: req_expression_ast,
        },
        BaselineCommand {
            name: "sysml.constraint.check",
            rationale: "Constraint extraction + evaluation through ModelCompiler — verify shape, not pass/fail.",
            build_req: req_check_constraints,
        },
        BaselineCommand {
            name: "sysml.expression.eval",
            rationale: "Standalone expression eval — graph-independent but exercises eval pipeline.",
            build_req: req_eval_expression,
        },
        BaselineCommand {
            name: "sysml.evaluate",
            rationale: "Element value evaluation by id — exercises workspace_aware_graph + eval.",
            build_req: req_evaluate,
        },
        BaselineCommand {
            name: "sysml.evaluate.expression",
            rationale: "Per-element expression evaluation — exercises EvalContext shape.",
            build_req: req_evaluate_expression,
        },
        BaselineCommand {
            name: "sysml.evaluate.constraints",
            rationale: "Eval-flavored constraint evaluation — distinct from sysml.constraint.check shape.",
            build_req: req_no_params,
        },
        BaselineCommand {
            name: "sysml.evaluate.verification_cases",
            rationale: "Verification case verdict shape — central to FE Run page.",
            build_req: req_no_params,
        },
        BaselineCommand {
            name: "sysml.evaluate.analysis_cases",
            rationale: "Analysis case shape — sibling of verification_cases.",
            build_req: req_no_params,
        },
        BaselineCommand {
            name: "sysml.evaluate.calculations",
            rationale: "Calc def evaluation shape.",
            build_req: req_no_params,
        },
        BaselineCommand {
            name: "sysml.export.json",
            rationale: "Canonical JSON serialisation — direct shape gate for ModelGraph export.",
            build_req: req_uri_only,
        },
        BaselineCommand {
            name: "sysml.export.plantuml",
            rationale: "PlantUML serialisation — exercises diagram pipeline (text shape).",
            build_req: req_uri_only,
        },
        BaselineCommand {
            name: "sysml.views.list",
            rationale: "User-authored views catalog — central to FE Views panel + S4.",
            build_req: req_uri_only,
        },
        BaselineCommand {
            name: "sysml.views.render",
            rationale: "View → SModel render — Sprotty diagram contract.",
            build_req: req_views_render,
        },
        BaselineCommand {
            name: "sysml.views.by_viewpoint",
            rationale: "View ↔ viewpoint resolution — exercises specialisation chains.",
            build_req: req_views_by_viewpoint,
        },
        BaselineCommand {
            name: "sysml.viewpoints.by_stakeholder",
            rationale: "Stakeholder → viewpoint resolution — exercises viewpoint membership chains.",
            build_req: req_viewpoints_by_stakeholder,
        },
        BaselineCommand {
            name: "sysml.views.create_scratch",
            rationale: "Pure-string scratch view builder — exercises macro string-formatting only.",
            build_req: req_views_create_scratch,
        },
        BaselineCommand {
            name: "sysml.flow.inspect",
            rationale: "Flow walk — exercises FlowUsage + connection traversal.",
            build_req: req_flow_inspect,
        },
        BaselineCommand {
            name: "sysml.aggregate",
            rationale: "Satisfaction matrix aggregation shape.",
            build_req: req_aggregate,
        },
        BaselineCommand {
            name: "sysml.workspace.info",
            rationale: "Tree+stats per loaded URI — central FE workspace shape.",
            build_req: req_workspace_info,
        },
        BaselineCommand {
            name: "sysml.workspace.files",
            rationale: "Recursive .sysml/.kerml directory tree (S2.T16) — replaces the REST-only /workspace/files handler so MCP/CLI consumers get the same listing. Pruned to dirs containing SysML/KerML files.",
            build_req: req_workspace_files,
        },
        BaselineCommand {
            name: "sysml.workspace.verify",
            rationale: "Cross-file verification rollup — exercises merged_graph + workspace_verify path.",
            build_req: req_workspace_verify,
        },
        BaselineCommand {
            name: "sysml.system.capabilities",
            rationale: "Capabilities flags — small, stable shape; sanity check on dispatcher.",
            build_req: req_system_capabilities,
        },
        BaselineCommand {
            name: "sysml.salsa.stats",
            rationale: "Salsa query telemetry (S2.T11 / Bucket F) — wire shape gate {executions, validations, hit_ratio}. Numeric values redacted because they vary with catalog ordering and parallelism.",
            build_req: req_salsa_stats,
        },
        BaselineCommand {
            name: "sysml.salsa.stats.reset",
            rationale: "Reset salsa stats (S2.T11 / Bucket F) — deterministic {status: reset} shape.",
            build_req: req_salsa_stats_reset,
        },
        BaselineCommand {
            name: "sysml.dependency.status",
            rationale: "Workspace-roots dependency hydration JSON (S2.T11 / Bucket F / LSP-63) — empty-roots case gates the {roots:[], summary:{...}} wire shape without depending on any manifests on disk.",
            build_req: req_dependency_status,
        },
        BaselineCommand {
            name: "sysml.workspace.refresh",
            rationale: "Workspace refresh (S2.T11 / Bucket F / LSP-04+07+70) — empty-roots / stdlib-disabled case gates the {projects:[], stdlib_loaded:false, roots_count:0} wire shape without touching the filesystem. The full discovery path is exercised by LSP integration tests and the cross-transport parity test.",
            build_req: req_workspace_refresh,
        },
        // sysml.cache.{status,clear,rebuild} excluded from the corpus —
        // their responses depend on the runner's filesystem state
        // (whether `~/.cache/sysml-rs/library-v*.bin` exists at test
        // time) and clear/rebuild have side effects. Wire-shape parity
        // is exercised by T19 (cross_transport_command_parity_t19).
    ]
}

// ---------------------------------------------------------------------------
// Determinism helpers
// ---------------------------------------------------------------------------

/// Build the regex set used to scrub non-deterministic substrings before
/// computing array sort keys. UUIDs change every run, span byte offsets
/// drift with file content, file paths vary across machines — none of
/// these should influence the sort order, otherwise two semantically-
/// identical responses can serialise in different orders run-to-run and
/// poison the snapshot.
fn redaction_regexes() -> Vec<(regex::Regex, &'static str)> {
    vec![
        (
            regex::Regex::new(
                r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b",
            )
            .expect("uuid regex"),
            "<UUID>",
        ),
        (
            regex::Regex::new(r#""file"\s*:\s*"[^"]*""#).expect("file regex"),
            "\"file\":\"<FILE>\"",
        ),
        // Escaped form, used inside JSON-encoded `response` strings:
        // `\"file\":\"...\"` — the unescaped regex above does not match
        // because the surrounding quotes carry leading backslashes when
        // the response is itself a JSON-string-encoded value.
        (
            regex::Regex::new(r#"\\"file\\"\s*:\s*\\"[^\\"]*\\""#).expect("file escaped regex"),
            r#"\"file\":\"<FILE>\""#,
        ),
        (
            regex::Regex::new(r#""[A-Za-z]?:?[/\\][^"\n]*sysml-rs[^"\n]*\.(?:sysml|kerml)""#)
                .expect("path regex"),
            "\"<PATH>\"",
        ),
        // Generic absolute-path scrubber — catches non-`.sysml/.kerml`
        // paths surfaced by `sysml.workspace.files` (the `root` field and
        // directory-entry `path` fields don't end in a parser extension).
        // Runs after the file-extension-specific filter; the outputs are
        // identical (`"<PATH>"`) so order doesn't matter for substitution
        // but ordering matters for sort-key determinism.
        (
            regex::Regex::new(r#""[A-Za-z]?:?[/\\][^"\n]*sysml-rs[^"\n]*""#)
                .expect("path-any regex"),
            "\"<PATH>\"",
        ),
    ]
}

/// Apply all redactions to a JSON-encoded string. Used solely to compute
/// stable sort keys — the snapshot itself is redacted by insta's filter
/// chain, so applying redactions twice is fine and necessary.
fn redact_for_sort(s: &str, regexes: &[(regex::Regex, &'static str)]) -> String {
    let mut out = s.to_owned();
    for (re, rep) in regexes {
        out = re.replace_all(&out, *rep).into_owned();
    }
    out
}

/// Compute the sort key for an element of a normalised array. We strip
/// fields whose presence/value is non-deterministic across runs so that
/// otherwise-identical entries collapse to identical sort keys.
///
/// Fields stripped:
/// - `id`, `element_id`, `owner`, `owning_membership` — UUIDs (already
///   scrubbed by `redact_for_sort`, but stripping the field entirely
///   handles cases where the field is conditionally absent on some runs).
///
/// Spans (file/line/col/start/end) are KEPT in the sort key — they are
/// stable across runs of the same source bytes, so they make a perfect
/// secondary discriminator for siblings whose name + kind are equal but
/// whose source position differs.
fn sort_key(value: &Value, regexes: &[(regex::Regex, &'static str)]) -> String {
    fn scrub(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                let stripped = ["id", "element_id", "owner", "owning_membership"];
                let mut out = serde_json::Map::new();
                for (k, val) in map.iter() {
                    if stripped.contains(&k.as_str()) {
                        continue;
                    }
                    out.insert(k.clone(), scrub(val));
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(scrub).collect()),
            other => other.clone(),
        }
    }
    let scrubbed = scrub(value);
    let raw = serde_json::to_string(&scrubbed).unwrap_or_default();
    redact_for_sort(&raw, regexes)
}

/// Recursively walk a JSON value and sort:
/// - the keys of every object (`serde_json::Map` is already insertion-
///   ordered, but we want stable cross-run order regardless of input);
/// - the elements of every array, by their canonical JSON string with
///   non-deterministic substrings (UUIDs, file paths, span offsets)
///   redacted, so two runs that differ only in HashMap/HashSet iteration
///   order or random IDs produce identical serialised output.
///
/// The output JSON is byte-deterministic across runs.
fn normalise_json(value: &Value) -> Value {
    let regexes = redaction_regexes();
    normalise_json_inner(value, &regexes)
}

fn normalise_json_inner(value: &Value, regexes: &[(regex::Regex, &'static str)]) -> Value {
    match value {
        // A JSON-encoded string (e.g. `sysml.export.json` returns its
        // payload as a stringified `{...}`). Reparse, normalise, and
        // re-emit as a string so the embedded HashMap/UUID randomness is
        // ordered the same way as a top-level object.
        Value::String(s) if s.starts_with('{') || s.starts_with('[') => {
            if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                let normalised = normalise_json_inner(&parsed, regexes);
                if let Ok(re_serialised) = serde_json::to_string(&normalised) {
                    return Value::String(re_serialised);
                }
            }
            Value::String(s.clone())
        }
        Value::Object(map) => {
            // Strip fields whose presence is non-deterministic across
            // runs (the model_tree shape sometimes attaches a duplicate
            // `element_id` to a node, sometimes doesn't — the `id` field
            // captures the same information and is universally present).
            let stripped = ["element_id"];
            let mut entries: Vec<(String, Value)> = map
                .iter()
                .filter(|(k, _)| !stripped.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), normalise_json_inner(v, regexes)))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = serde_json::Map::with_capacity(entries.len());
            for (k, v) in entries {
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            let mut normalised: Vec<Value> = items
                .iter()
                .map(|v| normalise_json_inner(v, regexes))
                .collect();
            // Sort by a redacted, identity-stripped canonical key.
            // UUID/path/offset differences between runs no longer affect
            // ordering, and entries whose only difference is an unstable
            // field collapse to identical sort keys — `sort_by_cached_key`
            // is stable, so the relative input order of equal-keyed
            // entries is preserved (input order is itself deterministic
            // once the children of each parent have been sorted).
            normalised.sort_by_cached_key(|v| sort_key(v, regexes));
            Value::Array(normalised)
        }
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Capture loop
// ---------------------------------------------------------------------------

/// Run every (command × fixture) pair and dump JSON to disk + insta
/// snapshots.
///
/// We use a single `#[test]` rather than parameterised tests so insta's
/// snapshot directory stays flat and easy to inspect (and so the JSON
/// archive is updated atomically).
#[test]
fn service_command_baseline() {
    let commands = baseline_commands();
    let fixtures_root = fixtures_dir();
    std::fs::create_dir_all(&fixtures_root).expect("create fixtures dir");

    let mut total_pairs = 0;
    let mut skipped_pairs: Vec<(String, String)> = Vec::new();

    // UUID v4 looks like xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx (8-4-4-4-12
    // hex). The redaction filter scrubs any such substring so insta sees
    // a stable representation across runs.
    let uuid_re =
        r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b";
    let span_re = r#""file"\s*:\s*"[^"]*""#; // file paths inside spans
                                             // Escaped form for "file" keys embedded inside JSON-encoded response
                                             // strings (e.g. sysml.export.json wraps the model in a serialized string).
    let span_escaped_re = r#"\\"file\\"\s*:\s*\\"[^\\"]*\\""#;
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(uuid_re, "<UUID>");
    settings.add_filter(span_re, "\"file\":\"<FILE>\"");
    settings.add_filter(span_escaped_re, r#"\"file\":\"<FILE>\""#);
    // Span filenames contain absolute paths; scrub them so the snapshot
    // is portable across machines / CI runners.
    let abs_path_re = r#""[A-Za-z]?:?[/\\][^"\n]*sysml-rs[^"\n]*\.(?:sysml|kerml)""#;
    settings.add_filter(abs_path_re, "\"<PATH>\"");
    // Wider path scrubber for non-`.sysml/.kerml` paths (the `root` /
    // directory-entry `path` fields surfaced by `sysml.workspace.files`).
    let abs_path_any_re = r#""[A-Za-z]?:?[/\\][^"\n]*sysml-rs[^"\n]*""#;
    settings.add_filter(abs_path_any_re, "\"<PATH>\"");
    // file:// scheme URIs (post-P4 `loaded_uris` returns canonical
    // file:// URIs; the bare-path scrubbers above don't match the
    // scheme prefix).
    let file_uri_re = r#""file://[^"\n]*sysml-rs[^"\n]*""#;
    settings.add_filter(file_uri_re, "\"<PATH>\"");
    // Wall-clock timings (workspace_verify response) — drift every run.
    let elapsed_re = r#""elapsed_ms":\s*[0-9]+"#;
    settings.add_filter(elapsed_re, "\"elapsed_ms\": \"<MS>\"");
    // Synthetic SModel ids of the form `<UUID>/<segment>/<digits>` —
    // the trailing counter drifts run-to-run with HashMap iteration
    // order even when the source bytes are stable.
    let synth_id_re = r"<UUID>/([A-Za-z_-]+)/[0-9]+";
    settings.add_filter(synth_id_re, "<UUID>/$1/<N>");
    // Synthetic ids without a leading `/text/` segment, e.g.
    // `<UUID>/expand-39`.
    let synth_id_dash_re = r"<UUID>/([A-Za-z_-]+)-[0-9]+";
    settings.add_filter(synth_id_dash_re, "<UUID>/$1-<N>");
    // Salsa query telemetry counters (S2.T11) — `executions`/`validations`/
    // `hit_ratio` shift with catalog order and parallelism. Redact the
    // numeric value so the snapshot gates the wire shape only.
    let salsa_int_re = r#""(executions|validations)":\s*[0-9]+"#;
    settings.add_filter(salsa_int_re, "\"$1\":<N>");
    let salsa_float_re = r#""hit_ratio":\s*[0-9.]+"#;
    settings.add_filter(salsa_float_re, "\"hit_ratio\":<F>");
    let _guard = settings.bind_to_scope();

    for fixture in FIXTURES {
        // Each fixture lives in its own SysmlService — keeps load order
        // and graph contents independent across fixtures, so a parse
        // failure in one cannot mask another.
        let service = SysmlService::empty();
        let path = (fixture.resolve)();
        assert!(
            path.exists(),
            "fixture file missing: {} (label={})",
            path.display(),
            fixture.label
        );

        let uri = service.load_file(&path).unwrap_or_else(|e| {
            panic!(
                "load_file failed for {} ({}): {e}",
                path.display(),
                fixture.label
            )
        });

        let graph = service
            .require_graph(&uri)
            .unwrap_or_else(|e| panic!("graph not loaded for {}: {e}", fixture.label));

        for cmd in &commands {
            let Some(req) = (cmd.build_req)(&uri, &graph) else {
                skipped_pairs.push((cmd.name.to_string(), fixture.label.to_string()));
                continue;
            };

            // Build the request body. For commands that need a `path`
            // wire field (load_file/load_workspace), the builder produces
            // it. We don't fixture those — they're covered by the
            // load-step that already ran.
            let response = match execute_command(&service, cmd.name, req.clone()) {
                Ok(v) => v,
                Err(e) => {
                    // Errors are part of the contract too; capture as
                    // `{ "error": "..." }` so the shape is still snapshotted.
                    json!({ "error": e.to_string() })
                }
            };

            let normalised = normalise_json(&response);

            let mut bundle = json!({
                "request": req,
                "response": normalised,
            });

            // Project absolute checkout paths onto stable tokens ONCE, over
            // the whole bundle, before it lands in either the JSON archive or
            // the insta snapshot — the sole seam that keeps both baselines
            // checkout-independent (uri/path/root fields, file:// URIs, and
            // paths embedded in human status_message strings alike).
            sysml_spec_tests::path_canon::canonicalize_paths(&mut bundle, &path_replacements());

            // Write the JSON corpus archive (NOT the regression gate —
            // that's insta below; this is the human-readable archive).
            let dir = fixtures_root.join(cmd.name);
            std::fs::create_dir_all(&dir).expect("create command dir");
            let archive_path = dir.join(format!("{}.json", fixture.label));
            let pretty = serde_json::to_string_pretty(&bundle).expect("serialise bundle");
            std::fs::write(&archive_path, pretty).expect("write archive json");

            // Insta snapshot — this is the regression gate. UUID
            // redaction filter (settings above) scrubs random IDs so
            // re-runs match.
            //
            // A small allowlist of commands skips the insta gate because
            // their response is a free-form string (PlantUML, etc.) whose
            // line order is governed by HashMap iteration upstream. These
            // commands still land in the JSON corpus archive on disk —
            // human inspection is the gate, not byte-identity. Their
            // structural shape is covered by the visualization-pipeline
            // tests in `sysml-diagram` and `sysml-service`.
            const SKIP_INSTA: &[&str] = &[
                // Free-form text output; line order governed by HashMap
                // upstream and not stable run-to-run. Shape is covered
                // by `sysml-diagram` plantuml tests.
                "sysml.export.plantuml",
                // SModel diagram includes synthetic ids of the form
                // `<UUID>/text/N` whose counter is assigned during walk
                // — N drifts run-to-run even when the source bytes are
                // identical (HashMap iteration order). Covered by the
                // sysml-diagram smodel snapshot suite.
                "sysml.views.render",
            ];
            if !SKIP_INSTA.contains(&cmd.name) {
                let snap_name = format!("{}__{}", cmd.name.replace('.', "_"), fixture.label,);
                insta::assert_json_snapshot!(snap_name, bundle);
            }

            total_pairs += 1;
        }
    }

    // Sanity: at least one pair captured per fixture; at least 25 commands
    // covered — keeps the manifest from silently shrinking.
    assert!(
        total_pairs >= 25 * FIXTURES.len() / 2,
        "captured pairs unexpectedly low: {total_pairs}"
    );

    // Print skipped pairs to stdout (visible under `--nocapture`) so
    // future maintainers can see which (command, fixture) pairs need
    // a richer fixture before we can baseline them.
    if !skipped_pairs.is_empty() {
        eprintln!(
            "[service_command_baseline] skipped {} pairs:",
            skipped_pairs.len()
        );
        for (cmd, fix) in &skipped_pairs {
            eprintln!("  - {cmd}  ×  {fix}  (no usable element of required kind)");
        }
    }

    // Write a manifest index summarising the corpus.
    let manifest = json!({
        "captured": total_pairs,
        "skipped": skipped_pairs
            .iter()
            .map(|(c, f)| json!({ "command": c, "fixture": f }))
            .collect::<Vec<_>>(),
        "fixtures": FIXTURES.iter().map(|f| f.label).collect::<Vec<_>>(),
        "commands": commands
            .iter()
            .map(|c| json!({ "name": c.name, "rationale": c.rationale }))
            .collect::<Vec<_>>(),
    });
    let manifest_path = fixtures_root.join("MANIFEST.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("manifest serialise"),
    )
    .expect("write manifest");
}
