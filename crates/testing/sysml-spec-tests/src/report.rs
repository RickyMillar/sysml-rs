//! Coverage report generation.
//!
//! This module generates human-readable coverage reports.

use std::collections::{HashMap, HashSet};

use crate::{CoverageSummary, FileParseResult};

/// Generate a coverage report as a string.
pub fn generate_report(
    results: &[FileParseResult],
    summary: &CoverageSummary,
    element_kinds_produced: &HashSet<String>,
    element_kinds_expected: &HashSet<String>,
    rules_exercised: Option<&HashSet<String>>,
    total_rules: Option<usize>,
) -> String {
    let mut report = String::new();

    // Header
    report.push_str("=== SysML v2 Parser Coverage Report ===\n\n");

    // Corpus coverage
    report.push_str(&format!(
        "Corpus:       {}/{} files parsed ({:.1}%)\n",
        summary.passed_files,
        summary.total_files,
        summary.pass_percentage()
    ));

    // Grammar rules (if available)
    if let (Some(exercised), Some(total)) = (rules_exercised, total_rules) {
        let percentage = if total > 0 {
            (exercised.len() as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        report.push_str(&format!(
            "Rules:        {}/{} rules exercised ({:.1}%)\n",
            exercised.len(),
            total,
            percentage
        ));
    }

    // Element kinds
    let kinds_produced = element_kinds_produced.len();
    let kinds_expected = element_kinds_expected.len();
    let kinds_percentage = if kinds_expected > 0 {
        (kinds_produced as f64 / kinds_expected as f64) * 100.0
    } else {
        0.0
    };
    report.push_str(&format!(
        "ElementKinds: {}/{} kinds produced ({:.1}%)\n",
        kinds_produced, kinds_expected, kinds_percentage
    ));

    report.push('\n');

    // Failed files section
    let failed: Vec<_> = results.iter().filter(|r| !r.success).collect();
    if !failed.is_empty() {
        report.push_str(&format!("Failed Files ({}):\n", failed.len()));
        for result in &failed {
            // Extract just the filename for compact display
            let filename = result.path.rsplit('/').next().unwrap_or(&result.path);
            report.push_str(&format!("  - {}", filename));
            if !result.errors.is_empty() {
                // Show first error, truncated to 200 chars for readability
                // but prioritize showing the line:column location
                let first_error = &result.errors[0];
                let truncated = if first_error.len() > 200 {
                    format!("{}...", &first_error[..200])
                } else {
                    first_error.clone()
                };
                report.push_str(&format!(": {}", truncated));
            }
            report.push('\n');
        }
        report.push('\n');
    }

    // Uncovered element kinds
    let uncovered: HashSet<_> = element_kinds_expected
        .difference(element_kinds_produced)
        .collect();
    if !uncovered.is_empty() {
        let mut uncovered_sorted: Vec<_> = uncovered.into_iter().collect();
        uncovered_sorted.sort();
        report.push_str(&format!(
            "Uncovered ElementKinds ({}):\n",
            uncovered_sorted.len()
        ));
        for kind in uncovered_sorted {
            report.push_str(&format!("  - {}\n", kind));
        }
        report.push('\n');
    }

    // Error pattern grouping - helps identify common grammar gaps
    let mut error_patterns: HashMap<String, Vec<&str>> = HashMap::new();
    for result in results.iter().filter(|r| !r.success) {
        for error in &result.errors {
            // Extract the "expected X" pattern from the error
            if let Some(pattern) = extract_expected_pattern(error) {
                let filename = result.path.rsplit('/').next().unwrap_or(&result.path);
                error_patterns.entry(pattern).or_default().push(filename);
            }
        }
    }

    if !error_patterns.is_empty() {
        // Sort by frequency (most common first)
        let mut patterns: Vec<_> = error_patterns.into_iter().collect();
        patterns.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

        report.push_str("Error Patterns (grouped by grammar gap):\n");
        for (pattern, files) in patterns.iter().take(10) {
            report.push_str(&format!(
                "  [{:2}] expected {}\n",
                files.len(),
                truncate_str(pattern, 60)
            ));
            // Show up to 3 affected files
            for file in files.iter().take(3) {
                report.push_str(&format!("       - {}\n", file));
            }
            if files.len() > 3 {
                report.push_str(&format!("       ... and {} more\n", files.len() - 3));
            }
        }
        report.push('\n');
    }

    // Summary line
    if summary.unexpected_failures > 0 {
        report.push_str(&format!(
            "⚠️  {} unexpected failures (not in allow-list)\n",
            summary.unexpected_failures
        ));
    } else {
        report.push_str("✓ All failures are expected (in allow-list)\n");
    }

    report
}

/// Extract the "expected X" pattern from a pest error message.
fn extract_expected_pattern(error: &str) -> Option<String> {
    // Look for "expected X" pattern in the error
    if let Some(idx) = error.find("expected ") {
        let rest = &error[idx + 9..];
        // Find the end of the expected list (at newline or " --> ")
        let end = rest
            .find("\n")
            .or_else(|| rest.find(" --> "))
            .unwrap_or(rest.len());
        let pattern = rest[..end].trim();
        if !pattern.is_empty() {
            return Some(pattern.to_string());
        }
    }
    None
}

/// Truncate a string to max_len, adding "..." if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len])
    } else {
        s.to_string()
    }
}

/// Format a list of failures for assertion messages.
pub fn format_failures(failures: &[(String, Vec<String>)]) -> String {
    let mut output = String::new();
    for (path, errors) in failures {
        output.push_str(&format!("\n{}:\n", path));
        for error in errors.iter().take(3) {
            // Limit to 3 errors per file
            output.push_str(&format!("  - {}\n", error));
        }
        if errors.len() > 3 {
            output.push_str(&format!("  ... and {} more errors\n", errors.len() - 3));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_report_basic() {
        let results = vec![
            FileParseResult {
                path: "test1.sysml".to_string(),
                success: true,
                errors: vec![],
                element_count: 5,
            },
            FileParseResult {
                path: "test2.sysml".to_string(),
                success: false,
                errors: vec!["syntax error".to_string()],
                element_count: 0,
            },
        ];

        let summary = CoverageSummary {
            total_files: 2,
            passed_files: 1,
            failed_files: 1,
            expected_failures: 1,
            unexpected_failures: 0,
            rules_exercised: HashSet::new(),
            element_kinds_produced: HashSet::new(),
        };

        let kinds_produced: HashSet<_> = ["Package", "PartUsage"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let kinds_expected: HashSet<_> = ["Package", "PartUsage", "ActionUsage"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let report = generate_report(
            &results,
            &summary,
            &kinds_produced,
            &kinds_expected,
            None,
            None,
        );

        assert!(report.contains("1/2 files parsed"));
        assert!(report.contains("test2.sysml"));
        assert!(report.contains("ActionUsage"));
    }
}
