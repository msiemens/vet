import assert from "node:assert/strict";
import { test } from "node:test";
import { REFRESH_MS, refreshDelay } from "./checkRefresh.ts";

void test("running checks are asked about every 30 s, and not before a limit on the platform ends", () => {
  const now = 1_700_000_000_000;
  assert.equal(refreshDelay(undefined, now), REFRESH_MS);
  assert.equal(refreshDelay(now / 1000 + 5, now), REFRESH_MS, "a limit that ends sooner changes nothing");
  assert.equal(refreshDelay(now / 1000 + 120, now), 120_000);
  assert.equal(refreshDelay(now / 1000 - 60, now), REFRESH_MS, "a limit that has ended");
});
