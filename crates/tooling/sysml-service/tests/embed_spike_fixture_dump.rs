//! Embed-viewer spike fixture dump (ignored; run manually).
//!
//! Regenerates the real ViewModel JSON baked into
//! `editors/simulation-app/src/embed/fixtures/`, via the shared pruned export
//! path (`SysmlService::export_view_model` — the same function behind the CLI
//! `sysml export viewmodel`), so the text_map / interactions sidecars are
//! scoped to the ids each view references rather than whole-workspace spans.
//! Dumps every declared coffee-machine view (plus a scratch Browser-typed view
//! for the non-graph family) into `target/embed-spike-fixtures/`. Run with:
//!
//!   cargo test -p sysml-service --test embed_spike_fixture_dump -- --ignored --nocapture

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use sysml_project::discovery::OpenTarget;
use sysml_service::SysmlService;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root resolves")
}

#[test]
#[ignore = "manual fixture regeneration for the diagram-embed spike (~100s)"]
fn dump_embed_spike_fixtures() {
    let root = repo_root();
    let out_dir = root.join("target/embed-spike-fixtures");
    fs::create_dir_all(&out_dir).expect("create output dir");
    let expanded: HashSet<String> = HashSet::new();

    // Graph views: every declared view in the coffee-machine fixture.
    let cm = root.join("tests/fixtures/book-examples/coffee-machine");
    let svc = SysmlService::empty();
    svc.open_context(OpenTarget::Folder(cm.clone()))
        .expect("open coffee-machine fixture");
    let views = svc.views_list("__workspace__").expect("views_list");
    assert!(!views.is_empty(), "fixture declares views");
    for s in &views {
        let name = s.name.clone().unwrap_or_else(|| s.id.to_string());
        let value = svc
            .export_view_model(&s.id, &expanded, false)
            .expect("viewmodel renders");
        let path = out_dir.join(format!("coffee-machine-{name}.json"));
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).expect("write fixture");
        eprintln!("wrote {}", path.display());
    }

    // Non-graph (Browser/tree) view: scratch copy of the same model plus one
    // Browser-typed view, opened as its own workspace.
    let scratch = root.join("target/embed-spike-model");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).expect("create scratch model dir");
    for entry in fs::read_dir(&cm).expect("read fixture dir") {
        let entry = entry.expect("dir entry");
        if entry.path().extension().map(|e| e == "sysml").unwrap_or(false) {
            fs::copy(entry.path(), scratch.join(entry.file_name())).expect("copy model file");
        }
    }
    fs::write(
        scratch.join("embed-browser-view.sysml"),
        "package EmbedSpikeViews {\n    view def ComponentTreeView :> BrowserView {\n        expose Definitions::CoffeeMachine;\n    }\n}\n",
    )
    .expect("write browser view file");

    let svc2 = SysmlService::empty();
    svc2.open_context(OpenTarget::Folder(scratch.clone()))
        .expect("open scratch model");
    let views2 = svc2.views_list("__workspace__").expect("views_list scratch");
    let tree = views2
        .iter()
        .find(|s| s.name.as_deref() == Some("ComponentTreeView"))
        .expect("ComponentTreeView view present");
    let value = svc2
        .export_view_model(&tree.id, &expanded, false)
        .expect("browser viewmodel renders");
    let path = out_dir.join("coffee-machine-componentTree.json");
    fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).expect("write fixture");
    eprintln!("wrote {}", path.display());
}
