// The main menu with the review's commands registered, as the window has them, and the keys of
// every command in the registry.

import assert from "node:assert/strict";
import { test } from "node:test";
import { resolveMenu } from "../../lib/commandMenu.ts";
import { CommandRegistry, type KeyEventLike } from "../../lib/commands.ts";
import { canonicalSpec } from "../../lib/keymap.ts";
import { type MenuItem, mnemonicTarget } from "../../lib/ui/menuItem.ts";
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
  tab: "file:a.rs",
  tabCount: 3,
  viewMode: "unified",
  viewModes: ["unified", "split", "file"],
  byCommit: true,
  manyCommits: true,
};

const menuOf = (s: Partial<MenuState>, context: Partial<CommandContext>) => {
  const native = s.native ?? false;
  const registry = registryFor(native ? "Meta" : "Ctrl", context);
  return resolveMenu(registry, mainMenuSpec({ ...state, ...s }), registry.context(), native);
};
const submenu = (items: MenuItem[], id: string) => items.find((m) => m.id === id)?.children ?? [];

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

void test("every menu of the bar starts with a letter of its own, which opens it", () => {
  const bar = menuOf({}, working).filter((m) => m.children);
  const initials = bar.map((m) => m.label[0].toLowerCase());
  assert.equal(new Set(initials).size, initials.length);
  bar.forEach((m, i) => assert.equal(mnemonicTarget(bar, initials[i])?.id, m.id));
});

void test("the view modes are one radio group that shows what the active tab shows, and offers a tab only the modes it has", () => {
  const modes = (viewMode: MenuState["viewMode"], context: Partial<CommandContext> = {}) =>
    submenu(menuOf({ viewMode }, { ...working, ...context }), "view")
      .filter((i) => i.id.startsWith("view.mode."))
      .map((i) => [i.checked, i.enabled]);
  assert.deepEqual(modes("unified"), [
    [true, undefined],
    [false, undefined],
    [false, undefined],
  ]);
  assert.deepEqual(modes("file", { viewMode: "file", viewModes: ["file"] }), [
    [false, false],
    [false, false],
    [true, undefined],
  ]);
  assert.deepEqual(modes(undefined, { viewModes: [] }), [
    [false, false],
    [false, false],
    [false, false],
  ]);
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
  const items = menuOf({ screen: "queue" }, { ...working, queueShown: true });
  for (const id of [
    "tab.close",
    "tab.previous",
    "view.mode.file",
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

void test("the review's keys run on the Review screen and not while the Queue screen covers it", () => {
  const keys: [string, Partial<KeyEventLike>, string][] = [
    ["Next Hunk", { key: "ArrowDown", code: "ArrowDown", ctrlKey: true, altKey: true }, "step 1"],
    ["Jump to Source", { key: "F4", code: "F4" }, "jumpToSource"],
    ["Toggle Seen", { key: " ", code: "Space" }, "toggleSeen"],
    ["Next Tab", { key: "ArrowRight", code: "ArrowRight", altKey: true }, "stepTab 1"],
    ["File Text", { key: "#", code: "Digit3", altKey: true, shiftKey: true }, "setViewMode file"],
  ];
  const withTab = { ...working, tab: "changes" as const };
  for (const [name, init, call] of keys) {
    calls.length = 0;
    assert.equal(registryFor("Ctrl", { ...withTab, queueShown: true }).dispatch(press(init)), false, name);
    assert.deepEqual(calls, [], name);
    assert.equal(registryFor("Ctrl", { ...withTab, queueShown: false }).dispatch(press(init)), true, name);
    assert.deepEqual(calls, [call], name);
  }
});

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

void test("Alt+Shift and a digit show a view mode by the physical key, whatever the layout types with Shift", () => {
  // `key` is what the layout types with Shift held: US "!", "@", "#"; German "!", "\"", "§".
  const layouts = {
    US: ["!", "@", "#"],
    German: ["!", '"', "§"],
  };
  const win = registryFor("Ctrl", working);
  for (const [layout, keys] of Object.entries(layouts)) {
    calls.length = 0;
    keys.forEach((key, i) => {
      const event = press({ key, code: `Digit${i + 1}`, altKey: true, shiftKey: true });
      assert.equal(win.dispatch(event), true, `${layout} ${key}`);
    });
    assert.deepEqual(calls, ["setViewMode unified", "setViewMode split", "setViewMode file"], layout);
  }

  // The same keys in a text field belong to the field; Alt+digit alone is still the tool window.
  calls.length = 0;
  assert.equal(win.dispatch(press({ key: "!", code: "Digit1", altKey: true, shiftKey: true }, true)), false);
  assert.equal(win.dispatch(press({ key: "1", code: "Digit1", altKey: true })), true);
  assert.deepEqual(calls, ["toggleTool changes"]);

  // Option+Shift+digit types a symbol on macOS, but nowhere but in a text field.
  const mac = registryFor("Meta", working);
  calls.length = 0;
  assert.equal(mac.dispatch(press({ key: "⁄", code: "Digit1", altKey: true, shiftKey: true })), true);
  assert.equal(mac.dispatch(press({ key: "⁄", code: "Digit1", altKey: true, shiftKey: true }, true)), false);
  assert.deepEqual(calls, ["setViewMode unified"]);
});

void test("F flags like Space marks seen, and Alt+F and Alt+Shift+F step between flags, outside text fields only", () => {
  const keys: [string, Partial<KeyEventLike>, string][] = [
    ["F", { key: "f", code: "KeyF" }, "toggleFlag"],
    ["Alt+F", { key: "f", code: "KeyF", altKey: true }, "stepFlag 1"],
    ["Alt+Shift+F", { key: "F", code: "KeyF", altKey: true, shiftKey: true }, "stepFlag -1"],
    // Option+F types "ƒ" on macOS.
    ["Option+F", { key: "ƒ", code: "KeyF", altKey: true }, "stepFlag 1"],
    ["Option+Shift+F", { key: "Í", code: "KeyF", altKey: true, shiftKey: true }, "stepFlag -1"],
  ];
  for (const mod of ["Ctrl", "Meta"] as const) {
    const registry = registryFor(mod, working);
    for (const [name, init, call] of keys) {
      if (mod === "Ctrl" && name.startsWith("Option")) continue;
      calls.length = 0;
      assert.equal(registry.dispatch(press(init)), true, `${mod} ${name}`);
      assert.deepEqual(calls, [call], `${mod} ${name}`);
      assert.equal(registry.dispatch(press(init, true)), false, `${mod} ${name} in a field`);
    }
    calls.length = 0;
    assert.equal(
      registry.dispatch(press({ key: "ƒ", code: "KeyF", ctrlKey: true, altKey: true })),
      false,
      "Ctrl+Alt+F",
    );
    assert.equal(
      registry.dispatch(press({ key: "f", code: "KeyF", ctrlKey: true, altKey: true })),
      false,
      "Ctrl+Alt+F",
    );
    assert.deepEqual(calls, []);
  }
});

void test("the keys of Post, Vote, Toggle Since Last Review and Reveal File run outside text fields, and Ctrl+Alt+F stays free", () => {
  const win = registryFor("Ctrl", { ...working, pullRequest: true, outgoing: 1, voteSeparate: true });
  const keys: [string, Partial<KeyEventLike>, string][] = [
    ["Mod+Shift+K", { key: "K", code: "KeyK", ctrlKey: true, shiftKey: true }, "postComments"],
    ["Ctrl+Alt+V", { key: "v", code: "KeyV", ctrlKey: true, altKey: true }, "pickVote"],
    ["Ctrl+Alt+L", { key: "l", code: "KeyL", ctrlKey: true, altKey: true }, "changesSinceReview"],
    ["Alt+F2", { key: "F2", code: "F2", altKey: true }, "revealFile"],
  ];
  for (const [name, init, call] of keys) {
    calls.length = 0;
    assert.equal(win.dispatch(press(init)), true, name);
    assert.deepEqual(calls, [call], name);
    assert.equal(win.dispatch(press(init, true)), false, `${name} in a field`);
  }
  const mac = registryFor("Meta", { ...working, pullRequest: true, outgoing: 1 });
  calls.length = 0;
  assert.equal(mac.dispatch(press({ key: "K", code: "KeyK", metaKey: true, shiftKey: true })), true);
  assert.deepEqual(calls, ["postComments"]);

  // AltGr is Ctrl+Alt on Windows: with it down the key types, so no command takes it.
  for (const code of ["KeyV", "KeyL"]) {
    const altGr = press({
      key: "",
      code,
      ctrlKey: true,
      altKey: true,
      getModifierState: (m: string) => m === "AltGraph",
    });
    assert.equal(win.dispatch(altGr), false, code);
  }

  for (const mod of ["Ctrl", "Meta"] as const) {
    const owner = registryFor(mod)
      .list()
      .find(({ binding }) => binding && canonicalSpec(binding.spec) === "Ctrl-Alt-f");
    assert.equal(owner, undefined, mod);
  }
});

void test("the tool windows open with Alt and a digit, outside a text field only, and Option on macOS types nothing there", () => {
  const win = registryFor("Ctrl", working);
  calls.length = 0;
  assert.equal(win.dispatch(press({ key: "1", code: "Digit1", altKey: true })), true);
  assert.equal(win.dispatch(press({ key: "5", code: "Digit5", altKey: true })), true);
  assert.equal(win.dispatch(press({ key: "6", code: "Digit6", altKey: true })), true);
  assert.equal(win.dispatch(press({ key: "7", code: "Digit7", altKey: true })), true);
  assert.equal(win.dispatch(press({ key: "0", code: "Digit0", altKey: true })), true);
  assert.equal(win.dispatch(press({ key: "1", code: "Digit1", altKey: true }, true)), false);
  assert.deepEqual(calls, [
    "toggleTool changes",
    "toggleTool timeline",
    "toggleTool problems",
    "toggleTool find",
    "toggleTool structure",
  ]);

  // On a German Windows layout AltGr+digit types a character and stays text.
  const altGr = press({ key: "}", code: "Digit0", ctrlKey: true, altKey: true });
  assert.equal(win.dispatch(altGr), false);

  const mac = registryFor("Meta", working);
  calls.length = 0;
  assert.equal(mac.dispatch(press({ key: "¡", code: "Digit1", altKey: true })), true);
  assert.equal(mac.dispatch(press({ key: "¡", code: "Digit1", altKey: true }, true)), false);
  assert.deepEqual(calls, ["toggleTool changes"]);
});
