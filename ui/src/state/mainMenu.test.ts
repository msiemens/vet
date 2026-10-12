import assert from "node:assert/strict";
import { test } from "node:test";
import { resolveMenu } from "../lib/commandMenu.ts";
import { CommandRegistry } from "../lib/commands.ts";
import type { MenuItem } from "../lib/ui/menuItem.ts";
import { type AppActions, appCommands } from "./appCommands.ts";
import { keyId } from "./changesetKey.ts";
import type { CommandContext } from "./commands.ts";
import { type MenuState, mainMenuSpec } from "./mainMenu.ts";

const state: MenuState = {
  native: false,
  recent: [{ path: "/a/vet", name: "vet" }],
  mode: "split",
  theme: "dark",
  tools: [
    { id: "changes", shown: true },
    { id: "commits", shown: false },
  ],
};

const open: CommandContext = { repoOpen: true, reviewOpen: true, canQuit: true, diffView: false, hunk: -1 };

const calls: string[] = [];
const actions: AppActions = {
  pickProject: () => calls.push("pick"),
  openProject: (path) => calls.push(`open ${path}`),
  finishReview: () => calls.push("finish"),
  changesSinceReview: () => calls.push("since"),
  fetch: () => calls.push("fetch"),
  openChangeset: (key) => calls.push(`changeset ${keyId(key)}`),
  showQueue: () => calls.push("queue"),
  showChangesets: () => calls.push("changesets"),
  openPullRequest: () => calls.push("pullRequest"),
  showDialog: (d) => calls.push(`dialog ${d}`),
  quit: () => calls.push("quit"),
  setMode: (mode) => calls.push(`mode ${mode}`),
  toggleTool: (id) => calls.push(`tool ${id}`),
  setTheme: (theme) => calls.push(`theme ${theme}`),
  clearSemanticCaches: () => calls.push("clearCaches"),
  showLog: () => calls.push("showLog"),
};

const registry = new CommandRegistry<CommandContext>(open, "Ctrl");
registry.register(...appCommands(actions));

const menu = (s: Partial<MenuState> = {}, context: Partial<CommandContext> = {}): MenuItem[] =>
  resolveMenu(registry, mainMenuSpec({ ...state, ...s }), { ...open, ...context }, s.native ?? false);

const flatten = (items: MenuItem[]): MenuItem[] => items.flatMap((i) => [i, ...flatten(i.children ?? [])]);
const find = (items: MenuItem[], id: string) => flatten(items).find((i) => i.id === id);

void test("every leaf item runs a command", () => {
  for (const native of [false, true]) {
    for (const item of flatten(menu({ native }))) {
      if (item.children || item.separator) continue;
      calls.length = 0;
      assert.ok(registry.run(item.id, { ...open, manyCommits: true }), item.id);
      assert.equal(calls.length, 1, item.id);
    }
  }
});

void test("the macOS menu bar has an app menu; elsewhere Settings and Exit are in File", () => {
  const mac = menu({ native: true });
  assert.deepEqual(
    mac.map((m) => m.id),
    ["app", "file", "view", "vcs", "help"],
  );
  assert.equal(find(mac, "app.settings")?.accelerator, "Cmd+,");
  assert.equal(find(mac, "app.quit")?.label, "Quit Vet");
  assert.equal(find(mac, "app.quit")?.accelerator, "Cmd+Q");
  const other = menu();
  assert.deepEqual(
    other.map((m) => m.id),
    ["file", "view", "vcs", "help"],
  );
  assert.equal(find(other, "app.settings")?.shortcut, "Ctrl+Alt+S");
  assert.equal(find(other, "app.quit")?.label, "Exit");
  assert.equal(find(other, "app.quit")?.shortcut, undefined);
  assert.ok(other[0].children?.some((i) => i.id === "app.settings"));
});

void test("state shows as checks and enabled flags", () => {
  const items = menu({ recent: [] }, { repoOpen: false });
  assert.equal(find(items, "view.mode:split")?.checked, true);
  assert.equal(find(items, "view.mode:unified")?.checked, false);
  assert.equal(find(items, "view.theme:dark")?.checked, true);
  assert.equal(find(items, "recent")?.enabled, false);
  assert.equal(find(items, "tools")?.enabled, false);
  assert.equal(find(items, "view.mode:split")?.enabled, false);
  assert.equal(find(items, "theme")?.enabled, undefined);
});

void test("every tool window has its own key, and Commits is offered only for a changeset of several commits", () => {
  const tools = find(menu({}, { manyCommits: true }), "tools")?.children ?? [];
  assert.deepEqual(
    tools.map((t) => [t.label, t.shortcut, t.checked]),
    [
      ["Changes", "Ctrl+Alt+1", true],
      ["Commits", "Ctrl+Alt+3", false],
    ],
  );
  assert.equal(find(menu(), "view.tool.commits")?.enabled, false);
  assert.equal(find(menu({}, { manyCommits: true }), "view.tool.commits")?.enabled, undefined);
  calls.length = 0;
  assert.equal(registry.run("view.tool.commits", open), false);
  assert.equal(registry.run("view.tool.commits", { ...open, manyCommits: true }), true);
  assert.deepEqual(calls, ["tool commits"]);
});

void test("Finish Review is offered only while a review is open", () => {
  assert.equal(find(menu(), "review.finish")?.enabled, undefined);
  assert.equal(find(menu({}, { reviewOpen: false }), "review.finish")?.enabled, false);
  calls.length = 0;
  assert.equal(registry.run("review.finish", { ...open, reviewOpen: false }), false);
  assert.deepEqual(calls, []);
});

void test("Changes Since My Last Review is offered only while a review is open", () => {
  assert.equal(find(menu(), "review.sinceLast")?.enabled, undefined);
  assert.equal(find(menu({}, { reviewOpen: false }), "review.sinceLast")?.enabled, false);
  calls.length = 0;
  assert.equal(registry.run("review.sinceLast", { ...open, reviewOpen: false }), false);
  assert.equal(registry.run("review.sinceLast", open), true);
  assert.deepEqual(calls, ["since"]);
});

void test("Fetch is offered in the VCS menu while a project is open", () => {
  const vcs = menu().find((m) => m.id === "vcs");
  assert.ok(vcs?.children?.some((i) => i.id === "git.fetch"));
  assert.equal(find(menu({}, { repoOpen: false }), "git.fetch")?.enabled, false);
  calls.length = 0;
  assert.equal(registry.run("git.fetch", { ...open, repoOpen: false }), false);
  assert.equal(registry.run("git.fetch", open), true);
  assert.deepEqual(calls, ["fetch"]);
});

void test("Clear Semantic Caches is in the File menu and runs without a repository: the caches belong to the backend", () => {
  const file = menu({}, { repoOpen: false }).find((m) => m.id === "file");
  assert.equal(file?.children?.find((i) => i.id === "semantic.clearCaches")?.enabled, undefined);
  calls.length = 0;
  assert.equal(registry.run("semantic.clearCaches", { ...open, repoOpen: false, reviewOpen: false }), true);
  assert.deepEqual(calls, ["clearCaches"]);
});

void test("Show Log is in the Help menu and needs no repository", () => {
  const help = menu({}, { repoOpen: false }).find((m) => m.id === "help");
  assert.equal(help?.children?.find((i) => i.id === "help.showLog")?.enabled, undefined);
  calls.length = 0;
  assert.equal(registry.run("help.showLog", { ...open, repoOpen: false, reviewOpen: false }), true);
  assert.deepEqual(calls, ["showLog"]);
});

void test("items carry their argument, including paths with colons", () => {
  calls.length = 0;
  registry.run("file.openRecent:C:/src/vet", open);
  registry.run("view.theme:light", open);
  assert.equal(registry.run("view.theme:neon", open), false);
  assert.deepEqual(calls, ["open C:/src/vet", "theme light"]);
});
