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

// The popup is built from DOM nodes; this stands in for the few `document` calls a notice makes.
interface FakeNode {
  textContent: string;
  children: FakeNode[];
  append(...nodes: FakeNode[]): void;
}
const fakeNode = (): FakeNode => ({
  textContent: "",
  children: [],
  append(...nodes) {
    this.children.push(...nodes);
  },
});
const textOf = (node: FakeNode): string => node.textContent + node.children.map(textOf).join("");
const popupText = (build: () => unknown): string => {
  const real = Object.getOwnPropertyDescriptor(globalThis, "document");
  Object.defineProperty(globalThis, "document", { value: { createElement: fakeNode }, configurable: true });
  try {
    return textOf(build() as FakeNode);
  } finally {
    if (real) Object.defineProperty(globalThis, "document", real);
    else Reflect.deleteProperty(globalThis, "document");
  }
};

void test("an answer for a file type no language is configured for gives no popup", () => {
  assert.equal(hoverContent({ text: "", code: [], unavailable: unavailable("unsupported") }, []), undefined);
});

void test("a server that does not offer hover is silent, but still shows the diagnostics at the symbol", () => {
  const answer = { text: "", code: [], unavailable: unavailable("notOffered") };
  assert.equal(hoverContent(answer, []), undefined);
  const diagnostic = { message: "bad", severity: "error" } as never;
  assert.ok(hoverContent(answer, [diagnostic]));
});

void test("a server that fails says why, and a symbol the server knows nothing about stays silent", () => {
  const popup = hoverContent({ text: "", code: [], unavailable: unavailable("serverFailed") }, []);
  assert.ok(popup);
  assert.equal(popupText(popup), "why serverFailed");
  assert.equal(hoverContent({ text: "", code: [] }, []), undefined);
});
