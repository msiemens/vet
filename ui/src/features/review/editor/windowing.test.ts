import assert from "node:assert/strict";
import { test } from "node:test";
import type { Section } from "./section.ts";
import type { SectionHeights } from "./sectionHeights.ts";
import { Windowing, windowRanges } from "./windowing.ts";

// Ten sections of 100px each.
const indexAt = (y: number) => Math.min(9, Math.max(0, Math.floor(y / 100)));

void test("once scrolling has stopped, a screen above and two below get editors", () => {
  // A 200px viewport at 500px: from 300px up to 900px.
  assert.deepEqual(windowRanges(indexAt, 500, 200, true).near, [3, 9]);
});

void test("while scrolling, only the sections in view get editors", () => {
  assert.deepEqual(windowRanges(indexAt, 500, 200, false).near, [5, 7]);
});

void test("editors outside two screens above and three below are dropped", () => {
  assert.deepEqual(windowRanges(indexAt, 500, 200, true).far, [1, 9]);
  assert.deepEqual(windowRanges(indexAt, 500, 100, false).far, [3, 8]);
});

void test("near the top the range starts at the first section", () => {
  const { near, far } = windowRanges(indexAt, 50, 200, true);
  assert.deepEqual([near[0], far[0]], [0, 0]);
});

/** A column of ten loaded 100px sections scrolled to 500px, and the frames a test steps through. */
function column(msPerMount: number, viewport = 200) {
  const sections = Array.from({ length: 10 }, (_, i) => ({
    top: i * 100,
    height: 100,
    cards: [],
    loaded: {},
    handle: undefined as { views: never[] } | undefined,
    file: { hunks: [1], baseLines: 1, targetLines: 1 },
  }));
  const frames: (() => void)[] = [];
  let clock = 0;
  const global = globalThis as unknown as {
    requestAnimationFrame: (run: () => void) => number;
    cancelAnimationFrame: (id: number) => void;
  };
  const real = {
    frame: global.requestAnimationFrame,
    cancel: global.cancelAnimationFrame,
    now: performance.now.bind(performance),
  };
  global.requestAnimationFrame = (run) => frames.push(run);
  global.cancelAnimationFrame = () => void (frames.length = 0);
  performance.now = () => clock;
  let scrolled: ((e: Event) => void) | undefined;
  const root = {
    scrollTop: 500,
    clientHeight: viewport,
    addEventListener: (_type: string, listener: (e: Event) => void) => (scrolled = listener),
    removeEventListener: () => undefined,
  };
  const windowing = new Windowing(root, sections as unknown as Section[], { indexAt } as unknown as SectionHeights, {
    mount: (fi) => {
      sections[fi].handle = { views: [] };
      windowing.mounted.add(fi);
      clock += msPerMount;
    },
    unmount: () => 0,
  });
  windowing.start();
  return {
    /**
     * A frame as the browser runs it: the scroll event if there is one, then the frame requests made
     * so far. Answers how many editors the frame mounted.
     */
    frame(scrolls: boolean) {
      const before = windowing.mounted.size;
      if (scrolls) scrolled?.(new Event("scroll"));
      frames.splice(0).forEach((run) => run());
      return windowing.mounted.size - before;
    },
    restore() {
      windowing.destroy();
      global.requestAnimationFrame = real.frame;
      global.cancelAnimationFrame = real.cancel;
      performance.now = real.now;
    },
  };
}

void test("a frame mounts editors until its time budget is spent, the rest follow in the next frames", () => {
  const view = column(4);
  try {
    // Sections 5 to 7 are in view, 4 ms each: the third is over the 6 ms budget.
    assert.deepEqual([view.frame(true), view.frame(false), view.frame(false)], [2, 1, 0]);
  } finally {
    view.restore();
  }
});

void test("a frame whose scroll event already mounted does not mount again from a continuation", () => {
  const view = column(4, 400); // sections 5 to 9 are in view
  try {
    assert.equal(view.frame(true), 2);
    // The continuation of that pass is due in the same frame as the next scroll event's pass.
    assert.equal(view.frame(true), 2);
    assert.equal(view.frame(false), 1);
  } finally {
    view.restore();
  }
});
