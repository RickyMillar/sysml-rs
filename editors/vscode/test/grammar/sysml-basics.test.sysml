// SYNTAX TEST "source.sysml"

package MyPkg {
// <--- keyword.other.sysml
//      ^^^^^ entity.name.namespace.sysml
//            ^ punctuation.definition.block.begin.sysml
}
// <- punctuation.definition.block.end.sysml

package AnotherPackage {
//      ^^^^^^^^^^^^^^ entity.name.namespace.sysml
}

part def VehicleDef {
// <---- keyword.other.definition.sysml
//       ^^^^^^^^^^ entity.name.type.definition.sysml
}

part def Wheel {
// <---- keyword.other.definition.sysml
//  ^^^^ source.sysml
//       ^^^^^ entity.name.type.definition.sysml
}

action def DriveAction {
// <------ keyword.other.definition.sysml
//         ^^^^^^^^^^^ entity.name.type.definition.sysml
}

state def VehicleStates {
// <----- keyword.other.definition.sysml
//        ^^^^^^^^^^^^^ entity.name.type.definition.sysml
}

attribute def MassAttr {
// <--------- keyword.other.definition.sysml
//            ^^^^^^^^ entity.name.type.definition.sysml
}

enum def Color {
// <---- keyword.other.definition.sysml
//       ^^^^^ entity.name.type.definition.sysml
}

item def Fuel {
// <---- keyword.other.definition.sysml
//       ^^^^ entity.name.type.definition.sysml
}

port def FuelPort {
// <---- keyword.other.definition.sysml
//       ^^^^^^^^ entity.name.type.definition.sysml
}

interface def VehicleInterface {
// <--------- keyword.other.definition.sysml
//            ^^^^^^^^^^^^^^^^ entity.name.type.definition.sysml
}

constraint def MassLimit {
// <---------- keyword.other.definition.sysml
//             ^^^^^^^^^ entity.name.type.definition.sysml
}

requirement def SafetyReq {
// <----------- keyword.other.definition.sysml
//              ^^^^^^^^^ entity.name.type.definition.sysml
}

calc def TotalMass {
// <---- keyword.other.definition.sysml
//       ^^^^^^^^^ entity.name.type.definition.sysml
}

connection def Link {
// <---------- keyword.other.definition.sysml
//             ^^^^ entity.name.type.definition.sysml
}

allocation def SystemAlloc {
// <---------- keyword.other.definition.sysml
//             ^^^^^^^^^^^ entity.name.type.definition.sysml
}

concern def Reliability {
// <------- keyword.other.definition.sysml
//          ^^^^^^^^^^^ entity.name.type.definition.sysml
}

case def TestCase {
// <---- keyword.other.definition.sysml
//       ^^^^^^^^ entity.name.type.definition.sysml
}

view def SystemView {
// <---- keyword.other.definition.sysml
//       ^^^^^^^^^^ entity.name.type.definition.sysml
}

viewpoint def EngineerVP {
// <--------- keyword.other.definition.sysml
//            ^^^^^^^^^^ entity.name.type.definition.sysml
}

rendering def BasicRendering {
// <--------- keyword.other.definition.sysml
//            ^^^^^^^^^^^^^^ entity.name.type.definition.sysml
}

occurrence def Evt {
// <---------- keyword.other.definition.sysml
//             ^^^ entity.name.type.definition.sysml
}
