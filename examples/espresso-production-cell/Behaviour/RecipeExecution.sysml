package RecipeExecutionBehaviour {
    doc /* Recipe control-flow with fork/join (COV RT-SM / CELL-11, ACT gate).
     * An extraction recipe forks two preparation activities that proceed
     * concurrently (dose+grind and group-head preheat), joins when both are
     * complete, then runs the extraction and finishes. Fork/join completion is
     * the token-flow property the CELL-ACT gate checks: both branches must
     * complete before extract runs, and the action reaches `done` exactly once. */

    action def RecipeExecution {
        first start;

        then fork;
            then grind;
            then preheat;

        action grind;
        then ready;

        action preheat;
        then ready;

        join ready;
        then extract;

        action extract;
        then done;

        action done;
    }
}
