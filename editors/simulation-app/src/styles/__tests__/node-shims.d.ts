/**
 * Minimal ambient typings for the Node builtins used by
 * tokens-compat-gate.test.ts.
 *
 * The app's tsconfig has no `@types/node` (frontend code never touches
 * Node builtins), but this test runs under Vitest's Node process and
 * needs `fs`/`path` to walk `src/` for the compat-alias CI gate. Adding
 * `@types/node` as a real dependency would pull in the full Node lib
 * surface for a two-function need; these narrow shims keep the test
 * dependency-free while still typechecking under `tsc --noEmit`.
 */
declare module 'fs' {
  interface DirentLike {
    name: string;
    isDirectory(): boolean;
  }
  export function readFileSync(path: string, encoding: 'utf8'): string;
  export function readdirSync(
    path: string,
    options: { withFileTypes: true },
  ): DirentLike[];
  const fsDefault: {
    readFileSync: typeof readFileSync;
    readdirSync: typeof readdirSync;
  };
  export default fsDefault;
}

declare module 'path' {
  export function resolve(...segments: string[]): string;
  export function join(...segments: string[]): string;
  export function extname(path: string): string;
  export function relative(from: string, to: string): string;
  const pathDefault: {
    resolve: typeof resolve;
    join: typeof join;
    extname: typeof extname;
    relative: typeof relative;
  };
  export default pathDefault;
}

declare const __dirname: string;
declare const __filename: string;
