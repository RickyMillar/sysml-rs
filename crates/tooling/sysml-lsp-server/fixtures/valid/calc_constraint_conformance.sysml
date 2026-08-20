// Sprint 10: Calculation & Constraint conformance fixture
// Tests: return features, result expressions, assertions, invariants

package CalcConstraintConformance {

    // Calculation definition with return parameter and result expression
    calc def TotalMass {
        in partMass : Real;
        in fuelMass : Real;
        return totalMass : Real;
        partMass + fuelMass
    }

    // Calculation usage
    calc vehicleMass : TotalMass;

    // Constraint definition with result expression
    constraint def SpeedLimit {
        in speed : Real;
        in maxSpeed : Real;
        speed <= maxSpeed
    }

    // Constraint usage
    constraint speedCheck : SpeedLimit;

    // Assert constraint (positive)
    assert constraint withinLimits { true }

    // Assert not constraint (negated)
    assert not constraint overSpeed { false }

    // Invariant constraint inside a definition
    part def Vehicle {
        attribute speed : Real;
        inv safeSpeed { speed < 200 }
    }
}
