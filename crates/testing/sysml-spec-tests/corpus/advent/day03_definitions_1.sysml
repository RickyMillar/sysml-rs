package AdventOfSysMLv2_Day3 {

    part def SantaSleigh {
        part dashboard : NaughtyNiceDashboard;
        part cockpit {
            ref item driver;
        }
    }

    part def NaughtyNiceDashboard;
}
