package ThroughputRequirements {
    doc /* Throughput / responsiveness requirements (COV VER-SIM, CELL-15): each
     * station must warm up and become available within a bounded time, and the
     * recovery interval between shots is bounded. Synthetic bounds justified by
     * the declared SM timers (preheat 3 s, recover 1 s). */

    private import ScalarValues::*;

    requirement def <'REQ-T-01'> WarmupWithinBudget {
        doc /* A station shall reach Ready within 5 s of power-on. */
        subject readyTimeS : Real;
        require constraint { readyTimeS <= 5.0 }
    }

    requirement def <'REQ-T-02'> RecoveryBounded {
        doc /* Recovery between extractions shall not exceed 2 s. */
        subject recoverTimeS : Real;
        require constraint { recoverTimeS <= 2.0 }
    }
}
