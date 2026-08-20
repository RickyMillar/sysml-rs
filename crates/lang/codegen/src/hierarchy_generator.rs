//! Generator for ElementKind type hierarchy methods.
//!
//! This module generates methods on `ElementKind` that expose type hierarchy information:
//! - `supertypes()` - All supertypes in inheritance order
//! - `direct_supertypes()` - Immediate parent types only
//! - `is_subtype_of()` - Subtype checking
//! - Category predicates (`is_definition()`, `is_usage()`, etc.)
//! - Definition↔Usage mappings

use crate::ttl_parser::TypeInfo;
use std::collections::{HashMap, HashSet};

/// A queryable type hierarchy built from KerML and SysML vocabularies.
///
/// Provides efficient lookups for type relationships:
/// root types, subtypes, supertypes, and "has subtypes" checks.
#[derive(Debug)]
pub struct TypeHierarchy {
    /// Map from type name to all transitive supertypes.
    supertypes: HashMap<String, Vec<String>>,
    /// Map from type name to direct parent types.
    direct_supertypes: HashMap<String, Vec<String>>,
}

impl TypeHierarchy {
    /// Build a TypeHierarchy from KerML and SysML type info.
    pub fn new(kerml_types: &[TypeInfo], sysml_types: &[TypeInfo]) -> Self {
        let mut direct_supertypes: HashMap<String, Vec<String>> = HashMap::new();

        for type_info in kerml_types.iter().chain(sysml_types.iter()) {
            direct_supertypes.insert(type_info.name.clone(), type_info.supertypes.clone());
        }

        let mut supertypes: HashMap<String, Vec<String>> = HashMap::new();

        for type_info in kerml_types.iter().chain(sysml_types.iter()) {
            let mut all_supertypes = Vec::new();
            let mut visited = HashSet::new();
            collect_supertypes(
                &type_info.name,
                &direct_supertypes,
                &mut all_supertypes,
                &mut visited,
            );
            supertypes.insert(type_info.name.clone(), all_supertypes);
        }

        TypeHierarchy {
            supertypes,
            direct_supertypes,
        }
    }

    /// Returns true if the type has no parent in the hierarchy (is a root type).
    pub fn is_root_type(&self, type_name: &str) -> bool {
        self.direct_supertypes
            .get(type_name)
            .is_some_and(|parents| parents.is_empty())
    }

    /// Returns true if any other type in the hierarchy lists this type as a supertype.
    ///
    /// This identifies "base types" whose semantic rules should cascade to subtypes
    /// in the validation dispatcher.
    pub fn has_subtypes(&self, type_name: &str) -> bool {
        self.supertypes
            .values()
            .any(|supers| supers.iter().any(|s| s == type_name))
    }

    /// Returns the transitive supertypes of a type.
    pub fn get_supertypes(&self, type_name: &str) -> Option<&[String]> {
        self.supertypes.get(type_name).map(|v| v.as_slice())
    }

    /// Returns the direct (declared) supertypes of a type.
    pub fn get_direct_supertypes(&self, type_name: &str) -> Option<&[String]> {
        self.direct_supertypes.get(type_name).map(|v| v.as_slice())
    }

    /// Returns true if the type exists in the hierarchy at all.
    pub fn contains_type(&self, type_name: &str) -> bool {
        self.direct_supertypes.contains_key(type_name)
    }

    /// Strict subtype check: `type_name` is a (transitive) subtype of
    /// `ancestor`, NOT including itself.
    pub fn is_subtype_of(&self, type_name: &str, ancestor: &str) -> bool {
        self.supertypes
            .get(type_name)
            .is_some_and(|supers| supers.iter().any(|s| s == ancestor))
    }

    /// All relationship types: `Relationship` itself plus every transitive
    /// subtype. Deduplicated and sorted for deterministic output. This is THE
    /// definition of "relationship type" — generators, coverage validators,
    /// and tests must all use it rather than re-filtering by hand.
    pub fn relationship_type_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .direct_supertypes
            .keys()
            .map(String::as_str)
            .filter(|n| *n == "Relationship" || self.is_subtype_of(n, "Relationship"))
            .collect();
        names.sort_unstable();
        names
    }
}

/// Generate all hierarchy-related methods for the ElementKind enum.
///
/// This generates:
/// - `supertypes()` - Returns all supertypes (direct + transitive)
/// - `direct_supertypes()` - Returns only immediate parents
/// - `is_subtype_of()` - Checks if type is a subtype of another
/// - Category predicates: `is_definition()`, `is_usage()`, `is_relationship()`, etc.
/// - `corresponding_usage()` / `corresponding_definition()` - Definition↔Usage pairs
pub fn generate_hierarchy_methods(kerml_types: &[TypeInfo], sysml_types: &[TypeInfo]) -> String {
    let mut output = String::new();

    output.push_str("\n// === Type Hierarchy Methods ===\n\n");

    // Build the type hierarchy (TypeHierarchy is the one home for this)
    let hierarchy = TypeHierarchy::new(kerml_types, sysml_types).supertypes;

    // Get all type names (deduplicated)
    let all_types: Vec<&str> = get_all_type_names(kerml_types, sysml_types);

    // Generate supertypes method
    output.push_str(&generate_supertypes_method(&all_types, &hierarchy));

    // Generate direct_supertypes method
    output.push_str(&generate_direct_supertypes_method(
        &all_types,
        kerml_types,
        sysml_types,
    ));

    // Generate is_subtype_of method
    output.push_str(&generate_is_subtype_of_method());

    // Generate category predicates
    output.push_str(&generate_category_predicates(&all_types, &hierarchy));

    // Generate definition/usage mappings
    output.push_str(&generate_def_usage_mappings(&all_types));

    // Generate additional category predicates
    output.push_str(&generate_extra_predicates(&all_types, &hierarchy));

    // Generate syntax keyword mapping
    output.push_str(&generate_syntax_keyword(&all_types, &hierarchy));

    // Generate text template method
    output.push_str(&generate_text_template());

    // Generate containment validation
    output.push_str(&generate_can_own_method(&hierarchy));

    output
}

/// Recursively collect all supertypes.
fn collect_supertypes(
    type_name: &str,
    direct_map: &HashMap<String, Vec<String>>,
    result: &mut Vec<String>,
    visited: &mut HashSet<String>,
) {
    if visited.contains(type_name) {
        return;
    }
    visited.insert(type_name.to_owned());

    if let Some(direct) = direct_map.get(type_name) {
        for supertype in direct {
            if !result.contains(supertype) {
                result.push(supertype.clone());
            }
            collect_supertypes(supertype, direct_map, result, visited);
        }
    }
}

/// Get all unique type names from both vocabularies.
fn get_all_type_names<'a>(
    kerml_types: &'a [TypeInfo],
    sysml_types: &'a [TypeInfo],
) -> Vec<&'a str> {
    let mut names: Vec<&str> = kerml_types.iter().map(|t| t.name.as_str()).collect();
    for t in sysml_types {
        if !names.contains(&t.name.as_str()) {
            names.push(&t.name);
        }
    }
    names.sort();
    names
}

/// Generate the `supertypes()` method.
fn generate_supertypes_method(
    all_types: &[&str],
    hierarchy: &HashMap<String, Vec<String>>,
) -> String {
    let mut output = String::new();

    output.push_str("impl ElementKind {\n");
    output.push_str("    /// Returns all supertypes (direct + transitive) in inheritance order.\n");
    output.push_str("    ///\n");
    output.push_str("    /// The supertypes are ordered from most specific to most general.\n");
    output.push_str("    /// For example, `PartUsage.supertypes()` returns `[ItemUsage, OccurrenceUsage, Usage, ...]`.\n");
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str("    /// let supertypes = ElementKind::Feature.supertypes();\n");
    output.push_str("    /// assert!(supertypes.contains(&ElementKind::Type));\n");
    output.push_str("    /// ```\n");
    output.push_str("    pub fn supertypes(&self) -> &'static [ElementKind] {\n");
    output.push_str("        match self {\n");

    for type_name in all_types {
        let supertypes = hierarchy.get(*type_name).cloned().unwrap_or_default();
        // Filter to only include types that exist in our type list
        let valid_supertypes: Vec<&str> = supertypes
            .iter()
            .filter(|s| all_types.contains(&s.as_str()))
            .map(|s| s.as_str())
            .collect();

        if valid_supertypes.is_empty() {
            output.push_str(&format!("            ElementKind::{} => &[],\n", type_name));
        } else {
            output.push_str(&format!("            ElementKind::{} => &[\n", type_name));
            for supertype in &valid_supertypes {
                output.push_str(&format!("                ElementKind::{},\n", supertype));
            }
            output.push_str("            ],\n");
        }
    }

    output.push_str("        }\n");
    output.push_str("    }\n\n");
    output.push_str("}\n\n");

    output
}

/// Generate the `direct_supertypes()` method.
fn generate_direct_supertypes_method(
    all_types: &[&str],
    kerml_types: &[TypeInfo],
    sysml_types: &[TypeInfo],
) -> String {
    let mut output = String::new();

    // Build direct supertypes map
    let mut direct_map: HashMap<&str, Vec<&str>> = HashMap::new();
    for type_info in kerml_types.iter().chain(sysml_types.iter()) {
        let valid_supertypes: Vec<&str> = type_info
            .supertypes
            .iter()
            .filter(|s| all_types.contains(&s.as_str()))
            .map(|s| s.as_str())
            .collect();
        direct_map.insert(&type_info.name, valid_supertypes);
    }

    output.push_str("impl ElementKind {\n");
    output.push_str(
        "    /// Returns the direct supertypes (immediate parents) of this element kind.\n",
    );
    output.push_str("    ///\n");
    output.push_str(
        "    /// Unlike `supertypes()`, this only returns immediate parents, not the full\n",
    );
    output.push_str("    /// transitive closure.\n");
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str("    /// let direct = ElementKind::Feature.direct_supertypes();\n");
    output.push_str("    /// assert!(direct.contains(&ElementKind::Type));\n");
    output.push_str("    /// ```\n");
    output.push_str("    pub fn direct_supertypes(&self) -> &'static [ElementKind] {\n");
    output.push_str("        match self {\n");

    for type_name in all_types {
        let supertypes = direct_map.get(type_name).cloned().unwrap_or_default();

        if supertypes.is_empty() {
            output.push_str(&format!("            ElementKind::{} => &[],\n", type_name));
        } else {
            output.push_str(&format!("            ElementKind::{} => &[\n", type_name));
            for supertype in &supertypes {
                output.push_str(&format!("                ElementKind::{},\n", supertype));
            }
            output.push_str("            ],\n");
        }
    }

    output.push_str("        }\n");
    output.push_str("    }\n\n");
    output.push_str("}\n\n");

    output
}

/// Generate the `is_subtype_of()` method.
fn generate_is_subtype_of_method() -> String {
    let mut output = String::new();

    output.push_str("impl ElementKind {\n");
    output
        .push_str("    /// Check if this type is a subtype of another (including transitively).\n");
    output.push_str("    ///\n");
    output.push_str(
        "    /// Returns `true` if `other` appears anywhere in this type's supertype chain.\n",
    );
    output.push_str("    /// Note: A type is NOT considered a subtype of itself.\n");
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str("    /// assert!(ElementKind::Feature.is_subtype_of(ElementKind::Type));\n");
    output.push_str("    /// assert!(ElementKind::Feature.is_subtype_of(ElementKind::Element));\n");
    output
        .push_str("    /// assert!(!ElementKind::Feature.is_subtype_of(ElementKind::Feature));\n");
    output
        .push_str("    /// assert!(!ElementKind::Element.is_subtype_of(ElementKind::Feature));\n");
    output.push_str("    /// ```\n");
    output.push_str("    pub fn is_subtype_of(&self, other: ElementKind) -> bool {\n");
    output.push_str("        self.supertypes().contains(&other)\n");
    output.push_str("    }\n\n");
    output.push_str("}\n\n");

    output
}

/// Generate category predicates.
fn generate_category_predicates(
    all_types: &[&str],
    hierarchy: &HashMap<String, Vec<String>>,
) -> String {
    let mut output = String::new();

    // Identify types by category
    let definitions: Vec<&str> = all_types
        .iter()
        .filter(|t| t.ends_with("Definition"))
        .copied()
        .collect();

    let usages: Vec<&str> = all_types
        .iter()
        .filter(|t| t.ends_with("Usage"))
        .copied()
        .collect();

    let relationships: Vec<&str> = all_types
        .iter()
        .filter(|t| {
            **t == "Relationship"
                || hierarchy
                    .get(**t)
                    .is_some_and(|s| s.contains(&"Relationship".to_owned()))
        })
        .copied()
        .collect();

    let classifiers: Vec<&str> = all_types
        .iter()
        .filter(|t| {
            **t == "Classifier"
                || hierarchy
                    .get(**t)
                    .is_some_and(|s| s.contains(&"Classifier".to_owned()))
        })
        .copied()
        .collect();

    let features: Vec<&str> = all_types
        .iter()
        .filter(|t| {
            **t == "Feature"
                || hierarchy
                    .get(**t)
                    .is_some_and(|s| s.contains(&"Feature".to_owned()))
        })
        .copied()
        .collect();

    output.push_str("impl ElementKind {\n");

    // is_definition()
    output.push_str("    /// Returns `true` if this is a Definition type (e.g., PartDefinition, ActionDefinition).\n");
    output.push_str("    ///\n");
    output.push_str(
        "    /// Definition types define reusable element templates that can be instantiated\n",
    );
    output.push_str("    /// as Usage types.\n");
    output.push_str("    pub const fn is_definition(&self) -> bool {\n");
    output.push_str("        matches!(self,\n");
    for (i, def_type) in definitions.iter().enumerate() {
        if i == definitions.len() - 1 {
            output.push_str(&format!("            ElementKind::{}\n", def_type));
        } else {
            output.push_str(&format!("            ElementKind::{} |\n", def_type));
        }
    }
    output.push_str("        )\n");
    output.push_str("    }\n\n");

    // is_usage()
    output.push_str(
        "    /// Returns `true` if this is a Usage type (e.g., PartUsage, ActionUsage).\n",
    );
    output.push_str("    ///\n");
    output.push_str("    /// Usage types are instantiations or references to Definition types.\n");
    output.push_str("    pub const fn is_usage(&self) -> bool {\n");
    output.push_str("        matches!(self,\n");
    for (i, usage_type) in usages.iter().enumerate() {
        if i == usages.len() - 1 {
            output.push_str(&format!("            ElementKind::{}\n", usage_type));
        } else {
            output.push_str(&format!("            ElementKind::{} |\n", usage_type));
        }
    }
    output.push_str("        )\n");
    output.push_str("    }\n\n");

    // is_relationship()
    output.push_str(
        "    /// Returns `true` if this is a Relationship type or any of its subtypes.\n",
    );
    output.push_str("    ///\n");
    output.push_str("    /// Relationship types connect elements together (e.g., Specialization, FeatureTyping).\n");
    output.push_str("    pub const fn is_relationship(&self) -> bool {\n");
    output.push_str("        matches!(self,\n");
    for (i, rel_type) in relationships.iter().enumerate() {
        if i == relationships.len() - 1 {
            output.push_str(&format!("            ElementKind::{}\n", rel_type));
        } else {
            output.push_str(&format!("            ElementKind::{} |\n", rel_type));
        }
    }
    output.push_str("        )\n");
    output.push_str("    }\n\n");

    // is_classifier()
    output
        .push_str("    /// Returns `true` if this is a Classifier type or any of its subtypes.\n");
    output.push_str("    ///\n");
    output.push_str(
        "    /// Classifiers are Types that classify their instances (e.g., Class, DataType).\n",
    );
    output.push_str("    pub const fn is_classifier(&self) -> bool {\n");
    output.push_str("        matches!(self,\n");
    for (i, cls_type) in classifiers.iter().enumerate() {
        if i == classifiers.len() - 1 {
            output.push_str(&format!("            ElementKind::{}\n", cls_type));
        } else {
            output.push_str(&format!("            ElementKind::{} |\n", cls_type));
        }
    }
    output.push_str("        )\n");
    output.push_str("    }\n\n");

    // is_feature()
    output.push_str("    /// Returns `true` if this is a Feature type or any of its subtypes.\n");
    output.push_str("    ///\n");
    output.push_str("    /// Features are typed structural and/or behavioral elements.\n");
    output.push_str("    pub const fn is_feature(&self) -> bool {\n");
    output.push_str("        matches!(self,\n");
    for (i, feat_type) in features.iter().enumerate() {
        if i == features.len() - 1 {
            output.push_str(&format!("            ElementKind::{}\n", feat_type));
        } else {
            output.push_str(&format!("            ElementKind::{} |\n", feat_type));
        }
    }
    output.push_str("        )\n");
    output.push_str("    }\n\n");

    output.push_str("}\n\n");

    output
}

/// Generate Definition↔Usage mapping methods.
#[allow(clippy::expect_used)] // Invariants: strip_suffix after ends_with, find after contains
fn generate_def_usage_mappings(all_types: &[&str]) -> String {
    let mut output = String::new();

    // Find matching pairs
    let mut pairs: Vec<(&str, &str)> = Vec::new();

    for def_type in all_types.iter().filter(|t| t.ends_with("Definition")) {
        // Try to find matching Usage type
        let base_name = def_type
            .strip_suffix("Definition")
            .expect("invariant: ends_with checked above");
        let usage_name = format!("{}Usage", base_name);
        if all_types.contains(&usage_name.as_str()) {
            pairs.push((
                def_type,
                all_types
                    .iter()
                    .find(|&&t| t == usage_name)
                    .expect("invariant: contains checked above"),
            ));
        }
    }

    output.push_str("impl ElementKind {\n");

    // corresponding_usage()
    output.push_str("    /// For Definition types, returns the corresponding Usage type.\n");
    output.push_str("    ///\n");
    output.push_str(
        "    /// For example, `PartDefinition.corresponding_usage()` returns `Some(PartUsage)`.\n",
    );
    output.push_str("    /// Returns `None` for non-Definition types or Definitions without a matching Usage.\n");
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str("    /// assert_eq!(\n");
    output.push_str("    ///     ElementKind::PartDefinition.corresponding_usage(),\n");
    output.push_str("    ///     Some(ElementKind::PartUsage)\n");
    output.push_str("    /// );\n");
    output.push_str("    /// assert_eq!(ElementKind::Element.corresponding_usage(), None);\n");
    output.push_str("    /// ```\n");
    output.push_str("    pub const fn corresponding_usage(&self) -> Option<ElementKind> {\n");
    output.push_str("        match self {\n");
    for (def_type, usage_type) in &pairs {
        output.push_str(&format!(
            "            ElementKind::{} => Some(ElementKind::{}),\n",
            def_type, usage_type
        ));
    }
    output.push_str("            _ => None,\n");
    output.push_str("        }\n");
    output.push_str("    }\n\n");

    // corresponding_definition()
    output.push_str("    /// For Usage types, returns the corresponding Definition type.\n");
    output.push_str("    ///\n");
    output.push_str("    /// For example, `PartUsage.corresponding_definition()` returns `Some(PartDefinition)`.\n");
    output.push_str(
        "    /// Returns `None` for non-Usage types or Usages without a matching Definition.\n",
    );
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str("    /// assert_eq!(\n");
    output.push_str("    ///     ElementKind::PartUsage.corresponding_definition(),\n");
    output.push_str("    ///     Some(ElementKind::PartDefinition)\n");
    output.push_str("    /// );\n");
    output.push_str("    /// assert_eq!(ElementKind::Element.corresponding_definition(), None);\n");
    output.push_str("    /// ```\n");
    output.push_str("    pub const fn corresponding_definition(&self) -> Option<ElementKind> {\n");
    output.push_str("        match self {\n");
    for (def_type, usage_type) in &pairs {
        output.push_str(&format!(
            "            ElementKind::{} => Some(ElementKind::{}),\n",
            usage_type, def_type
        ));
    }
    output.push_str("            _ => None,\n");
    output.push_str("        }\n");
    output.push_str("    }\n\n");

    output.push_str("}\n");

    output
}

/// Generate additional category predicates beyond the basic ones.
fn generate_extra_predicates(
    all_types: &[&str],
    hierarchy: &HashMap<String, Vec<String>>,
) -> String {
    let mut output = String::new();

    // Control node types (from SysML spec: Fork, Join, Decision, Merge)
    let control_nodes = ["ForkNode", "JoinNode", "DecisionNode", "MergeNode"];

    // Expression types (internal computation, not diagram content)
    let expression_types: Vec<&str> = all_types
        .iter()
        .filter(|t| {
            **t == "Expression"
                || hierarchy
                    .get(**t)
                    .is_some_and(|s| s.contains(&"Expression".to_owned()))
        })
        .copied()
        .collect();

    output.push_str("impl ElementKind {\n");

    // is_control_node()
    output.push_str("    /// Returns `true` if this is a control flow node (Fork, Join, Decision, Merge).\n");
    output.push_str("    ///\n");
    output.push_str("    /// Control nodes appear in action flow and state transition diagrams\n");
    output.push_str("    /// as special graphical symbols (bars, diamonds).\n");
    output.push_str("    pub const fn is_control_node(&self) -> bool {\n");
    output.push_str("        matches!(self,\n");
    for (i, cn) in control_nodes.iter().enumerate() {
        if all_types.contains(cn) {
            if i == control_nodes.len() - 1 {
                output.push_str(&format!("            ElementKind::{}\n", cn));
            } else {
                output.push_str(&format!("            ElementKind::{} |\n", cn));
            }
        }
    }
    output.push_str("        )\n");
    output.push_str("    }\n\n");

    // is_expression()
    output.push_str("    /// Returns `true` if this is an Expression type or any of its subtypes.\n");
    output.push_str("    ///\n");
    output.push_str("    /// Expressions are internal computations (literals, operators, invocations)\n");
    output.push_str("    /// that typically don't appear as standalone diagram elements.\n");
    output.push_str("    pub const fn is_expression(&self) -> bool {\n");
    output.push_str("        matches!(self,\n");
    for (i, expr_type) in expression_types.iter().enumerate() {
        if i == expression_types.len() - 1 {
            output.push_str(&format!("            ElementKind::{}\n", expr_type));
        } else {
            output.push_str(&format!("            ElementKind::{} |\n", expr_type));
        }
    }
    output.push_str("        )\n");
    output.push_str("    }\n\n");

    output.push_str("}\n\n");

    output
}

/// SysML v2 keyword mapping extracted from the xtext grammar.
///
/// Maps ElementKind variants to their base SysML keyword(s).
/// Source: SysML.xtext keyword rules (e.g., PartKeyword = 'part', ActionKeyword = 'action').
/// Only types with concrete textual syntax have keywords; abstract KerML types return None.
pub(crate) const SYNTAX_KEYWORDS: &[(&str, &str)] = &[
    // Standard definitions and usages (keyword + "def" for definitions)
    ("PartDefinition", "part"),
    ("PartUsage", "part"),
    ("AttributeDefinition", "attribute"),
    ("AttributeUsage", "attribute"),
    ("PortDefinition", "port"),
    ("PortUsage", "port"),
    ("ItemDefinition", "item"),
    ("ItemUsage", "item"),
    ("OccurrenceDefinition", "occurrence"),
    ("OccurrenceUsage", "occurrence"),
    ("ConnectionDefinition", "connection"),
    ("ConnectionUsage", "connection"),
    ("InterfaceDefinition", "interface"),
    ("InterfaceUsage", "interface"),
    ("AllocationDefinition", "allocation"),
    ("AllocationUsage", "allocation"),
    ("FlowConnectionDefinition", "flow"),
    ("FlowConnectionUsage", "flow"),
    // Action family
    ("ActionDefinition", "action"),
    ("ActionUsage", "action"),
    ("AcceptActionUsage", "accept"),
    ("AssignmentActionUsage", "assign"),
    ("DecisionNode", "decide"),
    ("ForkNode", "fork"),
    ("JoinNode", "join"),
    ("MergeNode", "merge"),
    ("PerformActionUsage", "perform"),
    ("SendActionUsage", "send"),
    ("IfActionUsage", "if"),
    ("WhileLoopActionUsage", "while"),
    ("ForLoopActionUsage", "for"),
    // State family
    ("StateDefinition", "state"),
    ("StateUsage", "state"),
    ("TransitionUsage", "transition"),
    ("ExhibitStateUsage", "exhibit"),
    // Calculation/Constraint family
    ("CalculationDefinition", "calc"),
    ("CalculationUsage", "calc"),
    ("ConstraintDefinition", "constraint"),
    ("ConstraintUsage", "constraint"),
    ("AssertConstraintUsage", "assert"),
    // Requirement family
    ("RequirementDefinition", "requirement"),
    ("RequirementUsage", "requirement"),
    ("ConcernDefinition", "concern"),
    ("ConcernUsage", "concern"),
    ("StakeholderMembership", "stakeholder"),
    // Case family
    ("CaseDefinition", "case"),
    ("CaseUsage", "case"),
    ("UseCaseDefinition", "use case"),
    ("UseCaseUsage", "use case"),
    ("AnalysisCaseDefinition", "analysis"),
    ("AnalysisCaseUsage", "analysis"),
    ("VerificationCaseDefinition", "verification"),
    ("VerificationCaseUsage", "verification"),
    // View/Viewpoint/Rendering
    ("ViewDefinition", "view"),
    ("ViewUsage", "view"),
    ("ViewpointDefinition", "viewpoint"),
    ("ViewpointUsage", "viewpoint"),
    ("RenderingDefinition", "rendering"),
    ("RenderingUsage", "rendering"),
    // Enumeration
    ("EnumerationDefinition", "enum"),
    ("EnumerationUsage", "enum"),
    // Metadata
    ("MetadataDefinition", "metadata"),
    ("MetadataUsage", "metadata"),
    // Package/Namespace
    ("Package", "package"),
    ("LibraryPackage", "library"),
    // Connectors
    ("BindingConnectorAsUsage", "binding"),
    ("SuccessionAsUsage", "succession"),
    ("SuccessionFlowConnectionUsage", "succession flow"),
    // References
    ("ReferenceUsage", "ref"),
    // Comments/Documentation
    ("Comment", "comment"),
    ("Documentation", "doc"),
];

/// Generate the `syntax_keyword()` method on ElementKind.
fn generate_syntax_keyword(
    all_types: &[&str],
    _hierarchy: &HashMap<String, Vec<String>>,
) -> String {
    let mut output = String::new();

    output.push_str("impl ElementKind {\n");
    output.push_str("    /// Returns the SysML v2 textual keyword for this element kind.\n");
    output.push_str("    ///\n");
    output.push_str("    /// For definitions, this is the base keyword (e.g., `\"part\"` for PartDefinition).\n");
    output.push_str("    /// The full definition syntax is `keyword def Name { }`. For usages, the syntax\n");
    output.push_str("    /// is `keyword name;`. Returns `None` for abstract types with no textual syntax.\n");
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str("    /// assert_eq!(ElementKind::PartDefinition.syntax_keyword(), Some(\"part\"));\n");
    output.push_str("    /// assert_eq!(ElementKind::ActionUsage.syntax_keyword(), Some(\"action\"));\n");
    output.push_str("    /// assert_eq!(ElementKind::Element.syntax_keyword(), None);\n");
    output.push_str("    /// ```\n");
    output.push_str("    pub const fn syntax_keyword(&self) -> Option<&'static str> {\n");
    output.push_str("        match self {\n");

    // Build a lookup from the const table
    let keyword_map: HashMap<&str, &str> = SYNTAX_KEYWORDS.iter().copied().collect();

    for type_name in all_types {
        if let Some(keyword) = keyword_map.get(type_name) {
            output.push_str(&format!(
                "            ElementKind::{} => Some(\"{}\"),\n",
                type_name, keyword
            ));
        }
    }
    output.push_str("            _ => None,\n");
    output.push_str("        }\n");
    output.push_str("    }\n\n");
    output.push_str("}\n\n");

    output
}

/// Generate the `text_template()` method that produces SysML v2 source text for an element.
fn generate_text_template() -> String {
    let mut output = String::new();

    output.push_str("impl ElementKind {\n");
    output.push_str("    /// Generate a SysML v2 text template for creating a new element of this kind.\n");
    output.push_str("    ///\n");
    output.push_str("    /// Returns the textual syntax for a new element with a default name.\n");
    output.push_str("    /// - Definitions: `keyword def NewName { }`\n");
    output.push_str("    /// - Usages: `keyword newName;`\n");
    output.push_str("    /// - Packages: `package NewName { }`\n");
    output.push_str("    /// - Returns `None` for abstract types with no textual syntax.\n");
    output.push_str("    ///\n");
    output.push_str("    /// # Examples\n");
    output.push_str("    ///\n");
    output.push_str("    /// ```\n");
    output.push_str("    /// use sysml_core::ElementKind;\n");
    output.push_str("    ///\n");
    output.push_str("    /// assert_eq!(\n");
    output.push_str("    ///     ElementKind::PartDefinition.text_template(),\n");
    output.push_str("    ///     Some(\"part def NewPart {\\n    }\".to_owned())\n");
    output.push_str("    /// );\n");
    output.push_str("    /// assert_eq!(\n");
    output.push_str("    ///     ElementKind::PartUsage.text_template(),\n");
    output.push_str("    ///     Some(\"part newPart;\".to_owned())\n");
    output.push_str("    /// );\n");
    output.push_str("    /// ```\n");
    output.push_str("    pub fn text_template(&self) -> Option<String> {\n");
    output.push_str("        let keyword = self.syntax_keyword()?;\n");
    output.push_str("        // Derive a default name from the ElementKind variant name.\n");
    output.push_str("        // Strip common suffixes to get the semantic base (e.g., \"Part\" from \"PartUsage\").\n");
    output.push_str("        let kind_str = self.as_str();\n");
    output.push_str("        let base_name = kind_str\n");
    output.push_str("            .strip_suffix(\"Definition\")\n");
    output.push_str("            .or_else(|| kind_str.strip_suffix(\"Usage\"))\n");
    output.push_str("            .or_else(|| kind_str.strip_suffix(\"Node\"))\n");
    output.push_str("            .unwrap_or(kind_str);\n");
    output.push('\n');
    output.push_str("        if self.is_definition() {\n");
    output.push_str("            let def_name = format!(\"New{}\", base_name);\n");
    output.push_str("            Some(format!(\"{} def {} {{\\n    }}\", keyword, def_name))\n");
    output.push_str("        } else if matches!(self, ElementKind::Package | ElementKind::LibraryPackage) {\n");
    output.push_str("            let pkg_name = format!(\"New{}\", base_name);\n");
    output.push_str("            Some(format!(\"{} {} {{\\n    }}\", keyword, pkg_name))\n");
    output.push_str("        } else if self.is_usage() {\n");
    output.push_str("            // camelCase: \"new\" + base_name\n");
    output.push_str("            let usage_name = format!(\"new{}\", base_name);\n");
    output.push_str("            Some(format!(\"{} {};\", keyword, usage_name))\n");
    output.push_str("        } else {\n");
    output.push_str("            // Fallback for other types with keywords (e.g., Comment, Documentation)\n");
    output.push_str("            Some(format!(\"{} /* TODO */\", keyword))\n");
    output.push_str("        }\n");
    output.push_str("    }\n\n");
    output.push_str("}\n\n");

    output
}

/// Generate the `can_own()` method for containment validation.
fn generate_can_own_method(hierarchy: &HashMap<String, Vec<String>>) -> String {
    let mut output = String::new();

    // Determine which types are Namespace subtypes (can own other elements)
    let namespace_subtypes: HashSet<&str> = hierarchy
        .iter()
        .filter(|(name, supers)| {
            *name == "Namespace" || supers.contains(&"Namespace".to_owned())
        })
        .map(|(name, _)| name.as_str())
        .collect();

    // Determine which types are Feature subtypes (usages go inside definitions)
    let _feature_subtypes: HashSet<&str> = hierarchy
        .iter()
        .filter(|(name, supers)| {
            *name == "Feature" || supers.contains(&"Feature".to_owned())
        })
        .map(|(name, _)| name.as_str())
        .collect();

    output.push_str("impl ElementKind {\n");
    output.push_str("    /// Returns `true` if this element kind can own (contain) child elements.\n");
    output.push_str("    ///\n");
    output.push_str("    /// Only Namespace subtypes can own elements via OwningMembership.\n");
    output.push_str("    /// This is used to validate containment when creating elements from diagrams.\n");
    output.push_str("    pub const fn can_own_elements(&self) -> bool {\n");
    output.push_str("        // Namespace subtypes can own elements\n");
    output.push_str("        matches!(self,\n");

    let mut ns_types: Vec<&&str> = namespace_subtypes.iter().collect();
    ns_types.sort();
    for (i, ns_type) in ns_types.iter().enumerate() {
        if i == ns_types.len() - 1 {
            output.push_str(&format!("            ElementKind::{}\n", ns_type));
        } else {
            output.push_str(&format!("            ElementKind::{} |\n", ns_type));
        }
    }
    output.push_str("        )\n");
    output.push_str("    }\n\n");

    // can_contain: checks if a specific child kind can go inside this container kind
    output.push_str("    /// Returns `true` if an element of kind `child` can be placed inside `self`.\n");
    output.push_str("    ///\n");
    output.push_str("    /// Rules:\n");
    output.push_str("    /// - Only namespace types can contain anything\n");
    output.push_str("    /// - Definitions can contain their corresponding usages and other definitions\n");
    output.push_str("    /// - Packages can contain definitions, usages, and other packages\n");
    output.push_str("    /// - Usages with bodies (actions, states) can contain sub-usages\n");
    output.push_str("    pub fn can_contain(&self, child: ElementKind) -> bool {\n");
    output.push_str("        if !self.can_own_elements() {\n");
    output.push_str("            return false;\n");
    output.push_str("        }\n");
    output.push_str("        // Packages can contain anything with syntax\n");
    output.push_str("        if matches!(self, ElementKind::Package | ElementKind::LibraryPackage) {\n");
    output.push_str("            return child.syntax_keyword().is_some();\n");
    output.push_str("        }\n");
    output.push_str("        // Definitions can contain usages and nested definitions\n");
    output.push_str("        if self.is_definition() {\n");
    output.push_str("            return child.is_usage() || child.is_definition();\n");
    output.push_str("        }\n");
    output.push_str("        // Usages (actions, states, etc.) can contain sub-usages\n");
    output.push_str("        if self.is_usage() {\n");
    output.push_str("            return child.is_usage();\n");
    output.push_str("        }\n");
    output.push_str("        false\n");
    output.push_str("    }\n\n");

    output.push_str("}\n\n");

    output
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn make_type(name: &str, supertypes: &[&str]) -> TypeInfo {
        TypeInfo {
            name: name.to_string(),
            supertypes: supertypes.iter().map(|s| s.to_string()).collect(),
            comment: None,
        }
    }

    #[test]
    fn test_build_type_hierarchy() {
        let kerml = vec![
            make_type("Element", &[]),
            make_type("Namespace", &["Element"]),
            make_type("Type", &["Namespace"]),
            make_type("Feature", &["Type"]),
        ];
        let sysml = vec![];

        let hierarchy = TypeHierarchy::new(&kerml, &sysml);

        // Element has no supertypes
        assert_eq!(hierarchy.get_supertypes("Element").unwrap().len(), 0);

        // Namespace has Element
        assert!(hierarchy.is_subtype_of("Namespace", "Element"));

        // Type has Namespace and Element
        let type_supers = hierarchy.get_supertypes("Type").unwrap();
        assert!(type_supers.contains(&"Namespace".to_owned()));
        assert!(type_supers.contains(&"Element".to_owned()));

        // Feature has Type, Namespace, and Element
        let feature_supers = hierarchy.get_supertypes("Feature").unwrap();
        assert!(feature_supers.contains(&"Type".to_owned()));
        assert!(feature_supers.contains(&"Namespace".to_owned()));
        assert!(feature_supers.contains(&"Element".to_owned()));
    }

    #[test]
    fn test_generates_methods() {
        let kerml = vec![
            make_type("Element", &[]),
            make_type("Type", &["Element"]),
            make_type("Feature", &["Type"]),
            make_type("Relationship", &["Element"]),
            make_type("Specialization", &["Relationship"]),
            make_type("Classifier", &["Type"]),
        ];
        let sysml = vec![
            make_type("PartDefinition", &["Classifier"]),
            make_type("PartUsage", &["Feature"]),
        ];

        let code = generate_hierarchy_methods(&kerml, &sysml);

        // Check method signatures
        assert!(code.contains("pub fn supertypes(&self)"));
        assert!(code.contains("pub fn direct_supertypes(&self)"));
        assert!(code.contains("pub fn is_subtype_of(&self"));
        assert!(code.contains("pub const fn is_definition(&self)"));
        assert!(code.contains("pub const fn is_usage(&self)"));
        assert!(code.contains("pub const fn is_relationship(&self)"));
        assert!(code.contains("pub const fn is_classifier(&self)"));
        assert!(code.contains("pub const fn is_feature(&self)"));
        assert!(code.contains("pub const fn corresponding_usage(&self)"));
        assert!(code.contains("pub const fn corresponding_definition(&self)"));

        // Check some specific entries
        assert!(code.contains("ElementKind::PartDefinition => Some(ElementKind::PartUsage)"));
        assert!(code.contains("ElementKind::PartUsage => Some(ElementKind::PartDefinition)"));
    }

    #[test]
    fn test_handles_cycles() {
        // Even with cycles, should not infinite loop
        let kerml = vec![
            make_type("A", &["B"]),
            make_type("B", &["A"]), // Cycle
        ];
        let sysml = vec![];

        let hierarchy = TypeHierarchy::new(&kerml, &sysml);

        // Should complete without hanging
        assert!(hierarchy.contains_type("A"));
        assert!(hierarchy.contains_type("B"));
    }

    #[test]
    fn type_hierarchy_is_root_type() {
        let kerml = vec![
            make_type("Element", &[]),
            make_type("Type", &["Element"]),
            make_type("Feature", &["Type"]),
        ];
        let sysml = vec![];

        let hierarchy = TypeHierarchy::new(&kerml, &sysml);

        assert!(hierarchy.is_root_type("Element"), "Element has no parents");
        assert!(!hierarchy.is_root_type("Type"), "Type has parent Element");
        assert!(
            !hierarchy.is_root_type("Feature"),
            "Feature has parent Type"
        );
        assert!(
            !hierarchy.is_root_type("Unknown"),
            "Unknown type not in hierarchy"
        );
    }

    #[test]
    fn type_hierarchy_has_subtypes() {
        let kerml = vec![
            make_type("Element", &[]),
            make_type("Type", &["Element"]),
            make_type("Feature", &["Type"]),
        ];
        let sysml = vec![];

        let hierarchy = TypeHierarchy::new(&kerml, &sysml);

        assert!(
            hierarchy.has_subtypes("Element"),
            "Element is supertype of Type and Feature"
        );
        assert!(
            hierarchy.has_subtypes("Type"),
            "Type is supertype of Feature"
        );
        assert!(
            !hierarchy.has_subtypes("Feature"),
            "Feature has no subtypes"
        );
    }

    #[test]
    fn type_hierarchy_get_supertypes() {
        let kerml = vec![
            make_type("Element", &[]),
            make_type("Type", &["Element"]),
            make_type("Feature", &["Type"]),
        ];
        let sysml = vec![];

        let hierarchy = TypeHierarchy::new(&kerml, &sysml);

        let feature_supers = hierarchy.get_supertypes("Feature").unwrap();
        assert!(feature_supers.contains(&"Type".to_owned()));
        assert!(feature_supers.contains(&"Element".to_owned()));

        let element_supers = hierarchy.get_supertypes("Element").unwrap();
        assert!(element_supers.is_empty());
    }
}
