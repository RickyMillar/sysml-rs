package PhysicalLaws {
    doc /* Independently-derived lumped physical laws shared across the cell.
     * All forms are first-principles hydraulic/thermal engineering, chosen for
     * numerical stability, observability, and short CI-duration integration
     * (see README "Synthetic coefficients"). No product physics consulted.
     *
     * These are the reusable *laws*; concrete coefficients live on the Physics
     * ODE parts and are overridable per station instance. */

    private import ScalarValues::*;

    // Orifice/branch law: turbulent flow through a resistance is proportional
    // to the signed square root of the pressure drop. `signed_root` keeps the
    // branch well-defined (and sign-correct) under reverse flow.
    //   q_branch = valve * G * signed_root(p_supply - p_return)
    calc def SignedRoot {
        in x : Real;
        return r : Real = (if x >= 0.0 ? 1.0 else 0.0 - 1.0) * sqrt(abs(x));
    }

    // Conservation / accounting laws, authored as reusable constraint defs so
    // concrete parts assert them from the SAME quantities exposed to users
    // (COV VAL-CONSTR). `tol` bounds the lumped-model residual.
    constraint def MassBalance {
        in inflow : Real;
        in outflow : Real;
        in accumulation : Real;
        in tol : Real;
        abs(inflow - (outflow + accumulation)) <= tol
    }

    constraint def EnergyBalance {
        in powerIn : Real;
        in powerOut : Real;
        in storageRate : Real;
        in tol : Real;
        abs(powerIn - (powerOut + storageRate)) <= tol
    }

    // A bounded-range invariant reused by physical-plausibility requirements:
    // a physical quantity must stay within a declared operating envelope.
    constraint def WithinEnvelope {
        in value : Real;
        in lo : Real;
        in hi : Real;
        value >= lo and value <= hi
    }
}
