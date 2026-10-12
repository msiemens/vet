import assert from "node:assert/strict";
import { test } from "node:test";
import type { Base } from "../generated/Base";
import type { Manifest } from "../generated/Manifest";
import { isChangesetView, withChangesetBase } from "./manifestScope.ts";

const named = (name: string) => ({ name }) as Base;
const manifest = (extra: Partial<Manifest> = {}) => ({ base: named("parent"), ...extra }) as Manifest;

void test("only a manifest without range and commit is the changeset's", () => {
  assert.equal(isChangesetView(manifest(), undefined), true);
  assert.equal(isChangesetView(manifest(), 0), false);
  assert.equal(isChangesetView(manifest({ interdiff: {} as Manifest["interdiff"] }), undefined), false);
});

void test("a moved base of the changeset replaces the base of the changeset's own view", () => {
  assert.equal(withChangesetBase(manifest(), named("main"), undefined).base.name, "main");
});

void test("a moved base of the changeset leaves a commit's and a range's base alone", () => {
  const commit = manifest();
  assert.equal(withChangesetBase(commit, named("main"), 1).base.name, "parent");
  const range = manifest({ interdiff: {} as Manifest["interdiff"] });
  assert.equal(withChangesetBase(range, named("main"), undefined).base.name, "parent");
});
