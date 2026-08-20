//! Metadata query functions for extracting ToolExecution and ToolVariable annotations.
//!
//! SysML v2 metadata usages allow attaching tool-specific information to model elements.
//! These query functions extract structured data from `MetadataUsage` children typed as
//! `ToolExecution` or `ToolVariable`.
//!
//! ## Example SysML
//!
//! ```text
//! action def RunSimulation {
//!     metadata ToolExecution {
//!         attribute toolName = "OpenModelica";
//!         attribute uri = "omc://simulate";
//!     }
//!     in attribute pressure : Real {
//!         metadata ToolVariable {
//!             attribute name = "sim.pressure";
//!         }
//!     }
//! }
//! ```

use crate::{Element, ElementId, ElementKind, ModelGraph};

/// Information extracted from a `ToolExecution` metadata annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionInfo {
    /// The name of the external tool (e.g. "OpenModelica", "MATLAB").
    pub tool_name: String,
    /// An optional URI identifying the tool endpoint or command.
    pub uri: Option<String>,
}

/// A mapping between a SysML parameter and an external tool variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolVariableMapping {
    /// The SysML element name (the parameter's declared name).
    pub sysml_name: String,
    /// The external tool variable name (from the ToolVariable metadata).
    pub tool_name: String,
    /// The parameter direction.
    pub direction: ParamDirection,
    /// ODE derivative expression (dx/dt = f(x, t, params)), if provided.
    /// Used when `@ToolExecution { toolName = "builtin:ode-rk4" }` is present.
    pub derivative: Option<String>,
    /// Time-varying signal expression (e.g. `"48.0 * sin(314.159 * t)"`).
    /// Evaluated each ODE tick to update the parameter value from `t`.
    pub signal: Option<String>,
}

/// Direction of a parameter in a tool integration context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamDirection {
    /// Input parameter.
    In,
    /// Output parameter.
    Out,
    /// Bidirectional parameter.
    InOut,
}

impl ParamDirection {
    /// Parse a direction from a string value (as stored in element properties).
    #[allow(clippy::should_implement_trait)] // returns Option, not Result
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "in" => Some(ParamDirection::In),
            "out" => Some(ParamDirection::Out),
            "inout" => Some(ParamDirection::InOut),
            _ => None,
        }
    }

    /// Return the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            ParamDirection::In => "in",
            ParamDirection::Out => "out",
            ParamDirection::InOut => "inout",
        }
    }
}

impl std::fmt::Display for ParamDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Match a found type reference against a wanted bare type name.
///
/// Accepts both the bare name (`"Signal"`) and a qualified reference whose
/// last `::` segment matches (`"SimExtensions::Signal"`).
fn type_ref_matches(found: &str, wanted: &str) -> bool {
    found == wanted || found.ends_with(&format!("::{wanted}"))
}

/// Check whether a metadata element is typed as the given type name.
///
/// Bombproof against partially-resolved models — checks all five strategies:
/// 1. The element's `unresolvedTypeName` property (set by parser before resolution).
///    Matches both the bare name (`"ToolExecution"`) and qualified suffix
///    (`"Tooling::ToolExecution"` ends with `"::ToolExecution"`).
/// 2. The metadata element's own `name` (parsers often name the
///    `MetadataUsage` after its type, e.g. `metadata ToolExecution { ... }`).
/// 3. Outgoing `TypeOf` relationships whose target element has the matching name.
/// 4. `FeatureTyping` elements whose `typedFeature` points at this metadata
///    and whose `type` resolves to an element with the matching name.
/// 5. `FeatureTyping` children owned by this metadata element carrying an
///    `unresolved_type` prop (the tree-sitter parser's `@Type` lowering mints
///    exactly this shape: anonymous `MetadataUsage` + owned `FeatureTyping`
///    with `unresolved_type`). Bare and qualified (last-`::`-segment) match.
pub fn is_metadata_typed_as(graph: &ModelGraph, metadata: &Element, type_name: &str) -> bool {
    // Strategy 1: Check unresolvedTypeName property (parser sets this before resolution)
    if let Some(val) = metadata.get_prop("unresolvedTypeName") {
        if let Some(name) = val.as_str() {
            if type_ref_matches(name, type_name) {
                return true;
            }
        }
    }

    // Strategy 2: Check the metadata element's own name matches the type name.
    // In many parsed models, the MetadataUsage is named after its type
    // (e.g., `metadata ToolExecution { ... }` produces name = "ToolExecution").
    if let Some(ref name) = metadata.name {
        if name == type_name {
            return true;
        }
    }

    // Strategy 3: Walk outgoing FeatureTyping relationships from this metadata element
    // and check if the target type element has the expected name.
    for rel_id in graph
        .outgoing(&metadata.id)
        .filter(|r| r.kind == crate::RelationshipKind::TypeOf)
    {
        if let Some(target) = graph.get_element(&rel_id.target) {
            if target.name.as_deref() == Some(type_name) {
                return true;
            }
        }
    }

    // Strategy 4: Search FeatureTyping elements that reference this metadata element
    // as their typedFeature.
    for elem in graph.elements_by_kind(&ElementKind::FeatureTyping) {
        if let Some(typed_feature) = elem.get_prop("typedFeature") {
            if typed_feature.as_ref() == Some(&metadata.id) {
                // Check if the typing's type property points to an element named type_name
                if let Some(type_val) = elem.get_prop("type") {
                    if let Some(type_id) = type_val.as_ref() {
                        if let Some(type_elem) = graph.get_element(type_id) {
                            if type_elem.name.as_deref() == Some(type_name) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }

    // Strategy 5: FeatureTyping children owned by this metadata element with an
    // `unresolved_type` prop. This is the canonical tree-sitter parser shape
    // for `@Type` annotations: the MetadataUsage stays ANONYMOUS and the type
    // reference rides on an owned FeatureTyping child (dispatch.rs
    // "metadata_usage" arm). Consumers must match on the typing, not the name.
    for child in graph.children_of(&metadata.id) {
        if child.kind != ElementKind::FeatureTyping {
            continue;
        }
        if let Some(tn) = child.get_prop("unresolved_type").and_then(|v| v.as_str()) {
            if type_ref_matches(tn, type_name) {
                return true;
            }
        }
    }

    false
}

/// Check whether an element carries a `MetadataUsage` child typed as the given
/// type name (e.g. `@Signal`, `@PowerPort`).
///
/// The single home for "does this element have @Foo on it?" lookups — every
/// consumer (physics classification, runtime, diagnostics) should route
/// through here rather than open-coding the child walk. Matching is by type
/// name only (bare or qualified, last-`::`-segment) via
/// [`is_metadata_typed_as`]; no stdlib metadata def needs to exist.
pub fn has_metadata_typed(graph: &ModelGraph, element_id: &ElementId, type_name: &str) -> bool {
    graph
        .children_of(element_id)
        .filter(|c| c.kind == ElementKind::MetadataUsage)
        .any(|meta| is_metadata_typed_as(graph, meta, type_name))
}

/// Extract a string-valued attribute from the children of a metadata element.
///
/// Looks for a child with the given name and returns its value as a string.
/// Checks `AttributeUsage` and `ReferenceUsage` (metadata body members may parse
/// as `DefaultReferenceUsage`). Tries `default`, `value`, and `unresolved_value`
/// properties in that order. A resolved `Ref` value renders as the referenced
/// element's name (e.g. an enum literal like `StatusKind::tbd` resolves to `tbd`).
///
/// The single home for "read attribute X off this metadata annotation" — every
/// consumer (tool-execution extraction, status/maturity columns) routes through
/// here rather than open-coding the child walk.
pub fn metadata_string_attr(
    graph: &ModelGraph,
    parent_id: &ElementId,
    attr_name: &str,
) -> Option<String> {
    for child in graph.children_of(parent_id) {
        let is_candidate = matches!(
            child.kind,
            ElementKind::AttributeUsage | ElementKind::ReferenceUsage
        );
        if !is_candidate || child.name.as_deref() != Some(attr_name) {
            continue;
        }
        // Try "default", "value", then "unresolved_value"
        for prop_name in &["default", "value", "unresolved_value"] {
            if let Some(val) = child.get_prop(prop_name) {
                if let Some(s) = val.as_str() {
                    return Some(s.to_owned());
                }
                if let Some(target_id) = val.as_ref() {
                    if let Some(name) = graph.get_element(target_id).and_then(|e| e.name.clone()) {
                        return Some(name);
                    }
                }
            }
        }
        // A reference value (`status = StatusKind::tbd`) lowers as an owned
        // FeatureReferenceExpression child named after the referenced path,
        // not as a prop on the attribute.
        for grandchild in graph.children_of(&child.id) {
            if grandchild.kind == ElementKind::FeatureReferenceExpression {
                if let Some(name) = &grandchild.name {
                    return Some(name.clone());
                }
            }
        }
    }
    None
}

/// Read the `status` value of a `ModelingMetadata::StatusInfo` annotation on an
/// element, if present — e.g. `@StatusInfo { status = StatusKind::tbd; }` yields
/// `Some("tbd")`.
///
/// Matching is name-based via [`is_metadata_typed_as`] (display-column read, not
/// edge semantics — a user-shadowed `StatusInfo` can at worst mislabel a chip in
/// one tool view). A qualified enum reference is normalized to its last segment
/// so both a raw `StatusKind::tbd` string and a resolved literal render as `tbd`.
pub fn status_info_value(graph: &ModelGraph, element_id: &ElementId) -> Option<String> {
    for child in graph.children_of(element_id) {
        if child.kind != ElementKind::MetadataUsage {
            continue;
        }
        if !is_metadata_typed_as(graph, child, "StatusInfo") {
            continue;
        }
        if let Some(raw) = metadata_string_attr(graph, &child.id, "status") {
            let normalized = raw.rsplit("::").next().unwrap_or(&raw).trim().to_owned();
            if !normalized.is_empty() {
                return Some(normalized);
            }
        }
    }
    None
}

/// The normative closed set of `VerificationMethodKind` enum literals
/// (VerificationCases.sysml:90-101) — the ONE home for write-side
/// validation of layer-1 method vocabulary (e.g.
/// `sysml.workflow.attest_verification` rejects anything outside it).
/// The READ side ([`verification_methods`]) deliberately stays
/// non-filtering: dropping an unknown declared value would silently hide
/// model content. Write strict, read lenient — never two hand-typed lists.
pub const VERIFICATION_METHOD_KINDS: [&str; 4] = ["inspect", "analyze", "demo", "test"];

/// Read the declared verification methods off an element's
/// `@VerificationMethod` annotations — the normative
/// `VerificationCases::VerificationMethod` metadata def
/// (`attribute kind : VerificationMethodKind[1..*]`, annotating "a
/// verification case or action").
///
/// The single home for the declared-method read (B4 method column). All
/// three pilot authoring shapes are covered:
/// - `@VerificationMethod{ kind = analyze; }` — the `kind` member owns one
///   `FeatureReferenceExpression` named after the literal;
/// - `@VerificationMethod{ kind = (test, demo); }` — the `kind` member owns
///   an `OperatorExpression(",")` whose `FeatureReferenceExpression`
///   children carry `argIndex` (comma chains of 3+ values nest, so the walk
///   recurses);
/// - qualified references (`VerificationMethodKind::test`) normalize to the
///   last `::` segment, like [`status_info_value`].
///
/// OWNED annotations only — a usage typed by an annotated def does NOT
/// surface the def's methods (annotations attach to the annotated element;
/// the pilot's own example re-annotates the usage). Values are returned as
/// declared, in declaration order, deduplicated — NOT filtered to the
/// standard enum's four literals: this is a display read at the same trust
/// level as [`status_info_value`], and dropping an unknown value would
/// silently hide model content.
pub fn verification_methods(graph: &ModelGraph, element_id: &ElementId) -> Vec<String> {
    let mut methods: Vec<String> = Vec::new();
    // Multiple annotations on one element are legal; `children_of` is a
    // hash index, so order them by document position for determinism.
    let mut metas: Vec<&Element> = graph
        .children_of(element_id)
        .filter(|child| child.kind == ElementKind::MetadataUsage)
        .filter(|child| is_metadata_typed_as(graph, child, "VerificationMethod"))
        .collect();
    metas.sort_by(|a, b| {
        let key = |e: &Element| e.spans.first().map(|s| (s.file.clone(), s.start));
        key(a).cmp(&key(b)).then_with(|| a.id.cmp(&b.id))
    });
    for meta in metas {
        for kind_attr in graph.children_of(&meta.id) {
            let is_candidate = matches!(
                kind_attr.kind,
                ElementKind::AttributeUsage | ElementKind::ReferenceUsage
            );
            if !is_candidate || kind_attr.name.as_deref() != Some("kind") {
                continue;
            }
            // Prop-stored string value (alternate lowerings / hand-built
            // graphs) — same prop precedence as `metadata_string_attr`.
            let prop_value = ["default", "value", "unresolved_value"]
                .iter()
                .find_map(|prop| kind_attr.get_prop(prop).and_then(|v| v.as_str()));
            if let Some(raw) = prop_value {
                push_method(&mut methods, raw);
                continue;
            }
            collect_method_refs(graph, &kind_attr.id, &mut methods);
        }
    }
    methods
}

/// Normalize (last `::` segment, trimmed) and push, deduplicating while
/// preserving first-seen order.
fn push_method(methods: &mut Vec<String>, raw: &str) {
    let normalized = raw.rsplit("::").next().unwrap_or(raw).trim().to_owned();
    if !normalized.is_empty() && !methods.iter().any(|m| *m == normalized) {
        methods.push(normalized);
    }
}

/// Collect `FeatureReferenceExpression` names under an expression owner in
/// `argIndex` order, recursing through nested comma `OperatorExpression`
/// chains (`(a, b, c)` left-chains in the lowering).
fn collect_method_refs(graph: &ModelGraph, owner_id: &ElementId, methods: &mut Vec<String>) {
    let mut items: Vec<(i64, &Element)> = graph
        .children_of(owner_id)
        .filter(|child| {
            matches!(
                child.kind,
                ElementKind::FeatureReferenceExpression | ElementKind::OperatorExpression
            )
        })
        .map(|child| {
            let index = child
                .get_prop("argIndex")
                .and_then(crate::Value::as_int)
                .unwrap_or(i64::MAX);
            (index, child)
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    for (_, child) in items {
        match child.kind {
            ElementKind::FeatureReferenceExpression => {
                if let Some(name) = &child.name {
                    push_method(methods, name);
                }
            }
            ElementKind::OperatorExpression => collect_method_refs(graph, &child.id, methods),
            _ => {}
        }
    }
}

/// Extract `ToolExecution` metadata from an element's children.
///
/// Walks the element's children looking for `MetadataUsage` nodes typed as
/// `ToolExecution`. Returns the first match with extracted `toolName` and `uri`.
///
/// # Arguments
///
/// * `graph` - The model graph containing all elements
/// * `element` - The element to inspect for ToolExecution metadata
///
/// # Returns
///
/// `Some(ToolExecutionInfo)` if a ToolExecution metadata annotation was found,
/// `None` otherwise.
pub fn get_tool_execution(graph: &ModelGraph, element: &Element) -> Option<ToolExecutionInfo> {
    for child in graph.children_of(&element.id) {
        if child.kind != ElementKind::MetadataUsage {
            continue;
        }

        if !is_metadata_typed_as(graph, child, "ToolExecution") {
            continue;
        }

        // Extract toolName (required) and uri (optional)
        let tool_name = metadata_string_attr(graph, &child.id, "toolName")?;
        let uri = metadata_string_attr(graph, &child.id, "uri");

        return Some(ToolExecutionInfo { tool_name, uri });
    }

    None
}

/// Extract `ToolVariable` mappings from an element's parameter children.
///
/// For each child that is an `AttributeUsage` or `PortUsage`, checks if it has
/// a `MetadataUsage` child typed as `ToolVariable`. If so, extracts the variable
/// name and parameter direction to build a mapping.
///
/// # Arguments
///
/// * `graph` - The model graph containing all elements
/// * `element` - The element whose parameters to inspect for ToolVariable metadata
///
/// # Returns
///
/// A vector of `ToolVariableMapping` entries, one per parameter that has a
/// ToolVariable annotation.
pub fn get_tool_variables(graph: &ModelGraph, element: &Element) -> Vec<ToolVariableMapping> {
    let mut mappings = Vec::new();

    for param in graph.children_of(&element.id) {
        // Only consider parameter-like children
        if param.kind != ElementKind::AttributeUsage && param.kind != ElementKind::PortUsage {
            continue;
        }

        let sysml_name = match &param.name {
            Some(name) => name.clone(),
            None => continue, // Skip unnamed parameters
        };

        // Look for ToolVariable metadata on this parameter
        for meta_child in graph.children_of(&param.id) {
            if meta_child.kind != ElementKind::MetadataUsage {
                continue;
            }

            if !is_metadata_typed_as(graph, meta_child, "ToolVariable") {
                continue;
            }

            // Extract the tool variable name
            let Some(tool_name) = metadata_string_attr(graph, &meta_child.id, "name") else {
                continue;
            };

            // Extract derivative expression (ODE dx/dt), if present
            let derivative = metadata_string_attr(graph, &meta_child.id, "derivative");

            // Extract signal expression (time-varying input), if present
            let signal = metadata_string_attr(graph, &meta_child.id, "signal");

            // Determine direction from the parameter's "direction" property
            let direction = param
                .get_prop("direction")
                .and_then(|v| v.as_str())
                .and_then(ParamDirection::from_str)
                .unwrap_or(ParamDirection::In);

            mappings.push(ToolVariableMapping {
                sysml_name: sysml_name.clone(),
                tool_name,
                direction,
                derivative,
                signal,
            });

            // Only take the first ToolVariable per parameter
            break;
        }
    }

    mappings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Element;

    /// Helper: create a ModelGraph with a ToolExecution-annotated action.
    ///
    /// Structure:
    /// ```text
    /// ActionDefinition "RunSimulation"
    ///   └─ MetadataUsage "ToolExecution"  (unresolvedTypeName = "ToolExecution")
    ///       ├─ AttributeUsage "toolName"  (default = "OpenModelica")
    ///       └─ AttributeUsage "uri"       (default = "omc://simulate")
    /// ```
    fn graph_with_tool_execution() -> (ModelGraph, ElementId) {
        let mut graph = ModelGraph::new();

        let action =
            Element::new_with_kind(ElementKind::ActionDefinition).with_name("RunSimulation");
        let action_id = graph.add_element(action);

        let metadata = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_name("ToolExecution")
            .with_owner(action_id.clone())
            .with_prop("unresolvedTypeName", "ToolExecution");
        let meta_id = graph.add_element(metadata);

        let tool_name_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("toolName")
            .with_owner(meta_id.clone())
            .with_prop("default", "OpenModelica");
        graph.add_element(tool_name_attr);

        let uri_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("uri")
            .with_owner(meta_id)
            .with_prop("default", "omc://simulate");
        graph.add_element(uri_attr);

        (graph, action_id)
    }

    /// Helper: create a ModelGraph with ToolVariable-annotated parameters.
    ///
    /// Structure:
    /// ```text
    /// ActionDefinition "RunSimulation"
    ///   ├─ AttributeUsage "pressure"  (direction = "in")
    ///   │   └─ MetadataUsage "ToolVariable"  (unresolvedTypeName = "ToolVariable")
    ///   │       └─ AttributeUsage "name"  (default = "sim.pressure")
    ///   ├─ AttributeUsage "temperature"  (direction = "out")
    ///   │   └─ MetadataUsage "ToolVariable"
    ///   │       └─ AttributeUsage "name"  (default = "sim.temperature")
    ///   └─ PortUsage "dataPort"  (direction = "inout")
    ///       └─ MetadataUsage "ToolVariable"
    ///           └─ AttributeUsage "name"  (default = "sim.dataPort")
    /// ```
    fn graph_with_tool_variables() -> (ModelGraph, ElementId) {
        let mut graph = ModelGraph::new();

        let action =
            Element::new_with_kind(ElementKind::ActionDefinition).with_name("RunSimulation");
        let action_id = graph.add_element(action);

        // Parameter 1: pressure (in)
        let pressure = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("pressure")
            .with_owner(action_id.clone())
            .with_prop("direction", "in");
        let pressure_id = graph.add_element(pressure);

        let pressure_meta = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_name("ToolVariable")
            .with_owner(pressure_id.clone())
            .with_prop("unresolvedTypeName", "ToolVariable");
        let pressure_meta_id = graph.add_element(pressure_meta);

        let pressure_name = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("name")
            .with_owner(pressure_meta_id)
            .with_prop("default", "sim.pressure");
        graph.add_element(pressure_name);

        // Parameter 2: temperature (out)
        let temperature = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("temperature")
            .with_owner(action_id.clone())
            .with_prop("direction", "out");
        let temp_id = graph.add_element(temperature);

        let temp_meta = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_name("ToolVariable")
            .with_owner(temp_id.clone())
            .with_prop("unresolvedTypeName", "ToolVariable");
        let temp_meta_id = graph.add_element(temp_meta);

        let temp_name = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("name")
            .with_owner(temp_meta_id)
            .with_prop("default", "sim.temperature");
        graph.add_element(temp_name);

        // Parameter 3: dataPort (inout, PortUsage)
        let data_port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("dataPort")
            .with_owner(action_id.clone())
            .with_prop("direction", "inout");
        let port_id = graph.add_element(data_port);

        let port_meta = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_name("ToolVariable")
            .with_owner(port_id.clone())
            .with_prop("unresolvedTypeName", "ToolVariable");
        let port_meta_id = graph.add_element(port_meta);

        let port_name = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("name")
            .with_owner(port_meta_id)
            .with_prop("default", "sim.dataPort");
        graph.add_element(port_name);

        (graph, action_id)
    }

    #[test]
    fn test_get_tool_execution_found() {
        let (graph, action_id) = graph_with_tool_execution();
        let action = graph.get_element(&action_id).unwrap();

        let info = get_tool_execution(&graph, action);
        assert!(info.is_some(), "should find ToolExecution metadata");

        let info = info.unwrap();
        assert_eq!(info.tool_name, "OpenModelica");
        assert_eq!(info.uri, Some("omc://simulate".to_string()));
    }

    #[test]
    fn test_get_tool_execution_no_uri() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("Simple");
        let action_id = graph.add_element(action);

        let metadata = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_name("ToolExecution")
            .with_owner(action_id.clone())
            .with_prop("unresolvedTypeName", "ToolExecution");
        let meta_id = graph.add_element(metadata);

        let tool_name_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("toolName")
            .with_owner(meta_id)
            .with_prop("default", "MATLAB");
        graph.add_element(tool_name_attr);

        let action = graph.get_element(&action_id).unwrap();
        let info = get_tool_execution(&graph, action).unwrap();
        assert_eq!(info.tool_name, "MATLAB");
        assert_eq!(info.uri, None);
    }

    #[test]
    fn test_get_tool_execution_not_found() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("NoMeta");
        let action_id = graph.add_element(action);

        let action = graph.get_element(&action_id).unwrap();
        assert!(get_tool_execution(&graph, action).is_none());
    }

    #[test]
    fn test_get_tool_execution_wrong_metadata_type() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("WrongMeta");
        let action_id = graph.add_element(action);

        // Metadata typed as something else
        let metadata = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_name("SomethingElse")
            .with_owner(action_id.clone())
            .with_prop("unresolvedTypeName", "SomethingElse");
        graph.add_element(metadata);

        let action = graph.get_element(&action_id).unwrap();
        assert!(get_tool_execution(&graph, action).is_none());
    }

    #[test]
    fn test_get_tool_variables_found() {
        let (graph, action_id) = graph_with_tool_variables();
        let action = graph.get_element(&action_id).unwrap();

        let vars = get_tool_variables(&graph, action);
        assert_eq!(vars.len(), 3);

        // Find each mapping (order may vary due to BTreeMap iteration)
        let pressure = vars.iter().find(|v| v.sysml_name == "pressure").unwrap();
        assert_eq!(pressure.tool_name, "sim.pressure");
        assert_eq!(pressure.direction, ParamDirection::In);

        let temperature = vars.iter().find(|v| v.sysml_name == "temperature").unwrap();
        assert_eq!(temperature.tool_name, "sim.temperature");
        assert_eq!(temperature.direction, ParamDirection::Out);

        let data_port = vars.iter().find(|v| v.sysml_name == "dataPort").unwrap();
        assert_eq!(data_port.tool_name, "sim.dataPort");
        assert_eq!(data_port.direction, ParamDirection::InOut);
    }

    #[test]
    fn test_get_tool_variables_empty() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("NoParams");
        let action_id = graph.add_element(action);

        let action = graph.get_element(&action_id).unwrap();
        let vars = get_tool_variables(&graph, action);
        assert!(vars.is_empty());
    }

    #[test]
    fn test_get_tool_variables_param_without_metadata() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("PlainParams");
        let action_id = graph.add_element(action);

        // Parameter without any ToolVariable metadata
        let param = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("speed")
            .with_owner(action_id.clone())
            .with_prop("direction", "in");
        graph.add_element(param);

        let action = graph.get_element(&action_id).unwrap();
        let vars = get_tool_variables(&graph, action);
        assert!(vars.is_empty());
    }

    #[test]
    fn test_get_tool_variables_default_direction() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("DefaultDir");
        let action_id = graph.add_element(action);

        // Parameter without explicit direction
        let param = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("value")
            .with_owner(action_id.clone());
        let param_id = graph.add_element(param);

        let meta = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_name("ToolVariable")
            .with_owner(param_id.clone())
            .with_prop("unresolvedTypeName", "ToolVariable");
        let meta_id = graph.add_element(meta);

        let name_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("name")
            .with_owner(meta_id)
            .with_prop("default", "tool.value");
        graph.add_element(name_attr);

        let action = graph.get_element(&action_id).unwrap();
        let vars = get_tool_variables(&graph, action);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].direction, ParamDirection::In); // defaults to In
    }

    /// Helper: attach an ANONYMOUS MetadataUsage + FeatureTyping child to an
    /// element, mirroring the tree-sitter parser's `@Type` lowering shape
    /// (dispatch.rs "metadata_usage" arm): the MetadataUsage has NO name and
    /// NO unresolvedTypeName here so the test exercises the FeatureTyping
    /// strategy specifically.
    fn attach_anonymous_metadata_typing(
        graph: &mut ModelGraph,
        owner: &ElementId,
        type_ref: &str,
    ) -> ElementId {
        let meta =
            Element::new_with_kind(ElementKind::MetadataUsage).with_owner(owner.clone());
        let meta_id = graph.add_element(meta);

        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_owner(meta_id.clone())
            .with_prop("typedFeature", crate::Value::Ref(meta_id.clone()))
            .with_prop("unresolved_type", type_ref);
        graph.add_element(typing);

        meta_id
    }

    #[test]
    fn test_has_metadata_typed_anonymous_feature_typing_shape() {
        let mut graph = ModelGraph::new();
        let port = Element::new_with_kind(ElementKind::PortDefinition).with_name("SensePort");
        let port_id = graph.add_element(port);

        attach_anonymous_metadata_typing(&mut graph, &port_id, "Signal");

        assert!(has_metadata_typed(&graph, &port_id, "Signal"));
        assert!(!has_metadata_typed(&graph, &port_id, "PowerPort"));
    }

    #[test]
    fn test_has_metadata_typed_qualified_last_segment_match() {
        let mut graph = ModelGraph::new();
        let port = Element::new_with_kind(ElementKind::PortDefinition).with_name("SensePort");
        let port_id = graph.add_element(port);

        attach_anonymous_metadata_typing(&mut graph, &port_id, "SimExtensions::SignalPort");

        assert!(has_metadata_typed(&graph, &port_id, "SignalPort"));
        // Last-segment match only — "Port" alone must not match.
        assert!(!has_metadata_typed(&graph, &port_id, "Port"));
    }

    #[test]
    fn test_has_metadata_typed_no_metadata() {
        let mut graph = ModelGraph::new();
        let port = Element::new_with_kind(ElementKind::PortDefinition).with_name("PlainPort");
        let port_id = graph.add_element(port);

        assert!(!has_metadata_typed(&graph, &port_id, "Signal"));
    }

    #[test]
    fn test_has_metadata_typed_unresolved_type_name_prop() {
        // The parser ALSO sets `unresolvedTypeName` on the MetadataUsage
        // itself; strategy 1 must keep matching that shape.
        let mut graph = ModelGraph::new();
        let port = Element::new_with_kind(ElementKind::PortDefinition).with_name("SensePort");
        let port_id = graph.add_element(port);

        let meta = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_owner(port_id.clone())
            .with_prop("unresolvedTypeName", "Ext::Signal");
        graph.add_element(meta);

        assert!(has_metadata_typed(&graph, &port_id, "Signal"));
    }

    #[test]
    fn test_param_direction_from_str() {
        assert_eq!(ParamDirection::from_str("in"), Some(ParamDirection::In));
        assert_eq!(ParamDirection::from_str("out"), Some(ParamDirection::Out));
        assert_eq!(
            ParamDirection::from_str("inout"),
            Some(ParamDirection::InOut)
        );
        assert_eq!(ParamDirection::from_str("invalid"), None);
    }

    #[test]
    fn test_param_direction_display() {
        assert_eq!(ParamDirection::In.to_string(), "in");
        assert_eq!(ParamDirection::Out.to_string(), "out");
        assert_eq!(ParamDirection::InOut.to_string(), "inout");
    }

    #[test]
    fn test_combined_tool_execution_and_variables() {
        let mut graph = ModelGraph::new();

        // Action with both ToolExecution and ToolVariable annotations
        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("FullAction");
        let action_id = graph.add_element(action);

        // ToolExecution metadata
        let exec_meta = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_name("ToolExecution")
            .with_owner(action_id.clone())
            .with_prop("unresolvedTypeName", "ToolExecution");
        let exec_meta_id = graph.add_element(exec_meta);

        let tool_name_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("toolName")
            .with_owner(exec_meta_id.clone())
            .with_prop("default", "Simulink");
        graph.add_element(tool_name_attr);

        let uri_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("uri")
            .with_owner(exec_meta_id)
            .with_prop("default", "matlab://simulink/run");
        graph.add_element(uri_attr);

        // Parameter with ToolVariable
        let param = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("torque")
            .with_owner(action_id.clone())
            .with_prop("direction", "out");
        let param_id = graph.add_element(param);

        let var_meta = Element::new_with_kind(ElementKind::MetadataUsage)
            .with_name("ToolVariable")
            .with_owner(param_id.clone())
            .with_prop("unresolvedTypeName", "ToolVariable");
        let var_meta_id = graph.add_element(var_meta);

        let name_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("name")
            .with_owner(var_meta_id)
            .with_prop("default", "engine.torque");
        graph.add_element(name_attr);

        // Verify both work together
        let action = graph.get_element(&action_id).unwrap();

        let exec_info = get_tool_execution(&graph, action).unwrap();
        assert_eq!(exec_info.tool_name, "Simulink");
        assert_eq!(exec_info.uri, Some("matlab://simulink/run".to_string()));

        let vars = get_tool_variables(&graph, action);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].sysml_name, "torque");
        assert_eq!(vars[0].tool_name, "engine.torque");
        assert_eq!(vars[0].direction, ParamDirection::Out);
    }

    #[test]
    fn status_info_value_reads_and_normalizes_status() {
        let mut graph = ModelGraph::new();
        let req_id = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementDefinition).with_name("SysReq"),
        );
        let meta_id = graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(req_id.clone())
                .with_prop("unresolvedTypeName", "StatusInfo"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("status")
                .with_owner(meta_id)
                .with_prop("value", "StatusKind::tbd"),
        );
        assert_eq!(status_info_value(&graph, &req_id), Some("tbd".to_owned()));
    }

    #[test]
    fn status_info_value_resolves_ref_values_and_absence() {
        let mut graph = ModelGraph::new();
        // Enum literal element the status value resolves to.
        let literal_id = graph.add_element(
            Element::new_with_kind(ElementKind::EnumerationUsage).with_name("done"),
        );
        let req_id = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementDefinition).with_name("SysReq"),
        );
        let meta_id = graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(req_id.clone())
                .with_prop("unresolvedTypeName", "ModelingMetadata::StatusInfo"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("status")
                .with_owner(meta_id)
                .with_prop("value", crate::Value::Ref(literal_id)),
        );
        assert_eq!(status_info_value(&graph, &req_id), Some("done".to_owned()));

        // No StatusInfo annotation → None, never a default.
        let bare_id = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage).with_name("Bare"),
        );
        assert_eq!(status_info_value(&graph, &bare_id), None);
    }

    /// Mint the parser's exact `@VerificationMethod{ kind = …; }` shape:
    /// anonymous MetadataUsage (unresolvedTypeName) owning a ReferenceUsage
    /// named `kind`. Returns the `kind` member's id for value attachment.
    fn attach_verification_method(
        graph: &mut ModelGraph,
        owner: &ElementId,
        span_start: usize,
    ) -> ElementId {
        let meta_id = graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(owner.clone())
                .with_prop("unresolvedTypeName", "VerificationMethod")
                .with_span(crate::Span::new("t.sysml", span_start, span_start + 10)),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::ReferenceUsage)
                .with_name("kind")
                .with_owner(meta_id),
        )
    }

    #[test]
    fn verification_methods_single_reference_form() {
        // @VerificationMethod{ kind = analyze; } — one owned FRE.
        let mut graph = ModelGraph::new();
        let case_id = graph.add_element(
            Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("VC"),
        );
        let kind_id = attach_verification_method(&mut graph, &case_id, 0);
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                .with_name("analyze")
                .with_owner(kind_id),
        );
        assert_eq!(verification_methods(&graph, &case_id), vec!["analyze"]);
    }

    #[test]
    fn verification_methods_tuple_ordered_by_arg_index() {
        // @VerificationMethod{ kind = (test, demo); } — comma
        // OperatorExpression; children_of is hash-ordered, argIndex is the
        // declared order. Qualified names normalize to the last segment.
        let mut graph = ModelGraph::new();
        let case_id = graph.add_element(
            Element::new_with_kind(ElementKind::VerificationCaseUsage).with_name("vc"),
        );
        let kind_id = attach_verification_method(&mut graph, &case_id, 0);
        let op_id = graph.add_element(
            Element::new_with_kind(ElementKind::OperatorExpression)
                .with_owner(kind_id)
                .with_prop("operator", ","),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                .with_name("VerificationMethodKind::demo")
                .with_owner(op_id.clone())
                .with_prop("argIndex", crate::Value::Int(1)),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                .with_name("VerificationMethodKind::test")
                .with_owner(op_id)
                .with_prop("argIndex", crate::Value::Int(0)),
        );
        assert_eq!(verification_methods(&graph, &case_id), vec!["test", "demo"]);
    }

    #[test]
    fn verification_methods_nested_comma_chain_and_dedup() {
        // (inspect, test, test) as a left-chained comma tree:
        // op_outer(op_inner(inspect, test), test) — recursion flattens in
        // declaration order; the duplicate collapses.
        let mut graph = ModelGraph::new();
        let case_id = graph.add_element(
            Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("VC"),
        );
        let kind_id = attach_verification_method(&mut graph, &case_id, 0);
        let op_outer = graph.add_element(
            Element::new_with_kind(ElementKind::OperatorExpression)
                .with_owner(kind_id)
                .with_prop("operator", ","),
        );
        let op_inner = graph.add_element(
            Element::new_with_kind(ElementKind::OperatorExpression)
                .with_owner(op_outer.clone())
                .with_prop("operator", ",")
                .with_prop("argIndex", crate::Value::Int(0)),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                .with_name("test")
                .with_owner(op_outer)
                .with_prop("argIndex", crate::Value::Int(1)),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                .with_name("inspect")
                .with_owner(op_inner.clone())
                .with_prop("argIndex", crate::Value::Int(0)),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                .with_name("test")
                .with_owner(op_inner)
                .with_prop("argIndex", crate::Value::Int(1)),
        );
        assert_eq!(
            verification_methods(&graph, &case_id),
            vec!["inspect", "test"]
        );
    }

    #[test]
    fn verification_methods_prop_value_and_absence() {
        // Prop-stored string value (hand-built graphs) + no-annotation case.
        let mut graph = ModelGraph::new();
        let case_id = graph.add_element(
            Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("VC"),
        );
        let meta_id = graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(case_id.clone())
                .with_prop("unresolvedTypeName", "VerificationMethod"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("kind")
                .with_owner(meta_id)
                .with_prop("value", "VerificationMethodKind::demo"),
        );
        assert_eq!(verification_methods(&graph, &case_id), vec!["demo"]);

        let bare_id = graph.add_element(
            Element::new_with_kind(ElementKind::VerificationCaseUsage).with_name("bare"),
        );
        assert!(verification_methods(&graph, &bare_id).is_empty());

        // A different annotation type must not read as a method.
        let other_id = graph.add_element(
            Element::new_with_kind(ElementKind::VerificationCaseUsage).with_name("other"),
        );
        let other_meta = graph.add_element(
            Element::new_with_kind(ElementKind::MetadataUsage)
                .with_owner(other_id.clone())
                .with_prop("unresolvedTypeName", "StatusInfo"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("kind")
                .with_owner(other_meta)
                .with_prop("value", "test"),
        );
        assert!(verification_methods(&graph, &other_id).is_empty());
    }

    #[test]
    fn verification_methods_multiple_annotations_document_order() {
        // Two annotations on one element — document order, not hash order.
        let mut graph = ModelGraph::new();
        let case_id = graph.add_element(
            Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("VC"),
        );
        let kind_late = attach_verification_method(&mut graph, &case_id, 100);
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                .with_name("demo")
                .with_owner(kind_late),
        );
        let kind_early = attach_verification_method(&mut graph, &case_id, 10);
        graph.add_element(
            Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                .with_name("test")
                .with_owner(kind_early),
        );
        assert_eq!(verification_methods(&graph, &case_id), vec!["test", "demo"]);
    }
}
