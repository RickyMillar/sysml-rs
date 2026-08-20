//! Read `.kpar` archives.

use std::io::Read;
use std::path::Path;

use super::archive_error::KparError;
use super::schema::{ProjectInfo, ProjectMetadata};
use super::KparArchive;

/// Read and parse a `.kpar` archive from disk.
///
/// The archive is expected to be a ZIP file containing a root directory
/// with `.project.json`, `.meta.json`, and source files.
pub fn read_kpar(path: &Path) -> Result<KparArchive, KparError> {
    let file = std::fs::File::open(path).map_err(|e| KparError::io(path, e))?;
    let reader = std::io::BufReader::new(file);
    read_kpar_from_reader(reader, path)
}

/// Read a `.kpar` archive from any reader.
pub fn read_kpar_from_reader<R: Read + std::io::Seek>(
    reader: R,
    archive_path: &Path,
) -> Result<KparArchive, KparError> {
    let mut archive = zip::ZipArchive::new(reader)?;

    // Discover the root directory name by looking at the first entry.
    let root_dir = discover_root_dir(&mut archive, archive_path)?;

    // Read .project.json
    let project_json_path = format!("{root_dir}/.project.json");
    let project_info = read_json_entry::<ProjectInfo>(&mut archive, &project_json_path)?;

    // Read .meta.json
    let meta_json_path = format!("{root_dir}/.meta.json");
    let metadata = read_json_entry::<ProjectMetadata>(&mut archive, &meta_json_path)?;

    // Read source files
    let mut source_files = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let entry_name = entry.name().to_owned();

        // Skip directories and hidden files (.project.json, .meta.json, .DS_Store)
        if entry.is_dir() {
            continue;
        }

        // Must be under the root directory
        let Some(relative) = entry_name.strip_prefix(&format!("{root_dir}/")) else {
            continue;
        };

        // Skip hidden files and macOS metadata
        if relative.starts_with('.') || relative.starts_with("__MACOSX") {
            continue;
        }

        // Only include .sysml and .kerml files
        if !relative.ends_with(".sysml") && !relative.ends_with(".kerml") {
            continue;
        }

        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(|e| KparError::io(entry_name.as_str(), e))?;
        source_files.push((relative.to_owned(), contents));
    }

    Ok(KparArchive {
        root_dir,
        project_info,
        metadata,
        source_files,
    })
}

/// Discover the root directory name from the archive entries.
fn discover_root_dir<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    archive_path: &Path,
) -> Result<String, KparError> {
    // Find .project.json to determine the root directory
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        let name = entry.name().to_owned();
        if name.ends_with("/.project.json") {
            // Invariant: ends_with guard above guarantees strip_suffix succeeds
            #[allow(clippy::unwrap_used)]
            let root = name.strip_suffix("/.project.json").unwrap().to_owned();
            return Ok(root);
        }
    }

    Err(KparError::missing_file(archive_path, ".project.json"))
}

/// Read and parse a JSON file from inside the ZIP archive.
fn read_json_entry<T: serde::de::DeserializeOwned>(
    archive: &mut zip::ZipArchive<impl Read + std::io::Seek>,
    entry_path: &str,
) -> Result<T, KparError> {
    let mut entry = archive
        .by_name(entry_path)
        .map_err(|_zip_err| KparError::MissingFile {
            archive: std::path::PathBuf::from("<archive>"),
            file: entry_path.to_owned(),
        })?;

    let mut contents = String::new();
    entry
        .read_to_string(&mut contents)
        .map_err(|e| KparError::Io {
            path: entry_path.into(),
            source: e,
        })?;

    serde_json::from_str(&contents).map_err(|e| KparError::JsonParse {
        file: entry_path.to_owned(),
        message: e.to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn reference_kpar_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../references/sysmlv2/SysML/20250201")
            .join(name)
    }

    #[test]
    fn read_systems_library_kpar() {
        let path = reference_kpar_path("Systems-Library.kpar");
        if !path.exists() {
            eprintln!(
                "Skipping test: reference KPAR not found at {}",
                path.display()
            );
            return;
        }

        let archive = read_kpar(&path).unwrap();
        assert_eq!(archive.root_dir, "Systems Library");
        assert_eq!(archive.project_info.name, "SysML Systems Library");
        assert_eq!(archive.project_info.version, "2.0.0");
        assert!(archive.project_info.usage.len() >= 3);

        // Check metadata
        assert!(!archive.metadata.index.is_empty());
        assert!(archive.metadata.index.contains_key("Actions"));
        assert!(archive.metadata.index.contains_key("Parts"));
        assert!(archive.metadata.index.contains_key("States"));

        // Check source files
        assert!(!archive.source_files.is_empty());
        let file_names: Vec<&str> = archive.source_file_names();
        assert!(file_names.contains(&"Parts.sysml"));
        assert!(file_names.contains(&"Actions.sysml"));
    }

    #[test]
    fn read_analysis_domain_library_kpar() {
        let path = reference_kpar_path("Analysis-Domain-Library.kpar");
        if !path.exists() {
            eprintln!(
                "Skipping test: reference KPAR not found at {}",
                path.display()
            );
            return;
        }

        let archive = read_kpar(&path).unwrap();
        assert_eq!(archive.root_dir, "Analysis");
        assert_eq!(archive.project_info.name, "SysML Analysis Library");
        assert_eq!(archive.project_info.version, "2.0.0");

        // Analysis depends on Systems Library, Semantic Library, Data Type Library,
        // Function Library, and Quantities-and-Units Library
        assert!(archive.project_info.usage.len() >= 5);

        // Check that usage entries have proper resource URLs
        let resources: Vec<&str> = archive
            .project_info
            .usage
            .iter()
            .map(|u| u.resource.as_str())
            .collect();
        assert!(resources.iter().any(|r| r.contains("Systems-Library")));
    }

    #[test]
    fn read_all_reference_kpars() {
        let kpar_names = [
            "Systems-Library.kpar",
            "Analysis-Domain-Library.kpar",
            "Cause-and-Effect-Domain-Library.kpar",
            "Geometry-Domain-Library.kpar",
            "Metadata-Domain-Library.kpar",
            "Quantities-and-Units-Domain-Library.kpar",
            "Requirement-Derivation-Domain-Library.kpar",
        ];

        for name in &kpar_names {
            let path = reference_kpar_path(name);
            if !path.exists() {
                eprintln!("Skipping {name}: not found");
                continue;
            }

            let archive = read_kpar(&path).unwrap_or_else(|e| {
                panic!("Failed to read {name}: {e}");
            });

            // All should have valid project info
            assert!(!archive.project_info.name.is_empty(), "{name}: empty name");
            assert!(
                !archive.project_info.version.is_empty(),
                "{name}: empty version"
            );

            // All should have source files
            assert!(!archive.source_files.is_empty(), "{name}: no source files");

            // All should have metadata with index
            assert!(!archive.metadata.index.is_empty(), "{name}: empty index");
        }
    }
}
