use sysml_parser_incremental::TreeSitterParser;

fn assert_fixture_parses(label: &str, source: &str) -> tree_sitter::Tree {
    let tree = TreeSitterParser::new()
        .parse_tree(source)
        .unwrap_or_else(|| panic!("{label}: tree-sitter returned no tree"));
    assert!(
        !tree.root_node().has_error(),
        "{label}: fixture must parse without ERROR/MISSING nodes:\n{}",
        tree.root_node().to_sexp()
    );
    tree
}

#[test]
fn keyword_derivation_form_parses() {
    let tree = assert_fixture_parses(
        "keyword derivation",
        include_str!("fixtures/g24-keyword-derivation.sysml"),
    );
    let cst = tree.root_node().to_sexp();
    assert!(cst.contains("(annotated_connection_usage"), "{cst}");
    assert_eq!(cst.matches("(connection_end_usage").count(), 2, "{cst}");
    assert_eq!(
        cst.matches("(prefix_metadata_annotation").count(),
        3,
        "{cst}"
    );
}

#[test]
fn untyped_part_usage_parses_in_requirement_body() {
    let tree = assert_fixture_parses(
        "untyped part in requirement body",
        include_str!("fixtures/requirement-body-untyped-part.sysml"),
    );
    let cst = tree.root_node().to_sexp();
    assert!(cst.contains("(requirement_body"), "{cst}");
    assert!(cst.contains("(standard_usage"), "{cst}");
}

#[test]
fn frame_is_a_contextual_requirement_body_keyword() {
    let tree = assert_fixture_parses(
        "part named frame in requirement body",
        include_str!("fixtures/requirement-body-part-named-frame.sysml"),
    );
    let cst = tree.root_node().to_sexp();
    assert!(cst.contains("name: (identifier)"), "{cst}");
    assert!(!cst.contains("(frame_constraint"), "{cst}");
}
