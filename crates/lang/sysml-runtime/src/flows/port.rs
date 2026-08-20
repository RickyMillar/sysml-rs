//! Port instance representation for typed flow routing.
//!
//! Bridges sysml-core's port type model (PortDefinition, PortUsage,
//! ConjugatedPortDefinition, FeatureDirectionKind) into the runtime's
//! execution engine. PortInstanceIR is the runtime-side IR for ports,
//! populated from ModelGraph during the compile step.
//!
//! ## Architecture
//!
//! ```text
//! ModelGraph (sysml-core)
//!     │  compile_ports()
//!     ▼
//! PortRegistry (HashMap<"owner.port", PortInstanceIR>)
//!     │  resolve_path(), get(), get_mut()
//!     ▼
//! FlowRouter (route_pending with optional registry)
//! ```

#![allow(clippy::indexing_slicing)]
use std::collections::HashMap;
use std::fmt;

use sysml_core::Value;
use sysml_span::Diagnostic;

/// Direction of data flow through a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortDirection {
    In,
    Out,
    InOut,
    Undirected,
}

impl PortDirection {
    /// Check if two directions are compatible for a flow connection.
    /// Out→In and InOut→anything are valid. Conjugation reverses direction.
    pub fn is_compatible_with(&self, other: &PortDirection) -> bool {
        match (self, other) {
            // Standard out-to-in
            (PortDirection::Out, PortDirection::In) => true,
            // InOut connects to anything
            (PortDirection::InOut, _) | (_, PortDirection::InOut) => true,
            // Undirected connects to anything
            (PortDirection::Undirected, _) | (_, PortDirection::Undirected) => true,
            // In-to-out (reverse, requires conjugation)
            (PortDirection::In, PortDirection::Out) => true,
            // Same direction is a warning, not an error
            _ => false,
        }
    }

    /// Return the conjugated (reversed) direction.
    pub fn conjugate(&self) -> PortDirection {
        match self {
            PortDirection::In => PortDirection::Out,
            PortDirection::Out => PortDirection::In,
            PortDirection::InOut => PortDirection::InOut,
            PortDirection::Undirected => PortDirection::Undirected,
        }
    }
}

impl fmt::Display for PortDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PortDirection::In => write!(f, "in"),
            PortDirection::Out => write!(f, "out"),
            PortDirection::InOut => write!(f, "inout"),
            PortDirection::Undirected => write!(f, "undirected"),
        }
    }
}

/// A typed feature on a port instance (e.g., `flowRate`, `temperature`).
#[derive(Debug, Clone)]
pub struct PortFeature {
    /// Feature name (e.g., "flowRate")
    pub name: String,
    /// Direction of this feature within the port
    pub direction: PortDirection,
    /// Type constraint (e.g., "Real", "Boolean")
    pub type_name: Option<String>,
    /// Current value (updated during simulation)
    pub value: Value,
}

/// Runtime representation of a port instance on a specific part.
///
/// Populated from ModelGraph during `compile_ports()` by reading
/// sysml-core's PortUsage → PortDefinition typing chain.
#[derive(Debug, Clone)]
pub struct PortInstanceIR {
    /// Owning part instance name (e.g., "waterTank")
    pub owner: String,
    /// Port name (e.g., "waterOut")
    pub name: String,
    /// PortDefinition name (e.g., "WaterPort")
    pub definition: Option<String>,
    /// Typed features with current values
    pub features: HashMap<String, PortFeature>,
    /// Flow direction of the port
    pub direction: PortDirection,
    /// Whether this is a conjugated port (~PortDef)
    pub is_conjugated: bool,
    /// Array multiplicity (None = scalar, Some(n) = [n])
    pub multiplicity: Option<usize>,
}

impl PortInstanceIR {
    /// Create a new port instance with minimal required fields.
    pub fn new(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            name: name.into(),
            definition: None,
            features: HashMap::new(),
            direction: PortDirection::Undirected,
            is_conjugated: false,
            multiplicity: None,
        }
    }

    /// Builder: set port definition name.
    pub fn with_definition(mut self, def: impl Into<String>) -> Self {
        self.definition = Some(def.into());
        self
    }

    /// Builder: set port direction.
    pub fn with_direction(mut self, dir: PortDirection) -> Self {
        self.direction = dir;
        self
    }

    /// Builder: mark as conjugated.
    pub fn conjugated(mut self) -> Self {
        self.is_conjugated = true;
        self.direction = self.direction.conjugate();
        self
    }

    /// Builder: set multiplicity.
    pub fn with_multiplicity(mut self, n: usize) -> Self {
        self.multiplicity = Some(n);
        self
    }

    /// Add a feature to this port.
    pub fn add_feature(&mut self, feature: PortFeature) {
        self.features.insert(feature.name.clone(), feature);
    }

    /// Get the effective direction (conjugated if needed).
    pub fn effective_direction(&self) -> PortDirection {
        if self.is_conjugated {
            self.direction.conjugate()
        } else {
            self.direction
        }
    }

    /// Get a feature value by name.
    pub fn get_feature_value(&self, name: &str) -> Option<&Value> {
        self.features.get(name).map(|f| &f.value)
    }

    /// Set a feature value by name. Returns true if the feature exists.
    pub fn set_feature_value(&mut self, name: &str, value: Value) -> bool {
        if let Some(feature) = self.features.get_mut(name) {
            feature.value = value;
            true
        } else {
            false
        }
    }

    /// The routing key for this port: "owner.name"
    pub fn key(&self) -> String {
        format!("{}.{}", self.owner, self.name)
    }

    /// Check if this port is structurally compatible with another port.
    ///
    /// A source port is compatible with a target port if every feature
    /// (attribute name + direction) of the source has a matching feature
    /// in the target. This implements structural subtyping per SysML v2:
    /// the target may have *more* features than the source, but every
    /// source feature must be present in the target with a compatible
    /// direction and (if specified on both) matching type.
    pub fn is_compatible_with(&self, target: &PortInstanceIR) -> bool {
        for (name, src_feat) in &self.features {
            match target.features.get(name) {
                Some(tgt_feat) => {
                    // Check direction compatibility at the feature level.
                    // out->in and in->out are compatible (conjugation).
                    // Same direction is also compatible (both parts may
                    // expose the same direction when conjugation is on the port).
                    if !feature_directions_compatible(&src_feat.direction, &tgt_feat.direction) {
                        return false;
                    }
                    // Check type compatibility when both features specify a type.
                    if let (Some(src_type), Some(tgt_type)) =
                        (&src_feat.type_name, &tgt_feat.type_name)
                    {
                        if src_type != tgt_type {
                            return false;
                        }
                    }
                }
                None => return false, // target missing required feature
            }
        }
        true
    }
}

/// Check if two feature-level directions are compatible for structural subtyping.
///
/// Feature directions are compatible when they form a conjugate pair (out/in)
/// or when they match exactly. InOut and Undirected are compatible with anything.
fn feature_directions_compatible(src: &PortDirection, tgt: &PortDirection) -> bool {
    match (src, tgt) {
        // Conjugate pairs
        (PortDirection::Out, PortDirection::In) | (PortDirection::In, PortDirection::Out) => true,
        // InOut is universally compatible
        (PortDirection::InOut, _) | (_, PortDirection::InOut) => true,
        // Undirected is universally compatible
        (PortDirection::Undirected, _) | (_, PortDirection::Undirected) => true,
        // Same direction is compatible
        (a, b) => a == b,
    }
}

/// Error type for path resolution failures.
#[derive(Debug, Clone, PartialEq)]
pub enum PathError {
    /// No port found for the given owner.port key
    UnknownPort(String),
    /// Port exists but the requested feature doesn't
    UnknownFeature { port_key: String, feature: String },
    /// Path has invalid number of segments (not 2 or 3)
    InvalidDepth { path: String, segments: usize },
    /// Path is empty
    EmptyPath,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::UnknownPort(key) => write!(f, "no port found for '{key}'"),
            PathError::UnknownFeature { port_key, feature } => {
                write!(f, "port '{port_key}' has no feature '{feature}'")
            }
            PathError::InvalidDepth { path, segments } => {
                write!(f, "path '{path}' has {segments} segments (expected 2 or 3)")
            }
            PathError::EmptyPath => write!(f, "empty path"),
        }
    }
}

/// Result of resolving a hierarchical port path.
#[derive(Debug, Clone)]
pub enum ResolvedPath<'a> {
    /// 2-segment: "owner.port" → full port instance
    Port(&'a PortInstanceIR),
    /// 3-segment: "owner.port.feature" → specific feature value
    Feature {
        port: &'a PortInstanceIR,
        feature: &'a PortFeature,
    },
}

/// Registry of all port instances in a model, keyed by "owner.port".
///
/// This is an optional companion to FlowRouter (ADR-2). When provided,
/// the router validates port types and binds feature values on delivery.
/// When absent, the router falls back to string-key routing.
#[derive(Debug, Clone, Default)]
pub struct PortRegistry {
    ports: HashMap<String, PortInstanceIR>,
}

impl PortRegistry {
    pub fn new() -> Self {
        Self {
            ports: HashMap::new(),
        }
    }

    /// Register a port instance. Key is automatically derived from owner.name.
    pub fn register(&mut self, port: PortInstanceIR) {
        let key = port.key();
        self.ports.insert(key, port);
    }

    /// Get a port by key ("owner.port").
    pub fn get(&self, key: &str) -> Option<&PortInstanceIR> {
        self.ports.get(key)
    }

    /// Get a mutable port by key.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut PortInstanceIR> {
        self.ports.get_mut(key)
    }

    /// Check if a port exists.
    pub fn contains(&self, key: &str) -> bool {
        self.ports.contains_key(key)
    }

    /// Number of registered ports.
    pub fn len(&self) -> usize {
        self.ports.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.ports.is_empty()
    }

    /// Iterate over all ports.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &PortInstanceIR)> {
        self.ports.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Find a port by its port name (the last segment of the key).
    ///
    /// This handles the case where flow endpoints use instance paths
    /// (e.g., `"breaker.phaseIn"`) but the registry uses definition-owner
    /// keys (e.g., `"DualPoleBreaker.phaseIn"`). Returns the first match.
    pub fn find_by_port_name(&self, port_name: &str) -> Option<&PortInstanceIR> {
        self.ports.values().find(|p| p.name == port_name)
    }

    /// Resolve a hierarchical path to a port or feature.
    ///
    /// - 2 segments: `"tank.waterOut"` → `ResolvedPath::Port`
    /// - 3 segments: `"tank.waterOut.flowRate"` → `ResolvedPath::Feature`
    pub fn resolve_path(&self, path: &str) -> Result<ResolvedPath<'_>, PathError> {
        if path.is_empty() {
            return Err(PathError::EmptyPath);
        }

        let segments: Vec<&str> = path.split('.').collect();
        match segments.len() {
            0 | 1 => Err(PathError::InvalidDepth {
                path: path.to_owned(),
                segments: segments.len(),
            }),
            2 => {
                // "owner.port" → look up port
                let key = path;
                self.ports
                    .get(key)
                    .map(ResolvedPath::Port)
                    .ok_or_else(|| PathError::UnknownPort(key.to_owned()))
            }
            3 => {
                // "owner.port.feature" → look up port then feature
                let port_key = format!("{}.{}", segments[0], segments[1]);
                let feature_name = segments[2];

                let port = self
                    .ports
                    .get(&port_key)
                    .ok_or_else(|| PathError::UnknownPort(port_key.clone()))?;

                let feature =
                    port.features
                        .get(feature_name)
                        .ok_or_else(|| PathError::UnknownFeature {
                            port_key,
                            feature: feature_name.to_owned(),
                        })?;

                Ok(ResolvedPath::Feature { port, feature })
            }
            n => Err(PathError::InvalidDepth {
                path: path.to_owned(),
                segments: n,
            }),
        }
    }

    /// Find all ports owned by a specific part.
    pub fn ports_for_owner(&self, owner: &str) -> Vec<&PortInstanceIR> {
        self.ports.values().filter(|p| p.owner == owner).collect()
    }

    /// Find all ports with a specific definition type.
    pub fn ports_with_definition(&self, def_name: &str) -> Vec<&PortInstanceIR> {
        self.ports
            .values()
            .filter(|p| p.definition.as_deref() == Some(def_name))
            .collect()
    }

    /// Validate that all flow connections have structurally compatible port types.
    ///
    /// Returns diagnostics for connections where the source port's features
    /// are not a structural subset of the target port's features.
    pub fn validate_connections(
        &self,
        links: &crate::links::LinkGraph,
        graph: &sysml_core::ModelGraph,
    ) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        // RSC-3.5e.5 W2: the FlowUsage subset of the classified link graph
        // (connector-only links excluded, as the legacy flow input did). The
        // flow display id is recovered from the graph (byte-identical to the
        // former `FlowConnectionIR::id`).
        for link in links
            .iter()
            .filter(|l| l.kind == crate::links::LinkSourceKind::FlowUsage)
        {
            let source_key = link.source.key();
            let target_key = link.target.key();

            if let (Some(src), Some(tgt)) = (self.get(&source_key), self.get(&target_key)) {
                if !src.is_compatible_with(tgt) {
                    diags.push(
                        Diagnostic::warning(format!(
                            "flow '{}': source port '{}' is not structurally compatible \
                             with target port '{}'",
                            link.display_label(graph),
                            source_key,
                            target_key
                        ))
                        .with_code("FL016"),
                    );
                }
            }
        }
        diags
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// RSC-3.5e.5 W2: a one-link FlowUsage `LinkGraph` (the input shape
    /// `validate_connections` now consumes). FL016 is keyed on endpoint
    /// compatibility, not the flow label, so the element id is synthetic.
    fn one_flow_lg(
        src_owner: &str,
        src_port: &str,
        tgt_owner: &str,
        tgt_port: &str,
    ) -> crate::links::LinkGraph {
        use crate::links::{LinkClass, LinkEndpoint, LinkGraph, LinkIR, LinkSourceKind};
        use sysml_core::physics::classify::ClassificationConfidence;
        let mut lg = LinkGraph::new();
        lg.intern(LinkIR {
            element_id: sysml_core::ElementId::from_string(format!("flow:{src_owner}.{src_port}")),
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: src_owner.into(),
                port: src_port.into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: tgt_owner.into(),
                port: tgt_port.into(),
                resolved_registry_key: None,
            },
            class: LinkClass::MessageChannel,
            class_confidence: ClassificationConfidence::Declared,
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        });
        lg
    }

    fn make_water_port(owner: &str, name: &str, direction: PortDirection) -> PortInstanceIR {
        let mut port = PortInstanceIR::new(owner, name)
            .with_definition("WaterPort")
            .with_direction(direction);
        port.add_feature(PortFeature {
            name: "flowRate".into(),
            direction: PortDirection::Out,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        port.add_feature(PortFeature {
            name: "temperature".into(),
            direction: PortDirection::Out,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        port
    }

    #[test]
    fn port_instance_key() {
        let port = PortInstanceIR::new("tank", "waterOut");
        assert_eq!(port.key(), "tank.waterOut");
    }

    #[test]
    fn port_direction_compatibility() {
        assert!(PortDirection::Out.is_compatible_with(&PortDirection::In));
        assert!(PortDirection::InOut.is_compatible_with(&PortDirection::In));
        assert!(PortDirection::InOut.is_compatible_with(&PortDirection::Out));
        assert!(!PortDirection::Out.is_compatible_with(&PortDirection::Out));
        assert!(!PortDirection::In.is_compatible_with(&PortDirection::In));
        assert!(PortDirection::Undirected.is_compatible_with(&PortDirection::Out));
    }

    #[test]
    fn port_direction_conjugation() {
        assert_eq!(PortDirection::In.conjugate(), PortDirection::Out);
        assert_eq!(PortDirection::Out.conjugate(), PortDirection::In);
        assert_eq!(PortDirection::InOut.conjugate(), PortDirection::InOut);
    }

    #[test]
    fn port_feature_values() {
        let mut port = make_water_port("tank", "waterOut", PortDirection::Out);
        assert_eq!(port.get_feature_value("flowRate"), Some(&Value::Float(0.0)));
        assert!(port.set_feature_value("flowRate", Value::Float(1.5)));
        assert_eq!(port.get_feature_value("flowRate"), Some(&Value::Float(1.5)));
        assert!(!port.set_feature_value("nonexistent", Value::Float(0.0)));
    }

    #[test]
    fn conjugated_port_reverses_direction() {
        let port = PortInstanceIR::new("brewer", "waterIn")
            .with_direction(PortDirection::In)
            .conjugated();
        assert!(port.is_conjugated);
        // Direction stored as conjugated
        assert_eq!(port.direction, PortDirection::Out);
        // effective_direction re-conjugates back
        assert_eq!(port.effective_direction(), PortDirection::In);
    }

    #[test]
    fn registry_crud() {
        let mut reg = PortRegistry::new();
        assert!(reg.is_empty());

        reg.register(make_water_port("tank", "waterOut", PortDirection::Out));
        reg.register(make_water_port("brewer", "waterIn", PortDirection::In));

        assert_eq!(reg.len(), 2);
        assert!(reg.contains("tank.waterOut"));
        assert!(reg.contains("brewer.waterIn"));
        assert!(!reg.contains("tank.steamOut"));

        let port = reg.get("tank.waterOut").unwrap();
        assert_eq!(port.definition.as_deref(), Some("WaterPort"));
        assert_eq!(port.direction, PortDirection::Out);
    }

    #[test]
    fn registry_resolve_2_segment() {
        let mut reg = PortRegistry::new();
        reg.register(make_water_port("tank", "waterOut", PortDirection::Out));

        let resolved = reg.resolve_path("tank.waterOut").unwrap();
        match resolved {
            ResolvedPath::Port(p) => {
                assert_eq!(p.owner, "tank");
                assert_eq!(p.name, "waterOut");
            }
            _ => panic!("expected Port variant"),
        }
    }

    #[test]
    fn registry_resolve_3_segment() {
        let mut reg = PortRegistry::new();
        reg.register(make_water_port("tank", "waterOut", PortDirection::Out));

        let resolved = reg.resolve_path("tank.waterOut.flowRate").unwrap();
        match resolved {
            ResolvedPath::Feature { port, feature } => {
                assert_eq!(port.name, "waterOut");
                assert_eq!(feature.name, "flowRate");
                assert_eq!(feature.value, Value::Float(0.0));
            }
            _ => panic!("expected Feature variant"),
        }
    }

    #[test]
    fn registry_resolve_errors() {
        let mut reg = PortRegistry::new();
        reg.register(make_water_port("tank", "waterOut", PortDirection::Out));

        // Unknown port
        assert_eq!(
            reg.resolve_path("tank.steamOut").unwrap_err(),
            PathError::UnknownPort("tank.steamOut".into())
        );

        // Unknown feature
        assert_eq!(
            reg.resolve_path("tank.waterOut.pressure").unwrap_err(),
            PathError::UnknownFeature {
                port_key: "tank.waterOut".into(),
                feature: "pressure".into()
            }
        );

        // Invalid depth
        assert!(matches!(
            reg.resolve_path("tank").unwrap_err(),
            PathError::InvalidDepth { segments: 1, .. }
        ));
        assert!(matches!(
            reg.resolve_path("a.b.c.d").unwrap_err(),
            PathError::InvalidDepth { segments: 4, .. }
        ));

        // Empty
        assert_eq!(reg.resolve_path("").unwrap_err(), PathError::EmptyPath);
    }

    #[test]
    fn registry_query_by_owner() {
        let mut reg = PortRegistry::new();
        reg.register(make_water_port("tank", "waterOut", PortDirection::Out));
        reg.register(make_water_port("tank", "steamOut", PortDirection::Out));
        reg.register(make_water_port("brewer", "waterIn", PortDirection::In));

        let tank_ports = reg.ports_for_owner("tank");
        assert_eq!(tank_ports.len(), 2);

        let brewer_ports = reg.ports_for_owner("brewer");
        assert_eq!(brewer_ports.len(), 1);
    }

    #[test]
    fn registry_query_by_definition() {
        let mut reg = PortRegistry::new();
        reg.register(make_water_port("tank", "waterOut", PortDirection::Out));
        reg.register(
            PortInstanceIR::new("tank", "powerIn")
                .with_definition("PowerPort")
                .with_direction(PortDirection::In),
        );

        let water_ports = reg.ports_with_definition("WaterPort");
        assert_eq!(water_ports.len(), 1);
        assert_eq!(water_ports[0].name, "waterOut");
    }

    #[test]
    fn port_multiplicity() {
        let port = PortInstanceIR::new("molex", "circuitOut")
            .with_definition("CircuitOutputPort")
            .with_multiplicity(4);
        assert_eq!(port.multiplicity, Some(4));
    }

    // -------------------------------------------------------------------
    // Structural subtype compatibility tests (Feature 3.4)
    // -------------------------------------------------------------------

    /// Helper: build a port with arbitrary features.
    fn port_with_features(
        owner: &str,
        name: &str,
        features: Vec<(&str, PortDirection, Option<&str>)>,
    ) -> PortInstanceIR {
        let mut port = PortInstanceIR::new(owner, name);
        for (feat_name, dir, ty) in features {
            port.add_feature(PortFeature {
                name: feat_name.into(),
                direction: dir,
                type_name: ty.map(String::from),
                value: Value::Null,
            });
        }
        port
    }

    #[test]
    fn compatible_same_features() {
        let src = port_with_features(
            "sensor",
            "out",
            vec![
                ("temperature", PortDirection::Out, Some("Real")),
                ("timestamp", PortDirection::Out, Some("Real")),
            ],
        );
        let tgt = port_with_features(
            "controller",
            "in",
            vec![
                ("temperature", PortDirection::In, Some("Real")),
                ("timestamp", PortDirection::In, Some("Real")),
            ],
        );
        assert!(src.is_compatible_with(&tgt));
    }

    #[test]
    fn incompatible_missing_feature() {
        let src = port_with_features(
            "sensor",
            "out",
            vec![
                ("temperature", PortDirection::Out, None),
                ("pressure", PortDirection::Out, None),
            ],
        );
        let tgt = port_with_features(
            "controller",
            "in",
            vec![("temperature", PortDirection::In, None)],
        );
        // target is missing "pressure" that source provides
        assert!(!src.is_compatible_with(&tgt));
    }

    #[test]
    fn compatible_extra_target_features() {
        // Target has MORE features than source — OK (structural subtyping)
        let src = port_with_features(
            "sensor",
            "out",
            vec![("temperature", PortDirection::Out, None)],
        );
        let tgt = port_with_features(
            "controller",
            "in",
            vec![
                ("temperature", PortDirection::In, None),
                ("humidity", PortDirection::In, None),
            ],
        );
        assert!(src.is_compatible_with(&tgt));
    }

    #[test]
    fn incompatible_type_mismatch() {
        let src = port_with_features(
            "sensor",
            "out",
            vec![("value", PortDirection::Out, Some("Real"))],
        );
        let tgt = port_with_features(
            "controller",
            "in",
            vec![("value", PortDirection::In, Some("Boolean"))],
        );
        assert!(!src.is_compatible_with(&tgt));
    }

    #[test]
    fn compatible_when_only_source_has_type() {
        // Source specifies a type but target doesn't — compatible
        // (target is unconstrained on type)
        let src = port_with_features(
            "sensor",
            "out",
            vec![("value", PortDirection::Out, Some("Real"))],
        );
        let tgt = port_with_features("controller", "in", vec![("value", PortDirection::In, None)]);
        assert!(src.is_compatible_with(&tgt));
    }

    #[test]
    fn compatible_same_direction_features() {
        // Same direction at feature level is valid
        let src = port_with_features("a", "p", vec![("x", PortDirection::Out, None)]);
        let tgt = port_with_features("b", "q", vec![("x", PortDirection::Out, None)]);
        assert!(src.is_compatible_with(&tgt));
    }

    #[test]
    fn compatible_inout_feature() {
        // InOut feature is compatible with any direction
        let src = port_with_features("a", "p", vec![("x", PortDirection::InOut, None)]);
        let tgt = port_with_features("b", "q", vec![("x", PortDirection::Out, None)]);
        assert!(src.is_compatible_with(&tgt));
    }

    #[test]
    fn compatible_empty_source() {
        // Source with no features is trivially compatible with anything
        let src = PortInstanceIR::new("a", "p");
        let tgt = port_with_features("b", "q", vec![("x", PortDirection::In, None)]);
        assert!(src.is_compatible_with(&tgt));
    }

    #[test]
    fn validate_connections_detects_incompatible() {
        let mut reg = PortRegistry::new();
        reg.register(port_with_features(
            "sensor",
            "out",
            vec![
                ("temperature", PortDirection::Out, Some("Real")),
                ("pressure", PortDirection::Out, Some("Real")),
            ],
        ));
        // Target missing "pressure"
        reg.register(port_with_features(
            "display",
            "in",
            vec![("temperature", PortDirection::In, Some("Real"))],
        ));

        let links = one_flow_lg("sensor", "out", "display", "in");
        let diags = reg.validate_connections(&links, &sysml_core::ModelGraph::new());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code.as_deref(), Some("FL016"));
        assert!(diags[0].message.contains("sensor.out"));
        assert!(diags[0].message.contains("display.in"));
    }

    #[test]
    fn validate_connections_passes_compatible() {
        let mut reg = PortRegistry::new();
        reg.register(port_with_features(
            "sensor",
            "out",
            vec![("temperature", PortDirection::Out, Some("Real"))],
        ));
        reg.register(port_with_features(
            "display",
            "in",
            vec![
                ("temperature", PortDirection::In, Some("Real")),
                ("extra", PortDirection::In, None),
            ],
        ));

        let links = one_flow_lg("sensor", "out", "display", "in");
        let diags = reg.validate_connections(&links, &sysml_core::ModelGraph::new());
        assert!(
            diags.is_empty(),
            "expected no FL016 diagnostics, got: {:?}",
            diags
        );
    }

    #[test]
    fn validate_connections_skips_unknown_ports() {
        // If either port isn't in the registry, validate_connections silently skips
        let reg = PortRegistry::new();
        let links = one_flow_lg("ghost", "out", "phantom", "in");
        let diags = reg.validate_connections(&links, &sysml_core::ModelGraph::new());
        assert!(diags.is_empty());
    }
}
