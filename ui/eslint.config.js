// Architecture boundaries mirror ARCHITECTURE.md ("Rules").

import js from "@eslint/js";
import { createTypeScriptImportResolver } from "eslint-import-resolver-typescript";
import prettier from "eslint-config-prettier";
import { importX } from "eslint-plugin-import-x";
import svelte from "eslint-plugin-svelte";
import { readdirSync } from "node:fs";
import { defineConfig } from "eslint/config";
import tseslint from "typescript-eslint";

const features = readdirSync("src/features", { withFileTypes: true })
  .filter((e) => e.isDirectory())
  .map((e) => e.name);

/** A feature is reached from outside only through its index. */
const featureZones = features.map((f) => ({
  target: ["./src/app", ...features.filter((g) => g !== f).map((g) => `./src/features/${g}`)],
  from: `./src/features/${f}`,
  except: ["./index.ts"],
  message: `Import features/${f} through its index.ts.`,
}));

const connection = { group: ["**/connection", "**/connection.ts"] };

const svelteImports = { group: ["svelte", "svelte/*", "**/*.svelte", "**/*.svelte.ts", "**/*.svelte.js"] };

const manifestShapes = {
  group: ["FileChange", "Manifest", "ReviewUpdate"].flatMap((name) => [
    `**/generated/${name}`,
    `**/generated/${name}.ts`,
  ]),
  message: "Read the split types of state/reviewModel.ts; only the changeset store handles the manifest's shapes.",
};

export default defineConfig(
  { ignores: ["src/generated/**", "dist/**", "src-tauri/**", "node_modules/**"] },
  js.configs.recommended,
  tseslint.configs.strictTypeChecked,
  tseslint.configs.stylisticTypeChecked,
  svelte.configs.recommended,
  importX.flatConfigs.typescript,
  {
    languageOptions: {
      parserOptions: { projectService: true, extraFileExtensions: [".svelte"] },
    },
    settings: { "import-x/resolver-next": [createTypeScriptImportResolver()] },
    rules: {
      "import-x/no-cycle": "error",
      "no-undef": "off", // the TS checker reports undefined names
      "@typescript-eslint/restrict-template-expressions": ["error", { allowNumber: true }],
      "@typescript-eslint/no-confusing-void-expression": ["error", { ignoreArrowShorthand: true }],
    },
  },
  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: { parserOptions: { parser: tseslint.parser } },
  },
  {
    // The TS program cannot see through `.svelte` imports and `<script module>` types, so
    // they are `any` or error types here; svelte-check covers these files. The parser also
    // models `{#snippet}` declarations as void functions, so `{@render snippet(...)}` looks
    // like a void expression in an expression.
    files: ["**/*.svelte"],
    rules: {
      "@typescript-eslint/no-confusing-void-expression": "off",
      "@typescript-eslint/no-unsafe-argument": "off",
      "@typescript-eslint/no-unsafe-assignment": "off",
      "@typescript-eslint/no-unsafe-call": "off",
      "@typescript-eslint/no-unsafe-member-access": "off",
    },
  },
  {
    files: ["src/**"],
    rules: {
      "import-x/no-restricted-paths": [
        "error",
        {
          zones: [
            { target: "./src/lib", from: ["./src/features", "./src/state", "./src/app"] },
            { target: "./src/state", from: ["./src/features", "./src/app"] },
            { target: "./src/features", from: "./src/app" },
            ...featureZones,
          ],
        },
      ],
    },
  },
  {
    // Only state/backend.ts talks to the connection, and the connection's tests. The manifest's shapes mix
    // the view's structure, the live review state and the changeset's facts: only the store, the client and
    // the tests handle them, everything else reads the split types of `state/reviewModel.ts`.
    files: ["src/**"],
    ignores: ["src/state/backend.ts", "src/state/connection.test.ts"],
    rules: { "no-restricted-imports": ["error", { patterns: [connection, manifestShapes] }] },
  },
  {
    files: ["src/state/changeset.svelte.ts", "src/state/reviewModel.ts", "src/**/*.test.ts"],
    ignores: ["src/state/connection.test.ts"],
    rules: { "no-restricted-imports": ["error", { patterns: [connection] }] },
  },
  {
    files: ["src/features/review/editor/**"],
    ignores: ["src/**/*.test.ts"],
    rules: { "no-restricted-imports": ["error", { patterns: [connection, manifestShapes, svelteImports] }] },
  },
  {
    files: ["src/features/review/editor/**/*.test.ts"],
    rules: { "no-restricted-imports": ["error", { patterns: [connection, svelteImports] }] },
  },
  {
    // A view draws review state it pulls from the `ReviewSource` when it draws, and keeps none: the
    // CodeMirror fields hold what they drew, and are only ever fed from the source.
    files: ["src/features/review/editor/**"],
    ignores: ["src/**/*.test.ts"],
    rules: {
      "no-restricted-syntax": [
        "error",
        {
          selector: "PropertyDefinition TSTypeReference[typeName.name=/^(FileReview|FileState|Completion|HunkState)$/]",
          message: "Views pull review state from the ReviewSource; they do not keep it.",
        },
        {
          selector: "TSPropertySignature TSTypeReference[typeName.name=/^(FileReview|FileState|Completion)$/]",
          message: "Views pull review state from the ReviewSource; they do not keep it.",
        },
      ],
    },
  },
  {
    // The module hooks and the browser stubs are untyped glue around node's and the compilers' APIs.
    files: ["test/harness/*.mjs"],
    rules: {
      "@typescript-eslint/no-unsafe-assignment": "off",
      "@typescript-eslint/no-unsafe-call": "off",
      "@typescript-eslint/no-unsafe-member-access": "off",
      "@typescript-eslint/no-unsafe-return": "off",
      "@typescript-eslint/no-empty-function": "off",
    },
  },
  prettier,
  svelte.configs.prettier,
);
