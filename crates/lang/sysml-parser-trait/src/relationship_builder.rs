//! Relationship element creation helpers.
//!
//! This module provides standalone functions for creating relationship elements
//! (Specialization, FeatureTyping, Subsetting, Redefinition, ReferenceSubsetting).
//! These functions are parser-agnostic and can be used by any parser backend.
//!
//! ## Ownership Model
//!
//! In SysML v2, relationships are owned by their source element. For example:
//! - A FeatureTyping is owned by the typed feature
//! - A Specialization is owned by the specific type
//! - A Subsetting is owned by the subsetting feature
//!
//! The functions in this module create the relationship elements and add them
//! to the graph with proper ownership through `add_owned_element()`.

use sysml_core::{
    CanonicalKey, Element, ElementId, ElementKind, ModelGraph, Span, Value, VisibilityKind,
};

/// Mint a relationship-shaped Element under a stable canonical key (ADR-009).
///
/// Relationship elements (Specialization, FeatureTyping, Subsetting, …) are
/// always anonymous — they have no source-level name. Their canonical key is
/// derived from `(parent_key, "{ElementKind}:{role}", sibling_index)`, where
/// `role` is the relationship's role at the parent (e.g. `"general"`,
/// `"target"`, `"membership"`). Concatenating role into the kind segment
/// keeps two relationships of the same kind but different roles distinct
/// without growing `CanonicalKey`'s API surface.
/// Mint a relationship-shaped Element under a stable canonical key
/// (ADR-009) and return both the element and its canonical key.
///
/// The key is needed by `*_with_key` callers so the wrapping
/// `OwningMembership` can also be minted via
/// [`ModelGraph::add_owned_element_with_key`] for full reparse stability
/// (ADR-009 §Relationships).
fn mint_relationship_element_keyed(
    kind: ElementKind,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> (Element, CanonicalKey) {
    let segment = format!("{}:{}", kind.as_str(), role);
    let key = CanonicalKey::for_anonymous(parent_key, &segment, sibling_index);
    let element = Element::new_with_key(kind, &key);
    (element, key)
}

/// Create a Specialization element linking a specific type to its general type.
///
/// The relationship is owned by the specific type.
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add the element to
/// * `specific_id` - The ID of the specific type (the subtype)
/// * `general_qname` - The qualified name of the general type (stored unresolved)
/// * `span` - Optional span for source location tracking
///
/// # Returns
///
/// The ElementId of the created Specialization element.
pub fn create_specialization(
    graph: &mut ModelGraph,
    specific_id: ElementId,
    general_qname: String,
    span: Option<Span>,
) -> ElementId {
    let mut element = Element::new_with_kind(ElementKind::Specialization);
    element.set_prop("specific", Value::Ref(specific_id.clone()));
    element.set_prop("unresolved_general", Value::String(general_qname));

    if let Some(s) = span {
        element.spans.push(s);
    }

    // Owned by the specific type
    graph.add_owned_element(element, specific_id, VisibilityKind::Public)
}

/// Create a Subclassification element linking a subclassifier to its superclassifier.
///
/// Per KerML spec, `Subclassification` is a `Specialization` where both the specific
/// and general Types are Classifiers. This is created when a definition uses `:>` or
/// `specializes` syntax (e.g., `part def Car :> Vehicle`).
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add the element to
/// * `subclassifier_id` - The ID of the more specific classifier
/// * `superclassifier_qname` - The qualified name of the more general classifier
/// * `span` - Optional span for source location tracking
///
/// # Returns
///
/// The ElementId of the created Subclassification element.
pub fn create_subclassification(
    graph: &mut ModelGraph,
    subclassifier_id: ElementId,
    superclassifier_qname: String,
    span: Option<Span>,
) -> ElementId {
    let mut element = Element::new_with_kind(ElementKind::Subclassification);
    element.set_prop("subclassifier", Value::Ref(subclassifier_id.clone()));
    element.set_prop(
        "unresolved_superclassifier",
        Value::String(superclassifier_qname),
    );
    // Also set the generic specific/general props for compatibility with Specialization handlers
    element.set_prop("specific", Value::Ref(subclassifier_id.clone()));

    if let Some(s) = span {
        element.spans.push(s);
    }

    // Owned by the subclassifier
    graph.add_owned_element(element, subclassifier_id, VisibilityKind::Public)
}

/// Create a FeatureTyping element linking a typed feature to its type.
///
/// The relationship is owned by the typed feature.
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add the element to
/// * `typed_feature_id` - The ID of the feature being typed
/// * `type_qname` - The qualified name of the type (stored unresolved)
/// * `span` - Optional span for source location tracking
///
/// # Returns
///
/// The ElementId of the created FeatureTyping element.
pub fn create_feature_typing(
    graph: &mut ModelGraph,
    typed_feature_id: ElementId,
    type_qname: String,
    span: Option<Span>,
) -> ElementId {
    let mut element = Element::new_with_kind(ElementKind::FeatureTyping);
    element.set_prop("typedFeature", Value::Ref(typed_feature_id.clone()));
    element.set_prop("unresolved_type", Value::String(type_qname));

    if let Some(s) = span {
        element.spans.push(s);
    }

    // Owned by the typed feature
    graph.add_owned_element(element, typed_feature_id, VisibilityKind::Public)
}

/// Create a ConjugatedPortTyping element linking a feature to a conjugated port definition.
///
/// Per SysML spec, `ConjugatedPortTyping` is a `FeatureTyping` whose type is a
/// `ConjugatedPortDefinition`. This is created when the surface syntax uses `~TypeName`
/// (e.g., `port p : ~WaterPort`).
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add the element to
/// * `typed_feature_id` - The ID of the feature being typed
/// * `port_def_qname` - The qualified name of the original port definition (without `~` prefix)
/// * `span` - Optional span for source location tracking
///
/// # Returns
///
/// The ElementId of the created ConjugatedPortTyping element.
pub fn create_conjugated_port_typing(
    graph: &mut ModelGraph,
    typed_feature_id: ElementId,
    port_def_qname: String,
    span: Option<Span>,
) -> ElementId {
    let mut element = Element::new_with_kind(ElementKind::ConjugatedPortTyping);
    element.set_prop("typedFeature", Value::Ref(typed_feature_id.clone()));
    // Store the original port def name for resolution to find its ConjugatedPortDefinition
    element.set_prop("unresolved_type", Value::String(port_def_qname));
    element.set_prop("isConjugated", Value::Bool(true));

    if let Some(s) = span {
        element.spans.push(s);
    }

    // Owned by the typed feature
    graph.add_owned_element(element, typed_feature_id, VisibilityKind::Public)
}

/// Create a ConjugatedPortDefinition element for a PortDefinition.
///
/// Per SysML spec, every PortDefinition implicitly owns a ConjugatedPortDefinition.
/// This enables `~PortName` typing syntax to resolve to the conjugated form.
/// The ConjugatedPortDefinition is named `~{PortDefName}` and is owned by the
/// PortDefinition's parent namespace.
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add the element to
/// * `port_def_id` - The ID of the original PortDefinition
/// * `port_def_name` - The name of the PortDefinition (used to derive `~Name`)
/// * `parent_id` - The parent namespace that owns both the PortDef and its conjugate
/// * `span` - Optional span for source location tracking
///
/// # Returns
///
/// The ElementId of the created ConjugatedPortDefinition element.
pub fn create_conjugated_port_definition(
    graph: &mut ModelGraph,
    port_def_id: ElementId,
    port_def_name: &str,
    parent_id: Option<ElementId>,
    span: Option<Span>,
) -> ElementId {
    let mut element = Element::new_with_kind(ElementKind::ConjugatedPortDefinition);
    element.name = Some(format!("~{}", port_def_name));
    element.set_prop("originalPortDefinition", Value::Ref(port_def_id));

    if let Some(s) = span {
        element.spans.push(s);
    }

    // Owned by the same parent namespace as the PortDefinition
    match parent_id {
        Some(pid) => graph.add_owned_element(element, pid, VisibilityKind::Public),
        None => graph.add_element(element),
    }
}

/// Create a CrossSubsetting element linking a crossing feature to a crossed feature.
///
/// Per KerML spec, `CrossSubsetting` is a `Subsetting` where the subsetting feature
/// crosses (intersects with) another feature. This is created when `crosses` syntax
/// is used (e.g., `part lane : Lane crosses road : Road`).
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add the element to
/// * `crossing_feature_id` - The ID of the feature that crosses
/// * `crossed_qname` - The qualified name of the crossed feature (stored unresolved)
/// * `span` - Optional span for source location tracking
///
/// # Returns
///
/// The ElementId of the created CrossSubsetting element.
pub fn create_cross_subsetting(
    graph: &mut ModelGraph,
    crossing_feature_id: ElementId,
    crossed_qname: String,
    span: Option<Span>,
) -> ElementId {
    let mut element = Element::new_with_kind(ElementKind::CrossSubsetting);
    element.set_prop("subsettingFeature", Value::Ref(crossing_feature_id.clone()));
    element.set_prop(
        "unresolved_crossedFeature",
        Value::String(crossed_qname.clone()),
    );
    // Also set as unresolved_subsettedFeature for resolution compatibility
    // (CrossSubsetting is a Subsetting; crossedFeature redefines subsettedFeature)
    element.set_prop("unresolved_subsettedFeature", Value::String(crossed_qname));

    if let Some(s) = span {
        element.spans.push(s);
    }

    // Owned by the crossing feature
    graph.add_owned_element(element, crossing_feature_id, VisibilityKind::Public)
}

/// Create an Annotation element linking an annotating element to an annotated element.
///
/// Per KerML spec, `Annotation` is a `Relationship` that associates an `AnnotatingElement`
/// (like Comment, Documentation, or MetadataUsage) with the `Element` it annotates.
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add the element to
/// * `annotating_element_id` - The ID of the annotating element (source)
/// * `annotated_qname` - The qualified name of the annotated element (stored unresolved)
/// * `span` - Optional span for source location tracking
///
/// # Returns
///
/// The ElementId of the created Annotation element.
pub fn create_annotation(
    graph: &mut ModelGraph,
    annotating_element_id: ElementId,
    annotated_qname: String,
    span: Option<Span>,
) -> ElementId {
    let mut element = Element::new_with_kind(ElementKind::Annotation);
    element.set_prop(
        "annotatingElement",
        Value::Ref(annotating_element_id.clone()),
    );
    element.set_prop(
        "unresolved_annotatedElement",
        Value::String(annotated_qname),
    );

    if let Some(s) = span {
        element.spans.push(s);
    }

    // Owned by the annotating element
    graph.add_owned_element(element, annotating_element_id, VisibilityKind::Public)
}

/// Create a Subsetting element linking a subsetting feature to its subsetted feature.
///
/// The relationship is owned by the subsetting feature.
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add the element to
/// * `subsetting_feature_id` - The ID of the feature that subsets
/// * `subsetted_qname` - The qualified name of the subsetted feature (stored unresolved)
/// * `span` - Optional span for source location tracking
///
/// # Returns
///
/// The ElementId of the created Subsetting element.
pub fn create_subsetting(
    graph: &mut ModelGraph,
    subsetting_feature_id: ElementId,
    subsetted_qname: String,
    span: Option<Span>,
) -> ElementId {
    let mut element = Element::new_with_kind(ElementKind::Subsetting);
    element.set_prop(
        "subsettingFeature",
        Value::Ref(subsetting_feature_id.clone()),
    );
    element.set_prop(
        "unresolved_subsettedFeature",
        Value::String(subsetted_qname),
    );

    if let Some(s) = span {
        element.spans.push(s);
    }

    // Owned by the subsetting feature
    graph.add_owned_element(element, subsetting_feature_id, VisibilityKind::Public)
}

/// Create a Redefinition element linking a redefining feature to its redefined feature.
///
/// The relationship is owned by the redefining feature.
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add the element to
/// * `redefining_feature_id` - The ID of the feature that redefines
/// * `redefined_qname` - The qualified name of the redefined feature (stored unresolved)
/// * `span` - Optional span for source location tracking
///
/// # Returns
///
/// The ElementId of the created Redefinition element.
pub fn create_redefinition(
    graph: &mut ModelGraph,
    redefining_feature_id: ElementId,
    redefined_qname: String,
    span: Option<Span>,
) -> ElementId {
    let mut element = Element::new_with_kind(ElementKind::Redefinition);
    element.set_prop(
        "redefiningFeature",
        Value::Ref(redefining_feature_id.clone()),
    );
    element.set_prop(
        "unresolved_redefinedFeature",
        Value::String(redefined_qname),
    );

    if let Some(s) = span {
        element.spans.push(s);
    }

    // Owned by the redefining feature
    graph.add_owned_element(element, redefining_feature_id, VisibilityKind::Public)
}

/// Create a ReferenceSubsetting element linking a referencing feature to its referenced feature.
///
/// The relationship is owned by the referencing feature.
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add the element to
/// * `referencing_feature_id` - The ID of the feature that references
/// * `referenced_qname` - The qualified name of the referenced feature (stored unresolved)
/// * `span` - Optional span for source location tracking
///
/// # Returns
///
/// The ElementId of the created ReferenceSubsetting element.
pub fn create_reference_subsetting(
    graph: &mut ModelGraph,
    referencing_feature_id: ElementId,
    referenced_qname: String,
    span: Option<Span>,
) -> ElementId {
    let mut element = Element::new_with_kind(ElementKind::ReferenceSubsetting);
    element.set_prop(
        "referencingFeature",
        Value::Ref(referencing_feature_id.clone()),
    );
    element.set_prop(
        "unresolved_referencedFeature",
        Value::String(referenced_qname),
    );

    if let Some(s) = span {
        element.spans.push(s);
    }

    // Owned by the referencing feature
    graph.add_owned_element(element, referencing_feature_id, VisibilityKind::Public)
}

// =====================================================================
// Canonical-key (`_with_key`) variants — ADR-009 / S1
//
// Each helper above has a `*_with_key` twin that takes a parent canonical
// key, a role string, and a sibling index. The minted relationship element
// gets a reparse-stable id derived from `(parent_key, "{kind}:{role}",
// sibling_index)`. The non-`_with_key` helpers keep their fresh-UUID
// behaviour so unmigrated parser sites remain unaffected.
// =====================================================================

/// Canonical-key variant of [`create_specialization`]. The minted
/// Specialization element's id is derived from the source feature's
/// canonical key — it stays stable across reparses.
///
/// `role` is the relationship's role at its parent (e.g. `"general"`,
/// or a parser-defined disambiguator); `sibling_index` is the zero-based
/// index among same-kind / same-role siblings of `parent_key`.
pub fn create_specialization_with_key(
    graph: &mut ModelGraph,
    specific_id: ElementId,
    general_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) = mint_relationship_element_keyed(
        ElementKind::Specialization,
        parent_key,
        role,
        sibling_index,
    );
    element.set_prop("specific", Value::Ref(specific_id.clone()));
    element.set_prop("unresolved_general", Value::String(general_qname));
    if let Some(s) = span {
        element.spans.push(s);
    }
    graph.add_owned_element_with_key(
        element,
        specific_id,
        VisibilityKind::Public,
        parent_key,
        &child_key,
    )
}

/// Canonical-key variant of [`create_subclassification`].
pub fn create_subclassification_with_key(
    graph: &mut ModelGraph,
    subclassifier_id: ElementId,
    superclassifier_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) = mint_relationship_element_keyed(
        ElementKind::Subclassification,
        parent_key,
        role,
        sibling_index,
    );
    element.set_prop("subclassifier", Value::Ref(subclassifier_id.clone()));
    element.set_prop(
        "unresolved_superclassifier",
        Value::String(superclassifier_qname),
    );
    element.set_prop("specific", Value::Ref(subclassifier_id.clone()));
    if let Some(s) = span {
        element.spans.push(s);
    }
    graph.add_owned_element_with_key(
        element,
        subclassifier_id,
        VisibilityKind::Public,
        parent_key,
        &child_key,
    )
}

/// Canonical-key variant of [`create_feature_typing`].
pub fn create_feature_typing_with_key(
    graph: &mut ModelGraph,
    typed_feature_id: ElementId,
    type_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) = mint_relationship_element_keyed(
        ElementKind::FeatureTyping,
        parent_key,
        role,
        sibling_index,
    );
    element.set_prop("typedFeature", Value::Ref(typed_feature_id.clone()));
    element.set_prop("unresolved_type", Value::String(type_qname));
    if let Some(s) = span {
        element.spans.push(s);
    }
    graph.add_owned_element_with_key(
        element,
        typed_feature_id,
        VisibilityKind::Public,
        parent_key,
        &child_key,
    )
}

/// Canonical-key variant of [`create_conjugated_port_typing`].
pub fn create_conjugated_port_typing_with_key(
    graph: &mut ModelGraph,
    typed_feature_id: ElementId,
    port_def_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) = mint_relationship_element_keyed(
        ElementKind::ConjugatedPortTyping,
        parent_key,
        role,
        sibling_index,
    );
    element.set_prop("typedFeature", Value::Ref(typed_feature_id.clone()));
    element.set_prop("unresolved_type", Value::String(port_def_qname));
    element.set_prop("isConjugated", Value::Bool(true));
    if let Some(s) = span {
        element.spans.push(s);
    }
    graph.add_owned_element_with_key(
        element,
        typed_feature_id,
        VisibilityKind::Public,
        parent_key,
        &child_key,
    )
}

/// Canonical-key variant of [`create_conjugated_port_definition`].
///
/// Note: the conjugated port definition is named (`~PortName`), so its
/// canonical key follows the named rule. We use the port-def name +
/// `~` prefix to distinguish it from its original.
pub fn create_conjugated_port_definition_with_key(
    graph: &mut ModelGraph,
    port_def_id: ElementId,
    port_def_name: &str,
    parent_id: Option<ElementId>,
    span: Option<Span>,
    parent_key: &CanonicalKey,
) -> ElementId {
    let conj_name = format!("~{}", port_def_name);
    let key = CanonicalKey::for_named(parent_key, "ConjugatedPortDefinition", &conj_name);
    let mut element = Element::new_with_key(ElementKind::ConjugatedPortDefinition, &key);
    element.name = Some(conj_name);
    element.set_prop("originalPortDefinition", Value::Ref(port_def_id));
    if let Some(s) = span {
        element.spans.push(s);
    }
    match parent_id {
        Some(pid) => graph.add_owned_element_with_key(
            element,
            pid,
            VisibilityKind::Public,
            parent_key,
            &key,
        ),
        None => graph.add_element(element),
    }
}

/// Canonical-key variant of [`create_cross_subsetting`].
pub fn create_cross_subsetting_with_key(
    graph: &mut ModelGraph,
    crossing_feature_id: ElementId,
    crossed_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) = mint_relationship_element_keyed(
        ElementKind::CrossSubsetting,
        parent_key,
        role,
        sibling_index,
    );
    element.set_prop("subsettingFeature", Value::Ref(crossing_feature_id.clone()));
    element.set_prop(
        "unresolved_crossedFeature",
        Value::String(crossed_qname.clone()),
    );
    element.set_prop("unresolved_subsettedFeature", Value::String(crossed_qname));
    if let Some(s) = span {
        element.spans.push(s);
    }
    graph.add_owned_element_with_key(
        element,
        crossing_feature_id,
        VisibilityKind::Public,
        parent_key,
        &child_key,
    )
}

/// Canonical-key variant of [`create_annotation`].
pub fn create_annotation_with_key(
    graph: &mut ModelGraph,
    annotating_element_id: ElementId,
    annotated_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) = mint_relationship_element_keyed(
        ElementKind::Annotation,
        parent_key,
        role,
        sibling_index,
    );
    element.set_prop(
        "annotatingElement",
        Value::Ref(annotating_element_id.clone()),
    );
    element.set_prop(
        "unresolved_annotatedElement",
        Value::String(annotated_qname),
    );
    if let Some(s) = span {
        element.spans.push(s);
    }
    graph.add_owned_element_with_key(
        element,
        annotating_element_id,
        VisibilityKind::Public,
        parent_key,
        &child_key,
    )
}

/// Canonical-key variant of [`create_subsetting`].
pub fn create_subsetting_with_key(
    graph: &mut ModelGraph,
    subsetting_feature_id: ElementId,
    subsetted_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) = mint_relationship_element_keyed(
        ElementKind::Subsetting,
        parent_key,
        role,
        sibling_index,
    );
    element.set_prop(
        "subsettingFeature",
        Value::Ref(subsetting_feature_id.clone()),
    );
    element.set_prop(
        "unresolved_subsettedFeature",
        Value::String(subsetted_qname),
    );
    if let Some(s) = span {
        element.spans.push(s);
    }
    graph.add_owned_element_with_key(
        element,
        subsetting_feature_id,
        VisibilityKind::Public,
        parent_key,
        &child_key,
    )
}

/// Create a STANDALONE `Subsetting` relationship — the namespace-member form
/// `subset X subsets Y;` (KerML.xtext:679-688), as opposed to the owned `:>`
/// form on a feature decl handled by [`create_subsetting_with_key`].
///
/// Unlike the owned form, BOTH endpoints are references resolved by name: the
/// `subsettingFeature` is stored as `unresolved_subsettingFeature` (the owned
/// form sets it to the owning feature directly). The relationship is owned by
/// the enclosing namespace (`namespace_id`), or added as a root element if none.
#[allow(clippy::too_many_arguments)]
pub fn create_standalone_subsetting_with_key(
    graph: &mut ModelGraph,
    namespace_id: Option<ElementId>,
    subsetting_qname: String,
    subsetted_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) = mint_relationship_element_keyed(
        ElementKind::Subsetting,
        parent_key,
        role,
        sibling_index,
    );
    element.set_prop(
        "unresolved_subsettingFeature",
        Value::String(subsetting_qname),
    );
    element.set_prop(
        "unresolved_subsettedFeature",
        Value::String(subsetted_qname),
    );
    if let Some(s) = span {
        element.spans.push(s);
    }
    match namespace_id {
        Some(pid) => graph.add_owned_element_with_key(
            element,
            pid,
            VisibilityKind::Public,
            parent_key,
            &child_key,
        ),
        None => graph.add_element(element),
    }
}

/// Canonical-key variant of [`create_redefinition`].
pub fn create_redefinition_with_key(
    graph: &mut ModelGraph,
    redefining_feature_id: ElementId,
    redefined_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) = mint_relationship_element_keyed(
        ElementKind::Redefinition,
        parent_key,
        role,
        sibling_index,
    );
    element.set_prop(
        "redefiningFeature",
        Value::Ref(redefining_feature_id.clone()),
    );
    element.set_prop(
        "unresolved_redefinedFeature",
        Value::String(redefined_qname),
    );
    if let Some(s) = span {
        element.spans.push(s);
    }
    graph.add_owned_element_with_key(
        element,
        redefining_feature_id,
        VisibilityKind::Public,
        parent_key,
        &child_key,
    )
}

/// Canonical-key variant of [`create_reference_subsetting`].
pub fn create_reference_subsetting_with_key(
    graph: &mut ModelGraph,
    referencing_feature_id: ElementId,
    referenced_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) = mint_relationship_element_keyed(
        ElementKind::ReferenceSubsetting,
        parent_key,
        role,
        sibling_index,
    );
    element.set_prop(
        "referencingFeature",
        Value::Ref(referencing_feature_id.clone()),
    );
    element.set_prop(
        "unresolved_referencedFeature",
        Value::String(referenced_qname),
    );
    if let Some(s) = span {
        element.spans.push(s);
    }
    graph.add_owned_element_with_key(
        element,
        referencing_feature_id,
        VisibilityKind::Public,
        parent_key,
        &child_key,
    )
}

// ---------------------------------------------------------------------------
// Type-relationship operators (KerML §7.3): Unioning / Intersecting /
// Differencing / Disjoining are Relationships owned by the *source* Type; each
// records the source Type as a Ref under its spec source-role name and the
// (as-yet-unresolved) target Type name under `unresolved_<targetRole>`, which
// name resolution converts to the resolved `<targetRole>` Ref. TypeFeaturing
// and FeatureInverting follow the same shape for Features. Source/target role
// names are the vocab's (Kerml-Vocab.ttl), e.g. Unioning: typeUnioned →
// unioningType.
fn create_type_relationship_with_key(
    graph: &mut ModelGraph,
    kind: ElementKind,
    source_role: &'static str,
    target_role: &str,
    source_id: ElementId,
    target_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    role: &str,
    sibling_index: usize,
) -> ElementId {
    let (mut element, child_key) =
        mint_relationship_element_keyed(kind, parent_key, role, sibling_index);
    element.set_prop(source_role, Value::Ref(source_id.clone()));
    element.set_prop(format!("unresolved_{target_role}"), Value::String(target_qname));
    if let Some(s) = span {
        element.spans.push(s);
    }
    graph.add_owned_element_with_key(
        element,
        source_id,
        VisibilityKind::Public,
        parent_key,
        &child_key,
    )
}

/// Create a `Unioning` (`unions`): the source Type's `unioningType` is one of
/// its unioned types. Owned by the source (`typeUnioned`).
pub fn create_unioning_with_key(
    graph: &mut ModelGraph,
    source_id: ElementId,
    target_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    sibling_index: usize,
) -> ElementId {
    create_type_relationship_with_key(
        graph,
        ElementKind::Unioning,
        "typeUnioned",
        "unioningType",
        source_id,
        target_qname,
        span,
        parent_key,
        "unioning",
        sibling_index,
    )
}

/// Create an `Intersecting` (`intersects`). Owned by the source
/// (`typeIntersected`); target `intersectingType`.
pub fn create_intersecting_with_key(
    graph: &mut ModelGraph,
    source_id: ElementId,
    target_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    sibling_index: usize,
) -> ElementId {
    create_type_relationship_with_key(
        graph,
        ElementKind::Intersecting,
        "typeIntersected",
        "intersectingType",
        source_id,
        target_qname,
        span,
        parent_key,
        "intersecting",
        sibling_index,
    )
}

/// Create a `Differencing` (`differences`). Owned by the source
/// (`typeDifferenced`); target `differencingType`.
pub fn create_differencing_with_key(
    graph: &mut ModelGraph,
    source_id: ElementId,
    target_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    sibling_index: usize,
) -> ElementId {
    create_type_relationship_with_key(
        graph,
        ElementKind::Differencing,
        "typeDifferenced",
        "differencingType",
        source_id,
        target_qname,
        span,
        parent_key,
        "differencing",
        sibling_index,
    )
}

/// Create a `Disjoining` (`disjoint from`). Owned by the source
/// (`typeDisjoined`); target `disjoiningType`.
pub fn create_disjoining_with_key(
    graph: &mut ModelGraph,
    source_id: ElementId,
    target_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    sibling_index: usize,
) -> ElementId {
    create_type_relationship_with_key(
        graph,
        ElementKind::Disjoining,
        "typeDisjoined",
        "disjoiningType",
        source_id,
        target_qname,
        span,
        parent_key,
        "disjoining",
        sibling_index,
    )
}

/// Create a `TypeFeaturing` (`featured by`). Owned by the source Feature
/// (`featureOfType`); target `featuringType`.
pub fn create_type_featuring_with_key(
    graph: &mut ModelGraph,
    source_id: ElementId,
    target_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    sibling_index: usize,
) -> ElementId {
    create_type_relationship_with_key(
        graph,
        ElementKind::TypeFeaturing,
        "featureOfType",
        "featuringType",
        source_id,
        target_qname,
        span,
        parent_key,
        "type_featuring",
        sibling_index,
    )
}

/// Create a `FeatureInverting` (`inverse of`). Owned by the source Feature
/// (`featureInverted`); target `invertingFeature`.
pub fn create_feature_inverting_with_key(
    graph: &mut ModelGraph,
    source_id: ElementId,
    target_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    sibling_index: usize,
) -> ElementId {
    create_type_relationship_with_key(
        graph,
        ElementKind::FeatureInverting,
        "featureInverted",
        "invertingFeature",
        source_id,
        target_qname,
        span,
        parent_key,
        "feature_inverting",
        sibling_index,
    )
}

/// Create a `Conjugation` (`~` / `conjugates`, KerML ConjugationPart §337).
/// The declaring type is the (already-known) `conjugatedType` — recorded as a
/// resolved Ref on `source_id` — and the clause target is the `originalType`
/// (captured unresolved for `pass2::resolve_conjugation`, which also resolves
/// `unresolved_originalType`). Owned by the source (the conjugatedType).
pub fn create_conjugation_with_key(
    graph: &mut ModelGraph,
    source_id: ElementId,
    target_qname: String,
    span: Option<Span>,
    parent_key: &CanonicalKey,
    sibling_index: usize,
) -> ElementId {
    create_type_relationship_with_key(
        graph,
        ElementKind::Conjugation,
        "conjugatedType",
        "originalType",
        source_id,
        target_qname,
        span,
        parent_key,
        "conjugation",
        sibling_index,
    )
}

/// Canonical-key variant of [`create_usage_relationships`].
///
/// Each per-target relationship gets `sibling_index = the_target_index`
/// within its role's vector. Roles match the extraction's vector names:
/// `"typing"`, `"subsetting"`, `"redefinition"`, `"reference"`, `"crosses"`.
pub fn create_usage_relationships_with_key(
    graph: &mut ModelGraph,
    feature_id: &ElementId,
    extraction: &crate::extraction::UsageExtraction,
    span: Option<&Span>,
    parent_key: &CanonicalKey,
) {
    for (i, type_qname) in extraction.typings.iter().enumerate() {
        create_feature_typing_with_key(
            graph,
            feature_id.clone(),
            type_qname.clone(),
            span.cloned(),
            parent_key,
            "typing",
            i,
        );
    }
    for (i, subsetted) in extraction.subsettings.iter().enumerate() {
        create_subsetting_with_key(
            graph,
            feature_id.clone(),
            subsetted.clone(),
            span.cloned(),
            parent_key,
            "subsetting",
            i,
        );
    }
    for (i, redefined) in extraction.redefinitions.iter().enumerate() {
        create_redefinition_with_key(
            graph,
            feature_id.clone(),
            redefined.clone(),
            span.cloned(),
            parent_key,
            "redefinition",
            i,
        );
    }
    for (i, referenced) in extraction.references.iter().enumerate() {
        create_reference_subsetting_with_key(
            graph,
            feature_id.clone(),
            referenced.clone(),
            span.cloned(),
            parent_key,
            "reference",
            i,
        );
    }
    for (i, crossed) in extraction.crosses.iter().enumerate() {
        create_cross_subsetting_with_key(
            graph,
            feature_id.clone(),
            crossed.clone(),
            span.cloned(),
            parent_key,
            "crosses",
            i,
        );
    }
}

/// Canonical-key variant of [`create_definition_relationships`].
pub fn create_definition_relationships_with_key(
    graph: &mut ModelGraph,
    specific_id: &ElementId,
    extraction: &crate::extraction::DefinitionExtraction,
    span: Option<&Span>,
    parent_key: &CanonicalKey,
) {
    for (i, target_qname) in extraction.subclassifications.iter().enumerate() {
        create_specialization_with_key(
            graph,
            specific_id.clone(),
            target_qname.clone(),
            span.cloned(),
            parent_key,
            "subclassification",
            i,
        );
    }
}

/// Create all relationship elements from a UsageExtraction.
///
/// This is a convenience function that creates FeatureTyping, Subsetting,
/// Redefinition, and ReferenceSubsetting elements for all targets in the
/// extraction.
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add elements to
/// * `feature_id` - The ID of the feature element
/// * `extraction` - The UsageExtraction containing relationship targets
/// * `span` - Optional span for source location tracking
pub fn create_usage_relationships(
    graph: &mut ModelGraph,
    feature_id: &ElementId,
    extraction: &crate::extraction::UsageExtraction,
    span: Option<&Span>,
) {
    // Create FeatureTyping elements
    for type_qname in &extraction.typings {
        create_feature_typing(graph, feature_id.clone(), type_qname.clone(), span.cloned());
    }

    // Create Subsetting elements
    for subsetted in &extraction.subsettings {
        create_subsetting(graph, feature_id.clone(), subsetted.clone(), span.cloned());
    }

    // Create Redefinition elements
    for redefined in &extraction.redefinitions {
        create_redefinition(graph, feature_id.clone(), redefined.clone(), span.cloned());
    }

    // Create ReferenceSubsetting elements
    for referenced in &extraction.references {
        create_reference_subsetting(graph, feature_id.clone(), referenced.clone(), span.cloned());
    }

    // Create CrossSubsetting elements
    for crossed in &extraction.crosses {
        create_cross_subsetting(graph, feature_id.clone(), crossed.clone(), span.cloned());
    }
}

/// Create Specialization elements from a DefinitionExtraction.
///
/// # Arguments
///
/// * `graph` - The ModelGraph to add elements to
/// * `specific_id` - The ID of the definition element
/// * `extraction` - The DefinitionExtraction containing subclassification targets
/// * `span` - Optional span for source location tracking
pub fn create_definition_relationships(
    graph: &mut ModelGraph,
    specific_id: &ElementId,
    extraction: &crate::extraction::DefinitionExtraction,
    span: Option<&Span>,
) {
    for target_qname in &extraction.subclassifications {
        create_specialization(
            graph,
            specific_id.clone(),
            target_qname.clone(),
            span.cloned(),
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_create_specialization() {
        let mut graph = ModelGraph::new();

        // Create a definition first
        let def = Element::new_with_kind(ElementKind::PartDefinition).with_name("MyPart");
        let def_id = graph.add_element(def);

        // Create specialization
        let spec_id = create_specialization(&mut graph, def_id.clone(), "Base".to_string(), None);

        let spec = graph.get_element(&spec_id).unwrap();
        assert_eq!(spec.kind, ElementKind::Specialization);
        assert_eq!(
            spec.get_prop("specific").and_then(|v| v.as_ref()),
            Some(&def_id)
        );
        assert_eq!(
            spec.get_prop("unresolved_general").and_then(|v| v.as_str()),
            Some("Base")
        );
    }

    #[test]
    fn test_create_feature_typing() {
        let mut graph = ModelGraph::new();

        let usage = Element::new_with_kind(ElementKind::PartUsage).with_name("part");
        let usage_id = graph.add_element(usage);

        let typing_id =
            create_feature_typing(&mut graph, usage_id.clone(), "Integer".to_string(), None);

        let typing = graph.get_element(&typing_id).unwrap();
        assert_eq!(typing.kind, ElementKind::FeatureTyping);
        assert_eq!(
            typing.get_prop("typedFeature").and_then(|v| v.as_ref()),
            Some(&usage_id)
        );
        assert_eq!(
            typing.get_prop("unresolved_type").and_then(|v| v.as_str()),
            Some("Integer")
        );
    }

    #[test]
    fn test_create_subsetting() {
        let mut graph = ModelGraph::new();

        let usage = Element::new_with_kind(ElementKind::PartUsage).with_name("subset");
        let usage_id = graph.add_element(usage);

        let sub_id = create_subsetting(&mut graph, usage_id.clone(), "superset".to_string(), None);

        let sub = graph.get_element(&sub_id).unwrap();
        assert_eq!(sub.kind, ElementKind::Subsetting);
        assert_eq!(
            sub.get_prop("subsettingFeature").and_then(|v| v.as_ref()),
            Some(&usage_id)
        );
        assert_eq!(
            sub.get_prop("unresolved_subsettedFeature")
                .and_then(|v| v.as_str()),
            Some("superset")
        );
    }

    #[test]
    fn test_create_redefinition() {
        let mut graph = ModelGraph::new();

        let usage = Element::new_with_kind(ElementKind::PartUsage).with_name("redefining");
        let usage_id = graph.add_element(usage);

        let redef_id =
            create_redefinition(&mut graph, usage_id.clone(), "redefined".to_string(), None);

        let redef = graph.get_element(&redef_id).unwrap();
        assert_eq!(redef.kind, ElementKind::Redefinition);
        assert_eq!(
            redef.get_prop("redefiningFeature").and_then(|v| v.as_ref()),
            Some(&usage_id)
        );
        assert_eq!(
            redef
                .get_prop("unresolved_redefinedFeature")
                .and_then(|v| v.as_str()),
            Some("redefined")
        );
    }

    #[test]
    fn test_create_reference_subsetting() {
        let mut graph = ModelGraph::new();

        let usage = Element::new_with_kind(ElementKind::ReferenceUsage).with_name("ref");
        let usage_id = graph.add_element(usage);

        let ref_id =
            create_reference_subsetting(&mut graph, usage_id.clone(), "target".to_string(), None);

        let ref_elem = graph.get_element(&ref_id).unwrap();
        assert_eq!(ref_elem.kind, ElementKind::ReferenceSubsetting);
        assert_eq!(
            ref_elem
                .get_prop("referencingFeature")
                .and_then(|v| v.as_ref()),
            Some(&usage_id)
        );
        assert_eq!(
            ref_elem
                .get_prop("unresolved_referencedFeature")
                .and_then(|v| v.as_str()),
            Some("target")
        );
    }

    #[test]
    fn test_create_usage_relationships() {
        use crate::extraction::UsageExtraction;

        let mut graph = ModelGraph::new();

        let usage = Element::new_with_kind(ElementKind::PartUsage).with_name("myPart");
        let usage_id = graph.add_element(usage);

        let extraction = UsageExtraction {
            typings: vec!["MyType".to_string()],
            subsettings: vec!["parent".to_string()],
            redefinitions: vec!["base".to_string()],
            references: vec!["ref".to_string()],
            ..Default::default()
        };

        create_usage_relationships(&mut graph, &usage_id, &extraction, None);

        // Count the created relationship elements
        let typing_count = graph
            .elements
            .values()
            .filter(|e| e.kind == ElementKind::FeatureTyping)
            .count();
        let subsetting_count = graph
            .elements
            .values()
            .filter(|e| e.kind == ElementKind::Subsetting)
            .count();
        let redef_count = graph
            .elements
            .values()
            .filter(|e| e.kind == ElementKind::Redefinition)
            .count();
        let refsub_count = graph
            .elements
            .values()
            .filter(|e| e.kind == ElementKind::ReferenceSubsetting)
            .count();

        assert_eq!(typing_count, 1);
        assert_eq!(subsetting_count, 1);
        assert_eq!(redef_count, 1);
        assert_eq!(refsub_count, 1);
    }

    #[test]
    fn test_create_definition_relationships() {
        use crate::extraction::DefinitionExtraction;

        let mut graph = ModelGraph::new();

        let def = Element::new_with_kind(ElementKind::PartDefinition).with_name("MyPart");
        let def_id = graph.add_element(def);

        let extraction = DefinitionExtraction {
            subclassifications: vec!["Base1".to_string(), "Base2".to_string()],
            ..Default::default()
        };

        create_definition_relationships(&mut graph, &def_id, &extraction, None);

        // Count the created specialization elements
        let spec_count = graph
            .elements
            .values()
            .filter(|e| e.kind == ElementKind::Specialization)
            .count();
        assert_eq!(spec_count, 2);
    }

    #[test]
    fn test_create_cross_subsetting() {
        let mut graph = ModelGraph::new();

        let usage = Element::new_with_kind(ElementKind::PartUsage).with_name("lane");
        let usage_id = graph.add_element(usage);

        let cross_id =
            create_cross_subsetting(&mut graph, usage_id.clone(), "road".to_string(), None);

        let cross = graph.get_element(&cross_id).unwrap();
        assert_eq!(cross.kind, ElementKind::CrossSubsetting);
        assert_eq!(
            cross.get_prop("subsettingFeature").and_then(|v| v.as_ref()),
            Some(&usage_id)
        );
        assert_eq!(
            cross
                .get_prop("unresolved_crossedFeature")
                .and_then(|v| v.as_str()),
            Some("road")
        );
        // Also has subsettedFeature for resolution compatibility
        assert_eq!(
            cross
                .get_prop("unresolved_subsettedFeature")
                .and_then(|v| v.as_str()),
            Some("road")
        );
    }

    #[test]
    fn test_create_annotation() {
        let mut graph = ModelGraph::new();

        let metadata = Element::new_with_kind(ElementKind::MetadataUsage).with_name("myMeta");
        let metadata_id = graph.add_element(metadata);

        let ann_id =
            create_annotation(&mut graph, metadata_id.clone(), "Vehicle".to_string(), None);

        let ann = graph.get_element(&ann_id).unwrap();
        assert_eq!(ann.kind, ElementKind::Annotation);
        assert_eq!(
            ann.get_prop("annotatingElement").and_then(|v| v.as_ref()),
            Some(&metadata_id)
        );
        assert_eq!(
            ann.get_prop("unresolved_annotatedElement")
                .and_then(|v| v.as_str()),
            Some("Vehicle")
        );
    }

    #[test]
    fn test_create_conjugated_port_definition() {
        let mut graph = ModelGraph::new();

        // Create a parent namespace
        let ns = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let ns_id = graph.add_element(ns);

        // Create a PortDefinition
        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("WaterPort");
        let port_def_id = graph.add_owned_element(port_def, ns_id.clone(), VisibilityKind::Public);

        // Create ConjugatedPortDefinition
        let conj_id = super::create_conjugated_port_definition(
            &mut graph,
            port_def_id.clone(),
            "WaterPort",
            Some(ns_id),
            None,
        );

        let conj = graph.get_element(&conj_id).unwrap();
        assert_eq!(conj.kind, ElementKind::ConjugatedPortDefinition);
        assert_eq!(conj.name.as_deref(), Some("~WaterPort"));
        assert_eq!(
            conj.get_prop("originalPortDefinition")
                .and_then(|v| v.as_ref()),
            Some(&port_def_id)
        );
    }

    // === Canonical-key variants (ADR-009 / S1) ===

    /// Build a fresh graph with a usage element placed under `(p::Pkg)`,
    /// returning `(graph, usage_id, parent_key)` for the canonical-key
    /// variant tests below.
    fn fresh_graph_with_usage() -> (ModelGraph, ElementId, CanonicalKey) {
        let mut graph = ModelGraph::new();
        let usage = Element::new_with_kind(ElementKind::PartUsage).with_name("part");
        let usage_id = graph.add_element(usage);
        let parent_key =
            CanonicalKey::for_named(&CanonicalKey::root("p"), "PartUsage", "part");
        (graph, usage_id, parent_key)
    }

    #[test]
    fn create_feature_typing_with_key_is_stable() {
        let (mut graph_a, usage_a, key_a) = fresh_graph_with_usage();
        let (mut graph_b, usage_b, key_b) = fresh_graph_with_usage();

        let id_a = create_feature_typing_with_key(
            &mut graph_a,
            usage_a,
            "Integer".to_string(),
            None,
            &key_a,
            "type",
            0,
        );
        let id_b = create_feature_typing_with_key(
            &mut graph_b,
            usage_b,
            "Integer".to_string(),
            None,
            &key_b,
            "type",
            0,
        );

        // Same parent_key + role + sibling_index → identical relationship id
        // across two independent graphs.
        assert_eq!(id_a, id_b);

        // Distinct sibling indices yield distinct ids.
        let (mut graph_c, usage_c, key_c) = fresh_graph_with_usage();
        let id_c = create_feature_typing_with_key(
            &mut graph_c,
            usage_c,
            "Integer".to_string(),
            None,
            &key_c,
            "type",
            1,
        );
        assert_ne!(id_a, id_c);
    }

    #[test]
    fn create_specialization_with_key_distinct_roles() {
        // Same parent + same sibling_index but different roles → different ids.
        let parent = CanonicalKey::root("p");

        let mut graph_a = ModelGraph::new();
        let def_a = Element::new_with_kind(ElementKind::PartDefinition).with_name("D");
        let def_a_id = graph_a.add_element(def_a);
        let id_a = create_specialization_with_key(
            &mut graph_a,
            def_a_id,
            "Base".to_string(),
            None,
            &parent,
            "general",
            0,
        );

        let mut graph_b = ModelGraph::new();
        let def_b = Element::new_with_kind(ElementKind::PartDefinition).with_name("D");
        let def_b_id = graph_b.add_element(def_b);
        let id_b = create_specialization_with_key(
            &mut graph_b,
            def_b_id,
            "Base".to_string(),
            None,
            &parent,
            "subclassification",
            0,
        );

        assert_ne!(id_a, id_b, "different roles must yield different ids");
    }

    #[test]
    fn create_specialization_without_key_uses_fresh_uuid() {
        // Sanity: the legacy entry point still mints fresh ids per call.
        let mut graph = ModelGraph::new();
        let def = Element::new_with_kind(ElementKind::PartDefinition).with_name("D");
        let def_id = graph.add_element(def);

        let id_a = create_specialization(&mut graph, def_id.clone(), "Base".to_string(), None);
        let id_b = create_specialization(&mut graph, def_id, "Base".to_string(), None);

        assert_ne!(id_a, id_b);
    }

    #[test]
    fn create_usage_relationships_with_key_is_stable_and_indexed() {
        use crate::extraction::UsageExtraction;

        let extraction = UsageExtraction {
            typings: vec!["T1".to_string(), "T2".to_string()],
            subsettings: vec!["S1".to_string()],
            redefinitions: vec!["R1".to_string()],
            ..Default::default()
        };

        let mk_graph = || -> (ModelGraph, ElementId, CanonicalKey) {
            let mut g = ModelGraph::new();
            let u = Element::new_with_kind(ElementKind::PartUsage).with_name("u");
            let uid = g.add_element(u);
            let pk = CanonicalKey::for_named(&CanonicalKey::root("p"), "PartUsage", "u");
            (g, uid, pk)
        };

        let (mut g1, u1, k1) = mk_graph();
        create_usage_relationships_with_key(&mut g1, &u1, &extraction, None, &k1);

        let (mut g2, u2, k2) = mk_graph();
        create_usage_relationships_with_key(&mut g2, &u2, &extraction, None, &k2);

        // Aggregate the relationship ids by kind (sorted) from both graphs.
        let collect_ids = |g: &ModelGraph, kind: ElementKind| -> Vec<ElementId> {
            let mut ids: Vec<ElementId> = g
                .elements
                .values()
                .filter(|e| e.kind == kind)
                .map(|e| e.id.clone())
                .collect();
            ids.sort();
            ids
        };

        // Two FeatureTyping (one per typing target), one Subsetting, one Redefinition.
        assert_eq!(
            collect_ids(&g1, ElementKind::FeatureTyping),
            collect_ids(&g2, ElementKind::FeatureTyping)
        );
        assert_eq!(collect_ids(&g1, ElementKind::FeatureTyping).len(), 2);
        assert_eq!(
            collect_ids(&g1, ElementKind::Subsetting),
            collect_ids(&g2, ElementKind::Subsetting)
        );
        assert_eq!(
            collect_ids(&g1, ElementKind::Redefinition),
            collect_ids(&g2, ElementKind::Redefinition)
        );

        // The two typings within g1 must differ (sibling_index 0 vs 1).
        let tids = collect_ids(&g1, ElementKind::FeatureTyping);
        assert_ne!(tids[0], tids[1]);
    }
}
