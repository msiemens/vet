import assert from "node:assert/strict";
import { test } from "node:test";
import { refreshTimes, showTime } from "./liveTime.ts";

const element = () => ({ dataset: {} as Record<string, string | undefined>, textContent: null as string | null });

void test("a time shown as 'now' reads as the time of day once the minute is over", () => {
  const sent = Date.UTC(2026, 9, 14, 12, 0, 0) / 1000;
  const shown = element();
  showTime(shown, sent, "detailed", sent * 1000 + 10_000);
  assert.equal(shown.textContent, "now");

  const other = element();
  refreshTimes({ querySelectorAll: () => [shown, other] }, sent * 1000 + 120_000);
  assert.notEqual(shown.textContent, "now");
  assert.equal(other.textContent, null, "an element that never showed a time is left alone");
});
