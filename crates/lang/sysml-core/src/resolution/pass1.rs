//! Pass 1 resolution handlers: type relationships that establish inheritance chains.
//!
//! These must be resolved first so inherited members become visible
//! in the scope table for pass 2.

use std::borrow::Cow;

use sysml_id::ElementId;

use super::context::ResolutionContext;
use super::{resolved_props, unresolved_props};

/// Resolve a Specialization element's general property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_specialization(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(general_ref) = element
        .props
        .get(unresolved_props::GENERAL)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, general_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::GENERAL),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::GENERAL.to_owned(),
                general_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a FeatureTyping element's type property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_feature_typing(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(type_ref) = element
        .props
        .get(unresolved_props::TYPE)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, type_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::TYPE),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::TYPE.to_owned(),
                type_ref.to_owned(),
            ));
        }
    }
}

/// Resolve a Subclassification element's superclassifier property.
#[cfg_attr(
    feature = "resolution-tracing",
    tracing::instrument(skip(ctx, updates, unresolved), fields(element_id = %element.id))
)]
pub(crate) fn resolve_subclassification(
    element: &crate::Element,
    scope_id: &ElementId,
    ctx: &mut ResolutionContext<'_>,
    updates: &mut Vec<(ElementId, Cow<'static, str>, ElementId)>,
    unresolved: &mut Vec<(ElementId, String, String)>,
) {
    if let Some(superclassifier_ref) = element
        .props
        .get(unresolved_props::SUPERCLASSIFIER)
        .and_then(|v| v.as_str())
    {
        if let Some(resolved_id) = ctx.resolve_qualified_name(scope_id, superclassifier_ref) {
            updates.push((
                element.id.clone(),
                Cow::Borrowed(resolved_props::SUPERCLASSIFIER),
                resolved_id,
            ));
        } else {
            unresolved.push((
                element.id.clone(),
                resolved_props::SUPERCLASSIFIER.to_owned(),
                superclassifier_ref.to_owned(),
            ));
        }
    }
}
