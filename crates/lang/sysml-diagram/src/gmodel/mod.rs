//! Spatial / floor-plan payload types for `GeometryView`.
//!
//! Produced by [`to_geometry_model`] and embedded in
//! `DiagramPayload::Geometry` for the wire format. Consumers (FE, MCP
//! clients, REST callers) work with primitives directly — no Sprotty
//! SModel or ELK layout involved.
//!
//! Phase 5b will add `x` / `y` / `width` / `height` attributes to a spatial
//! fixture model so this generator has real spatial content
//! to emit. Until then, models without those attributes round-trip as
//! an empty primitives list.

use serde::Serialize;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph};

/// A complete spatial payload.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeometryModel {
    /// Optional descriptive title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Generator tag (e.g. `"spatial_layout"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Bounding box of all primitives (computed). `None` for empty payloads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Viewport>,
    /// Drawable primitives in document order.
    pub primitives: Vec<GeometryPrimitive>,
}

/// Axis-aligned bounding box covering all primitives.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// A drawable spatial primitive.
///
/// v1 supports rectangles only — sufficient for floor plans, DIN rails,
/// modular Breaker layouts, and panel layouts. Circle / Line / Polygon /
/// Group can be added when the model warrants them.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "shape", rename_all = "camelCase")]
pub enum GeometryPrimitive {
    /// Axis-aligned rectangle.
    Rect {
        /// Stable identifier for the primitive.
        id: String,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        /// Optional display label rendered inside / near the rectangle.
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// CSS class hints for styling.
        #[serde(rename = "cssClasses", skip_serializing_if = "Vec::is_empty")]
        css_classes: Vec<String>,
        /// Optional element id for navigation back to the model.
        #[serde(rename = "elementId", skip_serializing_if = "Option::is_none")]
        element_id: Option<String>,
    },
}

// ── Generator ────────────────────────────────────────────────────────────

/// Build a `GeometryModel` from elements that carry `x` / `y` / `width` /
/// `height` numeric attributes. Elements missing any of those four are
/// skipped — the typed payload only carries fully-positioned primitives.
///
/// Each numeric attribute is resolved either as a direct property on the
/// element (`element.get_prop("x")`) or as an `AttributeUsage` child
/// named `x` carrying a `default` property — the latter is the natural
/// SysML way of writing `attribute x = 100.0;` and is what models will
/// actually use in practice. The runtime pipeline (`compiler.rs`)
/// resolves attribute defaults the same way.
pub fn to_geometry_model(graph: &ModelGraph, expose: Option<&ElementId>) -> GeometryModel {
    let mut primitives: Vec<GeometryPrimitive> = Vec::new();

    for element in graph.elements.values() {
        // 3.12 scoping (SPEC-SILENT — the spec defines GeometryView but not its
        // expose-scoping): with an expose, keep only the exposed element and its
        // spatial descendants; with none, exclude the standard library (mirrors
        // General/Grid/Sequence) so a no-expose Geometry doesn't dump the stdlib.
        let in_scope = match expose {
            Some(eid) => &element.id == eid || graph.is_descendant_of(&element.id, eid),
            None => !graph.is_library_element(&element.id),
        };
        if !in_scope {
            continue;
        }
        let Some((x, y)) = extract_position(graph, element) else {
            continue;
        };
        let Some((width, height)) = extract_size(graph, element) else {
            continue;
        };

        let label = element.name.clone();
        primitives.push(GeometryPrimitive::Rect {
            id: format!("rect:{}", element.id),
            x,
            y,
            width,
            height,
            label,
            css_classes: vec![format!("kind-{}", kind_css_token(element))],
            element_id: Some(element.id.to_string()),
        });
    }

    // Render order: largest area first so big containers (enclosures)
    // sit behind their smaller contents in the SVG. Stable on id for
    // ties so output is deterministic per workspace.
    primitives.sort_by(|a, b| {
        let area_a = primitive_area(a);
        let area_b = primitive_area(b);
        area_b
            .partial_cmp(&area_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| primitive_id(a).cmp(primitive_id(b)))
    });

    let viewport = compute_viewport(&primitives);

    GeometryModel {
        title: Some("Spatial Layout".to_owned()),
        kind: Some("spatial_layout".to_owned()),
        viewport,
        primitives,
    }
}

/// Resolve a numeric attribute by name. Looks first at the element's own
/// property bag (cheaper, used by hand-built test fixtures), then falls
/// back to walking AttributeUsage children — the path SysML source code
/// `attribute foo = 1.0;` produces.
fn resolve_attribute(graph: &ModelGraph, element: &Element, name: &str) -> Option<f64> {
    if let Some(direct) = element.get_prop(name).and_then(|v| v.as_float()) {
        return Some(direct);
    }
    graph
        .children_of(&element.id)
        .filter(|c| c.kind == ElementKind::AttributeUsage)
        .find(|c| c.name.as_deref() == Some(name))
        .and_then(|c| {
            c.get_prop("default")
                .or_else(|| c.get_prop("value"))
                .and_then(|v| v.as_float())
        })
}

fn extract_position(graph: &ModelGraph, element: &Element) -> Option<(f64, f64)> {
    let x = resolve_attribute(graph, element, "x")?;
    let y = resolve_attribute(graph, element, "y")?;
    Some((x, y))
}

fn extract_size(graph: &ModelGraph, element: &Element) -> Option<(f64, f64)> {
    let width = resolve_attribute(graph, element, "width")?;
    let height = resolve_attribute(graph, element, "height")?;
    Some((width, height))
}

fn kind_css_token(element: &Element) -> String {
    format!("{:?}", element.kind).to_lowercase()
}

fn primitive_area(prim: &GeometryPrimitive) -> f64 {
    let GeometryPrimitive::Rect { width, height, .. } = prim;
    width * height
}

fn primitive_id(prim: &GeometryPrimitive) -> &str {
    let GeometryPrimitive::Rect { id, .. } = prim;
    id
}

fn compute_viewport(primitives: &[GeometryPrimitive]) -> Option<Viewport> {
    if primitives.is_empty() {
        return None;
    }
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for prim in primitives {
        let GeometryPrimitive::Rect {
            x, y, width, height, ..
        } = prim;
        min_x = min_x.min(*x);
        min_y = min_y.min(*y);
        max_x = max_x.max(*x + *width);
        max_y = max_y.max(*y + *height);
    }
    Some(Viewport {
        min_x,
        min_y,
        max_x,
        max_y,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind};

    fn part_with_geometry(name: &str, x: f64, y: f64, w: f64, h: f64) -> Element {
        Element::new_with_kind(ElementKind::PartUsage)
            .with_name(name)
            .with_prop("x", x)
            .with_prop("y", y)
            .with_prop("width", w)
            .with_prop("height", h)
    }

    #[test]
    fn empty_graph_produces_empty_payload() {
        let graph = ModelGraph::new();
        let model = to_geometry_model(&graph, None);
        assert!(model.primitives.is_empty());
        assert!(model.viewport.is_none());
    }

    #[test]
    fn elements_without_geometry_attrs_are_skipped() {
        let mut graph = ModelGraph::new();
        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Naked");
        graph.add_element(part);
        let model = to_geometry_model(&graph, None);
        assert!(model.primitives.is_empty());
    }

    #[test]
    fn fully_positioned_part_yields_rect_primitive() {
        let mut graph = ModelGraph::new();
        graph.add_element(part_with_geometry("Enclosure", 0.0, 0.0, 600.0, 800.0));
        let model = to_geometry_model(&graph, None);
        assert_eq!(model.primitives.len(), 1);
        let GeometryPrimitive::Rect {
            x, y, width, height, label, element_id, ..
        } = &model.primitives[0];
        assert_eq!(*x, 0.0);
        assert_eq!(*y, 0.0);
        assert_eq!(*width, 600.0);
        assert_eq!(*height, 800.0);
        assert_eq!(label.as_deref(), Some("Enclosure"));
        assert!(element_id.is_some());
    }

    #[test]
    fn viewport_covers_all_primitives() {
        let mut graph = ModelGraph::new();
        graph.add_element(part_with_geometry("A", 0.0, 0.0, 100.0, 50.0));
        graph.add_element(part_with_geometry("B", 200.0, 100.0, 50.0, 50.0));
        let model = to_geometry_model(&graph, None);
        let vp = model.viewport.expect("viewport should exist for non-empty payload");
        assert_eq!(vp.min_x, 0.0);
        assert_eq!(vp.min_y, 0.0);
        assert_eq!(vp.max_x, 250.0); // 200 + 50
        assert_eq!(vp.max_y, 150.0); // 100 + 50
    }

    #[test]
    fn partial_geometry_is_filtered() {
        let mut graph = ModelGraph::new();
        // Has x/y but missing width/height — should be filtered out
        // of the typed payload.
        let partial = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("HalfPositioned")
            .with_prop("x", 10.0)
            .with_prop("y", 20.0);
        graph.add_element(partial);

        let model = to_geometry_model(&graph, None);
        assert!(
            model.primitives.is_empty(),
            "elements without size should be filtered, got {:?}",
            model.primitives
        );
    }

    #[test]
    fn primitive_carries_element_id_for_navigation() {
        let mut graph = ModelGraph::new();
        let id = graph.add_element(part_with_geometry("Tagged", 5.0, 5.0, 10.0, 10.0));
        let model = to_geometry_model(&graph, None);
        let GeometryPrimitive::Rect { element_id, .. } = &model.primitives[0];
        assert_eq!(element_id.as_deref(), Some(id.to_string().as_str()));
    }

    #[test]
    fn payload_serialises_with_shape_discriminator_and_camelcase_keys() {
        let mut graph = ModelGraph::new();
        graph.add_element(part_with_geometry("X", 0.0, 0.0, 1.0, 1.0));
        let model = to_geometry_model(&graph, None);
        let value = serde_json::to_value(&model).unwrap();
        let prim = &value.get("primitives").unwrap()[0];
        assert_eq!(prim.get("shape").and_then(|v| v.as_str()), Some("rect"));
        // FE mirror types (shared/api/model.ts) expect camelCase for
        // multi-word fields; verify the wire form here so the serde
        // attributes don't quietly regress.
        assert!(prim.get("cssClasses").is_some(), "expected camelCase cssClasses, got {:?}", prim);
        assert!(prim.get("elementId").is_some(), "expected camelCase elementId, got {:?}", prim);
        assert!(prim.get("css_classes").is_none(), "snake_case must not be present");
        assert!(prim.get("element_id").is_none(), "snake_case must not be present");
    }

    /// Mirrors the path real models take: `attribute x = 1.0;` parses
    /// into an AttributeUsage child of the part with `default` set.
    fn attr_child(name: &str, default: f64) -> Element {
        Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name(name)
            .with_prop("default", default)
    }

    #[test]
    fn attribute_usage_children_resolve_geometry() {
        let mut graph = ModelGraph::new();
        let part_id = graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage).with_name("BoxFromAttrs"),
        );
        graph.add_element(attr_child("x", 5.0).with_owner(part_id.clone()));
        graph.add_element(attr_child("y", 10.0).with_owner(part_id.clone()));
        graph.add_element(attr_child("width", 20.0).with_owner(part_id.clone()));
        graph.add_element(attr_child("height", 30.0).with_owner(part_id));

        let model = to_geometry_model(&graph, None);
        assert_eq!(model.primitives.len(), 1, "{:?}", model.primitives);
        let GeometryPrimitive::Rect {
            x, y, width, height, label, ..
        } = &model.primitives[0];
        assert_eq!((*x, *y, *width, *height), (5.0, 10.0, 20.0, 30.0));
        assert_eq!(label.as_deref(), Some("BoxFromAttrs"));
    }
}
