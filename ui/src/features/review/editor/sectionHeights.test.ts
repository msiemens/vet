import assert from "node:assert/strict";
import { test } from "node:test";
import type { Section } from "./section.ts";
import { SectionHeights, sectionAt } from "./sectionHeights.ts";
import type { ThreadLayer } from "./threadLayer.ts";

const metrics = { line: 20, hunk: 24, header: 26, collapsed: 27, gap: 8, preview: 100 };

/** Three sections of heights 100, 200 and 50, stacked from the top. */
function column(scrollTop: number) {
  const sections = [100, 200, 50].map((height, i) => ({
    top: [0, 100, 300][i],
    height,
    el: { style: { height: "" } },
  }));
  const root = { scrollTop };
  const heights = new SectionHeights(
    root,
    sections as unknown as Section[],
    metrics,
    {} as ThreadLayer,
    "unified",
    true,
  );
  return { root, sections, heights };
}

void test("the section at a height is the last one starting at or above it", () => {
  const tops = [{ top: 0 }, { top: 100 }, { top: 300 }];
  assert.deepEqual(
    [0, 99, 100, 299, 300, 9999].map((y) => sectionAt(tops, y)),
    [0, 0, 1, 1, 2, 2],
  );
  assert.equal(sectionAt([], 10), 0);
});

void test("a section that grows pushes the ones below it down", () => {
  const { sections, heights } = column(0);
  heights.resize(sections[0] as unknown as Section, 130);
  assert.deepEqual(
    sections.map((s) => s.top),
    [0, 130, 330],
  );
  assert.equal(sections[0].el.style.height, "130px");
});

void test("a change above the viewport moves the scroll position with it, so the view stays put", () => {
  const { root, sections, heights } = column(320);
  const moved = heights.resize(sections[0] as unknown as Section, 130);
  assert.equal(moved, 30);
  assert.equal(root.scrollTop, 350);
});

void test("a change in or below the viewport leaves the scroll position alone", () => {
  const { root, sections, heights } = column(150);
  assert.equal(heights.resize(sections[1] as unknown as Section, 260), 0);
  assert.equal(heights.resize(sections[2] as unknown as Section, 80), 0);
  assert.equal(root.scrollTop, 150);
});

void test("a change under half a pixel is ignored", () => {
  const { sections, heights } = column(0);
  assert.equal(heights.resize(sections[0] as unknown as Section, 100.4), 0);
  assert.equal(sections[0].height, 100);
});
