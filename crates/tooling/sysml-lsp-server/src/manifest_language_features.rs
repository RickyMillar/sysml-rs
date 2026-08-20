//! LSP editing helpers for `sysml.toml` files.

use std::collections::HashMap;
use std::path::PathBuf;

use sysml_manifest::StdlibConfig;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Command as LspCommand, CompletionItem,
    CompletionItemKind, CompletionResponse, CreateFile, CreateFileOptions, Diagnostic,
    DocumentChangeOperation, DocumentChanges, DocumentLink, InsertTextFormat, NumberOrString,
    Position, Range, ResourceOp, TextEdit, Url, WorkspaceEdit,
};

use crate::utils::parse_uri;

const SECTION_HEADERS: &[&str] = &[
    "[project]",
    "[workspace]",
    "[workspace.project]",
    "[dependencies]",
    "[stdlib]",
    "[package]",
];

const PROJECT_KEYS: &[&str] = &[
    "name",
    "version",
    "description",
    "license",
    "sysml-edition",
    "authors",
];
const WORKSPACE_KEYS: &[&str] = &["members", "exclude", "default-members"];
const WORKSPACE_PROJECT_KEYS: &[&str] = &["sysml-edition", "license", "version"];
const STDLIB_KEYS: &[&str] = &["include_only", "exclude"];
const PACKAGE_KEYS: &[&str] = &["iri"];

pub(crate) fn completion_for_manifest(
    content: &str,
    position: Position,
) -> Option<CompletionResponse> {
    let line_idx = position.line as usize;
    let line = content.lines().nth(line_idx).unwrap_or("");
    let col = (position.character as usize).min(line.len());
    let prefix = &line[..col];
    let section = current_section_at_line(content, line_idx);
    let mut items: Vec<CompletionItem> = Vec::new();

    let is_value_context = prefix.contains('=') && prefix.rsplit_once('=').is_some();

    if !is_value_context {
        if prefix.trim_start().starts_with('[') || prefix.trim().is_empty() {
            items.extend(SECTION_HEADERS.iter().map(|header| CompletionItem {
                label: (*header).to_owned(),
                kind: Some(CompletionItemKind::MODULE),
                detail: Some("Manifest section".to_owned()),
                insert_text: Some((*header).to_owned()),
                ..Default::default()
            }));
        }

        match section.as_deref() {
            Some("project") => items.extend(key_items(PROJECT_KEYS)),
            Some("workspace") => items.extend(key_items(WORKSPACE_KEYS)),
            Some("workspace.project") => items.extend(key_items(WORKSPACE_PROJECT_KEYS)),
            Some("stdlib") => items.extend(key_items(STDLIB_KEYS)),
            Some("package") => items.extend(key_items(PACKAGE_KEYS)),
            Some("dependencies") => {
                items.push(snippet_item(
                    "path dependency",
                    "my-lib = { path = \"../my-lib\" }",
                    "Add a local path dependency",
                ));
                items.push(snippet_item(
                    "git dependency",
                    "my-lib = { git = \"https://github.com/org/repo\", rev = \"<commit>\" }",
                    "Add a pinned git dependency",
                ));
                items.push(snippet_item(
                    "kpar dependency",
                    "my-lib = { kpar = \"https://example.com/lib.kpar\" }",
                    "Add a KPAR dependency",
                ));
                items.push(snippet_item(
                    "version dependency",
                    "my-lib = \"^1.0\"",
                    "Add a registry/version dependency",
                ));
            }
            _ => {}
        }
    } else {
        let key = key_before_equals(prefix).unwrap_or_default();
        match (section.as_deref(), key.as_str()) {
            (_, "sysml-edition") => {
                items.push(value_item("\"2025\"", "Current SysML edition"));
                items.push(value_item("\"2024\"", "Previous SysML edition"));
            }
            (_, "license") => {
                for spdx in ["MIT", "Apache-2.0", "BSD-3-Clause", "GPL-3.0-only"] {
                    items.push(value_item(&format!("\"{spdx}\""), "SPDX license"));
                }
            }
            (Some("stdlib"), "include_only") => {
                items.push(value_item("[]", "Array of stdlib library names"));
                for name in StdlibConfig::known_library_names() {
                    items.push(value_item(
                        &format!("\"{name}\""),
                        "Include this standard library",
                    ));
                }
            }
            (Some("stdlib"), "exclude") => {
                items.push(value_item("[]", "Array of stdlib library names"));
                items.push(value_item("\"all\"", "Disable all standard libraries"));
                items.push(value_item("\"*\"", "Disable all standard libraries"));
                for name in StdlibConfig::known_library_names() {
                    items.push(value_item(
                        &format!("\"{name}\""),
                        "Exclude this standard library",
                    ));
                }
            }
            (Some("workspace"), "members")
            | (Some("workspace"), "exclude")
            | (Some("workspace"), "default-members") => {
                items.push(value_item("[]", "Array of workspace-relative paths"));
            }
            _ => {}
        }
    }

    if items.is_empty() {
        None
    } else {
        Some(CompletionResponse::Array(items))
    }
}

pub(crate) fn document_links_for_manifest(uri: &str, content: &str) -> Vec<DocumentLink> {
    let mut links = Vec::new();
    let manifest_dir = parse_uri(uri)
        .and_then(|url| url.to_file_path().ok())
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    let Ok(value) = toml::from_str::<toml::Value>(content) else {
        return links;
    };
    let Some(root) = value.as_table() else {
        return links;
    };

    if let Some(deps) = root.get("dependencies").and_then(|v| v.as_table()) {
        for (name, dep) in deps {
            let range = find_dependency_line_range(content, name);
            if let Some(url) = dependency_target_url(dep, manifest_dir.as_ref()) {
                links.push(DocumentLink {
                    range,
                    target: Some(url),
                    tooltip: Some(format!("Dependency: {name}")),
                    data: None,
                });
            }
        }
    }

    if let Some(workspace) = root.get("workspace").and_then(|v| v.as_table()) {
        for key in ["members", "exclude", "default-members"] {
            let Some(values) = workspace.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for entry in values.iter().filter_map(|v| v.as_str()) {
                let Some(dir) = manifest_dir.as_ref() else {
                    continue;
                };
                let target = dir.join(entry);
                let Ok(url) = Url::from_file_path(&target) else {
                    continue;
                };
                links.push(DocumentLink {
                    range: find_key_value_line_range(content, key),
                    target: Some(url),
                    tooltip: Some(format!("Workspace path: {entry}")),
                    data: None,
                });
            }
        }
    }

    links
}

pub(crate) fn code_actions_for_manifest(
    uri: &str,
    content: &str,
    diagnostics: &[Diagnostic],
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();

    for diagnostic in diagnostics {
        let code = diagnostic.code.as_ref().and_then(|code| match code {
            NumberOrString::String(s) => Some(s.as_str()),
            _ => None,
        });

        match code {
            Some("M001") => {
                if let Some(action) = unknown_key_fix(uri, content, diagnostic) {
                    actions.push(action);
                }
            }
            Some("M020") | Some("M024") => {
                if let Some(action) = pin_git_rev_fix(uri, content, diagnostic) {
                    actions.push(action);
                }
            }
            Some("M035") => {
                if let Some(action) = create_missing_member_dir_fix(uri, diagnostic) {
                    actions.push(action);
                }
            }
            Some("M040") => {
                actions.push(dependency_status_command_action(diagnostic));
            }
            Some("M041") => {
                actions.push(dependency_update_command_action(diagnostic));
            }
            _ => {}
        }
    }

    actions
}

fn key_items(keys: &[&str]) -> Vec<CompletionItem> {
    keys.iter()
        .map(|key| CompletionItem {
            label: (*key).to_owned(),
            kind: Some(CompletionItemKind::FIELD),
            insert_text: Some(format!("{key} = ")),
            detail: Some("Manifest key".to_owned()),
            ..Default::default()
        })
        .collect()
}

fn snippet_item(label: &str, snippet: &str, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_owned(),
        kind: Some(CompletionItemKind::SNIPPET),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        insert_text: Some(snippet.to_owned()),
        detail: Some(detail.to_owned()),
        ..Default::default()
    }
}

fn value_item(label: &str, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_owned(),
        kind: Some(CompletionItemKind::VALUE),
        insert_text: Some(label.to_owned()),
        detail: Some(detail.to_owned()),
        ..Default::default()
    }
}

fn current_section_at_line(content: &str, line_idx: usize) -> Option<String> {
    let mut section = None;
    for (idx, line) in content.lines().enumerate() {
        if idx > line_idx {
            break;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = Some(
                trimmed
                    .trim_start_matches('[')
                    .trim_end_matches(']').to_owned(),
            );
        }
    }
    section
}

fn key_before_equals(prefix: &str) -> Option<String> {
    let (left, _) = prefix.rsplit_once('=')?;
    let key = left
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    if key.is_empty() {
        None
    } else {
        Some(key)
    }
}

fn dependency_target_url(dep: &toml::Value, manifest_dir: Option<&PathBuf>) -> Option<Url> {
    if let Some(version) = dep.as_str() {
        let encoded = version.replace('"', "");
        return Url::parse(&format!("https://pkg.sysml.rs/{encoded}")).ok();
    }

    let table = dep.as_table()?;
    if let Some(path) = table.get("path").and_then(|v| v.as_str()) {
        let root = manifest_dir?;
        let mut target = root.join(path);
        if target.is_dir() {
            let manifest = target.join("sysml.toml");
            if manifest.is_file() {
                target = manifest;
            }
        }
        return Url::from_file_path(target).ok();
    }
    if let Some(git) = table.get("git").and_then(|v| v.as_str()) {
        return Url::parse(git).ok();
    }
    if let Some(kpar) = table.get("kpar").and_then(|v| v.as_str()) {
        return Url::parse(kpar).ok();
    }

    None
}

fn find_dependency_line_range(content: &str, dep_name: &str) -> Range {
    let mut in_deps = false;
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == "[dependencies]" {
            in_deps = true;
            continue;
        }
        if in_deps && trimmed.starts_with('[') {
            break;
        }
        if in_deps && trimmed.starts_with(dep_name) {
            return Range {
                start: Position::new(line_idx as u32, 0),
                end: Position::new(line_idx as u32, line.len() as u32),
            };
        }
    }
    Range {
        start: Position::new(0, 0),
        end: Position::new(0, 0),
    }
}

fn find_key_value_line_range(content: &str, key: &str) -> Range {
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with(key) {
            let rest = trimmed[key.len()..].trim_start();
            if rest.starts_with('=') {
                return Range {
                    start: Position::new(line_idx as u32, 0),
                    end: Position::new(line_idx as u32, line.len() as u32),
                };
            }
        }
    }
    Range {
        start: Position::new(0, 0),
        end: Position::new(0, 0),
    }
}

fn unknown_key_fix(
    uri: &str,
    content: &str,
    diagnostic: &Diagnostic,
) -> Option<CodeActionOrCommand> {
    let data = diagnostic.data.as_ref()?;
    let key = data.get("key")?.as_str()?;
    let suggestion = data.get("suggestion")?.as_str()?;
    let line_idx = diagnostic.range.start.line as usize;
    let line = content.lines().nth(line_idx)?;
    let col = line.find(key)?;
    let replace_range = Range {
        start: Position::new(line_idx as u32, col as u32),
        end: Position::new(line_idx as u32, (col + key.len()) as u32),
    };

    let mut changes = HashMap::new();
    let url = Url::parse(uri).ok()?;
    changes.insert(
        url,
        vec![TextEdit {
            range: replace_range,
            new_text: suggestion.to_owned(),
        }],
    );

    Some(CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("Replace '{}' with '{}'", key, suggestion),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        is_preferred: Some(true),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    }))
}

fn pin_git_rev_fix(
    uri: &str,
    content: &str,
    diagnostic: &Diagnostic,
) -> Option<CodeActionOrCommand> {
    let dep_name = diagnostic
        .data
        .as_ref()
        .and_then(|data| data.get("dependency"))
        .and_then(|v| v.as_str())?;

    let range = find_dependency_line_range(content, dep_name);
    let line_idx = range.start.line as usize;
    let line = content.lines().nth(line_idx)?;

    if line.contains("rev") {
        return None;
    }
    let close_brace = line.rfind('}')?;
    let insert_range = Range {
        start: Position::new(line_idx as u32, close_brace as u32),
        end: Position::new(line_idx as u32, close_brace as u32),
    };

    let mut changes = HashMap::new();
    let url = Url::parse(uri).ok()?;
    changes.insert(
        url,
        vec![TextEdit {
            range: insert_range,
            new_text: ", rev = \"<commit>\"".to_owned(),
        }],
    );

    Some(CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("Pin '{}' with git rev", dep_name),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    }))
}

fn create_missing_member_dir_fix(
    uri: &str,
    diagnostic: &Diagnostic,
) -> Option<CodeActionOrCommand> {
    let missing = diagnostic
        .data
        .as_ref()
        .and_then(|data| data.get("path"))
        .and_then(|v| v.as_str())?;

    let manifest_path = parse_uri(uri)?.to_file_path().ok()?;
    let root = manifest_path.parent()?;
    let marker = root.join(missing).join(".gitkeep");
    let marker_url = Url::from_file_path(marker).ok()?;

    let create = DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
        uri: marker_url,
        options: Some(CreateFileOptions {
            overwrite: Some(false),
            ignore_if_exists: Some(true),
        }),
        annotation_id: None,
    }));

    Some(CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("Create workspace member directory '{}'", missing),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(WorkspaceEdit {
            document_changes: Some(DocumentChanges::Operations(vec![create])),
            ..Default::default()
        }),
        ..Default::default()
    }))
}

fn dependency_status_command_action(diagnostic: &Diagnostic) -> CodeActionOrCommand {
    CodeActionOrCommand::CodeAction(CodeAction {
        title: "Run dependency status".to_owned(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        command: Some(LspCommand {
            title: "SysML: Dependency Status".to_owned(),
            command: "sysml.dependencies.status".to_owned(),
            arguments: None,
        }),
        ..Default::default()
    })
}

fn dependency_update_command_action(diagnostic: &Diagnostic) -> CodeActionOrCommand {
    CodeActionOrCommand::CodeAction(CodeAction {
        title: "Update dependencies".to_owned(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        command: Some(LspCommand {
            title: "SysML: Update Dependencies".to_owned(),
            command: "sysml.dependencies.update".to_owned(),
            arguments: None,
        }),
        ..Default::default()
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn manifest_completion_suggests_sections() {
        let items = completion_for_manifest("", Position::new(0, 0));
        assert!(items.is_some());
    }

    #[test]
    fn manifest_completion_project_keys() {
        let content = "[project]\nna";
        let items = completion_for_manifest(content, Position::new(1, 2));
        let labels = match items {
            Some(CompletionResponse::Array(items)) => {
                items.into_iter().map(|item| item.label).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        };
        assert!(labels.iter().any(|label| label == "name"));
    }

    #[test]
    fn manifest_completion_stdlib_keys_and_values() {
        let key_items = completion_for_manifest("[stdlib]\nin", Position::new(1, 2));
        let key_labels = match key_items {
            Some(CompletionResponse::Array(items)) => {
                items.into_iter().map(|item| item.label).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        };
        assert!(key_labels.iter().any(|label| label == "include_only"));

        let value_items = completion_for_manifest(
            "[stdlib]\nexclude = ",
            Position::new(1, "exclude = ".len() as u32),
        );
        let value_labels = match value_items {
            Some(CompletionResponse::Array(items)) => {
                items.into_iter().map(|item| item.label).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        };
        assert!(value_labels.iter().any(|label| label == "\"all\""));
        assert!(value_labels.iter().any(|label| label == "\"analysis\""));
    }

    #[test]
    fn manifest_links_detect_git_dep() {
        let content = "[project]\nname=\"x\"\nversion=\"0.1.0\"\n\n[dependencies]\nfoo = { git = \"https://example.com/repo\" }\n";
        let links = document_links_for_manifest("file:///tmp/sysml.toml", content);
        assert!(!links.is_empty());
    }

    #[test]
    fn manifest_code_actions_include_dependency_status_quickfix_for_runtime_failure() {
        let diagnostic = Diagnostic {
            range: Range::default(),
            code: Some(NumberOrString::String("M040".to_string())),
            message: "dependency runtime failure".to_string(),
            ..Default::default()
        };
        let actions = code_actions_for_manifest("file:///tmp/sysml.toml", "", &[diagnostic]);
        let has_status_action = actions.iter().any(|action| match action {
            CodeActionOrCommand::CodeAction(code_action) => code_action
                .command
                .as_ref()
                .map(|command| command.command == "sysml.dependencies.status")
                .unwrap_or(false),
            CodeActionOrCommand::Command(command) => command.command == "sysml.dependencies.status",
        });
        assert!(
            has_status_action,
            "expected dependency status quick-fix for M040 diagnostic"
        );
    }

    #[test]
    fn manifest_code_actions_include_dependency_update_quickfix_for_update_hint() {
        let diagnostic = Diagnostic {
            range: Range::default(),
            code: Some(NumberOrString::String("M041".to_string())),
            message: "dependency update available".to_string(),
            ..Default::default()
        };
        let actions = code_actions_for_manifest("file:///tmp/sysml.toml", "", &[diagnostic]);
        let has_update_action = actions.iter().any(|action| match action {
            CodeActionOrCommand::CodeAction(code_action) => code_action
                .command
                .as_ref()
                .map(|command| command.command == "sysml.dependencies.update")
                .unwrap_or(false),
            CodeActionOrCommand::Command(command) => command.command == "sysml.dependencies.update",
        });
        assert!(
            has_update_action,
            "expected dependency update quick-fix for M041 diagnostic"
        );
    }
}
