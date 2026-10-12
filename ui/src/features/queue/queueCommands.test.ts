import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangesetKey } from "../../generated/ChangesetKey";
import { resolveMenu } from "../../lib/commandMenu.ts";
import { CommandRegistry } from "../../lib/commands.ts";
import { type AppActions, appCommands } from "../../state/appCommands.ts";
import type { CommandContext } from "../../state/commands.ts";
import { queueCommands, rowMenu } from "./queueCommands.ts";

// The pull request on view is not the row the menu is opened on.
const onView: CommandContext = {
  repoOpen: true,
  reviewOpen: true,
  canQuit: true,
  diffView: false,
  hunk: -1,
  pullRequest: true,
};

const pr = { key: { type: "pullRequest", number: 7 } as unknown as ChangesetKey, url: "https://host/pulls/7" };
const branch = { key: { type: "branch", name: "topic" } as unknown as ChangesetKey };

function setup() {
  const calls: string[] = [];
  const registry = new CommandRegistry<CommandContext>(onView, "Ctrl");
  registry.register(
    ...queueCommands({
      open: (key) => calls.push(`open ${JSON.stringify(key)}`),
      copyText: (text) => calls.push(`copy ${text}`),
    }),
    ...appCommands({ openInBrowser: (url) => calls.push(`browser ${url}`) } as unknown as AppActions),
  );
  const menu = (row: CommandContext["row"]) =>
    resolveMenu(registry, rowMenu, registry.context({ row }))
      .filter((i) => !i.separator)
      .map((i) => [i.id, i.enabled !== false] as const);
  return { registry, calls, menu };
}

void test("a pull request row enables every entry, a branch row only Open", () => {
  const { menu } = setup();
  assert.equal(menu(pr).length, 3);
  assert.ok(menu(pr).every(([, enabled]) => enabled));
  assert.deepEqual(
    menu(branch).filter(([, enabled]) => enabled),
    [["queue.open", true]],
  );
});

void test("the entries act on the row the menu was opened on, not the pull request on view", () => {
  const { registry, calls } = setup();
  const context = registry.context({ row: pr });
  registry.run("changeset.openInBrowser", context);
  registry.run("queue.copyLink", context);
  registry.run("queue.open", context);
  assert.deepEqual(calls, [
    "browser https://host/pulls/7",
    "copy https://host/pulls/7",
    `open ${JSON.stringify(pr.key)}`,
  ]);
});

void test("a branch row neither opens a page nor copies a link, even with a pull request on view", () => {
  const { registry, calls } = setup();
  const context = registry.context({ row: branch });
  assert.equal(registry.run("changeset.openInBrowser", context), false);
  assert.equal(registry.run("queue.copyLink", context), false);
  assert.deepEqual(calls, []);
});

void test("Open in Browser without a row still opens the pull request on view", () => {
  const { registry, calls } = setup();
  assert.equal(registry.run("changeset.openInBrowser", registry.context()), true);
  assert.deepEqual(calls, ["browser undefined"]);
});
