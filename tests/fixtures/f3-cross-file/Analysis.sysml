package F3Analysis {
    doc /* File A of the F3 cross-file fixture — the URI the analysis /
     * trade-study / whatif commands are invoked against. It carries no
     * analysis surfaces of its own; everything lives in F3Defs (file B).
     * A parse-only (`require_graph`) view of this file therefore cannot
     * discover the case or the whatif target, whereas the workspace-aware
     * elaboration (which merges both files) can. */

    private import F3Defs::*;

    part entryPoint;
}
