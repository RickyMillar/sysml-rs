package L11_Attributes_Quantities {

    private import ScalarValues::Real;
    private import ISQ::length;
    private import ISQ::mass;
    private import SI::kg;
    private import SI::cm;

    attribute def ReindeerFeatures {
        // Not variables, so cannot be constant!
        attribute noseColor : Color;
        attribute eyeColor : Color;
    }
    part def Reindeer {
        constant attribute features : ReindeerFeatures;
        attribute energyLevel : Real default := 100;
    }

    enum def Color {
        enum orange; enum yellow; enum green;
        enum blue; enum indigo; enum violet;
        enum silver; enum gold; enum red;
        enum brown; enum white; enum black;
    }
}
