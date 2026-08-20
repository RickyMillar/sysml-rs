//! ISQ dimension vector infrastructure.
//!
//! Every physical quantity in the International System of Quantities (ISQ) can be
//! expressed as a product of powers of 7 base quantities: length (L), mass (M),
//! time (T), electric current (I), thermodynamic temperature (Theta), amount of
//! substance (N), and luminous intensity (J).
//!
//! This module provides [`DimensionVector`] to represent these exponent tuples,
//! along with utilities to parse ISQ symbols from SysML standard library element
//! graphs and extract dimension vectors from unit definitions.

use std::fmt;
use std::ops::{Add, Neg, Sub};

use crate::{ElementId, ElementKind, ModelGraph, Value};

/// 7 ISQ base quantity exponents (L, M, T, I, Theta, N, J).
///
/// Each field holds the integer exponent for one base quantity.
/// For example, voltage (V) has dimension `L^2 * M * T^-3 * I^-1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DimensionVector {
    /// Length (L)
    pub length: i8,
    /// Mass (M)
    pub mass: i8,
    /// Time (T)
    pub time: i8,
    /// Electric current (I)
    pub current: i8,
    /// Thermodynamic temperature (Theta)
    pub temperature: i8,
    /// Amount of substance (N)
    pub amount: i8,
    /// Luminous intensity (J)
    pub luminosity: i8,
}

impl DimensionVector {
    /// Create a new dimension vector with all exponents specified.
    pub const fn new(
        length: i8,
        mass: i8,
        time: i8,
        current: i8,
        temperature: i8,
        amount: i8,
        luminosity: i8,
    ) -> Self {
        Self {
            length,
            mass,
            time,
            current,
            temperature,
            amount,
            luminosity,
        }
    }

    /// Returns `true` if all exponents are zero (dimensionless quantity).
    pub fn is_zero(&self) -> bool {
        self.length == 0
            && self.mass == 0
            && self.time == 0
            && self.current == 0
            && self.temperature == 0
            && self.amount == 0
            && self.luminosity == 0
    }

    /// Set the exponent for the given ISQ base quantity.
    pub fn set_base(&mut self, base: &IsqBase, exponent: i8) {
        match base {
            IsqBase::L => self.length = exponent,
            IsqBase::M => self.mass = exponent,
            IsqBase::T => self.time = exponent,
            IsqBase::I => self.current = exponent,
            IsqBase::Theta => self.temperature = exponent,
            IsqBase::N => self.amount = exponent,
            IsqBase::J => self.luminosity = exponent,
        }
    }
}

impl fmt::Display for DimensionVector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return write!(f, "1");
        }
        let components: &[(&str, i8)] = &[
            ("L", self.length),
            ("M", self.mass),
            ("T", self.time),
            ("I", self.current),
            ("\u{0398}", self.temperature), // Theta
            ("N", self.amount),
            ("J", self.luminosity),
        ];
        let mut first = true;
        for &(sym, exp) in components {
            if exp == 0 {
                continue;
            }
            if !first {
                write!(f, "\u{00B7}")?; // middle dot
            }
            first = false;
            if exp == 1 {
                write!(f, "{sym}")?;
            } else {
                write!(f, "{sym}^{exp}")?;
            }
        }
        Ok(())
    }
}

impl Add for DimensionVector {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            length: self.length + rhs.length,
            mass: self.mass + rhs.mass,
            time: self.time + rhs.time,
            current: self.current + rhs.current,
            temperature: self.temperature + rhs.temperature,
            amount: self.amount + rhs.amount,
            luminosity: self.luminosity + rhs.luminosity,
        }
    }
}

impl Sub for DimensionVector {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self {
            length: self.length - rhs.length,
            mass: self.mass - rhs.mass,
            time: self.time - rhs.time,
            current: self.current - rhs.current,
            temperature: self.temperature - rhs.temperature,
            amount: self.amount - rhs.amount,
            luminosity: self.luminosity - rhs.luminosity,
        }
    }
}

impl Neg for DimensionVector {
    type Output = Self;

    fn neg(self) -> Self {
        Self {
            length: -self.length,
            mass: -self.mass,
            time: -self.time,
            current: -self.current,
            temperature: -self.temperature,
            amount: -self.amount,
            luminosity: -self.luminosity,
        }
    }
}

/// Time dimension: T^1. Used in bond graph role classification
/// (capacitance = flow - effort + time, inductance = effort - flow + time).
pub const TIME_DIM: DimensionVector = DimensionVector::new(0, 0, 1, 0, 0, 0, 0);

/// The 7 ISQ base quantities.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IsqBase {
    /// Length
    L,
    /// Mass
    M,
    /// Time / Duration
    T,
    /// Electric current
    I,
    /// Thermodynamic temperature
    Theta,
    /// Amount of substance
    N,
    /// Luminous intensity
    J,
}

/// Parse an ISQ dot-path symbol to a base quantity.
///
/// Accepts forms like `"isq.I"`, `"I"`, `"isq.'Θ'"`, `"Theta"`, etc.
/// Splits on `'.'` and matches the last segment.
pub fn parse_isq_symbol(dot_path: &str) -> Option<IsqBase> {
    let segment = dot_path.rsplit('.').next()?;
    // Strip surrounding quotes if present (e.g., `'Θ'`)
    let segment = segment.trim_matches('\'');
    match segment {
        "L" => Some(IsqBase::L),
        "M" => Some(IsqBase::M),
        "T" => Some(IsqBase::T),
        "I" => Some(IsqBase::I),
        "\u{0398}" | "Theta" => Some(IsqBase::Theta),
        "N" => Some(IsqBase::N),
        "J" => Some(IsqBase::J),
        _ => None,
    }
}

/// Walk a unit element's `QuantityPowerFactor` children to extract dimensions.
///
/// The algorithm:
/// 1. Walk `graph.children_of(unit_elem_id)`
/// 2. Find children that have a redefinition of `"quantityDimension"`
/// 3. Walk THEIR children to find `QuantityPowerFactor` items (AttributeUsage children)
/// 4. Each power factor has children with redefinitions for `"quantity"` and `"exponent"`
/// 5. Parse the ISQ symbol suffix and exponent, set the corresponding field
pub fn extract_dimension_from_unit_element(
    unit_elem_id: &ElementId,
    graph: &ModelGraph,
) -> DimensionVector {
    let mut dim = DimensionVector::default();

    // Step 1-2: Find the quantityDimension child
    for child in graph.children_of(unit_elem_id) {
        let is_qty_dim = child
            .get_prop("unresolved_redefinedFeature")
            .and_then(|v| v.as_str())
            .map_or(false, |s| s == "quantityDimension");

        if !is_qty_dim {
            continue;
        }

        // Step 3: Walk quantityDimension's children for power factors
        for factor in graph.children_of(&child.id) {
            if factor.kind != ElementKind::AttributeUsage {
                continue;
            }

            let mut base: Option<IsqBase> = None;
            let mut exponent: i8 = 1;

            // Step 4: Each power factor has children with quantity and exponent
            for prop_child in graph.children_of(&factor.id) {
                let redef = prop_child
                    .get_prop("unresolved_redefinedFeature")
                    .and_then(|v| v.as_str());

                match redef {
                    Some("quantity") => {
                        // The value is something like "isq.I"
                        if let Some(val) = prop_child
                            .get_prop("value")
                            .or_else(|| prop_child.get_prop("unresolved_value"))
                            .and_then(|v| v.as_str())
                        {
                            base = parse_isq_symbol(val);
                        }
                    }
                    Some("exponent") => {
                        if let Some(val) = prop_child
                            .get_prop("value")
                            .or_else(|| prop_child.get_prop("unresolved_value"))
                        {
                            match val {
                                Value::Int(n) => exponent = *n as i8,
                                Value::Float(n) => exponent = *n as i8,
                                Value::String(s) => {
                                    if let Ok(n) = s.parse::<i8>() {
                                        exponent = n;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Step 5: Set the dimension
            if let Some(b) = &base {
                dim.set_base(b, exponent);
            }
        }
    }

    dim
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn dimension_vector_default_is_zero() {
        let d = DimensionVector::default();
        assert!(d.is_zero());
    }

    #[test]
    fn dimension_vector_non_zero() {
        let d = DimensionVector::new(2, 1, -3, -1, 0, 0, 0);
        assert!(!d.is_zero());
    }

    #[test]
    fn display_dimensionless() {
        assert_eq!(DimensionVector::default().to_string(), "1");
    }

    #[test]
    fn display_current() {
        let d = DimensionVector {
            current: 1,
            ..Default::default()
        };
        assert_eq!(d.to_string(), "I");
    }

    #[test]
    fn display_voltage() {
        // Voltage: L^2 * M * T^-3 * I^-1
        let d = DimensionVector::new(2, 1, -3, -1, 0, 0, 0);
        assert_eq!(d.to_string(), "L^2\u{00B7}M\u{00B7}T^-3\u{00B7}I^-1");
    }

    #[test]
    fn display_temperature() {
        let d = DimensionVector {
            temperature: 1,
            ..Default::default()
        };
        assert_eq!(d.to_string(), "\u{0398}");
    }

    #[test]
    fn parse_isq_symbol_all_bases() {
        assert_eq!(parse_isq_symbol("isq.L"), Some(IsqBase::L));
        assert_eq!(parse_isq_symbol("isq.M"), Some(IsqBase::M));
        assert_eq!(parse_isq_symbol("isq.T"), Some(IsqBase::T));
        assert_eq!(parse_isq_symbol("isq.I"), Some(IsqBase::I));
        assert_eq!(parse_isq_symbol("isq.'\u{0398}'"), Some(IsqBase::Theta));
        assert_eq!(parse_isq_symbol("Theta"), Some(IsqBase::Theta));
        assert_eq!(parse_isq_symbol("isq.N"), Some(IsqBase::N));
        assert_eq!(parse_isq_symbol("isq.J"), Some(IsqBase::J));
    }

    #[test]
    fn parse_isq_symbol_bare() {
        assert_eq!(parse_isq_symbol("I"), Some(IsqBase::I));
        assert_eq!(parse_isq_symbol("L"), Some(IsqBase::L));
    }

    #[test]
    fn parse_isq_symbol_unknown() {
        assert_eq!(parse_isq_symbol("isq.X"), None);
        assert_eq!(parse_isq_symbol(""), None);
    }

    #[test]
    fn add_dimension_vectors() {
        let voltage = DimensionVector::new(2, 1, -3, -1, 0, 0, 0);
        let current = DimensionVector::new(0, 0, 0, 1, 0, 0, 0);
        // Power = voltage * current → exponents add
        let power = voltage + current;
        assert_eq!(power, DimensionVector::new(2, 1, -3, 0, 0, 0, 0));
    }

    #[test]
    fn sub_dimension_vectors() {
        let voltage = DimensionVector::new(2, 1, -3, -1, 0, 0, 0);
        let current = DimensionVector::new(0, 0, 0, 1, 0, 0, 0);
        // Resistance = voltage / current → exponents subtract
        let resistance = voltage - current;
        assert_eq!(resistance, DimensionVector::new(2, 1, -3, -2, 0, 0, 0));
    }

    #[test]
    fn neg_dimension_vector() {
        let voltage = DimensionVector::new(2, 1, -3, -1, 0, 0, 0);
        let neg_v = -voltage;
        assert_eq!(neg_v, DimensionVector::new(-2, -1, 3, 1, 0, 0, 0));
    }

    #[test]
    fn time_dim_constant() {
        assert_eq!(TIME_DIM, DimensionVector::new(0, 0, 1, 0, 0, 0, 0));
    }

    #[test]
    fn extract_dimension_empty_graph() {
        let graph = ModelGraph::new();
        let id = ElementId::new_v4();
        let dim = extract_dimension_from_unit_element(&id, &graph);
        assert!(dim.is_zero());
    }

    #[test]
    fn extract_dimension_from_mock_unit() {
        use crate::Element;

        let mut graph = ModelGraph::new();

        // Create the unit element
        let unit_id = ElementId::new_v4();
        let unit = Element::new(unit_id.clone(), ElementKind::AttributeDefinition)
            .with_name("ElectricCurrentUnit");
        graph.add_element(unit);

        // Create quantityDimension child
        let qty_dim_id = ElementId::new_v4();
        let qty_dim = Element::new(qty_dim_id.clone(), ElementKind::AttributeUsage)
            .with_owner(unit_id.clone())
            .with_prop(
                "unresolved_redefinedFeature",
                Value::String("quantityDimension".into()),
            );
        graph.add_element(qty_dim);

        // Create a QuantityPowerFactor child
        let factor_id = ElementId::new_v4();
        let factor = Element::new(factor_id.clone(), ElementKind::AttributeUsage)
            .with_owner(qty_dim_id.clone());
        graph.add_element(factor);

        // Create quantity child (isq.I)
        let qty_child_id = ElementId::new_v4();
        let qty_child = Element::new(qty_child_id, ElementKind::AttributeUsage)
            .with_owner(factor_id.clone())
            .with_prop(
                "unresolved_redefinedFeature",
                Value::String("quantity".into()),
            )
            .with_prop("unresolved_value", Value::String("isq.I".into()));
        graph.add_element(qty_child);

        // Create exponent child (1)
        let exp_child_id = ElementId::new_v4();
        let exp_child = Element::new(exp_child_id, ElementKind::AttributeUsage)
            .with_owner(factor_id)
            .with_prop(
                "unresolved_redefinedFeature",
                Value::String("exponent".into()),
            )
            .with_prop("unresolved_value", Value::String("1".into()));
        graph.add_element(exp_child);

        let dim = extract_dimension_from_unit_element(&unit_id, &graph);
        assert_eq!(dim.current, 1);
        assert_eq!(dim.length, 0);
        assert_eq!(dim.mass, 0);
    }

    #[test]
    fn extract_dimension_voltage_mock() {
        use crate::Element;

        let mut graph = ModelGraph::new();

        // Unit element
        let unit_id = ElementId::new_v4();
        graph.add_element(
            Element::new(unit_id.clone(), ElementKind::AttributeDefinition)
                .with_name("SourceVoltageUnit"),
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

        // 4 power factors: L^2, M^1, T^-3, I^-1
        let factors: &[(&str, i8)] = &[("isq.L", 2), ("isq.M", 1), ("isq.T", -3), ("isq.I", -1)];
        for (qty, exp) in factors {
            let fid = ElementId::new_v4();
            graph.add_element(
                Element::new(fid.clone(), ElementKind::AttributeUsage)
                    .with_owner(qty_dim_id.clone()),
            );

            let qid = ElementId::new_v4();
            graph.add_element(
                Element::new(qid, ElementKind::AttributeUsage)
                    .with_owner(fid.clone())
                    .with_prop(
                        "unresolved_redefinedFeature",
                        Value::String("quantity".into()),
                    )
                    .with_prop("unresolved_value", Value::String((*qty).into())),
            );

            let eid = ElementId::new_v4();
            graph.add_element(
                Element::new(eid, ElementKind::AttributeUsage)
                    .with_owner(fid)
                    .with_prop(
                        "unresolved_redefinedFeature",
                        Value::String("exponent".into()),
                    )
                    .with_prop("unresolved_value", Value::Int(i64::from(*exp))),
            );
        }

        let dim = extract_dimension_from_unit_element(&unit_id, &graph);
        assert_eq!(dim, DimensionVector::new(2, 1, -3, -1, 0, 0, 0));
    }
}
