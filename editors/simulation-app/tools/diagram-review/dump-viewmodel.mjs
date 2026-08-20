/**
 * Raw ViewModel payload dump — no browser, just the API.
 *
 * Prints every node and every edge (kind, label, endpoints) for ONE view, so
 * a question about WHAT the composer emitted is answered from the wire bytes
 * instead of from reading Rust. Use this before forming any hypothesis about
 * duplicate labels, missing edges, or wrong stereotypes.
 *
 * Usage (needs api :8080 running in the SAME shell invocation):
 *   node tools/diagram-review/dump-viewmodel.mjs <workspace-root> <ViewName>
 */
const API = 'http://127.0.0.1:8080';
const ROOT = process.argv[2];
const WANT = process.argv[3];
const cmd = async (n, p) => {
  const r = await fetch(`${API}/api/command`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ command: n, params: p }),
  });
  const j = await r.json();
  if (j.error) throw new Error(`${n}: ${JSON.stringify(j.error)}`);
  return j;
};

await cmd('sysml.load_workspace', { root: ROOT });
const v = await cmd('sysml.query', {
  uri: '__workspace__',
  spec: {
    filter: { type: 'view', viewpoint_id: null },
    projection: 'summary_expand',
    sort: [{ field: 'name', dir: 'asc' }],
    limit: 1000,
  },
});
const row = (v.rows ?? []).find((r) => r.name === WANT);
if (!row) {
  console.error(`view ${WANT} not found; have: ${(v.rows ?? []).map((r) => r.name).join(', ')}`);
  process.exit(1);
}
const vm = await cmd('sysml.diagram.viewmodel', {
  uri: row.source_span?.file?.replace(/^file:\/\//, '') ?? '__workspace__',
  view_usage_id: row.id,
  expanded_ids: [],
});
console.log(JSON.stringify(vm, null, 2));
