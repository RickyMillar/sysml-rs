/**
 * SessionArchive — IndexedDB-backed session persistence per ADR-005.
 *
 * Uses the `idb` library for a lightweight IndexedDB wrapper. Completed
 * sessions are auto-archived so they survive page reloads and can be
 * loaded into the Compare workspace.
 *
 * Database: `sysml-sessions`
 * Object store: `archives` (keyPath: `id`)
 */

import { openDB, type IDBPDatabase } from 'idb';
import type { SessionDetail } from '../../features/sessions/types';

// ── Schema ─────────────────────────────────────────────────────────────

const DB_NAME = 'sysml-sessions';
const DB_VERSION = 1;
const STORE_NAME = 'archives';

export interface ArchivedSession {
  /** Session ID (primary key). */
  id: string;
  /** Human-readable label (from session summary). */
  label: string | null;
  /** URI of the model this session ran against. */
  uri: string;
  /** Session kind. */
  kind: 'simulation' | 'action' | 'orchestrator';
  /** Unix timestamp (ms) when the session was archived. */
  archivedAt: number;
  /** Final tick count. */
  tick: number;
  /** Final simulation time (ms). */
  timeMs: number;
  /** Full session detail at time of archival. */
  detail: SessionDetail;
  /** System topology snapshot (if available). */
  topology: unknown | null;
  /** Accumulated snapshot history (subset — the ring buffer's contents). */
  snapshotHistory: Array<Record<string, unknown>>;
}

/** Summary returned by listArchivedSessions (lightweight). */
export interface ArchivedSessionSummary {
  id: string;
  label: string | null;
  uri: string;
  kind: 'simulation' | 'action' | 'orchestrator';
  archivedAt: number;
  tick: number;
  timeMs: number;
}

// ── Database initialization ────────────────────────────────────────────

let dbPromise: Promise<IDBPDatabase> | null = null;

function getDb(): Promise<IDBPDatabase> {
  if (!dbPromise) {
    dbPromise = openDB(DB_NAME, DB_VERSION, {
      upgrade(db) {
        if (!db.objectStoreNames.contains(STORE_NAME)) {
          db.createObjectStore(STORE_NAME, { keyPath: 'id' });
        }
      },
    });
  }
  return dbPromise;
}

// ── Public API ─────────────────────────────────────────────────────────

/**
 * Archive a completed session to IndexedDB.
 */
export async function archiveSession(
  id: string,
  data: {
    detail: SessionDetail;
    topology: unknown | null;
    snapshotHistory: Array<Record<string, unknown>>;
  },
): Promise<void> {
  const db = await getDb();
  const record: ArchivedSession = {
    id,
    label: data.detail.summary.label,
    uri: data.detail.summary.uri,
    kind: data.detail.summary.kind,
    archivedAt: Date.now(),
    tick: data.detail.summary.tick,
    timeMs: data.detail.summary.time_ms,
    detail: data.detail,
    topology: data.topology,
    snapshotHistory: data.snapshotHistory,
  };
  await db.put(STORE_NAME, record);
}

/**
 * Load a single archived session by ID.
 */
export async function loadArchivedSession(
  id: string,
): Promise<ArchivedSession | null> {
  const db = await getDb();
  const record = await db.get(STORE_NAME, id);
  return (record as ArchivedSession) ?? null;
}

/**
 * List all archived sessions (lightweight summaries only).
 */
export async function listArchivedSessions(): Promise<ArchivedSessionSummary[]> {
  const db = await getDb();
  const all = (await db.getAll(STORE_NAME)) as ArchivedSession[];
  return all.map((a) => ({
    id: a.id,
    label: a.label,
    uri: a.uri,
    kind: a.kind,
    archivedAt: a.archivedAt,
    tick: a.tick,
    timeMs: a.timeMs,
  }));
}

/**
 * Delete an archived session by ID.
 */
export async function deleteArchivedSession(id: string): Promise<void> {
  const db = await getDb();
  await db.delete(STORE_NAME, id);
}
