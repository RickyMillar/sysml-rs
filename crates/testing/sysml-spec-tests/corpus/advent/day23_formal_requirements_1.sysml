package SantaSleighSpecification {
    part def SantaSleigh;
}

package UserRequirements {
    requirement def <UR1> DeliverGifts {
        subject sleigh : SantaSleighSpecification::SantaSleigh;
        doc /* Santa's sleigh shall deliver gifts to all nice children. */
    }
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

package RequirementBreakdown {

    private import SantaSleighDesign::*;

    requirement def <'TR1.1'> CockpitForSanta {

        subject sleigh : SantaSleigh;

        require constraint {

            language "English" /* The sleigh must have a cockpit for Santa. */

        }

        requirement : CockpitSize;

    }

    requirement def <'TR1.1.1'> CockpitSize {

        subject sleigh : SantaSleigh;

        doc /* The volume of the cockpit must be at least 3 times as much
             * as the volume of Santa. */

        assume constraint {

            sleigh.santa.weight >= 0

        }

        require constraint {

            sleigh.cockpit.volume >= 3 * sleigh.santa.weight / 1.03 [SI::kg/SI::L]

        }

    }

}
