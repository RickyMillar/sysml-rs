//! Canonical JSON serialization for SysML v2 ModelGraph with stable ordering.
//!
//! This module provides deterministic JSON serialization for ModelGraph,
//! ensuring that the same graph always produces the same JSON output.
//! This is essential for:
//!
//! - Content-addressable storage
//! - Diffing and comparison
//! - Reproducible builds
//! - Testing

use serde::{Deserialize, Serialize};

use crate::{Element, ModelGraph, Relationship};

/// Error type for serialization/deserialization failures.
#[derive(Debug, thiserror::Error)]
pub enum CanonError {
    /// JSON serialization error.
    #[error("serialization error: {0}")]
    SerializeError(String),
    /// JSON deserialization error.
    #[error("deserialization error: {0}")]
    DeserializeError(String),
}

impl From<serde_json::Error> for CanonError {
    fn from(e: serde_json::Error) -> Self {
        CanonError::DeserializeError(e.to_string())
    }
}

/// Canonical representation of a ModelGraph for serialization.
///
/// Elements and relationships are stored in sorted order by ID string
/// to ensure deterministic output.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CanonicalGraph {
    /// Schema version for forward compatibility.
    #[serde(default = "default_version")]
    version: String,
    /// Elements sorted by ID.
    elements: Vec<Element>,
    /// Relationships sorted by ID.
    relationships: Vec<Relationship>,
}

fn default_version() -> String {
    "1.0".to_owned()
}

impl From<&ModelGraph> for CanonicalGraph {
    fn from(graph: &ModelGraph) -> Self {
        // Collect and sort elements by ID string
        let mut elements: Vec<Element> = graph.elements.values().cloned().collect();
        elements.sort_by(|a, b| a.id.as_str().cmp(&b.id.as_str()));

        // Collect and sort relationships by ID string
        let mut relationships: Vec<Relationship> = graph.relationships.values().cloned().collect();
        relationships.sort_by(|a, b| a.id.as_str().cmp(&b.id.as_str()));

        CanonicalGraph {
            version: "1.0".to_owned(),
            elements,
            relationships,
        }
    }
}

impl From<CanonicalGraph> for ModelGraph {
    fn from(canon: CanonicalGraph) -> Self {
        let mut graph = ModelGraph::new();

        for element in canon.elements {
            graph.add_element(element);
        }

        for relationship in canon.relationships {
            graph.add_relationship(relationship);
        }

        graph
    }
}

/// Serialize a ModelGraph to canonical JSON string.
///
/// The output is deterministic: the same graph will always produce
/// the same JSON string. Elements and relationships are sorted by
/// their ID strings.
#[allow(clippy::expect_used)] // Infallible: all ModelGraph fields are serializable
pub fn to_json_string(graph: &ModelGraph) -> String {
    let canon = CanonicalGraph::from(graph);
    serde_json::to_string(&canon).expect("ModelGraph should always be serializable")
}

/// Serialize a ModelGraph to pretty-printed canonical JSON string.
///
/// Like `to_json_string`, but with indentation for readability.
#[allow(clippy::expect_used)] // Infallible: all ModelGraph fields are serializable
pub fn to_json_string_pretty(graph: &ModelGraph) -> String {
    let canon = CanonicalGraph::from(graph);
    serde_json::to_string_pretty(&canon).expect("ModelGraph should always be serializable")
}

/// Deserialize a ModelGraph from a JSON string.
pub fn from_json_str(json: &str) -> Result<ModelGraph, CanonError> {
    let canon: CanonicalGraph = serde_json::from_str(json)?;
    Ok(ModelGraph::from(canon))
}

/// Serialize a ModelGraph to a JSON value.
#[allow(clippy::expect_used)] // Infallible: all ModelGraph fields are serializable
pub fn to_json_value(graph: &ModelGraph) -> serde_json::Value {
    let canon = CanonicalGraph::from(graph);
    serde_json::to_value(canon).expect("ModelGraph should always be serializable")
}

/// Deserialize a ModelGraph from a JSON value.
pub fn from_json_value(value: serde_json::Value) -> Result<ModelGraph, CanonError> {
    let canon: CanonicalGraph = serde_json::from_value(value)?;
    Ok(ModelGraph::from(canon))
}

/// Compute a hash of the canonical JSON representation.
///
/// This can be used for content-addressable storage or change detection.
/// Uses a simple FNV-1a hash for demonstration; in production, consider
/// using SHA-256 or similar.
pub fn content_hash(graph: &ModelGraph) -> u64 {
    let json = to_json_string(graph);
    // FNV-1a hash
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in json.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Element, ElementKind, Relationship, RelationshipKind, Value};

    fn create_test_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();

        let elem1 = Element::new_with_kind(ElementKind::Package).with_name("A");
        let elem2 = Element::new_with_kind(ElementKind::PartUsage).with_name("B");
        let id1 = graph.add_element(elem1);
        let id2 = graph.add_element(elem2);

        let rel = Relationship::new(RelationshipKind::Owning, id1, id2);
        graph.add_relationship(rel);

        graph
    }

    #[test]
    fn roundtrip() {
        let graph = create_test_graph();
        let json = to_json_string(&graph);
        let restored = from_json_str(&json).unwrap();

        assert_eq!(graph.element_count(), restored.element_count());
        assert_eq!(graph.relationship_count(), restored.relationship_count());
    }

    #[test]
    fn deterministic_output() {
        let graph = create_test_graph();

        let json1 = to_json_string(&graph);
        let json2 = to_json_string(&graph);

        assert_eq!(json1, json2, "Output should be deterministic");
    }

    #[test]
    fn deterministic_after_roundtrip() {
        let graph = create_test_graph();
        let json1 = to_json_string(&graph);

        let restored = from_json_str(&json1).unwrap();
        let json2 = to_json_string(&restored);

        assert_eq!(
            json1, json2,
            "Output should be deterministic after roundtrip"
        );
    }

    #[test]
    fn content_hash_deterministic() {
        let graph = create_test_graph();

        let hash1 = content_hash(&graph);
        let hash2 = content_hash(&graph);

        assert_eq!(hash1, hash2, "Hash should be deterministic");
    }

    #[test]
    fn content_hash_changes_with_content() {
        let mut graph = create_test_graph();
        let hash1 = content_hash(&graph);

        // Add another element
        let elem = Element::new_with_kind(ElementKind::RequirementUsage).with_name("C");
        graph.add_element(elem);
        let hash2 = content_hash(&graph);

        assert_ne!(hash1, hash2, "Hash should change with content");
    }

    #[test]
    fn empty_graph_roundtrip() {
        let graph = ModelGraph::new();
        let json = to_json_string(&graph);
        let restored = from_json_str(&json).unwrap();

        assert!(restored.is_empty());
    }

    #[test]
    fn json_contains_version() {
        let graph = ModelGraph::new();
        let json = to_json_string(&graph);

        assert!(json.contains("\"version\":\"1.0\""));
    }

    #[test]
    fn pretty_print() {
        let graph = create_test_graph();
        let json = to_json_string_pretty(&graph);

        assert!(json.contains('\n'), "Pretty output should have newlines");
    }

    #[test]
    fn to_and_from_value() {
        let graph = create_test_graph();
        let value = to_json_value(&graph);
        let restored = from_json_value(value).unwrap();

        assert_eq!(graph.element_count(), restored.element_count());
    }

    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        /// Strategy to pick a random ElementKind from a representative set.
        fn arb_element_kind() -> impl Strategy<Value = ElementKind> {
            prop_oneof![
                Just(ElementKind::Package),
                Just(ElementKind::PartUsage),
                Just(ElementKind::PartDefinition),
                Just(ElementKind::AttributeUsage),
                Just(ElementKind::RequirementUsage),
                Just(ElementKind::ConstraintUsage),
            ]
        }

        /// Strategy to generate a Value variant.
        fn arb_value() -> impl Strategy<Value = Value> {
            prop_oneof![
                any::<bool>().prop_map(Value::Bool),
                any::<i64>().prop_map(Value::Int),
                "[a-zA-Z0-9_ ]{0,20}".prop_map(|s| Value::String(s)),
            ]
        }

        /// Strategy to generate a single element with optional name and props.
        fn arb_element() -> impl Strategy<Value = Element> {
            (
                arb_element_kind(),
                prop::option::of("[a-zA-Z_][a-zA-Z0-9_]{0,15}"),
                prop::collection::vec(("[a-z_]{1,10}", arb_value()), 0..=3),
            )
                .prop_map(|(kind, name, props)| {
                    let mut elem = Element::new_with_kind(kind);
                    if let Some(n) = name {
                        elem.name = Some(n);
                    }
                    for (k, v) in props {
                        elem.props.insert(k.into(), v);
                    }
                    elem
                })
        }

        /// Strategy to generate a graph with N elements and optional relationships.
        fn arb_graph(max_elems: usize) -> impl Strategy<Value = ModelGraph> {
            prop::collection::vec(arb_element(), 1..=max_elems).prop_map(|elems| {
                let mut graph = ModelGraph::new();
                for elem in elems {
                    graph.add_element(elem);
                }
                graph
            })
        }

        proptest! {
            #[test]
            fn roundtrip_preserves_identity(graph in arb_graph(10)) {
                let json = to_json_string(&graph);
                let restored = from_json_str(&json).unwrap();

                prop_assert_eq!(graph.element_count(), restored.element_count());
                prop_assert_eq!(graph.relationship_count(), restored.relationship_count());

                // Every element ID in original exists in restored
                for (id, elem) in &graph.elements {
                    let restored_elem = restored.elements.get(id);
                    prop_assert!(restored_elem.is_some(), "missing element {}", id);
                    let r = restored_elem.unwrap();
                    prop_assert_eq!(&elem.kind, &r.kind);
                    prop_assert_eq!(&elem.name, &r.name);
                }
            }

            #[test]
            fn deterministic_output(graph in arb_graph(10)) {
                let json1 = to_json_string(&graph);
                let json2 = to_json_string(&graph);
                prop_assert_eq!(json1, json2);
            }

            #[test]
            fn value_fidelity(
                kind in arb_element_kind(),
                name in "[a-zA-Z_][a-zA-Z0-9_]{0,10}",
                bool_val in any::<bool>(),
                int_val in any::<i64>(),
                str_val in "[a-zA-Z0-9 ]{0,15}",
            ) {
                let elem = Element::new_with_kind(kind)
                    .with_name(name)
                    .with_prop("bool_prop", Value::Bool(bool_val))
                    .with_prop("int_prop", Value::Int(int_val))
                    .with_prop("str_prop", Value::String(str_val.clone()));

                let mut graph = ModelGraph::new();
                let id = graph.add_element(elem);

                let json = to_json_string(&graph);
                let restored = from_json_str(&json).unwrap();
                let r = restored.elements.get(&id).unwrap();

                prop_assert_eq!(r.get_prop("bool_prop"), Some(&Value::Bool(bool_val)));
                prop_assert_eq!(r.get_prop("int_prop"), Some(&Value::Int(int_val)));
                prop_assert_eq!(r.get_prop("str_prop"), Some(&Value::String(str_val)));
            }
        }
    }
}
