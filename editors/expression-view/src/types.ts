// Mirrors crates/tooling/sysml-service/src/types.rs::ExpressionAstNode /
// ExpressionAstResult. The Rust side strips `argIndex` from props and
// sorts children by it, so consumers see a clean, ordered tree.

export type ExpressionAstNode = {
  /** `ElementKind` debug name, e.g. "OperatorExpression", "LiteralRational". */
  kind: string;
  /** `element.name` if set (function name, variable name, qualified name). */
  name?: string;
  /**
   * Non-default props. Common keys:
   * - `operator` (string): "+", "-", "*", "/", ">=", "and", "if", "select", ...
   * - `value` (number | string | boolean): literal value
   * - `isBodyParameter` (boolean): for lambda binding features
   */
  props: Record<string, unknown>;
  /** Already sorted by argIndex on the Rust side. */
  children: ExpressionAstNode[];
};

export type ExpressionAstResult = {
  element_id: string;
  element_name: string | null;
  element_kind: string;
  /** Original source text (from `unresolved_value`) for side-by-side display. */
  source: string | null;
  ast: ExpressionAstNode | null;
};
