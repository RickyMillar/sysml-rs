//! Structured query engine for row-shaped SysML model reads.
//!
//! This crate is deliberately transport-agnostic: callers provide a
//! [`sysml_core::ModelGraph`] and a [`QuerySpec`], and the engine returns a
//! paged [`QueryResult`]. Service/API/MCP layers own workspace selection,
//! authentication, and process-level caching.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use base64::Engine;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sysml_core::{
    build_view_index, is_requirement_kind, query as core_query, Element, ElementKind, ModelGraph,
    RelationshipKind, Value, ViewSummary,
};
use sysml_id::ElementId;
use sysml_span::Span;

pub mod suspect;

const DEFAULT_LIMIT: usize = 500;
const MCP_DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1000;
const MAX_FILTER_DEPTH: usize = 8;
const CURSOR_VERSION: u8 = 1;

/// Execution profile controls transport-specific defaults without changing the
/// query semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryProfile {
    /// Service/API default: suitable for frontend calls.
    Service,
    /// MCP default: smaller first pages to protect agent context windows.
    Mcp,
}

impl QueryProfile {
    fn default_limit(self) -> usize {
        match self {
            QueryProfile::Service => DEFAULT_LIMIT,
            QueryProfile::Mcp => MCP_DEFAULT_LIMIT,
        }
    }
}

/// Top-level query request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct QuerySpec {
    #[serde(default)]
    pub filter: Filter,
    #[serde(default)]
    pub projection: Projection,
    #[serde(default)]
    pub sort: Vec<SortKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl Default for QuerySpec {
    fn default() -> Self {
        Self {
            filter: Filter::All { filters: Vec::new() },
            projection: Projection::Summary,
            sort: Vec::new(),
            limit: None,
            cursor: None,
        }
    }
}

/// Projection requested by a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    Ids,
    Elements,
    Summary,
    SummaryExpand,
    Count,
}

impl Default for Projection {
    fn default() -> Self {
        Self::Summary
    }
}

impl Projection {
    pub fn all_variants() -> &'static [Projection] {
        &[
            Projection::Ids,
            Projection::Elements,
            Projection::Summary,
            Projection::SummaryExpand,
            Projection::Count,
        ]
    }
}

/// Sort key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct SortKey {
    pub field: SortField,
    #[serde(default)]
    pub dir: SortDir,
}

/// Sortable fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    Name,
    QualifiedName,
    Kind,
    OwnerDepth,
}

impl SortField {
    pub fn all_variants() -> &'static [SortField] {
        &[
            SortField::Name,
            SortField::QualifiedName,
            SortField::Kind,
            SortField::OwnerDepth,
        ]
    }
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortDir {
    Asc,
    Desc,
}

impl Default for SortDir {
    fn default() -> Self {
        Self::Asc
    }
}

impl SortDir {
    pub fn all_variants() -> &'static [SortDir] {
        &[SortDir::Asc, SortDir::Desc]
    }
}

/// Relationship role an element must occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelationRole {
    Source,
    Target,
}

impl RelationRole {
    pub fn all_variants() -> &'static [RelationRole] {
        &[RelationRole::Source, RelationRole::Target]
    }
}

/// Query cache status surfaced by the service layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryCacheStatus {
    Uncached,
    Hit,
    Miss,
}

impl QueryCacheStatus {
    pub fn all_variants() -> &'static [QueryCacheStatus] {
        &[
            QueryCacheStatus::Uncached,
            QueryCacheStatus::Hit,
            QueryCacheStatus::Miss,
        ]
    }
}

/// Composable query filter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Filter {
    All { filters: Vec<Filter> },
    Any { filters: Vec<Filter> },
    Not { filter: Box<Filter> },
    Kind { kinds: Vec<ElementKind> },
    NameMatch { name_match: NameMatch },
    QualifiedNameMatch { qualified_name_match: NameMatch },
    Owner { owner: OwnerFilter },
    IdIn { ids: Vec<ElementId> },
    HasRelation { has_relation: RelationFilter },
    /// Compatibility helper for legacy `sysml.unverified`.
    UnverifiedRequirement,
    /// User-authored elements only — excludes standard-library elements
    /// (the `requirement_rows` exclusion, generalized: pickers and list
    /// reads over the merged workspace graph must not surface stdlib
    /// internals as candidates).
    UserAuthored,
    /// Compatibility helper for view-related wrappers.
    View { viewpoint_id: Option<ElementId> },
    /// Compatibility helper for viewpoint-related wrappers.
    Viewpoint { stakeholder_id: Option<ElementId> },
}

impl Default for Filter {
    fn default() -> Self {
        Self::All { filters: Vec::new() }
    }
}

/// Name/string matcher.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
pub struct NameMatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
    #[serde(default)]
    pub ci: bool,
}

/// Ownership filter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
pub struct OwnerFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ElementId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ElementKind>,
    #[serde(default)]
    pub transitive: bool,
}

/// Relationship-existence filter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct RelationFilter {
    pub kind: RelationshipKind,
    pub role: RelationRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<ElementKind>,
}

/// Lightweight row shape for UI pickers and MCP exploration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementSummary {
    pub id: ElementId,
    pub name: Option<String>,
    pub qualified_name: Option<String>,
    pub kind: ElementKind,
    pub owner_id: Option<ElementId>,
    pub source_span: Option<Span>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion: Option<SummaryExpansion>,
}

/// Domain-specific one-level summary expansion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "data")]
pub enum SummaryExpansion {
    View(ViewSummary),
}

/// Query rows.  Serialized as the direct row payload so clients can treat
/// `result.rows` as either an array or a count depending on projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QueryRows {
    Ids(Vec<ElementId>),
    Elements(Vec<Element>),
    Summary(Vec<ElementSummary>),
    Count(usize),
}

/// Query response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub rows: QueryRows,
    pub total_estimate: Option<usize>,
    pub cursor: Option<String>,
    pub cursor_invalidated: bool,
    pub revision: u64,
    pub cache_status: QueryCacheStatus,
}

impl QueryResult {
    pub fn with_cache_status(mut self, status: QueryCacheStatus) -> Self {
        self.cache_status = status;
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("invalid query: {0}")]
    Invalid(String),
    #[error("invalid regex `{pattern}`: {message}")]
    Regex { pattern: String, message: String },
    #[error("invalid cursor: {0}")]
    Cursor(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CursorPayload {
    v: u8,
    revision: u64,
    last_id: ElementId,
}

/// Compute a deterministic revision fingerprint for a graph. The service uses
/// this to key process-level query cache entries and to gate
/// `cursor_invalidated`.
///
/// Delegates to `ModelGraph::fingerprint()` — the content-true hash in
/// `sysml-core` (ids, kinds, names, property values, spans, relationship
/// endpoints). The previous private id-only hash meant an in-place edit
/// (doc text, value, rename) that added/removed no ID yielded an identical
/// revision, so the service's query cache served stale rows and cursors
/// minted against different content validated (2026-07-16 staleness bug;
/// duplicate-path collapse per the workspace-scope plan §W6).
pub fn graph_revision(graph: &ModelGraph) -> u64 {
    graph.fingerprint()
}

pub fn execute_query(graph: &ModelGraph, spec: &QuerySpec, revision: u64) -> Result<QueryResult, QueryError> {
    execute_query_with_profile(graph, spec, revision, QueryProfile::Service)
}

pub fn execute_query_with_profile(
    graph: &ModelGraph,
    spec: &QuerySpec,
    revision: u64,
    profile: QueryProfile,
) -> Result<QueryResult, QueryError> {
    validate_filter(&spec.filter, 0)?;
    let limit = spec.limit.unwrap_or_else(|| profile.default_limit()).min(MAX_LIMIT);
    let cursor_payload = match &spec.cursor {
        Some(cursor) => Some(decode_cursor(cursor)?),
        None => None,
    };
    let cursor_invalidated = cursor_payload
        .as_ref()
        .is_some_and(|payload| payload.revision != revision);

    let mut rows: Vec<&Element> = graph
        .elements
        .values()
        .filter(|element| matches_filter(graph, element, &spec.filter))
        .collect();
    sort_rows(graph, &mut rows, &spec.sort);

    let total = rows.len();
    let (selected, next_cursor) = if spec.projection == Projection::Count {
        (Vec::new(), None)
    } else {
        paginate(&rows, limit, cursor_payload.as_ref(), revision)?
    };

    let rows = match spec.projection {
        Projection::Ids => QueryRows::Ids(selected.into_iter().map(|element| element.id.clone()).collect()),
        Projection::Elements => QueryRows::Elements(selected.into_iter().cloned().collect()),
        Projection::Summary => QueryRows::Summary(
            selected
                .into_iter()
                .map(|element| summarize(graph, element, None))
                .collect(),
        ),
        Projection::SummaryExpand => {
            let view_index = build_view_expansion_map(graph);
            QueryRows::Summary(
                selected
                    .into_iter()
                    .map(|element| summarize(graph, element, view_index.get(&element.id).cloned()))
                    .collect(),
            )
        }
        Projection::Count => QueryRows::Count(total),
    };

    Ok(QueryResult {
        rows,
        total_estimate: Some(total),
        cursor: next_cursor,
        cursor_invalidated,
        revision,
        cache_status: QueryCacheStatus::Uncached,
    })
}

/// Shared pagination over a sorted element list: apply the cursor's start
/// offset, take one page, and mint the next cursor when rows remain.
fn paginate<'g>(
    rows: &[&'g Element],
    limit: usize,
    cursor_payload: Option<&CursorPayload>,
    revision: u64,
) -> Result<(Vec<&'g Element>, Option<String>), QueryError> {
    let total = rows.len();
    let start = cursor_payload
        .and_then(|payload| rows.iter().position(|element| element.id == payload.last_id))
        .map_or(0, |idx| idx.saturating_add(1));
    let selected: Vec<&Element> = rows.iter().skip(start).take(limit).copied().collect();
    let next_cursor = if start + selected.len() >= total {
        None
    } else {
        selected
            .last()
            .map(|element| encode_cursor(revision, element.id.clone()))
            .transpose()?
    };
    Ok((selected, next_cursor))
}

fn validate_filter(filter: &Filter, depth: usize) -> Result<(), QueryError> {
    if depth > MAX_FILTER_DEPTH {
        return Err(QueryError::Invalid(format!(
            "filter nesting exceeds max depth {MAX_FILTER_DEPTH}"
        )));
    }
    match filter {
        Filter::All { filters } | Filter::Any { filters } => {
            for child in filters {
                validate_filter(child, depth + 1)?;
            }
        }
        Filter::Not { filter } => validate_filter(filter, depth + 1)?,
        Filter::NameMatch { name_match } | Filter::QualifiedNameMatch { qualified_name_match: name_match } => {
            validate_name_match(name_match)?;
        }
        Filter::Kind { .. }
        | Filter::Owner { .. }
        | Filter::IdIn { .. }
        | Filter::HasRelation { .. }
        | Filter::UnverifiedRequirement
        | Filter::UserAuthored
        | Filter::View { .. }
        | Filter::Viewpoint { .. } => {}
    }
    Ok(())
}

fn validate_name_match(name_match: &NameMatch) -> Result<(), QueryError> {
    let modes = [
        name_match.exact.is_some(),
        name_match.prefix.is_some(),
        name_match.contains.is_some(),
        name_match.regex.is_some(),
    ]
    .into_iter()
    .filter(|set| *set)
    .count();
    if modes != 1 {
        return Err(QueryError::Invalid(
            "name_match must set exactly one of exact, prefix, contains, regex".to_owned(),
        ));
    }
    if let Some(pattern) = &name_match.regex {
        Regex::new(pattern).map_err(|err| QueryError::Regex {
            pattern: pattern.clone(),
            message: err.to_string(),
        })?;
    }
    Ok(())
}

fn matches_filter(graph: &ModelGraph, element: &Element, filter: &Filter) -> bool {
    match filter {
        Filter::All { filters } => filters.iter().all(|child| matches_filter(graph, element, child)),
        Filter::Any { filters } => filters.iter().any(|child| matches_filter(graph, element, child)),
        Filter::Not { filter } => !matches_filter(graph, element, filter),
        Filter::Kind { kinds } => kinds.iter().any(|kind| kind == &element.kind),
        Filter::NameMatch { name_match } => element
            .name
            .as_deref()
            .is_some_and(|name| matches_name(name, name_match)),
        Filter::QualifiedNameMatch { qualified_name_match } => qualified_name(graph, element)
            .as_deref()
            .is_some_and(|name| matches_name(name, qualified_name_match)),
        Filter::Owner { owner } => matches_owner(graph, element, owner),
        Filter::IdIn { ids } => ids.iter().any(|id| id == &element.id),
        Filter::HasRelation { has_relation } => matches_relation(graph, element, has_relation),
        Filter::UnverifiedRequirement => is_unverified_requirement(graph, element),
        Filter::UserAuthored => !graph.is_library_element(&element.id),
        Filter::View { viewpoint_id } => matches_view(graph, element, viewpoint_id.as_ref()),
        Filter::Viewpoint { stakeholder_id } => matches_viewpoint(graph, element, stakeholder_id.as_ref()),
    }
}

fn matches_name(value: &str, matcher: &NameMatch) -> bool {
    let (haystack, exact, prefix, contains) = if matcher.ci {
        (
            value.to_lowercase(),
            matcher.exact.as_ref().map(|s| s.to_lowercase()),
            matcher.prefix.as_ref().map(|s| s.to_lowercase()),
            matcher.contains.as_ref().map(|s| s.to_lowercase()),
        )
    } else {
        (
            value.to_owned(),
            matcher.exact.clone(),
            matcher.prefix.clone(),
            matcher.contains.clone(),
        )
    };
    if let Some(needle) = exact {
        return haystack == needle;
    }
    if let Some(needle) = prefix {
        return haystack.starts_with(&needle);
    }
    if let Some(needle) = contains {
        return haystack.contains(&needle);
    }
    if let Some(pattern) = &matcher.regex {
        let pattern = if matcher.ci {
            format!("(?i:{pattern})")
        } else {
            pattern.clone()
        };
        return Regex::new(&pattern)
            .map(|regex| regex.is_match(value))
            .unwrap_or(false);
    }
    false
}

fn matches_owner(graph: &ModelGraph, element: &Element, owner: &OwnerFilter) -> bool {
    if owner.id.is_none() && owner.kind.is_none() {
        return true;
    }
    let mut current = element.owner.clone();
    while let Some(owner_id) = current {
        let Some(owner_element) = graph.get_element(&owner_id) else {
            return false;
        };
        let id_ok = owner.id.as_ref().map_or(true, |id| id == &owner_id);
        let kind_ok = owner.kind.as_ref().map_or(true, |kind| kind == &owner_element.kind);
        if id_ok && kind_ok {
            return true;
        }
        if !owner.transitive {
            return false;
        }
        current = owner_element.owner.clone();
    }
    false
}

fn matches_relation(graph: &ModelGraph, element: &Element, filter: &RelationFilter) -> bool {
    let relationships: Vec<_> = match filter.role {
        RelationRole::Source => graph.outgoing(&element.id).collect(),
        RelationRole::Target => graph.incoming(&element.id).collect(),
    };
    relationships.into_iter().any(|rel| {
        if rel.kind != filter.kind {
            return false;
        }
        let other_id = match filter.role {
            RelationRole::Source => &rel.target,
            RelationRole::Target => &rel.source,
        };
        filter.target_kind.as_ref().map_or(true, |kind| {
            graph
                .get_element(other_id)
                .is_some_and(|other| &other.kind == kind)
        })
    })
}

fn is_unverified_requirement(graph: &ModelGraph, element: &Element) -> bool {
    if !matches!(element.kind, ElementKind::RequirementUsage | ElementKind::RequirementDefinition) {
        return false;
    }
    core_query::requirements_unverified(graph).any(|candidate| candidate.id == element.id)
}

fn matches_view(graph: &ModelGraph, element: &Element, viewpoint_id: Option<&ElementId>) -> bool {
    if !matches!(element.kind, ElementKind::ViewUsage | ElementKind::ViewDefinition) {
        return false;
    }
    match viewpoint_id {
        Some(id) => sysml_core::views_by_viewpoint(graph, id)
            .iter()
            .any(|summary| summary.id == element.id),
        None => true,
    }
}

fn matches_viewpoint(graph: &ModelGraph, element: &Element, stakeholder_id: Option<&ElementId>) -> bool {
    if !matches!(
        element.kind,
        ElementKind::ViewpointUsage | ElementKind::ViewpointDefinition
    ) {
        return false;
    }
    match stakeholder_id {
        Some(id) => sysml_core::viewpoints_by_stakeholder(graph, id)
            .iter()
            .any(|viewpoint_id| viewpoint_id == &element.id),
        None => true,
    }
}

fn sort_rows(graph: &ModelGraph, rows: &mut Vec<&Element>, sort: &[SortKey]) {
    rows.sort_by(|a, b| {
        for key in sort {
            let ord = compare_sort_key(graph, a, b, key.field);
            let ord = match key.dir {
                SortDir::Asc => ord,
                SortDir::Desc => ord.reverse(),
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        a.id.cmp(&b.id)
    });
}

fn compare_sort_key(graph: &ModelGraph, a: &Element, b: &Element, field: SortField) -> Ordering {
    match field {
        SortField::Name => a.name.cmp(&b.name),
        SortField::QualifiedName => qualified_name(graph, a).cmp(&qualified_name(graph, b)),
        SortField::Kind => format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)),
        SortField::OwnerDepth => owner_depth(graph, a).cmp(&owner_depth(graph, b)),
    }
}

fn owner_depth(graph: &ModelGraph, element: &Element) -> usize {
    let mut depth = 0;
    let mut current = element.owner.clone();
    while let Some(id) = current {
        depth += 1;
        current = graph.get_element(&id).and_then(|owner| owner.owner.clone());
    }
    depth
}

fn summarize(
    graph: &ModelGraph,
    element: &Element,
    expansion: Option<SummaryExpansion>,
) -> ElementSummary {
    ElementSummary {
        id: element.id.clone(),
        name: element.name.clone(),
        qualified_name: qualified_name(graph, element),
        kind: element.kind.clone(),
        owner_id: element.owner.clone(),
        source_span: element.spans.first().cloned(),
        expansion,
    }
}

fn qualified_name(graph: &ModelGraph, element: &Element) -> Option<String> {
    element
        .qname
        .as_ref()
        .map(|qname| qname.to_string())
        .or_else(|| graph.build_qualified_name(&element.id).map(|qname| qname.to_string()))
}

fn build_view_expansion_map(graph: &ModelGraph) -> HashMap<ElementId, SummaryExpansion> {
    build_view_index(graph)
        .into_iter()
        .map(|summary| (summary.id.clone(), SummaryExpansion::View(summary)))
        .collect()
}

// ---------------------------------------------------------------------------
// Requirement table rows (B2 — Requirements workbench)
// ---------------------------------------------------------------------------

/// Three-state verification classification (requirements-workbench-design.md §5):
/// a rollup containing a recorded fail beats everything; anything short of
/// "every linked case passed" is incomplete, including having no linked
/// verification cases at all.
///
/// Also the per-case verdict class the caller supplies to [`requirement_rows`]:
/// sysml-query cannot depend on sysml-runtime's `VerdictKind` (layers never
/// depend upward), so the service maps `Fail|Error → Fail`,
/// `Inconclusive → Incomplete`, `Pass → Pass` before calling in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequirementVerificationState {
    Fail,
    Incomplete,
    Pass,
}

impl RequirementVerificationState {
    pub fn all_variants() -> &'static [RequirementVerificationState] {
        &[
            RequirementVerificationState::Fail,
            RequirementVerificationState::Incomplete,
            RequirementVerificationState::Pass,
        ]
    }
}

/// A linked element on a requirement row (satisfier, verification case,
/// derivation endpoint, refinement target). Identity is the `ElementId`;
/// `name` is display-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementLinkRef {
    pub id: ElementId,
    pub name: Option<String>,
    pub kind: ElementKind,
}

/// Verification rollup for one requirement row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementVerificationRollup {
    pub state: RequirementVerificationState,
    /// Number of verification cases linked via `Verify` edges.
    pub cases_total: usize,
    /// How many of those cases passed.
    pub cases_passed: usize,
    /// HOW the underlying case verdicts were computed — `"static"`
    /// (constraints against current/default values) or `"trajectory"`
    /// (against a live run). BINDING label (§2.1a ruling (d), 2026-07-17):
    /// a static verdict on an ODE-backed case answers a different question
    /// and must never read as "the trajectory ran". Caller-supplied
    /// alongside `case_verdicts` — the map's producer is the only party
    /// that knows how it evaluated. `#[serde(default)]` (empty) only for
    /// wire compat with pre-ruling payloads.
    #[serde(default)]
    pub evaluation_mode: String,
}

/// One row of the requirements workbench table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementRow {
    pub id: ElementId,
    pub kind: ElementKind,
    /// The requirement ID — the declared short name (`<'REQ-001'>`). Per SysML
    /// spec §7.21.2 / the API's `reqId`, this IS the requirement identifier.
    pub req_id: Option<String>,
    pub name: Option<String>,
    /// Statement text: the bodies of all owned `Documentation` elements,
    /// joined in source order with a blank line. `None` when the requirement
    /// carries no doc comments.
    pub text: Option<String>,
    pub qualified_name: Option<String>,
    /// Nearest `Package` ancestor, when one exists.
    pub owning_package: Option<RequirementLinkRef>,
    /// Primary source span — rows are returned in document order
    /// (file, then byte offset), and this is the key that order comes from.
    pub source_span: Option<Span>,
    /// Outline nesting depth: the count of Requirement{Definition,Usage}
    /// ANCESTORS only — deliberately not raw containment depth (see
    /// `SortField::OwnerDepth` for that). A top-level requirement in any
    /// package is depth 0. Tool-side rendering convention, not model
    /// semantics.
    pub outline_depth: usize,
    /// Content-maturity status from a `ModelingMetadata::StatusInfo`
    /// annotation (e.g. "tbd", "done"), when present.
    pub maturity: Option<String>,
    pub satisfied_by: Vec<RequirementLinkRef>,
    pub verified_by: Vec<RequirementLinkRef>,
    pub verification: RequirementVerificationRollup,
    /// DECLARED verification methods (B4): the union of
    /// `@VerificationMethod{ kind = … }` annotations across this row's
    /// verifying cases (`verified_by` order, first-seen dedup). Model
    /// INTENT — layer (1) of the B10 taxonomy — distinct from
    /// `verification.evaluation_mode`, which is what the tool computed;
    /// consumers must not conflate the two chips. Empty when no verifying
    /// case declares a method.
    #[serde(default)]
    pub verification_methods: Vec<String>,
    /// Requirements this row derives FROM (outgoing `Derive` targets — the
    /// originals).
    pub derived_from: Vec<RequirementLinkRef>,
    /// Requirements derived from this row (incoming `Derive` sources).
    pub derives: Vec<RequirementLinkRef>,
    /// Requirements this row refines (outgoing `Refine` targets).
    pub refines: Vec<RequirementLinkRef>,
}

/// Request shape for [`requirement_rows`]. Rows are always document-ordered —
/// the workbench table is an outline, so order is source order by contract,
/// never a caller-chosen sort.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
pub struct RequirementRowsSpec {
    /// Optional extra filter applied on top of the base row scope
    /// (Requirement{Definition,Usage}, library elements excluded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Include verification check occurrences (`RequirementUsage`s owned
    /// by a `RequirementVerificationMembership` — the `verify requirement
    /// chk : Req;` declaration form) as rows. Default FALSE (steward
    /// ruling 2026-07-16): the normative library classifies these as "a
    /// record of the evaluations" — verification bookkeeping referencing a
    /// requirement, not peer requirement content. They remain fully
    /// visible via a def's `verified_by` rollup, and this flag reveals
    /// them as rows on request (they are user-authored source — a
    /// no-escape-hatch exclusion would hide content). A built-in role
    /// toggle, deliberately NOT a `Filter` variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_verification_occurrences: Option<bool>,
}

/// Paged requirement-rows response. Same envelope semantics as
/// [`QueryResult`] (cursor, revision, invalidation), distinct row payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementRowsResult {
    pub rows: Vec<RequirementRow>,
    pub total_estimate: Option<usize>,
    pub cursor: Option<String>,
    pub cursor_invalidated: bool,
    pub revision: u64,
}

/// Build the requirements-workbench table rows: every non-library
/// Requirement{Definition,Usage} in the graph, in document order, with
/// statement text, outline depth, maturity, link rollups, and the
/// three-state verification classification.
///
/// `case_verdicts` maps verification-case `ElementId`s to their evaluated
/// verdict class. It is a caller-computed input for the same reason the
/// elaborated `graph` is: producing it needs machinery from higher layers
/// (verification evaluation), while the row shape has exactly one home here.
/// A case absent from the map counts as not-run (→ incomplete).
pub fn requirement_rows(
    graph: &ModelGraph,
    spec: &RequirementRowsSpec,
    case_verdicts: &HashMap<ElementId, RequirementVerificationState>,
    evaluation_mode: &str,
    revision: u64,
    profile: QueryProfile,
) -> Result<RequirementRowsResult, QueryError> {
    if let Some(filter) = &spec.filter {
        validate_filter(filter, 0)?;
    }
    let limit = spec.limit.unwrap_or_else(|| profile.default_limit()).min(MAX_LIMIT);
    let cursor_payload = match &spec.cursor {
        Some(cursor) => Some(decode_cursor(cursor)?),
        None => None,
    };
    let cursor_invalidated = cursor_payload
        .as_ref()
        .is_some_and(|payload| payload.revision != revision);

    let include_checks = spec.include_verification_occurrences.unwrap_or(false);
    let mut rows: Vec<&Element> = graph
        .elements
        .values()
        .filter(|element| is_requirement_kind(element.kind.clone()))
        .filter(|element| !graph.is_library_element(&element.id))
        .filter(|element| {
            include_checks || !core_query::is_verification_check_usage(graph, &element.id)
        })
        .filter(|element| {
            spec.filter
                .as_ref()
                .map_or(true, |filter| matches_filter(graph, element, filter))
        })
        .collect();
    rows.sort_by(|a, b| document_order(a, b));

    let total = rows.len();
    let (selected, next_cursor) = paginate(&rows, limit, cursor_payload.as_ref(), revision)?;

    let rows = selected
        .into_iter()
        .map(|element| build_requirement_row(graph, element, case_verdicts, evaluation_mode))
        .collect();

    Ok(RequirementRowsResult {
        rows,
        total_estimate: Some(total),
        cursor: next_cursor,
        cursor_invalidated,
        revision,
    })
}

/// Union of declared `@VerificationMethod` kinds across the given verifying
/// cases, first-seen dedup in the caller's (name-sorted) case order. The
/// per-case read is the one-home `sysml_core::metadata::verification_methods`.
fn declared_methods_union(graph: &ModelGraph, cases: &[RequirementLinkRef]) -> Vec<String> {
    let mut methods: Vec<String> = Vec::new();
    for case in cases {
        for method in sysml_core::metadata::verification_methods(graph, &case.id) {
            if !methods.iter().any(|m| *m == method) {
                methods.push(method);
            }
        }
    }
    methods
}

/// Document order: (file, first-span byte offset); spanless elements sort
/// last; ties break on name then id for determinism.
fn document_order(a: &Element, b: &Element) -> Ordering {
    let key_a = a.spans.first().map(|s| (s.file.as_str(), s.start));
    let key_b = b.spans.first().map(|s| (s.file.as_str(), s.start));
    match (key_a, key_b) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
    .then_with(|| a.name.cmp(&b.name))
    .then_with(|| a.id.cmp(&b.id))
}

fn build_requirement_row(
    graph: &ModelGraph,
    element: &Element,
    case_verdicts: &HashMap<ElementId, RequirementVerificationState>,
    evaluation_mode: &str,
) -> RequirementRow {
    let satisfied_by = linked_refs(graph, &element.id, RelationshipKind::Satisfy, LinkEnd::Incoming);
    // Verify links go through the ONE rollup home (`elements_verifying`),
    // not raw incoming edges: the declaration form `verify requirement
    // check : ReqDef;` mints its edge onto the membership-owned check-usage,
    // and the def row is verified THROUGH it.
    let mut verified_by: Vec<RequirementLinkRef> = core_query::elements_verifying(graph, &element.id)
        .into_iter()
        .map(|case| RequirementLinkRef {
            id: case.id.clone(),
            name: case.name.clone(),
            kind: case.kind.clone(),
        })
        .collect();
    verified_by.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let verification = classify_verification(&verified_by, case_verdicts, evaluation_mode);
    let verification_methods = declared_methods_union(graph, &verified_by);

    RequirementRow {
        id: element.id.clone(),
        kind: element.kind.clone(),
        req_id: element.effective_short_name(graph).map(str::to_owned),
        name: element.name.clone(),
        text: requirement_doc_text(graph, &element.id),
        qualified_name: qualified_name(graph, element),
        owning_package: owning_package_ref(graph, element),
        source_span: element.spans.first().cloned(),
        outline_depth: outline_depth(graph, element),
        maturity: sysml_core::metadata::status_info_value(graph, &element.id),
        satisfied_by,
        verified_by,
        verification,
        verification_methods,
        derived_from: linked_refs(graph, &element.id, RelationshipKind::Derive, LinkEnd::Outgoing),
        derives: linked_refs(graph, &element.id, RelationshipKind::Derive, LinkEnd::Incoming),
        refines: linked_refs(graph, &element.id, RelationshipKind::Refine, LinkEnd::Outgoing),
    }
}

/// Statement text: bodies of all owned `Documentation` children, source
/// order, joined with a blank line. The spec's own examples put multiple
/// `doc` comments on one element, each independently meaningful — first-only
/// would silently drop model content. NOTE: `Documentation.locale` is
/// intentionally unhandled in v1 — locale-tagged multi-language docs will be
/// joined into one string; a locale-aware read is a deliberate future cut,
/// not an oversight.
fn requirement_doc_text(graph: &ModelGraph, id: &ElementId) -> Option<String> {
    let mut docs: Vec<&Element> = graph
        .children_of(id)
        .filter(|child| child.kind == ElementKind::Documentation)
        .collect();
    docs.sort_by(|a, b| document_order(a, b));
    let bodies: Vec<&str> = docs
        .iter()
        .filter_map(|doc| doc.get_prop("body").and_then(|v| v.as_str()))
        .filter(|body| !body.is_empty())
        .collect();
    if bodies.is_empty() {
        None
    } else {
        Some(bodies.join("\n\n"))
    }
}

fn owning_package_ref(graph: &ModelGraph, element: &Element) -> Option<RequirementLinkRef> {
    let mut current = element.owner.clone();
    while let Some(owner_id) = current {
        let owner = graph.get_element(&owner_id)?;
        if owner.kind == ElementKind::Package {
            return Some(RequirementLinkRef {
                id: owner.id.clone(),
                name: owner.name.clone(),
                kind: owner.kind.clone(),
            });
        }
        current = owner.owner.clone();
    }
    None
}

/// Count of requirement-kind ancestors — the outline nesting depth.
fn outline_depth(graph: &ModelGraph, element: &Element) -> usize {
    let mut depth = 0;
    let mut current = element.owner.clone();
    while let Some(owner_id) = current {
        let Some(owner) = graph.get_element(&owner_id) else {
            break;
        };
        if is_requirement_kind(owner.kind.clone()) {
            depth += 1;
        }
        current = owner.owner.clone();
    }
    depth
}

#[derive(Clone, Copy)]
enum LinkEnd {
    /// Follow incoming edges; the linked element is the SOURCE.
    Incoming,
    /// Follow outgoing edges; the linked element is the TARGET.
    Outgoing,
}

fn linked_refs(
    graph: &ModelGraph,
    id: &ElementId,
    kind: RelationshipKind,
    end: LinkEnd,
) -> Vec<RequirementLinkRef> {
    let ids: Vec<&ElementId> = match end {
        LinkEnd::Incoming => graph
            .incoming(id)
            .filter(|rel| rel.kind == kind)
            .map(|rel| &rel.source)
            .collect(),
        LinkEnd::Outgoing => graph
            .outgoing(id)
            .filter(|rel| rel.kind == kind)
            .map(|rel| &rel.target)
            .collect(),
    };
    let mut refs: Vec<RequirementLinkRef> = ids
        .into_iter()
        .filter_map(|linked_id| graph.get_element(linked_id))
        .map(|linked| RequirementLinkRef {
            id: linked.id.clone(),
            name: linked.name.clone(),
            kind: linked.kind.clone(),
        })
        .collect();
    refs.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    refs
}

/// Row-level three-state classification: a recorded fail wins; anything short
/// of "all linked cases evaluated and passed" (including zero links, or a case
/// missing from the verdict map) is incomplete.
fn classify_verification(
    verified_by: &[RequirementLinkRef],
    case_verdicts: &HashMap<ElementId, RequirementVerificationState>,
    evaluation_mode: &str,
) -> RequirementVerificationRollup {
    let cases_total = verified_by.len();
    let mut cases_passed = 0usize;
    let mut any_fail = false;
    let mut any_incomplete = cases_total == 0;
    for case in verified_by {
        match case_verdicts.get(&case.id) {
            Some(RequirementVerificationState::Pass) => cases_passed += 1,
            Some(RequirementVerificationState::Fail) => any_fail = true,
            Some(RequirementVerificationState::Incomplete) | None => any_incomplete = true,
        }
    }
    let state = if any_fail {
        RequirementVerificationState::Fail
    } else if any_incomplete {
        RequirementVerificationState::Incomplete
    } else {
        RequirementVerificationState::Pass
    };
    RequirementVerificationRollup {
        state,
        cases_total,
        cases_passed,
        evaluation_mode: evaluation_mode.to_owned(),
    }
}

// ── Requirement detail (R18 / B2.1) ─────────────────────────────────────
//
// The per-element "evaluated contract" read: a requirement IS a constraint
// check (`Requirements::RequirementCheck` — `result = allTrue(assumptions)
// implies allTrue(constraints)`), so a verdict is only inspectable when its
// inputs are visible. This is deliberately NOT a `RequirementRow` extension:
// rows are walked to exhaustion for grid/document mode, while the contract
// payload is unbounded and only the selected row surfaces it (design doc
// §2.1; precedent: `sysml.element` / `sysml.get_source`).

/// One assume/require constraint on a requirement — a verdict input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementConstraint {
    pub id: ElementId,
    pub name: Option<String>,
    /// Inline constraint body, pretty-printed from the structured expression
    /// AST (v2 unification — the parser mints real expression subtrees for
    /// assume/require bodies; verbatim source text only backstops
    /// hand-crafted graphs). `None` for the pure reference form.
    pub text: Option<String>,
    /// Reference-form target (`require constraint : SomeDef;`), when the
    /// referenced name resolves unambiguously in the graph. An ambiguous or
    /// unresolvable name yields `None` — never a guessed link (ADR-009).
    pub referenced_definition: Option<RequirementLinkRef>,
    /// The chain ancestor this constraint was inherited from, `None` for
    /// an OWNED constraint. Provenance labeling is BINDING (steward ruling
    /// 2026-07-16, upheld by §2.1a): an unlabeled inherited row is
    /// indistinguishable from an owned one and misleads about where to
    /// edit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_from: Option<RequirementLinkRef>,
    /// HOW the ancestor was reached — `"typing"` (`usage : Def`) or
    /// `"specialization"` (`def A :> B`), the edge INTO `inherited_from`.
    /// BINDING with the full-chain closure (§2.1a ruling 2026-07-17): a row
    /// that travelled two hops must never be misreported as one. `None`
    /// exactly when `inherited_from` is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_via: Option<String>,
}

/// An attribute owned by the requirement, with its statically-declared
/// value — the values assume/require constraint text reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementAttribute {
    pub id: ElementId,
    pub name: Option<String>,
    /// Declared/default value rendered via the model `Display` form
    /// (`40 [ms]`). `None` when the attribute declares no static value.
    pub value: Option<String>,
    /// Live session value, when the caller supplies one (the
    /// `case_verdicts` precedent: computing it needs higher-layer machinery,
    /// the shape has one home here).
    pub live_value: Option<String>,
}

/// The evaluated contract + narrative context of one requirement.
///
/// Bucket separation is binding (design doc §2.1): `subject`,
/// `assumed_constraints`/`required_constraints`, and
/// `referenced_attributes` are VERDICT INPUTS; `framed_concerns`, `actors`,
/// and `stakeholders` are narrative roles — consumers must not render them
/// next to the verdict chip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementDetail {
    pub id: ElementId,
    /// The subject parameter, from the elaborated `subject` ref (for the
    /// declaration form this targets the requirement's own
    /// SubjectMembership — the parameter itself).
    pub subject: Option<RequirementLinkRef>,
    /// OWNED constraints only — these names deliberately mirror the spec's
    /// owned-only derived properties `assumedConstraint`/`requiredConstraint`
    /// (SysML-vocab.ttl) and must never absorb inherited rows.
    pub assumed_constraints: Vec<RequirementConstraint>,
    pub required_constraints: Vec<RequirementConstraint>,
    /// Constraints the verdict evaluates from the requirement's FULL
    /// inheritance chain (§2.1a ruling 2026-07-17: unconditional closure —
    /// FeatureTyping + def specialization, transitive, KerML
    /// redefinition-suppressed; the 2026-07-16 owns-none single-hop gate is
    /// superseded). Populated UNCONDITIONALLY — inherited rows show even
    /// when the requirement owns constraints, because the evaluator
    /// aggregates both. Rows come from the SAME shared walker
    /// (`sysml-core::query::effective_requirement_constraints`) the
    /// evaluator uses; each carries `inherited_from` + `inherited_via`
    /// hop-kind provenance. A suppressed (redefined) inherited member is
    /// replaced by its redefining member, never shown twice.
    #[serde(default)]
    pub inherited_assumed_constraints: Vec<RequirementConstraint>,
    #[serde(default)]
    pub inherited_required_constraints: Vec<RequirementConstraint>,
    /// CONTENT requirement usages typed by this element — the reverse of
    /// the inherited-contract "· from" edge (single hop, same shared
    /// primitives, steward ruling 2026-07-16). Verification check
    /// occurrences are excluded by the same role predicate rows use: a
    /// check typed by this def is already visible through `verified_by`,
    /// and one fact must not appear under two taxonomies. Lives on the
    /// detail, not the row — reverse fan-out is unbounded and rows are
    /// walked to exhaustion.
    #[serde(default)]
    pub instantiated_by: Vec<RequirementLinkRef>,
    pub framed_concerns: Vec<RequirementLinkRef>,
    pub actors: Vec<RequirementLinkRef>,
    pub stakeholders: Vec<RequirementLinkRef>,
    pub referenced_attributes: Vec<RequirementAttribute>,
    /// Bodies of `Rationale` metadata annotations, source order, joined
    /// with a blank line (the multi-doc convention `requirement_doc_text`
    /// uses).
    pub rationale: Option<String>,
    /// DECLARED verification methods (B4) — same union-over-verifying-cases
    /// read as `RequirementRow::verification_methods` (one rollup home:
    /// `elements_verifying`), so the rail chip and the grid column can
    /// never disagree. Model intent, NOT what the tool computed.
    #[serde(default)]
    pub verification_methods: Vec<String>,
}

/// Compose the detail read for one requirement. Fails hard on an unknown id
/// or a non-requirement element — a detail view over the wrong element kind
/// is a caller bug, not a degraded mode.
///
/// `live_values` maps attribute `ElementId`s to session-evaluated display
/// values; pass an empty map when no session is running.
pub fn requirement_detail(
    graph: &ModelGraph,
    id: &ElementId,
    live_values: &HashMap<ElementId, String>,
) -> Result<RequirementDetail, QueryError> {
    let element = graph
        .get_element(id)
        .ok_or_else(|| QueryError::Invalid(format!("element not found: {id}")))?;
    if !is_requirement_kind(element.kind.clone()) {
        return Err(QueryError::Invalid(format!(
            "not a requirement: {id} is {:?}",
            element.kind
        )));
    }

    let subject = element
        .get_prop("subject")
        .and_then(Value::as_ref)
        .and_then(|subject_id| link_ref_to(graph, subject_id));

    // Full-chain contract (§2.1a ruling 2026-07-17, superseding the
    // 2026-07-16 owns-none single hop): owned + inherited buckets both come
    // from the ONE shared walker the evaluator uses
    // (`effective_requirement_constraints` — unconditional closure over
    // typing + def specialization, redefinition-suppressed), so the
    // displayed contract and the verdict can never drift. The walker's
    // per-level ordering follows `children_of` (a hash index) — members are
    // re-sorted into document order within each origin level, preserving
    // the nearest-first level order itself.
    let mut assumed_constraints = Vec::new();
    let mut required_constraints = Vec::new();
    let mut inherited_assumed_constraints = Vec::new();
    let mut inherited_required_constraints = Vec::new();
    {
        let mut members = core_query::effective_requirement_constraints(element, graph);
        // Group-stable sort: origin levels keep their chain order (owned
        // first, then nearest ancestor outward); members within one level
        // sort by document order.
        let level_of = |m: &core_query::EffectiveRequirementConstraint| {
            m.origin.map(|o| o.id.clone())
        };
        let mut level_rank: HashMap<Option<ElementId>, usize> = HashMap::new();
        for m in &members {
            let next = level_rank.len();
            level_rank.entry(level_of(m)).or_insert(next);
        }
        members.sort_by(|a, b| {
            level_rank[&level_of(a)]
                .cmp(&level_rank[&level_of(b)])
                .then_with(|| document_order(a.element, b.element))
        });
        for member in members {
            // Display renders membership rows only (bare `ConstraintUsage`
            // children have no assume/require role surface here — same
            // reader scope as before the widening).
            if member.element.kind != ElementKind::RequirementConstraintMembership {
                continue;
            }
            let row = constraint_membership_row(graph, member.element, member.origin, member.via);
            let is_assume = member.role == core_query::RequirementConstraintRole::Assume;
            match (member.origin.is_some(), is_assume) {
                (false, true) => assumed_constraints.push(row),
                (false, false) => required_constraints.push(row),
                (true, true) => inherited_assumed_constraints.push(row),
                (true, false) => inherited_required_constraints.push(row),
            }
        }
    }

    let framed_concerns = children_of_kind(graph, id, ElementKind::FramedConcernMembership)
        .into_iter()
        .map(|membership| RequirementLinkRef {
            id: membership.id.clone(),
            name: membership.name.clone(),
            kind: membership.kind.clone(),
        })
        .collect();

    let referenced_attributes = children_of_kind(graph, id, ElementKind::AttributeUsage)
        .into_iter()
        .map(|attr| RequirementAttribute {
            id: attr.id.clone(),
            // A template-instantiation binding (`attribute :>> gap = 8.0;`)
            // carries no name of its own — display the redefined feature's
            // name, the same key the evaluator binds under.
            name: attr
                .name
                .clone()
                .or_else(|| core_query::redefined_feature_name(attr, graph)),
            value: static_attribute_value(attr),
            live_value: live_values.get(&attr.id).cloned(),
        })
        .collect();

    let rationale_bodies: Vec<String> = children_of_kind(graph, id, ElementKind::MetadataUsage)
        .into_iter()
        .filter(|child| sysml_core::metadata::is_metadata_typed_as(graph, child, "Rationale"))
        .filter_map(|child| sysml_core::metadata::metadata_string_attr(graph, &child.id, "text"))
        .filter(|text| !text.is_empty())
        .collect();

    let mut instantiated_by: Vec<RequirementLinkRef> = core_query::requirement_usages_typed_by(
        graph, id,
    )
    .into_iter()
    .filter(|usage| !core_query::is_verification_check_usage(graph, &usage.id))
    .map(|usage| RequirementLinkRef {
        id: usage.id.clone(),
        name: usage.name.clone(),
        kind: usage.kind.clone(),
    })
    .collect();
    instantiated_by.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));

    // Same verifying-case set (and name-sorted order) the row's
    // `verified_by` uses, so the two surfaces stay in lockstep.
    let mut verifying: Vec<RequirementLinkRef> = core_query::elements_verifying(graph, id)
        .into_iter()
        .map(|case| RequirementLinkRef {
            id: case.id.clone(),
            name: case.name.clone(),
            kind: case.kind.clone(),
        })
        .collect();
    verifying.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let verification_methods = declared_methods_union(graph, &verifying);

    Ok(RequirementDetail {
        id: element.id.clone(),
        subject,
        assumed_constraints,
        required_constraints,
        inherited_assumed_constraints,
        inherited_required_constraints,
        instantiated_by,
        framed_concerns,
        actors: role_refs(graph, element, "actors"),
        stakeholders: role_refs(graph, element, "stakeholders"),
        referenced_attributes,
        rationale: if rationale_bodies.is_empty() {
            None
        } else {
            Some(rationale_bodies.join("\n\n"))
        },
        verification_methods,
    })
}

/// One constraint-membership row — the ONE row reader for owned and
/// inherited members alike (`inherited_from`/`inherited_via` distinguish
/// them); the member set itself comes from the shared sysml-core walker.
fn constraint_membership_row(
    graph: &ModelGraph,
    membership: &Element,
    origin: Option<&Element>,
    via: Option<core_query::RequirementChainHop>,
) -> RequirementConstraint {
    RequirementConstraint {
        id: membership.id.clone(),
        name: membership.name.clone(),
        // AST-first: the parser mints a structured expression subtree for
        // assume/require bodies (v2 unification, workbench design §7.1) on
        // the membership's owned ConstraintUsage (the spec's
        // `ownedConstraint`) — pretty-print via the shared body-owner hop;
        // the verbatim `constraint` prop only backstops hand-crafted graphs.
        text: sysml_core::expression_pretty::pretty_print_owner(
            core_query::requirement_constraint_body_owner(membership, graph),
            graph,
        )
        .or_else(|| {
            membership
                .get_prop("constraint")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        }),
        // The reference form's target, resolved from its ReferenceSubsetting
        // (bare-name) or FeatureTyping (`: Def`) — SysML-vocab.ttl:2576,
        // identity via the derived accessor, not a name-string lookup.
        referenced_definition: core_query::referenced_constraint_target(membership, graph)
            .and_then(|target| link_ref_to(graph, &target.id)),
        inherited_from: origin.map(|o| RequirementLinkRef {
            id: o.id.clone(),
            name: o.name.clone(),
            kind: o.kind.clone(),
        }),
        inherited_via: via.map(|hop| {
            match hop {
                core_query::RequirementChainHop::Typing => "typing",
                core_query::RequirementChainHop::Specialization => "specialization",
            }
            .to_owned()
        }),
    }
}

/// Children of one kind, in source order (the same document-order key the
/// row walk uses — `children_of` iterates a hash index).
fn children_of_kind<'g>(
    graph: &'g ModelGraph,
    id: &ElementId,
    kind: ElementKind,
) -> Vec<&'g Element> {
    let mut children: Vec<&Element> = graph
        .children_of(id)
        .filter(|child| child.kind == kind)
        .collect();
    children.sort_by(|a, b| document_order(a, b));
    children
}

fn link_ref_to(graph: &ModelGraph, id: &ElementId) -> Option<RequirementLinkRef> {
    graph.get_element(id).map(|el| RequirementLinkRef {
        id: el.id.clone(),
        name: el.name.clone(),
        kind: el.kind.clone(),
    })
}

/// Link refs from an elaborated role-list prop (`actors`/`stakeholders` =
/// `Value::List` of `Value::Ref`, source-ordered by the elaboration pass).
fn role_refs(graph: &ModelGraph, element: &Element, prop_key: &str) -> Vec<RequirementLinkRef> {
    let Some(Value::List(refs)) = element.get_prop(prop_key) else {
        return Vec::new();
    };
    refs.iter()
        .filter_map(Value::as_ref)
        .filter_map(|id| link_ref_to(graph, id))
        .collect()
}

/// Statically-declared attribute value, rendered to display text. Prop
/// precedence mirrors `metadata_string_attr`'s read order.
fn static_attribute_value(attr: &Element) -> Option<String> {
    for prop in ["value", "default", "unresolved_value"] {
        if let Some(text) = attr.get_prop(prop).and_then(|v| suspect::prop_display_text(v)) {
            return Some(text);
        }
    }
    None
}

fn encode_cursor(revision: u64, last_id: ElementId) -> Result<String, QueryError> {
    let payload = CursorPayload {
        v: CURSOR_VERSION,
        revision,
        last_id,
    };
    let json = serde_json::to_vec(&payload).map_err(|err| QueryError::Cursor(err.to_string()))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

fn decode_cursor(cursor: &str) -> Result<CursorPayload, QueryError> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|err| QueryError::Cursor(err.to_string()))?;
    let payload: CursorPayload =
        serde_json::from_slice(&bytes).map_err(|err| QueryError::Cursor(err.to_string()))?;
    if payload.v != CURSOR_VERSION {
        return Err(QueryError::Cursor(format!(
            "unsupported cursor version {}",
            payload.v
        )));
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use sysml_core::Relationship;

    fn graph() -> (ModelGraph, ElementId, ElementId, ElementId) {
        let mut graph = ModelGraph::new();
        let pkg_id = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("Pkg"));
        let part_id = graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("Engine")
                .with_owner(pkg_id.clone()),
        );
        let req_id = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("ReqPower")
                .with_owner(pkg_id.clone()),
        );
        graph.add_relationship(Relationship::new(
            RelationshipKind::Satisfy,
            part_id.clone(),
            req_id.clone(),
        ));
        (graph, pkg_id, part_id, req_id)
    }

    #[test]
    fn kind_filter_summary_projection() {
        let (graph, _, part_id, _) = graph();
        let spec = QuerySpec {
            filter: Filter::Kind {
                kinds: vec![ElementKind::PartUsage],
            },
            projection: Projection::Summary,
            ..QuerySpec::default()
        };
        let result = execute_query(&graph, &spec, graph_revision(&graph)).unwrap();
        let QueryRows::Summary(rows) = result.rows else {
            panic!("expected summary rows");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, part_id);
        assert_eq!(rows[0].qualified_name.as_deref(), Some("Pkg::Engine"));
    }

    #[test]
    fn user_authored_filter_excludes_library_elements() {
        // requirement_fixture registers a library package owning a
        // RequirementDefinition named RequirementCheck.
        let fx = requirement_fixture();
        let all = QuerySpec {
            filter: Filter::Kind {
                kinds: vec![ElementKind::RequirementDefinition],
            },
            projection: Projection::Summary,
            ..QuerySpec::default()
        };
        let names = |spec: &QuerySpec| -> Vec<String> {
            let result = execute_query(&fx.graph, spec, graph_revision(&fx.graph)).unwrap();
            let QueryRows::Summary(rows) = result.rows else {
                panic!("expected summary rows");
            };
            rows.into_iter().filter_map(|r| r.name).collect()
        };
        assert!(
            names(&all).contains(&"RequirementCheck".to_owned()),
            "without the filter, library elements match"
        );
        let user_only = QuerySpec {
            filter: Filter::All {
                filters: vec![
                    Filter::Kind {
                        kinds: vec![ElementKind::RequirementDefinition],
                    },
                    Filter::UserAuthored,
                ],
            },
            projection: Projection::Summary,
            ..QuerySpec::default()
        };
        let user_names = names(&user_only);
        assert!(
            !user_names.contains(&"RequirementCheck".to_owned()),
            "user_authored must exclude library elements: {user_names:?}"
        );
        assert!(user_names.contains(&"SysReq".to_owned()));
    }

    #[test]
    fn name_match_contains_ci() {
        let (graph, _, part_id, _) = graph();
        let spec = QuerySpec {
            filter: Filter::NameMatch {
                name_match: NameMatch {
                    contains: Some("eng".to_owned()),
                    ci: true,
                    ..NameMatch::default()
                },
            },
            projection: Projection::Ids,
            ..QuerySpec::default()
        };
        let result = execute_query(&graph, &spec, graph_revision(&graph)).unwrap();
        assert_eq!(result.rows, QueryRows::Ids(vec![part_id]));
    }

    #[test]
    fn owner_filter_direct() {
        let (graph, pkg_id, part_id, req_id) = graph();
        let spec = QuerySpec {
            filter: Filter::Owner {
                owner: OwnerFilter {
                    id: Some(pkg_id),
                    kind: None,
                    transitive: false,
                },
            },
            projection: Projection::Ids,
            sort: vec![SortKey {
                field: SortField::Name,
                dir: SortDir::Asc,
            }],
            ..QuerySpec::default()
        };
        let result = execute_query(&graph, &spec, graph_revision(&graph)).unwrap();
        assert_eq!(result.rows, QueryRows::Ids(vec![part_id, req_id]));
    }

    #[test]
    fn has_relation_source_target_kind() {
        let (graph, _, part_id, _) = graph();
        let spec = QuerySpec {
            filter: Filter::HasRelation {
                has_relation: RelationFilter {
                    kind: RelationshipKind::Satisfy,
                    role: RelationRole::Source,
                    target_kind: Some(ElementKind::RequirementUsage),
                },
            },
            projection: Projection::Ids,
            ..QuerySpec::default()
        };
        let result = execute_query(&graph, &spec, graph_revision(&graph)).unwrap();
        assert_eq!(result.rows, QueryRows::Ids(vec![part_id]));
    }

    #[test]
    fn count_projection_returns_total() {
        let (graph, _, _, _) = graph();
        let spec = QuerySpec {
            projection: Projection::Count,
            ..QuerySpec::default()
        };
        let result = execute_query(&graph, &spec, graph_revision(&graph)).unwrap();
        assert_eq!(result.rows, QueryRows::Count(3));
        assert_eq!(result.total_estimate, Some(3));
    }

    #[test]
    fn limit_and_cursor_page_results() {
        let (graph, _, _, _) = graph();
        let spec = QuerySpec {
            projection: Projection::Ids,
            limit: Some(1),
            sort: vec![SortKey {
                field: SortField::Name,
                dir: SortDir::Asc,
            }],
            ..QuerySpec::default()
        };
        let rev = graph_revision(&graph);
        let first = execute_query(&graph, &spec, rev).unwrap();
        assert!(first.cursor.is_some());
        let second = execute_query(
            &graph,
            &QuerySpec {
                cursor: first.cursor,
                ..spec
            },
            rev,
        )
        .unwrap();
        assert!(!second.cursor_invalidated);
        let QueryRows::Ids(rows) = second.rows else {
            panic!("expected ids");
        };
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn invalid_name_match_rejected() {
        let (graph, _, _, _) = graph();
        let spec = QuerySpec {
            filter: Filter::NameMatch {
                name_match: NameMatch::default(),
            },
            ..QuerySpec::default()
        };
        assert!(execute_query(&graph, &spec, graph_revision(&graph)).is_err());
    }

    #[test]
    fn element_kind_filter_serde_and_match_coverage() {
        for kind in ElementKind::iter() {
            let mut graph = ModelGraph::new();
            let id = graph.add_element(Element::new_with_kind(kind.clone()).with_name("x"));
            let spec = QuerySpec {
                filter: Filter::Kind { kinds: vec![kind] },
                projection: Projection::Ids,
                ..QuerySpec::default()
            };
            let json = serde_json::to_string(&spec).unwrap();
            let round_trip: QuerySpec = serde_json::from_str(&json).unwrap();
            let result = execute_query(&graph, &round_trip, graph_revision(&graph)).unwrap();
            assert_eq!(result.rows, QueryRows::Ids(vec![id]));
        }
    }

    #[test]
    fn relationship_kind_filter_serde_and_match_coverage() {
        for kind in RelationshipKind::iter() {
            let mut graph = ModelGraph::new();
            let source_id = graph.add_element(Element::new_with_kind(ElementKind::PartUsage).with_name("s"));
            let target_id = graph.add_element(Element::new_with_kind(ElementKind::RequirementUsage).with_name("t"));
            graph.add_relationship(Relationship::new(kind.clone(), source_id.clone(), target_id));
            let spec = QuerySpec {
                filter: Filter::HasRelation {
                    has_relation: RelationFilter {
                        kind: kind.clone(),
                        role: RelationRole::Source,
                        target_kind: None,
                    },
                },
                projection: Projection::Ids,
                ..QuerySpec::default()
            };
            let json = serde_json::to_string(&spec).unwrap();
            let round_trip: QuerySpec = serde_json::from_str(&json).unwrap();
            let result = execute_query(&graph, &round_trip, graph_revision(&graph)).unwrap();
            assert_eq!(result.rows, QueryRows::Ids(vec![source_id]));
        }
    }

    #[test]
    fn query_local_enums_have_coverage_lists() {
        assert_eq!(Projection::all_variants().len(), 5);
        assert_eq!(SortField::all_variants().len(), 4);
        assert_eq!(SortDir::all_variants().len(), 2);
        assert_eq!(RelationRole::all_variants().len(), 2);
        assert_eq!(QueryCacheStatus::all_variants().len(), 3);
        assert_eq!(RequirementVerificationState::all_variants().len(), 3);
    }

    // -- requirement_rows (B2) ------------------------------------------------

    use sysml_span::Span;

    struct ReqFixture {
        graph: ModelGraph,
        sys_req: ElementId,
        sub_req: ElementId,
        other_req: ElementId,
        vc_pass: ElementId,
        vc_fail: ElementId,
        vc_sub: ElementId,
    }

    /// Two files; SysReq (a.sysml) owns SubReq (outline nesting); OtherReq in
    /// b.sysml. SysReq: two docs, short name, StatusInfo maturity, satisfy +
    /// two verify links; SubReq derives-from + refines SysReq.
    fn requirement_fixture() -> ReqFixture {
        let mut graph = ModelGraph::new();
        let pkg_id = graph.add_element(
            Element::new_with_kind(ElementKind::Package)
                .with_name("Reqs")
                .with_span(Span::new("a.sysml", 0, 5)),
        );
        let sys_req = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementDefinition)
                .with_name("SysReq")
                .with_owner(pkg_id.clone())
                .with_prop("declaredShortName", "REQ-001")
                .with_span(Span::new("a.sysml", 10, 200)),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::Documentation)
                .with_owner(sys_req.clone())
                .with_prop("body", "The system shall X.")
                .with_span(Span::new("a.sysml", 20, 39)),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::Documentation)
                .with_owner(sys_req.clone())
                .with_prop("body", "Second clause.")
                .with_span(Span::new("a.sysml", 40, 54)),
        );
        // @StatusInfo { status = StatusKind::tbd; } — parser shape: anonymous
        // MetadataUsage + unresolvedTypeName, attribute child with the value.
        let meta_id = graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(sys_req.clone())
                .with_prop("unresolvedTypeName", "StatusInfo"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("status")
                .with_owner(meta_id)
                .with_prop("value", "StatusKind::tbd"),
        );
        let sub_req = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("SubReq")
                .with_owner(sys_req.clone())
                .with_prop("declaredShortName", "REQ-002")
                .with_span(Span::new("a.sysml", 60, 120)),
        );
        let other_req = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("OtherReq")
                .with_owner(pkg_id.clone())
                .with_span(Span::new("b.sysml", 5, 40)),
        );
        let part_id = graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("amp")
                .with_owner(pkg_id.clone()),
        );
        let vc_pass = graph.add_element(
            Element::new_with_kind(ElementKind::VerificationCaseUsage)
                .with_name("vcPass")
                .with_owner(pkg_id.clone()),
        );
        // vcPass declares @VerificationMethod{ kind = (test, demo); } —
        // parser shape: anonymous MetadataUsage → ReferenceUsage "kind" →
        // comma OperatorExpression with argIndex-ordered references.
        let vm_meta = graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(vc_pass.clone())
                .with_prop("unresolvedTypeName", "VerificationMethod"),
        );
        let vm_kind = graph.add_element(
            Element::new_with_kind(ElementKind::ReferenceUsage)
                .with_name("kind")
                .with_owner(vm_meta),
        );
        let vm_tuple = graph.add_element(
            Element::new_with_kind(ElementKind::OperatorExpression)
                .with_owner(vm_kind)
                .with_prop("operator", ","),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                .with_name("test")
                .with_owner(vm_tuple.clone())
                .with_prop("argIndex", Value::Int(0)),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                .with_name("demo")
                .with_owner(vm_tuple)
                .with_prop("argIndex", Value::Int(1)),
        );
        let vc_fail = graph.add_element(
            Element::new_with_kind(ElementKind::VerificationCaseUsage)
                .with_name("vcFail")
                .with_owner(pkg_id.clone()),
        );
        // vcFail declares @VerificationMethod{ kind = analyze; } via the
        // prop-stored value shape (qualified — must normalize).
        let vm_meta_fail = graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(vc_fail.clone())
                .with_prop("unresolvedTypeName", "VerificationMethod"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("kind")
                .with_owner(vm_meta_fail)
                .with_prop("value", "VerificationMethodKind::analyze"),
        );
        let vc_sub = graph.add_element(
            Element::new_with_kind(ElementKind::VerificationCaseUsage)
                .with_name("vcSub")
                .with_owner(pkg_id.clone()),
        );
        graph.add_relationship(Relationship::new(
            RelationshipKind::Satisfy,
            part_id,
            sys_req.clone(),
        ));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Verify,
            vc_pass.clone(),
            sys_req.clone(),
        ));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Verify,
            vc_fail.clone(),
            sys_req.clone(),
        ));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Verify,
            vc_sub.clone(),
            sub_req.clone(),
        ));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Derive,
            sub_req.clone(),
            sys_req.clone(),
        ));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Refine,
            sub_req.clone(),
            sys_req.clone(),
        ));
        // A library requirement must never appear as a row.
        let lib_pkg = graph.add_element(
            Element::new_with_kind(ElementKind::LibraryPackage).with_name("RequirementsLib"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::RequirementDefinition)
                .with_name("RequirementCheck")
                .with_owner(lib_pkg.clone()),
        );
        graph.register_library_package(lib_pkg);
        ReqFixture {
            graph,
            sys_req,
            sub_req,
            other_req,
            vc_pass,
            vc_fail,
            vc_sub,
        }
    }

    fn fixture_verdicts(f: &ReqFixture) -> HashMap<ElementId, RequirementVerificationState> {
        HashMap::from([
            (f.vc_pass.clone(), RequirementVerificationState::Pass),
            (f.vc_fail.clone(), RequirementVerificationState::Fail),
            (f.vc_sub.clone(), RequirementVerificationState::Pass),
        ])
    }

    fn all_rows(f: &ReqFixture) -> Vec<RequirementRow> {
        let result = requirement_rows(
            &f.graph,
            &RequirementRowsSpec::default(),
            &fixture_verdicts(f),
            "static",
            graph_revision(&f.graph),
            QueryProfile::Service,
        )
        .unwrap();
        result.rows
    }

    #[test]
    fn requirement_rows_document_order_and_depth() {
        let f = requirement_fixture();
        let rows = all_rows(&f);
        assert_eq!(
            rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            vec![f.sys_req.clone(), f.sub_req.clone(), f.other_req.clone()],
            "rows must follow (file, offset) source order, not name order"
        );
        assert_eq!(rows[0].outline_depth, 0);
        assert_eq!(rows[1].outline_depth, 1, "nested requirement is depth 1");
        assert_eq!(rows[2].outline_depth, 0);
    }

    #[test]
    fn requirement_rows_text_short_name_maturity_package() {
        let f = requirement_fixture();
        let rows = all_rows(&f);
        let sys = &rows[0];
        assert_eq!(sys.req_id.as_deref(), Some("REQ-001"));
        assert_eq!(
            sys.text.as_deref(),
            Some("The system shall X.\n\nSecond clause."),
            "all doc bodies join in source order"
        );
        assert_eq!(sys.maturity.as_deref(), Some("tbd"));
        assert_eq!(
            sys.owning_package.as_ref().and_then(|p| p.name.as_deref()),
            Some("Reqs")
        );
        assert_eq!(sys.qualified_name.as_deref(), Some("Reqs::SysReq"));
        assert_eq!(rows[2].text, None);
        assert_eq!(rows[2].maturity, None);
    }

    #[test]
    fn requirement_rows_links_and_three_state_rollup() {
        let f = requirement_fixture();
        let rows = all_rows(&f);
        let sys = &rows[0];
        let sub = &rows[1];
        let other = &rows[2];

        assert_eq!(sys.satisfied_by.len(), 1);
        assert_eq!(sys.satisfied_by[0].name.as_deref(), Some("amp"));
        assert_eq!(sys.verified_by.len(), 2);
        assert_eq!(
            sys.verification,
            RequirementVerificationRollup {
                evaluation_mode: "static".to_owned(),
                state: RequirementVerificationState::Fail,
                cases_total: 2,
                cases_passed: 1,
            },
            "a recorded fail wins the rollup"
        );
        assert_eq!(
            sys.derives.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            vec![f.sub_req.clone()]
        );

        assert_eq!(
            sub.verification.state,
            RequirementVerificationState::Pass,
            "all linked cases passed"
        );
        assert_eq!(
            sub.derived_from.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            vec![f.sys_req.clone()]
        );
        assert_eq!(
            sub.refines.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            vec![f.sys_req.clone()]
        );

        assert_eq!(
            other.verification,
            RequirementVerificationRollup {
                evaluation_mode: "static".to_owned(),
                state: RequirementVerificationState::Incomplete,
                cases_total: 0,
                cases_passed: 0,
            },
            "no verify links is incomplete, not pass"
        );
    }

    #[test]
    fn requirement_rows_declared_methods_union_across_cases() {
        let f = requirement_fixture();
        let rows = all_rows(&f);
        let sys = rows.iter().find(|r| r.id == f.sys_req).unwrap();
        // verified_by is name-sorted: vcFail (analyze, prop shape,
        // qualified) then vcPass (test, demo — argIndex order). First-seen
        // union, declared order preserved within each case.
        assert_eq!(sys.verification_methods, vec!["analyze", "test", "demo"]);
        // A verified requirement whose case declares no method: empty,
        // never a fabricated default.
        let sub = rows.iter().find(|r| r.id == f.sub_req).unwrap();
        assert!(sub.verification_methods.is_empty());
        let other = rows.iter().find(|r| r.id == f.other_req).unwrap();
        assert!(other.verification_methods.is_empty());
    }

    #[test]
    fn requirement_detail_declared_methods_match_row_union() {
        let f = requirement_fixture();
        let detail = requirement_detail(&f.graph, &f.sys_req, &HashMap::new()).unwrap();
        assert_eq!(
            detail.verification_methods,
            vec!["analyze", "test", "demo"],
            "detail and row read the same rollup home — they must agree"
        );
    }

    #[test]
    fn requirement_rows_unmapped_case_is_incomplete() {
        let f = requirement_fixture();
        // Verdict map missing vc_fail: SysReq must degrade to incomplete
        // (case not run), never silently pass.
        let verdicts = HashMap::from([(f.vc_pass.clone(), RequirementVerificationState::Pass)]);
        let result = requirement_rows(
            &f.graph,
            &RequirementRowsSpec::default(),
            &verdicts,
            "static",
            graph_revision(&f.graph),
            QueryProfile::Service,
        )
        .unwrap();
        let sys = result.rows.iter().find(|r| r.id == f.sys_req).unwrap();
        assert_eq!(sys.verification.state, RequirementVerificationState::Incomplete);
        assert_eq!(sys.verification.cases_passed, 1);
    }

    #[test]
    fn requirement_rows_excludes_library_and_pages() {
        let f = requirement_fixture();
        let verdicts = fixture_verdicts(&f);
        let rev = graph_revision(&f.graph);
        let spec = RequirementRowsSpec {
            limit: Some(1),
            ..RequirementRowsSpec::default()
        };
        let first = requirement_rows(&f.graph, &spec, &verdicts, "static", rev, QueryProfile::Service).unwrap();
        assert_eq!(first.total_estimate, Some(3), "library requirement excluded");
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.rows[0].id, f.sys_req);
        let cursor = first.cursor.expect("more pages");

        let second = requirement_rows(
            &f.graph,
            &RequirementRowsSpec {
                limit: Some(1),
                cursor: Some(cursor),
                ..RequirementRowsSpec::default()
            },
            &verdicts,
            "static",
            rev,
            QueryProfile::Service,
        )
        .unwrap();
        assert!(!second.cursor_invalidated);
        assert_eq!(second.rows[0].id, f.sub_req);
        let cursor = second.cursor.expect("one more page");

        let third = requirement_rows(
            &f.graph,
            &RequirementRowsSpec {
                limit: Some(1),
                cursor: Some(cursor),
                ..RequirementRowsSpec::default()
            },
            &verdicts,
            "static",
            rev,
            QueryProfile::Service,
        )
        .unwrap();
        assert_eq!(third.rows[0].id, f.other_req);
        assert!(third.cursor.is_none(), "final page mints no cursor");
    }

    #[test]
    fn requirement_rows_extra_filter_composes() {
        let f = requirement_fixture();
        let spec = RequirementRowsSpec {
            filter: Some(Filter::NameMatch {
                name_match: NameMatch {
                    contains: Some("Sub".to_owned()),
                    ..NameMatch::default()
                },
            }),
            ..RequirementRowsSpec::default()
        };
        let result = requirement_rows(
            &f.graph,
            &spec,
            &fixture_verdicts(&f),
            "static",
            graph_revision(&f.graph),
            QueryProfile::Service,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].id, f.sub_req);
    }

    // ── requirement_detail (R18 / B2.1) ──────────────────────────────

    struct DetailFixture {
        graph: ModelGraph,
        req: ElementId,
        subject_membership: ElementId,
        assume: ElementId,
        require: ElementId,
        constraint_def: ElementId,
        concern: ElementId,
        actor_membership: ElementId,
        attr: ElementId,
    }

    /// One requirement carrying the full contract: subject ref, one assume
    /// (inline text) + one require (reference form), a framed concern, an
    /// actor, a quantity-valued attribute, and a Rationale annotation —
    /// props in the exact shapes the elaborator/parser mint (W1/W2 register
    /// notes + ast_builder/requirements.rs).
    fn detail_fixture() -> DetailFixture {
        let mut graph = ModelGraph::new();
        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let constraint_def = graph.add_element(
            Element::new_with_kind(ElementKind::ConstraintDefinition)
                .with_name("MassLimit")
                .with_owner(pkg.clone()),
        );
        let req = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("ReqTrip")
                .with_owner(pkg.clone()),
        );
        let subject_membership = graph.add_element(
            Element::new_with_kind(ElementKind::SubjectMembership)
                .with_name("breaker")
                .with_owner(req.clone())
                .with_span(Span::new("a.sysml", 10, 20)),
        );
        if let Some(el) = graph.elements.get_mut(&req) {
            el.set_prop("subject", Value::Ref(subject_membership.clone()));
        }
        let assume = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementConstraintMembership)
                .with_owner(req.clone())
                .with_prop("role", "assume")
                .with_prop("constraint", "ambientTemp <= 40 [degC]")
                .with_span(Span::new("a.sysml", 30, 60)),
        );
        // Reference form (`require MassLimit;`) in the spec shape: the membership
        // owns a ConstraintUsage which owns a resolved ReferenceSubsetting to the
        // constraint def — `referenced_constraint_target` reads that identity
        // (SysML-vocab.ttl:2576), no `referencedConstraint` string prop.
        let require = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementConstraintMembership)
                .with_owner(req.clone())
                .with_prop("role", "require")
                .with_span(Span::new("a.sysml", 70, 100)),
        );
        let require_usage = graph.add_element(
            Element::new_with_kind(ElementKind::ConstraintUsage).with_owner(require.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::ReferenceSubsetting)
                .with_owner(require_usage.clone())
                .with_prop("referencingFeature", Value::Ref(require_usage.clone()))
                .with_prop("unresolved_referencedFeature", "MassLimit")
                .with_prop("referencedFeature", Value::Ref(constraint_def.clone())),
        );
        let concern = graph.add_element(
            Element::new_with_kind(ElementKind::FramedConcernMembership)
                .with_name("safety")
                .with_owner(req.clone())
                .with_span(Span::new("a.sysml", 110, 130)),
        );
        let actor_membership = graph.add_element(
            Element::new_with_kind(ElementKind::ActorMembership)
                .with_name("driver")
                .with_owner(req.clone())
                .with_span(Span::new("a.sysml", 140, 160)),
        );
        if let Some(el) = graph.elements.get_mut(&req) {
            el.set_prop(
                "actors",
                Value::List(vec![Value::Ref(actor_membership.clone())]),
            );
        }
        let attr = graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("maxTripTime")
                .with_owner(req.clone())
                .with_prop(
                    "value",
                    Value::quantity(
                        40.0,
                        sysml_core::physics::dimension::DimensionVector::default(),
                        Some("ms".to_owned()),
                    ),
                )
                .with_span(Span::new("a.sysml", 170, 190)),
        );
        let rationale = graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(req.clone())
                .with_prop("unresolvedTypeName", "Rationale"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("text")
                .with_owner(rationale)
                .with_prop("value", "Threshold from the 2025 trade study."),
        );
        DetailFixture {
            graph,
            req,
            subject_membership,
            assume,
            require,
            constraint_def,
            concern,
            actor_membership,
            attr,
        }
    }

    #[test]
    fn requirement_detail_composes_the_full_contract() {
        let f = detail_fixture();
        let detail = requirement_detail(&f.graph, &f.req, &HashMap::new()).unwrap();

        assert_eq!(detail.id, f.req);
        let subject = detail.subject.expect("subject ref");
        assert_eq!(subject.id, f.subject_membership);
        assert_eq!(subject.name.as_deref(), Some("breaker"));

        assert_eq!(detail.assumed_constraints.len(), 1);
        assert_eq!(detail.assumed_constraints[0].id, f.assume);
        assert_eq!(
            detail.assumed_constraints[0].text.as_deref(),
            Some("ambientTemp <= 40 [degC]")
        );
        assert_eq!(detail.assumed_constraints[0].referenced_definition, None);

        assert_eq!(detail.required_constraints.len(), 1);
        assert_eq!(detail.required_constraints[0].id, f.require);
        assert_eq!(detail.required_constraints[0].text, None);
        let referenced = detail.required_constraints[0]
            .referenced_definition
            .as_ref()
            .expect("reference form resolves");
        assert_eq!(referenced.id, f.constraint_def);

        assert_eq!(detail.framed_concerns.len(), 1);
        assert_eq!(detail.framed_concerns[0].id, f.concern);
        assert_eq!(detail.actors.len(), 1);
        assert_eq!(detail.actors[0].id, f.actor_membership);
        assert_eq!(detail.actors[0].name.as_deref(), Some("driver"));
        assert_eq!(detail.stakeholders, Vec::new());

        assert_eq!(detail.referenced_attributes.len(), 1);
        assert_eq!(detail.referenced_attributes[0].id, f.attr);
        assert_eq!(
            detail.referenced_attributes[0].value.as_deref(),
            Some("40 [ms]")
        );
        assert_eq!(detail.referenced_attributes[0].live_value, None);

        assert_eq!(
            detail.rationale.as_deref(),
            Some("Threshold from the 2025 trade study.")
        );
    }

    #[test]
    fn requirement_detail_threads_live_values() {
        let f = detail_fixture();
        let live = HashMap::from([(f.attr.clone(), "37.2 [ms]".to_owned())]);
        let detail = requirement_detail(&f.graph, &f.req, &live).unwrap();
        assert_eq!(
            detail.referenced_attributes[0].live_value.as_deref(),
            Some("37.2 [ms]")
        );
        // The static declaration stays visible alongside the live value.
        assert_eq!(
            detail.referenced_attributes[0].value.as_deref(),
            Some("40 [ms]")
        );
    }

    /// An UNRESOLVED reference form resolves to nothing — never a guess.
    /// (Identity model: ambiguity is a resolution-time concern; the resolver
    /// leaves `referencedFeature` unset when it cannot pick a unique target, and
    /// the read path fails hard on that rather than name-guessing.)
    #[test]
    fn requirement_detail_unresolved_reference_yields_none() {
        let mut f = detail_fixture();
        // Strip the resolved target from the reference form's ReferenceSubsetting,
        // leaving only the parse-time name — modelling an unresolved reference.
        let rs_id = f
            .graph
            .elements
            .values()
            .find(|e| e.kind == ElementKind::ReferenceSubsetting)
            .map(|e| e.id.clone())
            .expect("fixture has a ReferenceSubsetting");
        if let Some(rs) = f.graph.elements.get_mut(&rs_id) {
            rs.props.remove("referencedFeature");
        }
        let detail = requirement_detail(&f.graph, &f.req, &HashMap::new()).unwrap();
        assert_eq!(detail.required_constraints[0].referenced_definition, None);
    }

    /// Inherited contract (steward ruling 2026-07-16): a requirement usage
    /// that owns no constraints but is typed by a requirement def surfaces
    /// the def's constraints in the inherited buckets, each labeled with
    /// its provenance — exactly the evaluator's single-hop rule.
    #[test]
    fn requirement_detail_surfaces_inherited_contract_with_provenance() {
        let mut f = detail_fixture();
        // A def owning one assume + one require, and a bare usage typed by it.
        let def = f.graph.add_element(
            Element::new_with_kind(ElementKind::RequirementDefinition).with_name("TripReqDef"),
        );
        f.graph.add_element(
            Element::new_with_kind(ElementKind::RequirementConstraintMembership)
                .with_name("warm")
                .with_owner(def.clone())
                .with_prop("role", "assume")
                .with_prop("constraint", "temp > 0")
                .with_span(Span::new("b.sysml", 10, 20)),
        );
        f.graph.add_element(
            Element::new_with_kind(ElementKind::RequirementConstraintMembership)
                .with_name("fast")
                .with_owner(def.clone())
                .with_prop("role", "require")
                .with_prop("constraint", "t < 40")
                .with_span(Span::new("b.sysml", 30, 40)),
        );
        let usage = f.graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage).with_name("fastTripCheck"),
        );
        f.graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(usage.clone())
                .with_prop("type", Value::Ref(def.clone())),
        );

        let detail = requirement_detail(&f.graph, &usage, &HashMap::new()).unwrap();
        assert!(detail.assumed_constraints.is_empty());
        assert!(detail.required_constraints.is_empty());
        assert_eq!(detail.inherited_assumed_constraints.len(), 1);
        assert_eq!(detail.inherited_required_constraints.len(), 1);
        let req_row = &detail.inherited_required_constraints[0];
        assert_eq!(req_row.text.as_deref(), Some("t < 40"));
        let from = req_row.inherited_from.as_ref().expect("provenance is binding");
        assert_eq!(from.id, def);
        assert_eq!(from.name.as_deref(), Some("TripReqDef"));
        assert_eq!(
            req_row.inherited_via.as_deref(),
            Some("typing"),
            "hop-kind provenance is binding (§2.1a)"
        );

        // §2.1a re-bless (supersedes the 2026-07-16 owns-none gate this
        // block used to pin): a requirement that OWNS constraints STILL
        // aggregates its chain's — the closure is unconditional. Owned
        // buckets stay owned-only; inherited rows appear alongside.
        f.graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(f.req.clone())
                .with_prop("type", Value::Ref(def)),
        );
        let owned = requirement_detail(&f.graph, &f.req, &HashMap::new()).unwrap();
        assert_eq!(owned.assumed_constraints.len(), 1);
        assert_eq!(owned.assumed_constraints[0].inherited_from, None);
        assert_eq!(
            owned.inherited_assumed_constraints.len(),
            1,
            "owning constraints must NOT suppress the inherited contract"
        );
        assert_eq!(owned.inherited_required_constraints.len(), 1);
        assert_eq!(
            owned.inherited_required_constraints[0].inherited_via.as_deref(),
            Some("typing")
        );
    }

    /// Role-based rows + instantiated_by (steward ruling 2026-07-16): a
    /// verification check occurrence is bookkeeping, not a peer row —
    /// excluded by default, revealed by the spec flag; a def's
    /// `instantiated_by` lists CONTENT usages only (the check stays
    /// visible through `verified_by`, one fact one taxonomy).
    #[test]
    fn rows_exclude_check_occurrences_and_detail_lists_content_instantiations() {
        let mut graph = ModelGraph::new();
        let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("P"));
        let def = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementDefinition)
                .with_name("TripReqDef")
                .with_owner(pkg.clone())
                .with_span(Span::new("a.sysml", 0, 10)),
        );
        // Content usage typed by the def.
        let content = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("tripReq")
                .with_owner(pkg.clone())
                .with_span(Span::new("a.sysml", 20, 30)),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(content.clone())
                .with_prop("type", Value::Ref(def.clone())),
        );
        // Check occurrence: usage typed by the def, owned by a
        // RequirementVerificationMembership (verify-clause form).
        let membership = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementVerificationMembership)
                .with_owner(pkg.clone()),
        );
        let check = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage)
                .with_name("tripCheck")
                .with_owner(membership)
                .with_span(Span::new("a.sysml", 40, 50)),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureTyping)
                .with_owner(check.clone())
                .with_prop("type", Value::Ref(def.clone())),
        );

        let names = |spec: &RequirementRowsSpec| -> Vec<String> {
            requirement_rows(&graph, spec, &HashMap::new(), "static", 1, QueryProfile::Service)
                .unwrap()
                .rows
                .into_iter()
                .filter_map(|r| r.name)
                .collect()
        };
        // Default: content only.
        let default_rows = names(&RequirementRowsSpec::default());
        assert!(default_rows.contains(&"TripReqDef".to_owned()));
        assert!(default_rows.contains(&"tripReq".to_owned()));
        assert!(
            !default_rows.contains(&"tripCheck".to_owned()),
            "check occurrence must not be a peer row by default: {default_rows:?}"
        );
        // Reveal flag brings it back.
        let revealed = names(&RequirementRowsSpec {
            include_verification_occurrences: Some(true),
            ..Default::default()
        });
        assert!(revealed.contains(&"tripCheck".to_owned()));

        // instantiated_by on the def: the content usage only.
        let detail = requirement_detail(&graph, &def, &HashMap::new()).unwrap();
        assert_eq!(
            detail
                .instantiated_by
                .iter()
                .map(|r| r.name.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("tripReq")],
            "content usages only — the check occurrence rides verified_by, not here"
        );
    }

    /// Unknown ids and non-requirement elements fail hard.
    #[test]
    fn requirement_detail_fails_hard_on_bad_targets() {
        let f = detail_fixture();
        let missing = ElementId::new_v4();
        assert!(matches!(
            requirement_detail(&f.graph, &missing, &HashMap::new()),
            Err(QueryError::Invalid(_))
        ));
        assert!(matches!(
            requirement_detail(&f.graph, &f.constraint_def, &HashMap::new()),
            Err(QueryError::Invalid(_))
        ));
    }
}
