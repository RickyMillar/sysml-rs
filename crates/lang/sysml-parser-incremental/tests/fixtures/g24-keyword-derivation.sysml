package G24KeywordDerivation {
    private import RequirementDerivation::*;

    requirement vehicleMassRequirement;
    requirement chassisMassRequirement;

    #derivation connection {
        end #original ::> vehicleMassRequirement;
        end #derive ::> chassisMassRequirement;
    }
}
