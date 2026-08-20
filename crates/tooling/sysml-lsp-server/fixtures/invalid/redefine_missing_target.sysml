/* Fixture: redefine_missing_target.sysml
Intent: invalid
Expected: semantic error for redefinition target that does not exist. */

package RedefineTest {
part def Base {
part engine;
}
part def Child :> Base {
part engine :>> noSuchFeature;
}
}
