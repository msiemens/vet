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
    }),
  );
  return { registry, calls };
}

void test("the menu offers Open and Remove from Recent Projects for a row", () => {
  const { registry } = setup();
  const items = resolveMenu(registry, recentMenu, registry.context({ recentProject: { path: "/a" } }));
  assert.deepEqual(
    items.filter((i) => !i.separator).map((i) => [i.label, i.enabled !== false]),
    [
      ["Open", true],
      ["Remove from Recent Projects", true],
    ],
  );
});

void test("the entries act on the row the menu was opened on", () => {
  const { registry, calls } = setup();
  const context = registry.context({ recentProject: { path: "/b" } });
  registry.run("recent.open", context);
  registry.run("recent.forget", context);
  assert.deepEqual(calls, ["open /b", "forget /b"]);
});

void test("without a row neither entry does anything", () => {
  const { registry, calls } = setup();
  assert.equal(registry.run("recent.open", registry.context()), false);
  assert.equal(registry.run("recent.forget", registry.context()), false);
  assert.deepEqual(calls, []);
});
