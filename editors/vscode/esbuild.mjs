// @ts-check
import * as esbuild from "esbuild";

const production = process.argv.includes("--production");
const watch = process.argv.includes("--watch");

/** @type {esbuild.BuildOptions} Extension host (Node) build */
const extensionBuild = {
  entryPoints: ["src/extension.ts"],
  bundle: true,
  outfile: "dist/extension.js",
  external: ["vscode"],
  format: "cjs",
  platform: "node",
  target: "node18",
  sourcemap: !production,
  minify: production,
  treeShaking: true,
  logLevel: "info",
};

async function main() {
  if (watch) {
    const ctx = await esbuild.context(extensionBuild);
    await ctx.watch();
    console.log("Watching for changes...");
  } else {
    await esbuild.build(extensionBuild);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
