import assert from "node:assert/strict";
import { test } from "node:test";
import { MIN_MARK, MIN_THUMB, markRects, scrollForThumb, thumbRect } from "./scrollbar.ts";

void test("marks are scaled to the track and keep to it", () => {
  const rects = markRects(
    {
      total: 1000,
      marks: [
        { top: 0, height: 100, kind: "seen" },
        { top: 500, height: 200, kind: "unseen" },
        { top: 990, height: 100, kind: "deletion" },
      ],
    },
    100,
  );
  assert.deepEqual(rects, [
    { top: 0, height: 10, kind: "seen" },
    { top: 50, height: 20, kind: "unseen" },
    { top: 90, height: 10, kind: "deletion" },
  ]);
});

void test("a mark too small for the track still shows", () => {
  const [rect] = markRects({ total: 100_000, marks: [{ top: 50_000, height: 20, kind: "unseen" }] }, 500);
  assert.equal(rect.height, MIN_MARK);
  assert.equal(rect.top, 250);
});

void test("no marks are drawn without content or a track", () => {
  const marks = [{ top: 0, height: 10, kind: "seen" }];
  assert.deepEqual(markRects({ total: 0, marks }, 100), []);
  assert.deepEqual(markRects({ total: 100, marks }, 0), []);
});

void test("the thumb is the part in view, and stays on the track at both ends", () => {
  assert.deepEqual(thumbRect(1000, 100, 0, 200), { top: 0, height: 20 });
  assert.deepEqual(thumbRect(1000, 100, 450, 200), { top: 90, height: 20 });
  assert.deepEqual(thumbRect(1000, 100, 900, 200), { top: 180, height: 20 });
  assert.deepEqual(thumbRect(1000, 100, -40, 200), { top: 0, height: 20 });
  assert.deepEqual(thumbRect(1000, 100, 2000, 200), { top: 180, height: 20 });
});

void test("there is no thumb when everything is in view", () => {
  assert.equal(thumbRect(100, 100, 0, 200).height, 0);
  assert.equal(thumbRect(80, 100, 0, 200).height, 0);
});

void test("the thumb of long content keeps a size that can be grabbed, and still reaches both ends", () => {
  const top = thumbRect(1_000_000, 100, 0, 200);
  const bottom = thumbRect(1_000_000, 100, 1_000_000 - 100, 200);
  assert.equal(top.height, MIN_THUMB);
  assert.equal(top.top, 0);
  assert.equal(bottom.top + bottom.height, 200);
});

void test("dragging the thumb scrolls by where it is put, within the content", () => {
  const { height } = thumbRect(1000, 100, 0, 200);
  for (const scroll of [0, 300, 900]) {
    const { top } = thumbRect(1000, 100, scroll, 200);
    assert.equal(scrollForThumb(1000, 100, 200, height, top), scroll);
  }
  assert.equal(scrollForThumb(1000, 100, 200, height, -50), 0);
  assert.equal(scrollForThumb(1000, 100, 200, height, 500), 900);
});
