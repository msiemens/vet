// The dependency rules of ARCHITECTURE.md, checked on the import graph.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const src = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)]));
}

/** Every file with the resolved paths of its relative imports, relative to `src`. */
const files = walk(src)
  .filter((f) => /\.(ts|svelte)$/.test(f) && !f.includes(`${sep}generated${sep}`))
  .map((file) => {
    const text = readFileSync(file, "utf8");
    const specs = [...text.matchAll(/(?:from|import)\s*["']([^"']+)["']/g)].map((m) => m[1]);
    const imports = specs.map((s) => (s.startsWith(".") ? relative(src, resolve(dirname(file), s)).split(sep).join("/") : s));
    return { path: relative(src, file).split(sep).join("/"), imports };
  });

const violations = (rule: (path: string, target: string) => boolean) => files.flatMap((f) => f.imports.filter((t) => rule(f.path, t)).map((t) => `${f.path} -> ${t}`));

test("only state/backend.ts imports the connection", () => {
  assert.deepEqual(violations((p, t) => t === "connection" && p !== "state/backend.ts"), []);
});

test("lib imports nothing from features, state or app", () => {
  assert.deepEqual(violations((p, t) => p.startsWith("lib/") && /^(features|state|app)(\/|$)/.test(t)), []);
});

test("state imports nothing from features or app", () => {
  assert.deepEqual(violations((p, t) => p.startsWith("state/") && /^(features|app)(\/|$)/.test(t)), []);
});

test("features do not import app or dev", () => {
  assert.deepEqual(violations((p, t) => p.startsWith("features/") && /^(app|dev)(\/|$)/.test(t)), []);
});

test("a feature reaches another only through its index", () => {
  const bad = violations((p, t) => {
    const from = p.match(/^features\/([^/]+)\//)?.[1];
    const to = t.match(/^features\/([^/]+)(?:\/(.*))?$/);
    return !!from && !!to && to[1] !== from && !!to[2] && to[2] !== "index";
  });
  assert.deepEqual(bad, []);
});

test("app and dev reach features only through their index", () => {
  assert.deepEqual(violations((p, t) => /^(app|dev)\//.test(p) && /^features\/[^/]+\/.+/.test(t) && !/^features\/[^/]+\/index$/.test(t)), []);
});

test("review/editor has no Svelte imports", () => {
  assert.deepEqual(violations((p, t) => p.startsWith("features/review/editor/") && (t === "svelte" || t.startsWith("svelte/") || t.endsWith(".svelte") || t.endsWith(".svelte.ts") || t.endsWith("changeset.svelte"))), []);
});
