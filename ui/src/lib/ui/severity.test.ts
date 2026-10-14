import assert from "node:assert/strict";
import { test } from "node:test";
import { severityColor, severityIcon, severitySvg } from "./severity.ts";

void test("a hint is drawn as info, in the colour of info", () => {
  assert.equal(severityIcon("hint"), "info");
  assert.equal(severityColor("hint"), "var(--severity-info)");
  assert.equal(severitySvg("hint"), severitySvg("info"));
});

void test("each severity has its own icon and its own colour", () => {
  const severities = ["error", "warning", "info"] as const;
  assert.equal(new Set(severities.map(severityIcon)).size, 3);
  assert.equal(new Set(severities.map(severityColor)).size, 3);
});
