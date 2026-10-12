// Module hooks that let the UI's own sources run under node: TypeScript is transpiled (node's
// stripper cannot do parameter properties), `*.svelte.ts` modules are compiled by Svelte so the runes
// stores work, and the extensionless relative imports Vite resolves get their `.ts`.

import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

const require = createRequire(import.meta.url);
const compiler = await import(pathToFileURL(require.resolve("svelte/compiler")).href);
const compileModule = compiler.compileModule ?? compiler.default.compileModule;
const ts = (await import(pathToFileURL(require.resolve("typescript")).href)).default;

/** @param {string} specifier @param {object} context @param {Function} next */
export function resolve(specifier, context, next) {
  try {
    return next(specifier, context);
  } catch (error) {
    if (!specifier.startsWith(".") || error?.code !== "ERR_MODULE_NOT_FOUND") throw error;
    try {
      return next(`${specifier}.ts`, context);
    } catch {
      return next(`${specifier}/index.ts`, context);
    }
  }
}

/** @param {string} url @param {object} context @param {Function} next */
export function load(url, context, next) {
  const runes = /\.svelte\.(ts|js)$/.test(url);
  const typescript = url.endsWith(".ts") && !url.includes("/node_modules/");
  if (!runes && !typescript) return next(url, context);
  let code = String(next(url, { ...context, format: "module" }).source);
  if (typescript) {
    code = ts.transpileModule(code, {
      compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, useDefineForClassFields: true },
    }).outputText;
  }
  if (runes) code = compileModule(code, { generate: "client", filename: new URL(url).pathname, dev: false }).js.code;
  return { format: "module", source: code, shortCircuit: true };
}
