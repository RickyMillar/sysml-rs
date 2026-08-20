/* Fixture: wrong_typing.sysml
Expected: S015 Error — part usage typed by wrong definition kind.
- Message should mention "action" to explain why the typing is wrong
- Only S-series errors; no syntax errors or structural noise
Tests: wrong_typing_s015_with_explanation, wrong_typing_only_s_series */
package WrongTyping {
action def DoSomething;
part myPart : DoSomething;
}
