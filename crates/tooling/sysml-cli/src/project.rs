use std::collections::HashSet;

use clap::Subcommand;
use sysml_project::{
    discover_project, DiscoveryResult, Project, ProjectHandle, ProjectInfo, ProjectMeta, StdlibRegistry,
};

use crate::common::CliError;

/// Subcommands for `sysml project`.
#[derive(Subcommand)]
pub enum ProjectCommand {
    /// Initialize a new SysML project in the current directory
    Init {
        /// Project name (defaults to the current directory name)
        #[arg(long)]
        name: Option<String>,
        /// Project version (defaults to "0.1.0")
        #[arg(long, default_value = "0.1.0")]
        version: String,
    },
    /// Show project information
    Info,
    /// List standard library projects
    Stdlib {
        /// List all symbols exported by each library
        #[arg(long)]
        symbols: bool,
    },
}

/// Entry point for `sysml project <subcommand>`.
pub fn run(command: ProjectCommand) -> Result<(), CliError> {
    match command {
        ProjectCommand::Init { name, version } => run_init(name, version),
        ProjectCommand::Info => run_info(),
        ProjectCommand::Stdlib { symbols } => run_stdlib(symbols),
    }
}

/// `sysml project init [--name NAME] [--version VERSION]`
#[allow(clippy::needless_pass_by_value)]
fn run_init(name: Option<String>, version: String) -> Result<(), CliError> {
    let cwd = std::env::current_dir()
        .map_err(|e| CliError::internal(format!("cannot determine current directory: {e}")))?;

    let project_name = name.unwrap_or_else(|| {
        cwd.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".to_owned())
    });

    let project_path = cwd.join(".project.json");
    if project_path.exists() {
        return Err(CliError::user(
            "a .project.json already exists in this directory",
        ));
    }

    let info = ProjectInfo {
        name: project_name.clone(),
        description: None,
        version: version.clone(),
        topic: Vec::new(),
        usage: Vec::new(),
    };

    let project_json = serde_json::to_string_pretty(&info)
        .map_err(|e| CliError::internal(format!("failed to serialize .project.json: {e}")))?;
    std::fs::write(&project_path, &project_json)
        .map_err(|e| CliError::internal(format!("failed to write .project.json: {e}")))?;

    let meta = ProjectMeta {
        index: Default::default(),
        created: Some(now_iso8601()),
        metamodel: Some("https://www.omg.org/spec/SysML/20250201".to_owned()),
        checksum: Default::default(),
    };

    let meta_json = serde_json::to_string_pretty(&meta)
        .map_err(|e| CliError::internal(format!("failed to serialize .meta.json: {e}")))?;
    let meta_path = cwd.join(".meta.json");
    std::fs::write(&meta_path, &meta_json)
        .map_err(|e| CliError::internal(format!("failed to write .meta.json: {e}")))?;

    println!("initialized project '{}' v{}", project_name, version);
    println!("  created .project.json");
    println!("  created .meta.json");

    Ok(())
}

/// `sysml project info`
fn run_info() -> Result<(), CliError> {
    let cwd = std::env::current_dir()
        .map_err(|e| CliError::internal(format!("cannot determine current directory: {e}")))?;

    let project_dir = match discover_project(&cwd) {
        DiscoveryResult::Project(p) => p,
        DiscoveryResult::Workspace(p) => p,
        DiscoveryResult::NotFound => {
            return Err(CliError::user(
                "no SysML project found (missing .project.json)",
            ));
        }
    };

    let project = Project::from_directory(ProjectHandle(0), &project_dir).map_err(|e| {
        CliError::user(format!(
            "failed to load project at '{}': {e}",
            project_dir.display()
        ))
    })?;

    println!("Project: {}", project.info.name);
    println!("Version: {}", project.info.version);

    if let Some(desc) = &project.info.description {
        println!("Description: {desc}");
    }

    if !project.info.usage.is_empty() {
        println!("Dependencies:");
        for dep in &project.info.usage {
            println!("  {} ({})", dep.resource, dep.version_constraint);
        }
    }

    if let Some(meta) = &project.meta {
        let symbol_count = meta.index.len();
        let file_count = meta.index.values().collect::<HashSet<_>>().len();
        println!(
            "Symbol index: {} symbol{} across {} file{}",
            symbol_count,
            if symbol_count == 1 { "" } else { "s" },
            file_count,
            if file_count == 1 { "" } else { "s" },
        );
    }

    Ok(())
}

/// `sysml project stdlib [--symbols]`
fn run_stdlib(show_symbols: bool) -> Result<(), CliError> {
    let registry = StdlibRegistry::new()
        .map_err(|e| CliError::internal(format!("failed to load stdlib registry: {e}")))?;

    println!("Standard Library Projects ({}):", registry.len());
    println!();

    for proj in registry.iter() {
        let symbol_count = proj.meta.index.len();
        println!(
            "  {} v{} ({} symbol{})",
            proj.info.name,
            proj.info.version,
            symbol_count,
            if symbol_count == 1 { "" } else { "s" },
        );

        if show_symbols && !proj.meta.index.is_empty() {
            let mut symbols: Vec<_> = proj.meta.index.keys().collect();
            symbols.sort();
            for sym in symbols {
                let file = &proj.meta.index[sym];
                println!("    {sym} -> {file}");
            }
        }
    }

    Ok(())
}

/// Produce an ISO 8601 UTC timestamp for the current time.
fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Reuse the same civil-date algorithm from stdlib_cache.
    let days = (secs / 86_400) as i64;
    let day_secs = (secs % 86_400) as u32;
    let hour = day_secs / 3_600;
    let minute = (day_secs % 3_600) / 60;
    let second = day_secs % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_project_and_meta() {
        let dir = TempDir::new().unwrap();
        // Run init within the temp directory.
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let result = run(ProjectCommand::Init {
            name: Some("test-project".to_string()),
            version: "1.2.3".to_string(),
        });

        // Restore working directory before asserting (so cleanup works).
        std::env::set_current_dir(&original_dir).unwrap();

        result.unwrap();

        // Verify .project.json
        let project_path = dir.path().join(".project.json");
        assert!(project_path.exists(), ".project.json should exist");
        let project_json = std::fs::read_to_string(&project_path).unwrap();
        let info: ProjectInfo = serde_json::from_str(&project_json).unwrap();
        assert_eq!(info.name, "test-project");
        assert_eq!(info.version, "1.2.3");

        // Verify .meta.json
        let meta_path = dir.path().join(".meta.json");
        assert!(meta_path.exists(), ".meta.json should exist");
        let meta_json = std::fs::read_to_string(&meta_path).unwrap();
        let meta: ProjectMeta = serde_json::from_str(&meta_json).unwrap();
        assert!(meta.index.is_empty());
        assert!(meta.created.is_some());
    }

    #[test]
    fn init_rejects_existing_project() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(".project.json"),
            r#"{"name":"Existing","version":"1.0.0"}"#,
        )
        .unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let result = run(ProjectCommand::Init {
            name: None,
            version: "0.1.0".to_string(),
        });

        std::env::set_current_dir(&original_dir).unwrap();

        assert!(result.is_err());
    }

    #[test]
    fn info_reads_fixture_project() {
        let dir = TempDir::new().unwrap();

        // Create a fixture project
        let info = ProjectInfo {
            name: "FixtureProject".to_string(),
            description: Some("A test fixture".to_string()),
            version: "2.0.0".to_string(),
            topic: vec!["Test".to_string()],
            usage: vec![],
        };
        let info_json = serde_json::to_string_pretty(&info).unwrap();
        std::fs::write(dir.path().join(".project.json"), &info_json).unwrap();

        let mut meta_index = std::collections::HashMap::new();
        meta_index.insert("Parts".to_string(), "Parts.sysml".to_string());
        meta_index.insert("Actions".to_string(), "Actions.sysml".to_string());
        let meta = ProjectMeta {
            index: meta_index,
            created: Some("2026-03-01T00:00:00Z".to_string()),
            metamodel: None,
            checksum: Default::default(),
        };
        let meta_json = serde_json::to_string_pretty(&meta).unwrap();
        std::fs::write(dir.path().join(".meta.json"), &meta_json).unwrap();

        // Load project directly (avoid set_current_dir which races with parallel tests)
        let project = Project::from_directory(ProjectHandle(0), dir.path()).unwrap();
        assert_eq!(project.info.name, "FixtureProject");
        assert_eq!(project.info.version, "2.0.0");
        assert_eq!(project.info.description.as_deref(), Some("A test fixture"));
        assert!(project.meta.is_some());
        let meta = project.meta.as_ref().unwrap();
        assert_eq!(meta.index.len(), 2);
    }

    #[test]
    fn stdlib_lists_all_10_libraries() {
        let registry = StdlibRegistry::new().unwrap();
        assert_eq!(registry.len(), 10);

        // Verify run_stdlib succeeds
        let result = run(ProjectCommand::Stdlib { symbols: false });
        result.unwrap();
    }

    #[test]
    fn stdlib_with_symbols_flag() {
        let result = run(ProjectCommand::Stdlib { symbols: true });
        result.unwrap();
    }
}
