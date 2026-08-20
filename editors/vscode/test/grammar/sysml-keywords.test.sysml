// SYNTAX TEST "source.sysml"

// --- Definition keywords (xxx def Name) ---
// The space between the keyword and 'def' is unscoped.
// We test the first keyword and the 'def' keyword separately.

part def D1 { }
// <---- keyword.other.definition.sysml
//  ^ source.sysml
//   ^^^ keyword.other.definition.sysml
//       ^^ entity.name.type.definition.sysml

action def D2 { }
// <------ keyword.other.definition.sysml
//    ^ source.sysml
//     ^^^ keyword.other.definition.sysml
//         ^^ entity.name.type.definition.sysml

state def D3 { }
// <----- keyword.other.definition.sysml
//   ^ source.sysml
//    ^^^ keyword.other.definition.sysml

port def D4 { }
// <---- keyword.other.definition.sysml
//  ^ source.sysml
//   ^^^ keyword.other.definition.sysml

interface def D5 { }
// <--------- keyword.other.definition.sysml
//       ^ source.sysml
//        ^^^ keyword.other.definition.sysml

requirement def D6 { }
// <----------- keyword.other.definition.sysml
//         ^ source.sysml
//          ^^^ keyword.other.definition.sysml

constraint def D7 { }
// <---------- keyword.other.definition.sysml
//        ^ source.sysml
//         ^^^ keyword.other.definition.sysml

item def D8 { }
// <---- keyword.other.definition.sysml
//  ^ source.sysml
//   ^^^ keyword.other.definition.sysml

enum def D9 { }
// <---- keyword.other.definition.sysml
//  ^ source.sysml
//   ^^^ keyword.other.definition.sysml

attribute def D10 { }
// <--------- keyword.other.definition.sysml
//       ^ source.sysml
//        ^^^ keyword.other.definition.sysml

calc def D11 { }
// <---- keyword.other.definition.sysml
//  ^ source.sysml
//   ^^^ keyword.other.definition.sysml

allocation def D12 { }
// <---------- keyword.other.definition.sysml
//        ^ source.sysml
//         ^^^ keyword.other.definition.sysml

connection def D13 { }
// <---------- keyword.other.definition.sysml
//        ^ source.sysml
//         ^^^ keyword.other.definition.sysml

concern def D14 { }
// <------- keyword.other.definition.sysml
//     ^ source.sysml
//      ^^^ keyword.other.definition.sysml

case def D15 { }
// <---- keyword.other.definition.sysml
//  ^ source.sysml
//   ^^^ keyword.other.definition.sysml

view def D16 { }
// <---- keyword.other.definition.sysml
//  ^ source.sysml
//   ^^^ keyword.other.definition.sysml

viewpoint def D17 { }
// <--------- keyword.other.definition.sysml
//       ^ source.sysml
//        ^^^ keyword.other.definition.sysml

rendering def D18 { }
// <--------- keyword.other.definition.sysml
//       ^ source.sysml
//        ^^^ keyword.other.definition.sysml

metadata def D19 { }
// <-------- keyword.other.definition.sysml
//      ^ source.sysml
//       ^^^ keyword.other.definition.sysml

occurrence def D20 { }
// <---------- keyword.other.definition.sysml
//        ^ source.sysml
//         ^^^ keyword.other.definition.sysml

// --- Visibility keywords ---

public part x1;
// <-- keyword.other.visibility.sysml

private part x2;
// <--- keyword.other.visibility.sysml

protected part x3;
// <----- keyword.other.visibility.sysml

// --- Modifier keywords ---

abstract part def M1 { }
// <---- keyword.other.modifier.sysml

readonly attribute m2;
// <---- keyword.other.modifier.sysml

derived attribute m3;
// <--- keyword.other.modifier.sysml

ordered attribute m4;
// <--- keyword.other.modifier.sysml

nonunique attribute m5;
// <----- keyword.other.modifier.sysml

variation part def M6 { }
// <----- keyword.other.modifier.sysml

// --- Control keywords ---

if true { }
// <- keyword.control.sysml

else { }
// <- keyword.control.sysml

while true { }
// <-- keyword.control.sysml

for x { }
// <- keyword.control.sysml

loop { }
// <- keyword.control.sysml

fork f { }
// <- keyword.control.sysml

join j { }
// <- keyword.control.sysml

merge m { }
// <-- keyword.control.sysml

decide d { }
// <--- keyword.control.sysml

then n;
// <- keyword.control.sysml

// --- Import keywords ---

import Pkg1::*;
// <-- keyword.other.import.sysml

alias A = B;
// <-- keyword.other.import.sysml

// --- Action keywords ---

entry; first; succession;
// <-- keyword.other.action.sysml

exit; transition;
// <- keyword.other.action.sysml

send x to y;
// <- keyword.other.action.sysml

accept a;
// <--- keyword.other.action.sysml

perform p;
// <---- keyword.other.action.sysml

// --- Parameter keywords ---

in x;
// <- keyword.other.parameter.sysml

out y;
// <- keyword.other.parameter.sysml

inout z;
// <-- keyword.other.parameter.sysml

return r;
// <--- keyword.other.parameter.sysml

// --- Type keywords (KerML-level used in SysML) ---

type T1;
// <- keyword.other.type.sysml

classifier C1;
// <------ keyword.other.type.sysml

class Cl1;
// <-- keyword.other.type.sysml

struct S1;
// <--- keyword.other.type.sysml

datatype Dt1;
// <---- keyword.other.type.sysml

// --- Other keywords ---

about; of; to; by; at;
// <-- keyword.other.sysml

end; all; new;
// <- keyword.other.sysml

subject; objective; actor; stakeholder;
// <---- keyword.other.sysml

doc d; comment c;
// <- keyword.other.sysml

// --- Builtin variables ---

self;
// <- variable.language.sysml

this;
// <- variable.language.sysml

// --- New type keywords ---

bool;
// <- keyword.other.type.sysml

feature;
// <---- keyword.other.type.sysml

// --- New usage keywords (bare) ---

binding;
// <---- keyword.other.usage.sysml

connector;
// <------ keyword.other.usage.sysml

step;
// <- keyword.other.usage.sysml

expr;
// <- keyword.other.usage.sysml

// --- New relationship keywords ---

redefine;
// <----- keyword.other.relationship.sysml

superset;
// <----- keyword.other.relationship.sysml

// --- New action keywords ---

collect;
// <---- keyword.other.action.sysml

select;
// <--- keyword.other.action.sysml

start;
// <-- keyword.other.action.sysml

// --- New other keywords ---

that;
// <- keyword.other.sysml

analysis;
// <----- keyword.other.sysml

verification;
// <--------- keyword.other.sysml

use;
// <- keyword.other.sysml
