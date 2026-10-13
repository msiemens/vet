import assert from "node:assert/strict";
import { test } from "node:test";
import type { ServerInfo } from "../../generated/ServerInfo.ts";
import { serverRow } from "./serverRows.ts";

const found = (origin: "custom" | "project" | "installed" | "path"): ServerInfo => ({
  language: "rust",
  title: "Rust",
  server: "rust-analyzer",
  state: { type: "found", path: "/bin/ra", origin, version: "1.2", tsserver: null },
  customPath: origin === "custom" ? "/bin/ra" : null,
  pinnedVersion: "2026-01-01",
  installBlocker: null,
  canInstall: true,
  features: null,
});

const missing: ServerInfo = {
  language: "python",
  title: "Python",
  server: "Pyright",
  state: { type: "missing", reason: "pyright-langserver is not on PATH" },
  customPath: null,
  pinnedVersion: "1.1.400",
  installBlocker: null,
  canInstall: true,
  features: null,
};

void test("a found server is found unless its origin is custom, which is its own status and keeps the path to reset", () => {
  assert.equal(serverRow(found("installed")).status, "found");
  const row = serverRow(found("custom"));
  assert.equal(row.status, "custom");
  assert.equal(row.customPath, "/bin/ra");
});

void test("the table names a custom source by its path and a missing server's by nothing", () => {
  assert.equal(serverRow(found("custom")).source, "/bin/ra");
  assert.equal(serverRow(missing).source, "");
});

void test("a missing server carries the reason and the backend's title and can be installed when the backend says so", () => {
  const row = serverRow(missing);
  assert.equal(row.status, "missing");
  assert.equal(row.problem, "pyright-langserver is not on PATH");
  assert.equal(row.title, "Python");
  assert.ok(row.canInstall);
});

void test("install progress overrides the resolved state", () => {
  assert.equal(serverRow(missing, { running: true }).status, "installing");
  assert.equal(serverRow(missing, { running: true }).canInstall, false);
  const failed = serverRow(missing, { running: false, error: "exit 1" });
  assert.equal(failed.status, "failed");
  assert.equal(failed.problem, "exit 1");
  assert.ok(failed.canInstall);
});
