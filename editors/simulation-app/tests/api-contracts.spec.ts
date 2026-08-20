/**
 * API contract tests for the sysml-api backend.
 *
 * Pure HTTP tests -- no browser needed, uses Playwright's request context.
 * Validates every command response shape against the real backend.
 *
 * Requires:
 *   - API server: cargo run -p sysml-api (port 8080)
 */
import { test, expect, APIRequestContext } from '@playwright/test';
import { repoPath } from './repo-paths';

const API = 'http://localhost:8080';

// ── Fixture paths ──────────────────────────────────────────────────────
//
// `portFlow` used to be `editors/diagram/examples/demo-port-flow.sysml`,
// It is re-pointed at the port-message-delivery example, which carries what
// the flow/trace contracts read: two ports and a `message_channel` flow
// between them. Checked against a live backend — `sysml.flow.inspect` returns
// every field asserted below (ports with name/key/direction/owner/conjugated,
// a flow with id/source/target/succession) and reports zero diagnostics on it.
// `delivery` comes back empty, but no single-file model in the corpus
// populates it: delivery rows come from a run, not from static inspection,
// and the assertion below only requires the array to exist.
//
// The matching `demo-trade-study.sysml` has NO replacement — see the skipped
// 'Trade study' describe below for why — so there is no `tradeStudy` entry.

const FIXTURES = {
  trafficLight: repoPath('crates/tooling/sysml-cli/fixtures/traffic_light.sysml'),
  verification: repoPath('crates/tooling/sysml-cli/fixtures/test_verification.sysml'),
  portFlow: repoPath('examples/port-message-delivery/PortMessageDelivery.sysml'),
};

// ── Helpers ─────────────────────────────────────────────────────────────

async function cmd(
  request: APIRequestContext,
  command: string,
  params: Record<string, unknown> = {},
) {
  const r = await request.post(`${API}/api/command`, {
    data: { command, params },
  });
  expect(r.ok()).toBeTruthy();
  return r.json();
}

/** Start a simulation via the REST endpoint (not command dispatch). Returns { session_key, initial_state }. */
async function startSimulation(request: APIRequestContext, uri: string, smName: string) {
  const r = await request.post(`${API}/sessions/simulate/start`, {
    data: { uri, sm_name: smName },
  });
  expect(r.ok()).toBeTruthy();
  return r.json();
}

/** Step a simulation via REST. Returns { state, available_transitions, completed, ... }. */
async function stepSimulation(request: APIRequestContext, key: string, event?: string) {
  const r = await request.post(`${API}/sessions/${encodeURIComponent(key)}/step`, {
    data: event ? { event } : undefined,
  });
  expect(r.ok()).toBeTruthy();
  return r.json();
}

async function loadFile(request: APIRequestContext, path: string): Promise<string> {
  const r = await request.post(`${API}/files`, { data: { path } });
  expect(r.ok()).toBeTruthy();
  const body = await r.json();
  return body.uri;
}

// ── Shared state ────────────────────────────────────────────────────────

let trafficLightUri: string;
let verificationUri: string;
let portFlowUri: string;

// Track session IDs for cleanup
const sessionIdsToClean: string[] = [];

test.beforeAll(async ({ request }) => {
  trafficLightUri = await loadFile(request, FIXTURES.trafficLight);
  verificationUri = await loadFile(request, FIXTURES.verification);
  portFlowUri = await loadFile(request, FIXTURES.portFlow);
});

test.afterAll(async ({ request }) => {
  // Best-effort cleanup of all sessions created during tests
  for (const id of sessionIdsToClean) {
    await cmd(request, 'sysml.sessions.stop', { session_id: id }).catch(() => {});
  }
});

// ── Health & file loading ───────────────────────────────────────────────

test.describe('Health & file loading', () => {
  test('GET /health returns status ok and version string', async ({ request }) => {
    const r = await request.get(`${API}/health`);
    expect(r.ok()).toBeTruthy();
    const body = await r.json();

    expect(body).toHaveProperty('status', 'ok');
    expect(typeof body.version).toBe('string');
    expect(body.version.length).toBeGreaterThan(0);
  });

  test('POST /files returns uri; same file returns same URI', async ({ request }) => {
    const r1 = await request.post(`${API}/files`, {
      data: { path: FIXTURES.trafficLight },
    });
    expect(r1.ok()).toBeTruthy();
    const body1 = await r1.json();
    expect(typeof body1.uri).toBe('string');
    expect(body1.uri.length).toBeGreaterThan(0);

    // Loading the same file again should return the same URI
    const r2 = await request.post(`${API}/files`, {
      data: { path: FIXTURES.trafficLight },
    });
    const body2 = await r2.json();
    expect(body2.uri).toBe(body1.uri);
  });
});

// ── Model tree ──────────────────────────────────────────────────────────

test.describe('Model tree', () => {
  test('GET /models/{uri}/tree returns array of nodes with expected shape', async ({
    request,
  }) => {
    const r = await request.get(`${API}/models/${encodeURIComponent(trafficLightUri)}/tree`);
    expect(r.ok()).toBeTruthy();
    const tree = await r.json();

    expect(Array.isArray(tree)).toBeTruthy();
    expect(tree.length).toBeGreaterThan(0);

    const node = tree[0];
    expect(typeof node.id).toBe('string');
    expect(typeof node.name).toBe('string');
    expect(typeof node.kind).toBe('string');
    expect(Array.isArray(node.children)).toBeTruthy();
  });
});

// ── Session management ──────────────────────────────────────────────────

test.describe('Session management', () => {
  test('sessions.list is initially empty or contains prior sessions', async ({
    request,
  }) => {
    const result = await cmd(request, 'sysml.sessions.list');
    expect(Array.isArray(result)).toBeTruthy();
  });

  test('sessions.quota returns simulation, action, orchestrator quotas', async ({
    request,
  }) => {
    const result = await cmd(request, 'sysml.sessions.quota');

    for (const key of ['simulation', 'action', 'orchestrator']) {
      expect(result).toHaveProperty(key);
      const quota = result[key];
      expect(typeof quota.used).toBe('number');
      expect(typeof quota.cap).toBe('number');
    }
  });

  test('sessions.list after simulate.start has entry with expected fields', async ({
    request,
  }) => {
    // Start a session via REST endpoint
    const startResult = await startSimulation(request, trafficLightUri, 'TrafficLightStates');
    const sessionId = startResult.session_key;
    sessionIdsToClean.push(sessionId);

    const list = await cmd(request, 'sysml.sessions.list');
    expect(Array.isArray(list)).toBeTruthy();
    expect(list.length).toBeGreaterThanOrEqual(1);

    // Find our session
    const entry = list.find((s: Record<string, unknown>) => s.id === sessionId);
    expect(entry).toBeDefined();

    // Validate all expected fields
    const expectedFields: Record<string, string> = {
      id: 'string',
      kind: 'string',
      uri: 'string',
      subsystem_name: 'string',
      tick: 'number',
      time_ms: 'number',
      current_state: 'string',
      completed: 'boolean',
      is_expired: 'boolean',
      history_len: 'number',
      subsystem_count: 'number',
    };

    for (const [field, type] of Object.entries(expectedFields)) {
      expect(entry).toHaveProperty(field);
      expect(typeof entry[field]).toBe(type);
    }

    // fork_point_tick may be null or number
    expect(entry).toHaveProperty('fork_point_tick');

    // Cleanup
    await cmd(request, 'sysml.sessions.stop', { session_id: sessionId });
  });

  test('sessions.reap returns a number', async ({ request }) => {
    const result = await cmd(request, 'sysml.sessions.reap');
    expect(typeof result).toBe('number');
  });
});

// ── Simulation lifecycle ────────────────────────────────────────────────

test.describe('Simulation lifecycle', () => {
  test('simulate.start returns session_key in UUID format', async ({ request }) => {
    const result = await startSimulation(request, trafficLightUri, 'TrafficLightStates');

    expect(typeof result.session_key).toBe('string');
    // UUID format: 8-4-4-4-12 hex chars
    expect(result.session_key).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );

    sessionIdsToClean.push(result.session_key);
  });

  test('simulate.step returns state, available_transitions, completed', async ({
    request,
  }) => {
    const start = await startSimulation(request, trafficLightUri, 'TrafficLightStates');
    const key = start.session_key;
    sessionIdsToClean.push(key);

    const step = await stepSimulation(request, key);

    expect(typeof step.state).toBe('string');
    expect(Array.isArray(step.available_transitions)).toBeTruthy();
    expect(typeof step.completed).toBe('boolean');
  });

  test('sessions.step returns SessionSummary with expected fields', async ({
    request,
  }) => {
    const start = await startSimulation(request, trafficLightUri, 'TrafficLightStates');
    const key = start.session_key;
    sessionIdsToClean.push(key);

    const result = await cmd(request, 'sysml.sessions.step', { session_id: key });

    expect(typeof result.id).toBe('string');
    expect(typeof result.kind).toBe('string');
    expect(typeof result.tick).toBe('number');
    expect(typeof result.time_ms).toBe('number');
    expect(typeof result.current_state).toBe('string');
    expect(typeof result.completed).toBe('boolean');
  });

  test('sessions.info returns summary, subsystems, latest_snapshot', async ({
    request,
  }) => {
    const start = await startSimulation(request, trafficLightUri, 'TrafficLightStates');
    const key = start.session_key;
    sessionIdsToClean.push(key);

    // Step once so there is data
    await stepSimulation(request, key);

    const info = await cmd(request, 'sysml.sessions.info', { session_id: key });

    // summary
    expect(info).toHaveProperty('summary');
    expect(typeof info.summary.id).toBe('string');
    expect(typeof info.summary.kind).toBe('string');
    expect(typeof info.summary.tick).toBe('number');
    expect(typeof info.summary.time_ms).toBe('number');
    expect(typeof info.summary.current_state).toBe('string');
    expect(typeof info.summary.completed).toBe('boolean');

    // subsystems
    expect(Array.isArray(info.subsystems)).toBeTruthy();
    if (info.subsystems.length > 0) {
      const sub = info.subsystems[0];
      expect(typeof sub.name).toBe('string');
      expect(typeof sub.kind_label).toBe('string');
      expect(typeof sub.current_state).toBe('string');
      expect(typeof sub.completed).toBe('boolean');
      expect(Array.isArray(sub.available_transitions)).toBeTruthy();
    }

    // latest_snapshot
    expect(info).toHaveProperty('latest_snapshot');
  });

  test('sessions.fork returns SessionSummary with different id', async ({
    request,
  }) => {
    const start = await startSimulation(request, trafficLightUri, 'TrafficLightStates');
    const parentKey = start.session_key;
    sessionIdsToClean.push(parentKey);

    // Step parent so there is state to fork from
    await stepSimulation(request, parentKey);

    const fork = await cmd(request, 'sysml.sessions.fork', { session_id: parentKey });

    expect(typeof fork.id).toBe('string');
    expect(fork.id).not.toBe(parentKey);
    expect(typeof fork.kind).toBe('string');
    expect(typeof fork.tick).toBe('number');
    expect(typeof fork.current_state).toBe('string');
    expect(typeof fork.completed).toBe('boolean');

    sessionIdsToClean.push(fork.id);
  });

  test('sessions.diff returns structured diff between two sessions', async ({
    request,
  }) => {
    // Create two sessions from the same model and diverge them
    const startA = await startSimulation(request, trafficLightUri, 'TrafficLightStates');
    const idA = startA.session_key;
    sessionIdsToClean.push(idA);

    await stepSimulation(request, idA);

    // Fork to create session B
    const fork = await cmd(request, 'sysml.sessions.fork', { session_id: idA });
    const idB = fork.id;
    sessionIdsToClean.push(idB);

    // Step B further to create divergence
    await cmd(request, 'sysml.sessions.step', { session_id: idB });

    const diff = await cmd(request, 'sysml.sessions.diff', { a_id: idA, b_id: idB });

    expect(typeof diff.a_id).toBe('string');
    expect(typeof diff.b_id).toBe('string');
    expect(typeof diff.current_tick_a).toBe('number');
    expect(typeof diff.current_tick_b).toBe('number');
    expect(Array.isArray(diff.subsystem_diffs)).toBeTruthy();
    expect(Array.isArray(diff.variable_diffs)).toBeTruthy();
  });

  test('sessions.stop returns null or empty response', async ({ request }) => {
    const start = await startSimulation(request, trafficLightUri, 'TrafficLightStates');
    const key = start.session_key;

    const result = await cmd(request, 'sysml.sessions.stop', { session_id: key });

    // Accept null, undefined, empty object, or empty string
    const isEmpty =
      result === null ||
      result === undefined ||
      (typeof result === 'object' && Object.keys(result).length === 0) ||
      result === '';
    expect(isEmpty).toBeTruthy();
  });
});

// ── Constraint checking ─────────────────────────────────────────────────

test.describe('Constraint checking', () => {
  test('constraint.check returns array of constraint results', async ({ request }) => {
    const result = await cmd(request, 'sysml.constraint.check', { uri: verificationUri, overrides: [] });

    expect(Array.isArray(result)).toBeTruthy();
    expect(result.length).toBeGreaterThan(0);

    const entry = result[0];
    expect(typeof entry.name).toBe('string');
    expect(typeof entry.expression).toBe('string');
    // Per-occurrence verdict (replaces the old passed:bool). Pass / Fail /
    // Inconclusive / Error — Inconclusive is distinct from Fail.
    expect(typeof entry.verdict).toBe('string');
    expect(['Pass', 'Fail', 'Inconclusive', 'Error']).toContain(entry.verdict);
    expect(entry).toHaveProperty('actual');     // string | null
    expect(entry).toHaveProperty('expected');   // string | null
    expect(entry).toHaveProperty('message');    // string | null
  });
});

// ── Monte Carlo ─────────────────────────────────────────────────────────

test.describe('Monte Carlo', () => {
  test('montecarlo returns iterations, pass_rate, per_constraint', async ({
    request,
  }) => {
    const result = await cmd(request, 'sysml.montecarlo.run', {
      uri: verificationUri,
      iterations: 50,
    });

    expect(typeof result.iterations).toBe('number');
    expect(result.iterations).toBe(50);
    expect(typeof result.pass_rate).toBe('number');
    expect(typeof result.pass_count).toBe('number');
    expect(typeof result.fail_count).toBe('number');

    expect(Array.isArray(result.per_constraint)).toBeTruthy();
    if (result.per_constraint.length > 0) {
      const pc = result.per_constraint[0];
      expect(typeof pc.name).toBe('string');
      expect(typeof pc.pass_rate).toBe('number');
      expect(typeof pc.pass_count).toBe('number');
      expect(typeof pc.fail_count).toBe('number');
    }
  });
});

// ── Trade study ─────────────────────────────────────────────────────────

// This test needs a model `sysml.trade_study` can compile, and the repo does
// not contain one. `compile_trade_study` (crates/lang/sysml-runtime/src/cases/
// trade_study.rs) resolves an AnalysisCaseUsage by name and reads its child
// PartUsage/ItemUsage elements as the alternatives; there is no analysis
// USAGE anywhere under examples/ or crates/tooling/sysml-cli/fixtures/ (every
// analysis in the tree is an `analysis def`), so nothing can stand in for the
// deleted `demo-trade-study.sysml` and its `materialStudy` case. Un-skip once
// a trade-study fixture with named alternatives is authored.
test.describe.skip('Trade study', () => {
  test('trade_study returns alternatives with scores', async ({ request }) => {
    const result = await cmd(request, 'sysml.trade_study', {
      uri: '<no trade-study fixture in repo — see comment above>',
      study_name: 'materialStudy',
      overrides: [],
    });

    expect(typeof result.study_name).toBe('string');
    expect(Array.isArray(result.alternatives)).toBeTruthy();
    expect(result.alternatives.length).toBeGreaterThan(0);

    const alt = result.alternatives[0];
    expect(typeof alt.name).toBe('string');
    expect(typeof alt.score).toBe('number');

    expect(typeof result.best).toBe('string');
    expect(typeof result.best_score).toBe('number');
  });
});

// ── Flow inspection ─────────────────────────────────────────────────────

test.describe('Flow inspection', () => {
  test('flow.inspect returns ports, flows, delivery, diagnostics', async ({
    request,
  }) => {
    const result = await cmd(request, 'sysml.flow.inspect', { uri: portFlowUri });

    // ports
    expect(Array.isArray(result.ports)).toBeTruthy();
    if (result.ports.length > 0) {
      const port = result.ports[0];
      expect(typeof port.name).toBe('string');
      expect(typeof port.key).toBe('string');
      expect(typeof port.direction).toBe('string');
      expect(typeof port.owner).toBe('string');
      expect(typeof port.conjugated).toBe('boolean');
    }

    // flows
    expect(Array.isArray(result.flows)).toBeTruthy();
    if (result.flows.length > 0) {
      const flow = result.flows[0];
      expect(typeof flow.id).toBe('string');
      expect(typeof flow.source).toBe('string');
      expect(typeof flow.target).toBe('string');
      expect(typeof flow.succession).toBe('boolean');
    }

    // delivery and diagnostics
    expect(Array.isArray(result.delivery)).toBeTruthy();
    expect(Array.isArray(result.diagnostics)).toBeTruthy();
  });
});

// ── Trace ───────────────────────────────────────────────────────────────

test.describe('Trace', () => {
  test('trace returns lifelines and messages arrays', async ({ request }) => {
    const result = await cmd(request, 'sysml.trace', {
      uri: portFlowUri,
      inject_specs: [],
    });

    expect(Array.isArray(result.lifelines)).toBeTruthy();
    expect(Array.isArray(result.messages)).toBeTruthy();
  });
});
