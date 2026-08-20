/**
 * Repo-relative path resolution for the simulation-app test suites.
 *
 * Workspace and file paths cross the API as absolute paths, so tests must
 * resolve them against the checkout they are running from rather than a
 * hardcoded developer path. Set SYSML_REPO_ROOT to run against a different
 * checkout (e.g. the app under test served from another tree).
 */
import * as fs from 'fs';
import * as path from 'path';
import { fileURLToPath } from 'url';

/** editors/simulation-app/tests */
const HERE = path.dirname(fileURLToPath(import.meta.url));

function resolveRepoRoot(): string {
  const override = process.env.SYSML_REPO_ROOT;
  const root = override ? path.resolve(override) : path.resolve(HERE, '../../..');
  const looksRight =
    fs.existsSync(path.join(root, 'Cargo.toml')) && fs.existsSync(path.join(root, 'crates'));
  if (!looksRight) {
    throw new Error(
      `sysml-rs checkout not found at '${root}' (expected Cargo.toml and crates/ there). ` +
        `Set SYSML_REPO_ROOT to the repository root.`,
    );
  }
  return root;
}

export const REPO_ROOT = resolveRepoRoot();

/** Absolute path to a repo-relative location. */
export function repoPath(...segments: string[]): string {
  return path.join(REPO_ROOT, ...segments);
}
