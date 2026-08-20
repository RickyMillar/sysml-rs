//! Write `.kpar` archives.

use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;

use super::archive_error::KparError;
use super::schema::{ProjectInfo, ProjectMetadata, UsageEntry};
use super::KparArchive;
use sysml_manifest::SysmlManifest;

/// Write a `.kpar` archive to disk.
pub fn write_kpar(path: &Path, archive: &KparArchive) -> Result<(), KparError> {
    let file = std::fs::File::create(path).map_err(|e| KparError::io(path, e))?;
    let writer = std::io::BufWriter::new(file);
    write_kpar_to_writer(writer, archive)
}

/// Write a `.kpar` archive to any writer.
fn write_kpar_to_writer<W: Write + std::io::Seek>(
    writer: W,
    archive: &KparArchive,
) -> Result<(), KparError> {
    let mut zip = zip::ZipWriter::new(writer);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let root = &archive.root_dir;

    // Create root directory entry
    zip.add_directory(format!("{root}/"), options)?;

    // Write .project.json
    let project_json = serde_json::to_string_pretty(&archive.project_info)
        .map_err(|e| KparError::Other(format!("failed to serialize .project.json: {e}")))?;
    zip.start_file(format!("{root}/.project.json"), options)?;
    zip.write_all(project_json.as_bytes())
        .map_err(|e| KparError::Io {
            path: ".project.json".into(),
            source: e,
        })?;

    // Write .meta.json
    let meta_json = serde_json::to_string_pretty(&archive.metadata)
        .map_err(|e| KparError::Other(format!("failed to serialize .meta.json: {e}")))?;
    zip.start_file(format!("{root}/.meta.json"), options)?;
    zip.write_all(meta_json.as_bytes())
        .map_err(|e| KparError::Io {
            path: ".meta.json".into(),
            source: e,
        })?;

    // Write source files
    for (name, contents) in &archive.source_files {
        zip.start_file(format!("{root}/{name}"), options)?;
        zip.write_all(contents).map_err(|e| KparError::Io {
            path: name.into(),
            source: e,
        })?;
    }

    zip.finish().map_err(KparError::Zip)?;
    Ok(())
}

/// Builder for creating a `KparArchive` from a manifest and source directory.
pub struct KparBuilder {
    manifest: SysmlManifest,
    source_dir: std::path::PathBuf,
}

impl KparBuilder {
    /// Create a new builder from a manifest and source directory.
    pub fn new(manifest: SysmlManifest, source_dir: impl Into<std::path::PathBuf>) -> Self {
        KparBuilder {
            manifest,
            source_dir: source_dir.into(),
        }
    }

    /// Build the `KparArchive`.
    pub fn build(self) -> Result<KparArchive, KparError> {
        let project_info = self.build_project_info();
        let source_files = self.collect_source_files()?;
        let metadata = self.build_metadata(&source_files);

        Ok(KparArchive {
            root_dir: self.manifest.project.name,
            project_info,
            metadata,
            source_files,
        })
    }

    /// Convert manifest to spec-compliant `.project.json` content.
    fn build_project_info(&self) -> ProjectInfo {
        let mut info =
            ProjectInfo::new(&self.manifest.project.name, &self.manifest.project.version);
        info.description = self.manifest.project.description.clone();
        info.license = self.manifest.project.license.clone();

        // Add selected stdlib usage entries.
        let stdlib = self.manifest.effective_stdlib();
        for lib_name in stdlib.enabled_libraries() {
            if let (Some(url), Some(version_constraint)) = (
                sysml_manifest::StdlibConfig::library_kpar_url(lib_name),
                sysml_manifest::StdlibConfig::library_version_constraint(lib_name),
            ) {
                info.usage.push(UsageEntry {
                    resource: url.to_owned(),
                    version_constraint: Some(version_constraint.to_owned()),
                });
            }
        }

        // Add explicit dependency usage entries
        for (name, dep) in &self.manifest.dependencies {
            let resource = dependency_to_resource(name, dep);
            info.usage.push(UsageEntry {
                resource,
                version_constraint: None,
            });
        }

        info
    }

    /// Collect `.sysml` and `.kerml` files from the source directory.
    fn collect_source_files(&self) -> Result<Vec<(String, Vec<u8>)>, KparError> {
        let mut files = Vec::new();

        if !self.source_dir.exists() {
            return Ok(files);
        }

        collect_source_files_recursive(&self.source_dir, &self.source_dir, &mut files)?;
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(files)
    }

    /// Build `.meta.json` from collected source files.
    fn build_metadata(&self, source_files: &[(String, Vec<u8>)]) -> ProjectMetadata {
        let mut metadata = ProjectMetadata::new();
        metadata.set_created_now();

        let edition = &self.manifest.project.sysml_edition;
        metadata.metamodel = Some(format!("https://www.omg.org/spec/SysML/{edition}0201"));

        // Build the package-name → file index.
        // Extract top-level package names from source file contents.
        for (filename, contents) in source_files {
            if let Ok(text) = std::str::from_utf8(contents) {
                if let Some(pkg_name) = extract_package_name(text) {
                    metadata.add_index_entry(pkg_name, filename);
                }
            }
        }

        metadata
    }
}

/// Recursively collect .sysml/.kerml files from a directory.
fn collect_source_files_recursive(
    base_dir: &Path,
    current_dir: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), KparError> {
    let entries = std::fs::read_dir(current_dir).map_err(|e| KparError::io(current_dir, e))?;

    for entry in entries {
        let entry = entry.map_err(|e| KparError::io(current_dir, e))?;
        let path = entry.path();

        if path.is_dir() {
            // Skip hidden directories
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with('.') {
                collect_source_files_recursive(base_dir, &path, files)?;
            }
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "sysml" || ext == "kerml" {
                let relative = path
                    .strip_prefix(base_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                let contents = std::fs::read(&path).map_err(|e| KparError::io(&path, e))?;
                files.push((relative, contents));
            }
        }
    }

    Ok(())
}

/// Extract the first top-level `package` name from SysML source text.
///
/// Simple heuristic: looks for `package <Name>` at the start of a line.
fn extract_package_name(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        // Match patterns like: `package Foo {`, `package Foo;`, `standard library package Foo {`
        if let Some(rest) = trimmed.strip_prefix("package ") {
            return extract_identifier(rest);
        }
        if let Some(rest) = trimmed.strip_prefix("standard library package ") {
            return extract_identifier(rest);
        }
        if let Some(rest) = trimmed.strip_prefix("library package ") {
            return extract_identifier(rest);
        }
    }
    None
}

/// Extract an identifier from the start of a string (up to whitespace or `{` or `;`).
fn extract_identifier(s: &str) -> Option<String> {
    let name: String = s
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Convert a manifest dependency to a resource URL for `.project.json`.
fn dependency_to_resource(name: &str, dep: &sysml_manifest::Dependency) -> String {
    match dep {
        sysml_manifest::Dependency::Registry(_version) => {
            // Registry deps use auto-generated URN
            format!("urn:sysml:{name}")
        }
        sysml_manifest::Dependency::Detailed(d) => {
            if let Some(kpar_url) = &d.kpar {
                kpar_url.clone()
            } else if let Some(git_url) = &d.git {
                // Use the git URL as the resource identifier
                git_url.clone()
            } else if d.path.is_some() {
                // Path deps use auto-generated URN
                format!("urn:sysml:{name}")
            } else {
                format!("urn:sysml:{name}")
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn extract_package_name_standard() {
        assert_eq!(
            extract_package_name("package Foo {\n}\n"),
            Some("Foo".to_string())
        );
    }

    #[test]
    fn extract_package_name_library() {
        assert_eq!(
            extract_package_name("standard library package ScalarValues {\n}\n"),
            Some("ScalarValues".to_string())
        );
    }

    #[test]
    fn extract_package_name_with_leading_whitespace() {
        // Only matches at start of trimmed line
        assert_eq!(
            extract_package_name("  package Indented {\n}\n"),
            Some("Indented".to_string())
        );
    }

    #[test]
    fn extract_package_name_none_when_missing() {
        assert_eq!(
            extract_package_name("// just a comment\npart def Wheel;\n"),
            None
        );
    }

    #[test]
    fn round_trip_kpar_archive() {
        let mut info = ProjectInfo::new("Test Project", "0.1.0");
        info.description = Some("A test".to_string());
        info.add_usage("urn:sysml:dep-a", Some("1.0.0".to_string()));

        let mut meta = ProjectMetadata::new();
        meta.add_index_entry("Main", "Main.sysml");
        meta.created = Some("2025-01-01T00:00:00Z".to_string());

        let archive = KparArchive {
            root_dir: "Test Project".to_string(),
            project_info: info,
            metadata: meta,
            source_files: vec![
                ("Main.sysml".to_string(), b"package Main {\n}\n".to_vec()),
                (
                    "Sub.sysml".to_string(),
                    b"package Sub {\n  part def X;\n}\n".to_vec(),
                ),
            ],
        };

        // Write to a temp file
        let tmp = std::env::temp_dir().join(format!(
            "sysml-kpar-test-{}-{}.kpar",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write_kpar(&tmp, &archive).unwrap();

        // Read it back
        let loaded = crate::kpar::read_kpar(&tmp).unwrap();
        assert_eq!(loaded.root_dir, "Test Project");
        assert_eq!(loaded.project_info.name, "Test Project");
        assert_eq!(loaded.project_info.version, "0.1.0");
        assert_eq!(loaded.project_info.usage.len(), 1);
        assert_eq!(loaded.metadata.index["Main"], "Main.sysml");
        assert_eq!(loaded.source_files.len(), 2);

        // Verify file contents
        let main_file = loaded
            .source_files
            .iter()
            .find(|(n, _)| n == "Main.sysml")
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&main_file.1).unwrap(),
            "package Main {\n}\n"
        );

        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn kpar_builder_from_manifest() {
        let tmp = std::env::temp_dir().join(format!(
            "sysml-kpar-builder-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("Vehicle.sysml"),
            "package Vehicle {\n  part def Wheel;\n}\n",
        )
        .unwrap();

        let mut manifest = SysmlManifest::new("my-vehicle", "0.1.0");
        manifest.project.description = Some("Vehicle model".to_string());
        manifest.stdlib = Some(sysml_manifest::StdlibConfig {
            include_only: vec!["analysis".to_string()],
            exclude: Vec::new(),
        });
        manifest.add_dependency("base-types", sysml_manifest::Dependency::path("../base"));

        let builder = KparBuilder::new(manifest, &tmp);
        let archive = builder.build().unwrap();

        assert_eq!(archive.root_dir, "my-vehicle");
        assert_eq!(archive.project_info.name, "my-vehicle");

        // Should have: 1 selected stdlib (analysis) + 1 explicit dep = 2 usage entries
        assert_eq!(archive.project_info.usage.len(), 2);

        // Check stdlib usage entries are present
        let resources: Vec<&str> = archive
            .project_info
            .usage
            .iter()
            .map(|u| u.resource.as_str())
            .collect();
        assert!(resources
            .iter()
            .any(|r| r.contains("Analysis-Domain-Library")));
        assert!(resources.iter().any(|r| r.contains("urn:sysml:base-types")));

        // Check metadata
        assert!(archive.metadata.index.contains_key("Vehicle"));
        assert_eq!(archive.metadata.index["Vehicle"], "Vehicle.sysml");

        // Check source files
        assert_eq!(archive.source_files.len(), 1);
        assert_eq!(archive.source_files[0].0, "Vehicle.sysml");

        let _ = std::fs::remove_dir_all(tmp);
    }
}
