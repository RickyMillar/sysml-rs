use std::path::Path;

use sysml_project::*;

fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn stdlib_assets_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../references/sysmlv2/sysand/core/src/stdlib_assets/20250201")
}

fn pilot_workspace_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/.workspace.json")
}

// ── Round-trip all 10 stdlib .project.json files ──

static STDLIB_NAMES: &[&str] = &[
    "analysis-library",
    "cause-and-effect-library",
    "data-type-library",
    "function-library",
    "geometry-library",
    "metadata-library",
    "quantities-and-units-library",
    "requirement-derivation-library",
    "semantic-library",
    "systems-library",
];

#[test]
fn round_trip_all_stdlib_project_json() {
    let dir = stdlib_assets_dir();
    for name in STDLIB_NAMES {
        let path = dir.join(format!("{name}.project.json"));
        let original = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {name}.project.json: {e}"));
        let info = ProjectInfo::from_str(&original)
            .unwrap_or_else(|e| panic!("failed to parse {name}.project.json: {e}"));

        // Verify round-trip
        let serialized = serde_json::to_string(&info).unwrap();
        let reparsed: ProjectInfo = serde_json::from_str(&serialized).unwrap();
        assert_eq!(info, reparsed, "round-trip failed for {name}.project.json");

        // Verify key fields
        assert!(!info.name.is_empty(), "empty name in {name}");
        assert!(!info.version.is_empty(), "empty version in {name}");
    }
}

#[test]
fn round_trip_all_stdlib_meta_json() {
    let dir = stdlib_assets_dir();
    for name in STDLIB_NAMES {
        let path = dir.join(format!("{name}.meta.json"));
        let original = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {name}.meta.json: {e}"));
        let meta = ProjectMeta::from_str(&original)
            .unwrap_or_else(|e| panic!("failed to parse {name}.meta.json: {e}"));

        // Verify round-trip
        let serialized = serde_json::to_string(&meta).unwrap();
        let reparsed: ProjectMeta = serde_json::from_str(&serialized).unwrap();
        assert_eq!(meta, reparsed, "round-trip failed for {name}.meta.json");

        // Verify key fields
        assert!(!meta.index.is_empty(), "empty index in {name}");
    }
}

// ── Pilot Implementation .workspace.json ──

#[test]
fn parse_pilot_implementation_workspace() {
    let path = pilot_workspace_path();
    if !path.exists() {
        eprintln!("skipping: pilot workspace not found at {path:?}");
        return;
    }
    let ws = WorkspaceInfo::from_path(&path).unwrap();
    assert_eq!(ws.projects.len(), 10);

    // Check that all expected libraries are present
    let paths: Vec<&str> = ws.projects.iter().map(|p| p.path.as_str()).collect();
    assert!(paths.contains(&"Systems Library"));
    assert!(paths.contains(&"Kernel Libraries/Kernel Semantic Library"));
    assert!(paths.contains(&"Kernel Libraries/Kernel Data Type Library"));
    assert!(paths.contains(&"Kernel Libraries/Kernel Function Library"));
}

// ── Fixture tests ──

#[test]
fn load_simple_project_fixture() {
    let dir = fixtures_dir().join("simple-project");
    let project = Project::from_directory(ProjectHandle(0), &dir).unwrap();
    assert_eq!(project.info.name, "Simple Test Project");
    assert_eq!(project.info.version, "0.1.0");
    assert!(project.meta.is_some());

    let meta = project.meta.as_ref().unwrap();
    assert_eq!(
        meta.index.get("Vehicle"),
        Some(&"Vehicle.sysml".to_string())
    );

    // Read source
    let source = project.read_source("Vehicle.sysml").unwrap();
    assert!(source.contains("part def Vehicle"));
}

#[test]
fn load_workspace_fixture() {
    let dir = fixtures_dir().join("workspace");
    let ws = WorkspaceInfo::from_path(dir.join(".workspace.json")).unwrap();
    assert_eq!(ws.projects.len(), 2);

    // Load each sub-project
    for (i, wp) in ws.projects.iter().enumerate() {
        let project_dir = dir.join(&wp.path);
        let project = Project::from_directory(ProjectHandle(i as u32), &project_dir).unwrap();
        assert!(!project.info.name.is_empty());
    }
}

#[test]
fn discover_fixture_project() {
    let dir = fixtures_dir().join("simple-project");
    match discover_project(&dir) {
        DiscoveryResult::Project(p) => assert_eq!(p, dir),
        other => panic!("expected Project, got {other:?}"),
    }
}

#[test]
fn discover_fixture_workspace_from_subproject() {
    // project-a has its own .project.json, so discover_project finds it first
    let dir = fixtures_dir().join("workspace/project-a");
    match discover_project(&dir) {
        DiscoveryResult::Project(p) => assert_eq!(p, dir),
        other => panic!("expected Project, got {other:?}"),
    }

    // discover_workspace walks past .project.json and finds .workspace.json
    match discover_workspace(&dir) {
        DiscoveryResult::Workspace(p) => assert_eq!(p, fixtures_dir().join("workspace")),
        other => panic!("expected Workspace, got {other:?}"),
    }
}

#[test]
fn discover_empty_fixture() {
    let dir = fixtures_dir().join("empty");
    match discover_project(&dir) {
        // empty fixture might discover parent fixtures, so just check it doesn't panic
        DiscoveryResult::NotFound => {}
        DiscoveryResult::Project(_) | DiscoveryResult::Workspace(_) => {
            // acceptable — the empty dir is nested inside the fixtures dir
        }
    }
}

// ── StdlibRegistry integration ──

#[test]
fn stdlib_registry_full_integration() {
    let registry = StdlibRegistry::new().unwrap();

    // All 10 projects present
    assert_eq!(registry.len(), 10);

    // IRI lookup for both forms
    let sys_urn = registry.get_by_iri("urn:kpar:systems-library").unwrap();
    assert_eq!(sys_urn.info.name, "SysML Systems Library");

    let sys_https = registry
        .get_by_iri("https://www.omg.org/spec/SysML/20250201/SysML-Systems-Library")
        .unwrap();
    assert_eq!(sys_https.info.name, "SysML Systems Library");

    // Name lookup
    let kern = registry.get_by_name("Kernel Semantic Library").unwrap();
    assert_eq!(kern.urn_name, "semantic-library");

    // Kernel projects
    let kernels = registry.kernel_projects();
    assert_eq!(kernels.len(), 3);

    // Symbol index
    let index = registry.symbol_index();
    assert!(index.len() > 50);

    // Check expected symbols from different libraries
    assert!(!index.lookup("Parts").is_empty()); // systems-library
    assert!(!index.lookup("ScalarValues").is_empty()); // data-type-library
    assert!(!index.lookup("Base").is_empty()); // semantic-library
    assert!(!index.lookup("BaseFunctions").is_empty()); // function-library
    assert!(!index.lookup("AnalysisCases").is_empty()); // analysis-library
    assert!(!index.lookup("ISQSpaceTime").is_empty()); // quantities-and-units-library

    // No conflicts in stdlib
    let conflicts = index.conflicts();
    assert!(
        conflicts.is_empty(),
        "unexpected stdlib conflicts: {conflicts:?}"
    );
}

// ── .kpar integration (all 7 official kpars) ──

#[cfg(feature = "kpar")]
#[test]
fn all_official_kpars_verify_checksums() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../references/sysmlv2/SysML/20250201");
    if !dir.exists() {
        eprintln!("skipping: kpar directory not found");
        return;
    }

    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "kpar") {
            continue;
        }
        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        let mut reader =
            KparReader::open(&path).unwrap_or_else(|e| panic!("failed to open {filename}: {e}"));

        // Verify manifest is valid
        assert!(!reader.info().name.is_empty(), "empty name in {filename}");
        assert!(
            !reader.source_paths().is_empty(),
            "no sources in {filename}"
        );

        // Verify checksums if present
        if !reader.meta().checksum.is_empty() {
            let errors = reader
                .verify_checksums()
                .unwrap_or_else(|e| panic!("checksum verification failed for {filename}: {e}"));
            assert!(
                errors.is_empty(),
                "checksum errors in {filename}: {errors:?}"
            );
        }
    }
}

// ── Lock file integration ──

#[cfg(feature = "lock")]
#[test]
fn lock_file_round_trip_with_all_source_types() {
    use sysml_project::{KparLockFile, LockedProject, ProjectSource};

    let lock = KparLockFile {
        version: 1,
        projects: vec![
            LockedProject {
                name: "Kernel Semantic Library".to_string(),
                version: "1.0.0".to_string(),
                source: ProjectSource::Stdlib {
                    iri: "urn:kpar:semantic-library".to_string(),
                },
                dependencies: vec![],
                checksum: None,
            },
            LockedProject {
                name: "External Lib".to_string(),
                version: "2.0.0".to_string(),
                source: ProjectSource::Kpar {
                    path: "libs/external.kpar".to_string(),
                },
                dependencies: vec!["Kernel Semantic Library".to_string()],
                checksum: Some("abcdef1234567890".to_string()),
            },
            LockedProject {
                name: "My Project".to_string(),
                version: "0.1.0".to_string(),
                source: ProjectSource::Path {
                    path: ".".to_string(),
                },
                dependencies: vec![
                    "Kernel Semantic Library".to_string(),
                    "External Lib".to_string(),
                ],
                checksum: None,
            },
        ],
    };

    let toml_str = lock.to_string_pretty().unwrap();
    let parsed = KparLockFile::from_str(&toml_str).unwrap();
    assert_eq!(lock, parsed);
    assert_eq!(parsed.projects.len(), 3);
}
