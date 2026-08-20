use super::*;
use crate::{Element, ElementKind, VisibilityKind};
use sysml_span::Diagnostic;

fn create_test_hierarchy() -> (ModelGraph, ElementId, ElementId, ElementId) {
    let mut graph = ModelGraph::new();

    // Create TestPackage::SubPackage::PartDef
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPackage");
    let pkg_id = graph.add_element(pkg);

    let sub = Element::new_with_kind(ElementKind::Package).with_name("SubPackage");
    let sub_id = graph.add_owned_element(sub, pkg_id.clone(), VisibilityKind::Public);

    let part = Element::new_with_kind(ElementKind::PartDefinition).with_name("PartDef");
    let part_id = graph.add_owned_element(part, sub_id.clone(), VisibilityKind::Public);

    (graph, pkg_id, sub_id, part_id)
}

#[test]
fn scope_table_basic() {
    let mut table = ScopeTable::new();
    let id1 = ElementId::new_v4();
    let id2 = ElementId::new_v4();

    table.add_owned("foo".to_string(), id1.clone());
    table.add_owned_short("f".to_string(), id2.clone());

    assert_eq!(table.lookup_owned("foo"), Some(&id1));
    assert_eq!(table.lookup_owned("f"), Some(&id2));
    assert_eq!(table.lookup_owned("bar"), None);
}

#[test]
fn resolve_name_owned_member() {
    let (graph, pkg_id, sub_id, _) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&pkg_id, "SubPackage");

    assert_eq!(resolved, Some(sub_id));
}

#[test]
fn resolve_name_not_found() {
    let (graph, pkg_id, _, _) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&pkg_id, "NonExistent");

    assert!(resolved.is_none());
}

#[test]
fn resolve_name_via_parent() {
    let (graph, _, sub_id, _) = create_test_hierarchy();

    // Create another root package
    let mut graph = graph;
    let other = Element::new_with_kind(ElementKind::Package).with_name("OtherPackage");
    let other_id = graph.add_element(other);

    // From SubPackage, resolve OtherPackage (via parent -> global)
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&sub_id, "OtherPackage");

    assert_eq!(resolved, Some(other_id));
}

#[test]
fn resolve_qualified_name_simple() {
    let (graph, pkg_id, _, part_id) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_qualified_name(&pkg_id, "SubPackage::PartDef");

    assert_eq!(resolved, Some(part_id));
}

#[test]
fn resolve_qualified_name_global() {
    let (graph, _, _, part_id) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_qualified_name_global("TestPackage::SubPackage::PartDef");

    assert_eq!(resolved, Some(part_id));
}

#[test]
fn resolve_qualified_name_global_not_found() {
    let (graph, _, _, _) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_qualified_name_global("NonExistent::Path");

    assert!(resolved.is_none());
}

#[test]
fn resolution_context_diagnostics() {
    let graph = ModelGraph::new();
    let mut ctx = graph.resolution_context();

    ctx.add_diagnostic(Diagnostic::error("test error"));
    ctx.add_diagnostic(Diagnostic::warning("test warning"));

    assert!(ctx.diagnostics().has_errors());
    assert_eq!(ctx.diagnostics().len(), 2);
}

#[test]
fn convenience_methods() {
    let (graph, pkg_id, sub_id, part_id) = create_test_hierarchy();

    // Test resolve_name_in
    assert_eq!(
        graph.resolve_name_in(&pkg_id, "SubPackage"),
        Some(sub_id.clone())
    );

    // Test resolve_qualified
    assert_eq!(
        graph.resolve_qualified("TestPackage::SubPackage::PartDef"),
        Some(part_id)
    );
}

#[test]
fn scope_table_imported() {
    let mut table = ScopeTable::new();
    let id = ElementId::new_v4();

    table.add_imported("Imported".to_string(), id.clone(), VisibilityKind::Public);

    assert_eq!(table.lookup_imported("Imported"), Some(&id));
    assert_eq!(table.lookup_imported_visible("Imported", true), Some(&id));

    // Add a private import
    let id2 = ElementId::new_v4();
    table.add_imported("Private".to_string(), id2.clone(), VisibilityKind::Private);

    assert_eq!(table.lookup_imported("Private"), Some(&id2));
    assert_eq!(table.lookup_imported_visible("Private", true), None);
    assert_eq!(table.lookup_imported_visible("Private", false), Some(&id2));
}

#[test]
fn resolution_result_tracking() {
    let mut result = ResolutionResult::new();
    assert!(result.is_complete());
    assert!(!result.has_errors());

    result.resolved_count = 5;
    result.unresolved_count = 2;
    result.diagnostics.error("unresolved reference");

    assert!(!result.is_complete());
    assert!(result.has_errors());
}

// === Import Expansion Tests (Phase 2d.2) ===

/// Helper to create an import element with properties.
fn create_import(
    graph: &mut ModelGraph,
    owner_id: &ElementId,
    imported_ref: &str,
    is_namespace: bool,
    is_recursive: bool,
) -> ElementId {
    use crate::Value;

    let mut import = Element::new_with_kind(ElementKind::Import);
    import.set_prop(
        import_props::IMPORTED_REFERENCE,
        Value::String(imported_ref.to_string()),
    );
    if is_namespace {
        import.set_prop(import_props::IS_NAMESPACE, Value::Bool(true));
    }
    if is_recursive {
        import.set_prop(import_props::IS_RECURSIVE, Value::Bool(true));
    }

    graph.add_owned_element(import, owner_id.clone(), VisibilityKind::Public)
}

#[test]
fn resolve_via_membership_import() {
    let mut graph = ModelGraph::new();

    // Create LibPackage::Helper
    let lib = Element::new_with_kind(ElementKind::Package).with_name("LibPackage");
    let lib_id = graph.add_element(lib);

    let helper = Element::new_with_kind(ElementKind::PartDefinition).with_name("Helper");
    let helper_id = graph.add_owned_element(helper, lib_id.clone(), VisibilityKind::Public);

    // Create UserPackage with import LibPackage::Helper
    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);

    // Add a membership import (specific element)
    create_import(&mut graph, &user_id, "LibPackage::Helper", false, false);

    // From UserPackage, resolve "Helper" via import
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&user_id, "Helper");

    assert_eq!(resolved, Some(helper_id));
}

#[test]
fn resolve_via_namespace_import() {
    let mut graph = ModelGraph::new();

    // Create LibPackage with multiple members
    let lib = Element::new_with_kind(ElementKind::Package).with_name("LibPackage");
    let lib_id = graph.add_element(lib);

    let part_a = Element::new_with_kind(ElementKind::PartDefinition).with_name("PartA");
    let part_a_id = graph.add_owned_element(part_a, lib_id.clone(), VisibilityKind::Public);

    let part_b = Element::new_with_kind(ElementKind::PartDefinition).with_name("PartB");
    let part_b_id = graph.add_owned_element(part_b, lib_id.clone(), VisibilityKind::Public);

    // Create UserPackage with namespace import (::*)
    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);

    // Add a namespace import: import LibPackage::*
    create_import(&mut graph, &user_id, "LibPackage", true, false);

    // From UserPackage, resolve both "PartA" and "PartB" via import
    let mut ctx = graph.resolution_context();
    let resolved_a = ctx.resolve_name(&user_id, "PartA");
    let resolved_b = ctx.resolve_name(&user_id, "PartB");

    assert_eq!(resolved_a, Some(part_a_id));
    assert_eq!(resolved_b, Some(part_b_id));
}

#[test]
fn resolve_qualified_member_through_imported_namespace() {
    let mut graph = ModelGraph::new();

    // Library package that owns the member
    let isq_base = Element::new_with_kind(ElementKind::Package).with_name("ISQBase");
    let isq_base_id = graph.add_element(isq_base);
    let mass = Element::new_with_kind(ElementKind::AttributeUsage).with_name("mass");
    let mass_id = graph.add_owned_element(mass, isq_base_id.clone(), VisibilityKind::Public);

    // Re-exporting package: ISQ publicly imports ISQBase::*
    let isq = Element::new_with_kind(ElementKind::Package).with_name("ISQ");
    let isq_id = graph.add_element(isq);
    create_import(&mut graph, &isq_id, "ISQBase", true, false);

    // User scope that references ISQ::mass
    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_feature_reference(&user_id, "ISQ::mass");

    assert_eq!(
        resolved,
        Some(mass_id),
        "qualified member lookup should see imported members of intermediate namespace"
    );
}

#[test]
fn namespace_import_respects_visibility() {
    let mut graph = ModelGraph::new();

    // Create LibPackage with public and private members
    let lib = Element::new_with_kind(ElementKind::Package).with_name("LibPackage");
    let lib_id = graph.add_element(lib);

    let public_part = Element::new_with_kind(ElementKind::PartDefinition).with_name("PublicPart");
    let public_id = graph.add_owned_element(public_part, lib_id.clone(), VisibilityKind::Public);

    let private_part = Element::new_with_kind(ElementKind::PartDefinition).with_name("PrivatePart");
    let _private_id =
        graph.add_owned_element(private_part, lib_id.clone(), VisibilityKind::Private);

    // Create UserPackage with namespace import
    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);

    create_import(&mut graph, &user_id, "LibPackage", true, false);

    // From UserPackage, only public members should be resolvable
    let mut ctx = graph.resolution_context();
    let resolved_public = ctx.resolve_name(&user_id, "PublicPart");
    let resolved_private = ctx.resolve_name(&user_id, "PrivatePart");

    assert_eq!(resolved_public, Some(public_id));
    assert!(
        resolved_private.is_none(),
        "Private members should not be imported"
    );
}

#[test]
fn public_import_re_exports_through_intermediate_namespace() {
    use crate::Value;

    let mut graph = ModelGraph::new();

    // K::Item — original package.
    let k = Element::new_with_kind(ElementKind::Package).with_name("K");
    let k_id = graph.add_element(k);
    let item = Element::new_with_kind(ElementKind::PartDefinition).with_name("Item");
    let item_id = graph.add_owned_element(item, k_id.clone(), VisibilityKind::Public);

    // M re-exports K::*: `public import K::*;`.
    let m = Element::new_with_kind(ElementKind::Package).with_name("M");
    let m_id = graph.add_element(m);
    let mut m_imp = Element::new_with_kind(ElementKind::Import);
    m_imp.set_prop(import_props::IMPORTED_REFERENCE, Value::String("K".to_string()));
    m_imp.set_prop(import_props::IS_NAMESPACE, Value::Bool(true));
    m_imp.set_prop(import_props::VISIBILITY, "public");
    graph.add_owned_element(m_imp, m_id.clone(), VisibilityKind::Public);

    // N: `import M::*;` (default private). Item should reach N via M's re-export.
    let n = Element::new_with_kind(ElementKind::Package).with_name("N");
    let n_id = graph.add_element(n);
    let mut n_imp = Element::new_with_kind(ElementKind::Import);
    n_imp.set_prop(import_props::IMPORTED_REFERENCE, Value::String("M".to_string()));
    n_imp.set_prop(import_props::IS_NAMESPACE, Value::Bool(true));
    graph.add_owned_element(n_imp, n_id.clone(), VisibilityKind::Public);

    let mut ctx = graph.resolution_context();
    assert_eq!(
        ctx.resolve_name(&n_id, "Item"),
        Some(item_id),
        "public re-export should reach the outer namespace"
    );
}

#[test]
fn private_import_does_not_re_export() {
    use crate::Value;

    let mut graph = ModelGraph::new();

    let k = Element::new_with_kind(ElementKind::Package).with_name("K");
    let k_id = graph.add_element(k);
    let item = Element::new_with_kind(ElementKind::PartDefinition).with_name("HiddenItem");
    let _item_id = graph.add_owned_element(item, k_id.clone(), VisibilityKind::Public);

    // M imports K with default private visibility — must NOT re-export.
    let m = Element::new_with_kind(ElementKind::Package).with_name("M");
    let m_id = graph.add_element(m);
    let mut m_imp = Element::new_with_kind(ElementKind::Import);
    m_imp.set_prop(import_props::IMPORTED_REFERENCE, Value::String("K".to_string()));
    m_imp.set_prop(import_props::IS_NAMESPACE, Value::Bool(true));
    // No visibility prop → defaults to private in expand_imports / re-export filter.
    graph.add_owned_element(m_imp, m_id.clone(), VisibilityKind::Public);

    let n = Element::new_with_kind(ElementKind::Package).with_name("N");
    let n_id = graph.add_element(n);
    let mut n_imp = Element::new_with_kind(ElementKind::Import);
    n_imp.set_prop(import_props::IMPORTED_REFERENCE, Value::String("M".to_string()));
    n_imp.set_prop(import_props::IS_NAMESPACE, Value::Bool(true));
    graph.add_owned_element(n_imp, n_id.clone(), VisibilityKind::Public);

    let mut ctx = graph.resolution_context();
    assert!(
        ctx.resolve_name(&n_id, "HiddenItem").is_none(),
        "private (default) import must not re-export through the intermediate"
    );
}

#[test]
fn import_all_overrides_visibility_filter() {
    use crate::Value;

    let mut graph = ModelGraph::new();

    // LibPackage with a private member.
    let lib = Element::new_with_kind(ElementKind::Package).with_name("LibPackage");
    let lib_id = graph.add_element(lib);
    let private_part = Element::new_with_kind(ElementKind::PartDefinition).with_name("HiddenPart");
    let private_id =
        graph.add_owned_element(private_part, lib_id.clone(), VisibilityKind::Private);

    // UserPackage with `import all LibPackage::*;` (isImportAll = true).
    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);
    let mut import = Element::new_with_kind(ElementKind::Import);
    import.set_prop(
        import_props::IMPORTED_REFERENCE,
        Value::String("LibPackage".to_string()),
    );
    import.set_prop(import_props::IS_NAMESPACE, Value::Bool(true));
    import.set_prop(import_props::IS_IMPORT_ALL, Value::Bool(true));
    graph.add_owned_element(import, user_id.clone(), VisibilityKind::Public);

    // With isImportAll, the private member becomes resolvable.
    let mut ctx = graph.resolution_context();
    assert_eq!(
        ctx.resolve_name(&user_id, "HiddenPart"),
        Some(private_id),
        "isImportAll should import members regardless of declared visibility"
    );
}

#[test]
fn resolve_via_recursive_import() {
    let mut graph = ModelGraph::new();

    // Create LibPackage::Sub::DeepPart
    let lib = Element::new_with_kind(ElementKind::Package).with_name("LibPackage");
    let lib_id = graph.add_element(lib);

    let sub = Element::new_with_kind(ElementKind::Package).with_name("Sub");
    let sub_id = graph.add_owned_element(sub, lib_id.clone(), VisibilityKind::Public);

    let deep_part = Element::new_with_kind(ElementKind::PartDefinition).with_name("DeepPart");
    let deep_part_id = graph.add_owned_element(deep_part, sub_id.clone(), VisibilityKind::Public);

    // Also add a direct member
    let top_part = Element::new_with_kind(ElementKind::PartDefinition).with_name("TopPart");
    let top_part_id = graph.add_owned_element(top_part, lib_id.clone(), VisibilityKind::Public);

    // Create UserPackage with recursive import (::**)
    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);

    create_import(&mut graph, &user_id, "LibPackage", true, true);

    // From UserPackage, resolve both top-level and nested members
    let mut ctx = graph.resolution_context();
    let resolved_top = ctx.resolve_name(&user_id, "TopPart");
    let resolved_sub = ctx.resolve_name(&user_id, "Sub");
    let resolved_deep = ctx.resolve_name(&user_id, "DeepPart");

    assert_eq!(resolved_top, Some(top_part_id));
    assert_eq!(resolved_sub, Some(sub_id));
    assert_eq!(resolved_deep, Some(deep_part_id));
}

#[test]
fn owned_takes_precedence_over_imported() {
    let mut graph = ModelGraph::new();

    // Create LibPackage::Part
    let lib = Element::new_with_kind(ElementKind::Package).with_name("LibPackage");
    let lib_id = graph.add_element(lib);

    let lib_part = Element::new_with_kind(ElementKind::PartDefinition).with_name("Part");
    let _lib_part_id = graph.add_owned_element(lib_part, lib_id.clone(), VisibilityKind::Public);

    // Create UserPackage::Part (same name as imported)
    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);

    let user_part = Element::new_with_kind(ElementKind::PartDefinition).with_name("Part");
    let user_part_id = graph.add_owned_element(user_part, user_id.clone(), VisibilityKind::Public);

    // Add namespace import
    create_import(&mut graph, &user_id, "LibPackage", true, false);

    // Resolve "Part" - should get the owned one, not imported
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&user_id, "Part");

    assert_eq!(
        resolved,
        Some(user_part_id),
        "Owned members take precedence over imports"
    );
}

#[test]
fn distinct_same_name_imports_flagged_ambiguous() {
    // Two different libraries each export a *distinct* `Widget`, and the user
    // package imports both. The collision must be recorded (ADR-016 D5).
    let mut graph = ModelGraph::new();

    let lib_a = Element::new_with_kind(ElementKind::Package).with_name("LibA");
    let lib_a_id = graph.add_element(lib_a);
    let widget_a = Element::new_with_kind(ElementKind::PartDefinition).with_name("Widget");
    let widget_a_id = graph.add_owned_element(widget_a, lib_a_id.clone(), VisibilityKind::Public);

    let lib_b = Element::new_with_kind(ElementKind::Package).with_name("LibB");
    let lib_b_id = graph.add_element(lib_b);
    let widget_b = Element::new_with_kind(ElementKind::PartDefinition).with_name("Widget");
    let widget_b_id = graph.add_owned_element(widget_b, lib_b_id.clone(), VisibilityKind::Public);

    assert_ne!(widget_a_id, widget_b_id, "two distinct Widget elements");

    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);

    // import LibA::*; import LibB::*  — both bring in `Widget`.
    create_import(&mut graph, &user_id, "LibA", true, false);
    create_import(&mut graph, &user_id, "LibB", true, false);

    let mut ctx = graph.resolution_context();
    let table = ctx.get_full_scope_table(&user_id);

    let colliding = table
        .ambiguous_imported("Widget")
        .expect("Widget should be flagged as an ambiguous import");
    assert!(
        colliding.contains(&widget_a_id) && colliding.contains(&widget_b_id),
        "both distinct Widget ids should be recorded as colliding, got {colliding:?}"
    );
    assert_eq!(
        colliding.len(),
        2,
        "exactly two distinct ids collide, got {colliding:?}"
    );
}

#[test]
fn owned_shadows_imported_is_not_ambiguous() {
    // OWNED member with the same name as an IMPORTED one is spec-correct
    // shadowing, NOT ambiguity — different precedence tiers must not collide.
    let mut graph = ModelGraph::new();

    let lib = Element::new_with_kind(ElementKind::Package).with_name("LibPackage");
    let lib_id = graph.add_element(lib);
    let lib_part = Element::new_with_kind(ElementKind::PartDefinition).with_name("Part");
    let _lib_part_id = graph.add_owned_element(lib_part, lib_id.clone(), VisibilityKind::Public);

    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);
    let user_part = Element::new_with_kind(ElementKind::PartDefinition).with_name("Part");
    let user_part_id = graph.add_owned_element(user_part, user_id.clone(), VisibilityKind::Public);

    create_import(&mut graph, &user_id, "LibPackage", true, false);

    let mut ctx = graph.resolution_context();

    // Owned still wins (unchanged behaviour).
    assert_eq!(
        ctx.resolve_name(&user_id, "Part"),
        Some(user_part_id),
        "owned member must still win over import"
    );

    // And the name must NOT be flagged ambiguous: only one import contributed
    // `Part`, the owned member lives in a different tier.
    let table = ctx.get_full_scope_table(&user_id);
    assert!(
        table.ambiguous_imported("Part").is_none(),
        "owned-shadows-imported is not an import collision"
    );
}

#[test]
fn same_target_via_two_imports_is_not_ambiguous() {
    // The same element reached through two public imports (re-export) is NOT a
    // collision — only distinct ids collide.
    let mut graph = ModelGraph::new();

    let base = Element::new_with_kind(ElementKind::Package).with_name("Base");
    let base_id = graph.add_element(base);
    let thing = Element::new_with_kind(ElementKind::PartDefinition).with_name("Thing");
    let _thing_id = graph.add_owned_element(thing, base_id.clone(), VisibilityKind::Public);

    // Re-exporter publicly imports Base::*
    let reexport = Element::new_with_kind(ElementKind::Package).with_name("ReExport");
    let reexport_id = graph.add_element(reexport);
    create_import(&mut graph, &reexport_id, "Base", true, false);

    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);
    // import Base::*;  import ReExport::*  — both resolve `Thing` to the SAME id.
    create_import(&mut graph, &user_id, "Base", true, false);
    create_import(&mut graph, &user_id, "ReExport", true, false);

    let mut ctx = graph.resolution_context();
    let table = ctx.get_full_scope_table(&user_id);
    assert!(
        table.ambiguous_imported("Thing").is_none(),
        "the same target reached via two imports is not ambiguous"
    );
}

// === Inheritance Resolution Tests (Phase 2d.3) ===

/// Helper to create a Specialization element.
fn create_specialization(
    graph: &mut ModelGraph,
    owner_id: &ElementId,
    general_ref: &str,
) -> ElementId {
    use crate::Value;

    let mut spec = Element::new_with_kind(ElementKind::Specialization);
    spec.set_prop(
        unresolved_props::GENERAL,
        Value::String(general_ref.to_string()),
    );

    graph.add_owned_element(spec, owner_id.clone(), VisibilityKind::Public)
}

/// Helper to create a Redefinition element.
fn create_redefinition(
    graph: &mut ModelGraph,
    owner_id: &ElementId,
    redefined_ref: &str,
) -> ElementId {
    use crate::Value;

    let mut redef = Element::new_with_kind(ElementKind::Redefinition);
    redef.set_prop(
        unresolved_props::REDEFINED_FEATURE,
        Value::String(redefined_ref.to_string()),
    );

    graph.add_owned_element(redef, owner_id.clone(), VisibilityKind::Public)
}

#[test]
fn resolve_inherited_member() {
    let mut graph = ModelGraph::new();

    // Create Package
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    // Create BaseDef with a member
    let base_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("BaseDef");
    let base_id = graph.add_owned_element(base_def, pkg_id.clone(), VisibilityKind::Public);

    let inherited_part = Element::new_with_kind(ElementKind::PartUsage).with_name("inheritedPart");
    let inherited_id =
        graph.add_owned_element(inherited_part, base_id.clone(), VisibilityKind::Public);

    // Create DerivedDef that specializes BaseDef
    let derived_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("DerivedDef");
    let derived_id = graph.add_owned_element(derived_def, pkg_id.clone(), VisibilityKind::Public);

    // Add specialization: DerivedDef :> BaseDef
    create_specialization(&mut graph, &derived_id, "TestPkg::BaseDef");

    // From DerivedDef, resolve "inheritedPart" via inheritance
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&derived_id, "inheritedPart");

    assert_eq!(resolved, Some(inherited_id));
}

#[test]
fn owned_takes_precedence_over_inherited() {
    let mut graph = ModelGraph::new();

    // Create Package
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    // Create BaseDef with a member
    let base_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("BaseDef");
    let base_id = graph.add_owned_element(base_def, pkg_id.clone(), VisibilityKind::Public);

    let base_part = Element::new_with_kind(ElementKind::PartUsage).with_name("part");
    let _base_part_id = graph.add_owned_element(base_part, base_id.clone(), VisibilityKind::Public);

    // Create DerivedDef with its own "part" member
    let derived_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("DerivedDef");
    let derived_id = graph.add_owned_element(derived_def, pkg_id.clone(), VisibilityKind::Public);

    let derived_part = Element::new_with_kind(ElementKind::PartUsage).with_name("part");
    let derived_part_id =
        graph.add_owned_element(derived_part, derived_id.clone(), VisibilityKind::Public);

    // Add specialization
    create_specialization(&mut graph, &derived_id, "TestPkg::BaseDef");

    // Resolve "part" - should get the owned one, not inherited
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&derived_id, "part");

    assert_eq!(
        resolved,
        Some(derived_part_id),
        "Owned members take precedence over inherited"
    );
}

#[test]
fn redefinition_shadows_inherited() {
    let mut graph = ModelGraph::new();

    // Create Package
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    // Create BaseDef with a member
    let base_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("BaseDef");
    let base_id = graph.add_owned_element(base_def, pkg_id.clone(), VisibilityKind::Public);

    let base_part = Element::new_with_kind(ElementKind::PartUsage).with_name("part");
    let _base_part_id = graph.add_owned_element(base_part, base_id.clone(), VisibilityKind::Public);

    // Create DerivedDef with a redefinition
    let derived_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("DerivedDef");
    let derived_id = graph.add_owned_element(derived_def, pkg_id.clone(), VisibilityKind::Public);

    // Add specialization
    create_specialization(&mut graph, &derived_id, "TestPkg::BaseDef");

    // Add redefinition of "part"
    create_redefinition(&mut graph, &derived_id, "TestPkg::BaseDef::part");

    // Resolve "part" - should NOT find it (redefined but not re-declared)
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&derived_id, "part");

    // The redefinition shadows the inherited member, but since we haven't
    // added a new "part" member, it should not be found
    assert!(resolved.is_none(), "Redefined member should be shadowed");
}

#[test]
fn inherited_member_multiple_levels() {
    let mut graph = ModelGraph::new();

    // Create Package
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    // Create GrandBase with a member
    let grand_base = Element::new_with_kind(ElementKind::PartDefinition).with_name("GrandBase");
    let grand_id = graph.add_owned_element(grand_base, pkg_id.clone(), VisibilityKind::Public);

    let grand_part = Element::new_with_kind(ElementKind::PartUsage).with_name("grandPart");
    let grand_part_id =
        graph.add_owned_element(grand_part, grand_id.clone(), VisibilityKind::Public);

    // Create BaseDef that specializes GrandBase
    let base_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("BaseDef");
    let base_id = graph.add_owned_element(base_def, pkg_id.clone(), VisibilityKind::Public);

    create_specialization(&mut graph, &base_id, "TestPkg::GrandBase");

    // Create DerivedDef that specializes BaseDef
    let derived_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("DerivedDef");
    let derived_id = graph.add_owned_element(derived_def, pkg_id.clone(), VisibilityKind::Public);

    create_specialization(&mut graph, &derived_id, "TestPkg::BaseDef");

    // From DerivedDef, resolve "grandPart" via transitive inheritance
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&derived_id, "grandPart");

    assert_eq!(
        resolved,
        Some(grand_part_id),
        "Should resolve through inheritance chain"
    );
}

#[test]
fn private_members_not_inherited() {
    let mut graph = ModelGraph::new();

    // Create Package
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    // Create BaseDef with a private member
    let base_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("BaseDef");
    let base_id = graph.add_owned_element(base_def, pkg_id.clone(), VisibilityKind::Public);

    let private_part = Element::new_with_kind(ElementKind::PartUsage).with_name("privatePart");
    let _private_id =
        graph.add_owned_element(private_part, base_id.clone(), VisibilityKind::Private);

    // Also add a public member
    let public_part = Element::new_with_kind(ElementKind::PartUsage).with_name("publicPart");
    let public_id = graph.add_owned_element(public_part, base_id.clone(), VisibilityKind::Public);

    // Create DerivedDef that specializes BaseDef
    let derived_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("DerivedDef");
    let derived_id = graph.add_owned_element(derived_def, pkg_id.clone(), VisibilityKind::Public);

    create_specialization(&mut graph, &derived_id, "TestPkg::BaseDef");

    let mut ctx = graph.resolution_context();

    // Public member should be inherited
    let resolved_public = ctx.resolve_name(&derived_id, "publicPart");
    assert_eq!(resolved_public, Some(public_id));

    // Private member should NOT be inherited
    let resolved_private = ctx.resolve_name(&derived_id, "privatePart");
    assert!(
        resolved_private.is_none(),
        "Private members should not be inherited"
    );
}

// === Main Resolution Pass Tests (Phase 2d.4) ===

#[test]
fn resolve_references_specialization() {
    let mut graph = ModelGraph::new();

    // Create Package with BaseDef and DerivedDef
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    let base_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("BaseDef");
    let base_id = graph.add_owned_element(base_def, pkg_id.clone(), VisibilityKind::Public);

    let derived_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("DerivedDef");
    let derived_id = graph.add_owned_element(derived_def, pkg_id.clone(), VisibilityKind::Public);

    // Add specialization with unresolved reference
    create_specialization(&mut graph, &derived_id, "TestPkg::BaseDef");

    // Run resolution
    let result = resolve_references(&mut graph);

    // Should have resolved the specialization
    assert_eq!(result.resolved_count, 1);
    assert_eq!(result.unresolved_count, 0);
    assert!(!result.has_errors());

    // Check that the resolved property was set
    let specs: Vec<_> = graph
        .owned_members(&derived_id)
        .filter(|e| e.kind == ElementKind::Specialization)
        .collect();
    assert_eq!(specs.len(), 1);

    let general = specs[0].props.get(resolved_props::GENERAL);
    assert!(general.is_some(), "general property should be set");
    assert_eq!(general.and_then(|v| v.as_ref()), Some(&base_id));
}

#[test]
fn resolve_references_unresolved_reports_error() {
    let mut graph = ModelGraph::new();

    // Create Package with only DerivedDef (no BaseDef to resolve to)
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    let derived_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("DerivedDef");
    let derived_id = graph.add_owned_element(derived_def, pkg_id.clone(), VisibilityKind::Public);

    // Add specialization referencing non-existent type
    create_specialization(&mut graph, &derived_id, "TestPkg::NonExistent");

    // Run resolution
    let result = resolve_references(&mut graph);

    // Should have unresolved reference
    assert_eq!(result.resolved_count, 0);
    assert_eq!(result.unresolved_count, 1);
    assert!(result.has_errors());
}

#[test]
fn resolve_references_feature_typing() {
    use crate::Value;

    let mut graph = ModelGraph::new();

    // Create Package with TypeDef and a feature with typing
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    let type_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("TypeDef");
    let type_id = graph.add_owned_element(type_def, pkg_id.clone(), VisibilityKind::Public);

    // Create a FeatureTyping element with unresolved type
    let mut typing = Element::new_with_kind(ElementKind::FeatureTyping);
    typing.set_prop(
        unresolved_props::TYPE,
        Value::String("TestPkg::TypeDef".to_string()),
    );
    let _typing_id = graph.add_owned_element(typing, pkg_id.clone(), VisibilityKind::Public);

    // Run resolution
    let result = resolve_references(&mut graph);

    // Should have resolved the typing
    assert_eq!(result.resolved_count, 1);
    assert_eq!(result.unresolved_count, 0);

    // Find the FeatureTyping element and check resolved property
    let typings: Vec<_> = graph
        .owned_members(&pkg_id)
        .filter(|e| e.kind == ElementKind::FeatureTyping)
        .collect();
    assert_eq!(typings.len(), 1);

    let resolved_type = typings[0].props.get(resolved_props::TYPE);
    assert!(resolved_type.is_some());
    assert_eq!(resolved_type.and_then(|v| v.as_ref()), Some(&type_id));
}

#[test]
fn resolve_references_multiple_elements() {
    use crate::Value;

    let mut graph = ModelGraph::new();

    // Create Package with multiple types and relationships
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    let base_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("BaseDef");
    let base_id = graph.add_owned_element(base_def, pkg_id.clone(), VisibilityKind::Public);

    let type_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("TypeDef");
    let type_id = graph.add_owned_element(type_def, pkg_id.clone(), VisibilityKind::Public);

    let derived_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("DerivedDef");
    let derived_id = graph.add_owned_element(derived_def, pkg_id.clone(), VisibilityKind::Public);

    // Add specialization
    create_specialization(&mut graph, &derived_id, "TestPkg::BaseDef");

    // Add FeatureTyping
    let mut typing = Element::new_with_kind(ElementKind::FeatureTyping);
    typing.set_prop(
        unresolved_props::TYPE,
        Value::String("TestPkg::TypeDef".to_string()),
    );
    graph.add_owned_element(typing, pkg_id.clone(), VisibilityKind::Public);

    // Run resolution
    let result = resolve_references(&mut graph);

    // Should have resolved both
    assert_eq!(result.resolved_count, 2);
    assert_eq!(result.unresolved_count, 0);
    assert!(!result.has_errors());

    // Verify Specialization was resolved
    let specs: Vec<_> = graph
        .owned_members(&derived_id)
        .filter(|e| e.kind == ElementKind::Specialization)
        .collect();
    assert_eq!(
        specs[0]
            .props
            .get(resolved_props::GENERAL)
            .and_then(|v| v.as_ref()),
        Some(&base_id)
    );

    // Verify FeatureTyping was resolved
    let typings: Vec<_> = graph
        .owned_members(&pkg_id)
        .filter(|e| e.kind == ElementKind::FeatureTyping)
        .collect();
    assert_eq!(
        typings[0]
            .props
            .get(resolved_props::TYPE)
            .and_then(|v| v.as_ref()),
        Some(&type_id)
    );
}

#[test]
fn resolve_references_partial_success() {
    use crate::Value;

    let mut graph = ModelGraph::new();

    // Create Package with one resolvable and one unresolvable reference
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    let base_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("BaseDef");
    graph.add_owned_element(base_def, pkg_id.clone(), VisibilityKind::Public);

    let derived_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("DerivedDef");
    let derived_id = graph.add_owned_element(derived_def, pkg_id.clone(), VisibilityKind::Public);

    // Add resolvable specialization
    create_specialization(&mut graph, &derived_id, "TestPkg::BaseDef");

    // Add unresolvable FeatureTyping
    let mut typing = Element::new_with_kind(ElementKind::FeatureTyping);
    typing.set_prop(
        unresolved_props::TYPE,
        Value::String("TestPkg::NonExistent".to_string()),
    );
    graph.add_owned_element(typing, pkg_id.clone(), VisibilityKind::Public);

    // Run resolution
    let result = resolve_references(&mut graph);

    // Should have one resolved and one unresolved
    assert_eq!(result.resolved_count, 1);
    assert_eq!(result.unresolved_count, 1);
    assert!(result.has_errors());
}

// === Standard Library Tests (Phase 2d.5) ===

#[test]
fn register_library_package() {
    let mut graph = ModelGraph::new();

    // Create a library package (like Base)
    let lib_pkg = Element::new_with_kind(ElementKind::Package).with_name("Base");
    let lib_id = graph.add_element(lib_pkg);

    // Register it as a library package
    assert!(graph.register_library_package(lib_id.clone()));
    assert!(graph.is_library_package(&lib_id));
    assert_eq!(graph.library_packages().len(), 1);

    // Unregister
    assert!(graph.unregister_library_package(&lib_id));
    assert!(!graph.is_library_package(&lib_id));
}

#[test]
fn register_non_root_fails() {
    let mut graph = ModelGraph::new();

    // Create a parent package
    let parent = Element::new_with_kind(ElementKind::Package).with_name("Parent");
    let parent_id = graph.add_element(parent);

    // Create a child package
    let child = Element::new_with_kind(ElementKind::Package).with_name("Child");
    let child_id = graph.add_owned_element(child, parent_id.clone(), VisibilityKind::Public);

    // Registering a non-root package should fail
    assert!(!graph.register_library_package(child_id.clone()));
    assert!(!graph.is_library_package(&child_id));
}

#[test]
fn resolve_from_library_package() {
    let mut graph = ModelGraph::new();

    // Create library package "Base" with member "Anything"
    let base_pkg = Element::new_with_kind(ElementKind::Package).with_name("Base");
    let base_id = graph.add_element(base_pkg);
    graph.register_library_package(base_id.clone());

    let anything = Element::new_with_kind(ElementKind::Classifier).with_name("Anything");
    let anything_id = graph.add_owned_element(anything, base_id.clone(), VisibilityKind::Public);

    // Create a user package
    let user_pkg = Element::new_with_kind(ElementKind::Package).with_name("UserPkg");
    let user_id = graph.add_element(user_pkg);

    // Build the library name index — production code does this in
    // `resolve_references`; tests that bypass that path must do it themselves.
    graph.ensure_library_index();

    // Resolve "Anything" from user package - should find it in library
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&user_id, "Anything");

    assert_eq!(resolved, Some(anything_id));
}

#[test]
fn user_takes_precedence_over_library() {
    let mut graph = ModelGraph::new();

    // Create library package with "MyType"
    let lib_pkg = Element::new_with_kind(ElementKind::Package).with_name("Lib");
    let lib_id = graph.add_element(lib_pkg);
    graph.register_library_package(lib_id.clone());

    let lib_type = Element::new_with_kind(ElementKind::Classifier).with_name("MyType");
    let _lib_type_id = graph.add_owned_element(lib_type, lib_id.clone(), VisibilityKind::Public);

    // Create user package with its own "MyType"
    let user_pkg = Element::new_with_kind(ElementKind::Package).with_name("UserPkg");
    let user_id = graph.add_element(user_pkg);

    let user_type = Element::new_with_kind(ElementKind::Classifier).with_name("MyType");
    let user_type_id = graph.add_owned_element(user_type, user_id.clone(), VisibilityKind::Public);

    // Resolve "MyType" from user package - should get user's, not library's
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&user_id, "MyType");

    assert_eq!(
        resolved,
        Some(user_type_id),
        "User definitions take precedence over library"
    );
}

#[test]
fn resolve_library_package_by_name() {
    let mut graph = ModelGraph::new();

    // Create library package "Base"
    let base_pkg = Element::new_with_kind(ElementKind::Package).with_name("Base");
    let base_id = graph.add_element(base_pkg);
    graph.register_library_package(base_id.clone());

    // Create a user package
    let user_pkg = Element::new_with_kind(ElementKind::Package).with_name("UserPkg");
    let user_id = graph.add_element(user_pkg);

    // Resolve "Base" from user package - should find the library package itself
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&user_id, "Base");

    assert_eq!(resolved, Some(base_id));
}

#[test]
fn library_private_members_not_visible() {
    let mut graph = ModelGraph::new();

    // Create library package with private member
    let lib_pkg = Element::new_with_kind(ElementKind::Package).with_name("Lib");
    let lib_id = graph.add_element(lib_pkg);
    graph.register_library_package(lib_id.clone());

    let private_type = Element::new_with_kind(ElementKind::Classifier).with_name("PrivateType");
    let _private_id =
        graph.add_owned_element(private_type, lib_id.clone(), VisibilityKind::Private);

    let public_type = Element::new_with_kind(ElementKind::Classifier).with_name("PublicType");
    let public_id = graph.add_owned_element(public_type, lib_id.clone(), VisibilityKind::Public);

    // Create a user package
    let user_pkg = Element::new_with_kind(ElementKind::Package).with_name("UserPkg");
    let user_id = graph.add_element(user_pkg);

    graph.ensure_library_index();
    let mut ctx = graph.resolution_context();

    // Public member should be visible
    let resolved_public = ctx.resolve_name(&user_id, "PublicType");
    assert_eq!(resolved_public, Some(public_id));

    // Private member should NOT be visible
    let resolved_private = ctx.resolve_name(&user_id, "PrivateType");
    assert!(
        resolved_private.is_none(),
        "Library private members should not be visible"
    );
}

#[test]
fn add_library_package_convenience() {
    let mut graph = ModelGraph::new();

    // Use convenience method to add library package
    let lib_pkg = Element::new_with_kind(ElementKind::Package).with_name("Base");
    let lib_id = graph.add_library_package(lib_pkg);

    assert!(graph.is_library_package(&lib_id));
    assert!(graph.get_element(&lib_id).is_some());
}

#[test]
fn merge_as_library() {
    // Create library graph
    let mut lib_graph = ModelGraph::new();
    let base_pkg = Element::new_with_kind(ElementKind::Package).with_name("Base");
    let base_id = lib_graph.add_element(base_pkg);

    let anything = Element::new_with_kind(ElementKind::Classifier).with_name("Anything");
    let anything_id =
        lib_graph.add_owned_element(anything, base_id.clone(), VisibilityKind::Public);

    // Create user graph
    let mut user_graph = ModelGraph::new();
    let user_pkg = Element::new_with_kind(ElementKind::Package).with_name("UserPkg");
    let user_id = user_graph.add_element(user_pkg);

    // Merge library into user graph
    let count = user_graph.merge(lib_graph, true);
    assert!(count > 0);

    // Rebuild indexes after merge
    user_graph.rebuild_indexes();
    user_graph.ensure_library_index();

    // Verify library package was registered
    assert!(user_graph.is_library_package(&base_id));

    // Resolve "Anything" from user package
    let mut ctx = user_graph.resolution_context();
    let resolved = ctx.resolve_name(&user_id, "Anything");

    assert_eq!(resolved, Some(anything_id));
}

#[test]
fn multiple_library_packages() {
    let mut graph = ModelGraph::new();

    // Create multiple library packages
    let base_pkg = Element::new_with_kind(ElementKind::Package).with_name("Base");
    let base_id = graph.add_element(base_pkg);
    graph.register_library_package(base_id.clone());

    let anything = Element::new_with_kind(ElementKind::Classifier).with_name("Anything");
    let anything_id = graph.add_owned_element(anything, base_id.clone(), VisibilityKind::Public);

    let scalar_pkg = Element::new_with_kind(ElementKind::Package).with_name("ScalarValues");
    let scalar_id = graph.add_element(scalar_pkg);
    graph.register_library_package(scalar_id.clone());

    let integer = Element::new_with_kind(ElementKind::DataType).with_name("Integer");
    let integer_id = graph.add_owned_element(integer, scalar_id.clone(), VisibilityKind::Public);

    // Create user package
    let user_pkg = Element::new_with_kind(ElementKind::Package).with_name("UserPkg");
    let user_id = graph.add_element(user_pkg);

    graph.ensure_library_index();

    // Resolve from both library packages
    let mut ctx = graph.resolution_context();
    assert_eq!(ctx.resolve_name(&user_id, "Anything"), Some(anything_id));
    assert_eq!(ctx.resolve_name(&user_id, "Integer"), Some(integer_id));
    assert_eq!(ctx.resolve_name(&user_id, "Base"), Some(base_id));
    assert_eq!(ctx.resolve_name(&user_id, "ScalarValues"), Some(scalar_id));
}

// === Feature Chain Resolution Tests ===

#[test]
fn test_is_feature_chain() {
    // Pure feature chains (contain '.' outside quotes, no '::')
    assert!(ResolutionContext::is_feature_chain("a.b"));
    assert!(ResolutionContext::is_feature_chain("a.b.c"));
    assert!(ResolutionContext::is_feature_chain(
        "vehicle.engine.pistons"
    ));

    // Not feature chains
    assert!(!ResolutionContext::is_feature_chain("A::B"));
    assert!(!ResolutionContext::is_feature_chain("A::B::C"));
    assert!(!ResolutionContext::is_feature_chain("simple"));
    assert!(!ResolutionContext::is_feature_chain("'a.b'")); // Dot inside quotes
    assert!(!ResolutionContext::is_feature_chain("'some.path'")); // All inside quotes
                                                                  // Mixed qualified name with dot - NOT a pure feature chain
    assert!(!ResolutionContext::is_feature_chain("A::B.c"));
    assert!(!ResolutionContext::is_feature_chain(
        "Package::Type.feature"
    ));
}

#[test]
fn test_split_feature_chain_segments() {
    // Simple cases
    let segments: Vec<_> = ResolutionContext::split_feature_chain_segments("a.b.c").collect();
    assert_eq!(segments, vec!["a", "b", "c"]);

    let segments: Vec<_> = ResolutionContext::split_feature_chain_segments("x.y").collect();
    assert_eq!(segments, vec!["x", "y"]);

    // Single segment (no dots)
    let segments: Vec<_> = ResolutionContext::split_feature_chain_segments("single").collect();
    assert_eq!(segments, vec!["single"]);

    // Quoted names with dots inside
    let segments: Vec<_> = ResolutionContext::split_feature_chain_segments("'a.b'.c").collect();
    assert_eq!(segments, vec!["'a.b'", "c"]);

    let segments: Vec<_> = ResolutionContext::split_feature_chain_segments("a.'b.c'.d").collect();
    assert_eq!(segments, vec!["a", "'b.c'", "d"]);
}

#[test]
fn test_resolve_feature_chain_simple() {
    use crate::Value;

    let mut graph = ModelGraph::new();

    // Create Engine type with a 'pistons' feature
    let engine_type = Element::new_with_kind(ElementKind::PartDefinition).with_name("Engine");
    let engine_type_id = graph.add_element(engine_type);

    // Use with_owner() + add_element() to populate owner_to_children index
    // (required for children_of() which is used by resolve_feature_in_type)
    let pistons = Element::new_with_kind(ElementKind::PartUsage)
        .with_name("pistons")
        .with_owner(engine_type_id.clone());
    let pistons_id = graph.add_element(pistons);

    // Create Vehicle package with an 'engine' feature typed by Engine
    let vehicle_pkg = Element::new_with_kind(ElementKind::Package).with_name("VehiclePkg");
    let vehicle_pkg_id = graph.add_element(vehicle_pkg);

    // Use add_owned_element for engine feature (creates membership for scope resolution)
    let engine_feature = Element::new_with_kind(ElementKind::PartUsage).with_name("engine");
    let engine_feature_id = graph.add_owned_element(
        engine_feature,
        vehicle_pkg_id.clone(),
        VisibilityKind::Public,
    );

    // FeatureTyping: engine : Engine
    let mut typing = Element::new_with_kind(ElementKind::FeatureTyping);
    typing.set_prop("typedFeature", Value::Ref(engine_feature_id.clone()));
    typing.set_prop("type", Value::Ref(engine_type_id.clone()));
    graph.add_element(typing);

    // Resolve "engine.pistons" from VehiclePkg
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_feature_chain(&vehicle_pkg_id, "engine.pistons");

    assert_eq!(resolved, Some(pistons_id));
}

#[test]
fn test_resolve_feature_chain_via_qualified_name() {
    use crate::Value;

    let mut graph = ModelGraph::new();

    // Create Engine type with a 'pistons' feature
    let engine_type = Element::new_with_kind(ElementKind::PartDefinition).with_name("Engine");
    let engine_type_id = graph.add_element(engine_type);

    // Use with_owner() + add_element() to populate owner_to_children index
    let pistons = Element::new_with_kind(ElementKind::PartUsage)
        .with_name("pistons")
        .with_owner(engine_type_id.clone());
    let pistons_id = graph.add_element(pistons);

    // Create Vehicle package with an 'engine' feature typed by Engine
    let vehicle_pkg = Element::new_with_kind(ElementKind::Package).with_name("VehiclePkg");
    let vehicle_pkg_id = graph.add_element(vehicle_pkg);

    // Use add_owned_element for engine feature (creates membership for scope resolution)
    let engine_feature = Element::new_with_kind(ElementKind::PartUsage).with_name("engine");
    let engine_feature_id = graph.add_owned_element(
        engine_feature,
        vehicle_pkg_id.clone(),
        VisibilityKind::Public,
    );

    // FeatureTyping: engine : Engine
    let mut typing = Element::new_with_kind(ElementKind::FeatureTyping);
    typing.set_prop("typedFeature", Value::Ref(engine_feature_id.clone()));
    typing.set_prop("type", Value::Ref(engine_type_id.clone()));
    graph.add_element(typing);

    // resolve_qualified_name should automatically use feature chaining for "engine.pistons"
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_qualified_name(&vehicle_pkg_id, "engine.pistons");

    assert_eq!(resolved, Some(pistons_id));
}

#[test]
fn test_resolve_feature_chain_not_found() {
    let mut graph = ModelGraph::new();

    // Create a simple package with a feature
    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    // Use add_owned_element to create proper membership relationship
    let feature = Element::new_with_kind(ElementKind::PartUsage).with_name("myFeature");
    graph.add_owned_element(feature, pkg_id.clone(), VisibilityKind::Public);

    // Try to resolve a non-existent chain
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_feature_chain(&pkg_id, "myFeature.nonExistent");

    // Should fail because myFeature has no type
    assert!(resolved.is_none());
}

#[test]
fn test_resolve_feature_chain_first_segment_not_found() {
    let graph = ModelGraph::new();
    let fake_id = ElementId::new_v4();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_feature_chain(&fake_id, "nonExistent.field");

    assert!(resolved.is_none());
}

// === FI-2: Cross-Package Inheritance Tests ===

/// Helper to create a Specialization with a RESOLVED general reference.
/// This is the key fix: when the general reference is already resolved to an ElementId,
/// we should use it directly instead of re-resolving the name.
fn create_resolved_specialization(
    graph: &mut ModelGraph,
    owner_id: &ElementId,
    resolved_general_id: &ElementId,
) -> ElementId {
    use crate::Value;

    let mut spec = Element::new_with_kind(ElementKind::Specialization);
    spec.set_prop(
        resolved_props::GENERAL,
        Value::Ref(resolved_general_id.clone()),
    );

    graph.add_owned_element(spec, owner_id.clone(), VisibilityKind::Public)
}

#[test]
fn fi2_cross_package_inheritance_uses_resolved_id() {
    // FI-2: Test that when PackageB::Derived :> PackageA::Base is already resolved,
    // we use the resolved ElementId directly instead of extracting "Base" and
    // failing to re-resolve it in PackageB's scope.
    let mut graph = ModelGraph::new();

    // Create PackageA with BaseDef and an inherited member
    let pkg_a = Element::new_with_kind(ElementKind::Package).with_name("PackageA");
    let pkg_a_id = graph.add_element(pkg_a);

    let base_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("BaseDef");
    let base_id = graph.add_owned_element(base_def, pkg_a_id.clone(), VisibilityKind::Public);

    let inherited_feature =
        Element::new_with_kind(ElementKind::PartUsage).with_name("inheritedFeature");
    let inherited_id =
        graph.add_owned_element(inherited_feature, base_id.clone(), VisibilityKind::Public);

    // Create PackageB with Derived that specializes PackageA::BaseDef
    // Note: NO import from PackageA - this is the key scenario!
    let pkg_b = Element::new_with_kind(ElementKind::Package).with_name("PackageB");
    let pkg_b_id = graph.add_element(pkg_b);

    let derived_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("Derived");
    let derived_id = graph.add_owned_element(derived_def, pkg_b_id.clone(), VisibilityKind::Public);

    // Add specialization with RESOLVED reference (not unresolved name)
    // This simulates what happens after first-pass resolution has linked Derived to BaseDef
    create_resolved_specialization(&mut graph, &derived_id, &base_id);

    // From Derived, resolve "inheritedFeature" - should find it via inheritance
    // even though BaseDef is in a different package with no import
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&derived_id, "inheritedFeature");

    assert_eq!(
        resolved,
        Some(inherited_id),
        "FI-2: Should resolve inherited member using the resolved ElementId directly, \
         not by extracting the name and re-resolving (which would fail without an import)"
    );
}

#[test]
fn fi2_deep_cross_package_inheritance_chain() {
    // FI-2: Test deep inheritance chain across packages:
    // PackageC::Grandchild :> PackageB::Child :> PackageA::Base
    let mut graph = ModelGraph::new();

    // Package A: Base with a feature
    let pkg_a = Element::new_with_kind(ElementKind::Package).with_name("PackageA");
    let pkg_a_id = graph.add_element(pkg_a);

    let base = Element::new_with_kind(ElementKind::PartDefinition).with_name("Base");
    let base_id = graph.add_owned_element(base, pkg_a_id.clone(), VisibilityKind::Public);

    let base_feature = Element::new_with_kind(ElementKind::PartUsage).with_name("baseFeature");
    let base_feature_id =
        graph.add_owned_element(base_feature, base_id.clone(), VisibilityKind::Public);

    // Package B: Child :> Base
    let pkg_b = Element::new_with_kind(ElementKind::Package).with_name("PackageB");
    let pkg_b_id = graph.add_element(pkg_b);

    let child = Element::new_with_kind(ElementKind::PartDefinition).with_name("Child");
    let child_id = graph.add_owned_element(child, pkg_b_id.clone(), VisibilityKind::Public);

    create_resolved_specialization(&mut graph, &child_id, &base_id);

    // Package C: Grandchild :> Child
    let pkg_c = Element::new_with_kind(ElementKind::Package).with_name("PackageC");
    let pkg_c_id = graph.add_element(pkg_c);

    let grandchild = Element::new_with_kind(ElementKind::PartDefinition).with_name("Grandchild");
    let grandchild_id =
        graph.add_owned_element(grandchild, pkg_c_id.clone(), VisibilityKind::Public);

    create_resolved_specialization(&mut graph, &grandchild_id, &child_id);

    // From Grandchild, resolve "baseFeature" - should traverse 2 levels of inheritance
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&grandchild_id, "baseFeature");

    assert_eq!(
        resolved,
        Some(base_feature_id),
        "FI-2: Should resolve inherited member through deep cross-package inheritance chain"
    );
}

#[test]
fn fi2_inheritance_depth_limit() {
    // Test that the depth limit prevents infinite recursion
    let mut graph = ModelGraph::new();

    let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPkg");
    let pkg_id = graph.add_element(pkg);

    // Create a self-referential specialization (pathological case)
    let def = Element::new_with_kind(ElementKind::PartDefinition).with_name("SelfRef");
    let def_id = graph.add_owned_element(def, pkg_id.clone(), VisibilityKind::Public);

    // Add a member
    let feature = Element::new_with_kind(ElementKind::PartUsage).with_name("feature");
    graph.add_owned_element(feature, def_id.clone(), VisibilityKind::Public);

    // Create circular specialization (SelfRef :> SelfRef)
    // This should be handled by the visited set, but depth limit is extra protection
    create_resolved_specialization(&mut graph, &def_id, &def_id);

    // Should not hang or crash due to infinite recursion
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_name(&def_id, "feature");

    // Should find the feature (owned member found before inheritance)
    assert!(
        resolved.is_some(),
        "Should not crash on circular inheritance"
    );
}

// =============================================================================
// $:: Global Qualification Resolution Tests
// Spec: KerMLExpressions.xtext:541-551
// =============================================================================

#[test]
fn resolve_global_qualified_name_from_nested_scope() {
    // Create: TopPackage::SubPackage::Element
    // Resolve "$::TopPackage::SubPackage::Element" from SubPackage (nested scope)
    let (graph, _pkg_id, sub_id, part_id) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_qualified_name(&sub_id, "$::TestPackage::SubPackage::PartDef");

    assert_eq!(
        resolved,
        Some(part_id),
        "$:: should anchor resolution to root, finding TestPackage::SubPackage::PartDef"
    );
}

#[test]
fn resolve_global_qualified_single_segment() {
    // Resolve "$::TestPackage" from a nested scope
    let (graph, pkg_id, sub_id, _) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_qualified_name(&sub_id, "$::TestPackage");

    assert_eq!(
        resolved,
        Some(pkg_id),
        "$::TestPackage should resolve to root TestPackage"
    );
}

#[test]
fn resolve_global_qualified_not_found() {
    // Resolve "$::NonExistent::Path" should fail
    let (graph, _, sub_id, _) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_qualified_name(&sub_id, "$::NonExistent::Path");

    assert!(
        resolved.is_none(),
        "$::NonExistent::Path should not resolve"
    );
}

#[test]
fn resolve_global_qualified_bypasses_local_scope() {
    // Create a model where local scope has a name that shadows a root package
    let mut graph = ModelGraph::new();

    // Root-level "Pkg" package
    let root_pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
    let root_pkg_id = graph.add_element(root_pkg);

    // Element inside root Pkg
    let root_elem = Element::new_with_kind(ElementKind::PartDefinition).with_name("RootElement");
    let root_elem_id =
        graph.add_owned_element(root_elem, root_pkg_id.clone(), VisibilityKind::Public);

    // Outer package containing a nested "Pkg" that shadows the root one
    let outer = Element::new_with_kind(ElementKind::Package).with_name("Outer");
    let outer_id = graph.add_element(outer);

    let shadow_pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
    let shadow_pkg_id =
        graph.add_owned_element(shadow_pkg, outer_id.clone(), VisibilityKind::Public);

    // Element inside shadow Pkg (different from root one)
    let shadow_elem =
        Element::new_with_kind(ElementKind::PartDefinition).with_name("ShadowElement");
    let _shadow_elem_id =
        graph.add_owned_element(shadow_elem, shadow_pkg_id.clone(), VisibilityKind::Public);

    // From inside Outer, "$::Pkg::RootElement" should find the root Pkg, NOT the shadow
    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_qualified_name(&outer_id, "$::Pkg::RootElement");

    assert_eq!(
        resolved,
        Some(root_elem_id),
        "$:: should bypass local 'Pkg' shadow and resolve from root"
    );
}

#[test]
fn resolve_global_qualified_via_relative() {
    // Test that resolve_qualified_name_relative also handles $::
    let (graph, _pkg_id, sub_id, part_id) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved =
        ctx.resolve_qualified_name_relative(&sub_id, "$::TestPackage::SubPackage::PartDef");

    assert_eq!(
        resolved,
        Some(part_id),
        "$:: in relative resolution should still anchor to root"
    );
}

#[test]
fn resolve_global_qualified_via_feature_reference() {
    // Test that resolve_feature_reference also handles $::
    let (graph, _pkg_id, sub_id, part_id) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_feature_reference(&sub_id, "$::TestPackage::SubPackage::PartDef");

    assert_eq!(
        resolved,
        Some(part_id),
        "$:: in feature reference resolution should anchor to root"
    );
}

#[test]
fn resolve_without_global_qualification_still_works() {
    // Sanity check: regular qualified names without $:: still work
    let (graph, pkg_id, _, part_id) = create_test_hierarchy();

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_qualified_name(&pkg_id, "SubPackage::PartDef");

    assert_eq!(
        resolved,
        Some(part_id),
        "Regular qualified names without $:: should still work"
    );
}

// === Inherited-feature redefinition through Subclassification (ADR-016 P1) ===
//
// Regression coverage for the bug where `:>> redefinedFeature` of an *inherited*
// feature failed to resolve because `search_supertypes_recursive` read the
// `general` property for every specialization-family relationship. A `:> Super`
// on a definition is lowered to a `Subclassification`, whose target lives in the
// `superclassifier` property, so the supertype was never followed and the search
// stopped before reaching the grandparent that actually declares the feature.

/// Helper to create a Subclassification element (`Def :> Super`).
fn create_subclassification(
    graph: &mut ModelGraph,
    owner_id: &ElementId,
    superclassifier_ref: &str,
) -> ElementId {
    use crate::Value;

    let mut sub = Element::new_with_kind(ElementKind::Subclassification);
    sub.set_prop(
        unresolved_props::SUPERCLASSIFIER,
        Value::String(superclassifier_ref.to_string()),
    );

    graph.add_owned_element(sub, owner_id.clone(), VisibilityKind::Public)
}

/// Build a three-level inheritance chain Grand <- Base <- Derived using
/// `Subclassification` relationships, where Grand declares `feature`, and
/// Derived redefines that inherited `feature`. Returns the graph plus the ids
/// of (Derived def, the redefining feature, the inherited Grand feature).
///
/// `super_ref_fn` produces the supertype reference string for each level so we
/// can exercise both simple-name and qualified-name supertype references.
fn build_three_level_redefinition(
    super_ref_fn: impl Fn(&str) -> String,
) -> (ModelGraph, ElementId, ElementId, ElementId) {
    let mut graph = ModelGraph::new();

    let pkg = Element::new_with_kind(ElementKind::Package).with_name("MeasurementRefs");
    let pkg_id = graph.add_element(pkg);

    // Grand: declares the inherited feature.
    let grand =
        Element::new_with_kind(ElementKind::AttributeDefinition).with_name("VectorMeasurementRef");
    let grand_id = graph.add_owned_element(grand, pkg_id.clone(), VisibilityKind::Public);
    let grand_feature = Element::new_with_kind(ElementKind::AttributeUsage).with_name("dimensions");
    let grand_feature_id =
        graph.add_owned_element(grand_feature, grand_id.clone(), VisibilityKind::Public);

    // Base :> Grand
    let base =
        Element::new_with_kind(ElementKind::AttributeDefinition).with_name("ScalarMeasurementRef");
    let base_id = graph.add_owned_element(base, pkg_id.clone(), VisibilityKind::Public);
    create_subclassification(&mut graph, &base_id, &super_ref_fn("VectorMeasurementRef"));

    // Derived :> Base, redefining the inherited `dimensions`.
    let derived =
        Element::new_with_kind(ElementKind::AttributeDefinition).with_name("MeasurementUnit");
    let derived_id = graph.add_owned_element(derived, pkg_id.clone(), VisibilityKind::Public);
    create_subclassification(
        &mut graph,
        &derived_id,
        &super_ref_fn("ScalarMeasurementRef"),
    );

    // The redefining feature `dimensions` owns a Redefinition element.
    let redef_feature = Element::new_with_kind(ElementKind::AttributeUsage).with_name("dimensions");
    let redef_feature_id =
        graph.add_owned_element(redef_feature, derived_id.clone(), VisibilityKind::Public);
    create_redefinition(&mut graph, &redef_feature_id, "dimensions");

    (graph, derived_id, redef_feature_id, grand_feature_id)
}

#[test]
fn redefinition_resolves_inherited_feature_via_subclassification() {
    // Simple-name supertype references (`:> ScalarMeasurementRef`).
    let (mut graph, _derived_id, redef_feature_id, grand_feature_id) =
        build_three_level_redefinition(|name| name.to_string());

    let mut ctx = graph.resolution_context();
    // scope_id for the Redefinition is its owner: the redefining feature.
    let resolved = ctx.resolve_redefined_feature(&redef_feature_id, "dimensions");

    assert_eq!(
        resolved,
        Some(grand_feature_id),
        "Redefinition must resolve to the grandparent's inherited feature via the \
         supertype (Subclassification) chain, not the redefining feature itself"
    );
}

#[test]
fn redefinition_resolves_inherited_feature_via_qualified_subclassification() {
    // Qualified supertype references (`:> MeasurementRefs::ScalarMeasurementRef`),
    // as appears in real geometry/stdlib models.
    let (mut graph, _derived_id, redef_feature_id, grand_feature_id) =
        build_three_level_redefinition(|name| format!("MeasurementRefs::{name}"));

    let mut ctx = graph.resolution_context();
    let resolved = ctx.resolve_redefined_feature(&redef_feature_id, "dimensions");

    assert_eq!(
        resolved,
        Some(grand_feature_id),
        "Redefinition must follow qualified-name supertypes through the \
         Subclassification chain to the inherited feature"
    );
}

#[test]
fn redefinition_inherited_feature_resolves_through_full_pass() {
    // End-to-end: run the real two-pass resolver and confirm the Redefinition's
    // resolved `redefinedFeature` is the inherited grandparent feature (proving
    // resolution succeeds without relying on a global/library bare-name sweep:
    // `dimensions` is never an owned/global member of the Derived def's scope).
    let (mut graph, derived_id, _redef_feature_id, grand_feature_id) =
        build_three_level_redefinition(|name| name.to_string());

    let result = resolve_references(&mut graph);
    assert_eq!(
        result.unresolved_count, 0,
        "all references including the inherited redefinition should resolve"
    );

    // Find the Redefinition element under the Derived def's redefining feature
    // and confirm it points at the grandparent feature.
    let redef = graph
        .descendants(&derived_id)
        .into_iter()
        .find(|e| e.kind == ElementKind::Redefinition)
        .expect("redefinition element should exist");
    let resolved_target = redef
        .props
        .get(resolved_props::REDEFINED_FEATURE)
        .and_then(|v| v.as_ref())
        .cloned();
    assert_eq!(
        resolved_target,
        Some(grand_feature_id),
        "resolved redefinedFeature must be the inherited grandparent feature"
    );
}

/// Helper to create a Subsetting element (`:>` on a feature/usage).
fn create_subsetting(
    graph: &mut ModelGraph,
    owner_id: &ElementId,
    subsetted_ref: &str,
) -> ElementId {
    use crate::Value;

    let mut sub = Element::new_with_kind(ElementKind::Subsetting);
    sub.set_prop(
        unresolved_props::SUBSETTED_FEATURE,
        Value::String(subsetted_ref.to_string()),
    );

    graph.add_owned_element(sub, owner_id.clone(), VisibilityKind::Public)
}

// ===== P1.5: inherited-feature resolution survives the import gate =====
//
// These exercise the broadened inheritance index / supertype walk: a bare-name
// reference (subsetting, or any feature reference) to an INHERITED feature must
// resolve through the supertype chain WITH the bare-library gate ON — i.e. NOT
// via the library member sweep that P2 removes. The supertype chain is built
// from `Subclassification` (`:>` on a definition), whose target lives in
// `superclassifier`, exactly the relationship the old `general`-only index
// dropped.

/// Build a two-level Subclassification chain `Derived :> Base`, where `Base`
/// declares an inherited feature `baseFeature` that `Derived` does NOT own or
/// redefine. Returns (graph, Derived def id, inherited `baseFeature` id).
///
/// Because `Derived` neither owns nor redefines `baseFeature`, the only way to
/// resolve the bare name `baseFeature` from inside `Derived` is through the
/// `Subclassification` (`:>`) supertype — the relationship whose target lives in
/// `superclassifier`, which the old `general`-only inheritance index dropped.
fn build_subclassification_inherited_feature() -> (ModelGraph, ElementId, ElementId) {
    use crate::Value;

    let mut graph = ModelGraph::new();

    let pkg = Element::new_with_kind(ElementKind::Package).with_name("InheritPkg");
    let pkg_id = graph.add_element(pkg);

    // Base declares `baseFeature`.
    let base = Element::new_with_kind(ElementKind::AttributeDefinition).with_name("BaseDef");
    let base_id = graph.add_owned_element(base, pkg_id.clone(), VisibilityKind::Public);
    let base_feature = Element::new_with_kind(ElementKind::AttributeUsage).with_name("baseFeature");
    let base_feature_id =
        graph.add_owned_element(base_feature, base_id.clone(), VisibilityKind::Public);

    // Derived :> Base (Subclassification). Store the supertype reference under
    // `superclassifier` (NOT `general`) — this is exactly the relationship the
    // old `general`-only inheritance index dropped. The target is pre-resolved
    // (as pass 1 leaves it before feature references are resolved), so the
    // inheritance index can pick it up.
    let derived = Element::new_with_kind(ElementKind::AttributeDefinition).with_name("DerivedDef");
    let derived_id = graph.add_owned_element(derived, pkg_id.clone(), VisibilityKind::Public);
    let mut sub = Element::new_with_kind(ElementKind::Subclassification);
    sub.set_prop(resolved_props::SUPERCLASSIFIER, Value::Ref(base_id.clone()));
    graph.add_owned_element(sub, derived_id.clone(), VisibilityKind::Public);

    (graph, derived_id, base_feature_id)
}

#[test]
fn subsetting_resolves_inherited_feature_via_subclassification_gated() {
    // A usage inside Derived subsets the inherited `baseFeature` by bare name.
    // With the gate ON, this must resolve through the Subclassification chain
    // (not the soon-to-be-removed library member sweep).
    let (mut graph, derived_id, base_feature_id) = build_subclassification_inherited_feature();

    let subsetter = Element::new_with_kind(ElementKind::AttributeUsage).with_name("derivedSubset");
    let subsetter_id =
        graph.add_owned_element(subsetter, derived_id.clone(), VisibilityKind::Public);
    create_subsetting(&mut graph, &subsetter_id, "baseFeature");

    // Resolve the subsetting reference the way pass 2 does, WITH the gate ON.
    // scope is the subsetting relationship's owner (the subsetter feature).
    let mut ctx = graph.resolution_context().with_bare_library_gate(true);
    let resolved = ctx.resolve_feature_reference(&subsetter_id, "baseFeature");

    assert_eq!(
        resolved,
        Some(base_feature_id),
        "Subsetting of an inherited feature must resolve through the \
         Subclassification supertype chain with the bare-library gate ON, \
         not via the library member sweep"
    );
}

#[test]
fn inherited_member_resolves_through_subclassification_chain_gated() {
    // Exercise plain `resolve_name` (the path subsetting and every simple feature
    // reference funnels through). The inherited-member tier of Derived's scope
    // table must contain `baseFeature` reached via the Subclassification chain —
    // proving the inheritance index now follows `superclassifier`, not just
    // `general`.
    let (mut graph, derived_id, base_feature_id) = build_subclassification_inherited_feature();

    let mut ctx = graph.resolution_context().with_bare_library_gate(true);
    let resolved = ctx.resolve_name(&derived_id, "baseFeature");

    assert_eq!(
        resolved,
        Some(base_feature_id),
        "an inherited member reached via a Subclassification (:>) chain must be \
         in the inherited scope tier with the gate ON"
    );
}

#[test]
fn inherited_feature_resolves_across_fallback_graph_chain_gated() {
    use crate::Value;

    // Reproduce the real stdlib shape: a definition in the FILE graph whose
    // supertype chain crosses into a LIBRARY (fallback) graph, where the
    // inherited feature is declared two hops up. Mirrors
    //   `TensorQuantityValue :> Array`,  `Array :> OrderedCollection :> Collection`
    // with `Collection` declaring `elements` — all in `Collections.kerml`.

    // --- Library (fallback) graph: Collection { elements } <- OrderedCollection <- Array
    let mut lib = ModelGraph::new();
    let lib_pkg = Element::new_with_kind(ElementKind::Package).with_name("Collections");
    let lib_pkg_id = lib.add_element(lib_pkg);

    let collection = Element::new_with_kind(ElementKind::DataType).with_name("Collection");
    let collection_id =
        lib.add_owned_element(collection, lib_pkg_id.clone(), VisibilityKind::Public);
    let elements = Element::new_with_kind(ElementKind::Feature).with_name("elements");
    let elements_id =
        lib.add_owned_element(elements, collection_id.clone(), VisibilityKind::Public);

    let ordered = Element::new_with_kind(ElementKind::DataType).with_name("OrderedCollection");
    let ordered_id = lib.add_owned_element(ordered, lib_pkg_id.clone(), VisibilityKind::Public);
    // OrderedCollection :> Collection (resolved target — library is resolved up front).
    let mut oc_sub = Element::new_with_kind(ElementKind::Subclassification);
    oc_sub.set_prop(
        resolved_props::SUPERCLASSIFIER,
        Value::Ref(collection_id.clone()),
    );
    lib.add_owned_element(oc_sub, ordered_id.clone(), VisibilityKind::Public);

    let array = Element::new_with_kind(ElementKind::DataType).with_name("Array");
    let array_id = lib.add_owned_element(array, lib_pkg_id.clone(), VisibilityKind::Public);
    let mut arr_sub = Element::new_with_kind(ElementKind::Subclassification);
    arr_sub.set_prop(
        resolved_props::SUPERCLASSIFIER,
        Value::Ref(ordered_id.clone()),
    );
    lib.add_owned_element(arr_sub, array_id.clone(), VisibilityKind::Public);

    // --- File graph: TensorQuantityValue :> Array, with a usage subsetting `elements`.
    let mut file = ModelGraph::new();
    let tqv =
        Element::new_with_kind(ElementKind::AttributeDefinition).with_name("TensorQuantityValue");
    let tqv_id = file.add_element(tqv);
    // TensorQuantityValue :> Array — already resolved to the library `Array`.
    let mut tqv_sub = Element::new_with_kind(ElementKind::Subclassification);
    tqv_sub.set_prop(
        resolved_props::SUPERCLASSIFIER,
        Value::Ref(array_id.clone()),
    );
    file.add_owned_element(tqv_sub, tqv_id.clone(), VisibilityKind::Public);

    let num = Element::new_with_kind(ElementKind::AttributeUsage).with_name("num");
    let num_id = file.add_owned_element(num, tqv_id.clone(), VisibilityKind::Public);
    create_subsetting(&mut file, &num_id, "elements");

    // Resolve `elements` from the file usage, gate ON, with the library as the
    // fallback graph. Must walk File(Array) -> Lib(OrderedCollection) ->
    // Lib(Collection) and find `elements`.
    let mut ctx = ResolutionContext::new_with_fallback(&file, &lib).with_bare_library_gate(true);
    let resolved = ctx.resolve_feature_reference(&num_id, "elements");

    assert_eq!(
        resolved,
        Some(elements_id),
        "an inherited feature declared in the fallback (library) graph two hops \
         up a cross-graph Subclassification chain must resolve with the gate ON"
    );
}

// ===== P1.6: user-model `:>` supertype + inherited feature resolve via the
// full two-pass driver =====
//
// The P1.5 tests above pre-resolve `superclassifier`, so they never exercised
// pass 1's Subclassification dispatch. The bug P1.6 fixed lived precisely there:
// the pass-1 dispatch `match` checked the generic `Specialization` arm before the
// more-specific `Subclassification` arm. Because `Subclassification` is a subtype
// of `Specialization`, it was routed to `resolve_specialization` (which reads
// `unresolved_general`) instead of `resolve_subclassification` (which reads
// `unresolved_superclassifier`). The result: a user-model `part def Car :>
// Vehicle` produced ZERO resolution updates, its supertype edge never entered the
// inheritance index, and `Vehicle`'s inherited members were invisible to `Car`.
//
// These tests build the graph the way the parser leaves it — with the supertype
// name UNRESOLVED — and run the real `resolve_references` driver end to end.

/// Build `package P { part def Vehicle { part engine; } part def Car :> Vehicle
/// { part motor :> engine; } }` with both the Subclassification superclassifier
/// and the Subsetting subsettedFeature left UNRESOLVED (as the parser emits them).
/// Returns (graph, Subclassification id, Subsetting id, Vehicle::engine id).
fn build_unresolved_user_model_inheritance() -> (ModelGraph, ElementId, ElementId, ElementId) {
    use crate::Value;

    let mut graph = ModelGraph::new();

    let pkg = Element::new_with_kind(ElementKind::Package).with_name("P");
    let pkg_id = graph.add_element(pkg);

    // part def Vehicle { part engine; }
    let vehicle = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
    let vehicle_id = graph.add_owned_element(vehicle, pkg_id.clone(), VisibilityKind::Public);
    let engine = Element::new_with_kind(ElementKind::PartUsage).with_name("engine");
    let engine_id = graph.add_owned_element(engine, vehicle_id.clone(), VisibilityKind::Public);

    // part def Car :> Vehicle  — superclassifier UNRESOLVED (bare name "Vehicle").
    let car = Element::new_with_kind(ElementKind::PartDefinition).with_name("Car");
    let car_id = graph.add_owned_element(car, pkg_id.clone(), VisibilityKind::Public);
    let mut sub = Element::new_with_kind(ElementKind::Subclassification);
    sub.set_prop(
        unresolved_props::SUPERCLASSIFIER,
        Value::String("Vehicle".to_string()),
    );
    let sub_id = graph.add_owned_element(sub, car_id.clone(), VisibilityKind::Public);

    // part motor :> engine  — subsettedFeature UNRESOLVED (bare name "engine").
    let motor = Element::new_with_kind(ElementKind::PartUsage).with_name("motor");
    let motor_id = graph.add_owned_element(motor, car_id.clone(), VisibilityKind::Public);
    let subsetting_id = create_subsetting(&mut graph, &motor_id, "engine");

    (graph, sub_id, subsetting_id, engine_id)
}

#[test]
fn driver_resolves_user_model_subclassification_superclassifier() {
    // Pass 1 must dispatch the Subclassification to `resolve_subclassification`
    // (reading `unresolved_superclassifier`), not to `resolve_specialization`.
    let (mut graph, sub_id, _subsetting_id, _engine_id) = build_unresolved_user_model_inheritance();
    let vehicle_id = graph
        .lookup_by_name("Vehicle")
        .first()
        .cloned()
        .expect("Vehicle present");

    let _ = resolve_references(&mut graph);

    let sub = graph
        .get_element(&sub_id)
        .expect("subclassification present");
    let resolved = sub
        .props
        .get(resolved_props::SUPERCLASSIFIER)
        .and_then(|v| v.as_ref())
        .cloned();
    assert_eq!(
        resolved,
        Some(vehicle_id),
        "a user-model `part def Car :> Vehicle` must resolve its superclassifier \
         through the full driver (pass-1 Subclassification dispatch)"
    );
}

#[test]
fn driver_resolves_inherited_feature_subset_in_user_model() {
    // End-to-end: `motor :> engine` where `engine` is inherited by `Car` from
    // `Vehicle` via an (initially unresolved) `:>`. This failed before P1.6
    // because the supertype edge never reached the inheritance index.
    let (mut graph, _sub_id, subsetting_id, engine_id) = build_unresolved_user_model_inheritance();

    let result = resolve_references(&mut graph);

    assert_eq!(
        result.unresolved_count, 0,
        "no references should be left unresolved in the user model"
    );

    let subsetting = graph
        .get_element(&subsetting_id)
        .expect("subsetting present");
    let resolved = subsetting
        .props
        .get(resolved_props::SUBSETTED_FEATURE)
        .and_then(|v| v.as_ref())
        .cloned();
    assert_eq!(
        resolved,
        Some(engine_id),
        "`motor :> engine` must resolve to the inherited `Vehicle::engine` via \
         the supertype chain established in pass 1"
    );
}

#[test]
fn driver_resolves_inherited_feature_subset_in_user_model_gated() {
    // Same as above but proving the resolution does NOT depend on the
    // bare-library member sweep: it must hold with the import gate ON, since the
    // supertype edge and inherited member are entirely within the user model.
    let (mut graph, _sub_id, subsetting_id, engine_id) = build_unresolved_user_model_inheritance();

    // Drive both passes manually with the gate ON (the public `resolve_references`
    // wrapper does not expose the gate flag; the gate only affects the library
    // sweep tier, which this user-model case must never need).
    let result = {
        let (updates, res) = resolve_references_pure(&graph);
        apply_resolution_updates(&mut graph, &updates);
        res
    };
    // Re-resolve a usage reference through a gated context to confirm the
    // inherited member is reachable without the library sweep.
    let mut ctx = graph.resolution_context().with_bare_library_gate(true);
    let motor_owner = graph
        .get_element(&subsetting_id)
        .and_then(|e| e.owner.clone())
        .expect("subsetting owner present");
    let gated = ctx.resolve_feature_reference(&motor_owner, "engine");

    assert_eq!(result.unresolved_count, 0, "user model fully resolves");
    assert_eq!(
        gated,
        Some(engine_id),
        "the inherited `engine` must resolve via the supertype chain with the \
         bare-library gate ON (not the library member sweep)"
    );
}

// === Ambiguity Diagnostic Tests (ADR-016 D5) ===

/// Build a user model whose `UserPackage` imports two distinct libraries that
/// each export a `Widget`, plus a Specialization referencing the bare name
/// `Widget`. Returns `(file_graph, user_pkg_id, spec_id, widget_a_id, widget_b_id)`.
///
/// Everything lives in one graph; the *fallback* graph is what selects the user
/// (vs. library self-resolution) code path, so the discriminator is whether a
/// fallback is supplied to the driver — not which graph holds the candidates.
fn build_ambiguous_import_user_model() -> (ModelGraph, ElementId, ElementId, ElementId, ElementId) {
    let mut graph = ModelGraph::new();

    let lib_a = Element::new_with_kind(ElementKind::Package).with_name("LibA");
    let lib_a_id = graph.add_element(lib_a);
    let widget_a = Element::new_with_kind(ElementKind::PartDefinition).with_name("Widget");
    let widget_a_id = graph.add_owned_element(widget_a, lib_a_id.clone(), VisibilityKind::Public);

    let lib_b = Element::new_with_kind(ElementKind::Package).with_name("LibB");
    let lib_b_id = graph.add_element(lib_b);
    let widget_b = Element::new_with_kind(ElementKind::PartDefinition).with_name("Widget");
    let widget_b_id = graph.add_owned_element(widget_b, lib_b_id.clone(), VisibilityKind::Public);

    assert_ne!(widget_a_id, widget_b_id, "two distinct Widget elements");

    let user = Element::new_with_kind(ElementKind::Package).with_name("UserPackage");
    let user_id = graph.add_element(user);

    // import LibA::*; import LibB::*  — both bring in `Widget`.
    create_import(&mut graph, &user_id, "LibA", true, false);
    create_import(&mut graph, &user_id, "LibB", true, false);

    // A Specialization owned by UserPackage referencing the ambiguous bare name.
    let spec_id = create_specialization(&mut graph, &user_id, "Widget");

    (graph, user_id, spec_id, widget_a_id, widget_b_id)
}

#[test]
fn ambiguity_flagged_for_user_namespace() {
    let (file_graph, _user_id, spec_id, widget_a_id, widget_b_id) =
        build_ambiguous_import_user_model();

    // An empty fallback graph is enough to put resolution on the USER path
    // (the discriminator is `fallback_graph.is_some()`).
    let library_graph = ModelGraph::new();
    let exclude_ids = FxHashSet::default();
    let (updates, result) =
        resolve_references_with_fallback_pure(&file_graph, &library_graph, &exclude_ids);

    // 1. An E201 ambiguity diagnostic mentioning `Widget` is emitted.
    let e201: Vec<&Diagnostic> = result
        .diagnostics
        .iter()
        .filter(|d| d.code.as_deref() == Some("E201"))
        .collect();
    assert_eq!(
        e201.len(),
        1,
        "exactly one E201 expected, got {:?}",
        result.diagnostics.iter().collect::<Vec<_>>()
    );
    assert!(
        e201[0].message.contains("Widget"),
        "E201 message should mention the ambiguous name: {}",
        e201[0].message
    );

    // 2. Resolution picked the DETERMINISTIC minimum candidate id.
    let expected_pick = std::cmp::min(widget_a_id.clone(), widget_b_id.clone());
    let resolved_general = updates
        .iter()
        .find(|u| u.element_id == spec_id && u.property_name == resolved_props::GENERAL)
        .map(|u| u.resolved_value.clone());
    assert_eq!(
        resolved_general,
        Some(expected_pick),
        "ambiguous `Widget` must resolve to the minimum (deterministic) candidate id"
    );
}

#[test]
fn ambiguity_not_flagged_without_fallback() {
    // Same model, but resolved via the NO-fallback path (library self-resolution
    // scope). The ~83 benign stdlib cross-file re-export collisions live here and
    // must never be flagged — so no E201 may appear.
    let (file_graph, _user_id, _spec_id, _a, _b) = build_ambiguous_import_user_model();

    let (_updates, result) = resolve_references_pure(&file_graph);

    let e201_count = result
        .diagnostics
        .iter()
        .filter(|d| d.code.as_deref() == Some("E201"))
        .count();
    assert_eq!(
        e201_count, 0,
        "the no-fallback (library self-resolution) path must never emit E201"
    );
}

#[test]
fn ambiguity_deterministic_across_runs() {
    // The picked id and the candidate ordering in the diagnostic must be
    // identical across independent resolution runs.
    let (file_graph, _user_id, spec_id, _a, _b) = build_ambiguous_import_user_model();
    let library_graph = ModelGraph::new();
    let exclude_ids = FxHashSet::default();

    let run = || {
        let (updates, result) =
            resolve_references_with_fallback_pure(&file_graph, &library_graph, &exclude_ids);
        let pick = updates
            .iter()
            .find(|u| u.element_id == spec_id && u.property_name == resolved_props::GENERAL)
            .map(|u| u.resolved_value.clone());
        let msg = result
            .diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("E201"))
            .map(|d| d.message.clone());
        (pick, msg)
    };

    let first = run();
    let second = run();
    assert!(first.0.is_some(), "a deterministic pick must be recorded");
    assert!(first.1.is_some(), "an E201 message must be recorded");
    assert_eq!(
        first, second,
        "the resolved pick and ambiguity message must be stable across runs"
    );
}
