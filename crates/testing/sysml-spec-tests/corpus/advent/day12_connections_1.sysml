package L12_ConnectionDef_CrossSubsetting {
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
}
