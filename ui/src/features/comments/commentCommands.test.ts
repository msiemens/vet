import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../../generated/Thread";
import { resolveMenu } from "../../lib/commandMenu.ts";
import { CommandRegistry } from "../../lib/commands.ts";
import type { CommandContext } from "../../state/commands.ts";
import { attachMenu, threadCommands } from "./commentCommands.ts";

const idle: CommandContext = { repoOpen: true, reviewOpen: true, canQuit: true, diffView: false, hunk: -1 };

const ambiguous = {
  id: 7,
  anchor: {
    type: "ambiguous",
    candidates: [
      { path: "a.rs", start: 1, len: 1 },
      { path: "b.rs", start: 7, len: 3 },
    ],
  },
} as unknown as Thread;

function setup() {
  const attached: [number, number][] = [];
  const registry = new CommandRegistry<CommandContext>(idle, "Ctrl");
  registry.register(...threadCommands({ attach: (thread, at) => attached.push([thread, at]) }));
  return { registry, attached };
}

void test("the attach menu lists a candidate per item and a choice attaches the thread there", () => {
  const { registry, attached } = setup();
  const context = registry.context({ thread: ambiguous });
  assert.deepEqual(
    resolveMenu(registry, attachMenu(ambiguous), context).map((i) => [i.id, i.label]),
    [
      ["thread.attach:0", "a.rs:L2"],
      ["thread.attach:1", "b.rs:L8–10"],
    ],
  );
  assert.equal(registry.run("thread.attach:1", context), true);
  assert.deepEqual(attached, [[7, 1]]);
});

void test("a candidate the thread does not have is not attached", () => {
  const { registry, attached } = setup();
  const context = registry.context({ thread: ambiguous });
  assert.equal(registry.run("thread.attach:2", context), false);
  assert.equal(registry.run("thread.attach:x", context), false);
  assert.equal(registry.run("thread.attach:0", idle), false);
  assert.deepEqual(attached, []);
});
