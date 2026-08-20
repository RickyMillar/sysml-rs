//! Physics domain registry and builtin domain table.
//!
//! Maps ISQ dimension vectors to physical simulation domains (electrical, thermal,
//! hydraulic, mechanical) and classifies variables as effort or flow quantities.
//!
//! The builtin domain table encodes mathematical facts from ISO 80000 — the dimension
//! vectors for voltage, current, temperature, etc. are universal constants.

use std::collections::HashMap;

use crate::{ElementKind, ModelGraph};

use super::dimension::{extract_dimension_from_unit_element, DimensionVector, TIME_DIM};

/// A physical simulation domain with effort/flow variable classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicsDomain {
    /// Domain name (e.g., "electrical", "thermal").
    pub name: &'static str,
    /// Dimension vector for the effort variable (voltage, temperature, pressure, velocity).
    pub effort_dimensions: DimensionVector,
    /// Dimension vector for the flow variable (current, heat flow, mass flow, force).
    pub flow_dimensions: DimensionVector,
    /// Conservation law governing this domain.
    pub conservation: ConservationLaw,
}

/// Conservation law type for a physics domain.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConservationLaw {
    /// Sum of flow variables at a junction = 0 (KCL, mass conservation).
    FlowConservation,
    /// Energy balance: Q_in - Q_out = C * d(effort)/dt.
    EnergyBalance,
    /// No conservation -- values are copied directionally.
    SignalRouting,
}

/// Role of a variable within its physics domain.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VariableRole {
    /// Shared across connections (voltage, temperature, pressure, velocity).
    Effort,
    /// Conserved through connections (current, heat flow, mass flow, force).
    Flow,
    /// Neither effort nor flow (resistance, capacitance, etc.).
    Parameter,
    /// Energy storage element (capacitance, inductance, thermal mass).
    Storage,
    /// Continuous state variable tracked by ODE.
    StateVar,
}

/// Bond graph element role — the complete set of generalized variable types.
///
/// Every physically meaningful quantity in a domain can be classified into
/// exactly one of 12 roles, determined by dimension arithmetic against
/// the domain's effort (`e`), flow (`f`), and time (`t`) dimensions.
///
/// This enum is exhaustive by design — all `match` arms must handle every
/// variant, enforced by the compiler. No `_ =>` catch-alls allowed.
///
/// Reference: Karnopp, Margolis & Rosenberg, "System Dynamics" (bond graph theory).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BondGraphRole {
    // --- Primary variables (on bonds) ---
    /// Shared across connections (voltage, temperature, pressure, velocity).
    /// `dim = dim(effort)`
    Effort,
    /// Conserved through connections (current, heat flow, mass flow, force).
    /// `dim = dim(flow)`
    Flow,

    // --- Integrated variables (state variables) ---
    /// Generalized displacement: `q = ∫f dt` (charge, position, volume, angle).
    /// `dim = dim(flow) + dim(time)`
    Displacement,
    /// Generalized momentum: `p = ∫e dt` (flux linkage, impulse, pressure·time).
    /// `dim = dim(effort) + dim(time)`
    Momentum,

    // --- One-port elements ---
    /// Dissipator: `e = R·f` (resistor, damper, thermal resistance, pipe friction).
    /// `dim = dim(effort) - dim(flow)`
    Resistance,
    /// Inverse dissipator: `f = G·e` (conductance, admittance).
    /// `dim = dim(flow) - dim(effort)`
    Conductance,
    /// Effort storage: `e = q/C = (1/C)·∫f dt` (capacitor, spring, thermal mass).
    /// `dim = dim(flow) - dim(effort) + dim(time)`
    Capacitance,
    /// Flow storage: `f = p/I = (1/I)·∫e dt` (inductor, mass, fluid inertia).
    /// `dim = dim(effort) - dim(flow) + dim(time)`
    Inductance,

    // --- Derived quantities ---
    /// Instantaneous power: `P = e·f`.
    /// `dim = dim(effort) + dim(flow)`
    Power,
    /// Stored/transferred energy: `E = ∫P dt = ∫e·f dt`.
    /// `dim = dim(effort) + dim(flow) + dim(time)`
    Energy,
    /// Time derivative of effort: `de/dt` (acceleration, voltage slew rate).
    /// `dim = dim(effort) - dim(time)`
    EffortRate,
    /// Time derivative of flow: `df/dt` (jerk, current slew rate).
    /// `dim = dim(flow) - dim(time)`
    FlowRate,

    // --- Classification boundaries ---
    /// All exponents zero (ratios, angles, efficiency).
    Dimensionless,
    /// Does not match any known role in this domain.
    Unclassified,
}

impl BondGraphRole {
    /// The dimension formula as `(effort_coeff, flow_coeff, time_coeff)`.
    ///
    /// The expected dimension is: `e*a + f*b + t*c` where (a, b, c) = this tuple.
    /// Returns `None` for `Dimensionless` and `Unclassified` (no formula).
    pub const fn dimension_formula(self) -> Option<(i8, i8, i8)> {
        match self {
            Self::Effort => Some((1, 0, 0)),
            Self::Flow => Some((0, 1, 0)),
            Self::Displacement => Some((0, 1, 1)),
            Self::Momentum => Some((1, 0, 1)),
            Self::Resistance => Some((1, -1, 0)),
            Self::Conductance => Some((-1, 1, 0)),
            Self::Capacitance => Some((-1, 1, 1)),
            Self::Inductance => Some((1, -1, 1)),
            Self::Power => Some((1, 1, 0)),
            Self::Energy => Some((1, 1, 1)),
            Self::EffortRate => Some((1, 0, -1)),
            Self::FlowRate => Some((0, 1, -1)),
            Self::Dimensionless | Self::Unclassified => None,
        }
    }

    /// All 12 classifiable roles (excludes Dimensionless and Unclassified).
    pub const ALL_CLASSIFIABLE: &'static [BondGraphRole] = &[
        Self::Effort,
        Self::Flow,
        Self::Displacement,
        Self::Momentum,
        Self::Resistance,
        Self::Conductance,
        Self::Capacitance,
        Self::Inductance,
        Self::Power,
        Self::Energy,
        Self::EffortRate,
        Self::FlowRate,
    ];

    /// Convert to the coarser [`VariableRole`] for backward compatibility.
    pub fn to_variable_role(self) -> VariableRole {
        match self {
            Self::Effort | Self::EffortRate => VariableRole::Effort,
            Self::Flow | Self::FlowRate => VariableRole::Flow,
            Self::Displacement | Self::Momentum => VariableRole::StateVar,
            Self::Capacitance | Self::Inductance => VariableRole::Storage,
            Self::Resistance | Self::Conductance => VariableRole::Parameter,
            Self::Power | Self::Energy => VariableRole::Parameter,
            Self::Dimensionless | Self::Unclassified => VariableRole::Parameter,
        }
    }
}

// ---------------------------------------------------------------------------
// Builtin dimension vectors (ISO 80000 constants)
// ---------------------------------------------------------------------------

/// Voltage: L^2 * M * T^-3 * I^-1
const VOLTAGE_DIM: DimensionVector = DimensionVector {
    length: 2,
    mass: 1,
    time: -3,
    current: -1,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Electric current: I^1
const CURRENT_DIM: DimensionVector = DimensionVector {
    length: 0,
    mass: 0,
    time: 0,
    current: 1,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Temperature: Theta^1
const TEMPERATURE_DIM: DimensionVector = DimensionVector {
    length: 0,
    mass: 0,
    time: 0,
    current: 0,
    temperature: 1,
    amount: 0,
    luminosity: 0,
};

/// Heat flow rate / power: L^2 * M * T^-3
const HEAT_FLOW_DIM: DimensionVector = DimensionVector {
    length: 2,
    mass: 1,
    time: -3,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Pressure: L^-1 * M * T^-2
const PRESSURE_DIM: DimensionVector = DimensionVector {
    length: -1,
    mass: 1,
    time: -2,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Mass flow rate: M * T^-1
const MASS_FLOW_DIM: DimensionVector = DimensionVector {
    length: 0,
    mass: 1,
    time: -1,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Velocity: L * T^-1
const VELOCITY_DIM: DimensionVector = DimensionVector {
    length: 1,
    mass: 0,
    time: -1,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Force: L * M * T^-2
const FORCE_DIM: DimensionVector = DimensionVector {
    length: 1,
    mass: 1,
    time: -2,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Angular velocity: T^-1 (dimensionless angle / time)
const ANGULAR_VELOCITY_DIM: DimensionVector = DimensionVector {
    length: 0,
    mass: 0,
    time: -1,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Torque: L^2 * M * T^-2
const TORQUE_DIM: DimensionVector = DimensionVector {
    length: 2,
    mass: 1,
    time: -2,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Chemical potential: L^2 * M * T^-2 * N^-1
const CHEMICAL_POTENTIAL_DIM: DimensionVector = DimensionVector {
    length: 2,
    mass: 1,
    time: -2,
    current: 0,
    temperature: 0,
    amount: -1,
    luminosity: 0,
};

/// Molar flow rate: N * T^-1
const MOLAR_FLOW_DIM: DimensionVector = DimensionVector {
    length: 0,
    mass: 0,
    time: -1,
    current: 0,
    temperature: 0,
    amount: 1,
    luminosity: 0,
};

/// Luminous intensity: J^1
const LUMINOUS_INTENSITY_DIM: DimensionVector = DimensionVector {
    length: 0,
    mass: 0,
    time: 0,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 1,
};

/// Luminous flux: J^1 (same base dimension; steradian is dimensionless)
const LUMINOUS_FLUX_DIM: DimensionVector = DimensionVector {
    length: 0,
    mass: 0,
    time: 0,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 1,
};

/// Magnetomotive force (MMF): I (ampere-turns, same base as current)
const MMF_DIM: DimensionVector = DimensionVector {
    length: 0,
    mass: 0,
    time: 0,
    current: 1,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Rate of change of magnetic flux: L^2 * M * T^-3 * I^-1 (volt = Wb/s)
/// Note: same dimension as voltage. Disambiguation uses IsqCategory.
const MAGNETIC_FLUX_RATE_DIM: DimensionVector = DimensionVector {
    length: 2,
    mass: 1,
    time: -3,
    current: -1,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Sound pressure: L^-1 * M * T^-2 (Pa — same as hydraulic pressure)
/// Disambiguation uses IsqCategory::Acoustic.
const SOUND_PRESSURE_DIM: DimensionVector = DimensionVector {
    length: -1,
    mass: 1,
    time: -2,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

/// Volume velocity (acoustic flow): L^3 * T^-1
const VOLUME_VELOCITY_DIM: DimensionVector = DimensionVector {
    length: 3,
    mass: 0,
    time: -1,
    current: 0,
    temperature: 0,
    amount: 0,
    luminosity: 0,
};

impl PhysicsDomain {
    /// Classify a dimension vector into its bond graph role within this domain.
    ///
    /// Iterates all 12 classifiable roles, computing each expected dimension
    /// from `dimension_formula()` and comparing. Returns the first match,
    /// or `Dimensionless`/`Unclassified` if none match.
    pub fn classify_bond_graph_role(&self, dim: &DimensionVector) -> BondGraphRole {
        if dim.is_zero() {
            return BondGraphRole::Dimensionless;
        }

        let e = self.effort_dimensions;
        let f = self.flow_dimensions;

        for &role in BondGraphRole::ALL_CLASSIFIABLE {
            // Every classifiable role has a formula — unwrap is safe here
            let (ea, fa, ta) = role.dimension_formula().unwrap();
            let expected = scale_dim(e, ea) + scale_dim(f, fa) + scale_dim(TIME_DIM, ta);
            if *dim == expected {
                return role;
            }
        }

        BondGraphRole::Unclassified
    }
}

/// Multiply a dimension vector by a scalar coefficient (-1, 0, or 1).
/// Used by `classify_bond_graph_role` to compute `e*a + f*b + t*c`.
const fn scale_dim(d: DimensionVector, coeff: i8) -> DimensionVector {
    match coeff {
        0 => DimensionVector::new(0, 0, 0, 0, 0, 0, 0),
        1 => d,
        -1 => DimensionVector::new(
            -d.length,
            -d.mass,
            -d.time,
            -d.current,
            -d.temperature,
            -d.amount,
            -d.luminosity,
        ),
        _ => d, // only -1, 0, 1 are used
    }
}

/// Built-in physics domains — dimension vectors are mathematical facts from ISO 80000.
///
/// Each domain models a generalized effort/flow pair with a conservation law:
/// - **Effort**: shared across connections (voltage, temperature, pressure, velocity, ...)
/// - **Flow**: conserved through connections (current, heat flow, mass flow, force, ...)
///
/// Domains where effort/flow dimensions collide with other domains rely on
/// [`PhysicsDomainRegistry::classify_dimension_with_hint`] for disambiguation
/// via the ISQ category.
pub static BUILTIN_DOMAINS: &[PhysicsDomain] = &[
    PhysicsDomain {
        name: "electrical",
        effort_dimensions: VOLTAGE_DIM,
        flow_dimensions: CURRENT_DIM,
        conservation: ConservationLaw::FlowConservation,
    },
    PhysicsDomain {
        name: "thermal",
        effort_dimensions: TEMPERATURE_DIM,
        flow_dimensions: HEAT_FLOW_DIM,
        conservation: ConservationLaw::EnergyBalance,
    },
    PhysicsDomain {
        name: "hydraulic",
        effort_dimensions: PRESSURE_DIM,
        flow_dimensions: MASS_FLOW_DIM,
        conservation: ConservationLaw::FlowConservation,
    },
    PhysicsDomain {
        name: "mechanical_translational",
        effort_dimensions: VELOCITY_DIM,
        flow_dimensions: FORCE_DIM,
        conservation: ConservationLaw::FlowConservation,
    },
    PhysicsDomain {
        name: "mechanical_rotational",
        effort_dimensions: ANGULAR_VELOCITY_DIM,
        flow_dimensions: TORQUE_DIM,
        conservation: ConservationLaw::FlowConservation,
    },
    PhysicsDomain {
        name: "chemical",
        effort_dimensions: CHEMICAL_POTENTIAL_DIM,
        flow_dimensions: MOLAR_FLOW_DIM,
        conservation: ConservationLaw::FlowConservation,
    },
    // Note: luminous domain has effort=flow dimension (J^1). This means
    // classify_dimension cannot distinguish effort from flow — the ISQ category
    // or feature name is needed. This is intentional; the domain exists so that
    // luminous ISQ types don't silently fall to None.
    PhysicsDomain {
        name: "luminous",
        effort_dimensions: LUMINOUS_INTENSITY_DIM,
        flow_dimensions: LUMINOUS_FLUX_DIM,
        conservation: ConservationLaw::SignalRouting,
    },
    // Magnetic domain: effort = magnetomotive force (MMF, ampere-turns),
    // flow = dΦ/dt (rate of flux change, in V = Wb/s).
    // Permeance is the C-element. Electrical↔magnetic coupling uses a gyrator (GY).
    // Note: MMF has same base dimension as current (I), and dΦ/dt has same as
    // voltage — disambiguation requires IsqCategory hint.
    PhysicsDomain {
        name: "magnetic",
        effort_dimensions: MMF_DIM,
        flow_dimensions: MAGNETIC_FLUX_RATE_DIM,
        conservation: ConservationLaw::FlowConservation,
    },
    // Acoustic domain: effort = sound pressure (Pa), flow = volume velocity (m³/s).
    // Acoustic impedance is the R-element. Note: sound pressure has same dimension
    // as hydraulic pressure — disambiguation requires IsqCategory::Acoustic hint.
    PhysicsDomain {
        name: "acoustic",
        effort_dimensions: SOUND_PRESSURE_DIM,
        flow_dimensions: VOLUME_VELOCITY_DIM,
        conservation: ConservationLaw::FlowConservation,
    },
];

// ---------------------------------------------------------------------------
// PhysicsDomainRegistry
// ---------------------------------------------------------------------------

/// Registry mapping ISQ type names to dimension vectors and physics domains.
///
/// Built from a workspace `ModelGraph` by walking ISQ `AttributeDefinition` elements
/// that specialize `ScalarQuantityValue`, finding associated unit types, and extracting
/// dimension vectors from their `QuantityPowerFactor` children.
#[derive(Clone, Debug)]
pub struct PhysicsDomainRegistry {
    /// Builtin domain table reference.
    domains: &'static [PhysicsDomain],
    /// Cache: ISQ type name -> DimensionVector.
    type_dimensions: HashMap<String, DimensionVector>,
}

impl PhysicsDomainRegistry {
    /// Create a registry with builtin domains and the exhaustive ISQ type table.
    ///
    /// Pre-populates dimension vectors for all 281 non-dimensionless ISQ types
    /// from the standard library (ISO 80000). This means classification works
    /// without loading the ISQ standard library into the workspace graph.
    pub fn new() -> Self {
        let mut type_dimensions = HashMap::with_capacity(super::isq_types::ISQ_TYPES.len());

        for &(name, ref dim, _category) in super::isq_types::ISQ_TYPES {
            type_dimensions.insert(name.to_owned(), dim.clone());
        }

        Self {
            domains: BUILTIN_DOMAINS,
            type_dimensions,
        }
    }

    /// Build from a merged workspace graph. Walks ISQ `AttributeDefinition` elements:
    ///
    /// 1. Find all `AttributeDefinition` elements
    /// 2. For each, check if any child `Subclassification` has
    ///    `"unresolved_superclassifier"` containing `"ScalarQuantityValue"`
    /// 3. If so, it is an ISQ value type. Find the associated unit by:
    ///    strip `"Value"` suffix, append `"Unit"`, search for an element with that name
    /// 4. Call `extract_dimension_from_unit_element` on the unit
    /// 5. Cache: type_name -> DimensionVector
    pub fn from_workspace_graph(graph: &ModelGraph) -> Self {
        // Start with the hardcoded ISQ types as a baseline
        let baseline = Self::new();
        let mut type_dimensions = baseline.type_dimensions;

        // Step 1: Find all AttributeDefinition elements
        for attr_def in graph.elements_by_kind(&ElementKind::AttributeDefinition) {
            let type_name = match &attr_def.name {
                Some(n) => n.clone(),
                None => continue,
            };

            // Step 2: Check children for Subclassification with ScalarQuantityValue
            let is_scalar_qty = graph.children_of(&attr_def.id).any(|child| {
                if child.kind != ElementKind::Subclassification {
                    return false;
                }
                child
                    .get_prop("unresolved_superclassifier")
                    .and_then(|v| v.as_str())
                    .map_or(false, |s| s.contains("ScalarQuantityValue"))
            });

            if !is_scalar_qty {
                continue;
            }

            // Step 3: Find associated unit element
            let unit_name = if let Some(stripped) = type_name.strip_suffix("Value") {
                format!("{stripped}Unit")
            } else {
                continue;
            };

            let unit_ids = graph.lookup_by_name(&unit_name);
            if unit_ids.is_empty() {
                continue;
            }

            // Use the first matching unit element
            if let Some(unit_elem) = graph.elements.get(&unit_ids[0]) {
                // Step 4: Extract dimension vector
                let dim = extract_dimension_from_unit_element(&unit_elem.id, graph);

                if !dim.is_zero() {
                    // Step 5: Cache
                    type_dimensions.insert(type_name, dim);
                }
            }
        }

        Self {
            domains: BUILTIN_DOMAINS,
            type_dimensions,
        }
    }

    /// Classify a dimension vector against the builtin domains.
    ///
    /// Returns the matching domain and the variable role (effort or flow).
    /// Returns `None` if the dimension vector does not match any known domain.
    ///
    /// For ambiguous dimensions (e.g., Power and HeatFlowRate both have `L²·M·T⁻³`),
    /// use [`classify_dimension_with_hint`] to disambiguate using the ISQ category.
    pub fn classify_dimension(
        &self,
        dim: &DimensionVector,
    ) -> Option<(&PhysicsDomain, VariableRole)> {
        if dim.is_zero() {
            return None;
        }
        for domain in self.domains {
            if *dim == domain.effort_dimensions {
                return Some((domain, VariableRole::Effort));
            }
            if *dim == domain.flow_dimensions {
                return Some((domain, VariableRole::Flow));
            }
        }
        None
    }

    /// Classify a dimension vector with an ISQ category hint for disambiguation.
    ///
    /// Some dimension vectors are shared between domains:
    /// - `L²·M·T⁻³` is both electrical Power and thermal HeatFlowRate
    /// - `L²·M·T⁻²` is both mechanical Energy and mechanical Torque
    ///
    /// The `IsqCategory` from the type table tells us which ISQ domain the type
    /// belongs to, allowing correct classification.
    pub fn classify_dimension_with_hint(
        &self,
        dim: &DimensionVector,
        category: super::isq_types::IsqCategory,
    ) -> Option<(&PhysicsDomain, VariableRole)> {
        use super::isq_types::IsqCategory;

        if dim.is_zero() {
            return None;
        }

        // Collect all matching (domain, role) pairs
        let mut matches: Vec<(&PhysicsDomain, VariableRole)> = Vec::new();
        for domain in self.domains {
            if *dim == domain.effort_dimensions {
                matches.push((domain, VariableRole::Effort));
            }
            if *dim == domain.flow_dimensions {
                matches.push((domain, VariableRole::Flow));
            }
        }

        if matches.is_empty() {
            return None;
        }
        if matches.len() == 1 {
            return Some(matches[0].clone());
        }

        // Disambiguate using ISQ category
        let preferred_domain = match category {
            IsqCategory::Electromagnetic => "electrical",
            IsqCategory::Thermal => "thermal",
            IsqCategory::Mechanical => "mechanical_translational",
            IsqCategory::Acoustic => "acoustic",
            IsqCategory::Chemical => "chemical",
            IsqCategory::Luminous => "luminous",
            _ => "",
        };

        matches
            .iter()
            .find(|(d, _)| d.name == preferred_domain)
            .cloned()
            .or_else(|| Some(matches[0].clone()))
    }

    /// Classify a dimension vector into its full bond graph role.
    ///
    /// Unlike [`classify_dimension`] which only returns Effort/Flow, this method
    /// also identifies R/C/I/Power roles using dimension arithmetic.
    /// Returns the first domain where the dimension matches a non-trivial role.
    pub fn classify_dimension_full(
        &self,
        dim: &DimensionVector,
    ) -> Option<(&PhysicsDomain, BondGraphRole)> {
        if dim.is_zero() {
            return None;
        }
        // Priority: effort/flow first (backward compat), then R/C/I/Power
        for domain in self.domains {
            // The luminous domain has effort == flow (J¹), so its R/C/I
            // dimension arithmetic degenerates (e.g. C = J·T/J = T¹, which
            // would claim every plain time/duration quantity as a luminous
            // capacitor). Luminous classification requires an explicit
            // IsqCategory::Luminous hint — see classify_dimension_full_with_hint.
            if domain.name == "luminous" {
                continue;
            }
            let role = domain.classify_bond_graph_role(dim);
            if role != BondGraphRole::Unclassified && role != BondGraphRole::Dimensionless {
                return Some((domain, role));
            }
        }
        None
    }

    /// Classify with ISQ category hint for disambiguation.
    ///
    /// When a dimension matches multiple domains (e.g., `L²·M·T⁻³` is both
    /// electrical Power and thermal Flow), the category hint selects the correct domain.
    pub fn classify_dimension_full_with_hint(
        &self,
        dim: &DimensionVector,
        category: super::isq_types::IsqCategory,
    ) -> Option<(&PhysicsDomain, BondGraphRole)> {
        use super::isq_types::IsqCategory;

        if dim.is_zero() {
            return None;
        }

        let mut matches: Vec<(&PhysicsDomain, BondGraphRole)> = Vec::new();
        for domain in self.domains {
            // Luminous R/C/I arithmetic degenerates (effort == flow) — only
            // admit it when the ISQ category explicitly says Luminous.
            if domain.name == "luminous" && category != IsqCategory::Luminous {
                continue;
            }
            let role = domain.classify_bond_graph_role(dim);
            if role != BondGraphRole::Unclassified && role != BondGraphRole::Dimensionless {
                matches.push((domain, role));
            }
        }

        if matches.is_empty() {
            return None;
        }
        if matches.len() == 1 {
            return Some(matches[0]);
        }

        let preferred_domain = match category {
            IsqCategory::Electromagnetic => "electrical",
            IsqCategory::Thermal => "thermal",
            IsqCategory::Mechanical => "mechanical_translational",
            IsqCategory::Acoustic => "acoustic",
            IsqCategory::Chemical => "chemical",
            IsqCategory::Luminous => "luminous",
            _ => "",
        };

        matches
            .iter()
            .find(|(d, _)| d.name == preferred_domain)
            .copied()
            .or_else(|| Some(matches[0]))
    }

    /// Look up the cached dimension vector for an ISQ type name.
    pub fn dimension_for_type(&self, type_name: &str) -> Option<&DimensionVector> {
        self.type_dimensions.get(type_name)
    }

    /// Manually register a type-to-dimension mapping.
    pub fn register_type(&mut self, type_name: String, dim: DimensionVector) {
        self.type_dimensions.insert(type_name, dim);
    }

    /// Access the builtin domain table.
    pub fn domains(&self) -> &[PhysicsDomain] {
        self.domains
    }
}

impl Default for PhysicsDomainRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn builtin_domains_count() {
        assert_eq!(BUILTIN_DOMAINS.len(), 9);
    }

    #[test]
    fn duration_does_not_classify_as_luminous_capacitor() {
        // T¹ (DurationValue / timestamps) used to match the luminous domain's
        // C-element because luminous effort == flow == J¹ degenerates the
        // R/C/I arithmetic (C = J·T/J = T¹). Un-hinted full classification
        // must never return luminous.
        let registry = PhysicsDomainRegistry::new();
        let duration = DimensionVector {
            time: 1,
            ..Default::default()
        };
        if let Some((domain, role)) = registry.classify_dimension_full(&duration) {
            assert_ne!(
                domain.name, "luminous",
                "T¹ classified as luminous {role:?} — degenerate arithmetic leaked"
            );
        }
        // Hinted with a non-luminous category: still no luminous.
        if let Some((domain, _)) = registry
            .classify_dimension_full_with_hint(&duration, super::super::isq_types::IsqCategory::SpaceTime)
        {
            assert_ne!(domain.name, "luminous");
        }
    }

    #[test]
    fn luminous_still_reachable_with_explicit_hint() {
        let registry = PhysicsDomainRegistry::new();
        let lum = DimensionVector {
            luminosity: 1,
            ..Default::default()
        };
        let (domain, _) = registry
            .classify_dimension_full_with_hint(&lum, super::super::isq_types::IsqCategory::Luminous)
            .expect("J¹ with Luminous hint must classify");
        assert_eq!(domain.name, "luminous");
    }

    #[test]
    fn classify_voltage_as_electrical_effort() {
        let registry = PhysicsDomainRegistry::new();
        let voltage = DimensionVector::new(2, 1, -3, -1, 0, 0, 0);
        let result = registry.classify_dimension(&voltage);
        assert!(result.is_some());
        let (domain, role) = result.unwrap();
        assert_eq!(domain.name, "electrical");
        assert_eq!(role, VariableRole::Effort);
    }

    #[test]
    fn classify_current_as_electrical_flow() {
        let registry = PhysicsDomainRegistry::new();
        let current = DimensionVector {
            current: 1,
            ..Default::default()
        };
        let result = registry.classify_dimension(&current);
        assert!(result.is_some());
        let (domain, role) = result.unwrap();
        assert_eq!(domain.name, "electrical");
        assert_eq!(role, VariableRole::Flow);
    }

    #[test]
    fn classify_temperature_as_thermal_effort() {
        let registry = PhysicsDomainRegistry::new();
        let temp = DimensionVector {
            temperature: 1,
            ..Default::default()
        };
        let (domain, role) = registry.classify_dimension(&temp).unwrap();
        assert_eq!(domain.name, "thermal");
        assert_eq!(role, VariableRole::Effort);
    }

    #[test]
    fn classify_heat_flow_as_thermal_flow() {
        let registry = PhysicsDomainRegistry::new();
        let heat = DimensionVector::new(2, 1, -3, 0, 0, 0, 0);
        let (domain, role) = registry.classify_dimension(&heat).unwrap();
        assert_eq!(domain.name, "thermal");
        assert_eq!(role, VariableRole::Flow);
    }

    #[test]
    fn classify_pressure_as_hydraulic_effort() {
        let registry = PhysicsDomainRegistry::new();
        let pressure = DimensionVector::new(-1, 1, -2, 0, 0, 0, 0);
        let (domain, role) = registry.classify_dimension(&pressure).unwrap();
        assert_eq!(domain.name, "hydraulic");
        assert_eq!(role, VariableRole::Effort);
    }

    #[test]
    fn classify_mass_flow_as_hydraulic_flow() {
        let registry = PhysicsDomainRegistry::new();
        let mflow = DimensionVector::new(0, 1, -1, 0, 0, 0, 0);
        let (domain, role) = registry.classify_dimension(&mflow).unwrap();
        assert_eq!(domain.name, "hydraulic");
        assert_eq!(role, VariableRole::Flow);
    }

    #[test]
    fn classify_velocity_as_mechanical_effort() {
        let registry = PhysicsDomainRegistry::new();
        let vel = DimensionVector::new(1, 0, -1, 0, 0, 0, 0);
        let (domain, role) = registry.classify_dimension(&vel).unwrap();
        assert_eq!(domain.name, "mechanical_translational");
        assert_eq!(role, VariableRole::Effort);
    }

    #[test]
    fn classify_force_as_mechanical_flow() {
        let registry = PhysicsDomainRegistry::new();
        let force = DimensionVector::new(1, 1, -2, 0, 0, 0, 0);
        let (domain, role) = registry.classify_dimension(&force).unwrap();
        assert_eq!(domain.name, "mechanical_translational");
        assert_eq!(role, VariableRole::Flow);
    }

    #[test]
    fn classify_dimensionless_returns_none() {
        let registry = PhysicsDomainRegistry::new();
        assert!(registry
            .classify_dimension(&DimensionVector::default())
            .is_none());
    }

    #[test]
    fn classify_unknown_dimension_returns_none() {
        let registry = PhysicsDomainRegistry::new();
        // Resistance: L^2 * M * T^-3 * I^-2 -- not an effort or flow
        let resistance = DimensionVector::new(2, 1, -3, -2, 0, 0, 0);
        assert!(registry.classify_dimension(&resistance).is_none());
    }

    #[test]
    fn isq_types_exhaustively_populated() {
        let registry = PhysicsDomainRegistry::new();
        // Should have all 281 non-dimensionless ISQ types
        assert_eq!(
            registry.type_dimensions.len(),
            crate::physics::isq_types::ISQ_TYPES.len(),
            "registry should contain all ISQ types from the exhaustive table"
        );
    }

    #[test]
    fn classify_known_isq_types() {
        let registry = PhysicsDomainRegistry::new();
        // Electrical
        let current_dim = registry.dimension_for_type("ElectricCurrentValue").unwrap();
        let (domain, role) = registry.classify_dimension(current_dim).unwrap();
        assert_eq!(domain.name, "electrical");
        assert_eq!(role, VariableRole::Flow);
        let voltage_dim = registry
            .dimension_for_type("ElectricPotentialValue")
            .unwrap();
        let (domain, role) = registry.classify_dimension(voltage_dim).unwrap();
        assert_eq!(domain.name, "electrical");
        assert_eq!(role, VariableRole::Effort);
        // Thermal
        let temp_dim = registry
            .dimension_for_type("ThermodynamicTemperatureValue")
            .unwrap();
        let (domain, role) = registry.classify_dimension(temp_dim).unwrap();
        assert_eq!(domain.name, "thermal");
        assert_eq!(role, VariableRole::Effort);
        // Mechanical
        let force_dim = registry.dimension_for_type("ForceValue").unwrap();
        let (domain, role) = registry.classify_dimension(force_dim).unwrap();
        assert_eq!(domain.name, "mechanical_translational");
        assert_eq!(role, VariableRole::Flow);
        // Hydraulic
        let pressure_dim = registry.dimension_for_type("PressureValue").unwrap();
        let (domain, role) = registry.classify_dimension(pressure_dim).unwrap();
        assert_eq!(domain.name, "hydraulic");
        assert_eq!(role, VariableRole::Effort);
    }

    #[test]
    fn dimension_for_type_with_manual_registration() {
        let mut registry = PhysicsDomainRegistry::new();
        let current_dim = DimensionVector {
            current: 1,
            ..Default::default()
        };
        registry.register_type("ElectricCurrentValue".into(), current_dim.clone());
        assert_eq!(
            registry.dimension_for_type("ElectricCurrentValue"),
            Some(&current_dim)
        );
    }

    #[test]
    fn dimension_for_type_unknown() {
        let registry = PhysicsDomainRegistry::new();
        assert!(registry.dimension_for_type("UnknownType").is_none());
    }

    #[test]
    fn from_workspace_graph_empty() {
        let graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::from_workspace_graph(&graph);
        // Only contains the hardcoded ISQ types, no graph-derived ones
        let baseline = PhysicsDomainRegistry::new();
        assert_eq!(
            registry.type_dimensions.len(),
            baseline.type_dimensions.len()
        );
    }

    #[test]
    fn from_workspace_graph_with_isq_type() {
        use crate::{Element, ElementId, Value};

        let mut graph = ModelGraph::new();

        // Create the ISQ value type: ElectricCurrentValue :> ScalarQuantityValue
        let val_id = ElementId::new_v4();
        graph.add_element(
            Element::new(val_id.clone(), ElementKind::AttributeDefinition)
                .with_name("ElectricCurrentValue"),
        );

        // Add Subclassification child pointing to ScalarQuantityValue
        let sub_id = ElementId::new_v4();
        graph.add_element(
            Element::new(sub_id, ElementKind::Subclassification)
                .with_owner(val_id.clone())
                .with_prop(
                    "unresolved_superclassifier",
                    Value::String("ScalarQuantityValue".into()),
                ),
        );

        // Create the unit element: ElectricCurrentUnit
        let unit_id = ElementId::new_v4();
        graph.add_element(
            Element::new(unit_id.clone(), ElementKind::AttributeDefinition)
                .with_name("ElectricCurrentUnit"),
        );

        // quantityDimension child
        let qty_dim_id = ElementId::new_v4();
        graph.add_element(
            Element::new(qty_dim_id.clone(), ElementKind::AttributeUsage)
                .with_owner(unit_id.clone())
                .with_prop(
                    "unresolved_redefinedFeature",
                    Value::String("quantityDimension".into()),
                ),
        );

        // Power factor: quantity=isq.I, exponent=1
        let factor_id = ElementId::new_v4();
        graph.add_element(
            Element::new(factor_id.clone(), ElementKind::AttributeUsage)
                .with_owner(qty_dim_id.clone()),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(factor_id.clone())
                .with_prop(
                    "unresolved_redefinedFeature",
                    Value::String("quantity".into()),
                )
                .with_prop("unresolved_value", Value::String("isq.I".into())),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(factor_id)
                .with_prop(
                    "unresolved_redefinedFeature",
                    Value::String("exponent".into()),
                )
                .with_prop("unresolved_value", Value::String("1".into())),
        );

        let registry = PhysicsDomainRegistry::from_workspace_graph(&graph);
        let dim = registry.dimension_for_type("ElectricCurrentValue");
        assert!(dim.is_some(), "should find ElectricCurrentValue dimension");
        assert_eq!(dim.unwrap().current, 1);
        assert_eq!(dim.unwrap().length, 0);
    }

    // -----------------------------------------------------------------------
    // Bond graph role classification tests
    // -----------------------------------------------------------------------

    fn electrical_domain() -> &'static PhysicsDomain {
        BUILTIN_DOMAINS
            .iter()
            .find(|d| d.name == "electrical")
            .unwrap()
    }

    fn thermal_domain() -> &'static PhysicsDomain {
        BUILTIN_DOMAINS
            .iter()
            .find(|d| d.name == "thermal")
            .unwrap()
    }

    #[test]
    fn bond_graph_role_electrical_effort() {
        let voltage = DimensionVector::new(2, 1, -3, -1, 0, 0, 0);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&voltage),
            BondGraphRole::Effort
        );
    }

    #[test]
    fn bond_graph_role_electrical_flow() {
        let current = DimensionVector::new(0, 0, 0, 1, 0, 0, 0);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&current),
            BondGraphRole::Flow
        );
    }

    #[test]
    fn bond_graph_role_electrical_resistance() {
        // Resistance: L²·M·T⁻³·I⁻² = voltage(L²·M·T⁻³·I⁻¹) - current(I)
        let resistance = DimensionVector::new(2, 1, -3, -2, 0, 0, 0);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&resistance),
            BondGraphRole::Resistance
        );
    }

    #[test]
    fn bond_graph_role_electrical_capacitance() {
        // Capacitance: L⁻²·M⁻¹·T⁴·I² = current(I) - voltage(L²·M·T⁻³·I⁻¹) + time(T)
        let capacitance = DimensionVector::new(-2, -1, 4, 2, 0, 0, 0);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&capacitance),
            BondGraphRole::Capacitance
        );
    }

    #[test]
    fn bond_graph_role_electrical_inductance() {
        // Inductance: L²·M·T⁻²·I⁻² = voltage(L²·M·T⁻³·I⁻¹) - current(I) + time(T)
        let inductance = DimensionVector::new(2, 1, -2, -2, 0, 0, 0);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&inductance),
            BondGraphRole::Inductance
        );
    }

    #[test]
    fn bond_graph_role_electrical_power() {
        // Power: L²·M·T⁻³ = voltage + current
        let power = DimensionVector::new(2, 1, -3, 0, 0, 0, 0);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&power),
            BondGraphRole::Power
        );
    }

    #[test]
    fn bond_graph_role_thermal_resistance() {
        // Thermal resistance: dim(T) - dim(heat_flow) = Θ - L²·M·T⁻³
        let thermal_resistance = DimensionVector::new(-2, -1, 3, 0, 1, 0, 0);
        assert_eq!(
            thermal_domain().classify_bond_graph_role(&thermal_resistance),
            BondGraphRole::Resistance
        );
    }

    #[test]
    fn bond_graph_role_dimensionless() {
        let d = DimensionVector::default();
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&d),
            BondGraphRole::Dimensionless
        );
    }

    #[test]
    fn bond_graph_role_unclassified() {
        // Luminous intensity has nothing to do with electrical domain
        let luminous = DimensionVector::new(0, 0, 0, 0, 0, 0, 1);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&luminous),
            BondGraphRole::Unclassified
        );
    }

    #[test]
    fn bond_graph_role_electrical_displacement() {
        // Charge: T·I = flow(I) + time(T)
        let charge = DimensionVector::new(0, 0, 1, 1, 0, 0, 0);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&charge),
            BondGraphRole::Displacement
        );
    }

    #[test]
    fn bond_graph_role_electrical_momentum() {
        // Flux linkage: L²·M·T⁻²·I⁻¹ = voltage(L²·M·T⁻³·I⁻¹) + time(T)
        let flux_linkage = DimensionVector::new(2, 1, -2, -1, 0, 0, 0);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&flux_linkage),
            BondGraphRole::Momentum
        );
    }

    #[test]
    fn bond_graph_role_electrical_conductance() {
        // Conductance: L⁻²·M⁻¹·T³·I² = current(I) - voltage(L²·M·T⁻³·I⁻¹)
        let conductance = DimensionVector::new(-2, -1, 3, 2, 0, 0, 0);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&conductance),
            BondGraphRole::Conductance
        );
    }

    #[test]
    fn bond_graph_role_electrical_energy() {
        // Energy: L²·M·T⁻² = voltage + current + time
        let energy = DimensionVector::new(2, 1, -2, 0, 0, 0, 0);
        assert_eq!(
            electrical_domain().classify_bond_graph_role(&energy),
            BondGraphRole::Energy
        );
    }

    #[test]
    fn bond_graph_role_mechanical_displacement_and_momentum() {
        // Our mechanical_translational domain: effort=velocity, flow=force
        // Displacement = ∫flow dt = ∫force dt = impulse: L·M·T⁻¹
        // Momentum = ∫effort dt = ∫velocity dt = position: L
        let mech = BUILTIN_DOMAINS
            .iter()
            .find(|d| d.name == "mechanical_translational")
            .unwrap();

        let impulse = DimensionVector::new(1, 1, -1, 0, 0, 0, 0);
        assert_eq!(
            mech.classify_bond_graph_role(&impulse),
            BondGraphRole::Displacement
        );

        let position = DimensionVector::new(1, 0, 0, 0, 0, 0, 0);
        assert_eq!(
            mech.classify_bond_graph_role(&position),
            BondGraphRole::Momentum
        );
    }

    #[test]
    fn bond_graph_role_mechanical_acceleration() {
        // Acceleration = effort_rate = velocity(L·T⁻¹) - time(T) = L·T⁻²
        let mech = BUILTIN_DOMAINS
            .iter()
            .find(|d| d.name == "mechanical_translational")
            .unwrap();
        let accel = DimensionVector::new(1, 0, -2, 0, 0, 0, 0);
        assert_eq!(
            mech.classify_bond_graph_role(&accel),
            BondGraphRole::EffortRate
        );
    }

    #[test]
    fn bond_graph_role_to_variable_role() {
        assert_eq!(
            BondGraphRole::Effort.to_variable_role(),
            VariableRole::Effort
        );
        assert_eq!(BondGraphRole::Flow.to_variable_role(), VariableRole::Flow);
        assert_eq!(
            BondGraphRole::Resistance.to_variable_role(),
            VariableRole::Parameter
        );
        assert_eq!(
            BondGraphRole::Capacitance.to_variable_role(),
            VariableRole::Storage
        );
        assert_eq!(
            BondGraphRole::Inductance.to_variable_role(),
            VariableRole::Storage
        );
        assert_eq!(
            BondGraphRole::Displacement.to_variable_role(),
            VariableRole::StateVar
        );
        assert_eq!(
            BondGraphRole::Momentum.to_variable_role(),
            VariableRole::StateVar
        );
        assert_eq!(
            BondGraphRole::Conductance.to_variable_role(),
            VariableRole::Parameter
        );
        assert_eq!(
            BondGraphRole::Energy.to_variable_role(),
            VariableRole::Parameter
        );
        assert_eq!(
            BondGraphRole::EffortRate.to_variable_role(),
            VariableRole::Effort
        );
        assert_eq!(
            BondGraphRole::FlowRate.to_variable_role(),
            VariableRole::Flow
        );
    }

    #[test]
    fn dimension_formula_completeness() {
        // Every classifiable role must have a formula
        for &role in BondGraphRole::ALL_CLASSIFIABLE {
            assert!(
                role.dimension_formula().is_some(),
                "{:?} must have a dimension formula",
                role,
            );
        }
        // Non-classifiable roles must not
        assert!(BondGraphRole::Dimensionless.dimension_formula().is_none());
        assert!(BondGraphRole::Unclassified.dimension_formula().is_none());
    }

    #[test]
    fn bond_graph_role_isq_types_classified() {
        // Verify that known ISQ parameter types now classify as R/C/I
        let registry = PhysicsDomainRegistry::new();
        let e = electrical_domain();

        let resistance_dim = registry.dimension_for_type("ResistanceValue").unwrap();
        assert_eq!(
            e.classify_bond_graph_role(resistance_dim),
            BondGraphRole::Resistance
        );

        let capacitance_dim = registry.dimension_for_type("CapacitanceValue").unwrap();
        assert_eq!(
            e.classify_bond_graph_role(capacitance_dim),
            BondGraphRole::Capacitance
        );

        let inductance_dim = registry.dimension_for_type("InductanceValue").unwrap();
        assert_eq!(
            e.classify_bond_graph_role(inductance_dim),
            BondGraphRole::Inductance
        );
    }

    /// Exhaustive test: every ISQ type is classifiable against at least one domain.
    /// Asserts minimum coverage threshold and that all 12 roles are represented.
    #[test]
    fn bond_graph_role_exhaustive_isq_coverage() {
        use crate::physics::isq_types::ISQ_TYPES;

        let registry = PhysicsDomainRegistry::new();
        let mut classified = 0;
        let mut by_role: std::collections::HashMap<BondGraphRole, usize> =
            std::collections::HashMap::new();

        for &(_name, ref dim, _category) in ISQ_TYPES {
            if let Some((_domain, role)) = registry.classify_dimension_full(dim) {
                classified += 1;
                *by_role.entry(role).or_insert(0) += 1;
            }
        }

        // At least 130 of 281 ISQ types should classify (47%+).
        assert!(
            classified >= 130,
            "expected at least 130 classified ISQ types, got {}",
            classified
        );

        // All 12 classifiable roles should be represented.
        for &role in BondGraphRole::ALL_CLASSIFIABLE {
            assert!(
                by_role.contains_key(&role),
                "role {:?} not represented in ISQ classification",
                role
            );
        }
    }
}
