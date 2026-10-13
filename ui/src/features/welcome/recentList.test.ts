import assert from "node:assert/strict";
import { test } from "node:test";
import { initials, searchRecent } from "./recentList.ts";

const project = (path: string) => ({ path, openedAt: 0 });
const projects = ["/src/ledger-sync", "/src/work/billing-api", "C:\\src\\web-dashboard", "/src/vet"].map(project);

void test("initials are the first letters of the first two words, else the first two characters", () => {
  assert.equal(initials("ledger-sync"), "LS");
  assert.equal(initials("web_dashboard_ui"), "WD");
  assert.equal(initials("vet"), "VE");
  assert.equal(initials("x"), "X");
  assert.equal(initials("éclair"), "ÉC");
});

void test("search keeps the order and needs every word in the name or the path", () => {
  const names = (query: string) => searchRecent(projects, query).map((m) => m.name);
  assert.deepEqual(names(""), ["ledger-sync", "billing-api", "web-dashboard", "vet"]);
  assert.deepEqual(names("SYNC"), ["ledger-sync"]);
  assert.deepEqual(names("work api"), ["billing-api"]);
  assert.deepEqual(names("  src  "), ["ledger-sync", "billing-api", "web-dashboard", "vet"]);
  assert.deepEqual(names("nothing"), []);
});

void test("search reports where it matched, merged, in the name and the path", () => {
  const [match] = searchRecent(projects, "ger sync");
  assert.deepEqual(match.nameRanges, [
    [3, 6],
    [7, 11],
  ]);
  assert.deepEqual(match.pathRanges, [
    [8, 11],
    [12, 16],
  ]);
  assert.deepEqual(searchRecent(projects, "ledger led")[0].nameRanges, [[0, 6]]);
});

void test("search matches the first letters of a name's words", () => {
  assert.deepEqual(
    searchRecent(projects, "ls").map((m) => m.name),
    ["ledger-sync"],
  );
  assert.deepEqual(searchRecent(projects, "ls")[0].nameRanges, [
    [0, 1],
    [7, 8],
  ]);
});
