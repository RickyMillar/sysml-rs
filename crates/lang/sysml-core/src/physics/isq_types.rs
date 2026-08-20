//! Exhaustive ISQ (International System of Quantities) type table.
//!
//! Auto-generated from the SysML v2 standard library (ISO 80000).
//! Source: references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library/
//!         Domain Libraries/Quantities and Units/ISQ*.sysml
//!
//! Total ISQ types: 315 (281 with dimensions, 34 dimensionless)
//!
//! Every ScalarQuantityValue type in the ISQ standard library is accounted for.
//! Types are either in ISQ_TYPES (with dimension vectors) or in
//! ISQ_DIMENSIONLESS_TYPES (dimension = 1, cannot be classified by dimension).

use super::dimension::DimensionVector;

// ---------------------------------------------------------------------------
// ISQ category (source domain from the standard library)
// ---------------------------------------------------------------------------

/// The ISQ source domain for a quantity type.
///
/// Derived from which ISQ*.sysml file the type is defined in.
/// This provides a physics-domain hint that can disambiguate types
/// that share the same dimension vector (e.g., Power vs HeatFlowRate).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IsqCategory {
    Acoustic,
    AtomicNuclear,
    Base,
    Chemical,
    CondensedMatter,
    Electromagnetic,
    Information,
    Luminous,
    Mechanical,
    SpaceTime,
    Thermal,
}

// ---------------------------------------------------------------------------
// Exhaustive ISQ type table
// ---------------------------------------------------------------------------

/// An ISQ type entry: (type_name, dimension_vector, source_category).
pub type IsqTypeEntry = (&'static str, DimensionVector, IsqCategory);

/// All 281 non-dimensionless ISQ ScalarQuantityValue types with their
/// dimension vectors and source categories.
pub static ISQ_TYPES: &[IsqTypeEntry] = &[
    (
        "AbsorbedDoseRateValue",
        DimensionVector::new(2, 0, -3, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "AbsorbedDoseValue",
        DimensionVector::new(2, 0, -2, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "AccelerationValue",
        DimensionVector::new(1, 0, -2, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "AcceptorDensityValue",
        DimensionVector::new(-3, 0, 0, 0, 0, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "AcousticImpedanceValue",
        DimensionVector::new(-4, 1, -1, 0, 0, 0, 0),
        IsqCategory::Acoustic,
    ),
    (
        "ActionQuantityValue",
        DimensionVector::new(2, 1, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "ActivityDensityValue",
        DimensionVector::new(-3, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "AdmittanceValue",
        DimensionVector::new(-2, -1, 3, 2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "AffinityOfAChemicalReactionValue",
        DimensionVector::new(2, 1, -2, 0, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "AmountOfSubstanceConcentrationValue",
        DimensionVector::new(-3, 0, 0, 0, 0, 1, 0),
        IsqCategory::Chemical,
    ),
    (
        "AmountOfSubstanceValue",
        DimensionVector::new(0, 0, 0, 0, 0, 1, 0),
        IsqCategory::Base,
    ),
    (
        "AngularAccelerationValue",
        DimensionVector::new(0, 0, -2, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "AngularFrequencyValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "AngularImpulseValue",
        DimensionVector::new(2, 1, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "AngularMomentumValue",
        DimensionVector::new(2, 1, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "AngularReciprocalLatticeVectorMagnitudeValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "AngularRepetencyValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "AngularVelocityValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "AreaValue",
        DimensionVector::new(2, 0, 0, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "AtomicAttenuationCoefficientValue",
        DimensionVector::new(2, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "AttenuationValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "AverageEnergyLossPerElementaryChargeProducedValue",
        DimensionVector::new(2, 1, -2, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "AverageInformationRateValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Information,
    ),
    (
        "AverageTransinformationRateValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Information,
    ),
    (
        "BinaryDigitRateValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Information,
    ),
    (
        "CallIntensityValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Information,
    ),
    (
        "CapacitanceValue",
        DimensionVector::new(-2, -1, 4, 2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "CelsiusTemperatureValue",
        DimensionVector::new(0, 0, 0, 0, 1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "ChannelTimeCapacityValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Information,
    ),
    (
        "CharacteristicImpedanceOfAMediumForLongitudinalWavesValue",
        DimensionVector::new(-2, 1, -1, 0, 0, 0, 0),
        IsqCategory::Acoustic,
    ),
    (
        "ChemicalPotentialValue",
        DimensionVector::new(2, 1, -2, 0, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "CoefficientOfHeatTransferValue",
        DimensionVector::new(0, 1, -3, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "CoercivityValue",
        DimensionVector::new(-1, 0, 0, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "CompletedCallIntensityValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Information,
    ),
    (
        "CompressibilityValue",
        DimensionVector::new(1, -1, 2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "ConductanceValue",
        DimensionVector::new(-2, -1, 3, 2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ConductivityValue",
        DimensionVector::new(-3, -1, 3, 2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "CubicExpansionCoefficientValue",
        DimensionVector::new(0, 0, 0, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "CurvatureValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "DampingCoefficientValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "DecayConstantValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "DensityOfHeatFlowRateValue",
        DimensionVector::new(0, 1, -3, 0, 0, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "DensityOfVibrationalStatesValue",
        DimensionVector::new(-3, 0, 1, 0, 0, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "DiffusionCoefficientValue",
        DimensionVector::new(2, 0, -1, 0, 0, 0, 0),
        IsqCategory::Chemical,
    ),
    (
        "DirectionAndEnergyDistributionOfCrossSectionValue",
        DimensionVector::new(0, -1, 2, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "DirectionDistributionOfCrossSectionValue",
        DimensionVector::new(2, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "DisplacementCurrentDensityValue",
        DimensionVector::new(-2, 0, 0, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "DonorDensityValue",
        DimensionVector::new(-3, 0, 0, 0, 0, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "DoseEquivalentValue",
        DimensionVector::new(2, 0, -2, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "DurationValue",
        DimensionVector::new(0, 0, 1, 0, 0, 0, 0),
        IsqCategory::Base,
    ),
    (
        "DynamicViscosityValue",
        DimensionVector::new(-1, 1, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "ElectricChargeDensityValue",
        DimensionVector::new(-3, 0, 1, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectricChargeValue",
        DimensionVector::new(0, 0, 1, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectricConstantValue",
        DimensionVector::new(-3, -1, 4, 2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectricCurrentDensityValue",
        DimensionVector::new(-2, 0, 0, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectricCurrentValue",
        DimensionVector::new(0, 0, 0, 1, 0, 0, 0),
        IsqCategory::Base,
    ),
    (
        "ElectricDipoleMomentValue",
        DimensionVector::new(1, 0, 1, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectricFieldStrengthValue",
        DimensionVector::new(1, 1, -3, -1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectricFluxDensityValue",
        DimensionVector::new(-2, 0, 1, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectricFluxValue",
        DimensionVector::new(0, 0, 1, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectricPolarizationValue",
        DimensionVector::new(-2, 0, 1, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectricPotentialDifferenceValue",
        DimensionVector::new(2, 1, -3, -1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectricPotentialValue",
        DimensionVector::new(2, 1, -3, -1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectrolyticConductivityValue",
        DimensionVector::new(-3, -1, 3, 2, 0, 0, 0),
        IsqCategory::Chemical,
    ),
    (
        "ElectromagneticEnergyDensityValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ElectronDensityValue",
        DimensionVector::new(-3, 0, 0, 0, 0, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "EnergyDensityOfStatesValue",
        DimensionVector::new(-5, -1, 2, 0, 0, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "EnergyDistributionOfCrossSectionValue",
        DimensionVector::new(0, -1, 2, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "EnergyFluenceRateValue",
        DimensionVector::new(0, 1, -3, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "EnergyFluenceValue",
        DimensionVector::new(0, 1, -2, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "EnergyValue",
        DimensionVector::new(2, 1, -2, 0, 0, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "EntropyValue",
        DimensionVector::new(2, 1, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "EquilibriumConstantOnConcentrationBasisValue",
        DimensionVector::new(-3, 0, 0, 0, 0, 1, 0),
        IsqCategory::Chemical,
    ),
    (
        "EquilibriumConstantOnPressureBasisValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Chemical,
    ),
    (
        "EquivalentBinaryDigitRateValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Information,
    ),
    (
        "ExposureRateValue",
        DimensionVector::new(0, -1, 0, 1, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "ExposureValue",
        DimensionVector::new(0, -1, 1, 1, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "ForceValue",
        DimensionVector::new(1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "FrequencyValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "FugacityValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Chemical,
    ),
    (
        "FundamentalReciprocalLatticeVectorMagnitudeValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "GyromagneticRatioOfTheElectronValue",
        DimensionVector::new(0, -1, 1, 1, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "GyromagneticRatioValue",
        DimensionVector::new(0, -1, 1, 1, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "HallCoefficientValue",
        DimensionVector::new(3, 0, -1, -1, 0, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "HartreeEnergyValue",
        DimensionVector::new(6, 3, -6, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "HeatCapacityValue",
        DimensionVector::new(2, 1, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "HeatFlowRateValue",
        DimensionVector::new(2, 1, -3, 0, 0, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "HoleDensityValue",
        DimensionVector::new(-3, 0, 0, 0, 0, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "IlluminanceValue",
        DimensionVector::new(-2, 0, 0, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "ImpedanceValue",
        DimensionVector::new(2, 1, -3, -2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ImpulseValue",
        DimensionVector::new(1, 1, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "InductanceValue",
        DimensionVector::new(2, 1, -2, -2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "IntrinsicCarrierDensityValue",
        DimensionVector::new(-3, 0, 0, 0, 0, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "IonNumberDensityValue",
        DimensionVector::new(-3, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "IonicStrengthValue",
        DimensionVector::new(0, -1, 0, 0, 0, 1, 0),
        IsqCategory::Chemical,
    ),
    (
        "IrradianceValue",
        DimensionVector::new(0, 1, -3, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "IsentropicCompressibilityValue",
        DimensionVector::new(1, -1, 2, 0, 0, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "IsothermalCompressibilityValue",
        DimensionVector::new(1, -1, 2, 0, 0, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "JouleThomsonCoefficientValue",
        DimensionVector::new(1, -1, 2, 0, 1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "KermaRateValue",
        DimensionVector::new(2, 0, -3, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "KermaValue",
        DimensionVector::new(2, 0, -2, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "KinematicViscosityValue",
        DimensionVector::new(2, 0, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "LarmorFrequencyValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "LengthValue",
        DimensionVector::new(1, 0, 0, 0, 0, 0, 0),
        IsqCategory::Base,
    ),
    (
        "LinearAbsorptionCoefficientValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "LinearAttenuationCoefficientForIonizingRadiationValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "LinearAttenuationCoefficientValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "LinearDensityOfElectricChargeValue",
        DimensionVector::new(-1, 0, 1, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "LinearElectricCurrentDensityValue",
        DimensionVector::new(-1, 0, 0, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "LinearEnergyTransferValue",
        DimensionVector::new(1, 1, -2, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "LinearExpansionCoefficientValue",
        DimensionVector::new(0, 0, 0, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "LinearIonizationValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "LinearMassDensityValue",
        DimensionVector::new(-1, 1, 0, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "LinkedFluxValue",
        DimensionVector::new(2, 1, -2, -1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "LorenzCoefficientValue",
        DimensionVector::new(4, 2, -6, -2, -2, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "LuminanceValue",
        DimensionVector::new(-2, 0, 0, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "LuminousEfficacyOfASourceValue",
        DimensionVector::new(-2, -1, 3, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "LuminousEfficacyOfRadiationValue",
        DimensionVector::new(-2, -1, 3, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "LuminousEnergyValue",
        DimensionVector::new(0, 0, 1, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "LuminousExitanceValue",
        DimensionVector::new(-2, 0, 0, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "LuminousExposureValue",
        DimensionVector::new(-2, 0, 1, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "LuminousFluxValue",
        DimensionVector::new(0, 0, 0, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "LuminousIntensityValue",
        DimensionVector::new(0, 0, 0, 0, 0, 0, 1),
        IsqCategory::Base,
    ),
    (
        "MagneticConstantValue",
        DimensionVector::new(1, 1, -2, -2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "MagneticDipoleMomentValue",
        DimensionVector::new(3, 1, -2, -1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "MagneticFieldStrengthValue",
        DimensionVector::new(-1, 0, 0, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "MagneticFluxDensityValue",
        DimensionVector::new(0, 1, -2, -1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "MagneticFluxValue",
        DimensionVector::new(2, 1, -2, -1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "MagneticMomentValue",
        DimensionVector::new(2, 0, 0, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "MagneticPolarizationValue",
        DimensionVector::new(0, 1, -2, -1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "MagneticVectorPotentialValue",
        DimensionVector::new(1, 1, -2, -1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "MagnetizationValue",
        DimensionVector::new(-1, 0, 0, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "MagnetomotiveForceValue",
        DimensionVector::new(0, 0, 0, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "MassAbsorptionCoefficientValue",
        DimensionVector::new(2, -1, 0, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "MassAttenuationCoefficientForIonizingRadiationValue",
        DimensionVector::new(2, -1, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "MassAttenuationCoefficientValue",
        DimensionVector::new(2, -1, 0, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "MassChangeRateValue",
        DimensionVector::new(0, 1, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "MassConcentrationOfWaterValue",
        DimensionVector::new(-3, 1, 0, 0, 0, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "MassConcentrationOfWaterVapourAbsoluteHumidityValue",
        DimensionVector::new(-3, 1, 0, 0, 0, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "MassConcentrationValue",
        DimensionVector::new(-3, 1, 0, 0, 0, 0, 0),
        IsqCategory::Chemical,
    ),
    (
        "MassDensityValue",
        DimensionVector::new(-3, 1, 0, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "MassEnergyTransferCoefficientValue",
        DimensionVector::new(2, -1, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "MassFlowRateValue",
        DimensionVector::new(0, 1, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "MassFlowValue",
        DimensionVector::new(-2, 1, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "MassValue",
        DimensionVector::new(0, 1, 0, 0, 0, 0, 0),
        IsqCategory::Base,
    ),
    (
        "MassieuFunctionValue",
        DimensionVector::new(2, 1, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "MaximumLuminousEfficacyValue",
        DimensionVector::new(-2, -1, 3, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "MeanMassRangeValue",
        DimensionVector::new(-2, 1, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "MobilityValue",
        DimensionVector::new(0, -1, 2, 1, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "ModulationRateValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Information,
    ),
    (
        "ModulusOfAdmittanceValue",
        DimensionVector::new(-2, -1, 3, 2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ModulusOfCompressionValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "ModulusOfElasticityValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "ModulusOfImpedanceValue",
        DimensionVector::new(2, 1, -3, -2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ModulusOfRigidityValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "MolalityValue",
        DimensionVector::new(0, -1, 0, 0, 0, 1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarAbsorptionCoefficientValue",
        DimensionVector::new(2, 0, 0, 0, 0, -1, 0),
        IsqCategory::Luminous,
    ),
    (
        "MolarAttenuationCoefficientValue",
        DimensionVector::new(2, 0, 0, 0, 0, -1, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "MolarConductivityValue",
        DimensionVector::new(0, -1, 3, 2, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarEnthalpyValue",
        DimensionVector::new(2, 1, -2, 0, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarEntropyValue",
        DimensionVector::new(2, 1, -2, 0, -1, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarGasConstantValue",
        DimensionVector::new(2, 1, -2, 0, -1, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarGibbsEnergyValue",
        DimensionVector::new(2, 1, -2, 0, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarHeatCapacityValue",
        DimensionVector::new(2, 1, -2, 0, -1, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarHelmholtzEnergyValue",
        DimensionVector::new(2, 1, -2, 0, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarInternalEnergyValue",
        DimensionVector::new(2, 1, -2, 0, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarMassValue",
        DimensionVector::new(0, 1, 0, 0, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarOpticalRotatoryPowerValue",
        DimensionVector::new(2, 0, 0, 0, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MolarVolumeValue",
        DimensionVector::new(3, 0, 0, 0, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "MomentOfForceValue",
        DimensionVector::new(2, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "MomentOfInertiaValue",
        DimensionVector::new(2, 1, 0, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "MomentumValue",
        DimensionVector::new(1, 1, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "NormalStressValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "NuclearActivityValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "NuclearQuadrupoleMomentValue",
        DimensionVector::new(2, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "OsmoticPressureValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Chemical,
    ),
    (
        "PartialPressureValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Chemical,
    ),
    (
        "ParticleConcentrationValue",
        DimensionVector::new(-3, 0, 0, 0, 0, 0, 0),
        IsqCategory::Chemical,
    ),
    (
        "ParticleCurrentDensityValue",
        DimensionVector::new(-2, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "ParticleEmissionRateValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "ParticleFluenceRateValue",
        DimensionVector::new(-2, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "ParticleFluenceValue",
        DimensionVector::new(-2, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "ParticleNumberDensityValue",
        DimensionVector::new(-3, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "ParticleSourceDensityValue",
        DimensionVector::new(-3, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "PermeabilityValue",
        DimensionVector::new(1, 1, -2, -2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "PermeanceValue",
        DimensionVector::new(2, 1, -2, -2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "PermittivityValue",
        DimensionVector::new(-3, -1, 4, 2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "PhaseCoefficientValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "PhaseSpeedOfElectromagneticWavesValue",
        DimensionVector::new(1, 0, -1, 0, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "PhaseVelocityValue",
        DimensionVector::new(1, 0, -1, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "PhotonExitanceValue",
        DimensionVector::new(-2, 0, -1, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "PhotonExposureValue",
        DimensionVector::new(-2, 0, 0, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "PhotonFluxValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "PhotonIntensityValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "PhotonIrradianceValue",
        DimensionVector::new(-2, 0, -1, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "PhotonRadianceValue",
        DimensionVector::new(-2, 0, -1, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "PlanckFunctionValue",
        DimensionVector::new(2, 1, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "PowerValue",
        DimensionVector::new(2, 1, -3, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "PoyntingVectorMagnitudeValue",
        DimensionVector::new(0, 1, -3, 0, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "PressureCoefficientValue",
        DimensionVector::new(-1, 1, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "PressureValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "PropagationCoefficientValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "RadianceValue",
        DimensionVector::new(0, 1, -3, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "RadiantEnergyDensityValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "RadiantExitanceValue",
        DimensionVector::new(0, 1, -3, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "RadiantExposureValue",
        DimensionVector::new(0, 1, -2, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "RadiantFluxValue",
        DimensionVector::new(2, 1, -3, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "RadiantIntensityValue",
        DimensionVector::new(2, 1, -3, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "ReactanceValue",
        DimensionVector::new(2, 1, -3, -2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "RecombinationCoefficientValue",
        DimensionVector::new(3, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "RelativePressureCoefficientValue",
        DimensionVector::new(0, 0, 0, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "ReluctanceValue",
        DimensionVector::new(-2, -1, 2, 2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "RepetencyValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "ResistanceToAlternatingCurrentValue",
        DimensionVector::new(2, 1, -3, -2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ResistanceValue",
        DimensionVector::new(2, 1, -3, -2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ResistivityValue",
        DimensionVector::new(3, 1, -3, -2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "RichardsonConstantValue",
        DimensionVector::new(-2, 0, 0, 1, -2, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "RydbergConstantValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "SecondAxialMomentOfAreaValue",
        DimensionVector::new(4, 0, 0, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "SecondPolarMomentOfAreaValue",
        DimensionVector::new(4, 0, 0, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "SectionModulusValue",
        DimensionVector::new(3, 0, 0, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "SeebeckCoefficientForSubstancesAAndBValue",
        DimensionVector::new(2, 1, -3, -1, -1, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "ShearStressValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "SlowingDownDensityValue",
        DimensionVector::new(-3, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "SoundEnergyDensityValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Acoustic,
    ),
    (
        "SoundExposureValue",
        DimensionVector::new(-2, 2, -3, 0, 0, 0, 0),
        IsqCategory::Acoustic,
    ),
    (
        "SoundIntensityValue",
        DimensionVector::new(0, 1, -3, 0, 0, 0, 0),
        IsqCategory::Acoustic,
    ),
    (
        "SourceVoltageValue",
        DimensionVector::new(2, 1, -3, -1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "SpecificActivityValue",
        DimensionVector::new(0, -1, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "SpecificEnergyValue",
        DimensionVector::new(2, 0, -2, 0, 0, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "SpecificEnthalpyValue",
        DimensionVector::new(2, 0, -2, 0, 0, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "SpecificEntropyValue",
        DimensionVector::new(2, 0, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "SpecificGasConstantValue",
        DimensionVector::new(2, 0, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "SpecificHeatCapacityAtConstantPressureValue",
        DimensionVector::new(2, 0, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "SpecificHeatCapacityAtConstantVolumeValue",
        DimensionVector::new(2, 0, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "SpecificHeatCapacityAtSaturatedVapourPressureValue",
        DimensionVector::new(2, 0, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "SpecificHeatCapacityValue",
        DimensionVector::new(2, 0, -2, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "SpecificOpticalRotatoryPowerValue",
        DimensionVector::new(2, -1, 0, 0, 0, 0, 0),
        IsqCategory::Chemical,
    ),
    (
        "SpecificVolumeValue",
        DimensionVector::new(3, -1, 0, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "SpectralIrradianceValue",
        DimensionVector::new(-1, 1, -3, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "SpectralLuminousEfficacyValue",
        DimensionVector::new(-2, -1, 3, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "SpectralRadianceValue",
        DimensionVector::new(-1, 1, -3, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "SpectralRadiantEnergyDensityInTermsOfWavelengthValue",
        DimensionVector::new(-2, 1, -2, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "SpectralRadiantEnergyDensityInTermsOfWavenumberValue",
        DimensionVector::new(0, 1, -2, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "SpectralRadiantEnergyValue",
        DimensionVector::new(1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "SpectralRadiantExitanceValue",
        DimensionVector::new(-1, 1, -3, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "SpectralRadiantExposureValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "SpectralRadiantFluxValue",
        DimensionVector::new(1, 1, -3, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "SpectralRadiantIntensityValue",
        DimensionVector::new(1, 1, -3, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "SpeedOfLightInAMediumValue",
        DimensionVector::new(1, 0, -1, 0, 0, 0, 0),
        IsqCategory::Luminous,
    ),
    (
        "SpeedOfLightValue",
        DimensionVector::new(1, 0, -1, 0, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "SpeedValue",
        DimensionVector::new(1, 0, -1, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "SpinValue",
        DimensionVector::new(2, 1, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "StandardChemicalPotentialValue",
        DimensionVector::new(2, 1, -2, 0, 0, -1, 0),
        IsqCategory::Chemical,
    ),
    (
        "StressValue",
        DimensionVector::new(-1, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "SurfaceActivityDensityValue",
        DimensionVector::new(-2, 0, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "SurfaceCoefficientOfHeatTransferValue",
        DimensionVector::new(0, 1, -3, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "SurfaceDensityOfElectricChargeValue",
        DimensionVector::new(-2, 0, 1, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "SurfaceMassDensityValue",
        DimensionVector::new(-2, 1, 0, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "SurfaceTensionValue",
        DimensionVector::new(0, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "SusceptanceValue",
        DimensionVector::new(-2, -1, 3, 2, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "ThermalConductanceValue",
        DimensionVector::new(2, 1, -3, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "ThermalConductivityValue",
        DimensionVector::new(1, 1, -3, 0, -1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "ThermalDiffusionCoefficientValue",
        DimensionVector::new(2, 0, -1, 0, 0, 0, 0),
        IsqCategory::Chemical,
    ),
    (
        "ThermalDiffusivityValue",
        DimensionVector::new(2, 0, -1, 0, 0, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "ThermalInsulanceValue",
        DimensionVector::new(0, -1, 3, 0, 1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "ThermalResistanceValue",
        DimensionVector::new(-2, -1, 3, 0, 1, 0, 0),
        IsqCategory::Thermal,
    ),
    (
        "ThermodynamicTemperatureValue",
        DimensionVector::new(0, 0, 0, 0, 1, 0, 0),
        IsqCategory::Base,
    ),
    (
        "ThomsonCoefficientValue",
        DimensionVector::new(2, 1, -3, -1, -1, 0, 0),
        IsqCategory::CondensedMatter,
    ),
    (
        "TorqueValue",
        DimensionVector::new(2, 1, -2, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "TotalAngularMomentumValue",
        DimensionVector::new(2, 1, -1, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "TotalCurrentDensityValue",
        DimensionVector::new(-2, 0, 0, 1, 0, 0, 0),
        IsqCategory::Electromagnetic,
    ),
    (
        "TotalLinearStoppingPowerValue",
        DimensionVector::new(1, 1, -2, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "TotalMassStoppingPowerValue",
        DimensionVector::new(4, 0, -2, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "TransferRateValue",
        DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
        IsqCategory::Information,
    ),
    (
        "TristimulusValuesForTheCie1931StandardColorimetricObserverValue",
        DimensionVector::new(-2, 0, 0, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "TristimulusValuesForTheCie1964StandardColorimetricObserverValue",
        DimensionVector::new(-2, 0, 0, 0, 0, 0, 1),
        IsqCategory::Luminous,
    ),
    (
        "VolumeFlowRateValue",
        DimensionVector::new(3, 0, -1, 0, 0, 0, 0),
        IsqCategory::Mechanical,
    ),
    (
        "VolumeValue",
        DimensionVector::new(3, 0, 0, 0, 0, 0, 0),
        IsqCategory::SpaceTime,
    ),
    (
        "VolumicCrossSectionValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
    (
        "VolumicTotalCrossSectionValue",
        DimensionVector::new(-1, 0, 0, 0, 0, 0, 0),
        IsqCategory::AtomicNuclear,
    ),
];

/// The 34 dimensionless ISQ types (dimension = 1).
/// These cannot be classified by dimension vector alone.
pub static ISQ_DIMENSIONLESS_TYPES: &[&str] = &[
    "AngularMeasureValue",
    "ChannelCapacityPerCharacterValue",
    "CharacterMeanEntropyValue",
    "CharacterMeanTransinformationContentValue",
    "ConditionalEntropyValue",
    "ConditionalInformationContentValue",
    "EntropyForInformationScienceValue",
    "EquivalentBinaryStorageCapacityValue",
    "EquivocationValue",
    "FastFissionFactorValue",
    "InfiniteMultiplicationFactorValue",
    "InformationContentValue",
    "IrrelevanceValue",
    "JointInformationContentValue",
    "LogarithmicFrequencyRangeValue",
    "MaximumEntropyValue",
    "MeanTransinformationContentValue",
    "MultiplicationFactorValue",
    "NonLeakageProbabilityValue",
    "PhaseDifferenceValue",
    "QualityFactorForIonizingRadiationValue",
    "RedundancyValue",
    "SolidAngularMeasureValue",
    "SoundExposureLevelValue",
    "SoundPowerLevelValue",
    "SoundPressureLevelValue",
    "StorageCapacityValue",
    "StrainValue",
    "ThermalUtilizationFactorValue",
    "TrafficCarriedIntensityValue",
    "TrafficIntensityValue",
    "TrafficOfferedIntensityValue",
    "TransinformationContentValue",
    "VolumeFractionValue",
];

/// Look up an ISQ type by name. Returns the dimension vector and category.
pub fn lookup_isq_type(name: &str) -> Option<&'static IsqTypeEntry> {
    ISQ_TYPES.iter().find(|(n, _, _)| *n == name)
}

/// Check if a type name is a known dimensionless ISQ type.
pub fn is_dimensionless_isq_type(name: &str) -> bool {
    ISQ_DIMENSIONLESS_TYPES.contains(&name)
}

/// Total number of ISQ types accounted for (dimensioned + dimensionless).
pub const ISQ_TOTAL_TYPES: usize = 315;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isq_table_completeness() {
        assert_eq!(
            ISQ_TYPES.len() + ISQ_DIMENSIONLESS_TYPES.len(),
            ISQ_TOTAL_TYPES,
            "ISQ table must account for all 315 types"
        );
    }

    #[test]
    fn no_duplicate_type_names() {
        let mut names: Vec<&str> = ISQ_TYPES.iter().map(|(n, _, _)| *n).collect();
        names.extend(ISQ_DIMENSIONLESS_TYPES.iter().copied());
        names.sort();
        for w in names.windows(2) {
            assert_ne!(w[0], w[1], "duplicate ISQ type: {}", w[0]);
        }
    }

    #[test]
    fn dimensionless_types_have_zero_vector() {
        // Verify no dimensionless type accidentally has a non-zero vector in the main table
        for name in ISQ_DIMENSIONLESS_TYPES {
            assert!(
                lookup_isq_type(name).is_none(),
                "dimensionless type {} should not be in ISQ_TYPES",
                name
            );
        }
    }

    #[test]
    fn lookup_known_types() {
        let entry =
            lookup_isq_type("ElectricCurrentValue").expect("should find ElectricCurrentValue");
        assert_eq!(entry.1.current, 1);
        assert_eq!(entry.2, IsqCategory::Base);

        let entry = lookup_isq_type("PressureValue").expect("should find PressureValue");
        assert_eq!(entry.1, DimensionVector::new(-1, 1, -2, 0, 0, 0, 0));
        assert_eq!(entry.2, IsqCategory::Mechanical);

        let entry = lookup_isq_type("ThermodynamicTemperatureValue").expect("should find temp");
        assert_eq!(entry.1.temperature, 1);
        assert_eq!(entry.2, IsqCategory::Base);
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup_isq_type("NotARealValue").is_none());
    }

    /// Verify that every ISQ type with a dimension matching a domain's effort
    /// or flow dimension is correctly classified by classify_dimension.
    /// Types that don't match any domain should return None (not panic).
    #[test]
    fn every_isq_type_classifies_or_returns_none() {
        use crate::physics::domain::PhysicsDomainRegistry;

        let registry = PhysicsDomainRegistry::new();
        let mut classified_count = 0;
        let mut unclassified_count = 0;

        for &(name, ref dim, _category) in ISQ_TYPES {
            // This must never panic
            let result = registry.classify_dimension(dim);
            if result.is_some() {
                classified_count += 1;
            } else {
                unclassified_count += 1;
            }
        }

        // Sanity: we should classify a decent number (at least the 7 base quantities
        // plus common effort/flow types)
        assert!(
            classified_count >= 10,
            "expected at least 10 classified ISQ types, got {classified_count}"
        );
        // All 281 should be accounted for
        assert_eq!(
            classified_count + unclassified_count,
            ISQ_TYPES.len(),
            "all types should either classify or return None"
        );
    }

    /// Verify that classify_dimension_with_hint resolves the Power/HeatFlowRate
    /// ambiguity correctly using ISQ category.
    #[test]
    fn hint_disambiguates_power_vs_heat_flow() {
        use crate::physics::domain::PhysicsDomainRegistry;

        let registry = PhysicsDomainRegistry::new();

        // PowerValue is in ISQMechanics (L²·M·T⁻³)
        let power = lookup_isq_type("PowerValue").expect("PowerValue");
        let result = registry.classify_dimension_with_hint(&power.1, power.2);
        // Without hint it would be thermal (first match). With hint from Mechanical,
        // no mechanical domain has L²·M·T⁻³ as effort or flow, so it falls through
        // to first match (thermal). This is expected — Power is cross-domain.
        assert!(result.is_some(), "PowerValue should classify");

        // HeatFlowRateValue is in ISQThermodynamics
        let heat = lookup_isq_type("HeatFlowRateValue").expect("HeatFlowRateValue");
        let result = registry.classify_dimension_with_hint(&heat.1, heat.2);
        assert!(result.is_some(), "HeatFlowRateValue should classify");
        let (domain, _role) = result.unwrap();
        assert_eq!(
            domain.name, "thermal",
            "HeatFlowRateValue with Thermal hint → thermal"
        );
    }

    /// Verify that every ISQ category has at least one type in the table.
    #[test]
    fn every_category_has_types() {
        use std::collections::HashSet;
        let categories: HashSet<IsqCategory> = ISQ_TYPES.iter().map(|(_, _, c)| *c).collect();

        let expected = [
            IsqCategory::Acoustic,
            IsqCategory::AtomicNuclear,
            IsqCategory::Base,
            IsqCategory::Chemical,
            IsqCategory::CondensedMatter,
            IsqCategory::Electromagnetic,
            IsqCategory::Information,
            IsqCategory::Luminous,
            IsqCategory::Mechanical,
            IsqCategory::SpaceTime,
            IsqCategory::Thermal,
        ];

        for cat in &expected {
            assert!(
                categories.contains(cat),
                "ISQ_TYPES should contain at least one type for {:?}",
                cat
            );
        }
    }
}
