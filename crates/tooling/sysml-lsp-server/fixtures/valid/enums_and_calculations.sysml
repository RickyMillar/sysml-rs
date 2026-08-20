/* Fixture: enums_and_calculations.sysml
Expected: valid SysML enum and calculation constructs.
Tests: EnumerationDef/Usage, CalculationDef/Usage, numeric/string literals */

package EnumsAndCalculations {
    enum def Color {
        enum Red;
        enum Green;
        enum Blue;
    }

    enum def Priority {
        enum Low;
        enum Medium;
        enum High;
        enum Critical;
    }

    attribute def Length;
    attribute def Area;

    calc def ComputeArea {
        in attribute width : Length;
        in attribute height : Length;
        return result : Area;
    }

    calc def ConvertUnits {
        in attribute value : Real;
        in attribute factor : Real;
        return result : Real;
    }

    part def Widget {
        attribute color : Color;
        attribute priority : Priority;
        attribute name : String = "DefaultWidget";
        attribute count : Integer = 42;
        attribute ratio : Real = 3.14;

        calc area : ComputeArea;
    }
}
