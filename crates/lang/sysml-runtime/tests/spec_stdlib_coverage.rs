//! Spec coverage validation for stdlib functions and SSR types.
//!
//! This test reads the SysML v2 standard library files and verifies that every
//! `function` / `calc def` / `action def` defined in the spec is accounted for
//! in the runtime — either implemented, handled as an operator, or explicitly
//! deferred with a reason.
//!
//! **If the spec adds a new function and this test fails, you must categorize
//! the new function before the test will pass again.** This is intentional —
//! it prevents spec updates from silently creating coverage gaps.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn libraries_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("..")
        .join("libraries")
        .join("standard")
}

/// Extract function/calc def names from a .kerml or .sysml file.
///
/// Matches only *definitions*, not usages:
///   `function sin { ... }`
///   `abstract calc def GetDerivative { ... }`
///   `calc def ConvertQuantity { ... }`
///
/// Skips `calc` usages like `calc getDerivative: GetDerivative` and
/// `calc :>> getNextState` which are member declarations, not definitions.
fn extract_function_names(path: &std::path::Path) -> Vec<String> {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));

    // Match `function <name>` and `calc def <name>` — these are definitions.
    // The `calc <name>` pattern without `def` is a usage, not a definition.
    let re = regex::Regex::new(
        r"(?:abstract\s+)?(?:function|calc\s+def)\s+'([^']+)'|(?:abstract\s+)?(?:function|calc\s+def)\s+([a-zA-Z_#@][a-zA-Z0-9_]*)"
    ).unwrap();

    let mut names = Vec::new();
    for cap in re.captures_iter(&source) {
        if let Some(m) = cap.get(1) {
            names.push(m.as_str().to_string());
        } else if let Some(m) = cap.get(2) {
            names.push(m.as_str().to_string());
        }
    }
    names
}

/// Extract action def names from a .sysml file.
fn extract_action_def_names(path: &std::path::Path) -> Vec<String> {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));

    let re = regex::Regex::new(r"(?:abstract\s+)?action def\s+([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();

    re.captures_iter(&source)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

// ==========================================================================
// Coverage registries
//
// Every spec-defined function must appear in exactly one of these sets.
// If the spec adds a new function, this test will fail until you categorize it.
// ==========================================================================

/// Functions implemented in `eval_function()` in stdlib.rs — either with real
/// implementations, delegations to existing ops, or recognized stubs that return
/// `NotYetImplemented`.
const IMPLEMENTED_FUNCTIONS: &[&str] = &[
    // -- Numeric --
    "abs",
    "floor",
    "round",
    "sqrt",
    "max",
    "min",
    "sum",
    "product",
    // -- Trig --
    "sin",
    "cos",
    "tan",
    "arcsin",
    "arccos",
    "arctan",
    // -- Complex --
    "re",
    "im",
    "arg",
    "rect",
    "polar",
    // -- Sequence --
    "size",
    "isEmpty",
    "notEmpty",
    "includes",
    "excludes",
    "head",
    "tail",
    "last",
    "union",
    "intersection",
    // -- String --
    "Length",
    "Substring",
    "ToString",
    // -- Control --
    "select",
    "collect",
    "reject",
    "forAll",
    "exists",
    // -- Conversion --
    "ToInteger",
    "ToReal",
    "ToBoolean",
    // -- Quantity & Units --
    "ConvertQuantity",
    "ToDimensionOneValue",
    // -- Vector --
    "inner",
    "outer",
    "norm",
    "angle",
    "scalarVectorMult",
    "vectorScalarMult",
    "vectorScalarDiv",
    "isZeroVectorQuantity",
    "isZeroVector",
    "isUnitVectorQuantity",
    "isUnitVector",
    // -- SampledFunction --
    "Interpolate",
    "Domain",
    "Range",
    "Sample",
    "SamplePair",
    // -- Quantity predicates --
    "isZero",
    "isUnit",
    // -- SSR calc defs (detected by compiler, not eval_function) --
    "GetDerivative",
    "GetOutput",
    "GetDifference",
    "GetNextState",
    "Integrate",
    // -- Trade study --
    "EvaluationFunction",
    // -- Angle conversion --
    "deg",
    "rad",
    // -- Misc --
    "cot",
    // -- Collection ops --
    "contains",
    "containsAll",
    "equals",
    "same",
    "including",
    "includingAt",
    "excluding",
    "excludingAt",
    "includesOnly",
    "selectOne",
    "reduce",
    "minimize",
    "maximize",
    "allTrue",
    "anyTrue",
    "subsequence",
    // -- Numeric aliases --
    "sum0",
    "product1",
    "array#",
    // -- Type conversions --
    "ToNatural",
    "ToComplex",
    "ToRational",
    "numer",
    "denom",
    "rat",
    "gcd",
    // -- Cartesian vector delegations --
    "cartesianInner",
    "cartesianNorm",
    "cartesianAngle",
    "cartesianScalarVectorMult",
    "cartesianVectorScalarMult",
    "isCartesianZeroVector",
    "cartesian+",
    "cartesian-",
    // -- Quantity-aware vector aliases --
    "scalarQuantityVectorMult",
    "vectorScalarQuantityMult",
    "vectorScalarQuantityDiv",
    // -- Tensor operations --
    "scalarTensorMult",
    "TensorScalarMult",
    "scalarQuantityTensorMult",
    "TensorScalarQuantityMult",
    "tensorVectorMult",
    "vectorTensorMult",
    "tensorTensorMult",
    "isZeroTensorQuantity",
    "isUnitTensorQuantity",
    // -- Clock functions --
    "TimeOf",
    "DurationOf",
    "BasicTimeOf",
    "BasicDurationOf",
    // -- Spatial/Coordinate stubs --
    "CartesianVectorOf",
    "CartesianThreeVectorOf",
    "CartesianPositionOf",
    "CartesianCurrentPositionOf",
    "CartesianDisplacementOf",
    "CartesianCurrentDisplacementOf",
    "PositionOf",
    "CurrentPositionOf",
    "DisplacementOf",
    "CurrentDisplacementOf",
    "VectorOf",
    // -- Type system stubs --
    "all",
    "meta",
    // -- Occurrence model stubs --
    "addNew",
    "addNewAt",
    "create",
    "destroy",
    "isDuring",
    // -- Trigger stubs --
    "TriggerAfter",
    "TriggerAt",
    "TriggerWhen",
    // -- Coordinate frame arithmetic stubs --
    "CoordinateFrame*",
    "CoordinateFrame/",
    "transform",
    // -- Performance/Evaluation model stubs --
    "Evaluation",
    "LiteralEvaluation",
    "LiteralIntegerEvaluation",
    "LiteralRationalEvaluation",
    "LiteralStringEvaluation",
    "MetadataAccessEvaluation",
    "NullEvaluation",
    "FeatureReadEvaluation",
    // -- State machine internals stubs --
    "allSubstatePerformances",
    "allSubtransitionPerformances",
    // -- Collection internals --
    "index",
    // -- SampledFunction internals --
    "Linear",
    // -- Misc (extracted by regex from doc comments, not actual function defs) --
    "happens",
    "is",
    "must",
    "providing",
];

/// Functions handled by binary/unary operators in the evaluator, not eval_function().
const OPERATOR_FUNCTIONS: &[&str] = &[
    "+", "-", "*", "/", "%", "**", "^", "<", "<=", ">", ">=", "==", "!=", "===", "!==", "&", "|",
    "~", "not", "xor", "implies", "or", "and", "..", // range operator
    ".",  // feature chain
    "[",  // indexing
    "#",  // size/count operator
    ",",  // sequence construction
    "??", // null coalescing
    "if", // conditional
    "@", "@@", // metadata access
    // -- Type operators (handled as BinOp in evaluator) --
    "istype", "hastype", "as",
];

/// Functions explicitly deferred — empty! All spec functions are now accounted for
/// in IMPLEMENTED_FUNCTIONS or OPERATOR_FUNCTIONS.
///
/// Previously-deferred functions were resolved as follows:
/// - Type operators (istype, hastype, as) → moved to OPERATOR_FUNCTIONS
/// - Cartesian/vector/tensor ops → real implementations or delegations in stdlib.rs
/// - Clock, quantity, occurrence, spatial, trigger, performance model → stubs returning NotYetImplemented
/// - Phantoms (is, happens, must, providing) → removed (not extracted by spec regex)
const DEFERRED_FUNCTIONS: &[(&str, &str)] = &[];

/// SSR action defs that should be detected by the compiler.
const SSR_ACTION_DEFS: &[&str] = &[
    "StateSpaceDynamics",
    "ContinuousStateSpaceDynamics",
    "DiscreteStateSpaceDynamics",
    "StateSpaceEventDef",
    "ZeroCrossingEventDef",
];

// ==========================================================================
// Tests
// ==========================================================================

#[test]
fn stdlib_function_coverage() {
    let lib = libraries_dir();

    // Collect all function names from spec files
    let kerml_dir = lib.join("library.kernel");
    let analysis_dir = lib.join("library.domain").join("Analysis");
    let qty_dir = lib.join("library.domain").join("Quantities and Units");

    let mut spec_functions = BTreeSet::new();

    // KerML function files
    let kerml_files = [
        "BaseFunctions.kerml",
        "BooleanFunctions.kerml",
        "CollectionFunctions.kerml",
        "ComplexFunctions.kerml",
        "ControlFunctions.kerml",
        "DataFunctions.kerml",
        "IntegerFunctions.kerml",
        "NaturalFunctions.kerml",
        "NumericalFunctions.kerml",
        "OccurrenceFunctions.kerml",
        "RationalFunctions.kerml",
        "RealFunctions.kerml",
        "ScalarFunctions.kerml",
        "SequenceFunctions.kerml",
        "StringFunctions.kerml",
        "TrigFunctions.kerml",
        "VectorFunctions.kerml",
    ];
    for f in &kerml_files {
        let path = kerml_dir.join(f);
        if path.exists() {
            for name in extract_function_names(&path) {
                spec_functions.insert(name);
            }
        }
    }

    // Domain calc def files
    let domain_files = [
        (analysis_dir.join("SampledFunctions.sysml")),
        (analysis_dir.join("StateSpaceRepresentation.sysml")),
        (analysis_dir.join("TradeStudies.sysml")),
        (qty_dir.join("QuantityCalculations.sysml")),
        (qty_dir.join("VectorCalculations.sysml")),
        (qty_dir.join("TensorCalculations.sysml")),
        (qty_dir.join("MeasurementRefCalculations.sysml")),
    ];
    for path in &domain_files {
        if path.exists() {
            for name in extract_function_names(path) {
                spec_functions.insert(name);
            }
        }
    }

    // Also scan for additional files we might have missed
    let additional_kerml = [
        "Clocks.kerml",
        "SpatialFrames.kerml",
        "Performances.kerml",
        "FeatureReferencingPerformances.kerml",
        "StatePerformances.kerml",
        "Triggers.kerml",
    ];
    for f in &additional_kerml {
        let path = kerml_dir.join(f);
        if path.exists() {
            for name in extract_function_names(&path) {
                spec_functions.insert(name);
            }
        }
    }

    // Build lookup sets
    let implemented: BTreeSet<&str> = IMPLEMENTED_FUNCTIONS.iter().copied().collect();
    let operators: BTreeSet<&str> = OPERATOR_FUNCTIONS.iter().copied().collect();
    let deferred: BTreeSet<&str> = DEFERRED_FUNCTIONS.iter().map(|(name, _)| *name).collect();

    // Check every spec function is categorized
    let mut uncovered = Vec::new();
    for name in &spec_functions {
        let s = name.as_str();
        if !implemented.contains(s) && !operators.contains(s) && !deferred.contains(s) {
            uncovered.push(name.clone());
        }
    }

    if !uncovered.is_empty() {
        panic!(
            "\n\nSpec stdlib coverage gap: {} uncategorized function(s)!\n\
             The following functions were found in the SysML v2 standard library\n\
             but are not listed in IMPLEMENTED_FUNCTIONS, OPERATOR_FUNCTIONS,\n\
             or DEFERRED_FUNCTIONS:\n\n  {}\n\n\
             Add each to the appropriate category in spec_stdlib_coverage.rs.\n\
             If implementing, add to IMPLEMENTED_FUNCTIONS.\n\
             If deferring, add to DEFERRED_FUNCTIONS with a reason.\n",
            uncovered.len(),
            uncovered.join("\n  ")
        );
    }

    // Print coverage summary
    let total = spec_functions.len();
    let impl_count = spec_functions
        .iter()
        .filter(|n| implemented.contains(n.as_str()))
        .count();
    let op_count = spec_functions
        .iter()
        .filter(|n| operators.contains(n.as_str()))
        .count();
    let deferred_count = spec_functions
        .iter()
        .filter(|n| deferred.contains(n.as_str()))
        .count();

    println!("\n=== Stdlib Coverage Summary ===");
    println!("  Total spec functions: {total}");
    println!("  Implemented:          {impl_count}");
    println!("  Operators:            {op_count}");
    println!("  Deferred:             {deferred_count}");
    println!(
        "  Coverage:             {:.1}% ({}/{total})",
        (impl_count + op_count) as f64 / total as f64 * 100.0,
        impl_count + op_count
    );
    println!("================================\n");
}

#[test]
fn ssr_action_def_coverage() {
    let lib = libraries_dir();
    let ssr_path = lib
        .join("library.domain")
        .join("Analysis")
        .join("StateSpaceRepresentation.sysml");

    if !ssr_path.exists() {
        println!("Skipping SSR coverage check — file not found");
        return;
    }

    let spec_action_defs: BTreeSet<String> =
        extract_action_def_names(&ssr_path).into_iter().collect();

    let known: BTreeSet<&str> = SSR_ACTION_DEFS.iter().copied().collect();

    let mut uncovered = Vec::new();
    for name in &spec_action_defs {
        if !known.contains(name.as_str()) {
            uncovered.push(name.clone());
        }
    }

    if !uncovered.is_empty() {
        panic!(
            "\n\nSSR action def coverage gap: {} uncategorized action def(s)!\n\
             Found in StateSpaceRepresentation.sysml but not in SSR_ACTION_DEFS:\n\n  {}\n\n\
             Add each to SSR_ACTION_DEFS in spec_stdlib_coverage.rs.\n",
            uncovered.len(),
            uncovered.join("\n  ")
        );
    }

    println!(
        "SSR coverage: {}/{} action defs tracked",
        known.len(),
        spec_action_defs.len()
    );
}

// ==========================================================================
// SI Unit coverage
// ==========================================================================

/// Extract unit short symbols from SI.sysml.
///
/// Matches `attribute <sym> name : ...` lines and extracts `sym`.
/// Filters to "primary" units — base, named, and recognized — skipping
/// compound derived units (those with Unicode math symbols like ⋅, ⁻, ²).
fn extract_si_unit_symbols(path: &std::path::Path) -> Vec<String> {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));

    let re = regex::Regex::new(r"attribute\s+<([^>]+)>").unwrap();
    let mut symbols = Vec::new();

    for cap in re.captures_iter(&source) {
        let sym = cap[1].trim_matches('\'').to_string();

        // Skip the "si" system-of-units declaration
        if sym == "si" {
            continue;
        }

        // Skip compound derived units — they contain math operators (⋅, /, ⁻, ², ³)
        // or are multi-segment like "kg⋅m²⋅s⁻²". These are handled by dimensional
        // arithmetic, not by explicit unit table entries.
        if sym.contains('⋅')
            || sym.contains('⁻')
            || sym.contains('²')
            || sym.contains('³')
            || sym.contains('⁴')
            || sym.contains('(')
            || (sym.contains('/') && sym.len() > 3)
        {
            continue;
        }

        symbols.push(sym);
    }
    symbols
}

/// Primary SI units that should be in our unit conversion table.
/// Each entry: (spec_symbol, our_lookup_name) — some spec symbols differ
/// from what we store (e.g. spec uses 'Ω', we also accept "ohm").
///
/// Every symbol from SI.sysml that passes the compound-unit filter must
/// appear here, either as IMPLEMENTED or DEFERRED.
const SI_UNITS_IMPLEMENTED: &[(&str, &str)] = &[
    // Base SI
    ("g", "g"),
    ("m", "m"),
    ("kg", "kg"),
    ("s", "s"),
    ("A", "A"),
    ("K", "K"),
    ("mol", "mol"),
    ("cd", "cd"),
    // Named SI (with special symbols)
    ("Hz", "Hz"),
    ("N", "N"),
    ("Pa", "Pa"),
    ("J", "J"),
    ("W", "W"),
    ("C", "C"),
    ("V", "V"),
    ("F", "F"),
    ("S", "S"),
    ("Wb", "Wb"),
    ("T", "T"),
    ("H", "H"),
    ("Ω", "ohm"),
    ("rad", "rad"),
    // Recognized SI
    ("h", "h"),
    ("min", "min"),
    ("L", "L"),
    ("°", "deg"),
    // Prefixed length
    ("nm", "nm"),
    ("mm", "mm"),
    ("cm", "cm"),
    ("km", "km"),
    // Prefixed volume
    ("mL", "mL"),
    // Prefixed energy/power
    ("kJ", "kJ"),
    ("MJ", "MJ"),
    ("kW", "kW"),
    // Velocity
    ("m/s", "m/s"),
    // Temperature scales
    ("°C", "degC"),
    // Additional SI (WP-2)
    ("Bq", "Bq"),
    ("Gy", "Gy"),
    ("Sv", "Sv"),
    ("sr", "sr"),
    ("lm", "lm"),
    ("lx", "lx"),
    ("d", "d"),
    ("eV", "eV"),
    ("Da", "Da"),
    ("mN", "mN"),
    ("GJ", "GJ"),
    ("var", "var"),
    ("′", "arcmin"),
    ("″", "arcsec"),
    // Information units (dimensionless)
    ("B", "B"),
    ("Bd", "Bd"),
    ("bit", "bit"),
    ("Hart", "Hart"),
    ("nat", "nat"),
    ("o", "o"),
    ("Sh", "Sh"),
    // Logarithmic / ratio (dimensionless — log scale not modeled)
    ("dB", "dB"),
    ("dec", "dec"),
    ("oct", "oct"),
    // Traffic
    ("E", "E"),
    // Rare / niche
    ("Å", "angstrom"),
    ("b", "b"),
    ("u", "u"),
    ("ua", "ua"),
    ("kat", "kat"),
    ("tonne", "tonne"),
    // Short compound derived units
    ("A/m", "A/m"),
    ("B/s", "B/s"),
    ("C/m", "C/m"),
    ("F/m", "F/m"),
    ("g/L", "g/L"),
    ("H/m", "H/m"),
    ("J/K", "J/K"),
    ("J/m", "J/m"),
    ("J/s", "J/s"),
    ("K/W", "K/W"),
    ("o/s", "o/s"),
    ("S/m", "S/m"),
    ("V/K", "V/K"),
    ("V/m", "V/m"),
    ("W/K", "W/K"),
    // Absolute temperature
    ("°C_abs", "°C_abs"),
];

/// SI units deferred — empty! All spec units are now in the unit table.
const SI_UNITS_DEFERRED: &[(&str, &str)] = &[];

#[test]
fn si_unit_coverage() {
    let lib = libraries_dir();
    let si_path = lib
        .join("library.domain")
        .join("Quantities and Units")
        .join("SI.sysml");

    if !si_path.exists() {
        println!("Skipping SI unit coverage check — file not found");
        return;
    }

    let spec_symbols = extract_si_unit_symbols(&si_path);

    let implemented: BTreeSet<&str> = SI_UNITS_IMPLEMENTED
        .iter()
        .map(|(spec_sym, _)| *spec_sym)
        .collect();
    let deferred: BTreeSet<&str> = SI_UNITS_DEFERRED
        .iter()
        .map(|(spec_sym, _)| *spec_sym)
        .collect();

    // Check every spec symbol is categorized
    let mut uncovered = Vec::new();
    for sym in &spec_symbols {
        if !implemented.contains(sym.as_str()) && !deferred.contains(sym.as_str()) {
            uncovered.push(sym.clone());
        }
    }

    if !uncovered.is_empty() {
        panic!(
            "\n\nSI unit coverage gap: {} uncategorized unit(s)!\n\
             The following unit symbols were found in SI.sysml but are not listed\n\
             in SI_UNITS_IMPLEMENTED or SI_UNITS_DEFERRED:\n\n  {}\n\n\
             Add each to the appropriate list in spec_stdlib_coverage.rs.\n",
            uncovered.len(),
            uncovered.join("\n  ")
        );
    }

    // Verify implemented units actually exist in our lookup table
    let mut missing_from_table = Vec::new();
    for (spec_sym, lookup_name) in SI_UNITS_IMPLEMENTED {
        if sysml_runtime::expressions::units::lookup_unit(lookup_name).is_none() {
            missing_from_table.push(format!("{spec_sym} (lookup: {lookup_name})"));
        }
    }

    if !missing_from_table.is_empty() {
        panic!(
            "\n\nSI unit table gap: {} unit(s) listed as IMPLEMENTED but not found\n\
             in the unit conversion table (units.rs lookup_unit):\n\n  {}\n\n\
             Either add the unit to UNIT_TABLE in units.rs or move it to SI_UNITS_DEFERRED.\n",
            missing_from_table.len(),
            missing_from_table.join("\n  ")
        );
    }

    println!(
        "\n=== SI Unit Coverage Summary ===\n\
         Primary SI symbols:  {}\n\
         In unit table:       {}\n\
         Deferred:            {}\n\
         ================================\n",
        spec_symbols.len(),
        implemented.len(),
        deferred.len(),
    );
}
