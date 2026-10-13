import assert from "node:assert/strict";
import { test } from "node:test";
import type { Diagnostic } from "../generated/Diagnostic.ts";
import type { Unavailable } from "../generated/Unavailable.ts";
import { type Answered, type BookHost, DiagnosticsBook, type FileDiagnostics } from "./diagnosticsBook.ts";

const diagnostic = (message: string, origin: Diagnostic["origin"] = "pending"): Diagnostic =>
  ({ message, origin }) as Diagnostic;

const down: Unavailable = { kind: "serverFailed", label: "failed", message: "it crashed", language: "rust" };

interface Call {
  path: string;
  againstBase: boolean;
  signal: AbortSignal;
  answer: (r: Answered) => void;
  fail: (e: Error) => void;
}

/** A host whose answers are given by the test, one call at a time. */
function setup(options: { busy?: () => boolean; idle?: () => Promise<void> } = {}) {
  const calls: Call[] = [];
  const hold: ReadonlyMap<string, FileDiagnostics>[] = [];
  let drops = 0;
  const host: BookHost = {
    ask: (_revision, path, againstBase, signal) =>
      new Promise((resolve, reject) => calls.push({ path, againstBase, signal, answer: resolve, fail: reject })),
    busy: options.busy ?? (() => false),
    untilIdle: options.idle ?? (() => Promise.resolve()),
    drop: () => drops++,
    hold: (held) => hold.push(held),
  };
  const files = new Map<string, FileDiagnostics>();
  const missing = new Map<string, Unavailable>();
  const book = new DiagnosticsBook(host, files, missing);
  const announced: string[] = [];
  book.onOrigins((p) => announced.push(p));
  return { book, calls, files, missing, announced, hold, drops: () => drops };
}

const settle = () => new Promise<void>((r) => setTimeout(r, 0));

void test("the target's diagnostics come at once with their origin pending, the comparison replaces them", async () => {
  const { book, calls, files, announced } = setup();
  const first = book.diagnostics(1, "a.rs");
  calls[0].answer({ diagnostics: [diagnostic("x")] });
  assert.deepEqual(
    (await first).map((d) => d.message),
    ["x"],
  );
  assert.equal(calls[0].againstBase, false);
  assert.equal(files.get("a.rs")?.compared, false);
  await settle();
  assert.equal(calls[1].againstBase, true);
  calls[1].answer({ diagnostics: [diagnostic("x", "existing")] });
  await settle();
  assert.equal(files.get("a.rs")?.compared, true);
  assert.equal(files.get("a.rs")?.diagnostics[0].origin, "existing");
  assert.deepEqual(announced, ["a.rs"]);
});

void test("a file is asked about once per revision", async () => {
  const { book, calls } = setup();
  const a = book.diagnostics(1, "a.rs");
  const b = book.diagnostics(1, "a.rs");
  calls[0].answer({ diagnostics: [] });
  await Promise.all([a, b]);
  assert.equal(calls.filter((c) => !c.againstBase).length, 1);
});

void test("a comparison that fails ends with the origins unknown, not pending", async () => {
  const { book, calls, files, announced } = setup();
  const first = book.diagnostics(1, "a.rs");
  calls[0].answer({ diagnostics: [diagnostic("x")] });
  await first;
  await settle();
  calls[1].fail(new Error("socket closed"));
  await settle();
  const file = files.get("a.rs");
  assert.equal(file?.compared, true);
  assert.equal(file.originUnavailable?.message, "socket closed");
  assert.equal(file.diagnostics[0].origin, "unknown");
  assert.deepEqual(announced, ["a.rs"]);
});

void test("a comparison the server cannot answer ends with the origins unknown for its reason", async () => {
  const { book, calls, files } = setup();
  const first = book.diagnostics(1, "a.rs");
  calls[0].answer({ diagnostics: [diagnostic("x")] });
  await first;
  await settle();
  calls[1].answer({ diagnostics: [], unavailable: down });
  await settle();
  assert.equal(files.get("a.rs")?.compared, true);
  assert.equal(files.get("a.rs")?.originUnavailable, down);
});

void test("a file the server cannot answer for is listed as missing and not compared", async () => {
  const { book, calls, missing, files } = setup();
  const first = book.diagnostics(1, "a.rs");
  calls[0].answer({ diagnostics: [], unavailable: down });
  assert.deepEqual(await first, []);
  await settle();
  assert.equal(calls.length, 1);
  assert.equal(missing.get("a.rs"), down);
  assert.equal(files.has("a.rs"), false);
});

void test("dropping the answers aborts what is underway and ignores what still arrives", async () => {
  const { book, calls, files, announced } = setup();
  const first = book.diagnostics(1, "a.rs");
  calls[0].answer({ diagnostics: [diagnostic("x")] });
  await first;
  await settle();
  const comparison = calls[1];
  book.invalidate();
  assert.equal(comparison.signal.aborted, true);
  comparison.answer({ diagnostics: [diagnostic("x", "existing")] });
  await settle();
  assert.equal(files.size, 0);
  assert.deepEqual(announced, []);
});

void test("a failure of a comparison that was dropped is not taken for the new answers' failure", async () => {
  const { book, calls, files } = setup();
  const first = book.diagnostics(1, "a.rs");
  calls[0].answer({ diagnostics: [diagnostic("x")] });
  await first;
  await settle();
  const comparison = calls[1];
  book.invalidate();
  const again = book.diagnostics(1, "a.rs");
  calls[2].answer({ diagnostics: [diagnostic("y")] });
  await again;
  comparison.fail(new Error("aborted"));
  await settle();
  assert.equal(files.get("a.rs")?.compared, false);
  assert.equal(files.get("a.rs")?.diagnostics[0].origin, "pending");
});

void test("the last revision's answers are held until the new one answers, and dropping them leaves nothing held", async () => {
  const { book, calls, hold } = setup();
  const first = book.diagnostics(1, "a.rs");
  calls[0].answer({ diagnostics: [diagnostic("old")] });
  await first;
  void book.diagnostics(2, "a.rs");
  assert.equal(hold.at(-1)?.get("a.rs")?.diagnostics[0].message, "old");
  book.invalidate();
  assert.equal(hold.at(-1)?.size, 0);
});

void test("a request of an older revision is refused", async () => {
  const { book, calls } = setup();
  void book.diagnostics(2, "a.rs");
  assert.deepEqual(await book.diagnostics(1, "b.rs"), []);
  assert.equal(calls.length, 1);
});

void test("the comparison waits for the server to finish its work", async () => {
  const releases: (() => void)[] = [];
  const idle = new Promise<void>((resolve) => releases.push(resolve));
  const { book, calls } = setup({ idle: () => idle });
  const first = book.diagnostics(1, "a.rs");
  calls[0].answer({ diagnostics: [] });
  await first;
  await settle();
  assert.equal(calls.length, 1);
  releases[0]();
  await settle();
  assert.equal(calls[1].againstBase, true);
});

void test("an answer taken while the server went to work is asked for again, and never trusted when it keeps working", async () => {
  let busy = false;
  const { book, calls, files } = setup({ busy: () => busy });
  const first = book.diagnostics(1, "a.rs");
  calls[0].answer({ diagnostics: [diagnostic("x")] });
  await first;
  await settle();
  busy = true;
  calls[1].answer({ diagnostics: [] });
  await settle();
  assert.equal(calls.length, 3, "asked again");
  calls[2].answer({ diagnostics: [] });
  await settle();
  assert.equal(calls.length, 4);
  calls[3].answer({ diagnostics: [] });
  await settle();
  assert.equal(files.get("a.rs")?.compared, true);
  assert.ok(files.get("a.rs")?.originUnavailable, "still unknown");
  assert.equal(files.get("a.rs")?.diagnostics[0].origin, "unknown");
});
