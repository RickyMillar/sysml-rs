//! Per-field text-edit computation for the requirements workbench
//! (requirements-workbench-design.md §7.2).
//!
//! The service COMPUTES a guarded [`TextEdit`]; the transport/client applies
//! it to the editor BUFFER (source text stays gospel, editor owns save —
//! never a server-side mutation). Every edit carries `expected_old_text`:
//! the client must verify the buffer slice matches before splicing and fail
//! loudly on mismatch ("stale buffer / span mismatch"), never mis-splice.
//!
//! Per-field TYPED entry points, deliberately not one generic field-edit
//! command (the same ruling that rejected the workflow store's generic
//! append: stringly fields weaken validation). Creating a field that does
//! not exist yet (no doc comment, no value, no @StatusInfo) is NOT an edit —
//! it is an insertion and ships with the declaration printer (§7.3); these
//! functions fail hard on absent targets instead of guessing an insertion
//! point.

use sysml_core::{ElementKind, ModelGraph, RelationshipKind};
use sysml_id::ElementId;

use crate::error::ServiceError;
use crate::position::offset_to_line_col;
use crate::text_edit::TextEdit;

/// A computed single-field replacement.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldEditComputed {
    /// File the edit applies to (from the edited element's span).
    pub uri: String,
    /// The element whose field is edited (the requirement / attribute usage).
    pub element_id: String,
    /// Which field: `doc` | `attribute_value` | `maturity`.
    pub field: String,
    pub edit: TextEdit,
}

/// The spec's closed `StatusKind` vocabulary (ModelingMetadata library).
/// A typo cannot be edited out of model history any more than out of an
/// audit log — invalid values die at the write boundary.
pub const STATUS_KINDS: &[&str] = &["open", "tbd", "tbr", "tbc", "done", "closed"];

fn edit_for_range(
    content: &str,
    uri: &str,
    element_id: &str,
    field: &str,
    start: usize,
    end: usize,
    new_text: String,
) -> Result<FieldEditComputed, ServiceError> {
    let expected = content.get(start..end).ok_or_else(|| {
        ServiceError::Internal(format!(
            "span {start}..{end} out of bounds for {uri} — stale parse?"
        ))
    })?;
    let (line_start, col_start) = offset_to_line_col(start, content);
    let (line_end, col_end) = offset_to_line_col(end, content);
    Ok(FieldEditComputed {
        uri: uri.to_owned(),
        element_id: element_id.to_owned(),
        field: field.to_owned(),
        edit: TextEdit {
            line_start,
            col_start,
            line_end,
            col_end,
            new_text,
            expected_old_text: Some(expected.to_owned()),
        },
    })
}

/// Replace the BODY of a requirement's (first, in document order) `doc`
/// comment: the text between `/*` and `*/`, delimiters kept.
pub fn compute_doc_edit(
    graph: &ModelGraph,
    content: &str,
    element_id: &ElementId,
    new_body: &str,
) -> Result<FieldEditComputed, ServiceError> {
    let new_body = new_body.trim();
    if new_body.is_empty() {
        return Err(ServiceError::InvalidInput(
            "doc body must not be blank — delete the doc comment in the editor instead".into(),
        ));
    }
    if new_body.contains("*/") {
        return Err(ServiceError::InvalidInput(
            "doc body must not contain the comment terminator `*/`".into(),
        ));
    }
    let mut docs: Vec<_> = graph
        .children_of(element_id)
        .filter(|c| c.kind == ElementKind::Documentation)
        .collect();
    docs.sort_by_key(|d| d.spans.first().map_or(usize::MAX, |s| s.start));
    let doc = docs.first().ok_or_else(|| {
        ServiceError::InvalidInput(
            "element has no doc comment to edit — adding one is a creation (printer) action".into(),
        )
    })?;
    let span = doc
        .spans
        .first()
        .ok_or_else(|| ServiceError::Internal("doc element has no source span".into()))?;
    let slice = content
        .get(span.start..span.end)
        .ok_or_else(|| ServiceError::Internal("doc span out of bounds — stale parse?".into()))?;
    let open = slice
        .find("/*")
        .ok_or_else(|| ServiceError::Internal("doc span carries no `/*` opener".into()))?;
    let close = slice
        .rfind("*/")
        .filter(|c| *c >= open + 2)
        .ok_or_else(|| ServiceError::Internal("doc span carries no `*/` terminator".into()))?;
    edit_for_range(
        content,
        &span.file,
        &element_id.to_string(),
        "doc",
        span.start + open + 2,
        span.start + close,
        format!(" {new_body} "),
    )
}

/// Replace the inline VALUE of an attribute usage: the text after the
/// binding `=` (or `:=`) up to the declaration terminator.
pub fn compute_attribute_value_edit(
    graph: &ModelGraph,
    content: &str,
    element_id: &ElementId,
    new_value: &str,
) -> Result<FieldEditComputed, ServiceError> {
    let new_value = new_value.trim();
    if new_value.is_empty() {
        return Err(ServiceError::InvalidInput("value must not be blank".into()));
    }
    if new_value.contains(';') || new_value.contains('\n') {
        return Err(ServiceError::InvalidInput(
            "value must be a single-line expression without `;`".into(),
        ));
    }
    let elem = graph.get_element(element_id).ok_or_else(|| {
        ServiceError::ElementNotFound(format!("element '{element_id}' not found"))
    })?;
    value_region_edit(content, elem, "attribute_value", new_value.to_owned())
}

/// Replace the `status` value of the element's `@StatusInfo` metadata with
/// `StatusKind::<status>`. `status` must be one of the spec's closed
/// [`STATUS_KINDS`] vocabulary.
pub fn compute_maturity_edit(
    graph: &ModelGraph,
    content: &str,
    element_id: &ElementId,
    status: &str,
) -> Result<FieldEditComputed, ServiceError> {
    let status = status.trim();
    if !STATUS_KINDS.contains(&status) {
        return Err(ServiceError::InvalidInput(format!(
            "invalid StatusKind '{status}' — must be one of {STATUS_KINDS:?}"
        )));
    }
    let meta = graph
        .children_of(element_id)
        .find(|c| {
            c.kind == ElementKind::MetadataUsage
                && sysml_core::metadata::is_metadata_typed_as(graph, c, "StatusInfo")
        })
        .ok_or_else(|| {
            ServiceError::InvalidInput(
                "element has no @StatusInfo metadata to edit — adding one is a creation \
                 (printer) action"
                    .into(),
            )
        })?;
    let status_attr = graph
        .children_of(&meta.id)
        .find(|c| {
            c.name.as_deref() == Some("status")
                && matches!(
                    c.kind,
                    ElementKind::AttributeUsage | ElementKind::ReferenceUsage
                )
        })
        .ok_or_else(|| {
            ServiceError::InvalidInput(
                "@StatusInfo metadata carries no `status` field to edit".into(),
            )
        })?;
    value_region_edit(
        content,
        status_attr,
        "maturity",
        format!("StatusKind::{status}"),
    )
}

// ─── Creation (§7.3): printer shapes from sysml-core + an anchored
//     insertion composer. Insertions REPLACE a real anchor (the body's tail
//     whitespace + closing `}`, or the declaration's `;`) so the
//     expected_old_text guard covers genuine text — never a zero-width
//     splice at a guessed offset. ────────────────────────────────────────

/// Create a new requirement member inside `parent_id` (a package or a
/// requirement — the document idiom's clause nesting). Shape from
/// `sysml_core::member_print::print_requirement_skeleton`.
pub fn compute_create_requirement(
    graph: &ModelGraph,
    content: &str,
    parent_id: &ElementId,
    short_name: Option<&str>,
    name: &str,
    doc_body: Option<&str>,
) -> Result<FieldEditComputed, ServiceError> {
    let name = name.trim();
    if !is_identifier(name) {
        return Err(ServiceError::InvalidInput(format!(
            "'{name}' is not a valid identifier"
        )));
    }
    if short_name.is_some_and(|s| s.contains('\'')) {
        return Err(ServiceError::InvalidInput(
            "short name must not contain a quote".into(),
        ));
    }
    let parent = graph
        .get_element(parent_id)
        .ok_or_else(|| ServiceError::ElementNotFound(format!("element '{parent_id}' not found")))?;
    if !matches!(
        parent.kind,
        ElementKind::Package
            | ElementKind::LibraryPackage
            | ElementKind::RequirementUsage
            | ElementKind::RequirementDefinition
    ) {
        return Err(ServiceError::InvalidInput(format!(
            "cannot create a requirement inside a {:?} — pick a package or requirement",
            parent.kind
        )));
    }
    insert_member_edit(content, parent, "create_requirement", |indent| {
        sysml_core::member_print::print_requirement_skeleton(short_name, name, doc_body, indent)
    })
}

/// Add a `doc /* … */` member to an element that has none (editing an
/// existing one is `compute_doc_edit`).
pub fn compute_add_doc(
    graph: &ModelGraph,
    content: &str,
    element_id: &ElementId,
    body: &str,
) -> Result<FieldEditComputed, ServiceError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(ServiceError::InvalidInput(
            "doc body must not be blank".into(),
        ));
    }
    if body.contains("*/") {
        return Err(ServiceError::InvalidInput(
            "doc body must not contain the comment terminator `*/`".into(),
        ));
    }
    let elem = graph.get_element(element_id).ok_or_else(|| {
        ServiceError::ElementNotFound(format!("element '{element_id}' not found"))
    })?;
    if graph
        .children_of(element_id)
        .any(|c| c.kind == ElementKind::Documentation)
    {
        return Err(ServiceError::InvalidInput(
            "element already has a doc comment — use edit_requirement_doc".into(),
        ));
    }
    insert_member_edit(content, elem, "add_doc", |indent| {
        format!(
            "{indent}{}",
            sysml_core::member_print::print_doc_comment(body)
        )
    })
}

/// Add `@StatusInfo { status = StatusKind::<status> }` to an element that
/// has no maturity metadata (editing an existing one is
/// `compute_maturity_edit`). `status` must be in the closed vocabulary.
pub fn compute_add_maturity(
    graph: &ModelGraph,
    content: &str,
    element_id: &ElementId,
    status: &str,
) -> Result<FieldEditComputed, ServiceError> {
    let status = status.trim();
    if !STATUS_KINDS.contains(&status) {
        return Err(ServiceError::InvalidInput(format!(
            "invalid StatusKind '{status}' — must be one of {STATUS_KINDS:?}"
        )));
    }
    let elem = graph.get_element(element_id).ok_or_else(|| {
        ServiceError::ElementNotFound(format!("element '{element_id}' not found"))
    })?;
    if graph.children_of(element_id).any(|c| {
        c.kind == ElementKind::MetadataUsage
            && sysml_core::metadata::is_metadata_typed_as(graph, c, "StatusInfo")
    }) {
        return Err(ServiceError::InvalidInput(
            "element already has @StatusInfo — use edit_requirement_maturity".into(),
        ));
    }
    insert_member_edit(content, elem, "add_maturity", |indent| {
        format!(
            "{indent}{}",
            sysml_core::member_print::print_status_info(status)
        )
    })
}

/// The four typed parameter memberships a requirement can gain (§7.7). Each
/// prints `<keyword> <name> : <Type>;`; they differ only in the keyword, the
/// accepted target-definition kind (spec-grounded: subject = Anything, actor/
/// stakeholder = Part, concern = Concern), and multiplicity (subject is [1]).
#[derive(Clone, Copy)]
pub enum RequirementRoleKind {
    Subject,
    Actor,
    Stakeholder,
    Concern,
}

impl RequirementRoleKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "subject" => Some(Self::Subject),
            "actor" => Some(Self::Actor),
            "stakeholder" => Some(Self::Stakeholder),
            "concern" => Some(Self::Concern),
            _ => None,
        }
    }
    fn keyword(self) -> &'static str {
        match self {
            Self::Subject => "subject",
            Self::Actor => "actor",
            Self::Stakeholder => "stakeholder",
            Self::Concern => "frame concern",
        }
    }
    fn accepts(self, kind: &ElementKind) -> bool {
        match self {
            // Subject is typed `Anything` in the library — accept any
            // definition (the FE offers part/item defs; the boundary stays
            // permissive per spec rather than inventing a narrower rule).
            Self::Subject => kind.is_definition(),
            Self::Actor | Self::Stakeholder => *kind == ElementKind::PartDefinition,
            Self::Concern => *kind == ElementKind::ConcernDefinition,
        }
    }
    fn accepts_desc(self) -> &'static str {
        match self {
            Self::Subject => "a definition",
            Self::Actor | Self::Stakeholder => "a part definition",
            Self::Concern => "a concern definition",
        }
    }
    fn is_singleton(self) -> bool {
        matches!(self, Self::Subject)
    }
}

/// Add a `<keyword> <name> : <Type>;` typed parameter membership (subject /
/// actor / stakeholder / framed concern) to a requirement (§7.7). `type_id`
/// is the referenced definition; its kind is validated per role. Subject is
/// singleton — fails if one already exists.
pub fn compute_add_requirement_role(
    graph: &ModelGraph,
    content: &str,
    requirement_id: &ElementId,
    role: RequirementRoleKind,
    type_id: &ElementId,
    name: &str,
) -> Result<FieldEditComputed, ServiceError> {
    let req = require_requirement(graph, requirement_id)?;
    let name = name.trim();
    if !is_identifier(name) {
        return Err(ServiceError::InvalidInput(format!(
            "'{name}' is not a valid identifier"
        )));
    }
    let target = graph
        .get_element(type_id)
        .ok_or_else(|| ServiceError::ElementNotFound(format!("element '{type_id}' not found")))?;
    if !role.accepts(&target.kind) {
        return Err(ServiceError::InvalidInput(format!(
            "'{}' is a {:?}, not {} — {} must reference {}",
            display_name(target),
            target.kind,
            role.accepts_desc(),
            role.keyword(),
            role.accepts_desc(),
        )));
    }
    if role.is_singleton() && req.get_prop("subject").is_some() {
        return Err(ServiceError::InvalidInput(
            "requirement already has a subject — a requirement has at most one".into(),
        ));
    }
    let type_ref = reference_text(graph, req, target)?;
    let keyword = role.keyword();
    insert_member_edit(content, req, "add_requirement_role", |indent| {
        format!(
            "{indent}{}",
            sysml_core::member_print::print_typed_role(keyword, name, &type_ref)
        )
    })
}

/// A SysML identifier: leading letter/`_`, then alphanumerics/`_`. The one
/// home for the create/attribute name check.
fn is_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Add an `assume/require constraint [name] { <expr> }` member to an element
/// (§7.7). `expr` is spliced verbatim into the guarded braces — a fixed
/// skeleton with user text, the `create_requirement` risk profile, NOT the
/// deferred AST printer (§7.4). Editing an existing constraint's expression
/// stays deferred. The §7.1 AST-unification dual-write makes the added
/// constraint visible to the evaluator + suspect-diff.
pub fn compute_add_constraint(
    graph: &ModelGraph,
    content: &str,
    element_id: &ElementId,
    is_assume: bool,
    name: Option<&str>,
    expr: &str,
) -> Result<FieldEditComputed, ServiceError> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err(ServiceError::InvalidInput(
            "constraint expression must not be blank".into(),
        ));
    }
    if expr.contains(['{', '}', ';', '\n', '\r', '\t']) {
        return Err(ServiceError::InvalidInput(
            "constraint expression must be a single-line expression (no braces or `;`)".into(),
        ));
    }
    if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
        if !is_identifier(n) {
            return Err(ServiceError::InvalidInput(format!(
                "'{n}' is not a valid constraint name"
            )));
        }
    }
    let elem = graph.get_element(element_id).ok_or_else(|| {
        ServiceError::ElementNotFound(format!("element '{element_id}' not found"))
    })?;
    insert_member_edit(content, elem, "add_constraint", |indent| {
        format!(
            "{indent}{}",
            sysml_core::member_print::print_requirement_constraint(is_assume, name, expr)
        )
    })
}

/// Add an `attribute <name> [= <value>];` member to an element (§7.7). `value`
/// is optional and text-level (a single-line expression, same honesty as
/// `compute_attribute_value_edit`); editing an existing value stays
/// `compute_attribute_value_edit`.
pub fn compute_add_attribute(
    graph: &ModelGraph,
    content: &str,
    element_id: &ElementId,
    name: &str,
    value: Option<&str>,
) -> Result<FieldEditComputed, ServiceError> {
    let name = name.trim();
    if !is_identifier(name) {
        return Err(ServiceError::InvalidInput(format!(
            "'{name}' is not a valid identifier"
        )));
    }
    let value = value.map(str::trim).filter(|v| !v.is_empty());
    if let Some(v) = value {
        if v.contains([';', '\n', '\r', '\t']) {
            return Err(ServiceError::InvalidInput(
                "attribute value must be a single-line expression (no `;`)".into(),
            ));
        }
    }
    let elem = graph.get_element(element_id).ok_or_else(|| {
        ServiceError::ElementNotFound(format!("element '{element_id}' not found"))
    })?;
    if graph
        .children_of(element_id)
        .any(|c| c.kind == ElementKind::AttributeUsage && c.name.as_deref() == Some(name))
    {
        return Err(ServiceError::InvalidInput(format!(
            "element already has an attribute named '{name}' — edit its value instead"
        )));
    }
    insert_member_edit(content, elem, "add_attribute", |indent| {
        format!(
            "{indent}{}",
            sysml_core::member_print::print_attribute_declaration(name, value)
        )
    })
}

/// Add a `@Rationale { text = "…" }` metadata member to an element (§7.7 build
/// set). v1 is add-only — a requirement may carry SEVERAL rationale
/// annotations (the read side joins them), so this never fails on "already
/// exists"; it appends another. Editing a specific rationale's text is a
/// future unit (shares the value-region span math with attribute edits).
pub fn compute_add_rationale(
    graph: &ModelGraph,
    content: &str,
    element_id: &ElementId,
    text: &str,
) -> Result<FieldEditComputed, ServiceError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ServiceError::InvalidInput(
            "rationale text must not be blank".into(),
        ));
    }
    if text.contains(['\n', '\r', '\t']) {
        return Err(ServiceError::InvalidInput(
            "rationale text must be a single line (no tabs or newlines)".into(),
        ));
    }
    let elem = graph.get_element(element_id).ok_or_else(|| {
        ServiceError::ElementNotFound(format!("element '{element_id}' not found"))
    })?;
    insert_member_edit(content, elem, "add_rationale", |indent| {
        format!("{indent}{}", sysml_core::member_print::print_rationale(text))
    })
}

// ─── Link writing (§7.6): satisfy/verify/derive statements composed from
//     member_print shapes. One command per relationship; direction-symmetric
//     adds reuse the same command with roles chosen by the caller. Duplicate
//     links (edge already in the graph) fail hard. ─────────────────────────

/// Insert `satisfy <req-ref>;` into the PICKED SUBJECT's body (§7.6: with
/// no `by` clause the elaborator reads the owner as satisfyingFeature —
/// the corpus idiom). `content` is the SUBJECT's file text.
pub fn compute_add_satisfy_link(
    graph: &ModelGraph,
    content: &str,
    requirement_id: &ElementId,
    subject_id: &ElementId,
) -> Result<FieldEditComputed, ServiceError> {
    let req = require_requirement(graph, requirement_id)?;
    let subject = graph.get_element(subject_id).ok_or_else(|| {
        ServiceError::ElementNotFound(format!("element '{subject_id}' not found"))
    })?;
    if graph
        .relationships_by_kind(&RelationshipKind::Satisfy)
        .any(|r| r.source == *subject_id && r.target == *requirement_id)
    {
        return Err(ServiceError::InvalidInput(format!(
            "'{}' already satisfies '{}' — the link exists",
            display_name(subject),
            display_name(req),
        )));
    }
    let req_ref = reference_text(graph, subject, req)?;
    insert_member_edit(content, subject, "add_satisfy_link", |indent| {
        format!(
            "{indent}{}",
            sysml_core::member_print::print_satisfy_statement(&req_ref)
        )
    })
}

/// Insert `verify <req-ref>;` into the picked case's `objective` body —
/// the only spec-legal home (`validateRequirementVerificationMembership-
/// OwningType`). A case with NO objective gets the whole
/// `objective { verify …; }` block in one insertion. `content` is the
/// CASE's file text.
pub fn compute_add_verify_link(
    graph: &ModelGraph,
    content: &str,
    requirement_id: &ElementId,
    case_id: &ElementId,
) -> Result<FieldEditComputed, ServiceError> {
    let req = require_requirement(graph, requirement_id)?;
    let case = graph
        .get_element(case_id)
        .ok_or_else(|| ServiceError::ElementNotFound(format!("element '{case_id}' not found")))?;
    if !matches!(
        case.kind,
        ElementKind::VerificationCaseDefinition | ElementKind::VerificationCaseUsage
    ) {
        return Err(ServiceError::InvalidInput(format!(
            "'{}' is a {:?}, not a verification case",
            display_name(case),
            case.kind
        )));
    }
    if graph
        .relationships_by_kind(&RelationshipKind::Verify)
        .any(|r| r.source == *case_id && r.target == *requirement_id)
    {
        return Err(ServiceError::InvalidInput(format!(
            "'{}' already verifies '{}' — the link exists",
            display_name(case),
            display_name(req),
        )));
    }
    let req_ref = reference_text(graph, case, req)?;
    let mut objectives: Vec<_> = graph
        .children_of(case_id)
        .filter(|c| c.kind == ElementKind::ObjectiveMembership)
        .collect();
    objectives.sort_by_key(|o| o.spans.first().map_or(usize::MAX, |s| s.start));
    match objectives.first() {
        Some(objective) => insert_member_edit(content, objective, "add_verify_link", |indent| {
            format!(
                "{indent}{}",
                sysml_core::member_print::print_verify_statement(&req_ref)
            )
        }),
        None => insert_member_edit(content, case, "add_verify_link", |indent| {
            sysml_core::member_print::print_objective_with_verify(&req_ref, indent)
        }),
    }
}

/// Insert a `#derivation connection` block into the DERIVED requirement's
/// owning package (§7.6 — the core grammar has no derive keyword; this is
/// the Requirement Derivation domain-library form). The
/// `RequirementDerivation` import is LOAD-BEARING (the elaborator
/// deliberately mints nothing without it), so when the owning-package chain
/// lacks one, the SAME insertion prepends
/// `private import RequirementDerivation::*;`. `content` is the DERIVED
/// requirement's file text.
pub fn compute_add_derive_link(
    graph: &ModelGraph,
    content: &str,
    requirement_id: &ElementId,
    original_id: &ElementId,
) -> Result<FieldEditComputed, ServiceError> {
    let derived = require_requirement(graph, requirement_id)?;
    let original = require_requirement(graph, original_id)?;
    if graph
        .relationships_by_kind(&RelationshipKind::Derive)
        .any(|r| r.source == *requirement_id && r.target == *original_id)
    {
        return Err(ServiceError::InvalidInput(format!(
            "'{}' already derives from '{}' — the link exists",
            display_name(derived),
            display_name(original),
        )));
    }
    let pkg = owning_package(graph, derived).ok_or_else(|| {
        ServiceError::InvalidInput(format!(
            "'{}' has no owning package — derivation connections are package members",
            display_name(derived)
        ))
    })?;
    let original_ref = reference_text(graph, pkg, original)?;
    let derived_ref = reference_text(graph, pkg, derived)?;
    let needs_import = !has_library_import(graph, pkg, "RequirementDerivation");
    insert_member_edit(content, pkg, "add_derive_link", |indent| {
        let conn = sysml_core::member_print::print_derivation_connection(
            &original_ref,
            &derived_ref,
            indent,
        );
        if needs_import {
            format!("{indent}private import RequirementDerivation::*;\n\n{conn}")
        } else {
            conn
        }
    })
}

/// Insert a `dependency from <refining> to <refined> { @Refinement; }` into
/// the REFINING requirement's owning package (§7.6 — refine is a KerML
/// Dependency + `ModelingMetadata::Refinement` metadata, NOT a keyword or
/// specialization). `requirement_id` is the refining/client end (the row's
/// outgoing `refines`); `refined_id` the supplier. The `ModelingMetadata`
/// import is LOAD-BEARING (`is_refinement_annotated` needs `@Refinement` to
/// resolve to the library element, else no Refine edge mints), so the same
/// insertion prepends `private import ModelingMetadata::*;` when absent.
/// `content` is the REFINING requirement's file text.
pub fn compute_add_refine_link(
    graph: &ModelGraph,
    content: &str,
    requirement_id: &ElementId,
    refined_id: &ElementId,
) -> Result<FieldEditComputed, ServiceError> {
    let refining = require_requirement(graph, requirement_id)?;
    let refined = require_requirement(graph, refined_id)?;
    if graph
        .relationships_by_kind(&RelationshipKind::Refine)
        .any(|r| r.source == *requirement_id && r.target == *refined_id)
    {
        return Err(ServiceError::InvalidInput(format!(
            "'{}' already refines '{}' — the link exists",
            display_name(refining),
            display_name(refined),
        )));
    }
    let pkg = owning_package(graph, refining).ok_or_else(|| {
        ServiceError::InvalidInput(format!(
            "'{}' has no owning package — refinement dependencies are package members",
            display_name(refining)
        ))
    })?;
    let refining_ref = reference_text(graph, pkg, refining)?;
    let refined_ref = reference_text(graph, pkg, refined)?;
    let needs_import = !has_library_import(graph, pkg, "ModelingMetadata");
    insert_member_edit(content, pkg, "add_refine_link", |indent| {
        let dep =
            sysml_core::member_print::print_refine_dependency(&refining_ref, &refined_ref, indent);
        if needs_import {
            format!("{indent}private import ModelingMetadata::*;\n\n{dep}")
        } else {
            dep
        }
    })
}

fn require_requirement<'g>(
    graph: &'g ModelGraph,
    element_id: &ElementId,
) -> Result<&'g sysml_core::Element, ServiceError> {
    let elem = graph.get_element(element_id).ok_or_else(|| {
        ServiceError::ElementNotFound(format!("element '{element_id}' not found"))
    })?;
    if !matches!(
        elem.kind,
        ElementKind::RequirementDefinition | ElementKind::RequirementUsage
    ) {
        return Err(ServiceError::InvalidInput(format!(
            "'{}' is a {:?}, not a requirement",
            display_name(elem),
            elem.kind
        )));
    }
    Ok(elem)
}

fn display_name(elem: &sysml_core::Element) -> String {
    elem.name
        .clone()
        .unwrap_or_else(|| elem.id.to_string())
}

/// The nearest `Package`/`LibraryPackage` ancestor (owner chain), for
/// package-level insertions.
fn owning_package<'g>(
    graph: &'g ModelGraph,
    elem: &sysml_core::Element,
) -> Option<&'g sysml_core::Element> {
    let mut cur = elem.owner.clone();
    while let Some(id) = cur {
        let owner = graph.get_element(&id)?;
        if matches!(
            owner.kind,
            ElementKind::Package | ElementKind::LibraryPackage
        ) {
            return Some(owner);
        }
        cur = owner.owner.clone();
    }
    None
}

/// Reference text for `target` as written from `context`'s scope (§7.6
/// ruling — a sanctioned printing extension, spec-silent on shortening):
/// the SIMPLE name when the target's direct owner sits on the context's
/// own owner chain (context included), the fully QUALIFIED name otherwise.
/// Deliberately NOT a visibility calculator — no `VisibilityKind`, no
/// import walk; the qualified form is always spec-legal and the contract
/// gate proves it round-trips.
fn reference_text(
    graph: &ModelGraph,
    context: &sysml_core::Element,
    target: &sysml_core::Element,
) -> Result<String, ServiceError> {
    let simple = target.name.as_deref().filter(|n| !n.is_empty());
    let Some(simple) = simple else {
        return Err(ServiceError::InvalidInput(format!(
            "element '{}' has no name — link targets must be named",
            target.id
        )));
    };
    let target_owner = target.owner.as_ref();
    let mut cur = Some(context.id.clone());
    while let Some(id) = cur {
        if target_owner == Some(&id) {
            return Ok(simple.to_owned());
        }
        cur = graph.get_element(&id).and_then(|e| e.owner.clone());
    }
    target
        .qname
        .as_ref()
        .map(|q| q.to_string())
        .or_else(|| {
            graph
                .build_qualified_name(&target.id)
                .map(|q| q.to_string())
        })
        .ok_or_else(|| {
            ServiceError::Internal(format!(
                "cannot build a qualified name for '{simple}' ({})",
                target.id
            ))
        })
}

/// PRESENCE check (never a resolvability probe — §7.6 ruling) for an import
/// whose namespace's FIRST segment is `top_segment`, anywhere on the
/// owning-package chain. Serves the load-bearing-import rides for both
/// derive (`RequirementDerivation`) and refine (`ModelingMetadata`).
fn has_library_import(graph: &ModelGraph, pkg: &sysml_core::Element, top_segment: &str) -> bool {
    let mut cur = Some(pkg.id.clone());
    while let Some(id) = cur {
        let has = graph.children_of(&id).any(|c| {
            matches!(
                c.kind,
                ElementKind::NamespaceImport | ElementKind::MembershipImport
            ) && c
                .get_prop("importedReference")
                .or_else(|| c.get_prop("unresolved_importedNamespace"))
                .and_then(|v| v.as_str())
                .is_some_and(|ns| ns.split("::").next().map(str::trim) == Some(top_segment))
        });
        if has {
            return true;
        }
        cur = graph.get_element(&id).and_then(|e| e.owner.clone());
    }
    false
}

/// Anchored member insertion at the END of `parent`'s body. Braced body:
/// the anchor is the tail whitespace + closing `}` (replaced by newline +
/// member + newline + indent + `}`). Semicolon body (`requirement bare;`):
/// the anchor is the `;` (the declaration grows a braced body). The member
/// callback receives the MEMBER indentation (parent's + one tab).
fn insert_member_edit(
    content: &str,
    parent: &sysml_core::Element,
    field: &str,
    member: impl Fn(&str) -> String,
) -> Result<FieldEditComputed, ServiceError> {
    let span = parent.spans.first().ok_or_else(|| {
        ServiceError::Internal(format!("element '{}' has no source span", parent.id))
    })?;
    let slice = content.get(span.start..span.end).ok_or_else(|| {
        ServiceError::Internal("element span out of bounds — stale parse?".into())
    })?;
    // Parent's own indentation: leading whitespace of the line its
    // declaration starts on.
    let line_start = content[..span.start].rfind('\n').map_or(0, |p| p + 1);
    let parent_indent: String = content[line_start..span.start]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let member_indent = format!("{parent_indent}\t");
    let member_text = member(&member_indent);

    let last = slice.trim_end();
    if last.ends_with('}') {
        // Anchor: from just after the last non-whitespace char BEFORE the
        // closing brace, through the brace (normalizes tail whitespace).
        let brace_rel = slice.rfind('}').expect("ends_with('}') checked");
        let before = slice[..brace_rel]
            .rfind(|c: char| !c.is_whitespace())
            .map_or(0, |p| {
                p + slice[p..].chars().next().map_or(1, char::len_utf8)
            });
        edit_for_range(
            content,
            &span.file,
            &parent.id.to_string(),
            field,
            span.start + before,
            span.start + brace_rel + 1,
            format!("\n{member_text}\n{parent_indent}}}"),
        )
    } else if last.ends_with(';') {
        let semi_rel = slice.rfind(';').expect("ends_with(';') checked");
        edit_for_range(
            content,
            &span.file,
            &parent.id.to_string(),
            field,
            span.start + semi_rel,
            span.start + semi_rel + 1,
            format!(" {{\n{member_text}\n{parent_indent}}}"),
        )
    } else {
        Err(ServiceError::InvalidInput(format!(
            "element '{}' has neither a braced body nor a `;` declaration — cannot insert",
            parent.id
        )))
    }
}

/// Shared value-region math: within the element's declaration span, the
/// value is the text after the binding `=` (also covering `:=`), trimmed,
/// ending before the trailing terminator. Text-level on purpose — LITERAL
/// values carry no value-only span (the parser stores them as a prop on the
/// declaration), and the `expected_old_text` guard makes the splice honest
/// regardless.
fn value_region_edit(
    content: &str,
    elem: &sysml_core::Element,
    field: &str,
    new_text: String,
) -> Result<FieldEditComputed, ServiceError> {
    let span = elem.spans.first().ok_or_else(|| {
        ServiceError::Internal(format!("element '{}' has no source span", elem.id))
    })?;
    let slice = content.get(span.start..span.end).ok_or_else(|| {
        ServiceError::Internal("element span out of bounds — stale parse?".into())
    })?;
    let eq = slice.find('=').ok_or_else(|| {
        ServiceError::InvalidInput(
            "declaration has no inline `= value` to edit — adding one is a creation \
             (printer) action"
                .into(),
        )
    })?;
    // Value text: after the `=`, skipping leading whitespace…
    let after_eq = eq + 1;
    let rel_start = after_eq
        + slice[after_eq..]
            .find(|c: char| !c.is_whitespace())
            .ok_or_else(|| {
                ServiceError::InvalidInput("declaration has `=` but no value after it".into())
            })?;
    // …up to (not including) the trailing terminator/close, right-trimmed.
    let tail = slice[rel_start..]
        .rfind(|c: char| c != ';' && c != '}' && !c.is_whitespace())
        .ok_or_else(|| {
            ServiceError::InvalidInput("declaration has `=` but no value after it".into())
        })?;
    let rel_end = rel_start
        + tail
        + slice[rel_start + tail..]
            .chars()
            .next()
            .map_or(1, char::len_utf8);
    edit_for_range(
        content,
        &span.file,
        &elem.id.to_string(),
        field,
        span.start + rel_start,
        span.start + rel_end,
        new_text,
    )
}
