// Tier 4: Allocation and dependency patterns
// Tests: allocate, allocation def, dependency

package AllocationTests {

    // Allocation definition
    allocation def FunctionToComponent;

    // Allocation usage
    part system {
        part softwareModule;
        part hardwareUnit;

        allocate softwareModule to hardwareUnit;
    }

    // Multiple allocations
    part deployed {
        part function1;
        part function2;
        part processor;
        part memory;

        allocate function1 to processor;
        allocate function2 to memory;
    }

    // Dependency
    part architecture {
        part client;
        part server;

        dependency from client to server;
    }

    // Occurrence usage
    occurrence def Impact;
    occurrence def Response;

    part timeline {
        occurrence impact : Impact;
        occurrence response : Response;
        succession impact then response;
    }
}
