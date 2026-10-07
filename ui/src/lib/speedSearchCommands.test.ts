import assert from "node:assert/strict";
import { test } from "node:test";
import { CommandRegistry, type KeyEventLike } from "./commands.ts";
import { SEARCHING } from "./speedSearch.ts";

const space = (target: unknown): KeyEventLike => ({
  key: " ",
  code: "Space",
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  target: target as EventTarget,
  preventDefault: () => undefined,
});

/** An element inside one that has `attribute`, answering `closest` for selectors that name it. */
const insideAttribute = (attribute: string) => ({
  closest: (selector: string) => (selector.includes(`[${attribute}]`) ? {} : null),
});

function registry() {
  const ran: string[] = [];
  const commands = new CommandRegistry({}, "Ctrl");
  commands.register({ id: "toggle", label: "Toggle", binding: "Space", run: () => ran.push("toggle") });
  commands.register({
    id: "always",
    label: "Always",
    binding: "Ctrl-s",
    inFields: true,
    run: () => ran.push("always"),
  });
  return { commands, ran };
}

void test("a command's key goes to a speed search that is typing in the list with the focus", () => {
  const { commands, ran } = registry();
  assert.equal(commands.dispatch(space(insideAttribute(SEARCHING))), false);
  assert.deepEqual(ran, []);
});

void test("the same key runs the command in a list that is not searching", () => {
  const { commands, ran } = registry();
  assert.equal(commands.dispatch(space(insideAttribute("data-other"))), true);
  assert.deepEqual(ran, ["toggle"]);
});

void test("a command that works in fields works in a searching list", () => {
  const { commands, ran } = registry();
  const save = { ...space(insideAttribute(SEARCHING)), key: "s", code: "KeyS", ctrlKey: true };
  assert.equal(commands.dispatch(save), true);
  assert.deepEqual(ran, ["always"]);
});
