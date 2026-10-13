// The commands of every feature are registered together, as only the window does; the features' indexes
// import components, which these tests cannot load.
/* eslint-disable import-x/no-restricted-paths */
import assert from "node:assert/strict";
import { test } from "node:test";
import { CommandRegistry } from "../../lib/commands.ts";
import { appCommands } from "../../state/appCommands.ts";
import type { CommandContext } from "../../state/commands.ts";
import { threadCommands } from "../comments/commentCommands.ts";
import { queueCommands } from "../queue/queueCommands.ts";
import { fileCommands, viewCommands } from "../review/reviewCommands.ts";
import { recentCommands } from "../welcome/recentCommands.ts";
import { PALETTE_COMMANDS, paletteCommands } from "./paletteCommands.ts";
import { actionCandidates, commandEntries } from "./paletteSources.ts";

/** Every command the window registers, with actions that do nothing. */
function registry() {
  const nothing = new Proxy({}, { get: () => () => undefined }) as never;
  const registry = new CommandRegistry<CommandContext>(
    { repoOpen: false, reviewOpen: false, canQuit: false, diffView: false, hunk: -1 },
    "Ctrl",
  );
  registry.register(
    ...appCommands(nothing),
    ...queueCommands(nothing),
    ...recentCommands(nothing),
    ...viewCommands(nothing),
    ...fileCommands(nothing),
    ...threadCommands(nothing),
    ...paletteCommands(nothing),
  );
  return registry;
}

/** Windows of different states, which decide what the commands say of themselves. */
const CONTEXTS: Partial<CommandContext>[] = [
  {},
  { repoOpen: true, queueShown: true },
  { repoOpen: true, changesetOpen: true },
  { repoOpen: true, changesetOpen: true, reviewOpen: true, tab: "changes", tabCount: 1, diffView: true },
  { repoOpen: true, changesetOpen: true, reviewOpen: true, tab: "file:src/a.rs", tabCount: 2, rangeShown: true },
  { repoOpen: true, changesetOpen: true, pullRequest: true, manyCommits: true, byCommit: true },
];

void test("every action that cannot run now says why, in every state of the window", () => {
  const all = registry();
  const mute = new Set<string>();
  for (const extra of CONTEXTS) {
    const context = all.context(extra);
    for (const a of actionCandidates(commandEntries(all, context))) if (a.off && !a.disabled) mute.add(a.key);
  }
  assert.deepEqual([...mute], []);
});

void test("the popup's own commands and those of a context menu's subject are not among the actions", () => {
  const keys = actionCandidates(commandEntries(registry(), registry().context())).map((a) => a.key);
  assert.deepEqual(
    keys.filter((k) => (PALETTE_COMMANDS as readonly string[]).includes(k)),
    [],
  );
  for (const subject of [
    "path.reviewOnly",
    "path.markSeen",
    "path.markUnseen",
    "queue.open",
    "queue.copyLink",
    "recent.open",
    "recent.forget",
    "thread.delete",
  ])
    assert.equal(keys.includes(subject), false, subject);
  assert.ok(keys.includes("path.exclude") && keys.includes("path.copy"));
});
