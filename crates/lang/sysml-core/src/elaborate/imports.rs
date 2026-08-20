//! Import elaboration.
//!
//! Normalizes import elements so downstream resolution and health checks can
//! find properties in consistent locations. The parser may store the imported
//! namespace in `unresolved_importedNamespace` while the resolver and health
//! checks look for `importedNamespace`.

use super::ElaborationReport;
use crate::{ElementId, ElementKind, ModelGraph, Value};

/// Elaborate imports: normalize property names and derive boolean flags.
pub(super) fn elaborate_imports(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    normalize_imported_namespace(graph, report);
    normalize_recursive_flag(graph, report);
    derive_is_namespace(graph, report);
}

/// Copy `unresolved_importedNamespace` → `importedNamespace` so the resolver
/// and health diagnostics can find it.
fn normalize_imported_namespace(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let mut import_ids = Vec::new();
    import_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::MembershipImport));
    import_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::NamespaceImport));

    let to_elaborate: Vec<(ElementId, String)> = import_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter_map(|e| {
            // Only elaborate if importedNamespace not already set
            if e.get_prop("importedNamespace").is_some() {
                return None;
            }
            let value = e
                .get_prop("unresolved_importedNamespace")
                .and_then(|v| v.as_str())?
                .to_owned();
            Some((e.id.clone(), value))
        })
        .collect();

    for (id, value) in to_elaborate {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("importedNamespace", Value::String(value));
            report.imports_elaborated += 1;
        }
    }
}

/// Normalize `isRecursive` from string to bool.
///
/// The parser may set `isRecursive` as a string `"true"` when encountering
/// the `**` recursive import syntax. This pass normalizes it to a proper bool.
fn normalize_recursive_flag(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let mut recursive_import_ids = Vec::new();
    recursive_import_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::MembershipImport));
    recursive_import_ids
        .extend_from_slice(graph.element_ids_by_kind(&ElementKind::NamespaceImport));

    let to_normalize: Vec<ElementId> = recursive_import_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter_map(|e| {
            let val = e.get_prop("isRecursive")?;
            // Already a bool — nothing to do
            if val.as_bool().is_some() {
                return None;
            }
            // String "true" needs normalizing
            if val.as_str() == Some("true") {
                return Some(e.id.clone());
            }
            None
        })
        .collect();

    for id in to_normalize {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("isRecursive", Value::Bool(true));
            report.imports_elaborated += 1;
        }
    }
}

/// Derive `isNamespace` flag based on element kind.
///
/// `NamespaceImport` → `isNamespace = true`, `MembershipImport` → `isNamespace = false`.
/// This makes it easy for downstream checks to query the import mode without
/// matching on the element kind.
fn derive_is_namespace(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let mut ns_import_ids = Vec::new();
    ns_import_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::MembershipImport));
    ns_import_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::NamespaceImport));

    let to_tag: Vec<(ElementId, bool)> = ns_import_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("isNamespace").is_none())
        .map(|e| {
            let is_ns = e.kind == ElementKind::NamespaceImport;
            (e.id.clone(), is_ns)
        })
        .collect();

    for (id, is_ns) in to_tag {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("isNamespace", Value::Bool(is_ns));
            report.imports_elaborated += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate;
    use crate::Element;

    #[test]
    fn copies_unresolved_imported_namespace() {
        let mut graph = ModelGraph::new();

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_name("import1")
            .with_prop("unresolved_importedNamespace", "ScalarValues");
        let imp_id = graph.add_element(imp);

        let report = elaborate(&mut graph);

        assert!(report.imports_elaborated >= 1);
        let elem = graph.get_element(&imp_id).unwrap();
        assert_eq!(
            elem.get_prop("importedNamespace").and_then(|v| v.as_str()),
            Some("ScalarValues")
        );
    }

    #[test]
    fn does_not_overwrite_existing_imported_namespace() {
        let mut graph = ModelGraph::new();

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_name("import1")
            .with_prop("importedNamespace", "Parts")
            .with_prop("unresolved_importedNamespace", "ScalarValues");
        let imp_id = graph.add_element(imp);

        elaborate(&mut graph);

        let elem = graph.get_element(&imp_id).unwrap();
        assert_eq!(
            elem.get_prop("importedNamespace").and_then(|v| v.as_str()),
            Some("Parts")
        );
    }

    #[test]
    fn normalizes_string_recursive_to_bool() {
        let mut graph = ModelGraph::new();

        let imp = Element::new_with_kind(ElementKind::MembershipImport)
            .with_name("import1")
            .with_prop("isRecursive", "true");
        let imp_id = graph.add_element(imp);

        elaborate(&mut graph);

        let elem = graph.get_element(&imp_id).unwrap();
        assert_eq!(
            elem.get_prop("isRecursive").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn preserves_existing_bool_recursive() {
        let mut graph = ModelGraph::new();

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_name("import1")
            .with_prop("isRecursive", Value::Bool(true));
        let imp_id = graph.add_element(imp);

        elaborate(&mut graph);

        let elem = graph.get_element(&imp_id).unwrap();
        assert_eq!(
            elem.get_prop("isRecursive").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn derives_is_namespace_flag() {
        let mut graph = ModelGraph::new();

        let ns_imp = Element::new_with_kind(ElementKind::NamespaceImport).with_name("nsImport");
        let ns_id = graph.add_element(ns_imp);

        let mem_imp = Element::new_with_kind(ElementKind::MembershipImport).with_name("memImport");
        let mem_id = graph.add_element(mem_imp);

        elaborate(&mut graph);

        let ns_elem = graph.get_element(&ns_id).unwrap();
        assert_eq!(
            ns_elem.get_prop("isNamespace").and_then(|v| v.as_bool()),
            Some(true)
        );

        let mem_elem = graph.get_element(&mem_id).unwrap();
        assert_eq!(
            mem_elem.get_prop("isNamespace").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn idempotent() {
        let mut graph = ModelGraph::new();

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_name("import1")
            .with_prop("unresolved_importedNamespace", "ScalarValues")
            .with_prop("isRecursive", "true");
        graph.add_element(imp);

        let r1 = elaborate(&mut graph);
        assert!(r1.imports_elaborated > 0);

        let r2 = elaborate(&mut graph);
        assert_eq!(r2.imports_elaborated, 0, "second elaborate should be no-op");
    }
}
