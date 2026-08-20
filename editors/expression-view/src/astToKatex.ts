import type { ExpressionAstNode } from './types';

// Precedence values match a left-to-right reading of the SysML v2 spec.
// Higher binds tighter. `atom` = max — never needs parens.
const PREC = {
  implies: 1,
  or: 2,
  xor: 2,
  and: 3,
  not: 4,
  eq: 5,
  rel: 6,
  add: 7,
  mul: 8,
  unary: 9,
  pow: 10,
  postfix: 11,
  atom: 99,
} as const;

type Prec = (typeof PREC)[keyof typeof PREC];

function precOf(op: string): Prec {
  switch (op) {
    case 'implies':
      return PREC.implies;
    case 'or':
      return PREC.or;
    case 'xor':
      return PREC.xor;
    case 'and':
      return PREC.and;
    case 'not':
    case '~':
      return PREC.not;
    case '==':
    case '!=':
    case '===':
    case '!==':
      return PREC.eq;
    case '..':
      return PREC.rel;
    case '<':
    case '<=':
    case '>':
    case '>=':
    case 'hastype':
    case 'istype':
    case '@':
    case '@@':
      return PREC.rel;
    case '+':
    case '-':
      return PREC.add;
    case '*':
    case '/':
    case '%':
      return PREC.mul;
    case '**':
    case '^':
      return PREC.pow;
    case '??':
      return PREC.or;
    case 'if':
      return PREC.implies;
    default:
      return PREC.atom;
  }
}

// KaTeX rendering for an operator symbol. Returns null for operators that
// need bespoke structure (div as \frac, if as \begin{cases}, ...).
function opToken(op: string): string | null {
  switch (op) {
    case '>=':
      return '\\geq';
    case '<=':
      return '\\leq';
    case '==':
      return '=';
    case '!=':
      return '\\neq';
    // `===` / `!==` are *reference* equality — spelled out because
    // math readers would read `\equiv` as mathematical identity.
    case '===':
      return '\\mathrel{\\text{refEq}}';
    case '!==':
      return '\\mathrel{\\text{refNotEq}}';
    // Range `a..b` — upright double-dot, unambiguous with the source.
    case '..':
      return '\\mathbin{..}';
    case '&&':
    case 'and':
      return '\\wedge';
    case '||':
    case 'or':
      return '\\vee';
    case 'xor':
      return '\\oplus';
    case 'implies':
      return '\\Rightarrow';
    case 'not':
      return '\\neg';
    // Unary bitwise complement. `\sim` would read as math
    // "equivalence to" — misleading. Spell it instead.
    case '~':
      return '\\mathord{\\text{bitnot}\\,}';
    case '*':
      return '\\cdot';
    case '%':
      return '\\bmod';
    case '+':
    case '-':
    case '<':
    case '>':
      return op;
    // Null coalescing: `a ?? b` reads as "a else b" (fall back when null).
    case '??':
      return '\\mathrel{\\text{else}}';
    // `@` is a spec-level synonym for `istype` (KerML
    // ClassificationTestOperator: 'hastype' | 'istype' | '@').
    // Render both the same so source variation doesn't leak into math.
    case '@':
    case 'istype':
      return '\\mathrel{\\text{istype}}';
    case 'hastype':
      return '\\mathrel{\\text{hastype}}';
    // `@@` is the MetaClassificationTestOperator — "is of metatype".
    case '@@':
      return '\\mathrel{\\text{metatype}}';
    case 'as':
      return '\\mathrel{\\text{as}}';
    case 'meta':
      return '\\mathrel{\\text{meta}}';
    default:
      return null;
  }
}

function escapeText(s: string): string {
  return s.replace(/([\\{}$&#^_%~])/g, '\\$1');
}

// `T_busbar` → `T_{\text{busbar}}`, `foo` → `\text{foo}`, `T_a_b` → `T_{a\_b}`.
function renderName(name: string): string {
  const firstUnderscore = name.indexOf('_');
  if (firstUnderscore <= 0) {
    // No subscript (or leading `_`): render as a single text atom.
    return `\\text{${escapeText(name)}}`;
  }
  const head = name.slice(0, firstUnderscore);
  const tail = name.slice(firstUnderscore + 1);
  const headRendered = /^[A-Za-z]$/.test(head) ? head : `\\text{${escapeText(head)}}`;
  return `${headRendered}_{\\text{${escapeText(tail)}}}`;
}

function asString(v: unknown): string | null {
  return typeof v === 'string' ? v : null;
}

function asNumber(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null;
}

function asBool(v: unknown): boolean | null {
  return typeof v === 'boolean' ? v : null;
}

// Wrap `inner` in `\left( ... \right)` when the child's precedence is
// weaker than what the parent context requires. `childPrec` is the
// outermost precedence of `inner`; `parentPrec` is the minimum strength
// allowed at that slot.
function paren(inner: string, childPrec: Prec, parentPrec: Prec): string {
  if (childPrec < parentPrec) {
    return `\\left(${inner}\\right)`;
  }
  return inner;
}

type Rendered = { tex: string; prec: Prec };

// Public: render an AST node to KaTeX source.
export function astToKatex(node: ExpressionAstNode): string {
  return renderNode(node).tex;
}

function renderNode(node: ExpressionAstNode): Rendered {
  switch (node.kind) {
    case 'LiteralInteger':
    case 'LiteralRational': {
      const v = asNumber(node.props.value);
      if (v !== null) return { tex: String(v), prec: PREC.atom };
      const s = asString(node.props.value);
      return { tex: s ?? '?', prec: PREC.atom };
    }
    case 'LiteralBoolean': {
      const b = asBool(node.props.value);
      return { tex: b === true ? '\\text{true}' : b === false ? '\\text{false}' : '?', prec: PREC.atom };
    }
    case 'LiteralString': {
      const s = asString(node.props.value) ?? '';
      return { tex: `\\text{\\textquotedblleft}\\text{${escapeText(s)}}\\text{\\textquotedblright}`, prec: PREC.atom };
    }
    case 'LiteralInfinity':
      return { tex: '\\infty', prec: PREC.atom };
    case 'NullExpression':
      return { tex: '\\text{null}', prec: PREC.atom };
    case 'LiteralExpression': {
      // Fallback — literal wrappers without a value prop.
      if (node.children.length === 1) return renderNode(node.children[0]!);
      return { tex: node.name ? renderName(node.name) : '?', prec: PREC.atom };
    }
    case 'FeatureReferenceExpression': {
      // `Type::Variant` stays as-is with a `\mathord` wrapper so it
      // doesn't get accidentally subscripted.
      const name = node.name ?? '?';
      if (name.includes('::')) {
        const parts = name.split('::').map(escapeText).join('{::}');
        return { tex: `\\mathrm{${parts}}`, prec: PREC.atom };
      }
      return { tex: renderName(name), prec: PREC.atom };
    }
    case 'FeatureChainExpression': {
      // Dotted chain: render each child joined by `.`.
      const parts = node.children.map((c) => renderNode(c).tex);
      if (parts.length === 0 && node.name) return { tex: renderName(node.name), prec: PREC.atom };
      return { tex: parts.join('.'), prec: PREC.atom };
    }
    case 'OperatorExpression':
      return renderOperator(node);
    case 'InvocationExpression':
      return renderInvocation(node);
    case 'SelectExpression':
      return renderCollection(node, 'select');
    case 'CollectExpression':
      return renderCollection(node, 'collect');
    case 'IndexExpression': {
      if (node.children.length >= 2) {
        const src = renderNode(node.children[0]!);
        const idx = renderNode(node.children[1]!);
        return {
          tex: `${paren(src.tex, src.prec, PREC.postfix)}\\#(${idx.tex})`,
          prec: PREC.postfix,
        };
      }
      return fallback(node);
    }
    case 'MetadataAccessExpression': {
      // Dot-form `elem.metadata`. The owner is in `name` (as emitted by
      // the parser); chained dots would be a `FeatureChainExpression`.
      const head = node.name ? renderName(node.name) : '\\text{?}';
      return { tex: `${head}.\\text{metadata}`, prec: PREC.postfix };
    }
    case 'InstantiationExpression': {
      const typeName = asString(node.props.type) ?? node.name ?? '?';
      const args = node.children.map((c) => renderNode(c).tex).join(', ');
      return {
        tex: `\\mathrm{${escapeText(typeName)}}(${args})`,
        prec: PREC.postfix,
      };
    }
    default:
      return fallback(node);
  }
}

function fallback(node: ExpressionAstNode): Rendered {
  const label = escapeText(node.kind);
  return { tex: `\\text{?\\langle ${label}\\rangle}`, prec: PREC.atom };
}

function renderOperator(node: ExpressionAstNode): Rendered {
  const op = asString(node.props.operator) ?? '?';
  const kids = node.children;

  // Conditional: if (cond, then, else)
  if (op === 'if' && kids.length === 3) {
    const c = renderNode(kids[0]!).tex;
    const t = renderNode(kids[1]!).tex;
    const e = renderNode(kids[2]!).tex;
    return {
      tex: `\\begin{cases} ${t} & \\text{if } ${c} \\\\ ${e} & \\text{otherwise} \\end{cases}`,
      prec: PREC.atom,
    };
  }

  // Unary prefix.
  if (kids.length === 1) {
    const child = renderNode(kids[0]!);
    const prec = op === 'not' || op === '~' ? PREC.not : PREC.unary;
    const tok = opToken(op);
    const sym = tok ?? `\\text{${escapeText(op)}}`;
    // Space for word-ish operators, tight for symbols.
    const sep = /[a-z]/.test(sym[1] ?? '') ? ' ' : '';
    return {
      tex: `${sym}${sep}${paren(child.tex, child.prec, prec)}`,
      prec,
    };
  }

  // Binary (or n-ary left-associative already flattened by the parser
  // into nested 2-child OperatorExpressions).
  if (kids.length === 2) {
    const lhs = renderNode(kids[0]!);
    const rhs = renderNode(kids[1]!);
    const prec = precOf(op);

    // Division → \frac{}{}.
    if (op === '/') {
      return {
        tex: `\\frac{${lhs.tex}}{${rhs.tex}}`,
        prec: PREC.atom,
      };
    }

    // Exponentiation → superscript (right-assoc: don't paren rhs if it's a power).
    if (op === '**' || op === '^') {
      const lhsParen = paren(lhs.tex, lhs.prec, PREC.postfix);
      const rhsParen = rhs.prec >= PREC.pow ? rhs.tex : `\\left(${rhs.tex}\\right)`;
      return {
        tex: `{${lhsParen}}^{${rhsParen}}`,
        prec: PREC.pow,
      };
    }

    const tok = opToken(op);
    const sym = tok ?? `\\mathbin{\\text{${escapeText(op)}}}`;

    // Left-associative for most ops: lhs at prec, rhs at prec+1.
    const lhsParen = paren(lhs.tex, lhs.prec, prec);
    const rhsAllowed = op === '**' || op === '^' ? prec : ((prec + 1) as Prec);
    const rhsParen = paren(rhs.tex, rhs.prec, rhsAllowed);

    return {
      tex: `${lhsParen} ${sym} ${rhsParen}`,
      prec,
    };
  }

  // n-ary / unknown arity fallback.
  const parts = kids.map((c) => renderNode(c).tex);
  const tok = opToken(op);
  const sym = tok ?? `\\text{${escapeText(op)}}`;
  return {
    tex: parts.join(` ${sym} `),
    prec: precOf(op),
  };
}

function renderInvocation(node: ExpressionAstNode): Rendered {
  const name = node.name ?? asString(node.props.name) ?? 'f';

  // Drop body-parameter children from the positional arg list —
  // they're lambda bindings, rendered as part of the body notation.
  const bodyParams = node.children.filter((c) => c.props.isBodyParameter === true);
  const argChildren = node.children.filter((c) => c.props.isBodyParameter !== true);

  // Arrow-call collection: `source->fn{args}` becomes InvocationExpression
  // with fn name = select/collect/forAll/exists/reject, first arg = source.
  const collectionOps = new Set(['select', 'collect', 'forAll', 'exists', 'reject']);
  if (collectionOps.has(name) && argChildren.length >= 1) {
    const src = renderNode(argChildren[0]!);
    const body = argChildren.slice(1).map((c) => renderNode(c).tex).join(', ');
    const binding = bodyParams[0]?.name ?? null;
    const bindingTex = binding ? `${renderName(binding)} \\mapsto ` : '';
    return {
      tex: `${paren(src.tex, src.prec, PREC.postfix)}\\rightarrow\\operatorname{${escapeText(name)}}\\{${bindingTex}${body}\\}`,
      prec: PREC.postfix,
    };
  }

  const args = argChildren.map((c) => renderNode(c).tex);

  // Built-in well-known functions.
  switch (name) {
    case 'abs':
      if (args.length === 1) return { tex: `\\left|${args[0]}\\right|`, prec: PREC.atom };
      break;
    case 'sqrt':
      if (args.length === 1) return { tex: `\\sqrt{${args[0]}}`, prec: PREC.atom };
      break;
    case 'sin':
    case 'cos':
    case 'tan':
    case 'ln':
    case 'log':
    case 'exp':
      if (args.length === 1) return { tex: `\\${name}\\left(${args[0]}\\right)`, prec: PREC.postfix };
      break;
  }

  return {
    tex: `\\operatorname{${escapeText(name)}}\\left(${args.join(', ')}\\right)`,
    prec: PREC.postfix,
  };
}

function renderCollection(node: ExpressionAstNode, label: 'select' | 'collect'): Rendered {
  if (node.children.length >= 2) {
    const src = renderNode(node.children[0]!);
    const body = renderNode(node.children[1]!);
    const glyph = label === 'select' ? '?' : '';
    return {
      tex: `${paren(src.tex, src.prec, PREC.postfix)}.${glyph}\\{${body.tex}\\}`,
      prec: PREC.postfix,
    };
  }
  return fallback(node);
}
