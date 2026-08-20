use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::Path;

use crate::checksum::{compute_checksum, ChecksumAlgorithm};
use crate::info::ProjectInfo;
use crate::meta::ProjectMeta;

/// Reader for `.kpar` (KerML Package Archive) ZIP files.
///
/// A `.kpar` file is a ZIP archive containing a project directory with:
/// - `.project.json` — the project manifest
/// - `.meta.json` — project metadata with symbol index and checksums
/// - Source files (`.sysml` / `.kerml`) referenced by the metadata
///
/// The manifest files may be at the archive root or inside a subdirectory.
pub struct KparReader<R: Read + Seek> {
    archive: zip::ZipArchive<R>,
    /// Cached project manifest.
    info: ProjectInfo,
    /// Cached project metadata.
    meta: ProjectMeta,
    /// Prefix path within the archive (e.g. "Systems Library/").
    prefix: String,
    /// Source file paths found in the archive (relative to prefix).
    source_paths: Vec<String>,
}

impl KparReader<std::io::BufReader<std::fs::File>> {
    /// Open a `.kpar` file from a filesystem path.
    pub fn open(path: impl AsRef<Path>) -> crate::Result<Self> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        Self::from_reader(reader)
    }
}

/// Find a file ending with `suffix` in the archive, returning the full entry name.
fn find_entry<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, suffix: &str) -> Option<String> {
    for i in 0..archive.len() {
        if let Ok(file) = archive.by_index_raw(i) {
            let name = file.name().to_owned();
            if name.ends_with(suffix) && !name.contains("__MACOSX") {
                return Some(name);
            }
        }
    }
    None
}

impl<R: Read + Seek> KparReader<R> {
    /// Create a reader from any `Read + Seek` source.
    pub fn from_reader(reader: R) -> crate::Result<Self> {
        let mut archive = zip::ZipArchive::new(reader)?;

        // Find .project.json (may be at root or in a subdirectory)
        let project_entry = find_entry(&mut archive, ".project.json").ok_or_else(|| {
            crate::Error::MissingField {
                field: ".project.json".to_owned(),
                context: "kpar archive".to_owned(),
            }
        })?;

        // Derive the prefix (everything before ".project.json")
        let prefix = project_entry
            .strip_suffix(".project.json")
            .unwrap_or("")
            .to_owned();

        // Read .project.json
        let info: ProjectInfo = {
            let mut file = archive.by_name(&project_entry)?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            serde_json::from_str(&contents)?
        };

        // Read .meta.json
        let meta_entry = format!("{prefix}.meta.json");
        let meta: ProjectMeta = {
            let mut file = archive.by_name(&meta_entry)?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            serde_json::from_str(&contents)?
        };

        // Collect source file paths relative to prefix
        let source_paths: Vec<String> = (0..archive.len())
            .filter_map(|i| {
                let file = archive.by_index(i).ok()?;
                let name = file.name().to_owned();
                if file.is_dir() || name.contains("__MACOSX") {
                    return None;
                }
                // Must be under our prefix and not a manifest
                let relative = name.strip_prefix(&prefix)?;
                if relative.starts_with('.') {
                    return None;
                }
                Some(relative.to_owned())
            })
            .collect();

        Ok(Self {
            archive,
            info,
            meta,
            prefix,
            source_paths,
        })
    }

    /// The project manifest.
    pub fn info(&self) -> &ProjectInfo {
        &self.info
    }

    /// The project metadata.
    pub fn meta(&self) -> &ProjectMeta {
        &self.meta
    }

    /// Read a source file by its path relative to the project root within the archive.
    pub fn source(&mut self, relative_path: &str) -> crate::Result<String> {
        let full_path = format!("{}{}", self.prefix, relative_path);
        let mut file = self.archive.by_name(&full_path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        Ok(contents)
    }

    /// Paths of all source files in the archive (relative to the project root).
    pub fn source_paths(&self) -> &[String] {
        &self.source_paths
    }

    /// Read all source files and return them as a map of relative path -> content.
    pub fn sources(&mut self) -> crate::Result<HashMap<String, String>> {
        let paths = self.source_paths.clone();
        let mut result = HashMap::new();
        for path in paths {
            let content = self.source(&path)?;
            result.insert(path, content);
        }
        Ok(result)
    }

    /// Verify checksums of all files that have them in `.meta.json`.
    pub fn verify_checksums(&mut self) -> crate::Result<Vec<crate::Error>> {
        let mut errors = Vec::new();
        let checksums = self.meta.checksum.clone();

        for (file_path, expected) in &checksums {
            let algorithm = ChecksumAlgorithm::from_name(&expected.algorithm)?;
            let content = self.source(file_path)?;
            let actual = compute_checksum(content.as_bytes(), algorithm);
            if actual != expected.value {
                errors.push(crate::Error::ChecksumMismatch {
                    path: file_path.clone(),
                    expected: expected.value.clone(),
                    actual,
                });
            }
        }

        Ok(errors)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// Path to the official `.kpar` files.
    fn kpar_dir() -> std::path::PathBuf {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest.join("../../../references/sysmlv2/SysML/20250201")
    }

    /// Path to the test `.kpar` file.
    fn test_kpar() -> std::path::PathBuf {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest.join("../../../references/sysmlv2/sysand/sysand/tests/data/test_lib.kpar")
    }

    #[test]
    fn open_systems_library_kpar() {
        let path = kpar_dir().join("Systems-Library.kpar");
        if !path.exists() {
            eprintln!("skipping: {path:?} not found");
            return;
        }
        let reader = KparReader::open(&path).unwrap();
        assert_eq!(reader.info().name, "SysML Systems Library");
        assert!(!reader.source_paths().is_empty());
    }

    #[test]
    fn read_all_official_kpars() {
        let dir = kpar_dir();
        if !dir.exists() {
            eprintln!("skipping: {dir:?} not found");
            return;
        }

        let expected_names = [
            ("Analysis-Domain-Library.kpar", "SysML Analysis Library"),
            (
                "Cause-and-Effect-Domain-Library.kpar",
                "SysML Cause and Effect Library",
            ),
            ("Geometry-Domain-Library.kpar", "SysML Geometry Library"),
            ("Metadata-Domain-Library.kpar", "SysML Metadata Library"),
            (
                "Quantities-and-Units-Domain-Library.kpar",
                "SysML Quantities and Units Library",
            ),
            (
                "Requirement-Derivation-Domain-Library.kpar",
                "SysML Requirement Derivation Library",
            ),
            ("Systems-Library.kpar", "SysML Systems Library"),
        ];

        for (filename, expected_name) in &expected_names {
            let path = dir.join(filename);
            let reader = KparReader::open(&path)
                .unwrap_or_else(|e| panic!("failed to open {filename}: {e}"));
            assert_eq!(
                reader.info().name,
                *expected_name,
                "name mismatch for {filename}"
            );
            assert!(
                !reader.source_paths().is_empty(),
                "no sources in {filename}"
            );
        }
    }

    #[test]
    fn verify_kpar_checksums() {
        let path = kpar_dir().join("Systems-Library.kpar");
        if !path.exists() {
            eprintln!("skipping: {path:?} not found");
            return;
        }
        let mut reader = KparReader::open(&path).unwrap();
        let errors = reader.verify_checksums().unwrap();
        assert!(errors.is_empty(), "checksum errors: {errors:?}");
    }

    #[test]
    fn read_test_kpar() {
        let path = test_kpar();
        if !path.exists() {
            eprintln!("skipping: {path:?} not found");
            return;
        }
        let reader = KparReader::open(&path).unwrap();
        assert!(!reader.info().name.is_empty());
    }

    #[test]
    fn read_source_from_kpar() {
        let path = kpar_dir().join("Systems-Library.kpar");
        if !path.exists() {
            eprintln!("skipping: {path:?} not found");
            return;
        }
        let mut reader = KparReader::open(&path).unwrap();
        let first_path = reader.source_paths()[0].clone();
        let content = reader.source(&first_path).unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn kpar_source_paths_are_relative() {
        let path = kpar_dir().join("Systems-Library.kpar");
        if !path.exists() {
            eprintln!("skipping: {path:?} not found");
            return;
        }
        let reader = KparReader::open(&path).unwrap();
        for source_path in reader.source_paths() {
            assert!(
                !source_path.contains('/') || !source_path.starts_with("Systems Library"),
                "source path should be relative: {source_path}"
            );
        }
    }
}
