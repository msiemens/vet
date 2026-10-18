import assert from "node:assert/strict";
import { test } from "node:test";
import { resolveMenu } from "../../lib/commandMenu.ts";
import { CommandRegistry } from "../../lib/commands.ts";
import type { CommandContext } from "../../state/commands.ts";
import { recentCommands, recentMenu } from "./recentCommands.ts";

const base: CommandContext = { repoOpen: true, reviewOpen: false, canQuit: true, diffView: false, hunk: -1 };

function setup() {
  const calls: string[] = [];
  const registry = new CommandRegistry<CommandContext>(base, "Ctrl");
  registry.register(
    ...recentCommands({
      open: (path) => calls.push(`open ${path}`),
      forget: (path) => calls.push(`forget ${path}`),
      remove: (path) => calls.push(`remove ${path}`),
    }),
  );
  return { registry, calls };
}

void test("the entries are on offer for the row the menu was opened on, and act on it", () => {
  const { registry, calls } = setup();
  const context = registry.context({ recentProject: { path: "/b" } });
  const entries = (menuContext: typeof context) =>
    resolveMenu(registry, recentMenu, menuContext).filter((i) => !i.separator);
  assert.ok(entries(context).length > 0 && entries(context).every((i) => i.enabled !== false));
  assert.ok(entries(registry.context()).every((i) => i.enabled === false));
  registry.run("recent.open", context);
  registry.run("recent.forget", context);
  registry.run("recent.remove", context);
  assert.deepEqual(calls, ["open /b", "forget /b", "remove /b"]);
});

void test("removing the project is offered after forgetting it", () => {
  const { registry } = setup();
  const labels = resolveMenu(registry, recentMenu, registry.context({ recentProject: { path: "/b" } }))
    .filter((i) => !i.separator)
    .map((i) => i.label);
  assert.deepEqual(labels, ["Open", "Remove from Recent Projects", "Remove project…"]);
});

void test("without a row neither entry does anything", () => {
  const { registry, calls } = setup();
  assert.equal(registry.run("recent.open", registry.context()), false);
  assert.equal(registry.run("recent.forget", registry.context()), false);
  assert.equal(registry.run("recent.remove", registry.context()), false);
  assert.deepEqual(calls, []);
});
