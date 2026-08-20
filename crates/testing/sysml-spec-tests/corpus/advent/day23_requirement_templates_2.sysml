package SantaSleighSpecification {
    part def SantaSleigh;
}

package SantaSleighDesign {

    part def SantaSleigh specializes SantaSleighSpecification::SantaSleigh {

        part cockpit {

            attribute volume :> ISQ::volume = 500 [SI::L];

        }

        ref part santa {

            attribute weight :> ISQ::mass = 150 [SI::kg];

        }

    }

}

package RequirementTemplates {

    requirement def SizingRequirement {

        doc /* The value of {toSize} must be at least {minimumScale} times
             * as large as {toFit}. */

        attribute toSize;

        attribute toFit;

        attribute minimumScale;

        assume constraint {

            toFit >= 0

        }

        require constraint {

            toSize >= toFit * minimumScale

        }

    }

}

package RequirementBreakdown {

    private import SantaSleighDesign::*;

    private import RequirementTemplates::*;

    requirement def <'TR1.1.1'> CockpitSize specializes SizingRequirement {

        subject sleigh : SantaSleigh;

        doc /* The volume of the cockpit must be at least 3 times as much
             * as the volume of Santa. */

        attribute :>> toSize = sleigh.cockpit.volume;

        attribute :>> toFit = sleigh.santa.weight / 1.03 [SI::kg/SI::L];

        attribute :>> minimumScale = 3;

    }

}
