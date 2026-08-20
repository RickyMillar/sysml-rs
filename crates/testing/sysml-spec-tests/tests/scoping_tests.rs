//! Minimal test cases for scoping strategies.
//!
//! These tests verify that each scoping strategy resolves names correctly
//! without requiring the full corpus. Each test focuses on a specific
//! scoping behavior.
//!
//! Run with:
//! ```bash
//! cargo test -p sysml-spec-tests scoping -- --nocapture
//! ```

use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

/// Parse SysML code and run resolution.
fn parse_and_resolve(code: &str) -> (usize, usize) {
    let parser = TreeSitterParser::new();
    let file = SysmlFile::new("test.sysml", code);
    let mut result = parser.parse(&[file]);
    let res = result.resolve();
    // Debug output for failed resolutions
    if res.unresolved_count > 0 {
        eprintln!("Unresolved references:");
        for diag in res.diagnostics.iter().filter(|d| d.is_error()) {
            eprintln!("  - {}", diag);
        }
    }
    (res.resolved_count, res.unresolved_count)
}

/// Check that a piece of code has no unresolved references.
fn assert_resolves(code: &str) {
    let (resolved, unresolved) = parse_and_resolve(code);
    assert_eq!(
        unresolved,
        0,
        "Expected all references to resolve, but {} unresolved out of {}",
        unresolved,
        resolved + unresolved
    );
}

#[allow(dead_code)]
/// Check that a piece of code has the expected resolution rate.
fn assert_resolution_rate(code: &str, expected_resolved: usize, expected_unresolved: usize) {
    let (resolved, unresolved) = parse_and_resolve(code);
    assert_eq!(
        (resolved, unresolved),
        (expected_resolved, expected_unresolved),
        "Expected {}/{} resolved/unresolved, got {}/{}",
        expected_resolved,
        expected_unresolved,
        resolved,
        unresolved
    );
}

// =============================================================================
// OwningNamespace Strategy Tests
// =============================================================================

mod owning_namespace {
    use super::*;

    #[test]
    fn local_type_reference() {
        // A part usage referencing a local type
        let code = r#"
            package Test {
                part def Vehicle;
                part car : Vehicle;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn nested_type_reference() {
        // Reference to a type in parent namespace
        let code = r#"
            package Test {
                part def Engine;
                package Inner {
                    part motor : Engine;
                }
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn specialization_reference() {
        // Specialization using :>
        let code = r#"
            package Test {
                part def Vehicle;
                part def Car :> Vehicle;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn subsetting_reference() {
        // Subsetting using :>
        let code = r#"
            package Test {
                part def Vehicle {
                    part engine;
                }
                part def Car :> Vehicle {
                    part motor :> engine;
                }
            }
        "#;
        assert_resolves(code);
    }
}

// =============================================================================
// NonExpressionNamespace Strategy Tests
// =============================================================================

mod non_expression_namespace {
    use super::*;

    #[test]
    fn feature_typing_in_part() {
        // FeatureTyping should skip expression scopes
        let code = r#"
            package Test {
                part def Engine;
                part car {
                    part engine : Engine;
                }
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn nested_feature_typing() {
        // Typing in deeply nested structure
        let code = r#"
            package Test {
                part def Cylinder;
                part def Engine {
                    part cylinders : Cylinder[4];
                }
                part car {
                    part engine : Engine {
                        part c : Cylinder;
                    }
                }
            }
        "#;
        assert_resolves(code);
    }
}

// =============================================================================
// RelativeNamespace Strategy Tests
// =============================================================================

mod relative_namespace {
    use super::*;

    #[test]
    #[ignore = "Relative namespace not yet implemented"]
    fn feature_chain_simple() {
        // Feature chain: vehicle.engine
        let code = r#"
            package Test {
                part def Engine;
                part def Vehicle {
                    part engine : Engine;
                }
                part car : Vehicle;

                // Reference to car.engine should work
                alias myEngine = car.engine;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    #[ignore = "Relative namespace not yet implemented"]
    fn feature_chain_multiple() {
        // Longer chain: vehicle.engine.cylinders
        let code = r#"
            package Test {
                part def Cylinder;
                part def Engine {
                    part cylinders : Cylinder[4];
                }
                part def Vehicle {
                    part engine : Engine;
                }
                part car : Vehicle;

                // Reference to car.engine.cylinders
                alias myCylinders = car.engine.cylinders;
            }
        "#;
        assert_resolves(code);
    }
}

// =============================================================================
// TransitionSpecific Strategy Tests
// =============================================================================

mod transition_specific {
    use super::*;

    #[test]
    #[ignore = "Transition scoping not yet implemented"]
    fn state_transition_trigger() {
        // Transition trigger should resolve in state context
        let code = r#"
            package Test {
                state def VehicleStates {
                    entry; then off;
                    state off;
                    state on;

                    transition off_to_on
                        first off
                        accept start
                        then on;
                }
            }
        "#;
        // This will fail until transition scoping is implemented
        assert_resolves(code);
    }
}

// =============================================================================
// Import Resolution Tests
// =============================================================================

mod imports {
    use super::*;

    #[test]
    fn import_single_element() {
        let code = r#"
            package Lib {
                part def Engine;
            }
            package Test {
                import Lib::Engine;
                part myEngine : Engine;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn import_namespace() {
        let code = r#"
            package Lib {
                part def Engine;
                part def Wheel;
            }
            package Test {
                import Lib::*;
                part myEngine : Engine;
                part myWheel : Wheel;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn qualified_reference() {
        let code = r#"
            package Lib {
                part def Engine;
            }
            package Test {
                part myEngine : Lib::Engine;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn cross_package_namespace_import() {
        // Package A defines types, Package B imports from A with ::*
        let code = r#"
            package MeasurementRefs {
                attribute def DerivedUnit;
                attribute def SimpleUnit;
            }
            package Quantities {
                import MeasurementRefs::*;
                attribute def MyUnit :> DerivedUnit;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn library_package_cross_import() {
        // Simulates standard library package cross-reference
        let code = r#"
            standard library package MeasurementRefs {
                attribute def DerivedUnit;
                attribute def DimensionOneValue;
            }
            standard library package ISQThermo {
                import MeasurementRefs::*;
                attribute def ThermalUnit :> DerivedUnit;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn private_import_namespace() {
        // Private imports should still make names visible within the package
        let code = r#"
            package Lib {
                attribute def BaseUnit;
            }
            package Test {
                private import Lib::*;
                attribute def MyUnit :> BaseUnit;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn nested_specialization_chain() {
        // Test inheritance chain where redefinition needs to find inherited feature
        let code = r#"
            package MeasurementRefs {
                abstract attribute def VectorMeasurementRef {
                    attribute dimensions;
                }
                abstract attribute def ScalarMeasurementRef :> VectorMeasurementRef {
                    attribute :>> dimensions = ();
                    attribute quantityDimension;
                }
                abstract attribute def MeasurementUnit :> ScalarMeasurementRef;
                abstract attribute def DerivedUnit :> MeasurementUnit;
            }
            package ISQThermo {
                import MeasurementRefs::*;
                attribute def ThermalResistanceUnit :> DerivedUnit {
                    attribute :>> quantityDimension;
                }
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn library_package_with_import_chain() {
        // Standard library packages with import chains
        let code = r#"
            standard library package Quantities {
                attribute def QuantityDimension;
            }
            standard library package MeasurementRefs {
                private import Quantities::*;
                abstract attribute def ScalarMeasurementRef {
                    attribute quantityDimension : QuantityDimension;
                }
            }
            standard library package ISQBase {
                private import MeasurementRefs::*;
                attribute def LengthUnit :> ScalarMeasurementRef;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn three_level_inheritance_with_redefinition() {
        // Simulates ISQ pattern: A <- B <- C <- D, redefine feature from A
        // Using plain `package` to test resolution (library packages have different lookup path)
        let code = r#"
            package MeasurementRefs {
                abstract attribute def VectorMeasurementRef {
                    attribute dimensions;
                    attribute isOrthogonal;
                }
                abstract attribute def ScalarMeasurementRef :> VectorMeasurementRef {
                    attribute :>> dimensions = ();
                    attribute :>> isOrthogonal = true;
                    attribute quantityDimension;
                    attribute mRefs;
                }
                abstract attribute def MeasurementUnit :> ScalarMeasurementRef;
                abstract attribute def DerivedUnit :> MeasurementUnit;
                abstract attribute def DimensionOneValue;
            }
            package ISQThermo {
                private import MeasurementRefs::*;
                attribute def ThermalUnit :> DerivedUnit {
                    attribute :>> quantityDimension;
                    attribute :>> mRefs;
                }
                attribute def ThermalValue :> DimensionOneValue;
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn three_level_inheritance_with_redefinition_library() {
        // Same as above but with standard library package
        // This tests the library package resolution path
        let code = r#"
            standard library package MeasurementRefs {
                abstract attribute def VectorMeasurementRef {
                    attribute dimensions;
                    attribute isOrthogonal;
                }
                abstract attribute def ScalarMeasurementRef :> VectorMeasurementRef {
                    attribute :>> dimensions = ();
                    attribute :>> isOrthogonal = true;
                    attribute quantityDimension;
                    attribute mRefs;
                }
                abstract attribute def MeasurementUnit :> ScalarMeasurementRef;
                abstract attribute def DerivedUnit :> MeasurementUnit;
                abstract attribute def DimensionOneValue;
            }
            standard library package ISQThermo {
                private import MeasurementRefs::*;
                attribute def ThermalUnit :> DerivedUnit {
                    attribute :>> quantityDimension;
                    attribute :>> mRefs;
                }
                attribute def ThermalValue :> DimensionOneValue;
            }
        "#;
        assert_resolves(code);
    }
}

// =============================================================================
// Inheritance Resolution Tests
// =============================================================================

mod inheritance {
    use super::*;

    #[test]
    #[ignore = "Inherited feature visibility requires inheritance resolution"]
    fn inherited_feature_visible() {
        // Features from supertypes should be visible
        let code = r#"
            package Test {
                part def Vehicle {
                    part engine;
                }
                part def Car :> Vehicle {
                    // Should see 'engine' from Vehicle
                    part turbocharged :> engine;
                }
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    #[ignore = "Redefinition hiding requires inheritance resolution"]
    fn redefinition_hides_inherited() {
        // Redefined feature should shadow inherited one
        let code = r#"
            package Test {
                part def Vehicle {
                    part engine;
                }
                part def ElectricCar :> Vehicle {
                    part motor :>> engine;  // Redefines engine
                }
            }
        "#;
        assert_resolves(code);
    }
}

// =============================================================================
// Edge Cases and Regression Tests
// =============================================================================

mod library_loading {
    use super::*;

    #[test]
    fn parse_measurement_references_minimal() {
        // Test parsing MeasurementReferences.sysml patterns
        let code = r#"
            standard library package MeasurementReferences {
                doc
                /*
                 * This is documentation.
                 */

                abstract attribute def ScalarMeasurementReference {
                    attribute :>> dimensions = ();
                    attribute :>> isOrthogonal = true;
                    attribute quantityDimension;
                }
                abstract attribute def MeasurementUnit :> ScalarMeasurementReference;
                abstract attribute def DerivedUnit :> MeasurementUnit;
            }
        "#;
        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", code);
        let result = parser.parse(&[file]);

        if result.has_errors() {
            for diag in result.diagnostics.iter() {
                eprintln!("Parse error: {}", diag);
            }
        }
        assert!(
            !result.has_errors(),
            "Parse failed with {} errors",
            result.error_count()
        );

        // Check DerivedUnit exists
        let derived_unit = result
            .graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some("DerivedUnit"));
        assert!(derived_unit.is_some(), "DerivedUnit not found");
    }

    #[test]
    fn parse_default_value() {
        // Test "default false" and "default true" syntax from MeasurementReferences.sysml
        let code = r#"
            package Test {
                attribute def A {
                    attribute isBound: Boolean[1] default false;
                    attribute isOrthogonal: Boolean[1] default true;
                }
            }
        "#;
        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", code);
        let result = parser.parse(&[file]);

        if result.has_errors() {
            for diag in result.diagnostics.iter() {
                eprintln!("Parse error: {}", diag);
            }
        }
        assert!(
            !result.has_errors(),
            "Parse failed with {} errors",
            result.error_count()
        );
    }

    #[test]
    fn parse_mref_self() {
        // Test "attribute :>> mRefs = self;" syntax
        let code = r#"
            package Test {
                attribute def A {
                    attribute :>> mRefs = self;
                }
            }
        "#;
        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", code);
        let result = parser.parse(&[file]);

        if result.has_errors() {
            for diag in result.diagnostics.iter() {
                eprintln!("Parse error: {}", diag);
            }
        }
        assert!(
            !result.has_errors(),
            "Parse failed with {} errors",
            result.error_count()
        );
    }

    #[test]
    fn parse_assert_constraint() {
        // Test "assert constraint { expr }" syntax from MeasurementReferences.sysml
        let code = r#"
            package Test {
                attribute def A {
                    attribute dims;
                    assert constraint { dims == 3 }
                }
            }
        "#;
        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", code);
        let result = parser.parse(&[file]);

        if result.has_errors() {
            for diag in result.diagnostics.iter() {
                eprintln!("Parse error: {}", diag);
            }
        }
        assert!(
            !result.has_errors(),
            "Parse failed with {} errors",
            result.error_count()
        );
    }

    #[test]
    fn parse_sequence_index_operator() {
        // Test sequence indexing with # operator
        let code = r#"
            package Test {
                attribute def A {
                    attribute dims;
                    assert constraint { dims#(1) == 3 }
                }
            }
        "#;
        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", code);
        let result = parser.parse(&[file]);

        if result.has_errors() {
            for diag in result.diagnostics.iter() {
                eprintln!("Parse error: {}", diag);
            }
        }
        assert!(
            !result.has_errors(),
            "Parse failed with {} errors",
            result.error_count()
        );
    }

    #[test]
    fn parse_arrow_for_all() {
        // Test ->forAll expression
        let code = r#"
            package Test {
                attribute def A {
                    attribute items;
                    assert constraint { items->forAll { in item; item == 0 } }
                }
            }
        "#;
        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", code);
        let result = parser.parse(&[file]);

        if result.has_errors() {
            for diag in result.diagnostics.iter() {
                eprintln!("Parse error: {}", diag);
            }
        }
        assert!(
            !result.has_errors(),
            "Parse failed with {} errors",
            result.error_count()
        );
    }

    #[test]
    fn parse_or_expression() {
        // Test "or" keyword in expressions (line 152 of MeasurementReferences.sysml)
        let code = r#"
            package Test {
                attribute def A {
                    attribute x;
                    attribute y;
                    assert constraint { x == 0 or y == 1 }
                }
            }
        "#;
        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", code);
        let result = parser.parse(&[file]);

        if result.has_errors() {
            for diag in result.diagnostics.iter() {
                eprintln!("Parse error: {}", diag);
            }
        }
        assert!(
            !result.has_errors(),
            "Parse failed with {} errors",
            result.error_count()
        );
    }

    #[test]
    fn parse_new_expression() {
        // Test "= new TypeName()" syntax (line 490 of MeasurementReferences.sysml).
        // The tree-sitter grammar supports the `new` constructor form
        // (KerMLExpressions.xtext rule `ConstructorExpression`); lowering mints a
        // distinct `ElementKind::ConstructorExpression` (F3 engine-gap closure).
        let code = r#"
            package Test {
                attribute def DimensionOneUnit;
                attribute one : DimensionOneUnit[1] = new DimensionOneUnit();
            }
        "#;
        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", code);
        let result = parser.parse(&[file]);

        if result.has_errors() {
            for diag in result.diagnostics.iter() {
                eprintln!("Parse error: {}", diag);
            }
        }
        assert!(
            !result.has_errors(),
            "Parse failed with {} errors",
            result.error_count()
        );
    }

    #[test]
    #[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
    fn parse_actual_measurement_references() {
        // Test parsing the actual MeasurementReferences.sysml file
        let Some(root) = sysml_spec_tests::try_find_references_dir() else {
            eprintln!("Skipping test: references directory not found");
            return;
        };
        let path = root.join(
            "SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.domain/Quantities and Units/MeasurementReferences.sysml",
        );
        if !path.exists() {
            eprintln!(
                "Skipping test: MeasurementReferences.sysml not found at {:?}",
                path
            );
            return;
        }
        let content = std::fs::read_to_string(&path).expect("Failed to read file");

        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("MeasurementReferences.sysml", &content);
        let result = parser.parse(&[file]);

        println!("Elements: {}", result.graph.element_count());
        if result.has_errors() {
            for diag in result.diagnostics.iter().take(10) {
                eprintln!("Parse error: {}", diag);
            }
        }

        // Check for MeasurementReferences package
        let meas_ref = result
            .graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some("MeasurementReferences"));
        println!("MeasurementReferences found: {}", meas_ref.is_some());

        // Don't assert - just report
        println!("Error count: {}", result.error_count());
    }

    #[test]
    fn parse_standard_library_package() {
        // Test that we can parse a standard library package
        let code = r#"
            standard library package MeasurementReferences {
                abstract attribute def TensorMeasurementReference;
                abstract attribute def DerivedUnit;
            }
        "#;
        let parser = TreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", code);
        let result = parser.parse(&[file]);

        assert!(
            !result.has_errors(),
            "Parse errors: {:?}",
            result.error_count()
        );

        // Check root package exists
        // Note: The parser creates some OwningMembership elements without owners,
        // which is a known parser issue. Filter for just packages.
        let root_packages: Vec<_> = result
            .graph
            .elements
            .values()
            .filter(|e| {
                e.owner.is_none()
                    && matches!(
                        e.kind,
                        sysml_core::ElementKind::Package | sysml_core::ElementKind::LibraryPackage
                    )
            })
            .collect();
        assert_eq!(root_packages.len(), 1, "Expected 1 root package element");
        assert_eq!(
            root_packages[0].name.as_deref(),
            Some("MeasurementReferences")
        );

        // Check it's a LibraryPackage
        assert!(
            root_packages[0].kind == sysml_core::ElementKind::Package
                || root_packages[0].kind == sysml_core::ElementKind::LibraryPackage,
            "Expected Package or LibraryPackage, got {:?}",
            root_packages[0].kind
        );
    }
}

mod edge_cases {
    use super::*;

    #[test]
    fn empty_package() {
        let code = r#"
            package Empty {
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn self_reference() {
        // A part that references itself (recursive structure)
        let code = r#"
            package Test {
                part def Node {
                    part children : Node[*];
                }
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn mutual_reference() {
        // Two types that reference each other
        let code = r#"
            package Test {
                part def A {
                    part b : B;
                }
                part def B {
                    part a : A;
                }
            }
        "#;
        assert_resolves(code);
    }

    #[test]
    fn shadowing() {
        // Local definition shadows outer one
        let code = r#"
            package Outer {
                part def Thing;
                package Inner {
                    part def Thing;  // Shadows Outer::Thing
                    part t : Thing;  // Should resolve to Inner::Thing
                }
            }
        "#;
        assert_resolves(code);
    }
}

// =============================================================================
// ADR-016 D5 — import-collision measurement (NOT a hard gate)
// =============================================================================
//
// Counts, across the whole corpus, how many distinct same-name import
// collisions occur (two+ DISTINCT element ids brought in under one name at the
// IMPORTED tier). This is instrumentation only: it never fails, it just prints
// the count + a sample so we can decide whether flipping
// `ScopedResolution::Ambiguous` to a hard diagnostic is safe.
//
// Run with:
//   SYSML_CORPUS_PATH=references/sysmlv2 \
//     cargo test -p sysml-spec-tests measure_import_collisions -- --ignored --nocapture
mod ambiguity_measurement {
    use super::*;
    use std::collections::BTreeMap;
    use sysml_spec_tests::{corpus::discover_corpus_files, CoverageConfig};

    #[test]
    #[ignore = "requires SYSML_CORPUS_PATH; measurement only, never gates"]
    fn measure_import_collisions() {
        let Some(config) = CoverageConfig::from_env() else {
            eprintln!("SYSML_CORPUS_PATH not set — skipping import-collision measurement");
            return;
        };

        let mut files = discover_corpus_files(&config);

        // Optional: also scan an arbitrary extra directory recursively (e.g. the
        // full Pilot stdlib at .../SysML-v2-Pilot-Implementation/sysml.library),
        // which is NOT part of the default corpus subdirs but is where heavy
        // re-export collisions would concentrate if they exist.
        if let Ok(extra) = std::env::var("SYSML_AMBIGUITY_EXTRA_DIR") {
            use std::path::Path;
            let mut extra_count = 0usize;
            for entry in walkdir::WalkDir::new(Path::new(&extra))
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let p = entry.path();
                if p.is_file() && p.extension().map_or(false, |e| e == "sysml") {
                    if let Ok(content) = std::fs::read_to_string(p) {
                        files.push(sysml_spec_tests::corpus::CorpusFile {
                            full_path: p.to_path_buf(),
                            relative_path: p.to_string_lossy().to_string(),
                            content,
                        });
                        extra_count += 1;
                    }
                }
            }
            eprintln!("[ambiguity] +{extra_count} extra files from {extra}");
        }

        // When SYSML_AMBIGUITY_COMBINED=1, parse+resolve ALL files together as
        // one multi-file graph. This exercises CROSS-FILE imports (e.g. stdlib
        // re-export chains) that isolated single-file resolution cannot see.
        let combined = std::env::var("SYSML_AMBIGUITY_COMBINED").is_ok();
        eprintln!(
            "[ambiguity] {} corpus files; mode = {}",
            files.len(),
            if combined {
                "COMBINED multi-file"
            } else {
                "per-file isolation"
            }
        );

        // name -> total distinct-collision occurrences across all namespaces/files
        let mut collisions: BTreeMap<String, usize> = BTreeMap::new();
        let mut total_collision_sites = 0usize;
        let mut files_with_collisions = 0usize;

        let parser = TreeSitterParser::new();

        let tally = |graph: &sysml_core::ModelGraph,
                     collisions: &mut BTreeMap<String, usize>,
                     total: &mut usize|
         -> bool {
            let ids: Vec<_> = graph.elements.keys().cloned().collect();
            let mut ctx = graph.resolution_context();
            let mut hit = false;
            for id in &ids {
                let table = ctx.get_full_scope_table(id);
                for (name, distinct_ids) in table.ambiguous_imported_iter() {
                    if distinct_ids.len() >= 2 {
                        *collisions.entry(name.clone()).or_default() += 1;
                        *total += 1;
                        hit = true;
                    }
                }
            }
            hit
        };

        if combined {
            let sysml_files: Vec<SysmlFile> = files
                .iter()
                .map(|f| SysmlFile::new(&f.relative_path, &f.content))
                .collect();
            let mut result = parser.parse(&sysml_files);
            let _ = result.resolve();
            if tally(&result.graph, &mut collisions, &mut total_collision_sites) {
                files_with_collisions = 1; // one combined graph
            }
        } else {
            for f in &files {
                let sf = SysmlFile::new(&f.relative_path, &f.content);
                let mut result = parser.parse(&[sf]);
                if result.has_errors() {
                    // Skip files that don't even parse — they can't be measured fairly.
                    continue;
                }
                let _ = result.resolve();
                if tally(&result.graph, &mut collisions, &mut total_collision_sites) {
                    files_with_collisions += 1;
                }
            }
        }

        eprintln!("=========================================================");
        eprintln!("[ambiguity] ADR-016 D5 import-collision measurement");
        eprintln!("[ambiguity] distinct colliding NAMES: {}", collisions.len());
        eprintln!(
            "[ambiguity] total collision SITES (name x namespace): {}",
            total_collision_sites
        );
        eprintln!(
            "[ambiguity] files exhibiting >=1 collision: {} / {}",
            files_with_collisions,
            files.len()
        );
        eprintln!("[ambiguity] sample colliding names (up to 30):");
        for (name, count) in collisions.iter().take(30) {
            eprintln!("    {name}  (x{count})");
        }
        eprintln!("=========================================================");

        // Measurement only — never gate.
    }
}

// =============================================================================
// ADR-016 P2 step 1 — import-gate migration measurement (NOT a hard gate)
// =============================================================================
//
// Runs resolution with the OPT-IN bare-library gate turned ON and reports the bare
// names that newly become unresolved, plus the `import Pkg::*;` that would fix
// each. This is the input to the migration pass (P2 step 2/3) and to the
// gate-flip decision (P2 step 4). It is measurement-only and never fails.
//
// The gate flag itself defaults OFF in `ResolutionContext`, so this measurement
// is the only thing that exercises gated resolution — production behavior is
// unchanged until the gate is flipped on in a separate, signed-off commit.
//
// MULTI-FILE (2026-05-28 re-scope). Each measured directory is loaded as ONE
// workspace: every file is parsed and merged into a single ModelGraph (mirroring
// production `load_files_from_dir`), elaborated ONCE with the stdlib as the linked
// fallback, then resolved against that combined graph. This is the honest model —
// the previous single-file proxy could not see sibling-file definitions, so a
// cross-file reference within our own models falsely read as "broken under the
// gate" (and elaborating per-file was O(files x lib), which dominated runtime).
//
// TWO SCOPES, reported separately:
//   * USER MODELS — tests/vis-coverage, tests/fixtures/shared,
//     editors/diagram/examples. This is the PRIMARY gate-flip-readiness metric:
//     the files an auto-import migration would actually touch.
//   * STDLIB — libraries/standard. A diagnostic only (it IS the stdlib, loaded as
//     the fallback graph, not user code to migrate). Reflective metamodel-
//     definition files (SysML.sysml / KerML.sysml — every `metadata def` models a
//     metaclass) are EXCLUDED: their bare refs (`ownedFeature`/`baseType`/…) point
//     at KerML metaclass features reached through the metamodel hierarchy, not at
//     importable members, so under the gate they are pure measurement artifacts
//     (memory: import-gated-resolution, 2026-05-28).
//
// EXCLUDES references/ and editors/zed/grammars/sysml/ (upstream/mirrored spec
// files we don't edit). Dirs are relative to the workspace root, discovered from
// CARGO_MANIFEST_DIR (this crate sits at crates/testing/sysml-spec-tests, so
// root = manifest_dir/../../..).
//
// Run with:
//   cargo test -p sysml-spec-tests measure_import_gate_migration \
//     --release -- --ignored --nocapture
mod import_gate_migration {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use sysml_core::resolution::unresolved_props;
    use sysml_core::ModelGraph;
    use sysml_parser_trait::library::{load_standard_library, LibraryConfig};

    /// How a newly-unresolved bare name would be fixed, derived from where its
    /// definition lives (mirrors `auto_import_actions`/`suggest_library_import`).
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum FixKind {
        /// Mechanically fixable via a known stdlib import (`import Pkg::*;`),
        /// where the name resolves to a member of a top-level library package.
        StdlibImport(String),
        /// Fixable via a workspace (cross-file, our-authored) import — the name
        /// is a top-level package/root defined in one of the measured files.
        WorkspaceImport(String),
        /// Genuinely unresolved: no definition found anywhere (missing def / typo
        /// / ambiguous). NOT mechanically fixable.
        Genuine,
    }

    /// Locate the workspace root from this crate's manifest dir.
    fn workspace_root() -> PathBuf {
        // crates/testing/sysml-spec-tests -> up 3 = workspace root
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest
            .ancestors()
            .nth(3)
            .map(|p| p.to_path_buf())
            .unwrap_or(manifest)
    }

    /// Our-authored model directories — the PRIMARY gate-flip-readiness scope.
    /// These are the files an auto-import migration would actually touch. EXCLUDES
    /// references/ and editors/zed/grammars/ (not listed) and libraries/standard
    /// (the stdlib itself — measured separately as a diagnostic; see `stdlib_dirs`).
    fn user_model_dirs(root: &Path) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("tests/vis-coverage", root.join("tests/vis-coverage")),
            ("tests/fixtures/shared", root.join("tests/fixtures/shared")),
            (
                "editors/diagram/examples",
                root.join("editors/diagram/examples"),
            ),
        ]
    }

    /// The standard library — measured SEPARATELY (it is the stdlib loaded as the
    /// fallback graph, not user code to migrate). Reflective metamodel-definition
    /// files are excluded (see `is_reflective_metamodel_file`).
    fn stdlib_dirs(root: &Path) -> Vec<(&'static str, PathBuf)> {
        vec![("libraries/standard", root.join("libraries/standard"))]
    }

    /// Files that reflectively model the SysML/KerML *metamodel* — every
    /// `metadata def` is a metaclass. Their bare references (`ownedFeature`,
    /// `baseType`, `owningNamespace`, …) point at KerML metaclass features reached
    /// through the metamodel hierarchy, not at importable library members, so under
    /// the gate they read as artifacts rather than real migration work. Excluded
    /// from the measurement (memory: import-gated-resolution, 2026-05-28 — "IG-1
    /// +145 is a measurement artifact, concentrated in reflective metamodel files").
    fn is_reflective_metamodel_file(path: &Path) -> bool {
        matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some("SysML.sysml") | Some("KerML.sysml")
        )
    }

    /// Accumulated measurement for one scope (USER MODELS or STDLIB).
    #[derive(Default)]
    struct ScopeStats {
        total_files: usize,
        parse_skipped: usize,
        reflective_skipped: usize,
        files_with_findings: usize,
        total_newly_unresolved: usize,
        stdlib_fixable: usize,
        workspace_fixable: usize,
        genuine: usize,
        bucket_a_stdlib_type: usize,
        bucket_b_inherited: usize,
        bucket_c_genuine: usize,
        lowercase_count: usize,
        /// distinct name -> last assigned bucket, for the per-name summary tail.
        name_bucket: BTreeMap<String, char>,
        /// file -> list of (annotated name, fix).
        report: BTreeMap<String, Vec<(String, FixKind)>>,
    }

    /// Recursively collect `.sysml` files under a directory, skipping anything
    /// that lands under an excluded path (defense-in-depth in case a measured
    /// dir ever symlinks into references/ or grammar mirrors).
    fn collect_sysml(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if !dir.exists() {
            return out;
        }
        for entry in walkdir::WalkDir::new(dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            let s = p.to_string_lossy();
            if s.contains("/references/") || s.contains("/editors/zed/grammars/") {
                continue;
            }
            if p.is_file() && p.extension().map_or(false, |e| e == "sysml") {
                out.push(p.to_path_buf());
            }
        }
        out.sort();
        out
    }

    /// Resolve the `import Pkg::*;` for a name whose definition is `def_id` in
    /// `graph`, by walking up to the nearest enclosing named package. Returns the
    /// containing package's qualified name (so the caller can emit
    /// `import <that>::*;`).
    fn enclosing_package_qname(graph: &ModelGraph, def_id: &sysml_id::ElementId) -> Option<String> {
        let mut current = graph.get_element(def_id)?.owner.clone();
        while let Some(owner_id) = current {
            if let Some(owner) = graph.get_element(&owner_id) {
                let is_pkg = owner.kind == sysml_core::ElementKind::Package
                    || owner.kind.is_subtype_of(sysml_core::ElementKind::Package);
                if is_pkg {
                    if let Some(q) = &owner.qname {
                        return Some(q.to_string());
                    }
                    if let Some(n) = &owner.name {
                        return Some(n.clone());
                    }
                }
                current = owner.owner.clone();
            } else {
                break;
            }
        }
        None
    }

    #[test]
    #[ignore = "measurement only; run with --ignored --nocapture, never gates"]
    fn measure_import_gate_migration() {
        let root = workspace_root();

        // ---- Load the standard library as the linked (fallback) graph. ----
        // Prefer SYSML_LIBRARY_PATH; else default to <root>/libraries/standard.
        let lib_path = std::env::var("SYSML_LIBRARY_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| root.join("libraries/standard"));
        let parser = TreeSitterParser::new();
        let library: Option<ModelGraph> = if lib_path.exists() {
            let mut cfg = LibraryConfig::new(&lib_path);
            // Non-strict: keep going past files that hit known TS parser gaps so
            // the library packages (ScalarValues, SI, ISQ, Base, …) still
            // register and stdlib short names resolve gate-OFF.
            cfg.strict = false;
            match load_standard_library(&parser, &cfg) {
                Ok(mut g) => {
                    // Build the library name index ONCE up front so per-file
                    // resolution doesn't rebuild it (keeps the measurement fast).
                    g.ensure_library_index();
                    eprintln!(
                        "[gate-mig] loaded stdlib from {} ({} elements)",
                        lib_path.display(),
                        g.element_count()
                    );
                    Some(g)
                }
                Err(e) => {
                    eprintln!("[gate-mig] WARNING: could not load stdlib ({e}); proceeding without it — stdlib short names will all read as genuinely-unresolved");
                    None
                }
            }
        } else {
            eprintln!(
                "[gate-mig] WARNING: {} missing; proceeding without stdlib",
                lib_path.display()
            );
            None
        };

        let library_ref = library.as_ref();

        // ---- Measure each scope as a multi-file workspace. ----
        let user = measure_scope(
            "USER MODELS",
            &user_model_dirs(&root),
            &root,
            &parser,
            library_ref,
        );
        let stdlib = measure_scope("STDLIB", &stdlib_dirs(&root), &root, &parser, library_ref);

        // ---- Report (two scopes, primary first). ----
        print_scope("USER MODELS (PRIMARY — gate-flip readiness)", &user);
        print_scope("STDLIB (diagnostic only — not user migration)", &stdlib);

        // Measurement only — never gate.
    }

    /// Resolve `name` in `scope` against `graph` (+ optional stdlib fallback),
    /// with the bare-library gate optionally on. Mirrors the driver's dual-graph
    /// resolution (`new_with_fallback`).
    fn resolves(
        graph: &ModelGraph,
        library: Option<&ModelGraph>,
        scope: &sysml_id::ElementId,
        name: &str,
        gated: bool,
    ) -> bool {
        let mut ctx = match library {
            Some(lib) => sysml_core::resolution::ResolutionContext::new_with_fallback(graph, lib),
            None => graph.resolution_context(),
        };
        ctx.set_bare_library_gate(gated);
        ctx.resolve_feature_reference(scope, name).is_some()
    }

    /// Gate-ON reachability through the inherited-feature / supertype-walk path
    /// (`resolve_redefined_feature`) — the (B) signal.
    fn resolves_inherited_gated(
        graph: &ModelGraph,
        library: Option<&ModelGraph>,
        scope: &sysml_id::ElementId,
        name: &str,
    ) -> bool {
        let mut ctx = match library {
            Some(lib) => sysml_core::resolution::ResolutionContext::new_with_fallback(graph, lib),
            None => graph.resolution_context(),
        };
        ctx.set_bare_library_gate(true);
        ctx.resolve_redefined_feature(scope, name).is_some()
    }

    /// Measure one scope: load each dir as a single merged workspace graph,
    /// elaborate ONCE with the stdlib as fallback, and bucket every bare name the
    /// gate newly breaks. Returns accumulated stats (printed by `print_scope`).
    fn measure_scope(
        scope_label: &str,
        dirs: &[(&'static str, PathBuf)],
        root: &Path,
        parser: &TreeSitterParser,
        library: Option<&ModelGraph>,
    ) -> ScopeStats {
        let mut stats = ScopeStats::default();

        for (label, dir) in dirs {
            let files = collect_sysml(dir);
            eprintln!("[gate-mig] {scope_label}/{label}: {} files", files.len());

            // Build ONE combined graph for this workspace dir (multi-file),
            // mirroring production `load_files_from_dir`: parse each file, merge into
            // a single ModelGraph, then resolve against it so cross-file references
            // resolve OWNED/INHERITED instead of falsely breaking under the gate.
            let mut combined = ModelGraph::new();
            for path in &files {
                stats.total_files += 1;
                if is_reflective_metamodel_file(path) {
                    stats.reflective_skipped += 1;
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(path) else {
                    continue;
                };
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                let sf = SysmlFile::new(&rel, &content);
                let result = parser.parse(std::slice::from_ref(&sf));
                if result.has_errors() {
                    // Parse errors are a different problem than import-gating; skip
                    // so we don't conflate them.
                    stats.parse_skipped += 1;
                    continue;
                }
                for (id, element) in result.graph.elements {
                    combined.elements.insert(id, element);
                }
                for (id, r) in result.graph.relationships {
                    combined.relationships.insert(id, r);
                }
            }
            combined.rebuild_indexes();

            // IG: elaborate ONCE on the combined graph (implicit generalization),
            // with the stdlib as the linked fallback. Per-file elaborate was
            // O(files x lib) and dominated runtime — this is the perf fix too. The
            // minted implicit-base edges are what let gate-ON inherited-feature
            // lookups reach `participant`/`source`/`target`/… via the supertype
            // chain instead of the bare library sweep (drains bucket B).
            sysml_core::elaborate::elaborate_with_library(&mut combined, library, None);
            combined.rebuild_indexes();
            let graph = &combined;

            let bare_refs = collect_bare_refs(graph);
            let mut seen: std::collections::HashSet<(sysml_id::ElementId, String)> =
                std::collections::HashSet::new();

            for (scope_id, name, prop_key, file) in &bare_refs {
                if !seen.insert((scope_id.clone(), name.clone())) {
                    continue;
                }

                // Gate OFF (historical) vs gate ON (proposed). Only names the gate
                // newly breaks matter.
                let off = resolves(graph, library, scope_id, name, false);
                let on = resolves(graph, library, scope_id, name, true);
                if !(off && !on) {
                    continue;
                }

                let inherited_on = resolves_inherited_gated(graph, library, scope_id, name);
                let capitalized = name.chars().next().is_some_and(|c| c.is_uppercase());
                let from_feature_ref = matches!(
                    prop_key.as_str(),
                    unresolved_props::REDEFINED_FEATURE
                        | unresolved_props::SUBSETTED_FEATURE
                        | unresolved_props::REFERENCED_FEATURE
                );

                let fix = classify_fix(graph, library, name);
                match &fix {
                    FixKind::StdlibImport(_) => stats.stdlib_fixable += 1,
                    FixKind::WorkspaceImport(_) => stats.workspace_fixable += 1,
                    FixKind::Genuine => stats.genuine += 1,
                }

                // Refined ADR-016 buckets (honest split of the raw count):
                //   (B) inherited-feature / metadata reference — supertype-reachable
                //       OR a lowercase feature-ish name via redef/subset/ref. NOT an
                //       import problem.
                //   (A) true missing stdlib TYPE import — capitalized, NOT
                //       inheritance-reachable, and a stdlib member exists.
                //   (C) genuine — everything else (no def anywhere sensible).
                let bucket = if inherited_on || (!capitalized && from_feature_ref) {
                    stats.bucket_b_inherited += 1;
                    'B'
                } else if capitalized && matches!(fix, FixKind::StdlibImport(_)) {
                    stats.bucket_a_stdlib_type += 1;
                    'A'
                } else {
                    stats.bucket_c_genuine += 1;
                    'C'
                };
                if !capitalized {
                    stats.lowercase_count += 1;
                }
                *stats.name_bucket.entry(name.clone()).or_insert(bucket) = bucket;
                stats.report.entry(file.clone()).or_default().push((
                    format!(
                        "{name} [{bucket}{}{}]",
                        if inherited_on { " inh" } else { "" },
                        if from_feature_ref { " featref" } else { "" }
                    ),
                    fix,
                ));
                stats.total_newly_unresolved += 1;
            }
        }

        stats.files_with_findings = stats.report.len();
        for findings in stats.report.values_mut() {
            findings.sort_by(|a, b| a.0.cmp(&b.0));
        }
        stats
    }

    /// Print one scope's accumulated stats + per-file breakdown.
    fn print_scope(title: &str, s: &ScopeStats) {
        let parsed_clean = s
            .total_files
            .saturating_sub(s.parse_skipped + s.reflective_skipped);
        eprintln!("=========================================================");
        eprintln!("[gate-mig] {title}");
        eprintln!(
            "[gate-mig] files: {} total, {parsed_clean} parsed-clean (parse-skipped {}, reflective-skipped {})",
            s.total_files, s.parse_skipped, s.reflective_skipped
        );
        eprintln!(
            "[gate-mig] files with >=1 newly-unresolved bare name: {}",
            s.files_with_findings
        );
        eprintln!(
            "[gate-mig] TOTAL newly-unresolved bare names: {}",
            s.total_newly_unresolved
        );
        eprintln!(
            "[gate-mig]   mechanically fixable via stdlib import:   {}",
            s.stdlib_fixable
        );
        eprintln!(
            "[gate-mig]   fixable via workspace (cross-file) import: {}",
            s.workspace_fixable
        );
        eprintln!(
            "[gate-mig]   genuinely unresolved (missing def/typo):   {}",
            s.genuine
        );
        eprintln!("---------------------------------------------------------");
        eprintln!("[gate-mig] REFINED ADR-016 buckets (honest split of the raw count):");
        eprintln!(
            "[gate-mig]   (A) true missing stdlib TYPE import:        {}",
            s.bucket_a_stdlib_type
        );
        eprintln!(
            "[gate-mig]   (B) inherited-feature / metadata reference: {}",
            s.bucket_b_inherited
        );
        eprintln!(
            "[gate-mig]   (C) genuine (missing def / typo):           {}",
            s.bucket_c_genuine
        );
        eprintln!(
            "[gate-mig]   (of total, lowercase/feature-ish names:     {})",
            s.lowercase_count
        );
        let distinct_a = s.name_bucket.values().filter(|&&b| b == 'A').count();
        let distinct_b = s.name_bucket.values().filter(|&&b| b == 'B').count();
        let distinct_c = s.name_bucket.values().filter(|&&b| b == 'C').count();
        eprintln!(
            "[gate-mig]   distinct NAMES per bucket: A={distinct_a} B={distinct_b} C={distinct_c}"
        );
        eprintln!("[gate-mig] distinct bucket-A names (true stdlib type imports):");
        for (n, _) in s.name_bucket.iter().filter(|(_, &b)| b == 'A') {
            eprintln!("      {n}");
        }
        eprintln!("[gate-mig] distinct bucket-C names (genuine/uncertain):");
        for (n, _) in s.name_bucket.iter().filter(|(_, &b)| b == 'C') {
            eprintln!("      {n}");
        }
        eprintln!("---------------------------------------------------------");
        eprintln!("[gate-mig] per-file breakdown:");
        for (file, findings) in &s.report {
            eprintln!("  {file}  ({} names)", findings.len());
            for (name, fix) in findings {
                match fix {
                    FixKind::StdlibImport(pkg) => {
                        eprintln!("      {name:<28} -> import {pkg}::*;   [stdlib]");
                    }
                    FixKind::WorkspaceImport(pkg) => {
                        eprintln!("      {name:<28} -> import {pkg}::*;   [workspace]");
                    }
                    FixKind::Genuine => {
                        eprintln!("      {name:<28} -> (no known import) [genuine/typo/ambiguous]");
                    }
                }
            }
        }
        eprintln!("=========================================================");
    }

    /// Collect (scope_id, bare_name, prop_key, file) tuples for every relationship
    /// reference that is a *bare* name (no `::`, no `.`). Scope = the relationship's
    /// owner, matching the driver's `scope_id = element.owner` rule. `file` is the
    /// source file of the reference-carrying element (for the per-file report; the
    /// combined multi-file graph no longer maps 1:1 to a single file).
    fn collect_bare_refs(
        graph: &ModelGraph,
    ) -> Vec<(sysml_id::ElementId, String, String, String)> {
        // The full set of unresolved-reference property keys the parser may set.
        const KEYS: &[&str] = &[
            unresolved_props::GENERAL,
            unresolved_props::TYPE,
            unresolved_props::SUBSETTED_FEATURE,
            unresolved_props::REDEFINED_FEATURE,
            unresolved_props::REFERENCED_FEATURE,
            unresolved_props::SUPERCLASSIFIER,
            unresolved_props::CONJUGATED_TYPE,
            unresolved_props::ORIGINAL_TYPE,
            unresolved_props::FEATURING_TYPE,
        ];

        let mut out = Vec::new();
        for element in graph.elements.values() {
            for key in KEYS {
                if let Some(name) = element.props.get(*key).and_then(|v| v.as_str()) {
                    // Bare names only: skip qualified (`::`) and feature chains (`.`).
                    if name.contains("::") || name.contains('.') {
                        continue;
                    }
                    let scope = element.owner.clone().unwrap_or_else(|| element.id.clone());
                    let file = element
                        .spans
                        .first()
                        .or(element.name_span.as_ref())
                        .map(|s| s.file.clone())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    out.push((scope, name.to_owned(), (*key).to_owned(), file));
                }
            }
        }
        out
    }

    /// Classify how a newly-unresolved bare name would be fixed.
    ///
    /// Mirrors `auto_import_actions` precedence: stdlib library graph first
    /// (preferred), then workspace/cross-file roots, else genuinely unresolved.
    fn classify_fix(graph: &ModelGraph, library: Option<&ModelGraph>, name: &str) -> FixKind {
        // 1. Stdlib: does a library package member carry this name? If so, the
        //    fix is `import <enclosing package>::*;`.
        if let Some(lib) = library {
            if let Some(id) = lib.resolve_in_library(name) {
                if let Some(pkg) = enclosing_package_qname(lib, id) {
                    return FixKind::StdlibImport(pkg);
                }
                // Member with no enclosing package (top-level lib pkg itself) —
                // a bare top-level package name shouldn't reach here (the gate
                // keeps those), but if it does it's effectively stdlib.
                if let Some(e) = lib.get_element(id) {
                    if let Some(n) = &e.name {
                        return FixKind::StdlibImport(n.clone());
                    }
                }
            }
        }

        // 2. Workspace: is there a same-named element defined in the current
        //    (user) graph that lives inside a named package? That's a cross-file
        //    `import <pkg>::*;` candidate.
        for element in graph.elements.values() {
            if element.name.as_deref() == Some(name) {
                if let Some(pkg) = enclosing_package_qname(graph, &element.id) {
                    return FixKind::WorkspaceImport(pkg);
                }
            }
        }

        FixKind::Genuine
    }
}
