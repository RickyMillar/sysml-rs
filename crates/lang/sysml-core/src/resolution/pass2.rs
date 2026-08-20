//! Pass 2 resolution handlers: feature relationships and other cross-references.
//!
//! These depend on inheritance chains established in pass 1, making inherited
//! members visible through the resolved type hierarchy.

use std::borrow::Cow;

use sysml_id::ElementId;

use super::context::ResolutionContext;
use super::{resolved_props, unresolved_props};

/// Resolve a Subsetting element's subsettedFeature property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_subsetting(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(subsetted_ref) = element
        .props
        .get(unresolved_props::SUBSETTED_FEATURE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_feature_reference(scope_id, subsetted_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::SUBSETTED_FEATURE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::SUBSETTED_FEATURE.to_owned(),
                subsetted_ref.to_owned(),
            ));
        }
    }

    // Standalone `subset X subsets Y;` (G08e): the subsetting endpoint is also a
    // by-name reference. The owned `:>` form instead sets `subsettingFeature`
    // directly to its owning feature, so this branch only fires for the
    // namespace-member form.
    if let Some(subsetting_ref) = element
        .props
        .get(unresolved_props::SUBSETTING_FEATURE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_feature_reference(scope_id, subsetting_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::SUBSETTING_FEATURE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::SUBSETTING_FEATURE.to_owned(),
                subsetting_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a Redefinition element's redefinedFeature property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_redefinition(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(redefined_ref) = element
        .props
        .get(unresolved_props::REDEFINED_FEATURE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_redefined_feature(scope_id, redefined_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::REDEFINED_FEATURE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::REDEFINED_FEATURE.to_owned(),
                redefined_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a ReferenceSubsetting element's referencedFeature property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_reference_subsetting(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(referenced_ref) = element
        .props
        .get(unresolved_props::REFERENCED_FEATURE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_feature_reference(scope_id, referenced_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::REFERENCED_FEATURE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::REFERENCED_FEATURE.to_owned(),
                referenced_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a Dependency element's source and target properties.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_dependency(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    // Resolve sources
    if let Some(sources) = element
        .props
        .get(unresolved_props::SOURCES)
        .and_then(|v| v.as_list())
    {
        for source_val in sources {
            if let Some(source_ref) = source_val.as_str() {
                if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, source_ref) {
                    updates.push((
                        element.id.clone(),
                        Cow::Borrowed(resolved_props::SOURCES),
                        resolved_id,
                    ));
                } else {
                    unresolved.push((
                        element.id.clone(),
                        resolved_props::SOURCES.to_owned(),
                        source_ref.to_owned(),
                    ));
                }
            }
        }
    }

    // Resolve targets
    if let Some(targets) = element
        .props
        .get(unresolved_props::TARGETS)
        .and_then(|v| v.as_list())
    {
        for target_val in targets {
            if let Some(target_ref) = target_val.as_str() {
                if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, target_ref) {
                    updates.push((
                        element.id.clone(),
                        Cow::Borrowed(resolved_props::TARGETS),
                        resolved_id,
                    ));
                } else {
                    unresolved.push((
                        element.id.clone(),
                        resolved_props::TARGETS.to_owned(),
                        target_ref.to_owned(),
                    ));
                }
            }
        }
    }

    // Also resolve client/supplier if present (alternative properties for Dependency)
    if let Some(client_ref) = element
        .props
        .get(unresolved_props::CLIENT)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, client_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::CLIENT),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::CLIENT.to_owned(),
                client_ref.to_owned(),
            ));
        }
    }

    if let Some(supplier_ref) = element
        .props
        .get(unresolved_props::SUPPLIER)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, supplier_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::SUPPLIER),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::SUPPLIER.to_owned(),
                supplier_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a Conjugation element's conjugatedType and originalType properties.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_conjugation(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    // Resolve conjugatedType
    if let Some(conjugated_ref) = element
        .props
        .get(unresolved_props::CONJUGATED_TYPE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, conjugated_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::CONJUGATED_TYPE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::CONJUGATED_TYPE.to_owned(),
                conjugated_ref.to_owned(),
            ));
        }
    }

    // Resolve originalType
    if let Some(original_ref) = element
        .props
        .get(unresolved_props::ORIGINAL_TYPE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, original_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::ORIGINAL_TYPE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::ORIGINAL_TYPE.to_owned(),
                original_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a TypeFeaturing element's featuringType property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_type_featuring(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(featuring_ref) = element
        .props
        .get(unresolved_props::FEATURING_TYPE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, featuring_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::FEATURING_TYPE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::FEATURING_TYPE.to_owned(),
                featuring_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a Disjoining element's disjoiningType property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_disjoining(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(disjoining_ref) = element
        .props
        .get(unresolved_props::DISJOINING_TYPE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, disjoining_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::DISJOINING_TYPE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::DISJOINING_TYPE.to_owned(),
                disjoining_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a Unioning element's unioningType property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_unioning(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(unioning_ref) = element
        .props
        .get(unresolved_props::UNIONING_TYPE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, unioning_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::UNIONING_TYPE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::UNIONING_TYPE.to_owned(),
                unioning_ref.to_owned(),
            ));
        }
    }
}

/// Resolve an Intersecting element's intersectingType property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_intersecting(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(intersecting_ref) = element
        .props
        .get(unresolved_props::INTERSECTING_TYPE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, intersecting_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::INTERSECTING_TYPE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::INTERSECTING_TYPE.to_owned(),
                intersecting_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a Differencing element's differencingType property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_differencing(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(differencing_ref) = element
        .props
        .get(unresolved_props::DIFFERENCING_TYPE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, differencing_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::DIFFERENCING_TYPE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::DIFFERENCING_TYPE.to_owned(),
                differencing_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a FeatureInverting element's invertingFeature property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_feature_inverting(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(inverting_ref) = element
        .props
        .get(unresolved_props::INVERTING_FEATURE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, inverting_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::INVERTING_FEATURE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::INVERTING_FEATURE.to_owned(),
                inverting_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a FeatureChaining element's crossedFeature property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_feature_chaining(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    // Note: Feature chaining resolution is more complex and may need
    // the FeatureChaining scoping strategy for proper resolution.
    // For now, we use the standard qualified name resolution.
    if let Some(crossed_ref) = element
        .props
        .get(unresolved_props::CROSSED_FEATURE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, crossed_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::CROSSED_FEATURE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::CROSSED_FEATURE.to_owned(),
                crossed_ref.to_owned(),
            ));
        }
    }
}

/// Resolve an Annotation element's annotatedElement property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_annotation(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(annotated_ref) = element
        .props
        .get(unresolved_props::ANNOTATED_ELEMENT)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, annotated_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::ANNOTATED_ELEMENT),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::ANNOTATED_ELEMENT.to_owned(),
                annotated_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a Membership element's memberElement property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_membership(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(member_ref) = element
        .props
        .get(unresolved_props::MEMBER_ELEMENT)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, member_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::MEMBER_ELEMENT),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::MEMBER_ELEMENT.to_owned(),
                member_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a ConjugatedPortDefinition element's conjugatedPortDefinition property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_conjugated_port_definition(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(port_def_ref) = element
        .props
        .get(unresolved_props::CONJUGATED_PORT_DEFINITION)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, port_def_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::CONJUGATED_PORT_DEFINITION),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::CONJUGATED_PORT_DEFINITION.to_owned(),
                port_def_ref.to_owned(),
            ));
        }
    }
}
