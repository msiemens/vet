import assert from "node:assert/strict";
import { test } from "node:test";
import { resolveMenu } from "../../lib/commandMenu.ts";
import { CommandRegistry, type KeyEventLike } from "../../lib/commands.ts";
import type { CommandContext, FocusedEditor } from "../../state/commands.ts";
import {
  type FileActions,
  type ViewActions,
  editorMenu,
  fileCommands,
  tabMenu,
  treeMenu,
  viewCommands,
} from "./reviewCommands.ts";

const idle: CommandContext = { repoOpen: true, reviewOpen: true, canQuit: true, diffView: false, hunk: -1 };

const calls: string[] = [];
let toggleFinds = true;
let reconfirmFinds = true;
let commentFinds = true;
let caretFinds = true;
let treeShown = true;
const view: ViewActions = {
  step: (delta) => calls.push(`step ${delta}`),
  toggleSeen: () => (calls.push("toggle"), toggleFinds),
  reconfirm: () => (calls.push("reconfirm"), reconfirmFinds),
  comment: () => (calls.push("comment"), commentFinds),
  jumpToSource: () => (calls.push("jump"), caretFinds),
  stepCommit: (delta) => (calls.push(`commit ${delta}`), true),
};
const files: FileActions = {
  copyText: (text) => calls.push(`copy ${text}`),
  excludePath: (path, dir) => calls.push(`exclude ${path} ${dir}`),
  reviewOnly: (path) => calls.push(`only ${path}`),
  closeTab: (kind, path) => calls.push(`close ${kind} ${path}`),
  closeOtherTabs: (kind, path) => calls.push(`closeOthers ${kind} ${path}`),
  revealFile: () => (calls.push("reveal"), treeShown),
};

function setup(context: Partial<CommandContext> = {}, mod: "Ctrl" | "Meta" = "Ctrl") {
  calls.length = 0;
  toggleFinds = reconfirmFinds = commentFinds = caretFinds = true;
  const registry = new CommandRegistry<CommandContext>({ ...idle, ...context }, mod);
  registry.register(...viewCommands(view), ...fileCommands(files));
  return registry;
}

const inChanges: Partial<CommandContext> = { diffView: true, tab: { kind: "changes" } };

function key(init: Partial<KeyEventLike> & { inField?: boolean }): KeyEventLike & { prevented: boolean } {
  const { inField, ...rest } = init;
  const event = {
    key: "",
    code: "",
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    target: { closest: (selector: string) => (inField && selector.includes("input") ? {} : null) } as never,
    prevented: false,
    preventDefault() {
      event.prevented = true;
    },
    ...rest,
  };
  return event;
}

const ctrlAlt = (k: string, code: string, init: Partial<KeyEventLike> = {}) =>
  key({ key: k, code, ctrlKey: true, altKey: true, ...init });
const down = () => ctrlAlt("ArrowDown", "ArrowDown");
const space = (init: Parameters<typeof key>[0] = {}) => key({ key: " ", code: "Space", ...init });

void test("Ctrl+Alt+Down and Up step the hunk, also from a text field", () => {
  const r = setup(inChanges);
  assert.equal(r.dispatch(down()), true);
  assert.equal(r.dispatch(ctrlAlt("ArrowUp", "ArrowUp", { target: { closest: () => ({}) } as never })), true);
  assert.deepEqual(calls, ["step 1", "step -1"]);
});

void test("the hunk keys leave the page alone where no view is shown", () => {
  const r = setup({ diffView: false });
  const event = down();
  assert.equal(r.dispatch(event), false);
  assert.equal(event.prevented, false);
  assert.deepEqual(calls, []);
});

void test("Space toggles, and passes on when there is nothing to toggle", () => {
  const r = setup(inChanges);
  assert.equal(r.dispatch(space()), true);
  toggleFinds = false;
  const event = space();
  assert.equal(r.dispatch(event), false);
  assert.equal(event.prevented, false);
  assert.deepEqual(calls, ["toggle", "toggle"]);
});

void test("R re-confirms, and passes on when there is no hunk seen against an old base", () => {
  const r = setup(inChanges);
  const press = (init: Parameters<typeof key>[0] = {}) => key({ key: "r", code: "KeyR", ...init });
  assert.equal(r.dispatch(press()), true);
  reconfirmFinds = false;
  const event = press();
  assert.equal(r.dispatch(event), false);
  assert.equal(event.prevented, false);
  assert.equal(r.dispatch(press({ inField: true })), false);
  assert.deepEqual(calls, ["reconfirm", "reconfirm"]);
});

void test("Space belongs to a text field", () => {
  const r = setup(inChanges);
  assert.equal(r.dispatch(space({ inField: true })), false);
  assert.deepEqual(calls, []);
});

void test("Ctrl+Alt+C comments, but not in a text field and not for AltGr+C", () => {
  const r = setup(inChanges);
  assert.equal(r.dispatch(ctrlAlt("c", "KeyC")), true);
  assert.equal(r.dispatch(ctrlAlt("c", "KeyC", { target: { closest: () => ({}) } as never })), false);
  const altGr = ctrlAlt("©", "KeyC", { getModifierState: (m: string) => m === "AltGraph" });
  assert.equal(r.dispatch(altGr), false);
  assert.deepEqual(calls, ["comment"]);
});

void test("[ and ] go to the previous and next commit, and only while reviewing by commit", () => {
  const open = key({ key: "[", code: "BracketLeft" });
  const close = key({ key: "]", code: "BracketRight" });
  const r = setup({ ...inChanges, byCommit: true });
  assert.equal(r.dispatch(open), true);
  assert.equal(r.dispatch(close), true);
  assert.deepEqual(calls, ["commit -1", "commit 1"]);

  const squash = setup(inChanges);
  const event = key({ key: "]", code: "BracketRight" });
  assert.equal(squash.dispatch(event), false);
  assert.equal(event.prevented, false);
  assert.deepEqual(calls, []);
});

void test("F4 jumps to source from the Changes tab only, and only when there is a caret", () => {
  const f4 = () => key({ key: "F4", code: "F4" });
  assert.equal(setup({ diffView: true, tab: { kind: "file", path: "a.rs" } }).dispatch(f4()), false);
  const r = setup(inChanges);
  caretFinds = false;
  const event = f4();
  assert.equal(r.dispatch(event), false);
  assert.equal(event.prevented, false);
  caretFinds = true;
  assert.equal(r.dispatch(f4()), true);
  assert.deepEqual(calls, ["jump", "jump"]);
});

void test("Mod+B goes to the definition from the focused editor: Cmd on macOS, Ctrl elsewhere", () => {
  const found: string[] = [];
  const editor: FocusedEditor = {
    side: "target",
    path: "a.rs",
    selection: false,
    selectedText: () => "",
    goToDefinition: () => found.push("definition"),
  };
  const cmd = () => key({ key: "b", code: "KeyB", metaKey: true });
  const ctrl = () => key({ key: "b", code: "KeyB", ctrlKey: true });
  const mac = setup({ editor }, "Meta");
  assert.equal(mac.dispatch(cmd()), true);
  assert.equal(mac.dispatch(ctrl()), false);
  assert.equal(setup({ editor }).dispatch(ctrl()), true);
  assert.equal(setup().dispatch(ctrl()), false);
  assert.deepEqual(found, ["definition", "definition"]);
});

void test("the file tree's menu offers the folder scope only for folders", () => {
  const r = setup();
  const labels = (dir: boolean) =>
    resolveMenu(r, treeMenu(dir), r.context({ target: { path: "src", dir } })).map((i) => i.label);
  assert.deepEqual(labels(false), ["Copy path", "Exclude from review"]);
  assert.deepEqual(labels(true), ["Copy path", "Exclude from review", "Review only this folder"]);
});

void test("a tree menu item acts on the row it was opened on", () => {
  const r = setup();
  const context = r.context({ target: { path: "src/lib", dir: true } });
  const items = resolveMenu(r, treeMenu(true), context);
  for (const item of items) r.run(item.id, context);
  assert.deepEqual(calls, ["copy src/lib", "exclude src/lib true", "only src/lib"]);
});

void test("a tab's menu cannot close the Changes tab, and closes the others around the tab it was opened on", () => {
  const r = setup();
  const menuOf = (tab: CommandContext["tab"]) => {
    const context = r.context({ tab });
    return { context, items: resolveMenu(r, tabMenu, context) };
  };
  const changes = menuOf({ kind: "changes" });
  assert.deepEqual(
    changes.items.map((i) => [i.id, i.enabled]),
    [
      ["tab.close", false],
      ["tab.closeOthers", undefined],
      ["-", undefined],
      ["path.copy", false],
    ],
  );
  const file = menuOf({ kind: "file", path: "a.rs" });
  r.run("tab.close", file.context);
  r.run("tab.closeOthers", file.context);
  r.run("path.copy", file.context);
  assert.deepEqual(calls, ["close file a.rs", "closeOthers file a.rs", "copy a.rs"]);
  assert.equal(r.run("tab.close", changes.context), false);
});

void test("Reveal the current file runs when a tree is there to show it and says so when there is none", () => {
  const r = setup();
  assert.equal(r.run("view.revealFile"), true);
  treeShown = false;
  assert.equal(r.run("view.revealFile"), false);
  treeShown = true;
  assert.deepEqual(calls, ["reveal", "reveal"]);
});

void test("the editor's menu offers copy only with a selection, and shows the keys of its commands", () => {
  const r = setup({ ...inChanges });
  const editor = (selection: boolean): FocusedEditor => ({
    side: "target",
    path: "a.rs",
    selection,
    selectedText: () => "text",
    goToDefinition: () => undefined,
  });
  const items = (selection: boolean) => resolveMenu(r, editorMenu, r.context({ editor: editor(selection) }));
  const copy = (list: ReturnType<typeof items>) => list.find((i) => i.id === "edit.copy");
  assert.equal(copy(items(false))?.enabled, false);
  assert.equal(copy(items(true))?.enabled, undefined);
  const shortcuts = Object.fromEntries(items(true).map((i) => [i.id, i.shortcut]));
  assert.equal(shortcuts["review.comment"], "Ctrl+Alt+C");
  assert.equal(shortcuts["review.goToDefinition"], "Ctrl+B");
  assert.equal(shortcuts["review.toggleSeen"], "Space");
});
