package TrafficLight {
    private import ScalarValues::String;

    part def TrafficLightController {
        attribute currentColor : String;

        state def TrafficLightStates {
            entry; then red;

            state red;
            state yellow;
            state green;

            transition red_to_green
                first red then green;
            transition green_to_yellow
                first green then yellow;
            transition yellow_to_red
                first yellow then red;
        }
    }

    part controller : TrafficLightController {
        attribute currentColor = "red";
    }
}
