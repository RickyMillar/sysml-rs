// BrewCycle action flow — standalone for diagram generation
// Shows: fork/join parallelism + decide/merge branching

action def BrewCycle {
    in attribute espressoMode : Boolean;

    action grind;
    action heat;
    action espressoExtract;
    action dripExtract;
    action dispense;

    // Grind and heat in parallel
    fork;
        then grind;
        then heat;
    join;

    // Choose extraction method
    decide;
    merge;

    // Wire it up with succession declarations
    succession first start then fork;
    succession first grind then join;
    succession first heat then join;
    succession first join then decide;
    succession first decide then espressoExtract;
    succession first decide then dripExtract;
    succession first espressoExtract then merge;
    succession first dripExtract then merge;
    succession first merge then dispense;
    succession first dispense then done;
}
