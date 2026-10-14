import assert from "node:assert/strict";
import { test } from "node:test";
import { CommandRegistry } from "../../lib/commands.ts";
import { keymapEntries, keymapGroups } from "./keymapRows.ts";

const run = () => undefined;

function listing(overrides: Record<string, string | null> = {}) {
  const r = new CommandRegistry<object>({}, "Ctrl");
  r.register(
    { id: "review.nextHunk", label: "Next hunk", binding: "Ctrl-Alt-ArrowDown", run },
    { id: "review.comment", label: "Add comment", binding: "Ctrl-Alt-c", run },
    { id: "zeta.thing", label: "A thing", run },
    { id: "app.settings", label: "Settings…", binding: "Ctrl-Alt-s", run },
    { id: "view.mode", label: "View mode", argument: true, run },
  );
  r.setBindings(overrides);
  return r.list();
}

void test("commands are grouped by the area of their id, in the page's order, and a command that takes an argument is not listed", () => {
  const groups = keymapGroups(listing(), {}, "");
  assert.deepEqual(
    groups.map((g) => [g.label, g.rows.map((r) => r.label)]),
    [
      ["Review", ["Add comment", "Next hunk"]],
      ["Application", ["Settings…"]],
      ["Zeta", ["A thing"]],
    ],
  );
});

void test("a search keeps the commands whose name, id, area or key contains every word", () => {
  const ids = (query: string) => keymapGroups(listing(), {}, query).flatMap((g) => g.rows.map((r) => r.id));
  assert.deepEqual(ids("hunk"), ["review.nextHunk"]);
  assert.deepEqual(ids("REVIEW comm"), ["review.comment"]);
  assert.deepEqual(ids("ctrl+alt+s"), ["app.settings"]);
  assert.deepEqual(ids("nothing like this"), []);
});

void test("a folded group shows its line and count but not its commands, and a search opens every group it found", () => {
  const groups = keymapGroups(listing(), {}, "");
  const shape = (folded: string[], searching: boolean) =>
    keymapEntries(groups, new Set(folded), searching).map((e) =>
      e.kind === "group" ? `${e.label} ${e.count} ${e.open ? "open" : "folded"}` : e.row.label,
    );
  assert.deepEqual(shape(["review"], false), [
    "Review 2 folded",
    "Application 1 open",
    "Settings…",
    "Zeta 1 open",
    "A thing",
  ]);
  assert.deepEqual(shape(["review", "zeta"], true), [
    "Review 2 open",
    "Add comment",
    "Next hunk",
    "Application 1 open",
    "Settings…",
    "Zeta 1 open",
    "A thing",
  ]);
});

void test("a row says whether its key was changed and which commands share it", () => {
  const overrides = { "review.comment": "Ctrl-Alt-s" };
  const rows = keymapGroups(listing(overrides), overrides, "").flatMap((g) => g.rows);
  const row = (id: string) => rows.find((r) => r.id === id);
  assert.deepEqual(row("review.comment")?.sharedWith, ["Settings…"]);
  assert.deepEqual(row("app.settings")?.sharedWith, ["Add comment"]);
  assert.equal(row("review.comment")?.changed, true);
  assert.equal(row("review.nextHunk")?.changed, false);
  assert.deepEqual(row("review.nextHunk")?.sharedWith, []);
});
