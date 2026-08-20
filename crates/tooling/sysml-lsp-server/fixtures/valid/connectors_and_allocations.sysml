// Fixture: connectors, bindings, successions, and allocations.
// Tests all 7 connector-like usage kinds for positive diagnostic validation.

package ConnectorFixture {

    part def System;
    part def Hardware;
    part def Software;
    port def DataPort;

    part system : System {
        part hw : Hardware;
        part sw : Software;
        port dataIn : DataPort;
        port dataOut : DataPort;
    }

    // ConnectorAsUsage
    connector link from system.hw to system.sw;

    // BindingConnectorAsUsage
    binding bind system.dataIn = system.dataOut;

    // ConnectionUsage
    connection conn connect system.hw to system.sw;

    // InterfaceUsage
    interface iface from system.dataIn to system.dataOut;

    // AllocationUsage
    allocate system.sw to system.hw;

    // SuccessionAsUsage (declared form)
    succession seq first system.hw then system.sw;

    // SuccessionAsUsage (shorthand form)
    action def Initialize;
    action def Configure;
    action init : Initialize;
    action config : Configure;
    first init;
    then config;
}
