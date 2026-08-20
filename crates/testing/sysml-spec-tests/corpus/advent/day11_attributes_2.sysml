package L11_Attributes_Quantities {

    attribute def ReindeerFeatures {
        // ...
        attribute antlerLength subsets ISQ::length;
    }
    part def Reindeer {
        constant attribute features : ReindeerFeatures;
        constant attribute weight redefines ISQ::mass;
        // ...
    }

    // ...

}
