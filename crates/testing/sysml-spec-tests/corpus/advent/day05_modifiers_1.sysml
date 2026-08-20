package AdventOfSysMLv2_Day5 {

    abstract part def CargoBay;
    part def MagicalCargoBay specializes CargoBay;

    part def SantaSleigh {
        abstract ref part reindeer : Reindeer[9];
        ref part dasher subsets reindeer;
        ref part dancer subsets reindeer;
        ref part prancer subsets reindeer;
        ref part vixen subsets reindeer;
        ref part comet subsets reindeer;
        ref part cupid subsets reindeer;
        ref part donner subsets reindeer;
        ref part blitzen subsets reindeer;
        ref part rudolph subsets reindeer;
    }

    part def Reindeer;
}
