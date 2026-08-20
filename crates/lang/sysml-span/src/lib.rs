//! # sysml-span
//!
//! Source locations, diagnostics, and severity levels for SysML v2.
//!
//! This crate provides types for tracking source code locations and
//! reporting diagnostics (errors, warnings, etc.).
//!
//! ## Features
//!
//! - `serde`: Enable serde serialization support
//!
//! ## Examples
//!
//! ```
//! use sysml_span::{Span, Diagnostic, Severity};
//!
//! // Create a span for a source location
//! let span = Span::with_location("example.sysml", 100, 150, 5, 10);
//!
//! // Create an error diagnostic
//! let error = Diagnostic::error("unexpected token")
//!     .with_span(span)
//!     .with_code("E001")
//!     .with_note("expected ';' here");
//!
//! assert!(error.is_error());
//! ```

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "pretty")]
mod pretty;
#[cfg(feature = "pretty")]
pub use pretty::{DiagnosticRenderer, HashMapSourceProvider, SourceProvider};

/// A span representing a range in a source file.
///
/// # Examples
///
/// ```
/// use sysml_span::Span;
///
/// // Create a span with byte offsets
/// let span = Span::new("file.sysml", 10, 20);
/// assert_eq!(span.len(), 10);
/// assert!(span.contains(15));
///
/// // Create a span with line/column info
/// let span = Span::with_location("file.sysml", 10, 20, 5, 3);
/// assert_eq!(span.line, Some(5));
/// assert_eq!(span.col, Some(3));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Span {
    /// The file path or URI.
    pub file: String,
    /// Start byte offset (0-indexed).
    pub start: usize,
    /// End byte offset (exclusive).
    pub end: usize,
    /// Start line number (1-indexed, optional).
    pub line: Option<u32>,
    /// Start column number (1-indexed, optional).
    pub col: Option<u32>,
}

impl Span {
    /// Create a new span with byte offsets only.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Span;
    ///
    /// let span = Span::new("file.sysml", 0, 100);
    /// assert_eq!(span.file, "file.sysml");
    /// assert_eq!(span.start, 0);
    /// assert_eq!(span.end, 100);
    /// ```
    pub fn new(file: impl Into<String>, start: usize, end: usize) -> Self {
        Span {
            file: file.into(),
            start,
            end,
            line: None,
            col: None,
        }
    }

    /// Create a new span with line and column information.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Span;
    ///
    /// let span = Span::with_location("file.sysml", 10, 20, 5, 3);
    /// assert_eq!(span.to_string(), "file.sysml:5:3");
    /// ```
    pub fn with_location(
        file: impl Into<String>,
        start: usize,
        end: usize,
        line: u32,
        col: u32,
    ) -> Self {
        Span {
            file: file.into(),
            start,
            end,
            line: Some(line),
            col: Some(col),
        }
    }

    /// Create a span at a single point.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Span;
    ///
    /// let span = Span::point("file.sysml", 50);
    /// assert_eq!(span.start, 50);
    /// assert_eq!(span.end, 50);
    /// assert!(span.is_empty());
    /// ```
    pub fn point(file: impl Into<String>, offset: usize) -> Self {
        Span::new(file, offset, offset)
    }

    /// Create a synthetic span (no real source location).
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Span;
    ///
    /// let span = Span::synthetic();
    /// assert_eq!(span.file, "<synthetic>");
    /// ```
    pub fn synthetic() -> Self {
        Span {
            file: "<synthetic>".to_owned(),
            start: 0,
            end: 0,
            line: None,
            col: None,
        }
    }

    /// Get the length of this span in bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Span;
    ///
    /// let span = Span::new("file.sysml", 10, 20);
    /// assert_eq!(span.len(), 10);
    /// ```
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Check if this span is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Span;
    ///
    /// let span = Span::point("file.sysml", 10);
    /// assert!(span.is_empty());
    ///
    /// let span = Span::new("file.sysml", 10, 20);
    /// assert!(!span.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Check if this span contains a byte offset.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Span;
    ///
    /// let span = Span::new("file.sysml", 10, 20);
    /// assert!(span.contains(15));
    /// assert!(!span.contains(25));
    /// assert!(span.contains(10));  // Inclusive start
    /// assert!(!span.contains(20)); // Exclusive end
    /// ```
    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset < self.end
    }

    /// Merge two spans into one covering both.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Span;
    ///
    /// let a = Span::new("file.sysml", 10, 20);
    /// let b = Span::new("file.sysml", 15, 30);
    /// let merged = a.merge(&b);
    /// assert_eq!(merged.start, 10);
    /// assert_eq!(merged.end, 30);
    /// ```
    pub fn merge(&self, other: &Span) -> Span {
        Span {
            file: self.file.clone(),
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            line: self.line.or(other.line),
            col: self.col.or(other.col),
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let (Some(line), Some(col)) = (self.line, self.col) {
            write!(f, "{}:{}:{}", self.file, line, col)
        } else {
            write!(f, "{}:{}-{}", self.file, self.start, self.end)
        }
    }
}

/// Severity level for diagnostics.
///
/// # Examples
///
/// ```
/// use sysml_span::Severity;
///
/// assert!(Severity::Error.is_error());
/// assert!(Severity::Warning.is_warning_or_error());
/// assert!(!Severity::Info.is_error());
///
/// // Severities are ordered
/// assert!(Severity::Info < Severity::Warning);
/// assert!(Severity::Warning < Severity::Error);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Severity {
    /// Informational message.
    Info,
    /// Warning that doesn't prevent processing.
    Warning,
    /// Error that prevents successful processing.
    #[default]
    Error,
}

impl Severity {
    /// Check if this is an error.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Severity;
    ///
    /// assert!(Severity::Error.is_error());
    /// assert!(!Severity::Warning.is_error());
    /// ```
    pub fn is_error(&self) -> bool {
        matches!(self, Severity::Error)
    }

    /// Check if this is a warning or error.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Severity;
    ///
    /// assert!(Severity::Error.is_warning_or_error());
    /// assert!(Severity::Warning.is_warning_or_error());
    /// assert!(!Severity::Info.is_warning_or_error());
    /// ```
    pub fn is_warning_or_error(&self) -> bool {
        matches!(self, Severity::Warning | Severity::Error)
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Info => write!(f, "info"),
            Severity::Warning => write!(f, "warning"),
            Severity::Error => write!(f, "error"),
        }
    }
}


/// A related location in a diagnostic, used to provide additional context.
///
/// # Examples
///
/// ```
/// use sysml_span::{RelatedLocation, Span};
///
/// let related = RelatedLocation {
///     span: Span::with_location("other.sysml", 50, 60, 10, 5),
///     message: "defined here".to_string(),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RelatedLocation {
    /// The source location.
    pub span: Span,
    /// A message describing the relationship.
    pub message: String,
}

impl RelatedLocation {
    /// Create a new related location.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::{RelatedLocation, Span};
    ///
    /// let span = Span::with_location("file.sysml", 10, 20, 5, 3);
    /// let related = RelatedLocation::new(span, "first defined here");
    /// ```
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        RelatedLocation {
            span,
            message: message.into(),
        }
    }
}

impl fmt::Display for RelatedLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.span, self.message)
    }
}

/// Tags that provide additional metadata about a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum DiagnosticTag {
    /// The diagnostic reports unnecessary or unused code (rendered faded in IDEs).
    Unnecessary,
    /// The diagnostic reports deprecated code (rendered with strikethrough in IDEs).
    Deprecated,
}

/// Tier classifying which analysis context a diagnostic depended on.
///
/// Tiers are used by the readiness filter (`P-RA3`) to decide whether a
/// diagnostic is safe to publish given the current `AnalysisHost` state.
/// A diagnostic that depends on context not yet loaded (e.g. cross-file
/// name resolution before workspace indexing) is filtered out entirely
/// rather than emitted-then-silenced.
///
/// Tiers are ordered from cheapest / always-safe (`Parse`) to most
/// dependent (`Constraint`, which requires evaluated runtime values).
///
/// # Examples
///
/// ```
/// use sysml_span::DiagnosticTier;
///
/// // Default tier is Parse — backward compatible with pre-tier callers.
/// let t = DiagnosticTier::default();
/// assert_eq!(t, DiagnosticTier::Parse);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum DiagnosticTier {
    /// Syntactically derivable from the file's own text. Always safe to emit.
    #[default]
    Parse,
    /// Within-file structural checks (S001-S004). Needs parse only.
    StructuralLocal,
    /// Bare-name resolution within the file's own scope.
    NameResLocal,
    /// Name resolution that requires the standard library loaded
    /// (e.g. `Real`, `Integer`).
    NameResLibrary,
    /// Name resolution that requires the project file set indexed
    /// (cross-file `WaterPort`-style references).
    NameResWorkspace,
    /// Import-health diagnostics (IM010, IM011, IM012).
    ImportHealth,
    /// Semantic passes that run after resolve: PH*, SM*, V*.
    Semantic,
    /// Constraint diagnostics that require evaluated runtime values.
    Constraint,
}

/// A diagnostic message with location and severity.
///
/// # Examples
///
/// ```
/// use sysml_span::{Diagnostic, Span};
///
/// let error = Diagnostic::error("unexpected token")
///     .with_code("E001")
///     .with_span(Span::with_location("file.sysml", 10, 20, 5, 3))
///     .with_note("expected identifier");
///
/// assert!(error.is_error());
/// assert_eq!(error.code, Some("E001".to_string()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Diagnostic {
    /// The severity of this diagnostic.
    pub severity: Severity,
    /// An optional error code.
    pub code: Option<String>,
    /// The main message.
    pub message: String,
    /// The source location (optional).
    pub span: Option<Span>,
    /// Additional notes or suggestions.
    pub notes: Vec<String>,
    /// Related locations that provide additional context.
    pub related: Vec<RelatedLocation>,
    /// Tags that provide additional metadata (e.g., unnecessary, deprecated).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub tags: Vec<DiagnosticTag>,
    /// Tier classifying which analysis context this diagnostic depended on.
    ///
    /// Defaults to `DiagnosticTier::Parse` so existing emission sites remain
    /// backward-compatible until they're explicitly tagged (Wave 2 of
    /// written before this field existed deserialize back into `Parse`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub tier: DiagnosticTier,
}

impl PartialOrd for Diagnostic {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Diagnostic {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.span
            .cmp(&other.span)
            .then_with(|| self.severity.cmp(&other.severity))
            .then_with(|| self.message.cmp(&other.message))
    }
}

impl Diagnostic {
    /// Create a new error diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostic;
    ///
    /// let error = Diagnostic::error("syntax error");
    /// assert!(error.is_error());
    /// ```
    pub fn error(message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            code: None,
            message: message.into(),
            span: None,
            notes: Vec::new(),
            related: Vec::new(),
            tags: Vec::new(),
            tier: DiagnosticTier::Parse,
        }
    }

    /// Create a new warning diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostic;
    ///
    /// let warning = Diagnostic::warning("unused variable");
    /// assert!(!warning.is_error());
    /// ```
    pub fn warning(message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            code: None,
            message: message.into(),
            span: None,
            notes: Vec::new(),
            related: Vec::new(),
            tags: Vec::new(),
            tier: DiagnosticTier::Parse,
        }
    }

    /// Create a new info diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostic;
    ///
    /// let info = Diagnostic::info("parsing file");
    /// assert!(!info.is_error());
    /// ```
    pub fn info(message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Info,
            code: None,
            message: message.into(),
            span: None,
            notes: Vec::new(),
            related: Vec::new(),
            tags: Vec::new(),
            tier: DiagnosticTier::Parse,
        }
    }

    /// Add a span to this diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::{Diagnostic, Span};
    ///
    /// let error = Diagnostic::error("error")
    ///     .with_span(Span::with_location("file.sysml", 10, 20, 5, 3));
    /// assert!(error.span.is_some());
    /// ```
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Add an error code to this diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostic;
    ///
    /// let error = Diagnostic::error("error").with_code("E001");
    /// assert_eq!(error.code, Some("E001".to_string()));
    /// ```
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Add a note to this diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostic;
    ///
    /// let error = Diagnostic::error("error")
    ///     .with_note("try this instead");
    /// assert_eq!(error.notes.len(), 1);
    /// ```
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Add multiple notes to this diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostic;
    ///
    /// let error = Diagnostic::error("error")
    ///     .with_notes(["hint 1", "hint 2"]);
    /// assert_eq!(error.notes.len(), 2);
    /// ```
    pub fn with_notes(mut self, notes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.notes.extend(notes.into_iter().map(|n| n.into()));
        self
    }

    /// Add a related location to this diagnostic.
    ///
    /// Related locations provide additional context about the diagnostic,
    /// such as where a symbol was first defined.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::{Diagnostic, Span};
    ///
    /// let error = Diagnostic::error("duplicate definition")
    ///     .with_span(Span::with_location("file.sysml", 100, 110, 10, 5))
    ///     .with_related(
    ///         Span::with_location("file.sysml", 50, 60, 5, 5),
    ///         "first defined here"
    ///     );
    /// assert_eq!(error.related.len(), 1);
    /// ```
    pub fn with_related(mut self, span: Span, message: impl Into<String>) -> Self {
        self.related.push(RelatedLocation::new(span, message));
        self
    }

    /// Add a related location struct to this diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::{Diagnostic, RelatedLocation, Span};
    ///
    /// let related = RelatedLocation::new(
    ///     Span::with_location("file.sysml", 50, 60, 5, 5),
    ///     "first defined here"
    /// );
    /// let error = Diagnostic::error("duplicate definition")
    ///     .with_related_location(related);
    /// assert_eq!(error.related.len(), 1);
    /// ```
    pub fn with_related_location(mut self, related: RelatedLocation) -> Self {
        self.related.push(related);
        self
    }

    /// Add a tag to this diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::{Diagnostic, DiagnosticTag};
    ///
    /// let warning = Diagnostic::warning("unused import")
    ///     .with_tag(DiagnosticTag::Unnecessary);
    /// assert_eq!(warning.tags.len(), 1);
    /// ```
    pub fn with_tag(mut self, tag: DiagnosticTag) -> Self {
        self.tags.push(tag);
        self
    }

    /// Set the readiness tier of this diagnostic.
    ///
    /// Tier classifies which analysis context the diagnostic depended on; the
    /// readiness filter (`P-RA3`) uses this to decide whether the diagnostic
    /// is safe to publish given the current `AnalysisHost` state.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::{Diagnostic, DiagnosticTier};
    ///
    /// let err = Diagnostic::error("unresolved name")
    ///     .with_tier(DiagnosticTier::NameResWorkspace);
    /// assert_eq!(err.tier, DiagnosticTier::NameResWorkspace);
    /// ```
    pub fn with_tier(mut self, tier: DiagnosticTier) -> Self {
        self.tier = tier;
        self
    }

    /// Check if this is an error.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostic;
    ///
    /// assert!(Diagnostic::error("error").is_error());
    /// assert!(!Diagnostic::warning("warning").is_error());
    /// ```
    pub fn is_error(&self) -> bool {
        self.severity.is_error()
    }

    /// Render this diagnostic using annotate-snippets (feature = "pretty").
    #[cfg(feature = "pretty")]
    pub fn render_snippet(&self, provider: &impl SourceProvider) -> String {
        DiagnosticRenderer::new().render(self, provider)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Format: severity[code]: message
        write!(f, "{}", self.severity)?;
        if let Some(code) = &self.code {
            write!(f, "[{}]", code)?;
        }
        write!(f, ": {}", self.message)?;

        // Add location if available
        if let Some(span) = &self.span {
            write!(f, "\n  --> {}", span)?;
        }

        // Add notes
        for note in &self.notes {
            write!(f, "\n  = note: {}", note)?;
        }

        // Add related locations
        for related in &self.related {
            write!(f, "\n  = related: {} ({})", related.span, related.message)?;
        }

        Ok(())
    }
}

/// A collection of diagnostics with helper methods.
///
/// # Examples
///
/// ```
/// use sysml_span::{Diagnostics, Diagnostic};
///
/// let mut diags = Diagnostics::new();
/// diags.error("error 1");
/// diags.warning("warning 1");
/// diags.error("error 2");
///
/// assert!(diags.has_errors());
/// assert_eq!(diags.error_count(), 2);
/// assert_eq!(diags.len(), 3);
/// ```
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, diag) in self.items.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{}", diag)?;
        }
        Ok(())
    }
}

impl Diagnostics {
    /// Create an empty diagnostics collection.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostics;
    ///
    /// let diags = Diagnostics::new();
    /// assert!(diags.is_empty());
    /// ```
    pub fn new() -> Self {
        Diagnostics { items: Vec::new() }
    }

    /// Add a diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::{Diagnostics, Diagnostic};
    ///
    /// let mut diags = Diagnostics::new();
    /// diags.push(Diagnostic::error("something went wrong"));
    /// assert_eq!(diags.len(), 1);
    /// ```
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }

    /// Add an error.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostics;
    ///
    /// let mut diags = Diagnostics::new();
    /// diags.error("something went wrong");
    /// assert!(diags.has_errors());
    /// ```
    pub fn error(&mut self, message: impl Into<String>) {
        self.push(Diagnostic::error(message));
    }

    /// Add a warning.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostics;
    ///
    /// let mut diags = Diagnostics::new();
    /// diags.warning("potential issue");
    /// assert!(!diags.has_errors());
    /// assert!(!diags.is_empty());
    /// ```
    pub fn warning(&mut self, message: impl Into<String>) {
        self.push(Diagnostic::warning(message));
    }

    /// Check if there are any errors.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostics;
    ///
    /// let mut diags = Diagnostics::new();
    /// assert!(!diags.has_errors());
    /// diags.error("error");
    /// assert!(diags.has_errors());
    /// ```
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.is_error())
    }

    /// Get the number of errors.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostics;
    ///
    /// let mut diags = Diagnostics::new();
    /// diags.error("error 1");
    /// diags.warning("warning");
    /// diags.error("error 2");
    /// assert_eq!(diags.error_count(), 2);
    /// ```
    pub fn error_count(&self) -> usize {
        self.items.iter().filter(|d| d.is_error()).count()
    }

    /// Get all diagnostics.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostics;
    ///
    /// let mut diags = Diagnostics::new();
    /// diags.error("error");
    /// for d in diags.iter() {
    ///     println!("{}", d);
    /// }
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter()
    }

    /// Get all diagnostics as a vec.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostics;
    ///
    /// let mut diags = Diagnostics::new();
    /// diags.error("error");
    /// let vec = diags.into_vec();
    /// assert_eq!(vec.len(), 1);
    /// ```
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }

    /// Check if empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostics;
    ///
    /// let diags = Diagnostics::new();
    /// assert!(diags.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Get the number of diagnostics.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::Diagnostics;
    ///
    /// let mut diags = Diagnostics::new();
    /// diags.error("error");
    /// diags.warning("warning");
    /// assert_eq!(diags.len(), 2);
    /// ```
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl FromIterator<Diagnostic> for Diagnostics {
    fn from_iter<T: IntoIterator<Item = Diagnostic>>(iter: T) -> Self {
        Diagnostics {
            items: iter.into_iter().collect(),
        }
    }
}

impl Extend<Diagnostic> for Diagnostics {
    fn extend<T: IntoIterator<Item = Diagnostic>>(&mut self, iter: T) {
        self.items.extend(iter);
    }
}

/// Pre-computed line offset table for O(log n) line/column lookups.
///
/// Pest's `Position::line_col()` is O(n) - it scans from byte 0 to the position,
/// counting newlines. For a file with N tokens, this results in O(N²) total work.
/// This struct pre-computes line offsets once, enabling O(log n) lookups via binary search.
///
/// # Examples
///
/// ```
/// use sysml_span::LineIndex;
///
/// let source = "line 1\nline 2\nline 3";
/// let index = LineIndex::new(source);
///
/// // Position at start of "line 2" (byte 7)
/// assert_eq!(index.line_col(7), (2, 1));
///
/// // Position in middle of "line 3" (byte 17 = 'n' in "line")
/// assert_eq!(index.line_col(17), (3, 4));
/// ```
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offset of each line start. line_offsets[0] = 0 (first line starts at byte 0).
    line_offsets: Vec<usize>,
}

impl LineIndex {
    /// Build a line index from source text. O(n) one-time cost.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::LineIndex;
    ///
    /// let index = LineIndex::new("a\nb\nc");
    /// // 3 lines: starts at bytes 0, 2, 4
    /// ```
    pub fn new(source: &str) -> Self {
        let mut offsets = vec![0];
        for (i, c) in source.char_indices() {
            if c == '\n' {
                offsets.push(i + 1);
            }
        }
        Self {
            line_offsets: offsets,
        }
    }

    /// Convert byte offset to (line, column). O(log n) via binary search.
    ///
    /// Line and column are 1-indexed to match pest's convention and editor expectations.
    ///
    /// # Examples
    ///
    /// ```
    /// use sysml_span::LineIndex;
    ///
    /// let index = LineIndex::new("hello\nworld");
    /// assert_eq!(index.line_col(0), (1, 1));   // 'h'
    /// assert_eq!(index.line_col(5), (1, 6));   // '\n'
    /// assert_eq!(index.line_col(6), (2, 1));   // 'w'
    /// assert_eq!(index.line_col(10), (2, 5));  // 'd'
    /// ```
    pub fn line_col(&self, offset: usize) -> (u32, u32) {
        // partition_point returns the first index where offset < line_offsets[i]
        // So line = partition_point gives us 1-indexed line number directly
        let line = self.line_offsets.partition_point(|&o| o <= offset);
        let line_start = self
            .line_offsets
            .get(line.saturating_sub(1))
            .copied()
            .unwrap_or(0);
        let col = offset - line_start + 1;
        (line as u32, col as u32)
    }

    /// Get the number of lines in the indexed source.
    pub fn line_count(&self) -> usize {
        self.line_offsets.len()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn span_basic() {
        let span = Span::new("test.sysml", 10, 20);
        assert_eq!(span.len(), 10);
        assert!(!span.is_empty());
        assert!(span.contains(15));
        assert!(!span.contains(25));
    }

    #[test]
    fn span_with_location() {
        let span = Span::with_location("test.sysml", 10, 20, 5, 3);
        assert_eq!(span.line, Some(5));
        assert_eq!(span.col, Some(3));
        assert_eq!(span.to_string(), "test.sysml:5:3");
    }

    #[test]
    fn span_merge() {
        let a = Span::new("test.sysml", 10, 20);
        let b = Span::new("test.sysml", 15, 30);
        let merged = a.merge(&b);
        assert_eq!(merged.start, 10);
        assert_eq!(merged.end, 30);
    }

    #[test]
    fn diagnostic_error() {
        let diag = Diagnostic::error("something went wrong")
            .with_code("E001")
            .with_span(Span::with_location("test.sysml", 0, 10, 1, 1))
            .with_note("try doing X instead");

        assert!(diag.is_error());
        assert_eq!(diag.code, Some("E001".to_string()));
        assert_eq!(diag.notes.len(), 1);
    }

    #[test]
    fn diagnostic_display() {
        let diag = Diagnostic::error("parse error")
            .with_code("E001")
            .with_span(Span::with_location("test.sysml", 0, 10, 1, 1));

        let s = diag.to_string();
        assert!(s.contains("error[E001]"));
        assert!(s.contains("parse error"));
        assert!(s.contains("test.sysml:1:1"));
    }

    #[test]
    fn diagnostics_collection() {
        let mut diags = Diagnostics::new();
        diags.error("error 1");
        diags.warning("warning 1");
        diags.error("error 2");

        assert!(diags.has_errors());
        assert_eq!(diags.error_count(), 2);
        assert_eq!(diags.len(), 3);
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
    }

    #[test]
    fn diagnostic_default_tier_is_parse() {
        // Existing constructors must yield Parse-tier diagnostics so callers
        // that haven't been migrated to .with_tier(…) yet stay backward-compatible.
        assert_eq!(Diagnostic::error("e").tier, DiagnosticTier::Parse);
        assert_eq!(Diagnostic::warning("w").tier, DiagnosticTier::Parse);
        assert_eq!(Diagnostic::info("i").tier, DiagnosticTier::Parse);
    }

    #[test]
    fn diagnostic_with_tier_roundtrips() {
        let diag = Diagnostic::error("unresolved name")
            .with_code("E200")
            .with_tier(DiagnosticTier::Semantic);
        assert_eq!(diag.tier, DiagnosticTier::Semantic);

        // Setting tier doesn't disturb other builder fields.
        assert_eq!(diag.code.as_deref(), Some("E200"));
        assert!(diag.is_error());
    }

    #[test]
    fn diagnostic_with_related() {
        let error = Diagnostic::error("duplicate definition")
            .with_span(Span::with_location("file.sysml", 100, 110, 10, 5))
            .with_related(
                Span::with_location("file.sysml", 50, 60, 5, 5),
                "first defined here",
            );

        assert_eq!(error.related.len(), 1);
        assert_eq!(error.related[0].message, "first defined here");

        let s = error.to_string();
        assert!(s.contains("related:"));
        assert!(s.contains("first defined here"));
    }

    #[test]
    fn related_location_display() {
        let related = RelatedLocation::new(
            Span::with_location("file.sysml", 10, 20, 5, 3),
            "defined here",
        );
        let s = related.to_string();
        assert!(s.contains("file.sysml:5:3"));
        assert!(s.contains("defined here"));
    }

    // LineIndex tests

    #[test]
    fn line_index_single_line() {
        let index = LineIndex::new("hello world");
        assert_eq!(index.line_col(0), (1, 1));
        assert_eq!(index.line_col(5), (1, 6));
        assert_eq!(index.line_col(10), (1, 11));
    }

    #[test]
    fn line_index_multiple_lines() {
        let index = LineIndex::new("line 1\nline 2\nline 3");
        // Line 1: bytes 0-6 ("line 1\n")
        assert_eq!(index.line_col(0), (1, 1));
        assert_eq!(index.line_col(5), (1, 6));
        // Line 2: bytes 7-13 ("line 2\n")
        assert_eq!(index.line_col(7), (2, 1));
        assert_eq!(index.line_col(12), (2, 6));
        // Line 3: bytes 14-19 ("line 3")
        assert_eq!(index.line_col(14), (3, 1));
        assert_eq!(index.line_col(19), (3, 6));
    }

    #[test]
    fn line_index_empty() {
        let index = LineIndex::new("");
        // Empty string still has line 1
        assert_eq!(index.line_count(), 1);
    }

    #[test]
    fn line_index_newline_at_end() {
        let index = LineIndex::new("hello\n");
        assert_eq!(index.line_col(0), (1, 1));
        assert_eq!(index.line_col(5), (1, 6)); // the newline character
    }

    #[test]
    fn line_index_consecutive_newlines() {
        let index = LineIndex::new("a\n\nb");
        // Line 1: "a\n"
        assert_eq!(index.line_col(0), (1, 1)); // 'a'
                                               // Line 2: "\n" (empty line)
        assert_eq!(index.line_col(2), (2, 1)); // second newline
                                               // Line 3: "b"
        assert_eq!(index.line_col(3), (3, 1)); // 'b'
    }
}
