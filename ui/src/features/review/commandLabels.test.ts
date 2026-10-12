import assert from "node:assert/strict";
import { test } from "node:test";
import { CommandRegistry } from "../../lib/commands.ts";
import { type AppActions, appCommands } from "../../state/appCommands.ts";
import type { CommandContext } from "../../state/commands.ts";
import { fileCommands, viewCommands } from "./reviewCommands.ts";

/** The commands of the app and the review, with actions that do nothing. */
function registry() {
  const nothing = new Proxy({}, { get: () => () => undefined });
  const registry = new CommandRegistry<CommandContext>(
    { repoOpen: false, reviewOpen: false, canQuit: false, diffView: false, hunk: -1 },
    "Ctrl",
  );
  registry.register(
    ...appCommands(nothing as AppActions),
    ...viewCommands(nothing as Parameters<typeof viewCommands>[0]),
    ...fileCommands(nothing as Parameters<typeof fileCommands>[0]),
  );
  return registry;
}

void test("no two commands share a label, so one name means one action", () => {
  const labels = registry()
    .list()
    .map((c) => c.label);
  assert.deepEqual(
    labels.filter((label, i) => labels.indexOf(label) !== i),
    [],
  );
});

void test("labels are in Title Case", () => {
  const small = new Set(["a", "an", "and", "as", "by", "for", "from", "in", "of", "on", "or", "the", "to"]);
  const wrong = registry()
    .list()
    .map((c) => c.label)
    .filter((label) => label.split(" ").some((word, i) => !/^[A-Z0-9(]/.test(word) && (i === 0 || !small.has(word))));
  assert.deepEqual(wrong, []);
});
