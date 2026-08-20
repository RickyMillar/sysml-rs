/* Fixture: unterminated_comment.sysml
Intent: invalid
Expected: syntax error for comment that never closes. */

package UnterminatedComment {
part def Valid;
/* this comment never closes
part def Swallowed;
}
