// SYNTAX TEST "source.kerml"

// --- Classifier keywords ---

type t;
// <- keyword.other.definition.kerml

classifier c;
// <------ keyword.other.definition.kerml

class cl;
// <-- keyword.other.definition.kerml

struct s;
// <--- keyword.other.definition.kerml

datatype dt;
// <---- keyword.other.definition.kerml

assoc a;
// <-- keyword.other.definition.kerml

interaction i;
// <--------- keyword.other.definition.kerml

behavior b;
// <------ keyword.other.definition.kerml

function f;
// <------ keyword.other.definition.kerml

predicate p;
// <------- keyword.other.definition.kerml

metaclass mc;
// <------- keyword.other.definition.kerml

// --- Feature keywords ---

feature ft;
// <---- keyword.other.usage.kerml

step st;
// <- keyword.other.usage.kerml

expr ex;
// <- keyword.other.usage.kerml

binding bd;
// <---- keyword.other.usage.kerml

connector cn;
// <------- keyword.other.usage.kerml

flow fl;
// <- keyword.other.usage.kerml

// --- Relationship keywords ---

specializes;
// <------- keyword.other.relationship.kerml

subsets;
// <--- keyword.other.relationship.kerml

redefines;
// <----- keyword.other.relationship.kerml

references;
// <------ keyword.other.relationship.kerml

chains;
// <-- keyword.other.relationship.kerml

conjugates;
// <------ keyword.other.relationship.kerml

inverses;
// <---- keyword.other.relationship.kerml

disjoint;
// <---- keyword.other.relationship.kerml

intersects;
// <------ keyword.other.relationship.kerml

unions;
// <-- keyword.other.relationship.kerml

differences;
// <------- keyword.other.relationship.kerml

featuring;
// <----- keyword.other.relationship.kerml

// --- Modifier keywords ---

abstract;
// <---- keyword.other.modifier.kerml

derived;
// <--- keyword.other.modifier.kerml

readonly;
// <---- keyword.other.modifier.kerml

ordered;
// <--- keyword.other.modifier.kerml

nonunique;
// <----- keyword.other.modifier.kerml

composite;
// <----- keyword.other.modifier.kerml

// --- Control keywords ---

if x { }
// <- keyword.control.kerml

else { }
// <- keyword.control.kerml

while x { }
// <-- keyword.control.kerml

for x { }
// <- keyword.control.kerml

// --- Import keywords ---

import A::*;
// <-- keyword.other.import.kerml

alias B = C;
// <-- keyword.other.import.kerml

// --- Parameter keywords ---

in x;
// <- keyword.other.parameter.kerml

out y;
// <- keyword.other.parameter.kerml

inout z;
// <-- keyword.other.parameter.kerml

return r;
// <--- keyword.other.parameter.kerml

// --- Operators ---

x == y;
//^^ keyword.operator.comparison.kerml

x != y;
//^^ keyword.operator.comparison.kerml

x + y;
//^ keyword.operator.arithmetic.kerml

x and y;
//^^^ keyword.operator.logical.kerml

x or y;
//^^ keyword.operator.logical.kerml

x = 1;
//^ keyword.operator.assignment.kerml

x hastype T;
//^^^^^^^ keyword.operator.other.kerml

x istype T;
//^^^^^^ keyword.operator.other.kerml

x as T;
//^^ keyword.operator.other.kerml

// --- Builtin variables ---

self;
// <- variable.language.kerml

this;
// <- variable.language.kerml

// --- New relationship keywords ---

redefine;
// <----- keyword.other.relationship.kerml

superset;
// <----- keyword.other.relationship.kerml
