/**
 * VariableRow — structural render contract.
 *
 * Uses react-dom/server to exercise the render path without a DOM. These
 * tests pin the *structure* of the row: presence of the pill, value,
 * sparkline, and aria attributes. Interaction (click, keyboard) is
 * covered in Playwright.
 */

import { describe, it, expect } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import { VariableRow } from '../VariableRow';
import { buildTree } from '../VariableTree';

function leafNode(name: string, opts: Record<string, unknown> = {}) {
  const tree = buildTree([{ name, value: 42, ...opts } as any]);
  // Descend to the leaf.
  let node = tree[0];
  while (node.children.length > 0) node = node.children[0];
  return node;
}

describe('VariableRow — leaf render', () => {
  it('renders the leaf label and formatted value', () => {
    const node = leafNode('circuit.voltage', { unit: 'V' });
    const html = renderToStaticMarkup(
      createElement(VariableRow, {
        node,
        collapsed: false,
        onToggleCollapse: () => {},
        selected: false,
        onSelect: () => {},
        pinned: false,
        showSparkline: false,
        sparklineSamples: [],
        onContextMenu: () => {},
        onActivate: () => {},
      }),
    );
    expect(html).toContain('voltage');
    expect(html).toContain('42');
    expect(html).toContain(' V'); // unit rendered alongside value
  });

  it('renders the constraint pill when a verdict is present', () => {
    const node = leafNode('breaker.current', { constraint: 'fail' });
    const html = renderToStaticMarkup(
      createElement(VariableRow, {
        node,
        collapsed: false,
        onToggleCollapse: () => {},
        selected: false,
        onSelect: () => {},
        pinned: false,
        showSparkline: false,
        sparklineSamples: [],
        onContextMenu: () => {},
        onActivate: () => {},
      }),
    );
    // Rendered via the shared VerdictBadge (R2.5) in compact size.
    expect(html).toContain('data-testid="variable-pill-breaker.current"');
    expect(html).toContain('data-verdict="fail"');
    // Aria label comes from VerdictBadge: "Verdict: Fail (<node.path>)".
    expect(html).toContain('aria-label="Verdict: Fail (breaker.current)"');
  });

  it('omits the pill for entries without a constraint', () => {
    const node = leafNode('stateless', {});
    const html = renderToStaticMarkup(
      createElement(VariableRow, {
        node,
        collapsed: false,
        onToggleCollapse: () => {},
        selected: false,
        onSelect: () => {},
        pinned: false,
        showSparkline: false,
        sparklineSamples: [],
        onContextMenu: () => {},
        onActivate: () => {},
      }),
    );
    expect(html).not.toContain('variable-pill-');
  });

  it('renders the sparkline when enabled and enough samples are provided', () => {
    const node = leafNode('trace');
    const html = renderToStaticMarkup(
      createElement(VariableRow, {
        node,
        collapsed: false,
        onToggleCollapse: () => {},
        selected: false,
        onSelect: () => {},
        pinned: false,
        showSparkline: true,
        sparklineSamples: [0, 1, 2, 3, 4, 5],
        onContextMenu: () => {},
        onActivate: () => {},
      }),
    );
    expect(html).toContain('<svg');
    expect(html).toContain('<polyline');
  });

  it('renders the pinned marker when pinned', () => {
    const node = leafNode('favourite');
    const html = renderToStaticMarkup(
      createElement(VariableRow, {
        node,
        collapsed: false,
        onToggleCollapse: () => {},
        selected: false,
        onSelect: () => {},
        pinned: true,
        showSparkline: false,
        sparklineSamples: [],
        onContextMenu: () => {},
        onActivate: () => {},
      }),
    );
    // Material symbol name "push_pin" is the canonical glyph.
    expect(html).toContain('push_pin');
  });
});

describe('VariableRow — group render', () => {
  it('renders the expand chevron and the leaf count badge', () => {
    const tree = buildTree([
      { name: 'pkg.a', value: 1 },
      { name: 'pkg.b', value: 2 },
    ]);
    const html = renderToStaticMarkup(
      createElement(VariableRow, {
        node: tree[0],
        collapsed: true,
        onToggleCollapse: () => {},
        selected: false,
        onSelect: () => {},
        pinned: false,
        showSparkline: false,
        sparklineSamples: [],
        onContextMenu: () => {},
        onActivate: () => {},
      }),
    );
    expect(html).toContain('aria-expanded="false"');
    expect(html).toContain('chevron_right');
    expect(html).toContain('>2<'); // leafCount badge
  });
});
