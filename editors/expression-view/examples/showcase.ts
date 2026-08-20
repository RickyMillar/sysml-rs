import { astToKatex, renderExpression } from '../src';
import { FIXTURES } from './fixtures';

const grid = document.getElementById('grid')!;

for (const fixture of FIXTURES) {
  const card = document.createElement('section');
  card.className = 'card';

  const title = document.createElement('h2');
  title.textContent = fixture.title;
  card.appendChild(title);

  if (fixture.result.source) {
    const src = document.createElement('div');
    src.className = 'source';
    src.textContent = fixture.result.source;
    card.appendChild(src);
  }

  const math = document.createElement('div');
  math.className = 'math';
  card.appendChild(math);

  if (fixture.result.ast) {
    try {
      renderExpression(math, fixture.result, { displayMode: false });
    } catch (err) {
      math.textContent = `render error: ${(err as Error).message}`;
    }

    const details = document.createElement('details');
    const summary = document.createElement('summary');
    summary.textContent = 'show TeX + AST';
    details.appendChild(summary);

    const tex = document.createElement('pre');
    tex.className = 'tex';
    tex.textContent = astToKatex(fixture.result.ast);
    details.appendChild(tex);

    const ast = document.createElement('pre');
    ast.className = 'ast';
    ast.textContent = JSON.stringify(fixture.result.ast, null, 2);
    details.appendChild(ast);

    card.appendChild(details);
  }

  grid.appendChild(card);
}
