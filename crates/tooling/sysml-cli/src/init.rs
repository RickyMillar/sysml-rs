//! `sysml init` — Create a new SysML project.

use std::fs;
use std::path::{Path, PathBuf};

use sysml_manifest::{SysmlManifest, MANIFEST_FILENAME};

use crate::common::CliError;

/// Run the `init` command.
pub fn run(name: Option<&str>, target_dir: Option<&Path>) -> Result<(), CliError> {
    let (project_dir, project_name) = determine_project_dir_and_name(name, target_dir)?;

    let manifest_path = project_dir.join(MANIFEST_FILENAME);
    if manifest_path.exists() {
        return Err(CliError::user(format!(
            "{} already exists in {}",
            MANIFEST_FILENAME,
            project_dir.display()
        )));
    }

    // Create project directory if it doesn't exist
    fs::create_dir_all(&project_dir).map_err(|e| {
        CliError::internal(format!(
            "failed to create directory {}: {e}",
            project_dir.display()
        ))
    })?;

    // Create sysml.toml
    let manifest = SysmlManifest::new(&project_name, "0.1.0");
    sysml_manifest::save_manifest(&manifest_path, &manifest)
        .map_err(|e| CliError::internal(format!("failed to write {MANIFEST_FILENAME}: {e}")))?;

    // Create src/ directory with an example file
    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir)
        .map_err(|e| CliError::internal(format!("failed to create src/ directory: {e}")))?;

    let example_file = src_dir.join("main.sysml");
    if !example_file.exists() {
        let example_content = format!(
            "package {name} {{\n    // Define your SysML model here\n}}\n",
            name = to_pascal_case(&project_name)
        );
        fs::write(&example_file, example_content)
            .map_err(|e| CliError::internal(format!("failed to write example file: {e}")))?;
    }

    println!(
        "Created SysML project '{}' in {}",
        project_name,
        project_dir.display()
    );
    println!("  {}", manifest_path.display());
    println!("  {}", example_file.display());

    Ok(())
}

fn determine_project_dir_and_name(
    name: Option<&str>,
    target_dir: Option<&Path>,
) -> Result<(PathBuf, String), CliError> {
    match (name, target_dir) {
        // `sysml init --name foo` — create new directory ./foo/
        (Some(name), None) => {
            let dir = PathBuf::from(name);
            Ok((dir, name.to_owned()))
        }
        // `sysml init` in an existing directory
        (None, None) => {
            let current_dir = std::env::current_dir()
                .map_err(|e| CliError::internal(format!("failed to get current directory: {e}")))?;
            let dir_name = current_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("my-project")
                .to_owned();
            Ok((current_dir, dir_name))
        }
        // `sysml init --name foo /some/path` or similar
        (Some(name), Some(dir)) => Ok((dir.to_path_buf(), name.to_owned())),
        (None, Some(dir)) => {
            let dir_name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("my-project")
                .to_owned();
            Ok((dir.to_path_buf(), dir_name))
        }
    }
}

/// Convert "my-project" to "MyProject" for package names.
fn to_pascal_case(s: &str) -> String {
    s.split(|c: char| c == '-' || c == '_' || c == ' ')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let mut result = first.to_uppercase().to_string();
                    result.extend(chars);
                    result
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case_conversion() {
        assert_eq!(to_pascal_case("my-project"), "MyProject");
        assert_eq!(to_pascal_case("coffee_machine"), "CoffeeMachine");
        assert_eq!(to_pascal_case("simple"), "Simple");
        assert_eq!(to_pascal_case("multi-word-name"), "MultiWordName");
    }
}
