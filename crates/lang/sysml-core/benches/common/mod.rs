//! Shared test data generators for sysml-core resolution benchmarks.

use sysml_core::{Element, ElementKind, ModelGraph, Value};
use sysml_id::ElementId;

/// Create a graph with library packages containing N members.
///
/// Structure:
/// - LibraryPackage1
///   - Type1, Type2, ..., TypeN
/// - LibraryPackage2
///   - Type1, Type2, ..., TypeN
pub fn create_library_graph(lib_packages: usize, members_per_package: usize) -> ModelGraph {
    let mut graph = ModelGraph::new();

    for p in 0..lib_packages {
        // Create library package
        let mut pkg = Element::new_with_kind(ElementKind::Package);
        pkg.name = Some(format!("LibPackage{}", p));
        let pkg_id = graph.add_element(pkg);
        graph.register_library_package(pkg_id.clone());

        // Add members to the package
        for m in 0..members_per_package {
            let mut member = Element::new_with_kind(ElementKind::PartDefinition);
            member.name = Some(format!("LibType{}_{}", p, m));
            member.owner = Some(pkg_id.clone());

            // Create owning membership
            let member_id = graph.add_element(member);
            let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
            membership.owner = Some(pkg_id.clone());
            membership.set_prop("memberElement", Value::Ref(member_id.clone()));
            membership.set_prop("memberName", Value::String(format!("LibType{}_{}", p, m)));
            membership.set_prop("visibility", Value::String("public".to_string()));
            graph.add_element(membership);
        }
    }

    graph
}

/// Create a graph with deeply nested library packages.
pub fn create_nested_library_graph(depth: usize, members_per_level: usize) -> ModelGraph {
    let mut graph = ModelGraph::new();

    // Create root library package
    let mut root_pkg = Element::new_with_kind(ElementKind::Package);
    root_pkg.name = Some("RootLib".to_string());
    let root_id = graph.add_element(root_pkg);
    graph.register_library_package(root_id.clone());

    let mut parent_id = root_id;

    for level in 0..depth {
        // Create nested package
        let mut nested_pkg = Element::new_with_kind(ElementKind::Package);
        nested_pkg.name = Some(format!("Level{}", level));
        nested_pkg.owner = Some(parent_id.clone());
        let nested_id = graph.add_element(nested_pkg.clone());

        // Add membership for nested package
        let mut pkg_membership = Element::new_with_kind(ElementKind::OwningMembership);
        pkg_membership.owner = Some(parent_id.clone());
        pkg_membership.set_prop("memberElement", Value::Ref(nested_id.clone()));
        pkg_membership.set_prop("memberName", Value::String(format!("Level{}", level)));
        pkg_membership.set_prop("visibility", Value::String("public".to_string()));
        graph.add_element(pkg_membership);

        // Add members at this level
        for m in 0..members_per_level {
            let mut member = Element::new_with_kind(ElementKind::PartDefinition);
            member.name = Some(format!("Type_L{}_{}", level, m));
            member.owner = Some(nested_id.clone());
            let member_id = graph.add_element(member);

            let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
            membership.owner = Some(nested_id.clone());
            membership.set_prop("memberElement", Value::Ref(member_id.clone()));
            membership.set_prop(
                "memberName",
                Value::String(format!("Type_L{}_{}", level, m)),
            );
            membership.set_prop("visibility", Value::String("public".to_string()));
            graph.add_element(membership);
        }

        parent_id = nested_id;
    }

    graph
}

/// Create a graph with inheritance chains of given depth.
///
/// Structure:
/// - BaseType0
/// - DerivedType1 :> BaseType0
/// - DerivedType2 :> DerivedType1
/// - ...
pub fn create_inheritance_graph(
    definitions: usize,
    inheritance_depth: usize,
) -> (ModelGraph, Vec<ElementId>) {
    let mut graph = ModelGraph::new();
    let mut def_ids = Vec::new();

    // Create a container package
    let mut pkg = Element::new_with_kind(ElementKind::Package);
    pkg.name = Some("TestPackage".to_string());
    let pkg_id = graph.add_element(pkg);

    for chain in 0..(definitions / inheritance_depth.max(1)) {
        let mut parent_id: Option<ElementId> = None;

        for level in 0..inheritance_depth.min(definitions - chain * inheritance_depth) {
            // Create definition
            let mut def = Element::new_with_kind(ElementKind::PartDefinition);
            def.name = Some(format!("Type_{}_{}", chain, level));
            def.owner = Some(pkg_id.clone());
            let def_id = graph.add_element(def);
            def_ids.push(def_id.clone());

            // Create membership
            let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
            membership.owner = Some(pkg_id.clone());
            membership.set_prop("memberElement", Value::Ref(def_id.clone()));
            membership.set_prop(
                "memberName",
                Value::String(format!("Type_{}_{}", chain, level)),
            );
            membership.set_prop("visibility", Value::String("public".to_string()));
            graph.add_element(membership);

            // Add inherited features to parent (so inheritance lookup finds something)
            for f in 0..3 {
                let mut feature = Element::new_with_kind(ElementKind::PartUsage);
                feature.name = Some(format!("feature_{}_{}", level, f));
                feature.owner = Some(def_id.clone());
                let feature_id = graph.add_element(feature);

                let mut feat_membership = Element::new_with_kind(ElementKind::FeatureMembership);
                feat_membership.owner = Some(def_id.clone());
                feat_membership.set_prop("memberElement", Value::Ref(feature_id.clone()));
                feat_membership.set_prop(
                    "memberName",
                    Value::String(format!("feature_{}_{}", level, f)),
                );
                feat_membership.set_prop("visibility", Value::String("public".to_string()));
                graph.add_element(feat_membership);
            }

            // Create specialization if there's a parent
            if let Some(pid) = parent_id {
                let mut spec = Element::new_with_kind(ElementKind::Specialization);
                spec.owner = Some(def_id.clone());

                // Get parent name for unresolved reference
                let parent_name = graph
                    .get_element(&pid)
                    .and_then(|e| e.name.clone())
                    .unwrap_or_else(|| format!("{:.8}", pid));
                spec.set_prop("unresolved_general", Value::String(parent_name));
                graph.add_element(spec);
            }

            parent_id = Some(def_id);
        }
    }

    (graph, def_ids)
}

/// Create a graph with diamond inheritance pattern.
pub fn create_diamond_inheritance_graph(supertypes: usize) -> (ModelGraph, ElementId) {
    let mut graph = ModelGraph::new();

    // Create a container package
    let mut pkg = Element::new_with_kind(ElementKind::Package);
    pkg.name = Some("DiamondPackage".to_string());
    let pkg_id = graph.add_element(pkg);

    // Create base type at top of diamond
    let mut base = Element::new_with_kind(ElementKind::PartDefinition);
    base.name = Some("BaseType".to_string());
    base.owner = Some(pkg_id.clone());
    let base_id = graph.add_element(base);

    // Add features to base
    for f in 0..5 {
        let mut feature = Element::new_with_kind(ElementKind::PartUsage);
        feature.name = Some(format!("baseFeature{}", f));
        feature.owner = Some(base_id.clone());
        let feature_id = graph.add_element(feature);

        let mut membership = Element::new_with_kind(ElementKind::FeatureMembership);
        membership.owner = Some(base_id.clone());
        membership.set_prop("memberElement", Value::Ref(feature_id.clone()));
        membership.set_prop("memberName", Value::String(format!("baseFeature{}", f)));
        membership.set_prop("visibility", Value::String("public".to_string()));
        graph.add_element(membership);
    }

    let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
    membership.owner = Some(pkg_id.clone());
    membership.set_prop("memberElement", Value::Ref(base_id.clone()));
    membership.set_prop("memberName", Value::String("BaseType".to_string()));
    membership.set_prop("visibility", Value::String("public".to_string()));
    graph.add_element(membership);

    // Create intermediate types
    let mut intermediate_ids = Vec::new();
    for i in 0..supertypes {
        let mut intermediate = Element::new_with_kind(ElementKind::PartDefinition);
        intermediate.name = Some(format!("IntermediateType{}", i));
        intermediate.owner = Some(pkg_id.clone());
        let intermediate_id = graph.add_element(intermediate);
        intermediate_ids.push(intermediate_id.clone());

        // Add membership
        let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
        membership.owner = Some(pkg_id.clone());
        membership.set_prop("memberElement", Value::Ref(intermediate_id.clone()));
        membership.set_prop(
            "memberName",
            Value::String(format!("IntermediateType{}", i)),
        );
        membership.set_prop("visibility", Value::String("public".to_string()));
        graph.add_element(membership);

        // Specialize from base
        let mut spec = Element::new_with_kind(ElementKind::Specialization);
        spec.owner = Some(intermediate_id.clone());
        spec.set_prop("unresolved_general", Value::String("BaseType".to_string()));
        graph.add_element(spec);

        // Add unique features to intermediate
        for f in 0..3 {
            let mut feature = Element::new_with_kind(ElementKind::PartUsage);
            feature.name = Some(format!("intermediateFeature{}_{}", i, f));
            feature.owner = Some(intermediate_id.clone());
            let feature_id = graph.add_element(feature);

            let mut membership = Element::new_with_kind(ElementKind::FeatureMembership);
            membership.owner = Some(intermediate_id.clone());
            membership.set_prop("memberElement", Value::Ref(feature_id.clone()));
            membership.set_prop(
                "memberName",
                Value::String(format!("intermediateFeature{}_{}", i, f)),
            );
            membership.set_prop("visibility", Value::String("public".to_string()));
            graph.add_element(membership);
        }
    }

    // Create derived type that inherits from all intermediates
    let mut derived = Element::new_with_kind(ElementKind::PartDefinition);
    derived.name = Some("DerivedType".to_string());
    derived.owner = Some(pkg_id.clone());
    let derived_id = graph.add_element(derived);

    let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
    membership.owner = Some(pkg_id.clone());
    membership.set_prop("memberElement", Value::Ref(derived_id.clone()));
    membership.set_prop("memberName", Value::String("DerivedType".to_string()));
    membership.set_prop("visibility", Value::String("public".to_string()));
    graph.add_element(membership);

    // Add specializations for each intermediate
    for (i, _) in intermediate_ids.iter().enumerate() {
        let mut spec = Element::new_with_kind(ElementKind::Specialization);
        spec.owner = Some(derived_id.clone());
        spec.set_prop(
            "unresolved_general",
            Value::String(format!("IntermediateType{}", i)),
        );
        graph.add_element(spec);
    }

    (graph, derived_id)
}

/// Create a graph with import relationships.
pub fn create_import_graph(
    packages: usize,
    imports_per_package: usize,
    members_per_package: usize,
) -> ModelGraph {
    let mut graph = ModelGraph::new();
    let mut pkg_ids = Vec::new();

    // Create all packages first
    for p in 0..packages {
        let mut pkg = Element::new_with_kind(ElementKind::Package);
        pkg.name = Some(format!("Package{}", p));
        let pkg_id = graph.add_element(pkg);
        pkg_ids.push(pkg_id.clone());

        // Add members
        for m in 0..members_per_package {
            let mut member = Element::new_with_kind(ElementKind::PartDefinition);
            member.name = Some(format!("Type{}_{}", p, m));
            member.owner = Some(pkg_id.clone());
            let member_id = graph.add_element(member);

            let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
            membership.owner = Some(pkg_id.clone());
            membership.set_prop("memberElement", Value::Ref(member_id.clone()));
            membership.set_prop("memberName", Value::String(format!("Type{}_{}", p, m)));
            membership.set_prop("visibility", Value::String("public".to_string()));
            graph.add_element(membership);
        }
    }

    // Add imports between packages
    for p in 0..packages {
        for i in 0..imports_per_package.min(packages - 1) {
            let target_pkg = (p + i + 1) % packages;
            let target_name = format!("Package{}", target_pkg);

            let mut import = Element::new_with_kind(ElementKind::NamespaceImport);
            import.owner = Some(pkg_ids[p].clone());
            import.set_prop("importedReference", Value::String(target_name));
            import.set_prop("isNamespace", Value::Bool(true));
            import.set_prop("visibility", Value::String("private".to_string()));
            graph.add_element(import);
        }
    }

    graph
}

/// Create a graph with recursive imports (chain: A imports B imports C).
pub fn create_recursive_import_graph(
    depth: usize,
    members_per_package: usize,
) -> (ModelGraph, ElementId) {
    let mut graph = ModelGraph::new();
    let mut pkg_ids = Vec::new();

    // Create packages from deepest to shallowest
    for level in (0..depth).rev() {
        let mut pkg = Element::new_with_kind(ElementKind::Package);
        pkg.name = Some(format!("Level{}", level));
        let pkg_id = graph.add_element(pkg);
        pkg_ids.push(pkg_id.clone());

        // Add members
        for m in 0..members_per_package {
            let mut member = Element::new_with_kind(ElementKind::PartDefinition);
            member.name = Some(format!("TypeL{}_{}", level, m));
            member.owner = Some(pkg_id.clone());
            let member_id = graph.add_element(member);

            let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
            membership.owner = Some(pkg_id.clone());
            membership.set_prop("memberElement", Value::Ref(member_id.clone()));
            membership.set_prop("memberName", Value::String(format!("TypeL{}_{}", level, m)));
            membership.set_prop("visibility", Value::String("public".to_string()));
            graph.add_element(membership);
        }

        // Add import to next level if not the deepest
        if level < depth - 1 {
            let target_name = format!("Level{}", level + 1);
            let mut import = Element::new_with_kind(ElementKind::NamespaceImport);
            import.owner = Some(pkg_id.clone());
            import.set_prop("importedReference", Value::String(target_name));
            import.set_prop("isNamespace", Value::Bool(true));
            import.set_prop("visibility", Value::String("public".to_string()));
            graph.add_element(import);
        }
    }

    // Return the top-level package (Level0)
    let top_id = pkg_ids.last().unwrap().clone();
    (graph, top_id)
}

/// Create a realistic model combining all features.
pub fn create_realistic_model(
    definitions: usize,
    usages_per_def: usize,
    library_members: usize,
    inheritance_depth: usize,
) -> ModelGraph {
    let mut graph = ModelGraph::new();

    // Create library package
    let mut lib_pkg = Element::new_with_kind(ElementKind::Package);
    lib_pkg.name = Some("StandardLibrary".to_string());
    let lib_pkg_id = graph.add_element(lib_pkg);
    graph.register_library_package(lib_pkg_id.clone());

    // Add library members
    for m in 0..library_members {
        let mut member = Element::new_with_kind(ElementKind::PartDefinition);
        member.name = Some(format!("LibType{}", m));
        member.owner = Some(lib_pkg_id.clone());
        let member_id = graph.add_element(member);

        let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
        membership.owner = Some(lib_pkg_id.clone());
        membership.set_prop("memberElement", Value::Ref(member_id.clone()));
        membership.set_prop("memberName", Value::String(format!("LibType{}", m)));
        membership.set_prop("visibility", Value::String("public".to_string()));
        graph.add_element(membership);
    }

    // Create user package
    let mut user_pkg = Element::new_with_kind(ElementKind::Package);
    user_pkg.name = Some("UserModel".to_string());
    let user_pkg_id = graph.add_element(user_pkg);

    // Add import from library
    let mut import = Element::new_with_kind(ElementKind::NamespaceImport);
    import.owner = Some(user_pkg_id.clone());
    import.set_prop(
        "importedReference",
        Value::String("StandardLibrary".to_string()),
    );
    import.set_prop("isNamespace", Value::Bool(true));
    import.set_prop("visibility", Value::String("private".to_string()));
    graph.add_element(import);

    // Create definitions with inheritance
    let mut def_ids = Vec::new();
    for d in 0..definitions {
        let mut def = Element::new_with_kind(ElementKind::PartDefinition);
        def.name = Some(format!("UserType{}", d));
        def.owner = Some(user_pkg_id.clone());
        let def_id = graph.add_element(def);
        def_ids.push(def_id.clone());

        let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
        membership.owner = Some(user_pkg_id.clone());
        membership.set_prop("memberElement", Value::Ref(def_id.clone()));
        membership.set_prop("memberName", Value::String(format!("UserType{}", d)));
        membership.set_prop("visibility", Value::String("public".to_string()));
        graph.add_element(membership);

        // Add inheritance from previous definition or library type
        if d > 0 && d % inheritance_depth == 0 {
            // Type from library
            let lib_type_idx = d % library_members.max(1);
            let mut spec = Element::new_with_kind(ElementKind::Specialization);
            spec.owner = Some(def_id.clone());
            spec.set_prop(
                "unresolved_general",
                Value::String(format!("LibType{}", lib_type_idx)),
            );
            graph.add_element(spec);
        } else if d > 0 {
            // Type from previous definition
            let mut spec = Element::new_with_kind(ElementKind::Specialization);
            spec.owner = Some(def_id.clone());
            spec.set_prop(
                "unresolved_general",
                Value::String(format!("UserType{}", d - 1)),
            );
            graph.add_element(spec);
        }

        // Add usages
        for u in 0..usages_per_def {
            let mut usage = Element::new_with_kind(ElementKind::PartUsage);
            usage.name = Some(format!("usage{}_{}", d, u));
            usage.owner = Some(def_id.clone());

            // Type the usage with a library type or previous definition
            let type_ref = if u % 2 == 0 {
                format!("LibType{}", u % library_members.max(1))
            } else {
                format!("UserType{}", d.saturating_sub(1))
            };

            let usage_id = graph.add_element(usage);

            let mut membership = Element::new_with_kind(ElementKind::FeatureMembership);
            membership.owner = Some(def_id.clone());
            membership.set_prop("memberElement", Value::Ref(usage_id.clone()));
            membership.set_prop("memberName", Value::String(format!("usage{}_{}", d, u)));
            membership.set_prop("visibility", Value::String("public".to_string()));
            graph.add_element(membership);

            // Add feature typing
            let mut typing = Element::new_with_kind(ElementKind::FeatureTyping);
            typing.owner = Some(usage_id.clone());
            typing.set_prop("unresolved_type", Value::String(type_ref));
            graph.add_element(typing);
        }
    }

    graph
}
