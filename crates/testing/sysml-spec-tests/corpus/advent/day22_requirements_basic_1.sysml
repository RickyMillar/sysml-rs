package SantaSleighSpecification {
    part def SantaSleigh;
}

package UserRequirements {
    requirement def <UR1> DeliverGifts {
        subject sleigh : SantaSleighSpecification::SantaSleigh;
        doc /* Santa's sleigh shall deliver gifts to all nice children. */
    }
}
