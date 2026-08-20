//! Design tokens (Bucket 1.3) — the **single Rust source of truth** for the
//! diagram color palette.
//!
//! Historically the OKLCH palette lived only in the deleted
//! `editors/diagram/src/css/sysml-diagram.css` (`:root` custom properties). That
//! made the colors a TypeScript-side concern the Rust pipeline could not reason
//! about. This module is the single Rust source, keyed by the existing
//! [`VisualKind`] taxonomy (see `visual_kind.rs`); the palette is serialized into
//! `ViewModel::tokens` and consumed directly by the React-SVG renderer. (The old
//! CSS emitter `emit_css_root` + its drift gate were retired with the Sprotty
//! `editors/diagram` package.)
//!
//! ## Scope (steward-ruled (A″), 2026-06-25)
//!
//! This task single-sources the **palette only**. Node *geometry*
//! (`shape-catalog.json`: sizes, padding, corner radii, shape-intrinsic SVG
//! params) and *typography* (`defaults.fonts`) are deliberately **NOT** carried
//! here: both feed the live Sprotty renderer's internal layout/text-measurement
//! contract (`shape-registry.ts`), and that contract is rewritten wholesale by
//! the Bucket-3 rip-and-replace renderer. Carrying them now would create an
//! un-gated duplicate (principle #5) or a dead-variable gate (principle #2).
//! They single-source in Bucket 3 under the new renderer's contract.
//!
//! ## Color representation
//!
//! Colors are validated **opaque strings** ([`Color`]) — CSS *is* the
//! computation layer; the spec is silent on color, so there is no color math in
//! Rust. [`Color::new`] fails hard on a malformed `oklch(…)` literal at
//! construction time, so a typo surfaces in the token table rather than silently
//! at render time.

use std::sync::Arc;
use std::sync::OnceLock;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use sysml_core::RelationshipKind;

use crate::visual_kind::{EdgeStyle, VisualKind};

/// A validated OKLCH color string — an opaque CSS value (no color math).
///
/// Construction validates the `oklch(L% C H [/ A])` shape and **panics** on a
/// malformed literal. The palette is a hardcoded table, so a panic here is a
/// fail-hard compile-adjacent error surfaced by the first test that builds the
/// tokens — not a runtime risk on user input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(transparent))]
pub struct Color(String);

impl Color {
    /// Construct a validated color. Panics if `s` is not a well-formed
    /// `oklch(…)` literal (see [`Color::validate`]).
    pub fn new(s: impl Into<String>) -> Self {
        let s = s.into();
        if let Err(e) = Self::validate(&s) {
            panic!("invalid design-token color {s:?}: {e}");
        }
        Self(s)
    }

    /// Validate an `oklch(L% C H)` / `oklch(L% C H / A)` literal. Lenient on
    /// whitespace and numeric formatting; strict on the overall shape so a gross
    /// typo (missing component, wrong function) is caught.
    pub fn validate(s: &str) -> Result<(), &'static str> {
        let body = s
            .strip_prefix("oklch(")
            .and_then(|b| b.strip_suffix(')'))
            .ok_or("must be of the form oklch(...)")?;
        // Split off an optional `/ alpha` tail.
        let (coords, alpha) = match body.split_once('/') {
            Some((c, a)) => (c, Some(a)),
            None => (body, None),
        };
        let comps: Vec<&str> = coords.split_whitespace().collect();
        if comps.len() != 3 {
            return Err("expected 3 space-separated components: L% C H");
        }
        // L must carry a % (e.g. "94%", "98.5%", "100%").
        let l = comps[0]
            .strip_suffix('%')
            .ok_or("lightness must end with %")?;
        if l.parse::<f64>().is_err() {
            return Err("lightness is not a number");
        }
        if comps[1].parse::<f64>().is_err() {
            return Err("chroma is not a number");
        }
        if comps[2].parse::<f64>().is_err() {
            return Err("hue is not a number");
        }
        if let Some(a) = alpha {
            if a.trim().parse::<f64>().is_err() {
                return Err("alpha is not a number");
            }
        }
        Ok(())
    }

    /// The raw CSS value (e.g. `"oklch(94% 0.04 155)"`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Fill / stroke / optional header-background for one node color category.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CategoryColors {
    pub fill: Color,
    pub stroke: Color,
    /// Header-bar background tint. `None` for categories that render without a
    /// header split (e.g. `comment`).
    pub header: Option<Color>,
}

/// Port stroke colors, by feature direction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PortColors {
    pub in_: Color,
    pub out: Color,
    pub inout: Color,
}

/// Relationship edge colors, by relationship family.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LinkColors {
    pub ownership: Color,
    pub typing: Color,
    pub specialization: Color,
    pub req: Color,
    pub verify: Color,
    pub derive: Color,
    pub flow: Color,
    pub dependency: Color,
    pub connection: Color,
    pub succession: Color,
}

/// Simulation-overlay colors + the inactive-dim opacity (a plain CSS number, not
/// a color).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SimColors {
    pub active: Color,
    pub active_glow: Color,
    pub transition: Color,
    pub completed: Color,
    pub inactive_opacity: f64,
}

/// Diagnostic-severity ramp (brief §3.4 `sev-*`) — drives the NE badge on canvas
/// nodes. Field names match the `sysml_span::Severity` lowercase wire values so
/// the renderer indexes `sev[entry.severity]` directly.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SevColors {
    pub info: Color,
    pub warning: Color,
    pub error: Color,
}

/// Verdict colors (brief §3.5 `vd-*`) — drives the SW verdict pill. Only pass
/// and fail carry a hue; `inconclusive` is mid-neutral (dashed pill) and `error`
/// is deliberately neutral-dark (hatched pill): *couldn't evaluate*, not *the
/// model is wrong*. Field names match the `VerdictKind` variants lowercased.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct VerdictColors {
    pub pass: Color,
    pub fail: Color,
    pub inconclusive: Color,
    pub error: Color,
}

/// Typography constants — the single Rust source of the renderer's text sizing.
/// Single-sourced here so the frontend never re-hardcodes them separately from
/// the layout engine. Steward-approved (2026-06-30).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TypographyTokens {
    /// Font size (px) for node name / header labels.
    pub label_font_size_px: f32,
    /// Font size (px) for compartment text lines.
    pub compartment_font_size_px: f32,
    /// Vertical stride (px) between compartment text baselines.
    pub compartment_line_stride_px: f32,
}

/// The full diagram color palette — the source of truth for the CSS `:root`
/// block. Named fields (not a map) so emission order is stable by construction
/// (the drift gate compares exact bytes).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Palette {
    // Canvas
    pub bg: Color,
    pub text: Color,
    pub muted: Color,
    /// Fine (24px) canvas grid line — just above the paper `bg`.
    pub grid_minor: Color,
    /// Coarse (96px) canvas grid line — a touch stronger than `grid_minor`.
    pub grid_major: Color,

    // Node categories
    pub package: CategoryColors,
    pub block: CategoryColors,
    pub action: CategoryColors,
    pub state: CategoryColors,
    pub requirement: CategoryColors,
    pub constraint: CategoryColors,
    pub interface: CategoryColors,
    pub item: CategoryColors,
    pub attribute: CategoryColors,
    pub enumeration: CategoryColors,
    pub usecase: CategoryColors,
    pub allocation: CategoryColors,
    pub flow: CategoryColors,
    pub occurrence: CategoryColors,
    pub view: CategoryColors,
    pub metadata: CategoryColors,
    pub comment: CategoryColors,
    /// Generic node fallback (`--sysml-node-*`).
    pub node_fallback: CategoryColors,

    // Standalone
    pub port: PortColors,
    pub actor_stroke: Color,
    pub link: LinkColors,
    pub lifeline_fill: Color,
    pub lifeline_stroke: Color,
    pub select: Color,
    pub edge_label_bg: Color,
    pub control_fill: Color,
    pub control_stroke: Color,
    pub compartment_text: Color,
    pub body_fill: Color,
    pub sim: SimColors,
    pub sev: SevColors,
    pub verdict: VerdictColors,
}

impl Palette {
    /// Resolve a category key (a field name, e.g. `"block"`) to its colors.
    /// Unknown keys fall back to the generic node colors.
    pub fn by_key(&self, key: &str) -> &CategoryColors {
        match key {
            "package" => &self.package,
            "block" => &self.block,
            "action" => &self.action,
            "state" => &self.state,
            "requirement" => &self.requirement,
            "constraint" => &self.constraint,
            "interface" => &self.interface,
            "item" => &self.item,
            "attribute" => &self.attribute,
            "enumeration" => &self.enumeration,
            "usecase" => &self.usecase,
            "allocation" => &self.allocation,
            "flow" => &self.flow,
            "occurrence" => &self.occurrence,
            "view" => &self.view,
            "metadata" => &self.metadata,
            "comment" => &self.comment,
            _ => &self.node_fallback,
        }
    }
}

/// Static edge-rendering rule for one `RelationshipKind` — the serialized form
/// of [`EdgeStyle`]. The renderer reads `arrowhead` + `line_style` to draw the
/// edge head (filled/hollow/open/none diamond/triangle) and dash pattern, and
/// `label` for the stereotype keyword (e.g. `"«satisfy»"`). Single-sourced from
/// [`EdgeStyle::from_relationship_kind`] so the frontend never re-lists styles.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EdgeStyleToken {
    /// `ArrowHead` variant name (`"Filled"`, `"Hollow"`, `"Open"`, `"None"`).
    pub arrowhead: String,
    /// `LineStyle` variant name (`"Solid"`, `"Dashed"`, `"Dotted"`).
    pub line_style: String,
    /// Stereotype keyword shown on the edge, if any (e.g. `"«satisfy»"`).
    pub label: Option<String>,
}

/// Top-level design-token bundle carried on the [`crate::ViewModel`].
///
/// Bundles the **static visual contract** the renderer needs: the color
/// `palette`, plus the classification tables single-sourced from Rust so the
/// frontend never re-implements them — `categories` (Bucket 2 F3), and `shapes`
/// + `edge_styles` (Bucket 3). All three are process-wide constants (the same
/// `Arc` rides every `ViewModel`); typography + geometry join later in Bucket 3.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DesignTokens {
    pub palette: Palette,
    /// `VisualKind` (serialized variant name) → palette **category key** (the
    /// `Palette` field name, e.g. `"block"`). The single Rust source of the
    /// `VisualKind → color category` mapping (Bucket 2 F3): a renderer resolves
    /// `palette[categories[node.visual_kind]]` instead of re-implementing
    /// [`DesignTokens::category_key`]. Built from `VisualKind::ALL`.
    pub categories: std::collections::BTreeMap<String, String>,
    /// `VisualKind` (serialized variant name) → [`Shape`](crate::visual_kind::Shape)
    /// variant name (e.g. `"RoundedRect"`, `"Diamond"`). The single Rust source
    /// of the `VisualKind → shape` mapping (Bucket 3): a renderer dispatches the
    /// node outline from this instead of re-listing control/shape kinds. Built
    /// from `VisualKind::ALL` + [`VisualKind::shape`](crate::visual_kind::VisualKind::shape).
    pub shapes: std::collections::BTreeMap<String, String>,
    /// `RelationshipKind` (serialized variant name) → [`EdgeStyleToken`]. The
    /// single Rust source of relationship edge styling (Bucket 3): the renderer
    /// looks up the edge's `Relationship(kind)` to get arrowhead + line style +
    /// stereotype keyword. Built from `RelationshipKind::ALL` +
    /// [`EdgeStyle::from_relationship_kind`].
    pub edge_styles: std::collections::BTreeMap<String, EdgeStyleToken>,
    /// Typography constants (label/compartment font sizes + stride). The single
    /// Rust source for text sizing, so the frontend never re-hardcodes these
    /// separately from the layout engine.
    pub typography: TypographyTokens,
}

impl DesignTokens {
    /// The shared, process-wide token instance. Tokens are a constant, so every
    /// `ViewModel` carries the same `Arc` — zero per-view cost.
    pub fn shared() -> Arc<DesignTokens> {
        static TOKENS: OnceLock<Arc<DesignTokens>> = OnceLock::new();
        TOKENS
            .get_or_init(|| Arc::new(DesignTokens::canonical()))
            .clone()
    }

    /// Build the canonical palette — the **dark "warm ink" instrument ground**
    /// (ninebar Phase 2, Wave 1). This is the *single* canonical theme: there is
    /// no `canonical_light()` peer and no theme parameter, deliberately.
    ///
    /// ## Why one theme, and why dark (2026-07-14, Phase 2 Wave 1 decision)
    ///
    /// - **Dark is the app default.** The chrome already committed to it
    ///   (`editors/simulation-app/src/styles/tokens.css` `:root` = the `n-950`
    ///   dark ground; light is the `[data-theme="light"]` override). The
    ///   design brief §14 ("read before choosing a background") *recommends*
    ///   dark-default for an eight-hour-a-day controls tool, reserving warm
    ///   paper for the report/docs — which sidesteps the AI-generated
    ///   cream-paper cliché.
    /// - **Theme stays out of the salsa cache key (F16 containment).** The
    ///   palette is serialized into `ViewModel::tokens`, and the `ViewModel` is
    ///   salsa-cached. If theme were a parameter here it would leak into that
    ///   cache key. Keeping a single canonical palette means a theme switch is
    ///   *never* a re-elaboration. A future light canvas (for the report/docs
    ///   surface) belongs in a frontend CSS-variable indirection layer, not in
    ///   this Rust table.
    /// - **The canvas owns its token set.** These values are self-contained and
    ///   do NOT reference the chrome's `--nb-*` tokens (brief §13: "the canvas
    ///   is not chrome; style it like an instrument").
    ///
    /// ## Colour model (brief §3.3 — "7 families")
    ///
    /// Shape carries element *type* (the 12-glyph `VisualKind` vocabulary);
    /// colour is a *family-level* cue for the **seven** kinds that share a
    /// canvas and must be told apart at a glance (part, action, state,
    /// requirement, constraint, interface/port, flow). Family hues are
    /// deliberately low-chroma so status overlays (amber = live, verdict, sev)
    /// sit on top without a fight. The long tail (package, item, attribute,
    /// enumeration, usecase, allocation, occurrence, view, metadata, comment,
    /// fallback) is **neutral warm** — shape + header label only, no hue.
    ///
    /// On the dark ground each hued family carries: a raised near-neutral body
    /// `fill` (just above paper), a bright `stroke` that reads at ≥3:1 vs paper
    /// (§11 contrast contract), and a tinted `header` band between the two.
    /// Text / control-glyph / container-body colours are LIGHT (the light-theme
    /// table's dark inks would vanish on this ground).
    pub fn canonical() -> DesignTokens {
        let cat = |fill: &str, stroke: &str, header: Option<&str>| CategoryColors {
            fill: Color::new(fill),
            stroke: Color::new(stroke),
            header: header.map(Color::new),
        };
        // Neutral warm long-tail category (brief §3.3): shape + label, no hue.
        let neu = || cat("oklch(29% 0.012 60)", "oklch(52% 0.014 62)", Some("oklch(34% 0.012 60)"));
        let palette = Palette {
                // Warm mid-dark instrument paper — a distinct canvas surface,
                // NOT the chrome's near-black `n-950`, so raised node cards read
                // as lighter and sunken things darker (brief §3.9 depth channel).
                bg: Color::new("oklch(24% 0.014 55)"),
                text: Color::new("oklch(95.5% 0.008 75)"),
                muted: Color::new("oklch(66% 0.016 68)"),
                // Graph-paper texture: two overlaid grids just above the paper.
                grid_minor: Color::new("oklch(27% 0.013 55)"),
                grid_major: Color::new("oklch(30% 0.014 55)"),

                // Long tail — neutral warm (no family hue).
                package: cat("oklch(28% 0.010 60)", "oklch(50% 0.012 60)", Some("oklch(33% 0.010 60)")),
                // ── The seven canvas families (brief §3.3) ──
                block: cat("oklch(29% 0.02 250)", "oklch(62% 0.10 250)", Some("oklch(35% 0.05 250)")),
                action: cat("oklch(29% 0.02 290)", "oklch(62% 0.10 290)", Some("oklch(35% 0.05 290)")),
                state: cat("oklch(30% 0.02 185)", "oklch(62% 0.09 185)", Some("oklch(35% 0.05 185)")),
                requirement: cat("oklch(29% 0.02 340)", "oklch(62% 0.10 340)", Some("oklch(35% 0.05 340)")),
                constraint: cat("oklch(30% 0.02 145)", "oklch(62% 0.09 145)", Some("oklch(35% 0.05 145)")),
                interface: cat("oklch(29% 0.02 210)", "oklch(62% 0.08 210)", Some("oklch(35% 0.045 210)")),
                flow: cat("oklch(29% 0.02 315)", "oklch(60% 0.07 315)", Some("oklch(35% 0.045 315)")),
                // Long tail — neutral warm (no family hue).
                item: neu(),
                attribute: neu(),
                enumeration: neu(),
                usecase: neu(),
                allocation: neu(),
                occurrence: neu(),
                view: neu(),
                metadata: neu(),
                comment: cat("oklch(30% 0.012 60)", "oklch(55% 0.014 62)", None),
                node_fallback: neu(),

                port: PortColors {
                    in_: Color::new("oklch(62% 0.09 155)"),
                    out: Color::new("oklch(64% 0.11 25)"),
                    inout: Color::new("oklch(60% 0.07 210)"),
                },
                actor_stroke: Color::new("oklch(80% 0.008 70)"),
                link: LinkColors {
                    ownership: Color::new("oklch(52% 0.014 60)"),
                    typing: Color::new("oklch(52% 0.014 60)"),
                    specialization: Color::new("oklch(52% 0.014 60)"),
                    req: Color::new("oklch(55% 0.07 340)"),
                    verify: Color::new("oklch(55% 0.07 340)"),
                    derive: Color::new("oklch(52% 0.06 340)"),
                    flow: Color::new("oklch(55% 0.06 315)"),
                    dependency: Color::new("oklch(52% 0.014 60)"),
                    connection: Color::new("oklch(55% 0.06 210)"),
                    succession: Color::new("oklch(52% 0.014 60)"),
                },
                lifeline_fill: Color::new("oklch(28% 0.012 230)"),
                lifeline_stroke: Color::new("oklch(58% 0.06 230)"),
                // Amber "echo" — the reserved-wedge accent (brief §3.2). Bright
                // ochre so it reads on the dark ground; "amber means now".
                select: Color::new("oklch(75% 0.13 65)"),
                edge_label_bg: Color::new("oklch(24% 0.014 55 / 0.85)"),
                // Control glyphs + container bodies are LIGHT on the dark ground.
                control_fill: Color::new("oklch(85% 0.008 70)"),
                control_stroke: Color::new("oklch(85% 0.008 70)"),
                compartment_text: Color::new("oklch(78% 0.014 70)"),
                body_fill: Color::new("oklch(27% 0.012 55)"),
                sim: SimColors {
                    // Live tick = amber (brief §4 channel budget); completed =
                    // sev-ok green; transition = a warmer amber.
                    active: Color::new("oklch(75% 0.14 65)"),
                    active_glow: Color::new("oklch(75% 0.14 65 / 0.35)"),
                    transition: Color::new("oklch(70% 0.13 65)"),
                    completed: Color::new("oklch(60% 0.11 150)"),
                    inactive_opacity: 0.45,
                },
                // Diagnostic badge ramp (brief §3.4), dark-tuned so badge fills
                // read on the dark paper (the brief's light-theme sev stops are
                // too dark to work as small solid badges here).
                sev: SevColors {
                    info: Color::new("oklch(58% 0.02 60)"),
                    warning: Color::new("oklch(68% 0.10 100)"),
                    error: Color::new("oklch(60% 0.15 25)"),
                },
                // Verdict pill (brief §3.5): pass/fail get a hue; inconclusive
                // is mid-neutral (dashed), error neutral-dark (hatched).
                verdict: VerdictColors {
                    pass: Color::new("oklch(55% 0.10 150)"),
                    fail: Color::new("oklch(52% 0.16 25)"),
                    inconclusive: Color::new("oklch(55% 0.014 60)"),
                    error: Color::new("oklch(33% 0.012 55)"),
                },
        };
        let categories = VisualKind::ALL
            .iter()
            .map(|k| (format!("{k:?}"), Self::category_key(*k).to_owned()))
            .collect();
        let shapes = VisualKind::ALL
            .iter()
            .map(|k| (format!("{k:?}"), format!("{:?}", k.shape())))
            .collect();
        let edge_styles = RelationshipKind::ALL
            .iter()
            .map(|k| {
                let style = EdgeStyle::from_relationship_kind(k);
                (
                    // MUST be the WIRE name, not `format!("{k:?}")`. The renderer
                    // indexes this map with the serialized edge kind
                    // (`edgeStyles[edge.kind.Relationship]`), and RelationshipKind
                    // serializes camelCase — a PascalCase key misses EVERY lookup
                    // and silently defaults every edge's arrowhead/line style.
                    k.wire_name().to_owned(),
                    EdgeStyleToken {
                        arrowhead: format!("{:?}", style.arrowhead),
                        line_style: format!("{:?}", style.line_style),
                        label: style.label.map(str::to_owned),
                    },
                )
            })
            .collect();
        DesignTokens {
            palette,
            categories,
            shapes,
            edge_styles,
            typography: TypographyTokens {
                label_font_size_px: 12.0,
                compartment_font_size_px: 11.0,
                compartment_line_stride_px: 16.0,
            },
        }
    }

    /// The palette **category key** (a [`Palette`] field name) for a
    /// [`VisualKind`] — the single source of the `VisualKind → color category`
    /// mapping. [`Self::category_colors`] and the serialized [`Self::categories`]
    /// map both derive from this one match (no duplicate path).
    pub fn category_key(kind: VisualKind) -> &'static str {
        match kind {
            VisualKind::Package => "package",
            VisualKind::Part | VisualKind::Connection | VisualKind::Rendering => "block",
            VisualKind::Action
            | VisualKind::Calculation
            | VisualKind::SendAction
            | VisualKind::AcceptAction => "action",
            VisualKind::State => "state",
            VisualKind::Requirement | VisualKind::Concern | VisualKind::VerificationCase => {
                "requirement"
            }
            VisualKind::Constraint => "constraint",
            VisualKind::Interface => "interface",
            VisualKind::Item => "item",
            VisualKind::Attribute => "attribute",
            VisualKind::Enumeration => "enumeration",
            VisualKind::UseCase | VisualKind::AnalysisCase => "usecase",
            VisualKind::Allocation => "allocation",
            VisualKind::Flow => "flow",
            VisualKind::Occurrence => "occurrence",
            VisualKind::View | VisualKind::Viewpoint => "view",
            VisualKind::Metadata => "metadata",
            VisualKind::Comment => "comment",
            VisualKind::Port
            | VisualKind::Actor
            | VisualKind::Lifeline
            | VisualKind::SqProxy
            | VisualKind::InitialNode
            | VisualKind::FinalNode
            | VisualKind::DecisionNode
            | VisualKind::MergeNode
            | VisualKind::ForkNode
            | VisualKind::JoinNode
            | VisualKind::TerminateNode
            | VisualKind::Generic => "node_fallback",
        }
    }

    /// The color category for a [`VisualKind`] — the entry point a renderer uses
    /// to style a node. Multiple kinds share a category (e.g. `Part`,
    /// `Connection` → `block`); control/special nodes fall back to the generic
    /// node colors and use the standalone `control_*` / `lifeline_*` fields.
    pub fn category_colors(&self, kind: VisualKind) -> &CategoryColors {
        // Single source: kind → key ([`category_key`]), key → palette field
        // ([`Palette::by_key`]). Ports / actors / control & sequence nodes /
        // plumbing key to `node_fallback` (distinctive colors live in the
        // standalone palette fields).
        self.palette.by_key(Self::category_key(kind))
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_validates_well_formed_oklch() {
        assert!(Color::validate("oklch(94% 0.04 155)").is_ok());
        assert!(Color::validate("oklch(100% 0 0)").is_ok());
        assert!(Color::validate("oklch(95% 0.005 230 / 0.85)").is_ok());
        assert!(Color::validate("oklch(22% 0.00 0)").is_ok());
    }

    #[test]
    fn color_rejects_malformed() {
        assert!(Color::validate("rgb(1,2,3)").is_err()); // wrong function
        assert!(Color::validate("oklch(94 0.04 155)").is_err()); // L missing %
        assert!(Color::validate("oklch(94% 0.04)").is_err()); // too few components
        assert!(Color::validate("oklch(94% x 155)").is_err()); // non-numeric chroma
    }

    #[test]
    #[should_panic(expected = "invalid design-token color")]
    fn color_new_panics_on_typo() {
        let _ = Color::new("oklch(94% 0.04)"); // fail-hard at construction
    }

    #[test]
    fn canonical_tokens_build() {
        // Exercises every Color::new in the table — a typo panics here.
        let t = DesignTokens::canonical();
        // Dark warm-ink ground (Phase 2 Wave 1): part/block = fam-part (hue 250).
        assert_eq!(t.palette.block.fill.as_str(), "oklch(29% 0.02 250)");
        assert!(t.palette.comment.header.is_none());
        // W3 overlay layering: sev ramp + verdict pills exist and follow the
        // brief — only pass/fail carry a hue (error is deliberately neutral).
        assert_eq!(t.palette.sev.error.as_str(), "oklch(60% 0.15 25)");
        assert_eq!(t.palette.verdict.fail.as_str(), "oklch(52% 0.16 25)");
        assert_eq!(t.palette.verdict.error.as_str(), "oklch(33% 0.012 55)");
    }

    #[test]
    fn category_colors_resolve_shared_categories() {
        let t = DesignTokens::canonical();
        // Part and Connection share the `block` category.
        assert_eq!(
            t.category_colors(VisualKind::Part),
            t.category_colors(VisualKind::Connection)
        );
        assert_eq!(
            t.category_colors(VisualKind::Part).fill.as_str(),
            "oklch(29% 0.02 250)"
        );
        // Requirement family shares the `req` category.
        assert_eq!(
            t.category_colors(VisualKind::Requirement),
            t.category_colors(VisualKind::VerificationCase)
        );
    }

    #[test]
    fn shared_returns_same_arc() {
        assert!(Arc::ptr_eq(&DesignTokens::shared(), &DesignTokens::shared()));
    }

    #[test]
    fn categories_map_covers_all_visual_kinds_and_matches_colors() {
        // F3: the serialized `categories` map is the single source a renderer
        // uses for VisualKind→color, so it must cover every kind and agree with
        // `category_colors` byte-for-byte.
        let t = DesignTokens::canonical();
        for k in VisualKind::ALL.iter().copied() {
            let name = format!("{k:?}");
            let key = t
                .categories
                .get(&name)
                .unwrap_or_else(|| panic!("every VisualKind must have a category entry: {name}"));
            assert_eq!(key.as_str(), DesignTokens::category_key(k));
            assert_eq!(t.palette.by_key(key), t.category_colors(k), "kind {name}");
        }
    }

    #[test]
    fn shapes_map_covers_all_visual_kinds_and_matches_shape() {
        // Bucket 3: the serialized `shapes` map is the single source a renderer
        // uses for VisualKind→shape, so it must cover every kind and agree with
        // `VisualKind::shape()`.
        let t = DesignTokens::canonical();
        assert_eq!(t.shapes.len(), VisualKind::ALL.len());
        for k in VisualKind::ALL.iter().copied() {
            let name = format!("{k:?}");
            let shape = t
                .shapes
                .get(&name)
                .unwrap_or_else(|| panic!("every VisualKind must have a shape entry: {name}"));
            assert_eq!(shape.as_str(), format!("{:?}", k.shape()));
        }
        // Spot-check a few so a wrong mapping (not just a missing key) is caught.
        assert_eq!(t.shapes["State"], "RoundedRect");
        assert_eq!(t.shapes["DecisionNode"], "Diamond");
        assert_eq!(t.shapes["InitialNode"], "FilledCircle");
        assert_eq!(t.shapes["UseCase"], "Ellipse");
    }

    #[test]
    fn edge_styles_map_covers_all_relationship_kinds_and_matches_style() {
        // Bucket 3: single source for relationship edge styling.
        let t = DesignTokens::canonical();
        assert_eq!(t.edge_styles.len(), RelationshipKind::ALL.len());
        for k in RelationshipKind::ALL.iter() {
            // Keyed by the WIRE name — the renderer indexes this map with the
            // serialized (camelCase) kind, so PascalCase keys would never resolve.
            let name = k.wire_name();
            let tok = t
                .edge_styles
                .get(name)
                .unwrap_or_else(|| panic!("every RelationshipKind must have an edge-style: {name}"));
            let style = EdgeStyle::from_relationship_kind(k);
            assert_eq!(tok.arrowhead, format!("{:?}", style.arrowhead));
            assert_eq!(tok.line_style, format!("{:?}", style.line_style));
            assert_eq!(tok.label.as_deref(), style.label);
        }
        // Spot-check §F-8 composite vs shared + a stereotype keyword. Keys are
        // the camelCase WIRE names the renderer actually looks up.
        assert_eq!(t.edge_styles["composition"].arrowhead, "Filled");
        assert_eq!(t.edge_styles["specialize"].arrowhead, "Hollow");
        assert_eq!(t.edge_styles["satisfy"].label.as_deref(), Some("«satisfy»"));
        assert_eq!(t.edge_styles["subsetting"].line_style, "Dotted");
        // R5 (Table 11): symmetric connectors are a plain line, NO arrowhead.
        assert_eq!(t.edge_styles["connection"].arrowhead, "None");
        assert_eq!(t.edge_styles["connection"].line_style, "Solid");
        assert_eq!(t.edge_styles["binding"].arrowhead, "None");
        // Multi-word variants must be camelCase, NOT PascalCase — the exact
        // drift that left this table unreachable from the renderer.
        assert!(t.edge_styles.contains_key("featureMembership"));
        assert!(t.edge_styles.contains_key("typeOf"));
        assert!(
            !t.edge_styles.contains_key("Connection"),
            "PascalCase keys must not reappear — the renderer looks up camelCase",
        );
    }

    #[test]
    fn typography_tokens_match_renderer_constants() {
        let t = DesignTokens::canonical();
        // These values must stay in sync with layout.ts FONT/LINE_H constants.
        assert_eq!(t.typography.label_font_size_px, 12.0);
        assert_eq!(t.typography.compartment_font_size_px, 11.0);
        assert_eq!(t.typography.compartment_line_stride_px, 16.0);
    }
}
