import type { ExpressionAstNode, ExpressionAstResult } from '../src/types';

// Handcrafted fixtures covering every kind emitted by
// crates/tooling/sysml-service/src/expression_ast.rs::is_expression_element.
// Each entry bundles the canonical source text with the AST the Rust
// backend would produce, so the showcase can render source + tree + math.

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

type Fixture = { title: string; result: ExpressionAstResult };

export const FIXTURES: Fixture[] = [
  {
    title: 'Temperature band constraint',
    result: {
      element_id: 'fix-1',
      element_name: 'tempInBand',
      element_kind: 'ConstraintUsage',
      source: 'temp >= 90.0 and temp <= 96.0',
      ast: op(
        'and',
        op('>=', ref('temp'), num(90.0)),
        op('<=', ref('temp'), num(96.0)),
      ),
    },
  },
  {
    title: 'Busbar thermal ODE (non-commutative arithmetic)',
    result: {
      element_id: 'fix-2',
      element_name: 'dT_busbar_dt',
      element_kind: 'CalculationUsage',
      source: '(Q_busbar - 0.8 * (T_busbar - T_circuitBay) - 0.3 * (T_busbar - T_backplane)) / 15.0',
      ast: op(
        '/',
        op(
          '-',
          op(
            '-',
            ref('Q_busbar'),
            op('*', num(0.8), op('-', ref('T_busbar'), ref('T_circuitBay'))),
          ),
          op('*', num(0.3), op('-', ref('T_busbar'), ref('T_backplane'))),
        ),
        num(15.0),
      ),
    },
  },
  {
    title: 'Pythagorean — sqrt + power',
    result: {
      element_id: 'fix-3',
      element_name: 'hypotenuse',
      element_kind: 'AttributeUsage',
      source: 'sqrt(a ** 2 + b ** 2)',
      ast: call(
        'sqrt',
        op('+', op('**', ref('a'), int(2)), op('**', ref('b'), int(2))),
      ),
    },
  },
  {
    title: 'Conditional (cases block)',
    result: {
      element_id: 'fix-4',
      element_name: 'sign',
      element_kind: 'CalculationUsage',
      source: 'if x > 0 ? 1 else -1',
      ast: op('if', op('>', ref('x'), int(0)), int(1), op('-', int(1))),
    },
  },
  {
    title: 'Qualified enum reference',
    result: {
      element_id: 'fix-5',
      element_name: 'breakerType',
      element_kind: 'AttributeUsage',
      source: 'BreakerCurveType::C',
      ast: ref('BreakerCurveType::C'),
    },
  },
  {
    title: 'Arrow-call select with binding',
    result: {
      element_id: 'fix-6',
      element_name: 'positives',
      element_kind: 'AttributeUsage',
      source: 'items->select{in x; x > 0}',
      ast: {
        kind: 'InvocationExpression',
        name: 'select',
        props: {},
        children: [
          ref('items'),
          { kind: 'FeatureReferenceExpression', name: 'x', props: { isBodyParameter: true }, children: [] },
          op('>', ref('x'), int(0)),
        ],
      },
    },
  },
  {
    title: 'Array index',
    result: {
      element_id: 'fix-7',
      element_name: 'third',
      element_kind: 'AttributeUsage',
      source: 'arr#(3)',
      ast: {
        kind: 'IndexExpression',
        props: {},
        children: [ref('arr'), int(3)],
      },
    },
  },
  {
    title: 'Metadata access',
    result: {
      element_id: 'fix-8',
      element_name: 'meta',
      element_kind: 'AttributeUsage',
      source: 'elem.metadata',
      ast: {
        kind: 'MetadataAccessExpression',
        name: 'elem',
        props: {},
        children: [],
      },
    },
  },
  {
    title: 'Classification operators (@ / @@)',
    result: {
      element_id: 'fix-9',
      element_name: 'classify',
      element_kind: 'ConstraintUsage',
      source: 'x @ Integer and y @@ String',
      ast: op(
        'and',
        op('@', ref('x'), ref('Integer')),
        op('@@', ref('y'), ref('String')),
      ),
    },
  },
  {
    title: 'Feature chain',
    result: {
      element_id: 'fix-10',
      element_name: 'busbarTemp',
      element_kind: 'AttributeUsage',
      source: 'busbar.thermal.temperature',
      ast: {
        kind: 'FeatureChainExpression',
        props: {},
        children: [ref('busbar'), ref('thermal'), ref('temperature')],
      },
    },
  },
  {
    title: 'IEC compliance constraint',
    result: {
      element_id: 'fix-11',
      element_name: 'iecTrip',
      element_kind: 'ConstraintUsage',
      source: 'sim_time_ms > maxTime_s * 1000',
      ast: op('>', ref('sim_time_ms'), op('*', ref('maxTime_s'), int(1000))),
    },
  },
  {
    title: 'Range (1..10)',
    result: {
      element_id: 'fix-13',
      element_name: 'window',
      element_kind: 'AttributeUsage',
      source: '1..10',
      ast: op('..', int(1), int(10)),
    },
  },
  {
    title: 'Reference equality (===)',
    result: {
      element_id: 'fix-14',
      element_name: 'sameRef',
      element_kind: 'ConstraintUsage',
      source: 'a === b',
      ast: op('===', ref('a'), ref('b')),
    },
  },
  {
    title: 'Bitwise complement (~)',
    result: {
      element_id: 'fix-15',
      element_name: 'inverted',
      element_kind: 'AttributeUsage',
      source: '~flags',
      ast: op('~', ref('flags')),
    },
  },
  {
    title: 'Null coalescing',
    result: {
      element_id: 'fix-12',
      element_name: 'defaulted',
      element_kind: 'AttributeUsage',
      source: 'override ?? baseline',
      ast: op('??', ref('override'), ref('baseline')),
    },
  },
];
