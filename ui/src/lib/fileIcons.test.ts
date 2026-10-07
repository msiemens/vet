import assert from "node:assert/strict";
import { test } from "node:test";
import { fileIcon, folderIcon } from "./fileIcons.ts";

void test("well-known names beat their extension", () => {
  assert.equal(fileIcon("Cargo.toml"), "rust");
  assert.equal(fileIcon("Cargo.lock"), "lock");
  assert.equal(fileIcon("package.json"), "nodejs");
  assert.equal(fileIcon("tsconfig.app.json"), "tsconfig");
  assert.equal(fileIcon("Dockerfile"), "docker");
  assert.equal(fileIcon(".gitignore"), "git");
  assert.equal(fileIcon("Makefile"), "makefile");
});

void test("names are matched whatever their case", () => {
  assert.equal(fileIcon("DOCKERFILE"), "docker");
  assert.equal(fileIcon("Main.RS"), "rust");
});

void test("other files get their extension's icon, and unknown ones the plain file", () => {
  assert.equal(fileIcon("main.rs"), "rust");
  assert.equal(fileIcon("App.svelte"), "svelte");
  assert.equal(fileIcon("logo.png"), "image");
  assert.equal(fileIcon("data.xyz"), "file");
  assert.equal(fileIcon("noextension"), "file");
  assert.equal(fileIcon(".hidden"), "file");
});

void test("a compound extension wins over its last part", () => {
  assert.equal(fileIcon("index.d.ts"), "typescript-def");
  assert.equal(fileIcon("parse.test.ts"), "test-ts");
  assert.equal(fileIcon("parse.ts"), "typescript");
  assert.equal(fileIcon("notes.v2.md"), "markdown");
});

void test("README, LICENSE and .env variants share an icon, but only as whole words", () => {
  assert.equal(fileIcon("README.md"), "readme");
  assert.equal(fileIcon("LICENSE-MIT"), "license");
  assert.equal(fileIcon(".env.local"), "tune");
  assert.equal(fileIcon("Dockerfile.dev"), "docker");
  assert.equal(fileIcon("readmeUtils.ts"), "typescript");
});

void test("a folder has an icon for open and closed, special for well-known names", () => {
  assert.equal(folderIcon("src", false), "folder-src");
  assert.equal(folderIcon("src", true), "folder-src-open");
  assert.equal(folderIcon("node_modules", false), "folder-node");
  assert.equal(folderIcon(".github", true), "folder-github-open");
  assert.equal(folderIcon("whatever", false), "folder");
  assert.equal(folderIcon("whatever", true), "folder-open");
});

void test("a chain of folders takes the icon of its last folder", () => {
  assert.equal(folderIcon("crates/vet-core/src", false), "folder-src");
  assert.equal(folderIcon("src/lib/Tests", true), "folder-test-open");
});
