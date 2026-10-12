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
  screen: "review",
  splitDiff: true,
  fileText: false,
  softWrap: true,
  theme: "dark",
  tools: [
    { id: "changes", shown: true },
    { id: "commits", shown: false },
    { id: "timeline", shown: true },
  ],
  range: false,
  canRestart: true,
  batchedReview: false,
};

const open: CommandContext = {
  repoOpen: true,
  reviewOpen: true,
  canQuit: true,
  diffView: false,
  hunk: -1,
  changesetOpen: true,
  pullRequest: true,
  outgoing: 2,
  reconfirmable: true,
};

const calls: string[] = [];
const actions: AppActions = {
  pickProject: () => calls.push("pick"),
  openProject: (path) => calls.push(`open ${path}`),
  finishReview: () => calls.push("finish"),
  changesSinceReview: () => calls.push("since"),
  wholeChangeset: () => calls.push("whole"),
  refresh: () => calls.push("refresh"),
  reconfirmAll: () => calls.push("reconfirmAll"),
  openChangeset: (key) => calls.push(`changeset ${keyId(key)}`),
  showScreen: (screen) => calls.push(`screen ${screen}`),
  showChangesets: () => calls.push("changesets"),
  openPullRequest: () => calls.push("pullRequest"),
  showDialog: (d) => calls.push(`dialog ${d}`),
  showKeymap: () => calls.push("keymap"),
  quit: () => calls.push("quit"),
  toggleSplit: () => calls.push("split"),
  toggleSoftWrap: () => calls.push("wrap"),
  toggleTool: (id) => calls.push(`tool ${id}`),
  setTheme: (theme) => calls.push(`theme ${theme}`),
  commentOnChangeset: () => calls.push("commentOnChangeset"),
  postComments: () => calls.push("post"),
  stageVote: (verdict) => calls.push(`vote ${verdict}`),
  clearSemanticCaches: () => calls.push("clearCaches"),
  showLog: () => calls.push("showLog"),
  restartBackend: () => calls.push("restart"),
};

const registry = new CommandRegistry<CommandContext>(open, "Ctrl");
registry.register(...appCommands(actions));

const menu = (s: Partial<MenuState> = {}, context: Partial<CommandContext> = {}): MenuItem[] =>
  resolveMenu(registry, mainMenuSpec({ ...state, ...s }), { ...open, ...context }, s.native ?? false);

const flatten = (items: MenuItem[]): MenuItem[] => items.flatMap((i) => [i, ...flatten(i.children ?? [])]);
const find = (items: MenuItem[], id: string) => flatten(items).find((i) => i.id === id);
const ids = (items: MenuItem[] | undefined) => (items ?? []).map((i) => i.id);
const submenu = (items: MenuItem[], id: string) => items.find((m) => m.id === id)?.children;

void test("every leaf item that is not the host's runs a command", () => {
  for (const native of [false, true]) {
    for (const item of flatten(menu({ native, votes: { verdicts: [{ id: "approved", label: "Approve" }] } }))) {
      if (item.children || item.separator || item.predefined) continue;
      calls.length = 0;
      assert.ok(registry.run(item.id, { ...open, manyCommits: true }), item.id);
      assert.equal(calls.length, 1, item.id);
    }
  }
});

void test("the menus follow the reviewer's nouns, with an app menu, Edit and Window around them on macOS", () => {
  assert.deepEqual(ids(menu()), ["file", "view", "navigate", "changeset", "review", "help"]);
  assert.deepEqual(ids(menu({ native: true })), [
    "app",
    "file",
    "edit",
    "view",
    "navigate",
    "changeset",
    "review",
    "window",
    "help",
  ]);
});

void test("macOS keeps the app items and the OS's own blocks to the host; elsewhere Settings and Exit are in File and About closes Help", () => {
  const mac = menu({ native: true });
  assert.deepEqual(ids(submenu(mac, "app")), [
    "app.about",
    "-",
    "app.settings",
    "-",
    "predefined.services",
    "-",
    "predefined.hide",
    "predefined.hideOthers",
    "predefined.showAll",
    "-",
    "app.quit",
  ]);
  assert.equal(find(mac, "app.quit")?.label, "Quit Vet");
  assert.deepEqual(ids(submenu(mac, "edit")), [
    "predefined.undo",
    "predefined.redo",
    "-",
    "predefined.cut",
    "predefined.copy",
    "predefined.paste",
    "predefined.selectAll",
  ]);
  assert.deepEqual(ids(submenu(mac, "window")), [
    "predefined.minimize",
    "predefined.zoom",
    "-",
    "predefined.fullscreen",
    "-",
    "predefined.bringAllToFront",
  ]);
  assert.ok(!ids(submenu(mac, "file")).some((id) => id === "app.settings" || id === "app.quit"));
  assert.ok(!ids(submenu(mac, "help")).includes("app.about"));

  const other = menu();
  assert.deepEqual(ids(submenu(other, "file")).slice(-3), ["app.settings", "-", "app.quit"]);
  assert.equal(find(other, "app.quit")?.label, "Exit");
  assert.equal(ids(submenu(other, "help")).at(-1), "app.about");
  assert.ok(flatten(other).every((i) => !i.predefined));
});

void test("only the chords whose command loses nothing to the menu become native accelerators on macOS", () => {
  const accelerators = Object.fromEntries(
    flatten(menu({ native: true }))
      .filter((i) => i.accelerator)
      .map((i) => [i.id, i.accelerator]),
  );
  assert.deepEqual(accelerators, {
    "app.settings": "Cmd+,",
    "app.quit": "Cmd+Q",
    "file.open": "Cmd+O",
    "view.queue": "Cmd+1",
    "view.review": "Cmd+2",
    "changeset.switch": "Cmd+Shift+O",
    "changeset.refresh": "Cmd+T",
  });
  // An item with an accelerator shows no key as text; the window's key handler still runs the others.
  assert.equal(find(menu({ native: true }), "file.open")?.shortcut, undefined);
  assert.equal(find(menu({ native: true }), "view.tool.changes")?.shortcut, "Alt+1");
});

void test("Open Project, Open Recent and the changeset items hold their places, and Switch Changeset has the key Open Pull Request lost", () => {
  const items = menu();
  assert.deepEqual(ids(submenu(items, "file")).slice(0, 3), ["file.open", "recent", "-"]);
  assert.equal(find(items, "changeset.switch")?.shortcut, "Ctrl+Shift+O");
  assert.equal(find(items, "changeset.openPullRequest")?.shortcut, undefined);
  assert.deepEqual(ids(submenu(items, "changeset")), [
    "changeset.switch",
    "changeset.openPullRequest",
    "-",
    "changeset.whole",
    "review.sinceLast",
    "-",
    "changeset.refresh",
  ]);
});

void test("without a project everything that needs one is disabled, not hidden, and says why", () => {
  const items = menu({ recent: [] }, { repoOpen: false, changesetOpen: false, reviewOpen: false, pullRequest: false });
  assert.equal(find(items, "recent")?.enabled, false);
  assert.equal(find(items, "file.open")?.enabled, undefined);
  for (const id of ["view.queue", "view.review", "changeset.switch", "changeset.refresh", "view.splitDiff"]) {
    const item = find(items, id);
    assert.equal(item?.enabled, false, id);
    assert.ok(item.reason, id);
  }
  assert.equal(find(items, "tools")?.enabled, false);
  assert.equal(find(items, "theme")?.enabled, undefined);
  assert.equal(find(items, "app.settings")?.enabled, undefined);
  assert.equal(find(items, "help.keymap")?.enabled, undefined);
  assert.equal(find(items, "view.softWrap")?.enabled, undefined);
});

void test("the Queue and Review screens are a radio pair, and the tool windows are checked only on the Review screen", () => {
  const onReview = menu();
  assert.deepEqual(
    ["view.queue", "view.review"].map((id) => [find(onReview, id)?.checked, find(onReview, id)?.radio]),
    [
      [false, true],
      [true, true],
    ],
  );
  const tools = (items: MenuItem[]) => submenu(submenu(items, "view") ?? [], "tools");
  assert.deepEqual(
    (tools(onReview) ?? []).filter((t) => !t.separator).map((t) => [t.id, t.checked]),
    [
      ["view.tool.changes", true],
      ["view.tool.all", false],
      ["view.tool.commits", false],
      ["view.tool.comments", false],
      ["view.tool.symbol", false],
      ["view.tool.timeline", true],
    ],
  );
  const onQueue = menu({ screen: "queue" });
  assert.equal(find(onQueue, "view.queue")?.checked, true);
  assert.ok((tools(onQueue) ?? []).every((t) => (t.separator ?? false) || t.checked === false));
});

void test("every tool window has its own Alt key, and Commits is offered only for a changeset of several commits", () => {
  const tools = (context: Partial<CommandContext> = {}) =>
    flatten(menu({}, context)).filter((i) => i.id.startsWith("view.tool."));
  assert.deepEqual(
    tools({ manyCommits: true }).map((t) => t.shortcut),
    ["Alt+1", "Alt+2", "Alt+3", "Alt+4", "Alt+5", "Alt+6"],
  );
  const commits = find(menu(), "view.tool.commits");
  assert.equal(commits?.enabled, false);
  assert.ok(commits.reason);
  assert.equal(find(menu({}, { manyCommits: true }), "view.tool.commits")?.enabled, undefined);
  calls.length = 0;
  assert.equal(registry.run("view.tool.commits", open), false);
  assert.equal(registry.run("view.tool.commits", { ...open, manyCommits: true }), true);
  assert.equal(registry.run("view.tool.timeline", open), true);
  assert.deepEqual(calls, ["tool commits", "tool timeline"]);
});

void test("the diff, wrap and theme settings show what is on", () => {
  const items = menu({ splitDiff: false, softWrap: false, theme: "light" });
  assert.equal(find(items, "view.splitDiff")?.checked, false);
  assert.equal(find(items, "view.softWrap")?.checked, false);
  assert.equal(find(items, "view.theme:light")?.checked, true);
  assert.equal(find(items, "view.theme:dark")?.checked, false);
  assert.equal(find(menu(), "view.splitDiff")?.checked, true);
  calls.length = 0;
  registry.run("view.splitDiff", open);
  registry.run("view.softWrap", open);
  assert.deepEqual(calls, ["split", "wrap"]);
});

void test("Whole Changeset and Changes Since My Last Review are a radio pair that follows the shown range", () => {
  const whole = menu({ range: false });
  assert.deepEqual([find(whole, "changeset.whole")?.checked, find(whole, "review.sinceLast")?.checked], [true, false]);
  const ranged = menu({ range: true });
  assert.deepEqual(
    [find(ranged, "changeset.whole")?.checked, find(ranged, "review.sinceLast")?.checked],
    [false, true],
  );

  const noReview = menu({}, { reviewOpen: false });
  assert.equal(find(noReview, "review.sinceLast")?.enabled, false);
  assert.ok(find(noReview, "review.sinceLast")?.reason);
  assert.equal(find(noReview, "changeset.whole")?.enabled, undefined);
  calls.length = 0;
  assert.equal(registry.run("review.sinceLast", { ...open, reviewOpen: false }), false);
  assert.equal(registry.run("changeset.whole", open), true);
  assert.equal(registry.run("review.sinceLast", open), true);
  assert.deepEqual(calls, ["whole", "since"]);
});

void test("Fetch and Refresh does what the queue's refresh button does, for a project", () => {
  assert.equal(find(menu({}, { repoOpen: false }), "changeset.refresh")?.enabled, false);
  calls.length = 0;
  assert.equal(registry.run("changeset.refresh", { ...open, repoOpen: false }), false);
  assert.equal(registry.run("changeset.refresh", open), true);
  assert.deepEqual(calls, ["refresh"]);
});

void test("Finish Review and Re-confirm All are offered only while there is something to act on", () => {
  assert.equal(find(menu(), "review.finish")?.enabled, undefined);
  const idle = menu({}, { reviewOpen: false, reconfirmable: false });
  assert.equal(find(idle, "review.finish")?.enabled, false);
  assert.equal(find(idle, "review.reconfirmAll")?.enabled, false);
  assert.ok(find(idle, "review.reconfirmAll")?.reason);
  calls.length = 0;
  assert.equal(registry.run("review.finish", { ...open, reviewOpen: false }), false);
  assert.equal(registry.run("review.reconfirmAll", { ...open, reconfirmable: false }), false);
  assert.equal(registry.run("review.finish", open), true);
  assert.equal(registry.run("review.reconfirmAll", open), true);
  assert.deepEqual(calls, ["finish", "reconfirmAll"]);
});

void test("Comment on Changeset needs a changeset on view", () => {
  assert.equal(find(menu({}, { changesetOpen: false }), "review.commentOnChangeset")?.enabled, false);
  calls.length = 0;
  assert.equal(registry.run("review.commentOnChangeset", { ...open, changesetOpen: false }), false);
  assert.equal(registry.run("review.commentOnChangeset", open), true);
  assert.deepEqual(calls, ["commentOnChangeset"]);
});

void test("posting is Post Comments, or Submit Review where the platform submits a review, and waits for something to send", () => {
  assert.equal(find(menu(), "review.post")?.label, "Post Comments…");
  assert.equal(find(menu({ batchedReview: true }), "review.post")?.label, "Submit Review…");
  assert.equal(find(menu(), "review.post")?.enabled, undefined);

  for (const [context, why] of [
    [{ pullRequest: false, outgoing: 0 }, "no pull request"],
    [{ outgoing: 0 }, "nothing waits"],
    [{ rangeShown: true }, "a range is shown"],
  ] as const) {
    const item = find(menu({}, context), "review.post");
    assert.equal(item?.enabled, false, why);
    assert.ok(item.reason, why);
  }
  calls.length = 0;
  assert.equal(registry.run("review.post", { ...open, outgoing: 0 }), false);
  assert.equal(registry.run("review.post", open), true);
  assert.deepEqual(calls, ["post"]);
});

void test("the Vote submenu exists only for a platform that keeps the vote apart, checks the chosen vote and stages another as the dropdown does", () => {
  const verdicts = [
    { id: "approved", label: "Approve" },
    { id: "waiting", label: "Wait for Author" },
    { id: "rejected", label: "Reject" },
  ];
  assert.equal(find(menu(), "vote"), undefined);

  const vote = find(menu({ votes: { verdicts, chosen: "waiting" } }), "vote");
  assert.deepEqual(
    vote?.children?.map((v) => [v.id, v.label, v.checked, v.radio]),
    [
      ["review.vote:approved", "Approve", false, true],
      ["review.vote:waiting", "Wait for Author", true, true],
      ["review.vote:rejected", "Reject", false, true],
    ],
  );
  assert.equal(vote.enabled, undefined);
  assert.deepEqual(
    find(menu({ votes: { verdicts } }), "vote")?.children?.map((v) => v.checked),
    [false, false, false],
  );

  calls.length = 0;
  assert.equal(registry.run("review.vote:rejected", open), true);
  assert.deepEqual(calls, ["vote rejected"]);

  const away = menu({ votes: { verdicts } }, { pullRequest: false });
  assert.equal(find(away, "vote")?.enabled, false);
  assert.equal(registry.run("review.vote:approved", { ...open, pullRequest: false }), false);
  const blocked = menu({ votes: { verdicts } }, { voteBlock: "Sign in to submit the review" });
  assert.equal(find(blocked, "vote")?.enabled, false);
  assert.equal(find(blocked, "review.vote:approved")?.reason, "Sign in to submit the review");
  assert.equal(registry.run("review.vote:approved", { ...open, voteBlock: "Sign in to submit the review" }), false);
  assert.deepEqual(calls, ["vote rejected"]);
});

void test("Restart Backend is offered only where the host can restart it", () => {
  assert.ok(find(menu({ canRestart: true }), "app.restartBackend"));
  assert.equal(find(menu({ canRestart: false }), "app.restartBackend"), undefined);
});

void test("Clear Semantic Caches and Show Log Folder are in the Help menu and need no repository", () => {
  const help = submenu(menu({}, { repoOpen: false, reviewOpen: false }), "help");
  for (const id of ["semantic.clearCaches", "help.showLog", "help.keymap"]) {
    assert.equal(help?.find((i) => i.id === id)?.enabled, undefined, id);
  }
  calls.length = 0;
  const idle = { ...open, repoOpen: false, reviewOpen: false };
  assert.equal(registry.run("semantic.clearCaches", idle), true);
  assert.equal(registry.run("help.showLog", idle), true);
  assert.equal(registry.run("help.keymap", idle), true);
  assert.deepEqual(calls, ["clearCaches", "showLog", "keymap"]);
});

void test("items carry their argument, including paths with colons", () => {
  calls.length = 0;
  registry.run("file.openRecent:C:/src/vet", open);
  registry.run("view.theme:light", open);
  assert.equal(registry.run("view.theme:neon", open), false);
  assert.deepEqual(calls, ["open C:/src/vet", "theme light"]);
});
