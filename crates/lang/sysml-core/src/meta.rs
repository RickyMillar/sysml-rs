//! Metadata types for SysML v2: applicability, clause references, and values.
//!
//! These types were originally in the `sysml-meta` crate and are now part of `sysml-core`.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use sysml_id::ElementId;

use crate::physics::DimensionVector;

/// Applicability status of a requirement or element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Applicability {
    /// The element is applicable.
    Applicable,
    /// The element is not applicable.
    NotApplicable,
    /// Applicability is to be determined.
    #[default]
    TBD,
}

impl Applicability {
    /// Check if this is applicable.
    pub fn is_applicable(&self) -> bool {
        matches!(self, Applicability::Applicable)
    }

    /// Check if this is not applicable.
    pub fn is_not_applicable(&self) -> bool {
        matches!(self, Applicability::NotApplicable)
    }

    /// Check if this is to be determined.
    pub fn is_tbd(&self) -> bool {
        matches!(self, Applicability::TBD)
    }
}

impl fmt::Display for Applicability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Applicability::Applicable => write!(f, "applicable"),
            Applicability::NotApplicable => write!(f, "not applicable"),
            Applicability::TBD => write!(f, "TBD"),
        }
    }
}

/// The kind/purpose of a clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum ClauseKind {
    /// Operational clause (normative).
    #[default]
    Operational,
    /// Test clause.
    Test,
    /// Informative clause (non-normative).
    Informative,
}

impl fmt::Display for ClauseKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClauseKind::Operational => write!(f, "operational"),
            ClauseKind::Test => write!(f, "test"),
            ClauseKind::Informative => write!(f, "informative"),
        }
    }
}

/// A reference to a clause in a standard document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClauseRef {
    /// The name/identifier of the standard (e.g., "ISO 26262").
    pub standard: String,
    /// The edition or version of the standard (optional).
    pub edition: Option<String>,
    /// The clause identifier (e.g., "5.4.3").
    pub clause_id: String,
}

impl ClauseRef {
    /// Create a new clause reference.
    pub fn new(standard: impl Into<String>, clause_id: impl Into<String>) -> Self {
        ClauseRef {
            standard: standard.into(),
            edition: None,
            clause_id: clause_id.into(),
        }
    }

    /// Create a new clause reference with edition.
    pub fn with_edition(
        standard: impl Into<String>,
        edition: impl Into<String>,
        clause_id: impl Into<String>,
    ) -> Self {
        ClauseRef {
            standard: standard.into(),
            edition: Some(edition.into()),
            clause_id: clause_id.into(),
        }
    }
}

impl fmt::Display for ClauseRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(edition) = &self.edition {
            write!(f, "{} ({}) §{}", self.standard, edition, self.clause_id)
        } else {
            write!(f, "{} §{}", self.standard, self.clause_id)
        }
    }
}

/// A flexible value type for element properties.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Value {
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Int(i64),
    /// Floating-point value.
    Float(f64),
    /// Complex number (re + im*i).
    Complex { re: f64, im: f64 },
    /// Physical quantity with ISQ dimensional analysis.
    ///
    /// Carries a numeric value, its ISQ dimension vector (for dimensional
    /// analysis during arithmetic), and an optional unit name for display.
    Quantity {
        value: f64,
        dimension: DimensionVector,
        unit: Option<String>,
    },
    /// String value.
    String(String),
    /// Enumeration value (stored as string).
    Enum(String),
    /// Reference to another element.
    Ref(ElementId),
    /// List of values.
    List(Vec<Value>),
    /// Map of key-value pairs.
    Map(BTreeMap<String, Value>),
    /// Null/empty value.
    #[default]
    Null,
}

impl Value {
    /// Feed this value's full content into a hasher, canonically.
    ///
    /// `Value` can't derive `Hash` (it carries `f64`s), so this is the
    /// one home for its content-hash rule: discriminant + payload,
    /// floats via `to_bits` (so `-0.0 != 0.0` and NaN bit patterns are
    /// distinguished — fine for change-detection, where a false
    /// "changed" costs a recompute and a false "unchanged" serves stale
    /// data). Feeds [`crate::Element::content_hash`] /
    /// [`crate::Relationship::content_hash`] and, through them, the
    /// salsa change-detection fingerprint
    /// (`ModelGraph::fingerprint`) — see the 2026-07-16 staleness bug
    /// where doc/value-only edits were invisible to a name+kind-only
    /// fingerprint and got backdated as "unchanged".
    pub fn content_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use std::hash::Hash;
        std::mem::discriminant(self).hash(state);
        match self {
            Value::Bool(b) => b.hash(state),
            Value::Int(i) => i.hash(state),
            Value::Float(f) => f.to_bits().hash(state),
            Value::Complex { re, im } => {
                re.to_bits().hash(state);
                im.to_bits().hash(state);
            }
            Value::Quantity {
                value,
                dimension,
                unit,
            } => {
                value.to_bits().hash(state);
                dimension.hash(state);
                unit.hash(state);
            }
            Value::String(s) => s.hash(state),
            Value::Enum(s) => s.hash(state),
            Value::Ref(id) => id.hash(state),
            Value::List(items) => {
                items.len().hash(state);
                for item in items {
                    item.content_hash(state);
                }
            }
            Value::Map(map) => {
                map.len().hash(state);
                for (k, v) in map {
                    k.hash(state);
                    v.content_hash(state);
                }
            }
            Value::Null => {}
        }
    }

    /// Create a null value.
    pub fn null() -> Self {
        Value::Null
    }

    /// Check if this is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Try to get as bool.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get as integer.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Try to get as float. Integers and quantities are automatically converted.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            Value::Quantity { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// Try to get as a quantity (value, dimension, unit).
    pub fn as_quantity(&self) -> Option<(f64, &DimensionVector, Option<&str>)> {
        match self {
            Value::Quantity {
                value,
                dimension,
                unit,
            } => Some((*value, dimension, unit.as_deref())),
            _ => None,
        }
    }

    /// Create a quantity value with the given dimension and optional unit name.
    pub fn quantity(value: f64, dimension: DimensionVector, unit: Option<String>) -> Self {
        Value::Quantity {
            value,
            dimension,
            unit,
        }
    }

    /// Try to get as string. Works for both String and Enum values.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            Value::Enum(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get as element reference.
    pub fn as_ref(&self) -> Option<&ElementId> {
        match self {
            Value::Ref(id) => Some(id),
            _ => None,
        }
    }

    /// Try to get as list.
    pub fn as_list(&self) -> Option<&Vec<Value>> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    /// Try to get as map.
    pub fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    /// Get the type name of this value.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Complex { .. } => "complex",
            Value::Quantity { .. } => "quantity",
            Value::String(_) => "string",
            Value::Enum(_) => "enum",
            Value::Ref(_) => "ref",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Null => "null",
        }
    }

    /// Compare two values numerically.
    pub fn partial_cmp_value(&self, other: &Value) -> Option<Ordering> {
        match (self.as_float(), other.as_float()) {
            (Some(a), Some(b)) => a.partial_cmp(&b),
            _ => None,
        }
    }

    /// Check if this value is numerically less than another.
    pub fn is_less_than(&self, other: &Value) -> Option<bool> {
        self.partial_cmp_value(other).map(|o| o == Ordering::Less)
    }

    /// Check if this value is numerically greater than another.
    pub fn is_greater_than(&self, other: &Value) -> Option<bool> {
        self.partial_cmp_value(other)
            .map(|o| o == Ordering::Greater)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{}", b),
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::Complex { re, im } => {
                if *im >= 0.0 {
                    write!(f, "{}+{}i", re, im)
                } else {
                    write!(f, "{}{}i", re, im)
                }
            }
            Value::Quantity {
                value,
                dimension,
                unit,
            } => {
                if let Some(u) = unit {
                    write!(f, "{} [{}]", value, u)
                } else if dimension.is_zero() {
                    write!(f, "{}", value)
                } else {
                    write!(f, "{} [{}]", value, dimension)
                }
            }
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Enum(e) => write!(f, "{}", e),
            Value::Ref(id) => write!(f, "@{}", id),
            Value::List(l) => {
                write!(f, "[")?;
                for (i, v) in l.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Map(m) => {
                write!(f, "{{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Null => write!(f, "null"),
        }
    }
}

// Convenience From implementations
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}

impl From<i32> for Value {
    fn from(i: i32) -> Self {
        Value::Int(i as i64)
    }
}

impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::Float(f)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_owned())
    }
}

impl From<ElementId> for Value {
    fn from(id: ElementId) -> Self {
        Value::Ref(id)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::List(v.into_iter().map(|x| x.into()).collect())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(v) => v.into(),
            None => Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applicability_checks() {
        assert!(Applicability::Applicable.is_applicable());
        assert!(Applicability::NotApplicable.is_not_applicable());
        assert!(Applicability::TBD.is_tbd());
    }

    #[test]
    fn applicability_display() {
        assert_eq!(Applicability::Applicable.to_string(), "applicable");
        assert_eq!(Applicability::NotApplicable.to_string(), "not applicable");
        assert_eq!(Applicability::TBD.to_string(), "TBD");
    }

    #[test]
    fn clause_ref_basic() {
        let clause = ClauseRef::new("ISO 26262", "5.4.3");
        assert_eq!(clause.to_string(), "ISO 26262 §5.4.3");
    }

    #[test]
    fn clause_ref_with_edition() {
        let clause = ClauseRef::with_edition("ISO 26262", "2018", "5.4.3");
        assert_eq!(clause.to_string(), "ISO 26262 (2018) §5.4.3");
    }

    #[test]
    fn value_bool() {
        let v = Value::Bool(true);
        assert_eq!(v.as_bool(), Some(true));
        assert_eq!(v.type_name(), "bool");
    }

    #[test]
    fn value_int() {
        let v = Value::Int(42);
        assert_eq!(v.as_int(), Some(42));
        assert_eq!(v.as_float(), Some(42.0));
    }

    #[test]
    fn value_string() {
        let v = Value::String("hello".to_string());
        assert_eq!(v.as_str(), Some("hello"));
    }

    #[test]
    fn value_list() {
        let v = Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        assert!(v.as_list().is_some());
        assert_eq!(v.as_list().unwrap().len(), 3);
    }

    #[test]
    fn value_map() {
        let mut map = BTreeMap::new();
        map.insert("key".to_string(), Value::Int(42));
        let v = Value::Map(map);
        assert!(v.as_map().is_some());
    }

    #[test]
    fn value_from_conversions() {
        let v: Value = true.into();
        assert!(matches!(v, Value::Bool(true)));

        let v: Value = 42i64.into();
        assert!(matches!(v, Value::Int(42)));

        let v: Value = "hello".into();
        assert!(matches!(v, Value::String(_)));
    }

    #[test]
    fn value_display() {
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::Int(42).to_string(), "42");
        assert_eq!(Value::String("hello".into()).to_string(), "\"hello\"");
        assert_eq!(Value::Null.to_string(), "null");
    }

    #[test]
    fn clause_kind_display() {
        assert_eq!(ClauseKind::Operational.to_string(), "operational");
        assert_eq!(ClauseKind::Test.to_string(), "test");
        assert_eq!(ClauseKind::Informative.to_string(), "informative");
    }

    #[test]
    fn value_partial_cmp_int() {
        let a = Value::Int(10);
        let b = Value::Int(20);
        assert_eq!(a.partial_cmp_value(&b), Some(Ordering::Less));
        assert_eq!(b.partial_cmp_value(&a), Some(Ordering::Greater));
        assert_eq!(a.partial_cmp_value(&a), Some(Ordering::Equal));
    }

    #[test]
    fn value_partial_cmp_float() {
        let a = Value::Float(3.14);
        let b = Value::Float(2.71);
        assert_eq!(a.partial_cmp_value(&b), Some(Ordering::Greater));
    }

    #[test]
    fn value_partial_cmp_mixed() {
        let a = Value::Int(10);
        let b = Value::Float(10.5);
        assert_eq!(a.partial_cmp_value(&b), Some(Ordering::Less));
    }

    #[test]
    fn value_partial_cmp_non_numeric() {
        let a = Value::String("hello".to_string());
        let b = Value::Int(10);
        assert_eq!(a.partial_cmp_value(&b), None);
    }

    #[test]
    fn value_comparison_helpers() {
        let a = Value::Int(10);
        let b = Value::Int(20);

        assert_eq!(a.is_less_than(&b), Some(true));
        assert_eq!(a.is_greater_than(&b), Some(false));
        assert_eq!(b.is_greater_than(&a), Some(true));
    }

    #[test]
    fn value_quantity_basic() {
        use crate::physics::DimensionVector;

        let length = DimensionVector::new(1, 0, 0, 0, 0, 0, 0);
        let v = Value::quantity(3.14, length, Some("m".to_string()));
        assert_eq!(v.type_name(), "quantity");
        assert_eq!(v.as_float(), Some(3.14));
        assert!(!v.is_null());

        let (val, dim, unit) = v.as_quantity().unwrap();
        assert_eq!(val, 3.14);
        assert_eq!(*dim, length);
        assert_eq!(unit, Some("m"));
    }

    #[test]
    fn value_quantity_display() {
        use crate::physics::DimensionVector;

        let length = DimensionVector::new(1, 0, 0, 0, 0, 0, 0);
        let v = Value::quantity(9.81, length, Some("m/s²".to_string()));
        assert_eq!(v.to_string(), "9.81 [m/s²]");

        // Without unit name, falls back to dimension
        let v2 = Value::quantity(9.81, length, None);
        assert_eq!(v2.to_string(), "9.81 [L]");

        // Dimensionless quantity displays as plain number
        let v3 = Value::quantity(3.14, DimensionVector::default(), None);
        assert_eq!(v3.to_string(), "3.14");
    }

    #[test]
    fn value_quantity_comparison() {
        use crate::physics::DimensionVector;

        let length = DimensionVector::new(1, 0, 0, 0, 0, 0, 0);
        let a = Value::quantity(5.0, length, Some("m".to_string()));
        let b = Value::quantity(10.0, length, Some("m".to_string()));

        assert_eq!(a.partial_cmp_value(&b), Some(Ordering::Less));
        assert_eq!(a.is_less_than(&b), Some(true));

        // Quantity compared with Float uses numeric value
        let c = Value::Float(7.5);
        assert_eq!(a.partial_cmp_value(&c), Some(Ordering::Less));
    }
}
