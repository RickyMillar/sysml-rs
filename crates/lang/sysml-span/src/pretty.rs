use std::collections::HashMap;

use annotate_snippets::{Level, Renderer, Snippet};

use crate::{Diagnostic, Severity, Span};

/// Source provider for diagnostic rendering.
///
/// Implementations should return the full source text for the requested file.
pub trait SourceProvider {
    fn source(&self, file: &str) -> Option<&str>;
}

/// Simple in-memory source provider backed by a HashMap.
#[derive(Debug, Default, Clone)]
pub struct HashMapSourceProvider {
    sources: HashMap<String, String>,
}

impl HashMapSourceProvider {
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
        }
    }

    pub fn with_source(mut self, file: impl Into<String>, source: impl Into<String>) -> Self {
        self.insert(file, source);
        self
    }

    pub fn insert(&mut self, file: impl Into<String>, source: impl Into<String>) {
        self.sources.insert(file.into(), source.into());
    }
}

impl SourceProvider for HashMapSourceProvider {
    fn source(&self, file: &str) -> Option<&str> {
        self.sources.get(file).map(|s| s.as_str())
    }
}

/// Renderer for converting Diagnostics into annotate-snippets output.
#[derive(Debug, Clone)]
pub struct DiagnosticRenderer {
    renderer: Renderer,
    fold: bool,
}

impl DiagnosticRenderer {
    /// Create a renderer with plain (no color) output.
    pub fn new() -> Self {
        Self::styled()
    }

    /// Create a renderer with plain (no color) output.
    pub fn plain() -> Self {
        Self::new()
    }

    /// Create a renderer with styled (colored) output.
    pub fn styled() -> Self {
        Self {
            renderer: Renderer::styled(),
            fold: true,
        }
    }

    /// Create a renderer from a custom annotate-snippets Renderer.
    pub fn with_renderer(renderer: Renderer) -> Self {
        Self {
            renderer,
            fold: true,
        }
    }

    /// Toggle line folding (hide lines without annotations).
    pub fn fold(mut self, fold: bool) -> Self {
        self.fold = fold;
        self
    }

    /// Render a single diagnostic using the provided source map.
    pub fn render(&self, diagnostic: &Diagnostic, provider: &impl SourceProvider) -> String {
        let mut primary_span = diagnostic.span.as_ref();
        let mut primary_label = Some(diagnostic.message.as_str());
        let mut primary_source = primary_span.and_then(|s| provider.source(&s.file));
        let mut used_related_index: Option<usize> = None;

        if primary_source.is_none() {
            for (idx, related) in diagnostic.related.iter().enumerate() {
                if let Some(source) = provider.source(&related.span.file) {
                    primary_span = Some(&related.span);
                    primary_source = Some(source);
                    used_related_index = Some(idx);
                    if !related.message.is_empty() {
                        primary_label = Some(related.message.as_str());
                    }
                    break;
                }
            }
        }

        let Some(span) = primary_span else {
            return diagnostic.to_string();
        };
        let Some(source) = primary_source else {
            return diagnostic.to_string();
        };

        let level = level_from_severity(diagnostic.severity);
        let mut message = level.title(diagnostic.message.as_str());
        if let Some(code) = diagnostic.code.as_deref() {
            message = message.id(code);
        }

        let main_snippet = snippet_for_span(level, span, primary_label, source, self.fold);
        message = message.snippet(main_snippet);

        if diagnostic.span.is_none() && used_related_index.is_some() {
            message = message
                .footer(Level::Note.title("primary span unavailable; showing related location"));
        }

        for note in &diagnostic.notes {
            message = message.footer(Level::Note.title(note.as_str()));
        }

        let mut missing_related: Vec<String> = Vec::new();
        for (idx, related) in diagnostic.related.iter().enumerate() {
            if used_related_index == Some(idx) {
                continue;
            }
            if let Some(related_source) = provider.source(&related.span.file) {
                let related_snippet = snippet_for_span(
                    Level::Note,
                    &related.span,
                    Some(related.message.as_str()),
                    related_source,
                    self.fold,
                );
                message = message.snippet(related_snippet);
            } else {
                missing_related.push(format!(
                    "related: {} ({})",
                    related.span,
                    related.message
                ));
            }
        }

        for footer in missing_related.iter() {
            message = message.footer(Level::Note.title(footer.as_str()));
        }

        let rendered = self.renderer.render(message).to_string();
        rendered
    }

    /// Render multiple diagnostics into a single string.
    pub fn render_all<'a>(
        &self,
        diagnostics: impl IntoIterator<Item = &'a Diagnostic>,
        provider: &impl SourceProvider,
    ) -> String {
        let mut out = String::new();
        for (idx, diag) in diagnostics.into_iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            out.push_str(self.render(diag, provider).as_str());
        }
        out
    }
}

impl Default for DiagnosticRenderer {
    fn default() -> Self {
        Self::new()
    }
}

fn level_from_severity(severity: Severity) -> Level {
    match severity {
        Severity::Error => Level::Error,
        Severity::Warning => Level::Warning,
        Severity::Info => Level::Info,
    }
}

fn snippet_for_span<'a>(
    level: Level,
    span: &'a Span,
    label: Option<&'a str>,
    source: &'a str,
    fold: bool,
) -> Snippet<'a> {
    let range = normalized_range(span.start, span.end, source.len());
    let mut annotation = level.span(range);
    if let Some(label) = label {
        annotation = annotation.label(label);
    }

    Snippet::source(source)
        .line_start(1)
        .origin(span.file.as_str())
        .annotation(annotation)
        .fold(fold)
}

fn normalized_range(start: usize, end: usize, len: usize) -> std::ops::Range<usize> {
    let mut start = start.min(len);
    let mut end = end.min(len);
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    if start == end && end < len {
        end += 1;
    }
    start..end
}
