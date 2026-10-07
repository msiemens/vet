import assert from "node:assert/strict";
import { test } from "node:test";
import type { Account } from "../../generated/Account.ts";
import { accountRow } from "./accountRows.ts";

const account = (over: Partial<Account>): Account => ({
  host: "github.com",
  kind: "github",
  source: "pat",
  user: "octocat",
  state: { type: "signedIn" },
  inUse: true,
  ...over,
});

void test("a CLI credential names the CLI and is not Vet's to remove", () => {
  const row = accountRow(account({ source: "cli", kind: "azureDevOps", host: "dev.azure.com" }), true);
  assert.deepEqual([row.credential, row.platform, row.removable, row.note], ["az CLI", "ADO", false, undefined]);
});

void test("a GitHub host other than github.com is an Enterprise host", () => {
  assert.equal(accountRow(account({}), true).platform, "GH");
  assert.equal(accountRow(account({ host: "git.contoso.com" }), true).platform, "GHE");
});

void test("a token is in the keychain only where the host has one, and a CLI in front of it is said", () => {
  const row = accountRow(account({ inUse: false }), true);
  assert.deepEqual([row.storage, row.removable], ["Token in Keychain", true]);
  assert.match(row.note ?? "", /Not used/);
  assert.match(accountRow(account({}), false).storage, /until Vet quits/);
});

void test("a refused or unchecked credential carries the platform's reason", () => {
  const rejected = accountRow(account({ user: null, state: { type: "rejected", message: "Bad credentials" } }), true);
  assert.deepEqual([rejected.status, rejected.problem, rejected.user], ["rejected", "Bad credentials", undefined]);
  const unverified = accountRow(account({ state: { type: "unverified", message: "no route to host" } }), true);
  assert.deepEqual([unverified.statusLabel, unverified.problem], ["Not verified", "no route to host"]);
});
