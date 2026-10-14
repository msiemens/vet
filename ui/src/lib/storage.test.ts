import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import { parseJson, readJson, readStored, storeFlag, storedFlag, writeStored } from "./storage.ts";

const store = new Map<string, string>();

beforeEach(() => {
  store.clear();
  Object.assign(globalThis, {
    localStorage: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => store.set(key, value),
      removeItem: (key: string) => store.delete(key),
    },
  });
});

afterEach(() => {
  Reflect.deleteProperty(globalThis, "localStorage");
});

void test("damaged or missing JSON is no value", () => {
  assert.deepEqual(parseJson('{"a":1}'), { a: 1 });
  assert.equal(parseJson("{not json"), undefined);
  assert.equal(parseJson(undefined), undefined);
});

void test("a stored value is checked by the reader, and one that fails the check is none", () => {
  const isNumbers = (v: unknown) => (Array.isArray(v) && v.every((n) => typeof n === "number") ? v : undefined);
  writeStored("k", "[1,2]");
  assert.deepEqual(readJson("k", isNumbers), [1, 2]);
  writeStored("k", '["a"]');
  assert.equal(readJson("k", isNumbers), undefined);
  writeStored("k", "{broken");
  assert.equal(readJson("k", isNumbers), undefined);
  assert.equal(readJson("absent", isNumbers), undefined);
});

void test("a flag is read in every spelling an earlier version wrote", () => {
  const read = (value: string | undefined, fallback: boolean) => {
    writeStored("flag", value);
    return storedFlag("flag", fallback);
  };
  assert.equal(read("1", false), true);
  assert.equal(read("open", false), true);
  assert.equal(read("0", true), false);
  assert.equal(read("shut", true), false);
  assert.equal(read(undefined, true), true);
  assert.equal(read(undefined, false), false);
  assert.equal(read("garbage", true), true);
});

void test("only a flag that differs from its default is kept", () => {
  storeFlag("flag", true, false);
  assert.equal(readStored("flag"), "1");
  assert.equal(storedFlag("flag", false), true);
  storeFlag("flag", false, false);
  assert.equal(readStored("flag"), undefined);
  storeFlag("flag", false, true);
  assert.equal(storedFlag("flag", true), false);
});
