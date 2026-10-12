// The main menu with the review's commands registered, as the window has them, and the keys of
// every command in the registry.

import assert from "node:assert/strict";
import { test } from "node:test";
import { resolveMenu } from "../../lib/commandMenu.ts";
import { CommandRegistry, type KeyEventLike } from "../../lib/commands.ts";
import { canonicalSpec } from "../../lib/keymap.ts";
import type { MenuItem } from "../../lib/ui/menuItem.ts";
import { type AppActions, appCommands } from "../../state/appCommands.ts";
import type { CommandContext } from "../../state/commands.ts";
import { type MenuState, mainMenuSpec } from "../../state/mainMenu.ts";
import { fileCommands, viewCommands } from "./reviewCommands.ts";

const calls: string[] = [];
// Every action answers that it did something and writes down its name.
const record = new Proxy<Record<string, (...args: unknown[]) => unknown>>(
  {},
  {
    get:
      (_, name: string) =>
      (...args: unknown[]) => {
        calls.push([name, ...args.map(String)].join(" "));
        return true;
      },
  },
);

function registryFor(mod: "Ctrl" | "Meta", context: Partial<CommandContext> = {}) {
  const base: CommandContext = { repoOpen: false, reviewOpen: false, canQuit: true, diffView: false, hunk: -1 };
  const registry = new CommandRegistry<CommandContext>({ ...base, ...context }, mod);
  registry.register(
    ...appCommands(record as unknown as AppActions),
    ...viewCommands(record as never),
    ...fileCommands(record as never),
  );
  return registry;
}

const state: MenuState = {
  native: false,
  recent: [],
  screen: "review",
  splitDiff: false,
  fileText: false,
  softWrap: true,
  theme: "system",
  tools: [],
  range: false,
  canRestart: false,
  batchedReview: false,
};

const working: Partial<CommandContext> = {
  repoOpen: true,
  reviewOpen: true,
  changesetOpen: true,
  diffView: true,
  hunk: 0,
  tab: { kind: "file", path: "a.rs" },
  tabCount: 3,
  fileView: "diff",
  byCommit: true,
  manyCommits: true,
};

const menuOf = (s: Partial<MenuState>, context: Partial<CommandContext>) => {
  const native = s.native ?? false;
  const registry = registryFor(native ? "Meta" : "Ctrl", context);
  return resolveMenu(registry, mainMenuSpec({ ...state, ...s }), registry.context(), native);
};
const submenu = (items: MenuItem[], id: string) => items.find((m) => m.id === id)?.children ?? [];
const ids = (items: MenuItem[]) => items.map((i) => i.id);

void test("no two commands share a key on either platform, nor does a native accelerator take another command's key", () => {
  for (const mod of ["Ctrl", "Meta"] as const) {
    const registry = registryFor(mod);
    const owners = new Map<string, string[]>();
    for (const { id, binding } of registry.list()) {
      if (!binding) continue;
      const key = canonicalSpec(binding.spec);
      owners.set(key, [...(owners.get(key) ?? []), id]);
    }
    const shared = [...owners].filter(([, commands]) => commands.length > 1);
    assert.deepEqual(shared, [], mod);
  }
  const mac = registryFor("Meta");
  const chords = new Map(mac.list().flatMap(({ id, binding }) => (binding ? [[id, binding.chord] as const] : [])));
  const accelerators = new Map<string, string>();
  for (const { id } of mac.list()) {
    const accelerator = mac.find(id)?.accelerator;
    if (accelerator) accelerators.set(accelerator, id);
  }
  for (const [chord, owner] of accelerators) {
    // Settings and Quit are the platform's own chords, not the window's.
    if (owner === "app.settings" || owner === "app.quit") continue;
    assert.equal(chords.get(owner), chord, `${owner} keeps the key it is bound to`);
  }
  for (const [id, chord] of chords) {
    const owner = accelerators.get(chord);
    assert.ok(owner === undefined || owner === id, `${id} is bound to the accelerator of ${owner}`);
  }
});

void test("Windows and macOS menus list the same commands in the same places apart from the app's own items", () => {
  const appOnly = ["app.settings", "app.quit", "app.about"];
  const flat = (items: MenuItem[]): string[] =>
    items.flatMap((i) =>
      i.predefined || i.separator || appOnly.includes(i.id) || ["app", "edit", "window"].includes(i.id)
        ? []
        : [i.id, ...flat(i.children ?? [])],
    );
  assert.deepEqual(flat(menuOf({ native: true }, working)), flat(menuOf({}, working)));
});

void test("the Review and Navigate menus hold the work of reviewing, each with the key to learn", () => {
  const items = menuOf({}, working);
  assert.deepEqual(ids(submenu(items, "review")), [
    "review.toggleSeen",
    "review.reconfirm",
    "review.reconfirmAll",
    "-",
    "review.comment",
    "review.commentOnChangeset",
    "-",
    "review.post",
    "-",
    "review.finish",
  ]);
  const navigate = submenu(items, "navigate");
  assert.deepEqual(
    navigate.filter((i) => !i.separator).map((i) => [i.id, i.shortcut]),
    [
      ["review.nextHunk", "Ctrl+Alt+Down"],
      ["review.previousHunk", "Ctrl+Alt+Up"],
      ["review.nextCommit", "]"],
      ["review.previousCommit", "["],
      ["review.jumpToSource", "F4"],
      ["review.goToDefinition", "Ctrl+B"],
      ["review.changeCard", "Alt+F1"],
      ["view.revealFile", undefined],
    ],
  );
  const mac = submenu(menuOf({ native: true }, working), "navigate");
  assert.equal(mac.find((i) => i.id === "review.nextHunk")?.shortcut, "⌃⌥↓");
  assert.equal(mac.find((i) => i.id === "review.goToDefinition")?.shortcut, "⌘B");
});

void test("the View menu holds the screens, the tool windows, the tabs and the diff settings", () => {
  const view = submenu(menuOf({}, working), "view");
  assert.deepEqual(ids(view), [
    "view.queue",
    "view.review",
    "-",
    "tools",
    "-",
    "tab.previous",
    "tab.next",
    "-",
    "view.splitDiff",
    "view.fileText",
    "view.softWrap",
    "-",
    "theme",
  ]);
  assert.deepEqual(
    view.filter((i) => i.id.startsWith("tab.")).map((i) => i.shortcut),
    ["Alt+Left", "Alt+Right"],
  );
  assert.deepEqual(
    submenu(menuOf({ native: true }, working), "view")
      .filter((i) => i.id.startsWith("tab."))
      .map((i) => i.shortcut),
    ["⇧⌘[", "⇧⌘]"],
  );
});

void test("without a project the same items are there, disabled, and the ones that need a view say what to open", () => {
  const idle = menuOf({}, {});
  const all = (items: MenuItem[]): MenuItem[] => items.flatMap((i) => [i, ...all(i.children ?? [])]);
  const disabled = all(idle).filter((i) => i.enabled === false && !i.children);
  for (const id of ["review.toggleSeen", "review.nextHunk", "tab.close", "view.revealFile", "review.finish"]) {
    const item = all(idle).find((i) => i.id === id);
    assert.equal(item?.enabled, false, id);
    assert.ok(item.reason, id);
  }
  assert.ok(
    disabled.every((i) => i.reason),
    "every disabled item says why",
  );
  for (const menu of ["view", "changeset", "review", "navigate"]) assert.ok(submenu(idle, menu).length > 0, menu);
  assert.equal(all(idle).find((i) => i.id === "file.open")?.enabled, undefined);
});

void test("on the queue screen the items that act on the review's views are disabled and say so", () => {
  const items = menuOf({ screen: "queue" }, working);
  for (const id of [
    "tab.close",
    "tab.previous",
    "view.fileText",
    "review.toggleSeen",
    "review.comment",
    "review.nextHunk",
    "review.jumpToSource",
    "view.revealFile",
  ]) {
    const item = [
      ...submenu(items, "file"),
      ...submenu(items, "view"),
      ...submenu(items, "review"),
      ...submenu(items, "navigate"),
    ].find((i) => i.id === id);
    assert.equal(item?.enabled, false, id);
    assert.ok(item.reason, id);
  }
  const onReview = menuOf({ screen: "review" }, working);
  assert.equal(submenu(onReview, "navigate").find((i) => i.id === "review.nextHunk")?.enabled, undefined);
});

void test("the tool windows open with Alt and a digit, outside a text field only, and Option on macOS types nothing there", () => {
  const press = (init: Partial<KeyEventLike>, inField = false) =>
    ({
      key: "",
      code: "",
      ctrlKey: false,
      altKey: false,
      shiftKey: false,
      metaKey: false,
      target: { closest: (selector: string) => (inField && selector.includes("input") ? {} : null) },
      preventDefault: () => undefined,
      ...init,
    }) as KeyEventLike;

  const win = registryFor("Ctrl", working);
  calls.length = 0;
  assert.equal(win.dispatch(press({ key: "1", code: "Digit1", altKey: true })), true);
  assert.equal(win.dispatch(press({ key: "6", code: "Digit6", altKey: true })), true);
  assert.equal(win.dispatch(press({ key: "1", code: "Digit1", altKey: true }, true)), false);
  assert.deepEqual(calls, ["toggleTool changes", "toggleTool timeline"]);

  // On a German Windows layout AltGr+digit types a character and stays text.
  const altGr = press({ key: "{", code: "Digit7", ctrlKey: true, altKey: true });
  assert.equal(win.dispatch(altGr), false);

  const mac = registryFor("Meta", working);
  calls.length = 0;
  assert.equal(mac.dispatch(press({ key: "¡", code: "Digit1", altKey: true })), true);
  assert.equal(mac.dispatch(press({ key: "¡", code: "Digit1", altKey: true }, true)), false);
  assert.deepEqual(calls, ["toggleTool changes"]);
});
