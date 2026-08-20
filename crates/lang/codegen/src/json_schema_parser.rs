//! Parser for JSON Schema files from the SysML v2 specification.
//!
//! This module extracts enum values from `*Kind.json` files.
//! (Relationship source/target constraints come from XMI — see
//! `xmi_relationship_parser` — which is the authoritative source.)

use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Parsed enum from JSON schema.
#[derive(Debug, Clone)]
pub struct JsonEnumInfo {
    pub name: String,
    pub values: Vec<String>,
}

/// Intermediate structure for parsing JSON schema.
#[derive(Debug, Deserialize)]
struct JsonSchema {
    title: Option<String>,
    #[serde(rename = "type")]
    type_field: Option<String>,
    #[serde(rename = "enum")]
    enum_values: Option<Vec<String>>,
}

/// Parse a JSON enum file (e.g., FeatureDirectionKind.json).
pub fn parse_enum_json(content: &str) -> Option<JsonEnumInfo> {
    let schema: JsonSchema = serde_json::from_str(content).ok()?;

    // Must be a string type with enum values
    if schema.type_field.as_deref() != Some("string") {
        return None;
    }

    let values = schema.enum_values?;
    let name = schema.title?;

    Some(JsonEnumInfo { name, values })
}

/// Parse all enum JSON files from a schema directory.
///
/// Discovers all *Kind.json files in the directory and parses them.
pub fn parse_all_enums_from_json(schema_dir: &Path) -> Vec<JsonEnumInfo> {
    // Discover all *Kind.json files in the directory
    let mut enums = Vec::new();

    if let Ok(entries) = fs::read_dir(schema_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                // Only process files ending with Kind.json
                if filename.ends_with("Kind.json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Some(enum_info) = parse_enum_json(&content) {
                            enums.push(enum_info);
                        }
                    }
                }
            }
        }
    }

    // Sort by name for consistent output
    enums.sort_by(|a, b| a.name.cmp(&b.name));
    enums
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_enum_json() {
        let content = r#"{
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://www.omg.org/spec/SysML/20250201/FeatureDirectionKind",
            "title": "FeatureDirectionKind",
            "type": "string",
            "enum": ["in", "inout", "out"]
        }"#;

        let result = parse_enum_json(content).unwrap();
        assert_eq!(result.name, "FeatureDirectionKind");
        assert_eq!(result.values, vec!["in", "inout", "out"]);
    }

}
