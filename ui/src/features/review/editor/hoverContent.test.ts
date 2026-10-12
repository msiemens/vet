import assert from "node:assert/strict";
import { test } from "node:test";
import type { Unavailable } from "../../../generated/Unavailable.ts";
import { hoverContent } from "./hoverContent.ts";

const unavailable = (kind: Unavailable["kind"]): Unavailable => ({
  kind,
  label: kind,
  message: `why ${kind}`,
  language: null,
});

void test("an answer for a file type no language is configured for gives no popup", () => {
  assert.equal(hoverContent({ text: "", code: [], unavailable: unavailable("unsupported") }, []), undefined);
});

void test("a server that fails says why, and a symbol the server knows nothing about stays silent", () => {
  assert.notEqual(hoverContent({ text: "", code: [], unavailable: unavailable("serverFailed") }, []), undefined);
  assert.equal(hoverContent({ text: "", code: [] }, []), undefined);
});
