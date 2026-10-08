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
  expiresAt: null,
  ...over,
});

void test("a token's end is shown where the platform tells it, and pointed out a week ahead and after", () => {
  const day = 24 * 3600;
  const now = 1_800_000_000;
  const at = (expiresAt: number) => accountRow(account({ expiresAt }), true, now * 1000).expiry;
  assert.equal(accountRow(account({}), true, now * 1000).expiry, undefined);
  assert.deepEqual([at(now + 30 * day)?.label.startsWith("Expires on "), at(now + 30 * day)?.soon], [true, false]);
  assert.equal(at(now + 3 * day)?.soon, true);
  assert.deepEqual([at(now - day)?.label.startsWith("Expired on "), at(now - day)?.soon], [true, true]);
});

void test("a CLI credential names the CLI and is not Vet's to remove", () => {
  const row = accountRow(account({ source: "cli", kind: "azureDevOps", host: "dev.azure.com" }), true);
  assert.deepEqual([row.credential, row.removable, row.note], ["az CLI", false, undefined]);
});

void test("a GitHub Enterprise host shows GitHub's mark, Azure DevOps its own", () => {
  assert.equal(accountRow(account({ host: "git.contoso.com" }), true).platform.icon, "github");
  assert.equal(accountRow(account({ kind: "azureDevOps", host: "dev.azure.com" }), true).platform.icon, "azureDevOps");
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
