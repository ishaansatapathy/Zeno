// Builds the extension into dist/, which is what Chrome loads ("Load unpacked").
//
//   npm run build          one build
//   npm run dev            rebuild whenever a file changes

import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { build, context } from "esbuild";

import { icon } from "./icons.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const src = path.join(root, "src");
const dist = path.join(root, "dist");
const watch = process.argv.includes("--watch");

const scripts = {
  background: "background/index.ts",
  popup: "popup/popup.ts",
  offscreen: "offscreen/offscreen.ts",
  "writer.worker": "offscreen/writer.worker.ts",
  bubble: "bubble/bubble.ts",
  "bubble-inject": "bubble/bubble-inject.ts",
  permissions: "permissions/permissions.ts",
};

const options = {
  entryPoints: Object.fromEntries(Object.entries(scripts).map(([name, file]) => [name, path.join(src, file)])),
  outdir: dist,
  bundle: true,
  // One plain script each, as a service worker and a content script need. No modules, no loaders.
  format: "iife",
  target: "chrome116",
  sourcemap: true,
  logLevel: "info",
};

const pages = [
  ["popup/popup.html", "popup.html"],
  ["popup/popup.css", "popup.css"],
  ["offscreen/offscreen.html", "offscreen.html"],
  ["bubble/bubble.html", "bubble.html"],
  ["permissions/permissions.html", "permissions.html"],
  ["permissions/permissions.css", "permissions.css"],
];

const fonts = [
  ["@fontsource/instrument-serif/files/instrument-serif-latin-400-normal.woff2", "instrument-serif-normal.woff2"],
  ["@fontsource/instrument-serif/files/instrument-serif-latin-400-italic.woff2", "instrument-serif-italic.woff2"],
  ["@fontsource-variable/instrument-sans/files/instrument-sans-latin-wght-normal.woff2", "instrument-sans.woff2"],
  ["@fontsource/ibm-plex-mono/files/ibm-plex-mono-latin-500-normal.woff2", "plex-mono-500.woff2"],
];

async function copyStatic() {
  const { version } = JSON.parse(await readFile(path.join(root, "package.json"), "utf8"));
  const manifest = JSON.parse(await readFile(path.join(src, "manifest.json"), "utf8"));
  manifest.version = version;
  await writeFile(path.join(dist, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  for (const [from, to] of pages) await cp(path.join(src, from), path.join(dist, to));
  await mkdir(path.join(dist, "fonts"), { recursive: true });
  for (const [from, to] of fonts) await cp(path.join(root, "node_modules", from), path.join(dist, "fonts", to));
  await mkdir(path.join(dist, "icons"), { recursive: true });
  for (const size of [16, 32, 48, 128]) await writeFile(path.join(dist, "icons", `${size}.png`), icon(size));
}

await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });
await copyStatic();
if (watch) {
  const ctx = await context(options);
  await ctx.watch();
  console.log("Watching. Reload the extension in chrome://extensions after a change.");
} else {
  await build(options);
}
