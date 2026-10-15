import assert from "node:assert/strict";
import { test } from "node:test";
import {
  aimIndex,
  bestMatch,
  highlightRuns,
  match,
  sliceRanges,
  speedSearchKey,
  stepMatch,
  type SpeedSearchKeyEvent,
} from "./speedSearch.ts";

const ranges = (query: string, text: string) => match(query, text)?.ranges;
const score = (query: string, text: string) => match(query, text)?.score ?? -Infinity;

void test("the search aims at the best match among the rows that matched, not the first", () => {
  const names = ["a_changeset", "x", "changeset", "y"];
  assert.equal(
    aimIndex("changeset", [0, 2], (i) => names[i]),
    2,
  );
  assert.equal(
    aimIndex("changeset", [], (i) => names[i]),
    -1,
  );
});

void test("Up and Down step from match to match, wrap round, and start at the end they come from", () => {
  const found = [2, 5, 9];
  assert.equal(stepMatch(found, 5, 1), 9);
  assert.equal(stepMatch(found, 9, 1), 2);
  assert.equal(stepMatch(found, 2, -1), 9);
  assert.equal(stepMatch(found, -1, 1), 2);
  assert.equal(stepMatch(found, 7, -1), 9);
  assert.equal(stepMatch([], 0, 1), undefined);
});

void test("the capitals of a name's words match it, and the match reports where", () => {
  assert.deepEqual(ranges("CSV", "ChangesetView"), [
    [0, 1],
    [6, 7],
    [9, 10],
  ]);
  assert.deepEqual(ranges("csv", "ChangesetView"), ranges("CSV", "ChangesetView"));
});

void test("words are also split at separators and acronyms", () => {
  assert.deepEqual(ranges("rc", "release_candidate"), [
    [0, 1],
    [8, 9],
  ]);
  assert.deepEqual(ranges("vc", "vet-core"), [
    [0, 1],
    [4, 5],
  ]);
  assert.deepEqual(ranges("xp", "XMLParser"), [
    [0, 1],
    [3, 4],
  ]);
});

void test("the characters of a match run on from a word's start", () => {
  assert.deepEqual(ranges("chaV", "ChangesetView"), [
    [0, 3],
    [9, 10],
  ]);
});

void test("a plain run of the text matches wherever it is", () => {
  assert.deepEqual(ranges("ange", "ChangesetView"), [[2, 6]]);
  assert.deepEqual(ranges("setV", "ChangesetView"), [[6, 10]]);
  assert.deepEqual(ranges(".git", "a.gitignore"), [[1, 5]]);
});

void test("a word cannot be skipped to reach the inside of a later one", () => {
  assert.equal(match("ci", "ChangesetView"), undefined);
  assert.equal(match("xyz", "ChangesetView"), undefined);
  assert.equal(match("longerthanthetext", "short"), undefined);
});

void test("an empty query matches every name and marks nothing", () => {
  assert.deepEqual(match("", "anything"), { score: 0, ranges: [] });
});

void test("a name that starts with the query ranks above one with the query at a later word", () => {
  assert.ok(score("ma", "main.rs") > score("ma", "Cargo_main"));
});

void test("characters on word starts rank above characters scattered within one word", () => {
  assert.ok(score("cv", "ChangesetView") > score("cv", "Canvas"));
});

void test("a match on word starts ranks above a plain run in the middle of a word", () => {
  assert.ok(score("ge", "GroupEntry") > score("ge", "Image"));
});

void test("of equal matches the shorter name ranks higher", () => {
  assert.ok(score("ab", "ab") > score("ab", "ab_longer_name"));
});

void test("the best match is the highest ranked, the first of equals, and -1 for none", () => {
  assert.equal(bestMatch("cs", ["xcs", "CacheStore", "CsvStore"]), 2);
  assert.equal(bestMatch("a", ["b", "a1", "a2"]), 1);
  assert.equal(bestMatch("q", ["b", "a"]), -1);
});

void test("a match across the cut between two parts is split between them", () => {
  const ranges: [number, number][] = [
    [1, 3],
    [5, 7],
  ];
  assert.deepEqual(sliceRanges(ranges, 0, 2), [[1, 2]]);
  assert.deepEqual(sliceRanges(ranges, 2, 6), [
    [0, 1],
    [3, 4],
  ]);
  assert.deepEqual(sliceRanges(ranges, 3, 5), []);
});

void test("highlight runs cover the text exactly", () => {
  assert.deepEqual(highlightRuns("ledger-sync", [[0, 3]]), [
    { text: "led", hit: true },
    { text: "ger-sync", hit: false },
  ]);
  assert.deepEqual(highlightRuns("abc", []), [{ text: "abc", hit: false }]);
  assert.deepEqual(
    highlightRuns("abcd", [
      [1, 2],
      [3, 4],
    ]).map((r) => r.text),
    ["a", "b", "c", "d"],
  );
});

const key = (k: string, extra: Partial<SpeedSearchKeyEvent> = {}): SpeedSearchKeyEvent => ({
  key: k,
  code: "",
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...extra,
});

/** A target inside an element that matches `selector`, as `closest` answers. */
const inside = (tag: string) =>
  ({ closest: (selector: string) => (selector.includes(tag) ? {} : null) }) as EventTarget;

void test("typing characters builds the query", () => {
  assert.equal(speedSearchKey("", key("c")), "c");
  assert.equal(speedSearchKey("c", key("S", { shiftKey: true })), "cS");
  assert.equal(speedSearchKey("ab", key("-")), "ab-");
});

void test("a space joins a query but does not start one, so it can still press a button", () => {
  assert.equal(speedSearchKey("", key(" ")), undefined);
  assert.equal(speedSearchKey("a", key(" ")), "a ");
});

void test("keys typed into a text field or an editor are not the search's", () => {
  for (const tag of ["input", "textarea", "contenteditable"]) {
    assert.equal(speedSearchKey("", key("a", { target: inside(tag) })), undefined, tag);
    assert.equal(speedSearchKey("ab", key("Backspace", { target: inside(tag) })), undefined, tag);
    assert.equal(speedSearchKey("ab", key("Escape", { target: inside(tag) })), undefined, tag);
  }
});

void test("Escape clears an active search and is left to its owner (a dialog, a menu) otherwise", () => {
  assert.equal(speedSearchKey("abc", key("Escape")), "");
  assert.equal(speedSearchKey("", key("Escape")), undefined);
});

void test("Backspace removes the last character, a whole one even outside the BMP", () => {
  assert.equal(speedSearchKey("abc", key("Backspace")), "ab");
  assert.equal(speedSearchKey("a😀", key("Backspace")), "a");
  assert.equal(speedSearchKey("", key("Backspace")), undefined);
});

void test("shortcuts, navigation keys and composition are not typing", () => {
  assert.equal(speedSearchKey("a", key("b", { ctrlKey: true })), undefined);
  assert.equal(speedSearchKey("a", key("b", { metaKey: true })), undefined);
  for (const named of ["ArrowDown", "Enter", "Tab", "Home", "F4", "Shift"]) {
    assert.equal(speedSearchKey("a", key(named)), undefined, named);
  }
  assert.equal(speedSearchKey("a", key("Dead")), undefined);
  assert.equal(speedSearchKey("a", key("x", { isComposing: true })), undefined);
  assert.equal(speedSearchKey("a", key("x", { keyCode: 229 })), undefined);
});

void test("characters typed with AltGr or Option are typed", () => {
  assert.equal(speedSearchKey("", key("@", { ctrlKey: true, altKey: true })), "@");
  assert.equal(speedSearchKey("", key("å", { altKey: true })), "å");
});

// The matcher as it was before it was made fast, kept here as the reference the new one must equal.
const R_HUMP = 5;
const R_CONSECUTIVE = 6;
const R_WITHIN_WORD = 2;
const R_FIRST = 10;
const R_AT_START = 4;
const R_SAME_CASE = 1;
const rLetter = (c: string) => /\p{L}/u.test(c);
const rDigit = (c: string) => /\p{N}/u.test(c);
const rUpper = (c: string) => rLetter(c) && c === c.toUpperCase() && c !== c.toLowerCase();
const rLower = (c: string) => rLetter(c) && c === c.toLowerCase() && c !== c.toUpperCase();
const rSame = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

function referenceHumpStarts(text: string): boolean[] {
  return text.split("").map((c, i) => {
    if (!rLetter(c) && !rDigit(c)) return false;
    if (i === 0) return true;
    const before = text.charAt(i - 1);
    if (!rLetter(before) && !rDigit(before)) return true;
    if (!rUpper(c)) return false;
    return rLower(before) || rDigit(before) || (rUpper(before) && rLower(text.charAt(i + 1)));
  });
}

function referenceRanges(indices: number[]): [number, number][] {
  const ranges: [number, number][] = [];
  for (const i of indices) {
    const last = ranges.at(-1);
    if (last?.[1] === i) last[1] = i + 1;
    else ranges.push([i, i + 1]);
  }
  return ranges;
}

function referenceHumps(query: string, text: string): { score: number; ranges: [number, number][] } | undefined {
  const m = query.length;
  const n = text.length;
  if (m > n) return undefined;
  const hump = referenceHumpStarts(text);
  const humpsUpTo: number[] = [];
  let count = 0;
  for (const h of hump) humpsUpTo.push((count += h ? 1 : 0));
  const best: { score: number; from: number }[][] = [];
  for (let j = 0; j < m; j++) {
    const row: { score: number; from: number }[] = Array.from({ length: n }, () => ({ score: -Infinity, from: -1 }));
    for (let i = 0; i < n; i++) {
      const c = text.charAt(i);
      if (!rSame(query.charAt(j), c)) continue;
      const exact = query.charAt(j) === c ? R_SAME_CASE : 0;
      if (j === 0) {
        if (hump[i]) row[i].score = R_FIRST + (i === 0 ? R_AT_START : 0) + exact;
        continue;
      }
      for (let k = 0; k < i; k++) {
        const before = best[j - 1][k].score;
        if (before === -Infinity) continue;
        let gain: number;
        if (k === i - 1) gain = R_CONSECUTIVE;
        else if (hump[i]) gain = R_HUMP;
        else if (humpsUpTo[i] === humpsUpTo[k]) gain = R_WITHIN_WORD - 0.1 * (i - k - 1);
        else continue;
        if (before + gain + exact > row[i].score) row[i] = { score: before + gain + exact, from: k };
      }
    }
    best.push(row);
  }
  const last = best[m - 1];
  let at = -1;
  for (let i = 0; i < n; i++) if (last[i].score > -Infinity && (at < 0 || last[i].score > last[at].score)) at = i;
  if (at < 0) return undefined;
  const indices: number[] = [];
  for (let j = m - 1, i = at; j >= 0; j--) {
    indices.unshift(i);
    i = best[j][i].from;
  }
  return { score: last[at].score, ranges: referenceRanges(indices) };
}

function referenceMatch(query: string, text: string): { score: number; ranges: [number, number][] } | undefined {
  if (!query) return { score: 0, ranges: [] };
  const humps = referenceHumps(query, text);
  const at = text.toLowerCase().indexOf(query.toLowerCase());
  const run = at < 0 ? undefined : { score: query.length * 0.5, ranges: [[at, at + query.length]] };
  const found = humps && run ? (humps.score >= run.score ? humps : run) : (humps ?? run);
  return found && { ...found, score: found.score - text.length * 0.01 };
}

const NAMES = [
  "ChangesetView",
  "release_candidate",
  "XMLParser",
  "paletteRows.ts",
  "src/features/palette/paletteRows.ts",
  "src/lib/speedSearch.ts",
  "libtest-mimic-0.8.1/Cargo.toml",
  "web-sys-0.3.100/src/features/gen_GpuCommandEncoderDescriptor.rs",
  "retry_policy",
  "AWS_S3_Bucket",
  "getHTTPResponse",
  "café_menu",
  "Straße",
  "İstanbul",
  "über_alles.rs",
  "ΩmegaCase",
  "中文字符",
  "müsMj",
  "a.b.c",
  "___x___",
];

/** A deterministic generator, so the same names and queries are tried every run. */
function referenceRng(seed: number): () => number {
  return () => {
    seed ^= seed << 13;
    seed >>>= 0;
    seed ^= seed >> 17;
    seed ^= seed << 5;
    seed >>>= 0;
    return seed / 0x100000000;
  };
}

void test("the fast matcher agrees with the original on random and real-looking names", () => {
  const rnd = referenceRng(0x2545f491);
  const alphabet = [
    ...Array.from("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-./ "),
    "ä",
    "ö",
    "ü",
    "é",
    "ñ",
    "ç",
    "ß",
    "İ",
    "α",
    "Ω",
    "Ж",
    "中",
    "\u{1f5c2}",
    "\u0301",
  ];
  const word = (max: number) => {
    const n = Math.floor(rnd() * max) + 1;
    let s = "";
    for (let i = 0; i < n; i++) s += alphabet[Math.floor(rnd() * alphabet.length)];
    return s;
  };
  const queryOf = (text: string) => {
    switch (Math.floor(rnd() * 4)) {
      case 0:
        return word(4);
      case 1:
        return text.slice(Math.floor(rnd() * Math.max(1, text.length)), Math.floor(rnd() * text.length) + 1);
      case 2:
        return Array.from(text)
          .filter(() => rnd() < 0.4)
          .join("")
          .slice(0, 6);
      default:
        return word(6);
    }
  };
  let checked = 0;
  const check = (query: string, text: string) => {
    assert.deepEqual(match(query, text), referenceMatch(query, text), JSON.stringify({ query, text }));
    checked++;
  };
  for (const text of NAMES) {
    check("", text);
    check(text, text);
    check(text.toLowerCase(), text);
    check(text.toUpperCase(), text);
    for (let i = 0; i < text.length; i++) check(text.slice(i, i + 3), text);
  }
  for (let i = 0; i < 40000; i++) {
    const text = word(14);
    check(queryOf(text), text);
  }
  assert.ok(checked > 40000);
});

void test("the fast matcher agrees with the original over a project's real paths", () => {
  const paths = [
    "crates/vet-backend/src/review/mod.rs",
    "crates/vet-app/src/notifications.rs",
    "ui/src/features/palette/paletteRows.ts",
    "ui/src/lib/speedSearch.svelte.ts",
    "ui/src/lib/ui/ChangeLetter.svelte",
    "notes/audit-findings.md",
    "ci/check-module-cycles.sh",
    "target/debug/deps/vet_backend-9a1b2c3d4e5f.db",
    "Cargo.lock",
    "ui/src/generated/SymbolKind.ts",
    "vendor/net-imap-0.11.0/src/client.rs",
    "a/b/c/d/e/f/g/h/i.rs",
  ];
  for (const text of paths) {
    for (const query of [
      "",
      "r",
      "rs",
      "mod",
      "vet",
      "backend",
      "bt",
      "vb",
      "palette",
      "PR",
      "cR",
      "zzz",
      "e.e",
      "db",
      "9a1",
      "ChangeLetter",
      "cv",
    ]) {
      assert.deepEqual(match(query, text), referenceMatch(query, text), JSON.stringify({ query, text }));
    }
  }
});
