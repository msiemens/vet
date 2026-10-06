import assert from "node:assert/strict";
import { test } from "node:test";
import { formatCommand, parseCommand, sourceOf } from "./command.ts";

void test("a plain command round-trips", () => {
  const line = formatCommand("cargo", ["run", "--bin", "app"]);
  assert.equal(line, "cargo run --bin app");
  assert.deepEqual(parseCommand(line), { program: "cargo", args: ["run", "--bin", "app"] });
});

void test("words with spaces or quotes are quoted and survive", () => {
  const args = ["a b", 'say "hi"', "", "back\\slash", "it's"];
  assert.deepEqual(parseCommand(formatCommand("p", args)), { program: "p", args });
});

void test("single quotes group literally, runs of blanks separate once", () => {
  assert.deepEqual(parseCommand("  echo   'a  b'\t\"c d\"x "), { program: "echo", args: ["a  b", "c dx"] });
});

void test("an empty line has no program", () => {
  assert.deepEqual(parseCommand("  "), { program: "", args: [] });
});

void test("the source names the manifest the id came from", () => {
  assert.equal(sourceOf("cargo::app"), "Cargo.toml");
  assert.equal(sourceOf("script:ui:test:unit"), "ui/package.json");
  assert.equal(sourceOf("python::serve"), "pyproject.toml");
  assert.equal(sourceOf("custom:1"), "custom");
});
