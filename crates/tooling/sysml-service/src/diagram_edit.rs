//! Diagram-driven edit compute — replaces the LSP-side `diagram_edit.rs`.
//!
//! The diagram webview emits a `DiagramEditRequest` (action + URI + payload).
//! This module computes the resulting `WorkspaceEdit` (line/col-based, no
//! `tower-lsp` dependency) and a status payload. Applying the edit (via
//! `workspace/applyEdit`) stays on the LSP side.

use std::collections::HashMap;

use serde::Deserialize;
use sysml_core::{ElementKind, ModelGraph};
use sysml_id::ElementId;
use sysml_ide_db::{AnalysisHost, Cancelled};

use crate::error::ServiceError;
use crate::position::offset_to_line_col;
use crate::text_edit::TextEdit;

// ─── Wire types ───────────────────────────────────────────────────────────────

/// Diagram edit request from the webview.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramEditRequest {
    pub uri: String,
    #[serde(flatten)]
    pub action: DiagramEditAction,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum DiagramEditAction {
    #[serde(rename_all = "camelCase")]
    Create {
        element_type_id: String,
        container_id: Option<String>,
        #[serde(default)]
        args: HashMap<String, String>,
    },
    #[serde(rename_all = "camelCase")]
    Delete { element_ids: Vec<String> },
    #[serde(rename_all = "camelCase")]
    EditLabel {
        element_id: String,
        new_text: String,
    },
    #[serde(rename_all = "camelCase")]
    AddSequenceMessage {
        lifeline_id: String,
        insertion_index: u32,
    },
    #[serde(rename_all = "camelCase")]
    AddSequenceLifeline,
}

/// Result of computing an edit. The LSP applies it via `workspace/applyEdit`
/// and synthesizes a final status response from these fields.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiagramEditComputed {
    /// The action name (`create` / `delete` / `editLabel` / …).
    pub action: String,
    /// The URI being edited.
    pub uri: String,
    /// The computed workspace edit. Empty `changes` means no edit.
    pub workspace_edit: DiagramWorkspaceEdit,
    /// Status message to surface to the user on success
    /// (replaces the LSP-side `show_command_result` content).
    pub status_message: String,
    /// Per-action status fields for the JSON-RPC response
    /// (e.g. `elementType`, `deletedCount`, `lifelineId`, `newText`).
    pub status_payload: serde_json::Value,
    /// `Delete`-only — element IDs that didn't have a span.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_found: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DiagramWorkspaceEdit {
    pub changes: Vec<DiagramFileEdits>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiagramFileEdits {
    pub uri: String,
    pub edits: Vec<TextEdit>,
}

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn compute_diagram_edit(
    host: &std::sync::Mutex<AnalysisHost>,
    request: &serde_json::Value,
) -> Result<DiagramEditComputed, ServiceError> {
    let req: DiagramEditRequest = serde_json::from_value(request.clone())
        .map_err(|e| ServiceError::InvalidInput(format!("invalid diagram edit request: {e}")))?;

    let uri = req.uri.clone();

    // Fetch document content and parsed graph via salsa.
    let (content, graph_arc) = {
        let guard = host.lock().unwrap();
        let Some(file_id) = guard.file_id(&uri) else {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        };
        let Some(sf) = guard.source_file(file_id) else {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        };
        let analysis = guard.analysis();
        let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            let content = analysis.file_text(sf).to_owned();
            let parsed = analysis.parse_file(sf);
            let graph = parsed.graph().clone();
            (content, std::sync::Arc::new(graph))
        }))
        .map_err(|_| ServiceError::Internal("salsa cancelled".into()))?;
        drop(analysis);
        result
    };
    let graph: &ModelGraph = &graph_arc;

    match req.action {
        DiagramEditAction::Create {
            element_type_id,
            container_id,
            args: _,
        } => Ok(create_element(
            &uri,
            &element_type_id,
            container_id.as_deref(),
            &content,
            graph,
        )),

        DiagramEditAction::Delete { element_ids } => {
            Ok(delete_elements(&uri, &element_ids, &content, graph))
        }

        DiagramEditAction::EditLabel {
            element_id,
            new_text,
        } => edit_label(&uri, &element_id, &new_text, &content, graph),

        DiagramEditAction::AddSequenceMessage {
            lifeline_id,
            insertion_index,
        } => Ok(add_sequence_message(
            &uri,
            &content,
            &lifeline_id,
            insertion_index,
        )),

        DiagramEditAction::AddSequenceLifeline => Ok(add_sequence_lifeline(&uri, &content)),
    }
}

// ─── Per-action implementations ───────────────────────────────────────────────

fn create_element(
    uri: &str,
    element_type_id: &str,
    container_id: Option<&str>,
    document_text: &str,
    graph: &ModelGraph,
) -> DiagramEditComputed {
    let template = element_template(element_type_id);
    let lines: Vec<&str> = document_text.lines().collect();
    let line_count = lines.len();

    let (insert_line, indent) = if let Some(container_id_str) = container_id {
        let eid = ElementId::from_string(container_id_str);
        graph
            .get_element(&eid)
            .and_then(|elem| elem.spans.first())
            .and_then(|span| find_container_insertion_point(document_text, span.end))
            .unwrap_or_else(|| find_insertion_point(&lines, line_count))
    } else {
        find_insertion_point(&lines, line_count)
    };

    let new_text = format!("{}{}\n\n", indent, template);

    DiagramEditComputed {
        action: "create".to_owned(),
        uri: uri.to_owned(),
        workspace_edit: single_edit(
            uri,
            insert_line as u32,
            0,
            insert_line as u32,
            0,
            new_text,
            None,
        ),
        status_message: format!("Created {element_type_id} in {uri}"),
        status_payload: serde_json::json!({
            "elementType": element_type_id,
            "uri": uri,
        }),
        not_found: Vec::new(),
    }
}

fn delete_elements(
    uri: &str,
    element_ids: &[String],
    document_text: &str,
    graph: &ModelGraph,
) -> DiagramEditComputed {
    let mut edits: Vec<TextEdit> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();

    for eid_str in element_ids {
        let eid = ElementId::from_string(eid_str);
        if let Some(elem) = graph.get_element(&eid) {
            if let Some(span) = elem.spans.first() {
                let (line_start, col_start) = offset_to_line_col(span.start, document_text);
                let (line_end, col_end) = offset_to_line_col(span.end, document_text);
                edits.push(TextEdit {
                    expected_old_text: None,
                    line_start,
                    col_start,
                    line_end,
                    col_end,
                    new_text: String::new(),
                });
            } else {
                not_found.push(eid_str.clone());
            }
        } else {
            not_found.push(eid_str.clone());
        }
    }

    // Sort reverse so earlier edits don't invalidate later offsets.
    edits.sort_by(|a, b| {
        b.line_start
            .cmp(&a.line_start)
            .then(b.col_start.cmp(&a.col_start))
    });

    let deleted = edits.len();
    let mut status_payload = serde_json::json!({
        "deletedCount": deleted,
        "uri": uri,
    });
    if !not_found.is_empty() {
        status_payload["notFound"] = serde_json::json!(not_found.clone());
    }

    let workspace_edit = if edits.is_empty() {
        DiagramWorkspaceEdit::default()
    } else {
        DiagramWorkspaceEdit {
            changes: vec![DiagramFileEdits {
                uri: uri.to_owned(),
                edits,
            }],
        }
    };

    DiagramEditComputed {
        action: "delete".to_owned(),
        uri: uri.to_owned(),
        workspace_edit,
        status_message: format!("Deleted {deleted} element(s) from {uri}"),
        status_payload,
        not_found,
    }
}

fn edit_label(
    uri: &str,
    element_id: &str,
    new_text: &str,
    document_text: &str,
    graph: &ModelGraph,
) -> Result<DiagramEditComputed, ServiceError> {
    let eid = ElementId::from_string(element_id);
    let elem = graph.get_element(&eid).ok_or_else(|| {
        ServiceError::ElementNotFound(format!("element '{element_id}' not found"))
    })?;
    let span = elem.name_span.as_ref().ok_or_else(|| {
        ServiceError::InvalidInput(format!("no name span found for element '{element_id}'"))
    })?;
    let (line_start, col_start) = offset_to_line_col(span.start, document_text);
    let (line_end, col_end) = offset_to_line_col(span.end, document_text);

    Ok(DiagramEditComputed {
        action: "editLabel".to_owned(),
        uri: uri.to_owned(),
        // Staleness guard (workbench design §7.2): the text currently at the
        // name span — a buffer-applying client must verify before splicing.
        workspace_edit: single_edit(
            uri,
            line_start,
            col_start,
            line_end,
            col_end,
            new_text.to_owned(),
            document_text.get(span.start..span.end).map(str::to_owned),
        ),
        status_message: format!("Renamed element '{element_id}' to '{new_text}'"),
        status_payload: serde_json::json!({
            "elementId": element_id,
            "newText": new_text,
            "uri": uri,
        }),
        not_found: Vec::new(),
    })
}

fn add_sequence_message(
    uri: &str,
    document_text: &str,
    lifeline_id: &str,
    _insertion_index: u32,
) -> DiagramEditComputed {
    let lines: Vec<&str> = document_text.lines().collect();
    let line_count = lines.len();
    let (insert_line, indent) = find_insertion_point(&lines, line_count);
    let new_text = format!("{}action newStep;\n", indent);

    DiagramEditComputed {
        action: "addSequenceMessage".to_owned(),
        uri: uri.to_owned(),
        workspace_edit: single_edit(
            uri,
            insert_line as u32,
            0,
            insert_line as u32,
            0,
            new_text,
            None,
        ),
        status_message: format!("Added sequence message on lifeline '{lifeline_id}'"),
        status_payload: serde_json::json!({
            "lifelineId": lifeline_id,
            "uri": uri,
        }),
        not_found: Vec::new(),
    }
}

fn add_sequence_lifeline(uri: &str, document_text: &str) -> DiagramEditComputed {
    let lines: Vec<&str> = document_text.lines().collect();
    let line_count = lines.len();
    let (insert_line, indent) = find_insertion_point(&lines, line_count);
    let new_text = format!(
        "\n{}action def NewParticipantAction {{\n{}    action step;\n{}}}\n{}action newParticipant : NewParticipantAction;\n",
        indent, indent, indent, indent,
    );

    DiagramEditComputed {
        action: "addSequenceLifeline".to_owned(),
        uri: uri.to_owned(),
        workspace_edit: single_edit(
            uri,
            insert_line as u32,
            0,
            insert_line as u32,
            0,
            new_text,
            None,
        ),
        status_message: format!("Added new lifeline to {uri}"),
        status_payload: serde_json::json!({ "uri": uri }),
        not_found: Vec::new(),
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn single_edit(
    uri: &str,
    line_start: u32,
    col_start: u32,
    line_end: u32,
    col_end: u32,
    new_text: String,
    expected_old_text: Option<String>,
) -> DiagramWorkspaceEdit {
    DiagramWorkspaceEdit {
        changes: vec![DiagramFileEdits {
            uri: uri.to_owned(),
            edits: vec![TextEdit {
                expected_old_text,
                line_start,
                col_start,
                line_end,
                col_end,
                new_text,
            }],
        }],
    }
}

/// Generate the SysML v2 text template for a given element kind string.
pub fn element_template(kind_str: &str) -> String {
    ElementKind::from_str(kind_str)
        .and_then(|kind| kind.text_template())
        .unwrap_or_else(|| format!("/* unknown element: {} */", kind_str))
}

/// Check if a container element can hold a child of the given kind.
pub fn validate_containment(container_kind: Option<ElementKind>, child_kind: ElementKind) -> bool {
    match container_kind {
        Some(container) => container.can_contain(child_kind),
        None => true,
    }
}

/// What relationship types can connect two elements.
pub fn valid_connections(source_kind: ElementKind, target_kind: ElementKind) -> Vec<ElementKind> {
    ElementKind::iter()
        .filter(|rel_kind| {
            if !rel_kind.is_relationship() {
                return false;
            }
            if rel_kind.syntax_keyword().is_none() {
                return false;
            }
            let source_ok = rel_kind
                .relationship_source_type()
                .is_some_and(|req| source_kind == req || source_kind.is_subtype_of(req));
            let target_ok = rel_kind
                .relationship_target_type()
                .is_some_and(|req| target_kind == req || target_kind.is_subtype_of(req));
            source_ok && target_ok
        })
        .collect()
}

/// List of element kinds that can be created inside a given container.
pub fn creatable_children(container_kind: Option<ElementKind>) -> Vec<ElementKind> {
    ElementKind::iter()
        .filter(|kind| {
            kind.syntax_keyword().is_some()
                && validate_containment(container_kind.clone(), kind.clone())
        })
        .collect()
}

fn find_container_insertion_point(document_text: &str, span_end: usize) -> Option<(usize, String)> {
    let text_before_end = &document_text[..span_end.min(document_text.len())];
    let brace_pos = text_before_end.rfind('}')?;
    let line = text_before_end[..brace_pos].matches('\n').count();
    let line_start = text_before_end[..brace_pos]
        .rfind('\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    let brace_line = &text_before_end[line_start..brace_pos];
    let leading_ws = brace_line.len() - brace_line.trim_start().len();
    let indent = " ".repeat(leading_ws + 4);
    Some((line, indent))
}

fn find_insertion_point(lines: &[&str], line_count: usize) -> (usize, String) {
    for i in (0..line_count).rev() {
        let trimmed = lines[i].trim();
        if trimmed == "}" {
            let leading_ws = lines[i].len() - lines[i].trim_start().len();
            let indent = " ".repeat(leading_ws + 4);
            return (i, indent);
        }
    }
    (line_count, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_template_part_def() {
        assert!(element_template("PartDefinition").contains("part def NewPart"));
    }

    #[test]
    fn element_template_unknown() {
        assert!(element_template("TotallyFake").contains("unknown"));
    }

    #[test]
    fn validate_containment_top_level() {
        assert!(validate_containment(None, ElementKind::PartDefinition));
    }

    #[test]
    fn creatable_children_package() {
        let pkg = creatable_children(Some(ElementKind::Package));
        assert!(pkg.contains(&ElementKind::PartDefinition));
    }
}
