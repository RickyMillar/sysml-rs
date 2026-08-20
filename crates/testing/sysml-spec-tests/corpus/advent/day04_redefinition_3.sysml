package AdventDay04_Example3 {
    part def Sleigh {
        part reindeer [*];
    }

    part def SantaSleigh specializes Sleigh {
        part redefines reindeer;
    }
}
