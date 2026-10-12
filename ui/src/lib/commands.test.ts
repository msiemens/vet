import assert from "node:assert/strict";
import { test } from "node:test";
import { type Command, CommandRegistry, type KeyEventLike } from "./commands.ts";
import { type MenuSpec, resolveMenu } from "./commandMenu.ts";

interface Ctx {
  open: boolean;
  tab?: string;
}

function registry(mod: "Ctrl" | "Meta" = "Ctrl") {
  return new CommandRegistry<Ctx>({ open: false }, mod);
}

interface KeyInit extends Partial<KeyEventLike> {
  inField?: boolean;
}

/** A keydown; `inField` puts its target in a text field. */
function key(init: KeyInit): KeyEventLike & { prevented: boolean } {
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

const ctrlAltDown = (init: KeyInit = {}) =>
  key({ key: "ArrowDown", code: "ArrowDown", ctrlKey: true, altKey: true, ...init });

function next(run: () => unknown, extra: Partial<Command<Ctx>> = {}): Command<Ctx> {
  return { id: "next", label: "Next", binding: "Ctrl-Alt-ArrowDown", run, ...extra };
}

void test("a key runs the command bound to it and the event is taken", () => {
  const r = registry();
  const ran: string[] = [];
  r.register(next(() => ran.push("next")));
  const event = ctrlAltDown();
  assert.equal(r.dispatch(event), true);
  assert.deepEqual(ran, ["next"]);
  assert.equal(event.prevented, true);
});

void test("a key without a binding is left alone", () => {
  const r = registry();
  r.register(next(() => assert.fail("ran")));
  const event = key({ key: "ArrowDown", code: "ArrowDown", altKey: true });
  assert.equal(r.dispatch(event), false);
  assert.equal(event.prevented, false);
});

void test("a command that is not enabled in the context does not run and leaves the key", () => {
  const r = registry();
  const ran: boolean[] = [];
  r.register(next(() => ran.push(true), { enabled: (c) => c.open }));
  const event = ctrlAltDown();
  assert.equal(r.dispatch(event), false);
  assert.equal(event.prevented, false);
  r.provide(() => ({ open: true }));
  assert.equal(r.dispatch(ctrlAltDown()), true);
  assert.deepEqual(ran, [true]);
});

void test("a command that finds nothing to do leaves its key to the page", () => {
  const r = registry();
  r.register(next(() => false));
  const event = ctrlAltDown();
  assert.equal(r.dispatch(event), false);
  assert.equal(event.prevented, false);
});

void test("the command bound to a key is the one enabled in the context", () => {
  const r = registry();
  const ran: string[] = [];
  r.register(
    { id: "a", label: "A", binding: "F4", enabled: (c) => c.tab === "a", run: () => ran.push("a") },
    { id: "b", label: "B", binding: "F4", enabled: (c) => c.tab === "b", run: () => ran.push("b") },
  );
  r.provide(() => ({ tab: "b" }));
  r.dispatch(key({ key: "F4", code: "F4" }));
  assert.deepEqual(ran, ["b"]);
});

void test("a text field keeps its keys unless the command is meant for fields", () => {
  const r = registry();
  const ran: string[] = [];
  r.register(
    { id: "space", label: "Space", binding: "Space", run: () => ran.push("space") },
    next(() => ran.push("next"), { inFields: true }),
  );
  const space = key({ key: " ", code: "Space", inField: true });
  assert.equal(r.dispatch(space), false);
  assert.equal(space.prevented, false);
  assert.equal(r.dispatch(ctrlAltDown({ inField: true })), true);
  assert.deepEqual(ran, ["next"]);
});

void test("AltGr characters, dead keys and composition are never taken", () => {
  const r = registry();
  r.register(
    { id: "comment", label: "Comment", binding: "Ctrl-Alt-c", run: () => assert.fail("took AltGr") },
    { id: "settings", label: "Settings", binding: "Ctrl-Alt-s", run: () => assert.fail("took AltGr") },
  );
  const altGr = (k: string, code: string) =>
    key({ key: k, code, ctrlKey: true, altKey: true, getModifierState: (m) => m === "AltGraph" });
  assert.equal(r.dispatch(altGr("©", "KeyC")), false);
  assert.equal(r.dispatch(altGr("ß", "KeyS")), false);
  assert.equal(r.dispatch(key({ key: "Dead", code: "KeyC", ctrlKey: true, altKey: true })), false);
  assert.equal(r.dispatch(key({ key: "c", code: "KeyC", ctrlKey: true, altKey: true, isComposing: true })), false);
});

void test("Mod is Cmd on macOS and Ctrl elsewhere", () => {
  const cmd = { key: "b", code: "KeyB", metaKey: true };
  const ctrl = { key: "b", code: "KeyB", ctrlKey: true };
  const mac = registry("Meta");
  const other = registry();
  for (const r of [mac, other]) r.register({ id: "def", label: "Definition", binding: "Mod-b", run: () => undefined });
  assert.equal(mac.dispatch(key(cmd)), true);
  assert.equal(mac.dispatch(key(ctrl)), false);
  assert.equal(other.dispatch(key(ctrl)), true);
  assert.equal(other.dispatch(key(cmd)), false);
  assert.equal(mac.binding("def")?.label, "⌘B");
  assert.equal(other.binding("def")?.label, "Ctrl+B");
});

void test("a binding set as data replaces the default, and null removes it", () => {
  const r = registry();
  const ran: string[] = [];
  r.register(next(() => ran.push("next")));
  r.setBindings({ next: "Alt-n" });
  assert.equal(r.dispatch(ctrlAltDown()), false);
  assert.equal(r.dispatch(key({ key: "n", code: "KeyN", altKey: true })), true);
  assert.equal(r.binding("next")?.label, "Alt+N");
  r.setBindings({ next: null });
  assert.equal(r.binding("next"), undefined);
  assert.equal(r.dispatch(key({ key: "n", code: "KeyN", altKey: true })), false);
  assert.deepEqual(ran, ["next"]);
});

void test("unregistering removes the command and its key", () => {
  const r = registry();
  const off = r.register(next(() => assert.fail("ran")));
  off();
  assert.equal(r.dispatch(ctrlAltDown()), false);
  assert.equal(r.find("next"), undefined);
});

void test("two commands cannot share an id", () => {
  const r = registry();
  r.register(next(() => undefined));
  assert.throws(() => r.register(next(() => undefined)));
});

void test("a menu item runs its command with its argument, if the command is enabled", () => {
  const r = registry();
  const ran: (string | undefined)[] = [];
  r.register({ id: "open", label: "Open", enabled: (c) => c.open, run: (_, arg) => ran.push(arg) });
  assert.equal(r.run("open:/a:b"), false);
  r.provide(() => ({ open: true }));
  assert.equal(r.run("open:/a:b"), true);
  assert.deepEqual(ran, ["/a:b"]);
});

void test("the context merges sources and the subject of a context menu", () => {
  const r = registry();
  r.provide(() => ({ open: true }));
  r.provide(() => ({ tab: "t" }));
  assert.deepEqual(r.context({ tab: "u" }), { open: true, tab: "u" });
});

void test("a tooltip names the command's key", () => {
  const r = registry();
  r.register(
    next(() => undefined),
    { id: "plain", label: "Plain", run: () => undefined },
  );
  assert.equal(r.tooltip("next"), "Next (Ctrl+Alt+Down)");
  assert.equal(r.tooltip("next", "Go on"), "Go on (Ctrl+Alt+Down)");
  assert.equal(r.tooltip("plain"), "Plain");
});

void test("a control that does a command's job is titled by why the command cannot run, else by its key", () => {
  const r = registry();
  r.register(
    { ...next(() => undefined), enabled: (c) => c.open, disabledReason: () => "Open it first" },
    { id: "plain", label: "Plain", enabled: () => false, run: () => undefined },
  );
  assert.equal(r.hint("next", undefined, { open: false }), "Open it first");
  assert.equal(r.hint("next", undefined, { open: true }), "Next (Ctrl+Alt+Down)");
  assert.equal(r.hint("plain", "Do it", { open: false }), "Do it");
});

void test("a key spec reads as the platform writes keys", () => {
  assert.equal(registry("Ctrl").keyLabel("Mod-Enter"), "Ctrl+Enter");
  assert.equal(registry("Meta").keyLabel("Mod-Enter"), "⌘↩");
});

void test("menu items take label, shortcut and enabled state from the registry", () => {
  const r = registry();
  r.register(
    { ...next(() => undefined), accelerator: "Cmd+J", enabled: (c) => c.open },
    { id: "pick", label: "Pick", run: () => undefined },
  );
  const spec: MenuSpec = [
    "-",
    { command: "next" },
    "-",
    { command: "pick", arg: "x", label: "Pick x", checked: true },
    "-",
  ];
  const items = resolveMenu(r, spec, r.context());
  assert.deepEqual(items, [
    { id: "next", label: "Next", shortcut: "Ctrl+Alt+Down", chord: "Ctrl+Alt+Down", enabled: false },
    { id: "-", label: "", separator: true },
    { id: "pick:x", label: "Pick x", checked: true },
  ]);
  const [native] = resolveMenu(r, [{ command: "next" }], { open: true }, true);
  assert.equal(native.accelerator, "Cmd+J");
  assert.equal(native.shortcut, undefined);
  const [bare] = resolveMenu(r, [{ command: "pick" }], { open: true }, true);
  assert.equal(bare.accelerator, undefined);
});

void test("a submenu is disabled when none of its items is enabled, and commands that are not registered are left out", () => {
  const r = registry();
  r.register({ id: "pick", label: "Pick", enabled: (c) => c.open, run: () => undefined });
  const spec = [{ id: "sub", label: "Sub", items: [{ command: "pick" }, { command: "missing" }] }];
  const [sub] = resolveMenu(r, spec, { open: false });
  assert.equal(sub.enabled, false);
  assert.deepEqual(
    sub.children?.map((c) => c.id),
    ["pick"],
  );
  assert.equal(resolveMenu(r, spec, { open: true })[0].enabled, undefined);
});

void test("a command may have another default key on macOS, and a binding set as data still replaces it", () => {
  const tabs = (mod: "Ctrl" | "Meta") => {
    const r = registry(mod);
    r.register({
      id: "tab",
      label: "Tab",
      binding: "Alt-ArrowRight",
      macBinding: "Meta-Shift-BracketRight",
      run: () => 1,
    });
    return r;
  };
  assert.equal(tabs("Ctrl").binding("tab")?.spec, "Alt-ArrowRight");
  assert.equal(tabs("Meta").binding("tab")?.spec, "Meta-Shift-BracketRight");
  assert.equal(tabs("Meta").list()[0].defaultBinding?.spec, "Meta-Shift-BracketRight");
  const r = tabs("Meta");
  r.setBindings({ tab: "Alt-t" });
  assert.equal(r.binding("tab")?.spec, "Alt-t");
});

void test("Option+digit runs a command on macOS outside a text field, and types in one", () => {
  const r = registry("Meta");
  const ran: string[] = [];
  r.register({ id: "tool", label: "Tool", binding: "Alt-1", run: () => ran.push("tool") });
  assert.equal(r.dispatch(key({ key: "¡", code: "Digit1", altKey: true })), true);
  assert.equal(r.dispatch(key({ key: "¡", code: "Digit1", altKey: true, inField: true })), false);
  assert.deepEqual(ran, ["tool"]);
});

void test("a disabled command can say why, and a menu item carries the reason, from the command or from the menu", () => {
  const r = registry();
  r.register(
    { id: "a", label: "A", enabled: (c) => c.open, disabledReason: () => "Open it first", run: () => undefined },
    { id: "b", label: "B", enabled: () => false, run: () => undefined },
    { id: "c", label: "C", run: () => undefined },
  );
  assert.equal(r.disabledReason("a", { open: false }), "Open it first");
  assert.equal(r.disabledReason("a", { open: true }), undefined);
  assert.equal(r.disabledReason("b", { open: false }), undefined);
  const items = resolveMenu(r, [{ command: "a" }, { command: "b" }, { command: "c", disabled: "Not here" }], {
    open: false,
  });
  assert.deepEqual(
    items.map((i) => [i.id, i.enabled, i.reason]),
    [
      ["a", false, "Open it first"],
      ["b", false, undefined],
      ["c", false, "Not here"],
    ],
  );
  const [enabled] = resolveMenu(r, [{ command: "a" }], { open: true });
  assert.equal(enabled.reason, undefined);
});

void test("radio and predefined entries reach the menu as such", () => {
  const r = registry();
  r.register({ id: "pick", label: "Pick", run: () => undefined });
  const items = resolveMenu(r, [{ command: "pick", arg: "x", checked: true, radio: true }, { predefined: "undo" }], {
    open: false,
  });
  assert.deepEqual(items, [
    { id: "pick:x", label: "Pick", checked: true, radio: true },
    { id: "predefined.undo", label: "", predefined: "undo" },
  ]);
});
