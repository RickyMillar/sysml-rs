//! Corpus file discovery and parsing.
//!
//! This module handles discovering .sysml files in the reference corpus
//! and parsing them with the tree-sitter parser.

use std::collections::HashSet;
use std::path::PathBuf;

use rayon::prelude::*;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use walkdir::WalkDir;

use crate::{CoverageConfig, CoverageSummary, FileParseResult};

/// A corpus file discovered in the reference materials.
#[derive(Debug, Clone)]
pub struct CorpusFile {
    /// Full path to the file.
    pub full_path: PathBuf,
    /// Relative path from corpus root (for display/comparison).
    pub relative_path: String,
    /// File content.
    pub content: String,
}

/// Discover all .sysml files in the corpus directories.
pub fn discover_corpus_files(config: &CoverageConfig) -> Vec<CorpusFile> {
    let mut files = Vec::new();

    for subdir in &config.corpus_subdirs {
        let dir_path = config.corpus_path.join(subdir);
        if !dir_path.exists() {
            eprintln!("Warning: Corpus directory not found: {:?}", dir_path);
            continue;
        }

        for entry in WalkDir::new(&dir_path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "sysml") {
                if let Ok(content) = std::fs::read_to_string(path) {
                    let relative = path
                        .strip_prefix(&config.corpus_path)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| path.to_string_lossy().to_string());

                    files.push(CorpusFile {
                        full_path: path.to_path_buf(),
                        relative_path: relative,
                        content,
                    });
                }
            }
        }
    }

    // Sort by relative path for deterministic ordering
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    files
}

/// Parse a single corpus file.
pub fn parse_corpus_file(file: &CorpusFile) -> FileParseResult {
    let parser = TreeSitterParser::new();
    let sysml_file = SysmlFile::new(&file.relative_path, &file.content);
    let result = parser.parse(&[sysml_file]);

    FileParseResult {
        path: file.relative_path.clone(),
        success: result.is_ok(),
        errors: result
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.to_string())
            .collect(),
        element_count: result.graph.element_count(),
    }
}

/// Parse all corpus files and collect results.
///
/// Parsing is parallelized with rayon for faster corpus coverage runs.
pub fn parse_all_corpus_files(
    config: &CoverageConfig,
    allow_list: &HashSet<String>,
) -> (Vec<FileParseResult>, CoverageSummary) {
    let files = discover_corpus_files(config);
    let mut summary = CoverageSummary::default();

    summary.total_files = files.len();

    // Parse files in parallel using rayon
    let results: Vec<FileParseResult> = files.par_iter().map(|f| parse_corpus_file(f)).collect();

    for result in &results {
        if result.success {
            summary.passed_files += 1;
        } else {
            summary.failed_files += 1;

            // Check if this failure was expected
            let in_allow_list = allow_list.contains(&result.path)
                || allow_list.iter().any(|pattern| {
                    // Simple glob matching for patterns like "**/SysML.sysml"
                    if pattern.starts_with("**/") {
                        result.path.ends_with(&pattern[3..])
                    } else {
                        result.path.contains(pattern)
                    }
                });

            if in_allow_list {
                summary.expected_failures += 1;
            } else {
                summary.unexpected_failures += 1;
            }
        }
    }

    (results, summary)
}

/// Parse corpus files and collect element kinds produced.
pub fn collect_element_kinds(config: &CoverageConfig) -> HashSet<String> {
    let files = discover_corpus_files(config);
    let parser = TreeSitterParser::new();
    let mut kinds = HashSet::new();

    for file in &files {
        let sysml_file = SysmlFile::new(&file.relative_path, &file.content);
        let result = parser.parse(&[sysml_file]);

        for (_, element) in result.graph.elements {
            kinds.insert(format!("{:?}", element.kind));
        }
    }

    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_files_empty_config() {
        let config = CoverageConfig {
            corpus_path: PathBuf::from("/nonexistent"),
            corpus_subdirs: vec!["subdir"],
        };
        let files = discover_corpus_files(&config);
        assert!(files.is_empty());
    }
}
