package AdventOfSysMLv2_Day5_Multiplicities {

    part def SantaSleigh {
        ref item payloads [0..10];
        ref item passengers [1..*];
        abstract ref part reindeer : Reindeer[9];
        ref part dasher subsets reindeer;
    }

    part def Reindeer;
}
