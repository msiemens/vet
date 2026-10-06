import assert from "node:assert/strict";
import { test } from "node:test";
import type { MenuItem } from "../lib/ui/menuItem.ts";
import { type MenuContext, buildMenu, parseCommand } from "./mainMenu.ts";

const context: MenuContext = {
  mac: false,
  repoOpen: true,
  reviewOpen: true,
  canQuit: true,
  recent: [{ path: "/a/vet", name: "vet" }],
  mode: "split",
  theme: "dark",
  tools: [{ id: "files", label: "Files", shown: true }],
  devPanel: false,
};

const flatten = (items: MenuItem[]): MenuItem[] => items.flatMap((i) => [i, ...flatten(i.children ?? [])]);
const find = (items: MenuItem[], id: string) => flatten(items).find((i) => i.id === id);

void test("every leaf id parses to a command", () => {
  for (const mac of [false, true]) {
    const menu = buildMenu({ ...context, mac });
    for (const item of flatten(menu)) {
      if (item.children || item.separator) continue;
      assert.ok(parseCommand(item.id), item.id);
    }
  }
});

void test("the macOS menu bar has an app menu; elsewhere Settings and Exit are in File", () => {
  const mac = buildMenu({ ...context, mac: true });
  assert.deepEqual(
    mac.map((m) => m.id),
    ["app", "file", "view", "help"],
  );
  assert.equal(find(mac, "settings")?.shortcut, "Cmd+,");
  const other = buildMenu(context);
  assert.deepEqual(
    other.map((m) => m.id),
    ["file", "view", "help"],
  );
  assert.equal(find(other, "settings")?.shortcut, "Ctrl+Alt+S");
  assert.ok(other[0].children?.some((i) => i.id === "settings"));
});

void test("state shows as checks and enabled flags", () => {
  const menu = buildMenu({ ...context, recent: [], repoOpen: false });
  assert.equal(find(menu, "mode:split")?.checked, true);
  assert.equal(find(menu, "mode:unified")?.checked, false);
  assert.equal(find(menu, "theme:dark")?.checked, true);
  assert.equal(find(menu, "recent")?.enabled, false);
  assert.equal(find(menu, "tools")?.enabled, false);
});

void test("Finish Review is offered only while a review is open", () => {
  assert.equal(find(buildMenu(context), "finishReview")?.enabled, true);
  assert.equal(find(buildMenu({ ...context, reviewOpen: false }), "finishReview")?.enabled, false);
  assert.deepEqual(parseCommand("finishReview"), { type: "finishReview" });
});

void test("commands carry their argument, including paths with colons", () => {
  assert.deepEqual(parseCommand("recent:C:/src/vet"), { type: "openRecent", path: "C:/src/vet" });
  assert.deepEqual(parseCommand("theme:light"), { type: "theme", theme: "light" });
  assert.equal(parseCommand("theme:neon"), undefined);
});
