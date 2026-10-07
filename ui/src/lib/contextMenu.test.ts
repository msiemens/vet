import assert from "node:assert/strict";
import { test } from "node:test";
import { type ContextMenuRequest, nativeItems, openContextMenu } from "./contextMenu.ts";
import type { MenuItem } from "./ui/menuItem.ts";

const sep: MenuItem = { id: "-", label: "", separator: true };

void test("the host's menu shows shortcuts as text and registers no accelerator", () => {
  const items: MenuItem[] = [{ id: "copy", label: "Copy", shortcut: "Ctrl+C", accelerator: "Cmd+C", checked: true }];
  assert.deepEqual(nativeItems(items), [{ id: "copy", label: "Copy", shortcut: "Ctrl+C", checked: true }]);
});

void test("separators that lead, trail or double up and submenus without items are left out", () => {
  const items: MenuItem[] = [
    sep,
    { id: "a", label: "A" },
    sep,
    sep,
    { id: "empty", label: "Empty", children: [] },
    { id: "sub", label: "Sub", children: [sep, { id: "b", label: "B", enabled: false }, sep] },
    sep,
  ];
  assert.deepEqual(nativeItems(items), [
    { id: "a", label: "A" },
    sep,
    { id: "sub", label: "Sub", children: [{ id: "b", label: "B", enabled: false }] },
  ]);
});

function request(chosen: string[]): ContextMenuRequest {
  return { x: 3, y: 4, items: [{ id: "a", label: "A" }], onselect: (id) => chosen.push(id) };
}

void test("a choice in the host's menu runs after the focus is back", async () => {
  const log: string[] = [];
  const shown: unknown[] = [];
  const native = (items: MenuItem[], x: number, y: number) => {
    shown.push([items, x, y]);
    return Promise.resolve("a");
  };
  await openContextMenu(
    { ...request(log), onselect: (id) => log.push(`run ${id}`) },
    native,
    () => log.push("draw"),
    () => log.push("focus"),
  );
  assert.deepEqual(log, ["focus", "run a"]);
  assert.deepEqual(shown, [[[{ id: "a", label: "A" }], 3, 4]]);
});

void test("a menu closed without a choice runs nothing", async () => {
  const chosen: string[] = [];
  const log: string[] = [];
  await openContextMenu(
    request(chosen),
    () => Promise.resolve(null),
    () => log.push("draw"),
    () => log.push("focus"),
  );
  assert.deepEqual([chosen, log], [[], ["focus"]]);
});

void test("without a host the page draws the menu, and it also does when the host fails", async () => {
  const drawn: ContextMenuRequest[] = [];
  const errors = console.error;
  console.error = () => undefined;
  try {
    await openContextMenu(
      request([]),
      undefined,
      (r) => drawn.push(r),
      () => assert.fail("focus"),
    );
    await openContextMenu(
      request([]),
      () => Promise.reject(new Error("no menu")),
      (r) => drawn.push(r),
      () => assert.fail("focus"),
    );
  } finally {
    console.error = errors;
  }
  assert.equal(drawn.length, 2);
});
