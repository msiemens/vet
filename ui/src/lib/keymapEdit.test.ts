import assert from "node:assert/strict";
import { test } from "node:test";
import { CommandRegistry, type KeyEventLike } from "./commands.ts";
import { type Overrides, conflictsOf, parseOverrides, withBinding, withReplacedBinding } from "./keymapEdit.ts";

const registry = () => {
  const r = new CommandRegistry<object>({}, "Ctrl");
  const run = () => undefined;
  r.register(
    { id: "a.next", label: "Next", binding: "Ctrl-Alt-ArrowDown", run },
    { id: "a.open", label: "Open", binding: "Mod-o", run },
    { id: "b.free", label: "Free", run },
    { id: "b.pick", label: "Pick", argument: true, binding: "Ctrl-p", run },
  );
  return r;
};

const key = (init: Partial<KeyEventLike>): KeyEventLike & { prevented: boolean } => {
  const event = {
    key: "",
    code: "",
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    prevented: false,
    preventDefault() {
      event.prevented = true;
    },
    ...init,
  };
  return event;
};

const apply = (r: CommandRegistry<object>, overrides: Overrides) => {
  r.setBindings(overrides);
  return r.list();
};

void test("a key that another command has is reported with that command", () => {
  const listing = registry().list();
  assert.deepEqual(
    conflictsOf(listing, "b.free", "Alt-Ctrl-ArrowDown").map((c) => c.label),
    ["Next"],
  );
  assert.deepEqual(conflictsOf(listing, "a.next", "Ctrl-Alt-ArrowDown"), []);
  assert.deepEqual(conflictsOf(listing, "b.free", "Ctrl-Alt-x"), []);
});

void test("only a binding that differs from the default is an override", () => {
  const listing = registry().list();
  const [next, , free] = listing;
  assert.deepEqual(withBinding({}, free, "Ctrl-f"), { "b.free": "Ctrl-f" });
  assert.deepEqual(withBinding({ "a.next": "Alt-n" }, next, "Alt-Ctrl-ArrowDown"), {});
  assert.deepEqual(withBinding({}, next, null), { "a.next": null });
  assert.deepEqual(withBinding({ "b.free": "Ctrl-f" }, free, null), {});
});

void test("taking over a key unbinds the command that had it", () => {
  const r = registry();
  const overrides = withReplacedBinding({}, r.list(), "b.free", "Ctrl-o");
  assert.deepEqual(overrides, { "b.free": "Ctrl-o", "a.open": null });
  r.setBindings(overrides);
  assert.equal(r.binding("a.open"), undefined);
  assert.equal(r.binding("b.free")?.label, "Ctrl+O");
});

void test("a further key of a command clashes like its main key, and taking it over leaves the main key", () => {
  const r = registry();
  r.register({ id: "t.step", label: "Step", binding: "Alt-ArrowRight", alsoBindings: ["Ctrl-PageDown"], run: () => 1 });
  const listing = r.list();
  assert.deepEqual(
    listing.find((c) => c.id === "t.step")?.alsoBindings.map((b) => b.label),
    ["Ctrl+PageDown"],
  );
  assert.deepEqual(
    conflictsOf(listing, "b.free", "Ctrl-PageDown").map((c) => c.label),
    ["Step"],
  );

  const overrides = withReplacedBinding({}, listing, "b.free", "Ctrl-PageDown");
  r.setBindings(overrides);
  assert.equal(r.binding("t.step")?.label, "Alt+Right");
  assert.equal(r.binding("b.free")?.label, "Ctrl+PageDown");
  const taken = key({ key: "PageDown", code: "PageDown", ctrlKey: true });
  assert.equal(r.dispatch(taken), true);
  assert.deepEqual(r.list().find((c) => c.id === "t.step")?.alsoBindings, []);
});

void test("overrides survive being kept, and damaged ones cost only themselves", () => {
  const overrides: Overrides = { "a.next": "Alt-n", "a.open": null };
  const kept = (text: string) => parseOverrides(JSON.parse(text) as unknown);
  assert.deepEqual(kept(JSON.stringify(overrides)), overrides);
  assert.deepEqual(kept("[1]"), {});
  assert.deepEqual(kept('{"a.next":"Alt-n","a.open":3,"b.free":""}'), { "a.next": "Alt-n" });
});

void test("kept overrides decide which key runs a command and what its tooltip says", () => {
  const r = registry();
  const ran: string[] = [];
  const down = () => key({ key: "ArrowDown", code: "ArrowDown", ctrlKey: true, altKey: true });
  r.register({ id: "c.go", label: "Go", binding: "Ctrl-g", run: () => ran.push("go") });
  assert.equal(r.tooltip("c.go"), "Go (Ctrl+G)");

  apply(r, parseOverrides({ "c.go": "Alt-j", "a.next": null }));
  assert.equal(r.tooltip("c.go"), "Go (Alt+J)");
  assert.equal(r.tooltip("a.next"), "Next");
  assert.equal(r.dispatch(key({ key: "g", code: "KeyG", ctrlKey: true })), false);
  assert.equal(r.dispatch(key({ key: "j", code: "KeyJ", altKey: true })), true);
  assert.equal(r.dispatch(down()), false);
  assert.deepEqual(ran, ["go"]);

  apply(r, {});
  assert.equal(r.tooltip("c.go"), "Go (Ctrl+G)");
  assert.equal(r.dispatch(down()), true);
});

void test("the listing offers the commands a key can run, with their default keys", () => {
  const r = registry();
  r.setBindings({ "a.open": "Alt-o", "a.next": null });
  const listing = r.list();
  assert.deepEqual(
    listing.map((c) => [c.id, c.binding?.label, c.defaultBinding?.label]),
    [
      ["a.next", undefined, "Ctrl+Alt+Down"],
      ["a.open", "Alt+O", "Ctrl+O"],
      ["b.free", undefined, undefined],
    ],
  );
});

void test("while a key is being read no command runs, and the keys are taken from the page", () => {
  const r = registry();
  const ran: string[] = [];
  r.register({ id: "c.go", label: "Go", binding: "Ctrl-g", run: () => ran.push("go") });
  const seen: string[] = [];
  const stop = r.capture((e) => seen.push(e.key));
  let stopped = false;
  const press = key({ key: "g", code: "KeyG", ctrlKey: true });
  press.stopPropagation = () => (stopped = true);
  assert.equal(r.dispatch(press), true);
  assert.deepEqual([seen, ran, press.prevented, stopped], [["g"], [], true, true]);
  stop();
  assert.equal(r.dispatch(key({ key: "g", code: "KeyG", ctrlKey: true })), true);
  assert.deepEqual(ran, ["go"]);
});
