package F3Defs {
    doc /* File B of the F3 cross-file fixture. Holds the (self-contained,
     * correctly-classified) analysis / trade-study case and the whatif target.
     * The commands under test are invoked on Analysis.sysml's URI (file A);
     * a parse-only (`require_graph`) view of file A contains none of this, so
     * every command fails discovery. The workspace-aware elaboration merges
     * both files and resolves it. */

    part def Material;

    // Analysis-case USAGE (spec keyword is bare `analysis`; `analysis case`
    // is invalid SysML — SysML.xtext:2215-2224). This one usage form is
    // discovered by both compile_analysis_case and compile_trade_study.
    analysis MotorStudy {
        attribute objective = "minimize";
        part small : Material {
            attribute cost = 100.0;
        }
        part large : Material {
            attribute cost = 300.0;
        }
    }

    part rig {
        attribute speed = 50.0;
        assert constraint speedLimit { speed < 100.0 }
    }
}
