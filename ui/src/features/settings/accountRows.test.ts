import assert from "node:assert/strict";
import { test } from "node:test";
import type { Account } from "../../generated/Account.ts";
import { accountRow, usesCli } from "./accountRows.ts";

const account = (over: Partial<Account>): Account => ({
  host: "github.com",
  kind: "github",
  source: "pat",
  user: "octocat",
  state: { type: "signedIn" },
  inUse: true,
  ...over,
});

void test("a CLI credential names the CLI and says it is never stored", () => {
  const row = accountRow(account({ source: "cli", kind: "azureDevOps", host: "dev.azure.com" }));
  assert.equal(row.source, "Azure CLI (az)");
  assert.match(row.note ?? "", /never stores it/);
  assert.equal(row.removable, false);
});

void test("only a personal access token can be removed, and a CLI in front of it is said", () => {
  const row = accountRow(account({ inUse: false }));
  assert.equal(row.removable, true);
  assert.match(row.note ?? "", /Not used/);
});

void test("a refused or unchecked credential carries the platform's reason", () => {
  const rejected = accountRow(account({ user: null, state: { type: "rejected", message: "Bad credentials" } }));
  assert.deepEqual([rejected.status, rejected.problem, rejected.user], ["rejected", "Bad credentials", undefined]);
  const unverified = accountRow(account({ state: { type: "unverified", message: "no route to host" } }));
  assert.deepEqual([unverified.statusLabel, unverified.problem], ["Not verified", "no route to host"]);
});

void test("a CLI is used only when it supplies a host's credential", () => {
  assert.equal(usesCli([account({})]), false);
  assert.equal(usesCli([account({ source: "cli", inUse: false })]), false);
  assert.equal(usesCli([account({}), account({ source: "cli" })]), true);
});
