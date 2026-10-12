import assert from "node:assert/strict";
import { test } from "node:test";
import type { LocationRow } from "../generated/LocationRow.ts";
import type { LocationRows } from "../generated/LocationRows.ts";
import type { Unavailable } from "../generated/Unavailable.ts";
import { LocationSearch, type Search } from "./locationSearch.ts";
import type { PickerSink, PickerView } from "./pickerTypes.ts";

const row = (path: string, line: number): LocationRow => ({
  path,
  line,
  col: 1,
  within: null,
  text: "",
  highlights: [],
  matchStart: 0,
  matchEnd: 0,
  change: "unchanged",
  external: false,
});

const answer = (rows: LocationRow[], unavailable: Unavailable | null = null, total = rows.length): LocationRows => ({
  revision: 1,
  side: "target",
  rows,
  total,
  unavailable,
});

const stopped: Unavailable = {
  kind: "serverFailed",
  label: "Language server failed",
  message: "rust-analyzer stopped",
  language: "rust",
};

function setup(delayMs = 5, restart: () => Promise<string | undefined> = () => Promise.resolve(undefined)) {
  const events: string[] = [];
  let view: PickerView | undefined;
  const picker: PickerSink = {
    show: (v) => {
      view = v;
      events.push(`show ${v.body.phase}`);
    },
    close: () => {
      view = undefined;
      events.push("close");
    },
  };
  const search = new LocationSearch(
    {
      picker,
      notice: (text) => events.push(`notice ${text}`),
      open: (r, side) => events.push(`open ${r.path}:${r.line} ${side}`),
      isAbort: (e) => e instanceof DOMException && e.name === "AbortError",
      fail: (e, revision) => events.push(`fail ${String(e)} at ${revision}`),
      restartServer: async (language) => {
        events.push(`restart ${language}`);
        return restart();
      },
    },
    delayMs,
  );
  const ask = (reply: () => LocationRows | Promise<LocationRows>): Search => ({
    kind: "usages",
    symbol: "f",
    anchor: { left: 0, top: 0, bottom: 10 },
    side: "target",
    revision: 7,
    ask: () => Promise.resolve(reply()),
  });
  return { events, search, ask, view: () => view };
}

const settled = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
/** The events with a repeat of the one before left out: closing a popup twice is closing it. */
const collapsed = (events: string[]) => events.filter((event, i) => event !== events[i - 1]);

void test("one place is gone to without the popup showing", async () => {
  const { events, search, ask } = setup();
  await search.start(ask(() => answer([row("a.rs", 3)])));
  assert.deepEqual(collapsed(events), ["close", "open a.rs:3 target"]);
});

void test("no place shows the popup's empty state, and several its list", async () => {
  const none = setup();
  await none.search.start(none.ask(() => answer([])));
  assert.deepEqual(none.events, ["close", "show results"]);
  const view = none.view();
  assert.equal(view?.body.phase === "results" && view.body.rows.length, 0);

  const many = setup();
  await many.search.start(many.ask(() => answer([row("a.rs", 1), row("b.rs", 2)], null, 1340)));
  const shown = many.view()?.body;
  assert.equal(shown?.phase === "results" && shown.total, 1340);
});

void test("a search that takes a moment shows 'Searching…' until the answer replaces it", async () => {
  const { events, search, ask, view } = setup(5);
  await search.start(
    ask(async () => {
      await settled(30);
      return answer([row("a.rs", 1), row("a.rs", 2)]);
    }),
  );
  assert.deepEqual(events, ["close", "show searching", "show results"]);
  assert.equal(view()?.body.phase, "results");
});

void test("picking a place closes the popup and goes there on the side that was asked", async () => {
  const { events, search, ask, view } = setup();
  await search.start(ask(() => answer([row("a.rs", 1), row("a.rs", 2)])));
  events.length = 0;
  view()?.onpick(row("a.rs", 2));
  assert.deepEqual(events, ["close", "open a.rs:2 target"]);
});

void test("an answer that comes in after a newer search began is dropped", async () => {
  const { events, search, ask } = setup();
  const first = search.start(
    ask(async () => {
      await settled(20);
      return answer([row("old.rs", 1)]);
    }),
  );
  await search.start(ask(() => answer([row("new.rs", 5)])));
  await first;
  assert.deepEqual(
    events.filter((e) => e.startsWith("open")),
    ["open new.rs:5 target"],
  );
});

void test("a server that is down shows its state with a retry; any other reason is a notice", async () => {
  const down = setup();
  await down.search.start(down.ask(() => answer([], stopped)));
  const body = down.view()?.body;
  assert.equal(body?.phase === "stopped" && body.message, "rust-analyzer stopped");

  const other = setup();
  await other.search.start(
    other.ask(() =>
      answer([], { kind: "queryFailed", label: "x", message: "no implementation request", language: "python" }),
    ),
  );
  assert.deepEqual(other.events.slice(-2), ["close", "notice no implementation request"]);
});

void test("Restart in the popup restarts the server, and only once it runs asks again", async () => {
  const answers = [answer([], stopped), answer([row("a.rs", 3)])];
  const { events, search, ask, view } = setup(5, () => Promise.resolve(undefined));
  await search.start(ask(() => answers.shift() ?? answer([])));
  events.length = 0;
  view()?.onretry();
  await settled(20);
  assert.deepEqual(collapsed(events), ["show searching", "restart rust", "close", "open a.rs:3 target"]);
});

void test("a server that does not start again is told, and the search is not repeated", async () => {
  let asked = 0;
  const { events, search, ask, view } = setup(5, () => Promise.resolve("no such binary"));
  await search.start(
    ask(() => {
      asked++;
      return answer([], stopped);
    }),
  );
  events.length = 0;
  view()?.onretry();
  await settled(20);
  assert.deepEqual(events, [
    "show searching",
    "restart rust",
    "close",
    "notice Cannot restart the rust language server: no such binary",
  ]);
  assert.equal(asked, 1);
});

void test("closing the popup while the server restarts drops the search that would follow", async () => {
  let release: () => void = () => undefined;
  const { events, search, ask, view } = setup(5, () => new Promise((resolve) => (release = () => resolve(undefined))));
  await search.start(ask(() => answer([], stopped)));
  const popup = view();
  popup?.onretry();
  popup?.onclose();
  events.length = 0;
  release();
  await settled(20);
  assert.deepEqual(events, []);
});

void test("the changeset moving on says so, and any other failure is the backend's to report", async () => {
  const moved = setup();
  await moved.search.start(moved.ask(() => Promise.reject(new DOMException("x", "AbortError"))));
  assert.equal(moved.events.at(-1), "notice Changes arrived — try again");
  const broke = setup();
  await broke.search.start(broke.ask(() => Promise.reject(new Error("boom"))));
  assert.equal(broke.events.at(-1), "fail Error: boom at 7");
});
