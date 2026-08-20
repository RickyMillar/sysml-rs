package L15_Basic_Feature_Evaluation {

    private import ScalarValues::Real;
    private import ISQ::length;
    private import ISQ::mass;
    private import SI::kg;
    private import SI::cm;

    attribute def ReindeerFeatures {
        attribute noseColor : Color;
        attribute eyeColor : Color;
        attribute antlerLength subsets ISQ::length;
    }

    part def Reindeer {
        constant attribute features : ReindeerFeatures;
        constant attribute weight redefines ISQ::mass default 110 [kg];
        attribute energyLevel : Real default := 100;
    }

    part def Rudolph specializes Reindeer {
        constant attribute :>> features = new ReindeerFeatures(Color::red, Color::brown, 60 [cm]);
        constant attribute :>> weight = 100 [kg];
        attribute :>> energyLevel := 200;
    }

    enum def Color {
        enum orange; enum yellow; enum green;
        enum blue; enum indigo; enum violet;
        enum silver; enum gold; enum red;
        enum brown; enum white; enum black;
    }
}
