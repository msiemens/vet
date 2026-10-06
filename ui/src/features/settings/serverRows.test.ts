import assert from "node:assert/strict";
import { test } from "node:test";
import type { ServerInfo } from "../../generated/ServerInfo.ts";
import { appendLog, serverRow } from "./serverRows.ts";

const found = (origin: "custom" | "project" | "installed" | "path"): ServerInfo => ({
  language: "rust",
  server: "rust-analyzer",
  state: { type: "found", path: "/bin/ra", origin, version: "1.2", notes: ["note"] },
  customPath: origin === "custom" ? "/bin/ra" : null,
  pinnedVersion: "2026-01-01",
  installBlocker: null,
});

const missing: ServerInfo = {
  language: "python",
  server: "Pyright",
  state: { type: "missing", reason: "pyright-langserver is not on PATH" },
  customPath: null,
  pinnedVersion: "1.1.400",
  installBlocker: null,
};

void test("a found server shows where it came from", () => {
  const row = serverRow(found("installed"));
  assert.equal(row.status, "found");
  assert.equal(row.origin, "Installed by Vet");
  assert.equal(row.path, "/bin/ra");
  assert.equal(row.version, "1.2");
  assert.deepEqual(row.notes, ["note"]);
});

void test("a custom origin is the custom status and keeps the path to reset", () => {
  const row = serverRow(found("custom"));
  assert.equal(row.status, "custom");
  assert.equal(row.customPath, "/bin/ra");
});

void test("a missing server carries the reason and can be installed when pinned", () => {
  const row = serverRow(missing);
  assert.equal(row.status, "missing");
  assert.equal(row.problem, "pyright-langserver is not on PATH");
  assert.equal(row.title, "Python");
  assert.ok(row.canInstall);
});

void test("an install blocker or a missing pin turns Install off", () => {
  assert.equal(serverRow({ ...missing, installBlocker: "npm is missing" }).canInstall, false);
  assert.equal(serverRow({ ...missing, installBlocker: "npm is missing" }).installBlocker, "npm is missing");
  assert.equal(serverRow({ ...missing, pinnedVersion: null }).canInstall, false);
});

void test("install progress overrides the resolved state", () => {
  assert.equal(serverRow(missing, { running: true }).status, "installing");
  assert.equal(serverRow(missing, { running: true }).canInstall, false);
  const failed = serverRow(missing, { running: false, error: "exit 1" });
  assert.equal(failed.status, "failed");
  assert.equal(failed.problem, "exit 1");
  assert.ok(failed.canInstall);
});

void test("an unknown language falls back to its id", () => {
  const row = serverRow({ ...missing, language: "zig" });
  assert.equal(row.title, "zig");
  assert.equal(row.server, "Pyright");
});

void test("the install log keeps the newest lines", () => {
  let lines: string[] = [];
  for (let i = 0; i < 5; i++) lines = appendLog(lines, `l${i}`, 3);
  assert.deepEqual(lines, ["l2", "l3", "l4"]);
});
