import assert from "node:assert/strict";
import { test } from "node:test";
import { AVATAR_COLOURS, avatarColour, initials, openedLabel, searchRecent } from "./recentList.ts";

const project = (path: string) => ({ path, openedAt: 0 });
const projects = ["/src/ledger-sync", "/src/work/billing-api", "C:\\src\\web-dashboard", "/src/vet"].map(project);

void test("initials are the first letters of the first two words, else the first two characters", () => {
  assert.equal(initials("ledger-sync"), "LS");
  assert.equal(initials("web_dashboard_ui"), "WD");
  assert.equal(initials("vet"), "VE");
  assert.equal(initials("x"), "X");
  assert.equal(initials("éclair"), "ÉC");
});

void test("a project keeps its avatar colour, within the palette", () => {
  for (const { path } of projects) {
    const colour = avatarColour(path);
    assert.equal(colour, avatarColour(path));
    assert.ok(colour >= 1 && colour <= AVATAR_COLOURS);
  }
  const used = new Set(Array.from({ length: 60 }, (_, i) => avatarColour(`/src/project-${i}`)));
  assert.ok(used.size > 1, "paths spread over the palette");
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

void test("the opened label shortens with distance", () => {
  const now = new Date(2026, 9, 7, 15, 30).getTime();
  const at = (...date: [number, number, number?, number?, number?]) => new Date(...date).getTime() / 1000;
  assert.equal(openedLabel(now / 1000 - 20, now), "now");
  assert.equal(openedLabel(at(2026, 9, 7, 9, 5), now), "9:05 AM");
  assert.equal(openedLabel(at(2026, 9, 5, 23, 0), now), "Mon");
  assert.equal(openedLabel(at(2026, 8, 28, 12), now), "Sep 28");
  assert.equal(openedLabel(at(2025, 11, 31, 12), now), "Dec 31, 2025");
});
