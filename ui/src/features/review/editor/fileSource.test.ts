import assert from "node:assert/strict";
import { test } from "node:test";
import type { FileLayout } from "../../../state/reviewModel.ts";
import type { Backend } from "../../../state/backend.ts";
import { StaleRevision, fileSource } from "./fileSource.ts";

const reply = (revision: number) => ({
  type: "fileContents",
  revision,
  base: "old",
  target: "new",
  changes: [],
  baseHighlights: [],
  targetHighlights: [],
});

/** A backend that answers every file as of `revision` and counts the requests. */
const backendAt = (revision: number) => {
  const requests: string[] = [];
  const backend = {
    fileContents: (path: string) => {
      requests.push(path);
      return Promise.resolve(reply(revision));
    },
  } as unknown as Backend;
  return { backend, requests };
};

const file = (hunks: number, targetLines: number) =>
  ({
    path: "a.rs",
    hunks: Array.from({ length: hunks }, () => ({})),
    baseLines: 0,
    targetLines,
  }) as unknown as FileLayout;

void test("a changed file loads with the hunks of the manifest it belongs to", async () => {
  const { backend } = backendAt(3);
  const changed = file(2, 10);
  const loaded = await fileSource(backend, 3).contents(changed);
  assert.equal(loaded.hunks, changed.hunks);
});

void test("a reply of another revision is refused", async () => {
  const { backend } = backendAt(4);
  await assert.rejects(fileSource(backend, 3).contents(file(1, 10)), StaleRevision);
});

void test("a file without text lines is not requested: the backend refuses it", async () => {
  const { backend, requests } = backendAt(3);
  const loaded = await fileSource(backend, 3).contents(file(0, 0));
  assert.deepEqual([loaded.base, loaded.target, requests.length], ["", "", 0]);
});
