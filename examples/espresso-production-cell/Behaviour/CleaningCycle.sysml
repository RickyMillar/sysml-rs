package CleaningCycleBehaviour {
    doc /* Station cleaning cycle (COV RT-SM, CELL-11). A sequential maintenance
     * action: backflush the group head, dose a cleaning agent, rinse, then
     * report complete. Entered from the station lifecycle Cleaning state; kept
     * a simple linear succession so the cleaning sequence order is a stable,
     * checkable trace. */

    action def CleaningCycle {
        first start;
        then backflush;

        action backflush;
        then dose;

        action dose;
        then rinse;

        action rinse;
        then report;

        action report;
        then done;

        action done;
    }
}
