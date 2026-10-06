import assert from "node:assert/strict";
import { test } from "node:test";
import { MAX_CHARS, MAX_PIECES, RunLog } from "./runLog.ts";

const b64 = (bytes: number[] | string) =>
  Buffer.from(typeof bytes === "string" ? bytes : Uint8Array.from(bytes)).toString("base64");

void test("a character split between chunks decodes whole", () => {
  const log = new RunLog();
  const euro = [0xe2, 0x82, 0xac];
  log.push("stdout", b64(euro.slice(0, 2)));
  log.push("stdout", b64(euro.slice(2)));
  assert.equal(
    log
      .since(0)
      .map((p) => p.text)
      .join(""),
    "€",
  );
});

void test("invalid bytes become replacement characters and an unfinished one is flushed", () => {
  const log = new RunLog();
  log.push("stderr", b64([0xff, 0x41, 0xe2]));
  log.finish();
  assert.equal(
    log
      .since(0)
      .map((p) => p.text)
      .join(""),
    "�A�",
  );
});

void test("streams keep their own decoder and label", () => {
  const log = new RunLog();
  log.push("stdout", b64([0xe2]));
  log.push("stderr", b64("e"));
  assert.deepEqual(
    log.since(0).map((p) => [p.stream, p.text]),
    [["stderr", "e"]],
  );
});

void test("since skips what was already read", () => {
  const log = new RunLog();
  log.push("stdout", b64("a"));
  const seen = log.end;
  log.push("stdout", b64("b"));
  assert.deepEqual(
    log.since(seen).map((p) => p.text),
    ["b"],
  );
});

void test("the oldest pieces go when the scrollback is full", () => {
  const log = new RunLog();
  const chunk = "x".repeat(MAX_CHARS / 2);
  for (let i = 0; i < 3; i++) log.push("stdout", b64(chunk));
  assert.equal(log.trimmed, true);
  assert.deepEqual(
    log.since(0).map((p) => p.seq),
    [1, 2],
  );
});

void test("many tiny pieces are bounded too", () => {
  const log = new RunLog();
  for (let i = 0; i < MAX_PIECES + 5; i++) log.push("stdout", b64("."));
  const kept = log.since(0);
  assert.equal(kept.length, MAX_PIECES);
  assert.equal(kept[0].seq, 5);
});

void test("onchange fires per push with output", () => {
  const log = new RunLog();
  let calls = 0;
  log.onchange = () => calls++;
  log.push("stdout", b64("a"));
  log.finish();
  assert.equal(calls, 1);
});
