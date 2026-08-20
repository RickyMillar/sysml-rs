//! End-to-end integration tests for the project support pipeline.
//!
//! Tests the full flow: fixture discovery → project loading → salsa database →
//! symbol index → file loading → resolution.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sysml_ide_db::AnalysisHost;
use sysml_project::{
    discover_project, DiscoveryResult, Project, ProjectHandle, ProjectInfo, ProjectMeta, WorkspaceInfo,
};

/// Path to the test fixtures directory (relative to workspace root).
fn fixtures_dir() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("fixtures")
}

// ---------------------------------------------------------------------------
// Single project fixture tests
// ---------------------------------------------------------------------------

#[test]
fn load_vehicle_project_into_salsa() {
    let fixture = fixtures_dir().join("vehicle-project");
    assert!(fixture.join(".project.json").exists(), "fixture missing");

    let project = Project::from_directory(ProjectHandle(10), &fixture).unwrap();
    assert_eq!(project.info.name, "Vehicle Project");
    assert!(project.meta.is_some());

    let mut host = AnalysisHost::new();
    let pid = host.load_project(project);
    assert_eq!(pid, ProjectHandle(10));
    assert_eq!(host.project_count(), 1);

    // WorkspaceConfig should now exist
    let _config = host
        .workspace_config()
        .expect("workspace_config should be set");

    // Symbol index should have entries from .meta.json
    let analysis = host.analysis();
    let idx = analysis.symbol_index().expect("symbol_index should exist");
    assert!(idx.len() >= 3, "expected >= 3 symbols, got {}", idx.len());

    // Verify specific symbols
    let vehicle = analysis.resolve_symbol("Vehicle");
    assert!(vehicle.is_some(), "Vehicle symbol not found");
    let (vpid, vfile) = vehicle.unwrap();
    assert_eq!(vpid, ProjectHandle(10));
    assert_eq!(vfile, "Vehicle.sysml");

    let engine = analysis.resolve_symbol("Engine");
    assert!(engine.is_some(), "Engine symbol not found");
    assert_eq!(engine.unwrap().1, "Engine.sysml");

    let wheel = analysis.resolve_symbol("Wheel");
    assert!(wheel.is_some(), "Wheel symbol not found");
    assert_eq!(wheel.unwrap().1, "Wheel.sysml");

    // Non-existent symbol
    assert!(analysis.resolve_symbol("Nonexistent").is_none());
}

#[test]
fn ensure_file_loaded_reads_from_project() {
    let fixture = fixtures_dir().join("vehicle-project");
    let project = Project::from_directory(ProjectHandle(10), &fixture).unwrap();

    let mut host = AnalysisHost::new();
    host.load_project(project);

    // Load Engine.sysml from the project
    let file_id = host.ensure_file_loaded(ProjectHandle(10), "Engine.sysml");
    assert!(file_id.is_some(), "failed to load Engine.sysml");

    let fid = file_id.unwrap();
    let sf = host.source_file(fid).expect("source file should exist");
    let analysis = host.analysis();
    let text = analysis.file_text(sf);
    assert!(
        text.contains("part def Engine"),
        "Engine.sysml content wrong"
    );
    assert!(
        text.contains("horsepower"),
        "Engine.sysml missing horsepower attr"
    );

    // Loading the same file again should return the same ID (idempotent)
    let fid2 = host
        .ensure_file_loaded(ProjectHandle(10), "Engine.sysml")
        .unwrap();
    assert_eq!(fid, fid2);
}

#[test]
fn ensure_file_loaded_returns_none_for_missing_file() {
    let fixture = fixtures_dir().join("vehicle-project");
    let project = Project::from_directory(ProjectHandle(10), &fixture).unwrap();

    let mut host = AnalysisHost::new();
    host.load_project(project);

    assert!(host
        .ensure_file_loaded(ProjectHandle(10), "DoesNotExist.sysml")
        .is_none());
}

#[test]
fn ensure_file_loaded_returns_none_for_missing_project() {
    let fixture = fixtures_dir().join("vehicle-project");
    let project = Project::from_directory(ProjectHandle(10), &fixture).unwrap();

    let mut host = AnalysisHost::new();
    host.load_project(project);

    // Project 99 doesn't exist
    assert!(host
        .ensure_file_loaded(ProjectHandle(99), "Engine.sysml")
        .is_none());
}

#[test]
fn load_all_project_files_and_parse() {
    let fixture = fixtures_dir().join("vehicle-project");
    let project = Project::from_directory(ProjectHandle(10), &fixture).unwrap();

    let mut host = AnalysisHost::new();
    host.load_project(project);

    // Load all three files
    let engine_id = host
        .ensure_file_loaded(ProjectHandle(10), "Engine.sysml")
        .unwrap();
    let wheel_id = host
        .ensure_file_loaded(ProjectHandle(10), "Wheel.sysml")
        .unwrap();
    let vehicle_id = host
        .ensure_file_loaded(ProjectHandle(10), "Vehicle.sysml")
        .unwrap();

    let analysis = host.analysis();

    // Parse each file — should produce valid ModelGraphs
    let engine_sf = host.source_file(engine_id).unwrap();
    let engine_parsed = analysis.parse_file(engine_sf);
    assert!(
        !engine_parsed.graph().elements.is_empty(),
        "Engine.sysml should parse to non-empty graph"
    );

    let wheel_sf = host.source_file(wheel_id).unwrap();
    let wheel_parsed = analysis.parse_file(wheel_sf);
    assert!(
        !wheel_parsed.graph().elements.is_empty(),
        "Wheel.sysml should parse to non-empty graph"
    );

    let vehicle_sf = host.source_file(vehicle_id).unwrap();
    let vehicle_parsed = analysis.parse_file(vehicle_sf);
    assert!(
        !vehicle_parsed.graph().elements.is_empty(),
        "Vehicle.sysml should parse to non-empty graph"
    );

    // Verify total file count in database
    assert_eq!(host.file_count(), 3);
}

// ---------------------------------------------------------------------------
// Workspace fixture tests
// ---------------------------------------------------------------------------

#[test]
fn load_workspace_projects_into_salsa() {
    let fixture = fixtures_dir().join("multi-workspace");
    assert!(
        fixture.join(".workspace.json").exists(),
        "workspace fixture missing"
    );

    // Load workspace info
    let ws_info = WorkspaceInfo::from_path(fixture.join(".workspace.json")).unwrap();
    assert_eq!(ws_info.projects.len(), 2);

    // Load each project
    let mut host = AnalysisHost::new();

    let lib_dir = fixture.join("lib-project");
    let lib = Project::from_directory(ProjectHandle(10), &lib_dir).unwrap();
    assert_eq!(lib.info.name, "Shared Library");
    host.load_project(lib);

    let app_dir = fixture.join("app-project");
    let app = Project::from_directory(ProjectHandle(11), &app_dir).unwrap();
    assert_eq!(app.info.name, "Control App");
    host.load_project(app);

    assert_eq!(host.project_count(), 2);
    assert_eq!(host.salsa_project_count(), 2);

    // Symbol index should have entries from both projects
    let analysis = host.analysis();
    let idx = analysis.symbol_index().unwrap();

    // lib-project symbols
    let sensors = analysis.resolve_symbol("Sensors");
    assert!(sensors.is_some(), "Sensors symbol not found");
    assert_eq!(sensors.unwrap().0, ProjectHandle(10));

    let actuators = analysis.resolve_symbol("Actuators");
    assert!(actuators.is_some(), "Actuators symbol not found");
    assert_eq!(actuators.unwrap().0, ProjectHandle(10));

    // app-project symbols
    let controller = analysis.resolve_symbol("Controller");
    assert!(controller.is_some(), "Controller symbol not found");
    assert_eq!(controller.unwrap().0, ProjectHandle(11));

    // Total: 2 (lib) + 1 (app) = 3 symbols
    assert_eq!(idx.len(), 3);
}

#[test]
fn workspace_cross_project_file_loading() {
    let fixture = fixtures_dir().join("multi-workspace");

    let mut host = AnalysisHost::new();

    let lib = Project::from_directory(ProjectHandle(10), fixture.join("lib-project")).unwrap();
    host.load_project(lib);

    let app = Project::from_directory(ProjectHandle(11), fixture.join("app-project")).unwrap();
    host.load_project(app);

    // Load files from both projects
    let sensors_id = host
        .ensure_file_loaded(ProjectHandle(10), "Sensors.sysml")
        .unwrap();
    let _actuators_id = host
        .ensure_file_loaded(ProjectHandle(10), "Actuators.sysml")
        .unwrap();
    let controller_id = host
        .ensure_file_loaded(ProjectHandle(11), "Controller.sysml")
        .unwrap();

    let analysis = host.analysis();

    // Verify contents
    let sensors_sf = host.source_file(sensors_id).unwrap();
    assert!(analysis.file_text(sensors_sf).contains("TemperatureSensor"));

    let controller_sf = host.source_file(controller_id).unwrap();
    let controller_text = analysis.file_text(controller_sf);
    assert!(controller_text.contains("import Sensors::*"));
    assert!(controller_text.contains("import Actuators::*"));

    assert_eq!(host.file_count(), 3);
}

// ---------------------------------------------------------------------------
// Stdlib integration tests
// ---------------------------------------------------------------------------

#[test]
fn enable_stdlib_populates_symbol_index() {
    let mut host = AnalysisHost::new();
    host.enable_stdlib().expect("enable_stdlib should succeed");

    // All 10 stdlib projects should be loaded
    assert_eq!(host.salsa_project_count(), 10);

    let analysis = host.analysis();
    let idx = analysis.symbol_index().expect("should have symbol index");

    // Stdlib should have many symbols
    assert!(
        idx.len() > 50,
        "expected >50 stdlib symbols, got {}",
        idx.len()
    );

    // Key stdlib symbols should be resolvable
    assert!(
        analysis.resolve_symbol("Parts").is_some(),
        "Parts not found in stdlib"
    );
    assert!(
        analysis.resolve_symbol("Actions").is_some(),
        "Actions not found in stdlib"
    );
    assert!(
        analysis.resolve_symbol("States").is_some(),
        "States not found in stdlib"
    );
    assert!(
        analysis.resolve_symbol("Base").is_some(),
        "Base not found in stdlib"
    );
    assert!(
        analysis.resolve_symbol("ScalarValues").is_some(),
        "ScalarValues not found"
    );
}

#[test]
fn project_plus_stdlib_combined_index() {
    let fixture = fixtures_dir().join("vehicle-project");
    let project = Project::from_directory(ProjectHandle(10), &fixture).unwrap();

    let mut host = AnalysisHost::new();
    host.enable_stdlib().unwrap();
    host.load_project(project);

    // Should have 10 stdlib + 1 user project
    assert_eq!(host.salsa_project_count(), 11);

    let analysis = host.analysis();

    // User project symbols
    assert!(analysis.resolve_symbol("Vehicle").is_some());
    assert!(analysis.resolve_symbol("Engine").is_some());

    // Stdlib symbols
    assert!(analysis.resolve_symbol("Parts").is_some());
    assert!(analysis.resolve_symbol("Base").is_some());
}

// ---------------------------------------------------------------------------
// Project discovery integration
// ---------------------------------------------------------------------------

#[test]
fn discover_vehicle_project_fixture() {
    let fixture = fixtures_dir().join("vehicle-project");
    match discover_project(&fixture) {
        DiscoveryResult::Project(p) => assert_eq!(p, fixture),
        other => panic!("expected Project, got {other:?}"),
    }
}

#[test]
fn discover_workspace_fixture() {
    let fixture = fixtures_dir().join("multi-workspace");
    match discover_project(&fixture) {
        DiscoveryResult::Workspace(p) => assert_eq!(p, fixture),
        other => panic!("expected Workspace, got {other:?}"),
    }
}

#[test]
fn discover_project_from_subproject_dir() {
    // Starting from a subproject dir that has its own .project.json,
    // discover_project finds the project (not the workspace) because
    // it checks .project.json at the current level before walking up.
    let fixture = fixtures_dir().join("multi-workspace").join("app-project");
    match discover_project(&fixture) {
        DiscoveryResult::Project(p) => {
            assert_eq!(p, fixture);
        }
        other => panic!("expected Project, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Backward compatibility
// ---------------------------------------------------------------------------

#[test]
fn host_without_projects_works_normally() {
    let mut host = AnalysisHost::new();

    // No projects loaded — workspace_config should be None
    assert!(host.workspace_config().is_none());
    assert_eq!(host.project_count(), 0);
    assert_eq!(host.salsa_project_count(), 0);

    // Normal file operations still work
    let id = host.set_file_content("file:///test.sysml", "package Test {}".to_string());
    let sf = host.source_file(id).unwrap();

    let analysis = host.analysis();
    assert_eq!(analysis.file_text(sf), "package Test {}");

    // Parse and resolve should work without projects
    let parsed = analysis.parse_file(sf);
    assert!(!parsed.graph().elements.is_empty());

    let resolved = analysis.resolve_file_best(sf, None);
    let _graph = resolved.graph();

    // Symbol index should be None
    assert!(analysis.symbol_index().is_none());
    assert!(analysis.resolve_symbol("Test").is_none());
}

#[test]
fn host_with_projects_still_supports_direct_files() {
    let fixture = fixtures_dir().join("vehicle-project");
    let project = Project::from_directory(ProjectHandle(10), &fixture).unwrap();

    let mut host = AnalysisHost::new();
    host.load_project(project);

    // Direct file operations still work alongside projects
    let id = host.set_file_content(
        "file:///standalone.sysml",
        "package Standalone { part def Widget; }".to_string(),
    );
    let sf = host.source_file(id).unwrap();

    let analysis = host.analysis();
    assert!(analysis.file_text(sf).contains("Widget"));

    let parsed = analysis.parse_file(sf);
    assert!(!parsed.graph().elements.is_empty());
}

// ---------------------------------------------------------------------------
// Incremental recomputation
// ---------------------------------------------------------------------------

#[test]
fn symbol_index_recomputes_when_project_meta_changes() {
    use std::collections::HashMap;

    let mut host = AnalysisHost::new();

    // Create a project with initial meta
    let info = ProjectInfo {
        name: "Dynamic".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        topic: Vec::new(),
        usage: Vec::new(),
    };
    let meta = ProjectMeta {
        index: [("Foo".to_string(), "Foo.sysml".to_string())]
            .into_iter()
            .collect(),
        created: None,
        metamodel: None,
        checksum: HashMap::new(),
    };
    let project = Project {
        id: ProjectHandle(10),
        info,
        meta: Some(meta),
        root: sysml_project::ProjectRoot::InMemory,
    };
    host.load_project(project);

    let analysis1 = host.analysis();
    let idx1 = analysis1.symbol_index().unwrap();
    assert_eq!(idx1.len(), 1);
    assert!(analysis1.resolve_symbol("Foo").is_some());
    assert!(analysis1.resolve_symbol("Bar").is_none());
    drop(analysis1);

    // Update the salsa project's meta to add a new symbol
    // (In real usage, this happens when .meta.json changes on disk)
    let config = host.workspace_config().unwrap();
    let db = host.db_mut();
    let projects = config.projects(db);
    let salsa_proj = projects[0];
    let new_meta = Arc::new(ProjectMeta {
        index: [
            ("Foo".to_string(), "Foo.sysml".to_string()),
            ("Bar".to_string(), "Bar.sysml".to_string()),
        ]
        .into_iter()
        .collect(),
        created: None,
        metamodel: None,
        checksum: HashMap::new(),
    });
    use salsa::Setter;
    salsa_proj.set_meta(db).to(new_meta);

    // Symbol index should now have 2 entries
    let analysis2 = host.analysis();
    let idx2 = analysis2.symbol_index().unwrap();
    assert_eq!(idx2.len(), 2);
    assert!(analysis2.resolve_symbol("Foo").is_some());
    assert!(analysis2.resolve_symbol("Bar").is_some());
}

// ---------------------------------------------------------------------------
// Error resilience
// ---------------------------------------------------------------------------

#[test]
fn project_with_no_meta_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".project.json"),
        r#"{"name":"NoMeta","version":"1.0.0"}"#,
    )
    .unwrap();
    // No .meta.json file

    let project = Project::from_directory(ProjectHandle(10), dir.path()).unwrap();
    assert!(project.meta.is_none());

    let mut host = AnalysisHost::new();
    host.load_project(project);

    // Symbol index should exist but be empty
    let analysis = host.analysis();
    let idx = analysis.symbol_index().unwrap();
    assert_eq!(idx.len(), 0);
}

#[test]
fn project_with_empty_meta_index() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".project.json"),
        r#"{"name":"EmptyMeta","version":"1.0.0"}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join(".meta.json"), r#"{"index":{}}"#).unwrap();

    let project = Project::from_directory(ProjectHandle(10), dir.path()).unwrap();

    let mut host = AnalysisHost::new();
    host.load_project(project);

    let analysis = host.analysis();
    let idx = analysis.symbol_index().unwrap();
    assert_eq!(idx.len(), 0);
}
