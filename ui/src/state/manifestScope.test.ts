import assert from "node:assert/strict";
import { test } from "node:test";
import type { Base } from "../generated/Base";
import type { Manifest } from "../generated/Manifest";
import { changesetManifest, isChangesetView, sameManifestView, withChangesetBase } from "./manifestScope.ts";

const named = (name: string) => ({ name }) as Base;
const manifest = (extra: Partial<Manifest> = {}) => ({ base: named("parent"), ...extra }) as Manifest;

void test("only a manifest without range and commit is the changeset's", () => {
  assert.equal(isChangesetView(manifest(), undefined), true);
  assert.equal(isChangesetView(manifest(), 0), false);
  assert.equal(isChangesetView(manifest({ interdiff: {} as Manifest["interdiff"] }), undefined), false);
});

void test("the changeset's manifest is the one on screen in its own view, else the one read for the same revision", () => {
  const shown = manifest({ revision: 3 });
  const whole = manifest({ revision: 3, name: "whole" });
  assert.equal(changesetManifest(shown, undefined, undefined), shown);
  assert.equal(changesetManifest(shown, undefined, whole), shown);
  assert.equal(changesetManifest(shown, 1, whole), whole);
  assert.equal(changesetManifest(shown, 1, undefined), undefined);
  assert.equal(changesetManifest(shown, 1, { ...whole, revision: 2 }), undefined);
  assert.equal(changesetManifest(undefined, undefined, whole), undefined);
});

void test("a manifest is of the same view only for the same revision, range and commit", () => {
  const range = (from: number, to: number) => ({ interdiff: { from, to } as Manifest["interdiff"] });
  const held = manifest({ revision: 3, ...range(1, 2) });
  assert.equal(sameManifestView(held, undefined, manifest({ revision: 3, ...range(1, 2) }), undefined), true);
  assert.equal(sameManifestView(held, undefined, manifest({ revision: 4, ...range(1, 2) }), undefined), false);
  assert.equal(sameManifestView(held, undefined, manifest({ revision: 3, ...range(1, 3) }), undefined), false);
  assert.equal(sameManifestView(held, undefined, manifest({ revision: 3 }), undefined), false);
  assert.equal(sameManifestView(manifest({ revision: 3 }), 0, manifest({ revision: 3 }), 1), false);
  assert.equal(sameManifestView(manifest({ revision: 3 }), 1, manifest({ revision: 3 }), 1), true);
  assert.equal(sameManifestView(undefined, undefined, manifest({ revision: 3 }), undefined), false);
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
