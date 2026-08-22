//! Usage-form view typing must resolve the view kind: `view x : BrowserView`
//! carries its kind through FeatureTyping exactly as `view def X :> BrowserView`
//! carries it through Subclassification.

mod smodel_common;
use smodel_common::parse_sysml;

use sysml_core::build_view_index;
use sysml_diagram::smodel::ViewType;
use sysml_diagram::ViewRequest;

fn kind_of(source: &str, view_name: &str) -> ViewType {
    let mut graph = parse_sysml(source);
    sysml_core::elaborate::elaborate(&mut graph);
    let summary = build_view_index(&graph)
        .into_iter()
        .find(|s| s.name.as_deref() == Some(view_name))
        .expect("view discovered");
    ViewRequest::from_view_usage(&graph, &summary).view_type
}

#[test]
fn usage_form_bare_alias_dangles_to_general() {
    // Post-ruling: the stdlib defines no bare `Browser`, so usage-form typing
    // with the alias spelling must NOT classify (General + warning). The frame
    // still shows the verbatim declared type; the KIND does not follow it.
    assert_eq!(
        kind_of("package P { view x : Browser; }", "x"),
        ViewType::General
    );
}

#[test]
fn usage_form_subsetting_resolves_kind() {
    // Usage-form `:>` (an owned Subsetting, not Subclassification) must feed
    // the kind walk exactly like the def-form spelling — the J4
    // specializes_name usage-form pattern applied to view kind resolution.
    assert_eq!(
        kind_of("package P { view y :> BrowserView; }", "y"),
        ViewType::Browser
    );
}

#[test]
fn usage_form_typing_resolves_kind_qualified() {
    assert_eq!(
        kind_of("package P { view x : StandardViewDefinitions::BrowserView; }", "x"),
        ViewType::Browser
    );
}

#[test]
fn usage_form_typing_resolves_kind_bare_canonical() {
    assert_eq!(
        kind_of("package P { view x : BrowserView; }", "x"),
        ViewType::Browser
    );
}
