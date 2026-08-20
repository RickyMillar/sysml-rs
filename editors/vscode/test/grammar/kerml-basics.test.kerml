// SYNTAX TEST "source.kerml"

// --- Namespace and package ---

namespace MyNamespace {
//        ^^^^^^^^^^^ entity.name.namespace.kerml
}

package MyPkg {
//      ^^^^^ entity.name.namespace.kerml
}

library MyLib {
//      ^^^^^ entity.name.namespace.kerml
}

// --- Classifier definitions with name ---

type T1 { }
//   ^^ entity.name.type.definition.kerml

classifier C1 { }
//         ^^ entity.name.type.definition.kerml

class Cls1 { }
//    ^^^^ entity.name.type.definition.kerml

struct S1 { }
//     ^^ entity.name.type.definition.kerml

datatype DT1 { }
//       ^^^ entity.name.type.definition.kerml

assoc A1 { }
//    ^^ entity.name.type.definition.kerml

interaction I1 { }
//          ^^ entity.name.type.definition.kerml

behavior B1 { }
//       ^^ entity.name.type.definition.kerml

function F1 { }
//       ^^ entity.name.type.definition.kerml

predicate P1 { }
//        ^^ entity.name.type.definition.kerml

metaclass MC1 { }
//        ^^^ entity.name.type.definition.kerml

assoc struct AS1 { }
//           ^^^ entity.name.type.definition.kerml

metadata def MD1 { }
//           ^^^ entity.name.type.definition.kerml

// --- Comments ---

// a line comment
// <------------- comment.line.double-slash.kerml

/* block comment */
// <-------------- comment.block.kerml

/** documentation */
// <-------------- comment.block.documentation.kerml

/**/
// <- comment.block.empty.kerml

// --- Strings ---

"hello";
// <--- string.quoted.double.kerml

// --- Numbers ---

42;
// <- constant.numeric.kerml

3.14;
// <- constant.numeric.kerml

0xFF;
// <- constant.numeric.hex.kerml

// --- Constants ---

true;
// <- constant.language.kerml

false;
// <-- constant.language.kerml

null;
// <- constant.language.kerml
