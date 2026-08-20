//! Parser for XMI (UML metamodel) files to extract relationship source/target constraints.
//!
//! This module extracts relationship constraints by finding properties that redefine
//! `Relationship.source` and `Relationship.target` in the metamodel XMI files.
//!
//! ## XMI Structure
//!
//! The XMI files contain UML class definitions with owned attributes. Relationship types
//! redefine the `source` and `target` properties with more specific types:
//!
//! ```xml
//! <packagedElement xmi:id="Core-Types-Specialization" xmi:type="uml:Class" name="Specialization">
//!   <ownedAttribute xmi:id="..." name="specific">
//!     <redefinedProperty xmi:idref="Root-Elements-Relationship-source"/>
//!     <type xmi:idref="Core-Types-Type"/>
//!   </ownedAttribute>
//!   <ownedAttribute xmi:id="..." name="general">
//!     <redefinedProperty xmi:idref="Root-Elements-Relationship-target"/>
//!     <type xmi:idref="Core-Types-Type"/>
//!   </ownedAttribute>
//! </packagedElement>
//! ```

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Relationship constraint extracted from XMI.
#[derive(Debug, Clone)]
pub struct XmiRelationshipConstraint {
    /// The relationship type name (e.g., "Specialization", "Connector")
    pub class_name: String,
    /// The constrained source type (e.g., "Type", "Feature")
    pub source_type: String,
    /// The constrained target type (e.g., "Type", "Feature")
    pub target_type: String,
    /// True if source_type was explicitly found in XMI
    pub source_from_xmi: bool,
    /// True if target_type was explicitly found in XMI
    pub target_from_xmi: bool,
}

/// Error type for XMI parsing.
#[derive(Debug, thiserror::Error)]
pub enum XmiParseError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("XML error: {0}")]
    XmlError(#[from] quick_xml::Error),
    #[error("Missing attribute: {0}")]
    MissingAttribute(String),
}

/// Internal state while parsing XMI.
struct XmiParseState {
    /// Map from xmi:id to class name for resolving type references
    id_to_name: HashMap<String, String>,
    /// Current class context while parsing
    current_class: Option<CurrentClass>,
    /// Collected relationship constraints (class_name -> constraint)
    constraints: HashMap<String, PartialConstraint>,
    /// Depth tracking for nested elements
    depth: usize,
    /// The depth at which we entered the current class
    class_depth: usize,
    /// Current owned attribute being parsed
    current_attribute: Option<CurrentAttribute>,
    /// The depth at which we entered the current owned attribute
    attribute_depth: usize,
}

#[derive(Clone)]
struct CurrentClass {
    #[allow(dead_code)]
    id: String,
    name: String,
}

#[derive(Default)]
struct CurrentAttribute {
    has_source_redef: bool,
    has_target_redef: bool,
    type_ref: Option<String>,
}

#[derive(Default)]
struct PartialConstraint {
    source_type_ref: Option<String>,
    target_type_ref: Option<String>,
}

impl XmiParseState {
    fn new() -> Self {
        Self {
            id_to_name: HashMap::new(),
            current_class: None,
            constraints: HashMap::new(),
            depth: 0,
            class_depth: 0,
            current_attribute: None,
            attribute_depth: 0,
        }
    }
}

/// Helper to get attribute value from a start element.
fn get_attr(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == name)
        .map(|a| String::from_utf8_lossy(&a.value).to_string())
}

/// Parse a single XMI file to extract relationship constraints.
fn parse_xmi_file(content: &str) -> Result<XmiParseState, XmiParseError> {
    let mut reader = Reader::from_str(content);
    let mut state = XmiParseState::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                state.depth += 1;
                handle_start_element(&e, &mut state);
            }
            Ok(Event::Empty(e)) => {
                // Empty elements like <type xmi:idref="..."/> or <redefinedProperty .../>
                handle_empty_element(&e, &mut state);
            }
            Ok(Event::End(_)) => {
                handle_end_element(&mut state);
                state.depth -= 1;
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(XmiParseError::XmlError(e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(state)
}

fn handle_start_element(e: &BytesStart<'_>, state: &mut XmiParseState) {
    let local_name = e.local_name();
    let name_bytes = local_name.as_ref();

    match name_bytes {
        b"packagedElement" => {
            // Check if this is a uml:Class
            if let Some(xmi_type) = get_attr(e, b"xmi:type") {
                if xmi_type == "uml:Class" {
                    if let (Some(id), Some(name)) = (get_attr(e, b"xmi:id"), get_attr(e, b"name")) {
                        // Record ID -> name mapping
                        state.id_to_name.insert(id.clone(), name.clone());
                        // Enter class context
                        state.current_class = Some(CurrentClass { id, name });
                        state.class_depth = state.depth;
                    }
                }
            }
        }
        b"ownedAttribute" => {
            // Only process if we're in a class context
            if state.current_class.is_some() {
                state.current_attribute = Some(CurrentAttribute::default());
                state.attribute_depth = state.depth;
            }
        }
        _ => {}
    }
}

fn handle_empty_element(e: &BytesStart<'_>, state: &mut XmiParseState) {
    let local_name = e.local_name();
    let name_bytes = local_name.as_ref();

    // Handle empty packagedElement (class with no content)
    if name_bytes == b"packagedElement" {
        if let Some(xmi_type) = get_attr(e, b"xmi:type") {
            if xmi_type == "uml:Class" {
                if let (Some(id), Some(name)) = (get_attr(e, b"xmi:id"), get_attr(e, b"name")) {
                    state.id_to_name.insert(id, name);
                }
            }
        }
        return;
    }

    // Only process if we're inside an ownedAttribute
    if state.current_attribute.is_none() {
        return;
    }

    match name_bytes {
        b"redefinedProperty" => {
            if let Some(idref) = get_attr(e, b"xmi:idref") {
                if let Some(attr) = &mut state.current_attribute {
                    if idref == "Root-Elements-Relationship-source" {
                        attr.has_source_redef = true;
                    } else if idref == "Root-Elements-Relationship-target" {
                        attr.has_target_redef = true;
                    }
                }
            }
        }
        b"type" => {
            if let Some(idref) = get_attr(e, b"xmi:idref") {
                if let Some(attr) = &mut state.current_attribute {
                    attr.type_ref = Some(idref);
                }
            }
        }
        _ => {}
    }
}

fn handle_end_element(state: &mut XmiParseState) {
    // Check if we're leaving an ownedAttribute
    if state.current_attribute.is_some() && state.depth == state.attribute_depth {
        if let Some(attr) = state.current_attribute.take() {
            if let Some(ref class) = state.current_class {
                // If this attribute redefines source or target, record it
                if attr.has_source_redef || attr.has_target_redef {
                    let constraint = state
                        .constraints
                        .entry(class.name.clone())
                        .or_default();

                    if attr.has_source_redef {
                        constraint.source_type_ref = attr.type_ref.clone();
                    }
                    if attr.has_target_redef {
                        constraint.target_type_ref = attr.type_ref;
                    }
                }
            }
        }
    }

    // Check if we're leaving a class
    if state.current_class.is_some() && state.depth == state.class_depth {
        state.current_class = None;
    }
}

/// Extract the type name from an xmi:idref.
///
/// Format: "Core-Types-Type" -> "Type"
/// The name is the last segment after the final hyphen-separated component.
fn extract_type_name_from_id(id: &str) -> String {
    // Find the last hyphen-separated component
    id.rsplit('-').next().unwrap_or(id).to_owned()
}

/// Parse KerML.xmi and SysML.xmi to extract relationship constraints.
///
/// Returns a map from relationship type name to its constraint.
pub fn parse_relationship_constraints(
    kerml_xmi_path: &Path,
    sysml_xmi_path: &Path,
) -> Result<HashMap<String, XmiRelationshipConstraint>, XmiParseError> {
    // Parse both files
    let kerml_content = fs::read_to_string(kerml_xmi_path)?;
    let sysml_content = fs::read_to_string(sysml_xmi_path)?;

    let kerml_state = parse_xmi_file(&kerml_content)?;
    let sysml_state = parse_xmi_file(&sysml_content)?;

    // Merge ID maps (SysML may reference KerML types)
    let mut id_to_name = kerml_state.id_to_name.clone();
    id_to_name.extend(sysml_state.id_to_name.clone());

    // Merge constraints
    let mut partial_constraints = kerml_state.constraints;
    partial_constraints.extend(sysml_state.constraints);

    // Resolve type references to names
    let mut result = HashMap::new();

    for (class_name, partial) in partial_constraints {
        let source_type = partial
            .source_type_ref
            .as_ref()
            .map(|r| resolve_type_ref(r, &id_to_name))
            .unwrap_or_else(|| "Element".to_owned());

        let target_type = partial
            .target_type_ref
            .as_ref()
            .map(|r| resolve_type_ref(r, &id_to_name))
            .unwrap_or_else(|| "Element".to_owned());

        result.insert(
            class_name.clone(),
            XmiRelationshipConstraint {
                class_name,
                source_type,
                target_type,
                source_from_xmi: partial.source_type_ref.is_some(),
                target_from_xmi: partial.target_type_ref.is_some(),
            },
        );
    }

    Ok(result)
}

/// Resolve a type reference to its name.
fn resolve_type_ref(type_ref: &str, id_to_name: &HashMap<String, String>) -> String {
    // First try direct lookup
    if let Some(name) = id_to_name.get(type_ref) {
        return name.clone();
    }

    // Fall back to extracting from the ID format
    extract_type_name_from_id(type_ref)
}

/// Coverage report for relationship constraints.
#[derive(Debug)]
pub struct CoverageReport {
    /// Total number of relationship types
    pub total: usize,
    /// Number of constraints from XMI
    pub from_xmi: usize,
    /// Number of constraints from fallback
    pub from_fallback: usize,
    /// Relationship types with no constraint source
    pub missing: Vec<String>,
}

/// Validate that every Relationship subtype has constraints from XMI or fallback.
pub fn validate_relationship_coverage(
    relationship_type_names: &[&str],
    xmi_constraints: &HashMap<String, XmiRelationshipConstraint>,
    fallback_constraint_names: &[String],
) -> CoverageReport {
    let mut missing = Vec::new();
    let mut from_xmi = 0;
    let mut from_fallback = 0;

    for rel_type in relationship_type_names {
        if xmi_constraints.contains_key(*rel_type) {
            from_xmi += 1;
        } else if fallback_constraint_names.iter().any(|s| s == rel_type) {
            from_fallback += 1;
        } else {
            missing.push(rel_type.to_string());
        }
    }

    CoverageReport {
        total: relationship_type_names.len(),
        from_xmi,
        from_fallback,
        missing,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_type_name_from_id() {
        assert_eq!(extract_type_name_from_id("Core-Types-Type"), "Type");
        assert_eq!(
            extract_type_name_from_id("Core-Features-Feature"),
            "Feature"
        );
        assert_eq!(
            extract_type_name_from_id("Root-Elements-Relationship"),
            "Relationship"
        );
        assert_eq!(extract_type_name_from_id("SimpleType"), "SimpleType");
    }

    #[test]
    fn test_parse_simple_xmi() {
        let xmi = r#"<?xml version='1.0' encoding='UTF-8'?>
<xmi:XMI xmlns:xmi="http://www.omg.org/spec/XMI/20161101" xmlns:uml="http://www.omg.org/spec/UML/20161101">
  <uml:Package xmi:id="Test" name="Test">
    <packagedElement xmi:id="Test-Specialization" xmi:type="uml:Class" name="Specialization">
      <ownedAttribute xmi:id="Test-Specialization-specific" name="specific">
        <redefinedProperty xmi:idref="Root-Elements-Relationship-source"/>
        <type xmi:idref="Test-Type"/>
      </ownedAttribute>
      <ownedAttribute xmi:id="Test-Specialization-general" name="general">
        <redefinedProperty xmi:idref="Root-Elements-Relationship-target"/>
        <type xmi:idref="Test-Type"/>
      </ownedAttribute>
    </packagedElement>
    <packagedElement xmi:id="Test-Type" xmi:type="uml:Class" name="Type"/>
  </uml:Package>
</xmi:XMI>"#;

        let state = parse_xmi_file(xmi).unwrap();

        // Check ID to name mapping
        assert_eq!(
            state.id_to_name.get("Test-Specialization"),
            Some(&"Specialization".to_owned())
        );
        assert_eq!(state.id_to_name.get("Test-Type"), Some(&"Type".to_owned()));

        // Check constraints
        assert!(state.constraints.contains_key("Specialization"));
        let constraint = state.constraints.get("Specialization").unwrap();
        assert_eq!(constraint.source_type_ref, Some("Test-Type".to_owned()));
        assert_eq!(constraint.target_type_ref, Some("Test-Type".to_owned()));
    }

    #[test]
    fn test_parse_connector_pattern() {
        // Test the Connector pattern where source/target are Features
        let xmi = r#"<?xml version='1.0' encoding='UTF-8'?>
<xmi:XMI xmlns:xmi="http://www.omg.org/spec/XMI/20161101" xmlns:uml="http://www.omg.org/spec/UML/20161101">
  <uml:Package xmi:id="Test" name="Test">
    <packagedElement xmi:id="Test-Connector" xmi:type="uml:Class" name="Connector">
      <ownedAttribute xmi:id="Test-Connector-sourceFeature" name="sourceFeature">
        <redefinedProperty xmi:idref="Root-Elements-Relationship-source"/>
        <type xmi:idref="Test-Feature"/>
      </ownedAttribute>
      <ownedAttribute xmi:id="Test-Connector-targetFeature" name="targetFeature">
        <redefinedProperty xmi:idref="Root-Elements-Relationship-target"/>
        <type xmi:idref="Test-Feature"/>
      </ownedAttribute>
    </packagedElement>
    <packagedElement xmi:id="Test-Feature" xmi:type="uml:Class" name="Feature"/>
  </uml:Package>
</xmi:XMI>"#;

        let state = parse_xmi_file(xmi).unwrap();

        let constraint = state.constraints.get("Connector").unwrap();
        assert_eq!(constraint.source_type_ref, Some("Test-Feature".to_owned()));
        assert_eq!(constraint.target_type_ref, Some("Test-Feature".to_owned()));
    }

    #[test]
    fn test_coverage_report() {
        let mut xmi_constraints = HashMap::new();
        xmi_constraints.insert(
            "Specialization".to_owned(),
            XmiRelationshipConstraint {
                class_name: "Specialization".to_owned(),
                source_type: "Type".to_owned(),
                target_type: "Type".to_owned(),
                source_from_xmi: true,
                target_from_xmi: true,
            },
        );

        let relationship_types = vec!["Relationship", "Specialization", "Connector"];
        let fallback_names = vec!["Connector".to_owned()];

        let report =
            validate_relationship_coverage(&relationship_types, &xmi_constraints, &fallback_names);

        assert_eq!(report.total, 3);
        assert_eq!(report.from_xmi, 1);
        assert_eq!(report.from_fallback, 1);
        assert_eq!(report.missing, vec!["Relationship"]);
    }
}
