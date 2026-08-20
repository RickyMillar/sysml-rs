/* Fixture: error_recovery_following_items.sysml
Intent: regression
Expected: parser recovers after malformed line and still indexes following definitions. */

package Recovery {
part def Before;
@@@ this is not valid SysML @@@
part def After;
}
