/**
 * One-time monaco-editor setup for the simulation app.
 *
 * `@monaco-editor/react` defaults to fetching monaco from a CDN. We
 * bundle it ourselves so the editor works offline and we control the
 * exact version (already pinned via `package.json` dependencies).
 *
 * Both the monaco namespace and the Vite-emitted worker chunk are
 * imported dynamically — keeping them out of the static module graph
 * lets tests import editor descriptors (e.g. `panelRegistry`) without
 * pulling monaco's browser-only initialisation into jsdom.
 */

import { loader } from '@monaco-editor/react';

let configurePromise: Promise<void> | null = null;

export function configureMonaco(): Promise<void> {
  if (configurePromise) return configurePromise;
  configurePromise = (async () => {
    const monaco = await import('monaco-editor');
    const workerMod = (await import(
      // eslint-disable-next-line import/no-unresolved
      'monaco-editor/esm/vs/editor/editor.worker?worker'
    )) as { default: { new (): Worker } };
    const EditorWorker = workerMod.default;

    (self as unknown as { MonacoEnvironment: unknown }).MonacoEnvironment = {
      getWorker(_workerId: string, _label: string) {
        return new EditorWorker();
      },
    };

    loader.config({ monaco });
  })();
  return configurePromise;
}
