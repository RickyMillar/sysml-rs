//! Parser for XMI (UML metamodel) files to extract all class names.
//!
//! This module extracts all class names from the KerML and SysML XMI files
//! for cross-validation against TTL vocabulary types.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::xmi_relationship_parser::XmiParseError;

/// Helper to get attribute value from a start element.
fn get_attr(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == name)
        .map(|a| String::from_utf8_lossy(&a.value).to_string())
}

/// Parse a single XMI file to extract all class names.
fn parse_xmi_classes_from_content(content: &str) -> Result<HashSet<String>, XmiParseError> {
    let mut reader = Reader::from_str(content);
    let mut classes = HashSet::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local_name = e.local_name();
                if local_name.as_ref() == b"packagedElement" {
                    // Check if this is a uml:Class
                    if let Some(xmi_type) = get_attr(&e, b"xmi:type") {
                        if xmi_type == "uml:Class" {
                            if let Some(name) = get_attr(&e, b"name") {
                                // Filter out empty names and internal names
                                if !name.is_empty() && !name.starts_with('_') {
                                    classes.insert(name);
                                }
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(XmiParseError::XmlError(e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(classes)
}

/// Parse KerML.xmi and SysML.xmi to extract all class names.
///
/// Returns a set of all class names found in both XMI files.
pub fn parse_all_xmi_classes(
    kerml_xmi_path: &Path,
    sysml_xmi_path: &Path,
) -> Result<HashSet<String>, XmiParseError> {
    // Parse both files
    let kerml_content = fs::read_to_string(kerml_xmi_path)?;
    let sysml_content = fs::read_to_string(sysml_xmi_path)?;

    let kerml_classes = parse_xmi_classes_from_content(&kerml_content)?;
    let sysml_classes = parse_xmi_classes_from_content(&sysml_content)?;

    // Merge classes from both files
    let mut all_classes = kerml_classes;
    all_classes.extend(sysml_classes);

    Ok(all_classes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_xmi() {
        let xmi = r#"<?xml version='1.0' encoding='UTF-8'?>
<xmi:XMI xmlns:xmi="http://www.omg.org/spec/XMI/20161101" xmlns:uml="http://www.omg.org/spec/UML/20161101">
  <uml:Package xmi:id="Test" name="Test">
    <packagedElement xmi:id="Test-Element" xmi:type="uml:Class" name="Element"/>
    <packagedElement xmi:id="Test-Relationship" xmi:type="uml:Class" name="Relationship"/>
    <packagedElement xmi:id="Test-Feature" xmi:type="uml:Class" name="Feature"/>
    <packagedElement xmi:id="Test-Assoc" xmi:type="uml:Association" name="A_foo_bar"/>
  </uml:Package>
</xmi:XMI>"#;

        let classes = parse_xmi_classes_from_content(xmi).unwrap();

        assert_eq!(classes.len(), 3);
        assert!(classes.contains("Element"));
        assert!(classes.contains("Relationship"));
        assert!(classes.contains("Feature"));
        // Association should not be included
        assert!(!classes.contains("A_foo_bar"));
    }

    #[test]
    fn test_filters_empty_and_internal_names() {
        let xmi = r#"<?xml version='1.0' encoding='UTF-8'?>
<xmi:XMI xmlns:xmi="http://www.omg.org/spec/XMI/20161101" xmlns:uml="http://www.omg.org/spec/UML/20161101">
  <uml:Package xmi:id="Test" name="Test">
    <packagedElement xmi:id="Test-Element" xmi:type="uml:Class" name="Element"/>
    <packagedElement xmi:id="Test-Empty" xmi:type="uml:Class" name=""/>
    <packagedElement xmi:id="Test-Internal" xmi:type="uml:Class" name="_Internal"/>
  </uml:Package>
</xmi:XMI>"#;

        let classes = parse_xmi_classes_from_content(xmi).unwrap();

        assert_eq!(classes.len(), 1);
        assert!(classes.contains("Element"));
    }

    #[test]
    fn test_nested_classes() {
        let xmi = r#"<?xml version='1.0' encoding='UTF-8'?>
<xmi:XMI xmlns:xmi="http://www.omg.org/spec/XMI/20161101" xmlns:uml="http://www.omg.org/spec/UML/20161101">
  <uml:Package xmi:id="Test" name="Test">
    <packagedElement xmi:id="Test-Pkg" xmi:type="uml:Package" name="Core">
      <packagedElement xmi:id="Test-Element" xmi:type="uml:Class" name="Element"/>
      <packagedElement xmi:id="Test-Type" xmi:type="uml:Class" name="Type">
        <ownedAttribute xmi:id="Test-Type-attr" name="attr"/>
      </packagedElement>
    </packagedElement>
  </uml:Package>
</xmi:XMI>"#;

        let classes = parse_xmi_classes_from_content(xmi).unwrap();

        assert_eq!(classes.len(), 2);
        assert!(classes.contains("Element"));
        assert!(classes.contains("Type"));
    }
}
