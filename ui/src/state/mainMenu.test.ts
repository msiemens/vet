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
  softWrap: true,
  theme: "dark",
  tools: [
    { id: "changes", shown: true },
    { id: "commits", shown: false },
    { id: "timeline", shown: true },
    { id: "problems", shown: false },
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
  closeProject: () => calls.push("close"),
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
  openInBrowser: () => calls.push("browser"),
  showDialog: (d) => calls.push(`dialog ${d}`),
  showKeymap: () => calls.push("keymap"),
  showAccounts: () => calls.push("accounts"),
  showProjectSettings: () => calls.push("projectSettings"),
  quit: () => calls.push("quit"),
  toggleSoftWrap: () => calls.push("wrap"),
  toggleTool: (id) => calls.push(`tool ${id}`),
  stepProblem: (delta) => calls.push(`problem ${delta}`),
  setTheme: (theme) => calls.push(`theme ${theme}`),
  commentOnChangeset: () => calls.push("commentOnChangeset"),
  postComments: () => calls.push("post"),
  pickVote: () => calls.push("pickVote"),
  stageVote: (verdict) => calls.push(`vote ${verdict}`),
  clearSemanticCaches: () => calls.push("clearCaches"),
  showLog: () => calls.push("showLog"),
  restartBackend: () => calls.push("restart"),
  restartLanguageServer: (language) => calls.push(`restart ${language}`),
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
      assert.ok(registry.run(item.id, { ...open, manyCommits: true, serverLanguage: "rust", problems: true }), item.id);
      assert.equal(calls.length, 1, item.id);
    }
  }
});

void test("macOS keeps Settings, About and Quit in the app menu and leaves the OS's own blocks to the host; elsewhere Settings and Exit are in File and About closes Help", () => {
  const mac = menu({ native: true });
  const inApp = ids(submenu(mac, "app"));
  for (const id of ["app.about", "app.settings", "app.quit"]) assert.ok(inApp.includes(id), id);
  assert.ok(flatten(mac).some((i) => i.predefined));
  assert.equal(find(mac, "app.quit")?.label, "Quit Vet");
  assert.ok(!ids(submenu(mac, "file")).some((id) => id === "app.settings" || id === "app.quit"));
  assert.ok(!ids(submenu(mac, "help")).includes("app.about"));

  const other = menu();
  assert.deepEqual(ids(submenu(other, "file")).slice(-4), ["app.settings", "app.projectSettings", "-", "app.quit"]);
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

void test("Switch Changeset has the key Open Pull Request lost", () => {
  const items = menu();
  assert.equal(find(items, "changeset.switch")?.shortcut, "Ctrl+Shift+O");
  assert.equal(find(items, "changeset.openPullRequest")?.shortcut, undefined);
});

void test("Open in Browser needs a pull request on view and says so when there is none", () => {
  const on = find(menu(), "changeset.openInBrowser");
  assert.equal(on?.enabled, undefined);
  assert.equal(on?.shortcut, undefined);
  const off = find(menu({}, { pullRequest: false }), "changeset.openInBrowser");
  assert.equal(off?.enabled, false);
  assert.equal(off.reason, "No pull request is on view");
});

void test("without a project everything that needs one is disabled, not hidden, and says why", () => {
  const items = menu({ recent: [] }, { repoOpen: false, changesetOpen: false, reviewOpen: false, pullRequest: false });
  assert.equal(find(items, "recent")?.enabled, false);
  assert.equal(find(items, "file.open")?.enabled, undefined);
  for (const id of [
    "app.projectSettings",
    "file.close",
    "view.queue",
    "view.review",
    "changeset.switch",
    "changeset.refresh",
  ]) {
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
      ["view.tool.timeline", true],
      ["view.tool.problems", false],
      ["view.tool.find", false],
      ["view.tool.pipelines", false],
      ["view.tool.symbol", false],
      ["view.tool.structure", false],
    ],
  );
  // The keys count the left tools, then the bottom ones, then the right ones.
  assert.deepEqual(
    (tools(onReview) ?? []).filter((t) => !t.separator).map((t) => t.shortcut),
    ["Alt+1", "Alt+2", "Alt+3", "Alt+4", "Alt+5", "Alt+6", "Alt+7", "Alt+8", "Alt+9", "Alt+0"],
  );
  const onQueue = menu({ screen: "queue" });
  assert.equal(find(onQueue, "view.queue")?.checked, true);
  assert.ok((tools(onQueue) ?? []).every((t) => (t.separator ?? false) || t.checked === false));
});

void test("every tool window has a key of its own, and Commits is offered only for a changeset of several commits", () => {
  const shortcuts = flatten(menu({}, { manyCommits: true }))
    .filter((i) => i.id.startsWith("view.tool."))
    .map((t) => t.shortcut);
  assert.ok(shortcuts.length > 0 && shortcuts.every(Boolean));
  assert.equal(new Set(shortcuts).size, shortcuts.length);
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

void test("Next and Previous Problem do nothing, and say why in the menu, where the file on view has no problems", () => {
  const some = { ...open, problems: true };
  const none = find(menu({}, { problems: false }), "nav.nextProblem");
  assert.equal(none?.enabled, false);
  assert.ok(none.reason);
  assert.equal(find(menu({}, some), "nav.previousProblem")?.enabled, undefined);
  calls.length = 0;
  assert.equal(registry.run("nav.nextProblem", { ...open, problems: false }), false);
  assert.equal(registry.run("nav.previousProblem", { ...open, problems: false }), false);
  assert.deepEqual(calls, []);
  assert.equal(registry.run("nav.nextProblem", some), true);
  assert.equal(registry.run("nav.previousProblem", some), true);
  assert.deepEqual(calls, ["problem 1", "problem -1"]);
});

void test("the wrap and theme settings show what is on", () => {
  const items = menu({ softWrap: false, theme: "light" });
  assert.equal(find(items, "view.softWrap")?.checked, false);
  assert.equal(find(items, "view.theme:light")?.checked, true);
  assert.equal(find(items, "view.theme:dark")?.checked, false);
  calls.length = 0;
  registry.run("view.softWrap", open);
  assert.deepEqual(calls, ["wrap"]);
});

void test("one key toggles between Changes Since My Last Review and the whole changeset, as the shown range says", () => {
  calls.length = 0;
  assert.equal(registry.run("changeset.toggleSinceLast", open), true);
  assert.equal(registry.run("changeset.toggleSinceLast", { ...open, rangeShown: true }), true);
  assert.deepEqual(calls, ["since", "whole"]);
  assert.equal(registry.run("changeset.toggleSinceLast", { ...open, reviewOpen: false }), false);
  assert.ok(registry.disabledReason("changeset.toggleSinceLast", { ...open, reviewOpen: false }));
  // Back from a range needs only the changeset.
  assert.equal(registry.run("changeset.toggleSinceLast", { ...open, rangeShown: true, reviewOpen: false }), true);
});

void test("the vote picker opens only for a pull request on a platform that keeps the vote apart, and says why not", () => {
  const separate = { ...open, voteSeparate: true };
  calls.length = 0;
  assert.equal(registry.run("review.pickVote", separate), true);
  assert.deepEqual(calls, ["pickVote"]);
  for (const [context, why] of [
    [{ pullRequest: false }, "no pull request"],
    [{ voteSeparate: false }, "the platform votes with the review"],
    [{ voteBlock: "Sign in to submit the review" }, "the vote is blocked"],
  ] as const) {
    assert.equal(registry.run("review.pickVote", { ...separate, ...context }), false, why);
    assert.ok(registry.disabledReason("review.pickVote", { ...separate, ...context }), why);
  }
  assert.equal(registry.disabledReason("review.pickVote", { ...separate, voteBlock: "Sign in" }), "Sign in");
  assert.deepEqual(calls, ["pickVote"]);
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
  assert.equal(find(menu(), "review.post")?.shortcut, "Ctrl+Shift+K");
  assert.equal(find(menu({ batchedReview: true }), "review.post")?.shortcut, "Ctrl+Shift+K");

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

void test("Restart Language Server restarts the language of the active file, and says why it is off for a file without a server", () => {
  const item = (context: Partial<CommandContext>) =>
    submenu(menu({}, context), "help")?.find((i) => i.id === "app.restartLanguageServer");
  assert.equal(item({ serverLanguage: "python" })?.enabled, undefined);
  const off = item({});
  assert.ok(off);
  assert.equal(off.enabled, false);
  assert.equal(off.reason, "The active file has no language server");

  calls.length = 0;
  assert.equal(registry.run("app.restartLanguageServer", { ...open, serverLanguage: "python" }), true);
  assert.equal(registry.run("app.restartLanguageServer", open), false);
  assert.deepEqual(calls, ["restart python"]);
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
