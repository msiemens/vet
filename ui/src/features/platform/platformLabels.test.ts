import assert from "node:assert/strict";
import { test } from "node:test";
import type { PlatformStatus } from "../../generated/PlatformStatus.ts";
import { connectionLabel } from "./platformLabels.ts";

const status = (connection: PlatformStatus["connection"]): PlatformStatus => ({
  host: "github.com",
  kind: "github",
  connection,
});

void test("only a platform that needs a new sign-in asks for attention", () => {
  assert.deepEqual(connectionLabel(status({ type: "ok" })).warn, false);
  assert.deepEqual(connectionLabel(status({ type: "failed", message: "offline", retryAt: null })).warn, false);
  assert.deepEqual(connectionLabel(status({ type: "signedOut" })).warn, true);
  const expired = connectionLabel(status({ type: "tokenExpired", message: "Bad credentials" }));
  assert.deepEqual([expired.text, expired.title, expired.warn], ["GitHub · token expired", "Bad credentials", true]);
});
