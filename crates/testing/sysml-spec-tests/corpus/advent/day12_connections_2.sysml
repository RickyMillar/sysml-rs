package L12_ConnectionUsage_RefSubsetting {
    // Components definitions
    part def Sleigh {
        abstract ref part reindeer : Reindeer [9];
    }

    part def Reindeer {
        ref part pulling : Sleigh;
    }
    // Connection definitions connecting components, composing interface
    connection def Harness {
        end part sleigh : Sleigh crosses reindeer.pulling;
        end part reindeer : Reindeer crosses sleigh.reindeer;
    }

    // Connecting in a context
    part def ReindeerDrawnSleighAssembly {
        part sleigh : Sleigh;
        abstract part reindeer : Reindeer [9];
        part rudolph [1] : Reindeer subsets reindeer;

        // Internal structure, including the interface and the flows in that
        // are automatically set up when the connection is established.
        connection harnesses : Harness connect [1] sleigh to [9] reindeer;
        connection subsets harnesses {
            end [1] part :>> sleigh references ReindeerDrawnSleighAssembly::sleigh;
            end [1] part :>> reindeer references rudolph;
        }
    }
}
