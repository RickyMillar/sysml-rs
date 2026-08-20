//! SI unit conversion table and ConvertQuantity implementation.
//!
//! Maps unit names to their ISQ dimension vectors and conversion factors
//! relative to coherent SI base units. Conversion between any two compatible
//! units goes through the SI base: source → SI → target.

use sysml_core::physics::DimensionVector;

/// A unit entry: dimension vector, scale factor to SI base, and offset (for
/// temperature scales like Celsius).
///
/// Conversion from this unit to SI:  `si_value = value * scale + offset`
/// Conversion from SI to this unit:  `value = (si_value - offset) / scale`
#[derive(Debug, Clone, Copy)]
pub struct UnitEntry {
    pub dimension: DimensionVector,
    pub scale: f64,
    pub offset: f64,
}

impl UnitEntry {
    const fn new(dimension: DimensionVector, scale: f64) -> Self {
        Self {
            dimension,
            scale,
            offset: 0.0,
        }
    }

    const fn with_offset(dimension: DimensionVector, scale: f64, offset: f64) -> Self {
        Self {
            dimension,
            scale,
            offset,
        }
    }
}

// Dimension vector constants for readability
const DIMLESS: DimensionVector = DimensionVector::new(0, 0, 0, 0, 0, 0, 0);
const LENGTH: DimensionVector = DimensionVector::new(1, 0, 0, 0, 0, 0, 0);
const MASS: DimensionVector = DimensionVector::new(0, 1, 0, 0, 0, 0, 0);
const TIME: DimensionVector = DimensionVector::new(0, 0, 1, 0, 0, 0, 0);
const CURRENT: DimensionVector = DimensionVector::new(0, 0, 0, 1, 0, 0, 0);
const TEMP: DimensionVector = DimensionVector::new(0, 0, 0, 0, 1, 0, 0);
const AMOUNT: DimensionVector = DimensionVector::new(0, 0, 0, 0, 0, 1, 0);
const LUMINOUS: DimensionVector = DimensionVector::new(0, 0, 0, 0, 0, 0, 1);
const AREA: DimensionVector = DimensionVector::new(2, 0, 0, 0, 0, 0, 0);
const VOLUME: DimensionVector = DimensionVector::new(3, 0, 0, 0, 0, 0, 0);
const VELOCITY: DimensionVector = DimensionVector::new(1, 0, -1, 0, 0, 0, 0);
const ACCEL: DimensionVector = DimensionVector::new(1, 0, -2, 0, 0, 0, 0);
const FORCE: DimensionVector = DimensionVector::new(1, 1, -2, 0, 0, 0, 0);
const ENERGY: DimensionVector = DimensionVector::new(2, 1, -2, 0, 0, 0, 0);
const POWER: DimensionVector = DimensionVector::new(2, 1, -3, 0, 0, 0, 0);
const PRESSURE: DimensionVector = DimensionVector::new(-1, 1, -2, 0, 0, 0, 0);
const FREQUENCY: DimensionVector = DimensionVector::new(0, 0, -1, 0, 0, 0, 0);
const VOLTAGE: DimensionVector = DimensionVector::new(2, 1, -3, -1, 0, 0, 0);
const RESISTANCE: DimensionVector = DimensionVector::new(2, 1, -3, -2, 0, 0, 0);
const CAPACITANCE: DimensionVector = DimensionVector::new(-2, -1, 4, 2, 0, 0, 0);
const INDUCTANCE: DimensionVector = DimensionVector::new(2, 1, -2, -2, 0, 0, 0);
const CHARGE: DimensionVector = DimensionVector::new(0, 0, 1, 1, 0, 0, 0);
const MAG_FLUX: DimensionVector = DimensionVector::new(2, 1, -2, -1, 0, 0, 0);
const MAG_FLUX_DENSITY: DimensionVector = DimensionVector::new(0, 1, -2, -1, 0, 0, 0);
const CONDUCTANCE: DimensionVector = DimensionVector::new(-2, -1, 3, 2, 0, 0, 0);
const DENSITY: DimensionVector = DimensionVector::new(-3, 1, 0, 0, 0, 0, 0);
const TORQUE: DimensionVector = DimensionVector::new(2, 1, -2, 0, 0, 0, 0); // same as energy
const DOSE: DimensionVector = DimensionVector::new(2, 0, -2, 0, 0, 0, 0); // J/kg = m²·s⁻²
const ILLUMINANCE: DimensionVector = DimensionVector::new(-2, 0, 0, 0, 0, 0, 1); // lm/m² = cd·sr/m²
                                                                                 // --- Additional dimensions for compound derived units ---
const CATALYTIC: DimensionVector = DimensionVector::new(0, 0, -1, 0, 0, 1, 0); // mol·s⁻¹ = katal
const ELEC_FIELD: DimensionVector = DimensionVector::new(1, 1, -3, -1, 0, 0, 0); // V/m
const PERMITTIVITY: DimensionVector = DimensionVector::new(-3, -1, 4, 2, 0, 0, 0); // F/m
const PERMEABILITY: DimensionVector = DimensionVector::new(1, 1, -2, -2, 0, 0, 0); // H/m
const HEAT_CAPACITY: DimensionVector = DimensionVector::new(2, 1, -2, 0, -1, 0, 0); // J/K
const THERMAL_RES: DimensionVector = DimensionVector::new(-2, -1, 3, 0, 1, 0, 0); // K/W
const THERMAL_COND: DimensionVector = DimensionVector::new(2, 1, -3, 0, -1, 0, 0); // W/K
const SEEBECK: DimensionVector = DimensionVector::new(2, 1, -3, -1, -1, 0, 0); // V/K
const LINEAR_CHARGE: DimensionVector = DimensionVector::new(-1, 0, 1, 1, 0, 0, 0); // C/m
const MAG_FIELD_STRENGTH: DimensionVector = DimensionVector::new(-1, 0, 0, 1, 0, 0, 0); // A/m
const CONDUCTIVITY: DimensionVector = DimensionVector::new(-3, -1, 3, 2, 0, 0, 0); // S/m

/// Static table of known units. Searched linearly (table is small enough).
///
/// Each entry: (name, UnitEntry).
/// name matches SysML v2 standard library unit names (SI.sysml, USCustomaryUnits.sysml).
static UNIT_TABLE: &[(&str, UnitEntry)] = &[
    // === SI base units ===
    ("m", UnitEntry::new(LENGTH, 1.0)),
    ("metre", UnitEntry::new(LENGTH, 1.0)),
    ("meter", UnitEntry::new(LENGTH, 1.0)),
    ("kg", UnitEntry::new(MASS, 1.0)),
    ("kilogram", UnitEntry::new(MASS, 1.0)),
    ("s", UnitEntry::new(TIME, 1.0)),
    ("second", UnitEntry::new(TIME, 1.0)),
    ("A", UnitEntry::new(CURRENT, 1.0)),
    ("ampere", UnitEntry::new(CURRENT, 1.0)),
    ("K", UnitEntry::new(TEMP, 1.0)),
    ("kelvin", UnitEntry::new(TEMP, 1.0)),
    ("mol", UnitEntry::new(AMOUNT, 1.0)),
    ("mole", UnitEntry::new(AMOUNT, 1.0)),
    ("cd", UnitEntry::new(LUMINOUS, 1.0)),
    ("candela", UnitEntry::new(LUMINOUS, 1.0)),
    // === SI derived units ===
    ("Hz", UnitEntry::new(FREQUENCY, 1.0)),
    ("hertz", UnitEntry::new(FREQUENCY, 1.0)),
    ("N", UnitEntry::new(FORCE, 1.0)),
    ("newton", UnitEntry::new(FORCE, 1.0)),
    ("Pa", UnitEntry::new(PRESSURE, 1.0)),
    ("pascal", UnitEntry::new(PRESSURE, 1.0)),
    ("J", UnitEntry::new(ENERGY, 1.0)),
    ("joule", UnitEntry::new(ENERGY, 1.0)),
    ("W", UnitEntry::new(POWER, 1.0)),
    ("watt", UnitEntry::new(POWER, 1.0)),
    ("C", UnitEntry::new(CHARGE, 1.0)),
    ("coulomb", UnitEntry::new(CHARGE, 1.0)),
    ("V", UnitEntry::new(VOLTAGE, 1.0)),
    ("volt", UnitEntry::new(VOLTAGE, 1.0)),
    ("F", UnitEntry::new(CAPACITANCE, 1.0)),
    ("farad", UnitEntry::new(CAPACITANCE, 1.0)),
    ("\u{03A9}", UnitEntry::new(RESISTANCE, 1.0)), // Ω
    ("ohm", UnitEntry::new(RESISTANCE, 1.0)),
    ("S", UnitEntry::new(CONDUCTANCE, 1.0)),
    ("siemens", UnitEntry::new(CONDUCTANCE, 1.0)),
    ("Wb", UnitEntry::new(MAG_FLUX, 1.0)),
    ("weber", UnitEntry::new(MAG_FLUX, 1.0)),
    ("T", UnitEntry::new(MAG_FLUX_DENSITY, 1.0)),
    ("tesla", UnitEntry::new(MAG_FLUX_DENSITY, 1.0)),
    ("H", UnitEntry::new(INDUCTANCE, 1.0)),
    ("henry", UnitEntry::new(INDUCTANCE, 1.0)),
    // === SI prefixed length ===
    ("km", UnitEntry::new(LENGTH, 1e3)),
    ("kilometre", UnitEntry::new(LENGTH, 1e3)),
    ("cm", UnitEntry::new(LENGTH, 1e-2)),
    ("centimetre", UnitEntry::new(LENGTH, 1e-2)),
    ("mm", UnitEntry::new(LENGTH, 1e-3)),
    ("millimetre", UnitEntry::new(LENGTH, 1e-3)),
    ("\u{03BC}m", UnitEntry::new(LENGTH, 1e-6)), // μm
    ("um", UnitEntry::new(LENGTH, 1e-6)),
    ("nm", UnitEntry::new(LENGTH, 1e-9)),
    ("nanometre", UnitEntry::new(LENGTH, 1e-9)),
    // === SI prefixed mass ===
    ("g", UnitEntry::new(MASS, 1e-3)),
    ("gram", UnitEntry::new(MASS, 1e-3)),
    ("mg", UnitEntry::new(MASS, 1e-6)),
    ("milligram", UnitEntry::new(MASS, 1e-6)),
    ("\u{03BC}g", UnitEntry::new(MASS, 1e-9)), // μg
    ("ug", UnitEntry::new(MASS, 1e-9)),
    ("tonne", UnitEntry::new(MASS, 1e3)),
    // === SI prefixed time ===
    ("ms", UnitEntry::new(TIME, 1e-3)),
    ("millisecond", UnitEntry::new(TIME, 1e-3)),
    ("\u{03BC}s", UnitEntry::new(TIME, 1e-6)), // μs
    ("us", UnitEntry::new(TIME, 1e-6)),
    ("ns", UnitEntry::new(TIME, 1e-9)),
    ("nanosecond", UnitEntry::new(TIME, 1e-9)),
    ("min", UnitEntry::new(TIME, 60.0)),
    ("minute", UnitEntry::new(TIME, 60.0)),
    ("h", UnitEntry::new(TIME, 3600.0)),
    ("hour", UnitEntry::new(TIME, 3600.0)),
    // === SI prefixed energy/power ===
    ("kJ", UnitEntry::new(ENERGY, 1e3)),
    ("kilojoule", UnitEntry::new(ENERGY, 1e3)),
    ("MJ", UnitEntry::new(ENERGY, 1e6)),
    ("kW", UnitEntry::new(POWER, 1e3)),
    ("kilowatt", UnitEntry::new(POWER, 1e3)),
    ("MW", UnitEntry::new(POWER, 1e6)),
    // === SI prefixed pressure ===
    ("kPa", UnitEntry::new(PRESSURE, 1e3)),
    ("MPa", UnitEntry::new(PRESSURE, 1e6)),
    ("bar", UnitEntry::new(PRESSURE, 1e5)),
    // === SI prefixed frequency ===
    ("kHz", UnitEntry::new(FREQUENCY, 1e3)),
    ("MHz", UnitEntry::new(FREQUENCY, 1e6)),
    ("GHz", UnitEntry::new(FREQUENCY, 1e9)),
    // === SI prefixed voltage/current ===
    ("mV", UnitEntry::new(VOLTAGE, 1e-3)),
    ("kV", UnitEntry::new(VOLTAGE, 1e3)),
    ("mA", UnitEntry::new(CURRENT, 1e-3)),
    // === Temperature scales ===
    // Celsius: T_K = T_C + 273.15  →  si_value = value * 1.0 + 273.15
    ("\u{00B0}C", UnitEntry::with_offset(TEMP, 1.0, 273.15)),
    ("degC", UnitEntry::with_offset(TEMP, 1.0, 273.15)),
    ("Celsius", UnitEntry::with_offset(TEMP, 1.0, 273.15)),
    // Fahrenheit: T_K = (T_F + 459.67) * 5/9  →  si_value = value * 5/9 + 255.372...
    (
        "\u{00B0}F",
        UnitEntry::with_offset(TEMP, 5.0 / 9.0, 255.372_222_222_222_22),
    ),
    (
        "degF",
        UnitEntry::with_offset(TEMP, 5.0 / 9.0, 255.372_222_222_222_22),
    ),
    (
        "Fahrenheit",
        UnitEntry::with_offset(TEMP, 5.0 / 9.0, 255.372_222_222_222_22),
    ),
    // === Area / Volume ===
    ("m\u{00B2}", UnitEntry::new(AREA, 1.0)),
    ("m\u{00B3}", UnitEntry::new(VOLUME, 1.0)),
    ("L", UnitEntry::new(VOLUME, 1e-3)),
    ("litre", UnitEntry::new(VOLUME, 1e-3)),
    ("mL", UnitEntry::new(VOLUME, 1e-6)),
    // === Velocity / Acceleration ===
    ("m/s", UnitEntry::new(VELOCITY, 1.0)),
    ("km/h", UnitEntry::new(VELOCITY, 1.0 / 3.6)),
    ("m/s\u{00B2}", UnitEntry::new(ACCEL, 1.0)),
    // === Density ===
    ("kg/m\u{00B3}", UnitEntry::new(DENSITY, 1.0)),
    // === Torque ===
    ("N\u{00B7}m", UnitEntry::new(TORQUE, 1.0)),
    ("Nm", UnitEntry::new(TORQUE, 1.0)),
    // === US Customary ===
    ("in", UnitEntry::new(LENGTH, 0.0254)),
    ("inch", UnitEntry::new(LENGTH, 0.0254)),
    ("ft", UnitEntry::new(LENGTH, 0.3048)),
    ("foot", UnitEntry::new(LENGTH, 0.3048)),
    ("yd", UnitEntry::new(LENGTH, 0.9144)),
    ("yard", UnitEntry::new(LENGTH, 0.9144)),
    ("mi", UnitEntry::new(LENGTH, 1609.344)),
    ("mile", UnitEntry::new(LENGTH, 1609.344)),
    ("lb", UnitEntry::new(MASS, 0.453_592_37)),
    ("pound", UnitEntry::new(MASS, 0.453_592_37)),
    ("oz", UnitEntry::new(MASS, 0.028_349_523_125)),
    ("ounce", UnitEntry::new(MASS, 0.028_349_523_125)),
    ("lbf", UnitEntry::new(FORCE, 4.448_222)),
    ("psi", UnitEntry::new(PRESSURE, 6894.757)),
    ("mph", UnitEntry::new(VELOCITY, 0.447_04)),
    ("ft/s", UnitEntry::new(VELOCITY, 0.3048)),
    // === Additional SI prefixed ===
    ("mN", UnitEntry::new(FORCE, 1e-3)),
    ("millinewton", UnitEntry::new(FORCE, 1e-3)),
    ("GJ", UnitEntry::new(ENERGY, 1e9)),
    ("gigajoule", UnitEntry::new(ENERGY, 1e9)),
    // === Time (extended) ===
    ("d", UnitEntry::new(TIME, 86400.0)),
    ("day", UnitEntry::new(TIME, 86400.0)),
    // === Nuclear / Particle physics ===
    ("eV", UnitEntry::new(ENERGY, 1.602_176_487e-19)),
    ("electronvolt", UnitEntry::new(ENERGY, 1.602_176_487e-19)),
    ("Da", UnitEntry::new(MASS, 1.660_539_066_60e-27)),
    ("dalton", UnitEntry::new(MASS, 1.660_539_066_60e-27)),
    ("Bq", UnitEntry::new(FREQUENCY, 1.0)), // becquerel = s⁻¹
    ("becquerel", UnitEntry::new(FREQUENCY, 1.0)),
    // === Radiation dose ===
    ("Gy", UnitEntry::new(DOSE, 1.0)), // gray = J/kg
    ("gray", UnitEntry::new(DOSE, 1.0)),
    ("Sv", UnitEntry::new(DOSE, 1.0)), // sievert = J/kg
    ("sievert", UnitEntry::new(DOSE, 1.0)),
    // === Photometry ===
    ("sr", UnitEntry::new(DIMLESS, 1.0)), // steradian
    ("steradian", UnitEntry::new(DIMLESS, 1.0)),
    ("lm", UnitEntry::new(LUMINOUS, 1.0)), // lumen ≈ cd·sr (simplified)
    ("lumen", UnitEntry::new(LUMINOUS, 1.0)),
    ("lx", UnitEntry::new(ILLUMINANCE, 1.0)), // lux = lm/m²
    ("lux", UnitEntry::new(ILLUMINANCE, 1.0)),
    // === Reactive power ===
    ("var", UnitEntry::new(POWER, 1.0)), // volt-ampere reactive = W
    // === Dimensionless / Angle ===
    ("rad", UnitEntry::new(DIMLESS, 1.0)),
    ("radian", UnitEntry::new(DIMLESS, 1.0)),
    ("deg", UnitEntry::new(DIMLESS, std::f64::consts::PI / 180.0)),
    (
        "degree",
        UnitEntry::new(DIMLESS, std::f64::consts::PI / 180.0),
    ),
    ("arcmin", UnitEntry::new(DIMLESS, 2.908_882e-4)),
    ("\u{2032}", UnitEntry::new(DIMLESS, 2.908_882e-4)), // ′ prime
    ("arcsec", UnitEntry::new(DIMLESS, 4.848_137e-6)),
    ("\u{2033}", UnitEntry::new(DIMLESS, 4.848_137e-6)), // ″ double prime
    // === Rare / niche SI units ===
    ("\u{00C5}", UnitEntry::new(LENGTH, 1e-10)), // Å ångström
    ("angstrom", UnitEntry::new(LENGTH, 1e-10)),
    ("b", UnitEntry::new(AREA, 1e-28)), // barn (nuclear cross-section)
    ("u", UnitEntry::new(MASS, 1.660_539_066_60e-27)), // atomic mass unit (= dalton)
    ("ua", UnitEntry::new(LENGTH, 1.495_978_707e11)), // astronomical unit
    ("kat", UnitEntry::new(CATALYTIC, 1.0)), // katal (catalytic activity)
    ("katal", UnitEntry::new(CATALYTIC, 1.0)),
    // === Information units (dimensionless) ===
    ("B", UnitEntry::new(DIMLESS, 1.0)),    // byte
    ("Bd", UnitEntry::new(FREQUENCY, 1.0)), // baud (symbols/s)
    ("bit", UnitEntry::new(DIMLESS, 1.0)),  // bit
    ("Hart", UnitEntry::new(DIMLESS, 1.0)), // hartley
    ("nat", UnitEntry::new(DIMLESS, 1.0)),  // nat (natural unit of info)
    ("o", UnitEntry::new(DIMLESS, 1.0)),    // octet
    ("Sh", UnitEntry::new(DIMLESS, 1.0)),   // shannon
    // === Logarithmic / ratio (dimensionless — log scale not modeled) ===
    ("dB", UnitEntry::new(DIMLESS, 1.0)),  // decibel
    ("dec", UnitEntry::new(DIMLESS, 1.0)), // decade
    ("oct", UnitEntry::new(DIMLESS, 1.0)), // octave
    // === Traffic (dimensionless) ===
    ("E", UnitEntry::new(DIMLESS, 1.0)), // erlang
    // === Temperature interval ===
    ("\u{00B0}C_abs", UnitEntry::new(TEMP, 1.0)), // °C absolute (interval, no offset)
    // === Short compound derived units ===
    ("A/m", UnitEntry::new(MAG_FIELD_STRENGTH, 1.0)), // ampere per metre
    ("B/s", UnitEntry::new(FREQUENCY, 1.0)),          // byte per second (dimensionless rate)
    ("C/m", UnitEntry::new(LINEAR_CHARGE, 1.0)),      // coulomb per metre
    ("F/m", UnitEntry::new(PERMITTIVITY, 1.0)),       // farad per metre
    ("g/L", UnitEntry::new(DENSITY, 1.0)),            // gram per litre = kg/m³
    ("H/m", UnitEntry::new(PERMEABILITY, 1.0)),       // henry per metre
    ("J/K", UnitEntry::new(HEAT_CAPACITY, 1.0)),      // joule per kelvin
    ("J/m", UnitEntry::new(FORCE, 1.0)),              // joule per metre = newton
    ("J/s", UnitEntry::new(POWER, 1.0)),              // joule per second = watt
    ("K/W", UnitEntry::new(THERMAL_RES, 1.0)),        // kelvin per watt
    ("o/s", UnitEntry::new(FREQUENCY, 1.0)),          // octet per second
    ("S/m", UnitEntry::new(CONDUCTIVITY, 1.0)),       // siemens per metre
    ("V/K", UnitEntry::new(SEEBECK, 1.0)),            // volt per kelvin
    ("V/m", UnitEntry::new(ELEC_FIELD, 1.0)),         // volt per metre
    ("W/K", UnitEntry::new(THERMAL_COND, 1.0)),       // watt per kelvin
];

/// Look up a unit by name. Returns the `UnitEntry` if found.
pub fn lookup_unit(name: &str) -> Option<&'static UnitEntry> {
    UNIT_TABLE
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, entry)| entry)
}

/// Convert a magnitude between two units given their SI affine parameters
/// (`magnitude * scale + offset = SI value`).
///
/// This is the single arithmetic home for unit conversion: both the
/// name-keyed [`convert_quantity`] (eval-time `ConvertQuantity`) and the
/// mRef-keyed RSC-5.3 boundary auto-conversion (binding/flow endpoints, where
/// both `MeasurementRef`s are already resolved at compile time) compose it.
/// The full affine form handles offset units (e.g. °C↔K) — D-5.0.4's "convert
/// by `scale` ratio" is shorthand for this conversion, not a linear-only
/// constraint (the spec's `IntervalScale`/`QuantityValueMapping` mandates the
/// affine case, `MeasurementReferences.sysml`).
///
/// The caller guarantees the two units share dimension.
pub fn convert_magnitude(
    value: f64,
    src_scale: f64,
    src_offset: f64,
    tgt_scale: f64,
    tgt_offset: f64,
) -> f64 {
    let si_value = value * src_scale + src_offset;
    (si_value - tgt_offset) / tgt_scale
}

/// Convert a quantity value from one unit to another.
///
/// Conversion goes through SI base units:
///   source_value → SI value → target value
///
/// Returns `Err` if dimensions don't match.
pub fn convert_quantity(
    value: f64,
    source_dim: &DimensionVector,
    source_unit: Option<&str>,
    target_unit: &str,
) -> Result<(f64, DimensionVector, String), String> {
    let target = lookup_unit(target_unit).ok_or_else(|| format!("unknown unit: {target_unit}"))?;

    // If source has a known unit, convert through SI
    if let Some(src_name) = source_unit {
        if let Some(src) = lookup_unit(src_name) {
            if src.dimension != target.dimension {
                return Err(format!(
                    "cannot convert {} ({}) to {} ({})",
                    src_name, src.dimension, target_unit, target.dimension
                ));
            }
            // source → SI → target (the one conversion-math home)
            let target_value =
                convert_magnitude(value, src.scale, src.offset, target.scale, target.offset);
            return Ok((target_value, target.dimension, target_unit.to_string()));
        }
    }

    // No known source unit — check dimension compatibility and assume SI base
    if !source_dim.is_zero() && *source_dim != target.dimension {
        return Err(format!(
            "dimension mismatch: source has {} but target {} has {}",
            source_dim, target_unit, target.dimension
        ));
    }

    // Source is in SI base units (scale=1, offset=0)
    let target_value = convert_magnitude(value, 1.0, 0.0, target.scale, target.offset);
    Ok((target_value, target.dimension, target_unit.to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn lookup_si_base_units() {
        assert!(lookup_unit("m").is_some());
        assert!(lookup_unit("kg").is_some());
        assert!(lookup_unit("s").is_some());
        assert!(lookup_unit("A").is_some());
        assert!(lookup_unit("K").is_some());
        assert!(lookup_unit("mol").is_some());
        assert!(lookup_unit("cd").is_some());
    }

    #[test]
    fn lookup_derived_units() {
        let n = lookup_unit("N").unwrap();
        assert_eq!(n.dimension, FORCE);
        assert_eq!(n.scale, 1.0);

        let v = lookup_unit("V").unwrap();
        assert_eq!(v.dimension, VOLTAGE);
    }

    #[test]
    fn lookup_prefixed_units() {
        let km = lookup_unit("km").unwrap();
        assert_eq!(km.dimension, LENGTH);
        assert_eq!(km.scale, 1e3);

        let ms = lookup_unit("ms").unwrap();
        assert_eq!(ms.dimension, TIME);
        assert_eq!(ms.scale, 1e-3);
    }

    #[test]
    fn convert_km_to_m() {
        let (val, dim, unit) = convert_quantity(5.0, &LENGTH, Some("km"), "m").unwrap();
        assert!((val - 5000.0).abs() < 1e-10);
        assert_eq!(dim, LENGTH);
        assert_eq!(unit, "m");
    }

    #[test]
    fn convert_m_to_ft() {
        let (val, _, _) = convert_quantity(1.0, &LENGTH, Some("m"), "ft").unwrap();
        assert!((val - 3.280_839_895).abs() < 1e-6);
    }

    #[test]
    fn convert_celsius_to_kelvin() {
        let (val, _, _) = convert_quantity(100.0, &TEMP, Some("degC"), "K").unwrap();
        assert!((val - 373.15).abs() < 1e-10);
    }

    #[test]
    fn convert_kelvin_to_celsius() {
        let (val, _, _) = convert_quantity(273.15, &TEMP, Some("K"), "degC").unwrap();
        assert!(val.abs() < 1e-10);
    }

    #[test]
    fn convert_fahrenheit_to_celsius() {
        // 212°F = 100°C
        let (val, _, _) = convert_quantity(212.0, &TEMP, Some("degF"), "degC").unwrap();
        assert!((val - 100.0).abs() < 0.01);
    }

    #[test]
    fn convert_dimension_mismatch() {
        let result = convert_quantity(1.0, &LENGTH, Some("m"), "kg");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot convert"));
    }

    #[test]
    fn convert_unknown_unit() {
        let result = convert_quantity(1.0, &LENGTH, Some("m"), "furlongs");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown unit"));
    }

    #[test]
    fn convert_from_si_base_no_source_unit() {
        // Value in SI (metres), convert to km
        let (val, _, _) = convert_quantity(5000.0, &LENGTH, None, "km").unwrap();
        assert!((val - 5.0).abs() < 1e-10);
    }

    #[test]
    fn convert_magnitude_is_the_arithmetic_home() {
        // mA → A (linear, both scale-only). 5 mA = 0.005 A.
        let ma = lookup_unit("mA").unwrap();
        let a = lookup_unit("A").unwrap();
        let v = convert_magnitude(5.0, ma.scale, ma.offset, a.scale, a.offset);
        assert!((v - 0.005).abs() < 1e-12, "5 mA = 0.005 A, got {v}");
        // A → mA round-trips.
        let back = convert_magnitude(v, a.scale, a.offset, ma.scale, ma.offset);
        assert!(
            (back - 5.0).abs() < 1e-9,
            "round-trip back to 5 mA, got {back}"
        );
        // Identity: same (scale, offset) is a no-op.
        assert_eq!(convert_magnitude(42.0, 1.0, 0.0, 1.0, 0.0), 42.0);
        // Affine: 100 °C = 373.15 K (offset carried, not just scale ratio).
        let degc = lookup_unit("degC").unwrap();
        let k = lookup_unit("K").unwrap();
        let kv = convert_magnitude(100.0, degc.scale, degc.offset, k.scale, k.offset);
        assert!((kv - 373.15).abs() < 1e-9, "100 °C = 373.15 K, got {kv}");
    }

    #[test]
    fn convert_us_customary() {
        // 1 mile = 1609.344 m
        let (val, _, _) = convert_quantity(1.0, &LENGTH, Some("mi"), "m").unwrap();
        assert!((val - 1609.344).abs() < 1e-6);
    }

    #[test]
    fn convert_pressure() {
        // 1 bar = 100000 Pa
        let (val, _, _) = convert_quantity(1.0, &PRESSURE, Some("bar"), "Pa").unwrap();
        assert!((val - 100_000.0).abs() < 1e-6);
    }

    #[test]
    fn convert_angle_deg_to_rad() {
        let (val, _, _) = convert_quantity(180.0, &DIMLESS, Some("deg"), "rad").unwrap();
        assert!((val - std::f64::consts::PI).abs() < 1e-10);
    }
}
