import { describe, it, expect } from 'vitest';
import { astToKatex } from '../astToKatex';
import type { ExpressionAstNode } from '../types';

// Helper constructors to keep fixtures terse.
function op(operator: string, ...children: ExpressionAstNode[]): ExpressionAstNode {
  return { kind: 'OperatorExpression', props: { operator }, children };
}
function num(value: number): ExpressionAstNode {
  return { kind: 'LiteralRational', props: { value }, children: [] };
}
function int(value: number): ExpressionAstNode {
  return { kind: 'LiteralInteger', props: { value }, children: [] };
}
function ref(name: string): ExpressionAstNode {
  return { kind: 'FeatureReferenceExpression', name, props: {}, children: [] };
}
function call(name: string, ...args: ExpressionAstNode[]): ExpressionAstNode {
  return { kind: 'InvocationExpression', name, props: {}, children: args };
}
function chain(...segments: ExpressionAstNode[]): ExpressionAstNode {
  return { kind: 'FeatureChainExpression', props: {}, children: segments };
}

describe('astToKatex — literals', () => {
  it('renders integers and rationals as bare numbers', () => {
    expect(astToKatex(int(42))).toBe('42');
    expect(astToKatex(num(3.14))).toBe('3.14');
  });

  it('renders booleans as text', () => {
    expect(astToKatex({ kind: 'LiteralBoolean', props: { value: true }, children: [] })).toBe('\\text{true}');
  });

  it('renders infinity and null as symbols', () => {
    expect(astToKatex({ kind: 'LiteralInfinity', props: {}, children: [] })).toBe('\\infty');
    expect(astToKatex({ kind: 'NullExpression', props: {}, children: [] })).toBe('\\text{null}');
  });
});

describe('astToKatex — names & chains', () => {
  it('auto-subscripts names on the first underscore', () => {
    expect(astToKatex(ref('T_busbar'))).toBe('T_{\\text{busbar}}');
  });

  it('wraps non-underscored names in \\text', () => {
    expect(astToKatex(ref('temp'))).toBe('\\text{temp}');
  });

  it('preserves qualified enum references', () => {
    expect(astToKatex(ref('Status::Active'))).toBe('\\mathrm{Status{::}Active}');
  });

  it('joins feature chains with dots', () => {
    const tex = astToKatex(chain(ref('busbar'), ref('temp')));
    expect(tex).toBe('\\text{busbar}.\\text{temp}');
  });
});

describe('astToKatex — binary operators', () => {
  it('renders comparisons with \\geq / \\leq', () => {
    expect(astToKatex(op('>=', ref('temp'), num(90)))).toBe('\\text{temp} \\geq 90');
    expect(astToKatex(op('<=', ref('temp'), num(96)))).toBe('\\text{temp} \\leq 96');
  });

  it('renders multiplication as \\cdot', () => {
    expect(astToKatex(op('*', ref('a'), ref('b')))).toContain('\\cdot');
  });

  it('renders division as \\frac{}{}', () => {
    expect(astToKatex(op('/', ref('a'), ref('b')))).toBe('\\frac{\\text{a}}{\\text{b}}');
  });

  it('renders logical ops with \\wedge / \\vee', () => {
    expect(astToKatex(op('and', ref('p'), ref('q')))).toContain('\\wedge');
    expect(astToKatex(op('or', ref('p'), ref('q')))).toContain('\\vee');
  });
});

describe('astToKatex — associativity & precedence', () => {
  it('left-associative subtraction: (a - b) - c has no parens on lhs', () => {
    // Parser emits ((a - b) - c) as op('-', op('-', a, b), c)
    const tree = op('-', op('-', ref('a'), ref('b')), ref('c'));
    const tex = astToKatex(tree);
    expect(tex).toBe('\\text{a} - \\text{b} - \\text{c}');
  });

  it('subtraction where the RHS is itself subtraction parenthesizes', () => {
    // This is an abnormal right-nested tree (a - (b - c)) — must paren.
    const tree = op('-', ref('a'), op('-', ref('b'), ref('c')));
    const tex = astToKatex(tree);
    expect(tex).toMatch(/\\text\{a\} - \\left\(\\text\{b\} - \\text\{c\}\\right\)/);
  });

  it('right-associative exponentiation: a ** (b ** c) needs no parens', () => {
    const tree = op('**', ref('a'), op('**', ref('b'), ref('c')));
    const tex = astToKatex(tree);
    // ^ on the outer uses rhs as-is because its prec == pow
    expect(tex).toContain('^{');
    expect(tex).not.toMatch(/\\left\(.*\\text\{b\}.*\\text\{c\}.*\\right\)/);
  });

  it('multiplication binds tighter than addition: a + b * c has no parens on rhs', () => {
    const tree = op('+', ref('a'), op('*', ref('b'), ref('c')));
    const tex = astToKatex(tree);
    expect(tex).not.toContain('\\left(');
  });

  it('addition inside multiplication parenthesizes: (a + b) * c', () => {
    const tree = op('*', op('+', ref('a'), ref('b')), ref('c'));
    const tex = astToKatex(tree);
    expect(tex).toMatch(/\\left\(\\text\{a\} \+ \\text\{b\}\\right\)/);
  });
});

describe('astToKatex — unary', () => {
  it('renders unary minus', () => {
    const tree = op('-', ref('x'));
    expect(astToKatex(tree)).toBe('-\\text{x}');
  });

  it('renders logical not with \\neg', () => {
    const tree = op('not', ref('flag'));
    expect(astToKatex(tree)).toContain('\\neg');
  });
});

describe('astToKatex — conditional', () => {
  it('renders if as a cases block', () => {
    const tree = op('if', ref('cond'), int(1), int(0));
    const tex = astToKatex(tree);
    expect(tex).toContain('\\begin{cases}');
    expect(tex).toContain('\\text{if }');
    expect(tex).toContain('\\text{otherwise}');
  });
});

describe('astToKatex — invocations', () => {
  it('renders abs as pipes', () => {
    const tree = call('abs', ref('x'));
    expect(astToKatex(tree)).toBe('\\left|\\text{x}\\right|');
  });

  it('renders sqrt as \\sqrt{}', () => {
    const tree = call('sqrt', op('+', ref('x'), ref('y')));
    expect(astToKatex(tree)).toBe('\\sqrt{\\text{x} + \\text{y}}');
  });

  it('renders sin/cos/etc. as \\sin(...)', () => {
    expect(astToKatex(call('sin', ref('t')))).toBe('\\sin\\left(\\text{t}\\right)');
  });

  it('renders arbitrary functions as \\operatorname', () => {
    expect(astToKatex(call('myFn', ref('a'), ref('b')))).toContain('\\operatorname{myFn}');
  });
});

describe('astToKatex — collection ops', () => {
  it('renders arrow-call select with binding', () => {
    const tree: ExpressionAstNode = {
      kind: 'InvocationExpression',
      name: 'select',
      props: {},
      children: [
        ref('items'),
        { kind: 'FeatureReferenceExpression', name: 'x', props: { isBodyParameter: true }, children: [] },
        op('>', ref('x'), int(0)),
      ],
    };
    const tex = astToKatex(tree);
    expect(tex).toContain('\\rightarrow\\operatorname{select}');
    expect(tex).toContain('\\mapsto');
  });
});

describe('astToKatex — SysML-specific operators (rendered as words)', () => {
  it('renders @ as "istype" (spec synonym)', () => {
    const tex = astToKatex(op('@', ref('x'), ref('Integer')));
    expect(tex).toContain('\\mathrel{\\text{istype}}');
  });

  it('renders istype the same as @', () => {
    const atForm = astToKatex(op('@', ref('x'), ref('Integer')));
    const istypeForm = astToKatex(op('istype', ref('x'), ref('Integer')));
    expect(atForm).toBe(istypeForm);
  });

  it('renders @@ as "metatype"', () => {
    const tex = astToKatex(op('@@', ref('y'), ref('String')));
    expect(tex).toContain('\\mathrel{\\text{metatype}}');
  });

  it('renders hastype as a word', () => {
    const tex = astToKatex(op('hastype', ref('x'), ref('Integer')));
    expect(tex).toContain('\\mathrel{\\text{hastype}}');
  });

  it('renders ?? as the word "else"', () => {
    const tex = astToKatex(op('??', ref('a'), ref('b')));
    expect(tex).toContain('\\mathrel{\\text{else}}');
    expect(tex).not.toContain('?');
  });

  it('renders as/meta as relational words', () => {
    expect(astToKatex(op('as', ref('x'), ref('Integer')))).toContain('\\mathrel{\\text{as}}');
    expect(astToKatex(op('meta', ref('x'), ref('Integer')))).toContain('\\mathrel{\\text{meta}}');
  });

  it('renders === / !== as reference equality words (not \\equiv)', () => {
    const eq = astToKatex(op('===', ref('a'), ref('b')));
    const ne = astToKatex(op('!==', ref('a'), ref('b')));
    expect(eq).toContain('\\mathrel{\\text{refEq}}');
    expect(ne).toContain('\\mathrel{\\text{refNotEq}}');
    expect(eq).not.toContain('\\equiv');
  });

  it('renders range .. as upright double-dot', () => {
    const tex = astToKatex(op('..', int(1), int(10)));
    expect(tex).toContain('\\mathbin{..}');
  });

  it('renders unary ~ as word "bitnot" (not math \\sim)', () => {
    const tex = astToKatex(op('~', ref('flags')));
    expect(tex).toContain('\\text{bitnot}');
    expect(tex).not.toContain('\\sim');
  });
});

describe('astToKatex — index', () => {
  it('renders arr#(2) with the # glyph', () => {
    const tree: ExpressionAstNode = {
      kind: 'IndexExpression',
      props: {},
      children: [ref('arr'), int(2)],
    };
    expect(astToKatex(tree)).toBe('\\text{arr}\\#(2)');
  });
});

describe('astToKatex — unknown kinds', () => {
  it('renders an obvious placeholder that surfaces the gap', () => {
    const tree: ExpressionAstNode = { kind: 'SomeFutureExpression', props: {}, children: [] };
    expect(astToKatex(tree)).toBe('\\text{?\\langle SomeFutureExpression\\rangle}');
  });
});
