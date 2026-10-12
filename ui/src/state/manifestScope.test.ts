import assert from "node:assert/strict";
import { test } from "node:test";
import type { Manifest } from "../generated/Manifest";
import { isChangesetView } from "./manifestScope.ts";

const manifest = (extra: Partial<Manifest> = {}) => ({ ...extra }) as Manifest;

void test("only a manifest without range and commit is the changeset's", () => {
  assert.equal(isChangesetView(manifest(), undefined), true);
  assert.equal(isChangesetView(manifest(), 0), false);
  assert.equal(isChangesetView(manifest({ interdiff: {} as Manifest["interdiff"] }), undefined), false);
});
