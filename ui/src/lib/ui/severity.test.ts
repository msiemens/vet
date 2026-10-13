import assert from "node:assert/strict";
import { test } from "node:test";
import { ICONS, iconSvg } from "./icons.ts";
import { severityColor, severityIcon, severitySvg } from "./severity.ts";

void test("a hint is drawn as info, in the colour of info", () => {
  assert.equal(severityIcon("hint"), "info");
  assert.equal(severityColor("hint"), "var(--severity-info)");
  assert.equal(severitySvg("hint"), severitySvg("info"));
});

void test("each severity has its own icon and its own colour", () => {
  const svgs = (["error", "warning", "info"] as const).map((s) => severitySvg(s));
  assert.equal(new Set(svgs).size, 3);
  assert.match(severitySvg("error"), /color:var\(--severity-error\)/);
  assert.match(severitySvg("warning"), /color:var\(--severity-warning\)/);
  assert.ok(severitySvg("warning").includes(ICONS.warning));
});

void test("icons from the 16 px grid are drawn from their inner 14 px, the others on the 24 px grid", () => {
  assert.match(iconSvg("error", 14), /viewBox="1 1 14 14"/);
  assert.match(iconSvg("problems", 14), /viewBox="1 1 14 14"/);
  assert.match(iconSvg("search", 14), /viewBox="0 0 24 24"/);
});

void test("the severity icons are filled shapes and the Problems icon is an outline", () => {
  assert.match(iconSvg("error"), /fill="currentColor" stroke="none"/);
  assert.match(iconSvg("problems"), /fill="none" stroke="currentColor"/);
});
