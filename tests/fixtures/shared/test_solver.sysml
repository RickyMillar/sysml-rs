// ===========================================================================
// DEMO: Phase 11 — Parametric Constraint Solving
// ===========================================================================
//
// COMMANDS TO TEST:
//   sysml solve tests/fixtures/shared/test_solver.sysml
//   sysml solve tests/fixtures/shared/test_solver.sysml --set totalBudget=5000
//   sysml solve tests/fixtures/shared/test_solver.sysml --sweep engineMass:500:2000
//   sysml solve tests/fixtures/shared/test_solver.sysml --rollup mass
//   sysml solve tests/fixtures/shared/test_solver.sysml --json
//
// EXPECTED:
//   - Binding propagation: totalBudget flows through equality chains
//   - DOF analysis: reports equations vs unknowns
//   - Sensitivity sweep: identifies where massConstraint flips
//   - Rollup: sums mass across subparts
//
// ===========================================================================

package SolverDemo {

    part def Vehicle {
        attribute mass : ScalarValues::Real;

        part chassis {
            attribute mass = 800;
        }

        part engine {
            attribute mass = 1200;
        }

        part body {
            attribute mass = 600;
        }

        // Total mass should equal sum of components
        // (constraint — checked by sysml check)
        assert constraint massLimit { mass < 3000 }
    }

    part vehicle : Vehicle {
        attribute mass = 2600;
        attribute engineMass = 1200;
        attribute chassisMass = 800;
        attribute bodyMass = 600;
        attribute maxSpeed = 180;

        // Constraints with different thresholds
        assert constraint massConstraint { mass < 2800 }
        assert constraint speedConstraint { maxSpeed < 250 }

        // Binding connectors (equality constraints for propagation)
        bind engineMass = engine.mass;
    }
}
