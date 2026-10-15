import assert from "node:assert/strict";
import { test } from "node:test";
import { focusScope } from "./focusScope.ts";

/** The little of a document the service reads: the active element, the body and containment. */
class FakeElement {
  isConnected = true;
  focused = false;
  readonly children: FakeElement[] = [];

  contains(node: unknown): boolean {
    return node === this || this.children.some((child) => child.contains(node));
  }

  focus(): void {
    this.focused = true;
    dom.activeElement = this;
  }
}

const body = new FakeElement();
const dom = { activeElement: null as FakeElement | null, body };
Object.assign(globalThis, { HTMLElement: FakeElement, document: dom });

const el = () => new FakeElement();

void test("leaving gives the focus back to what had it", () => {
  const a = el();
  const surface = el();
  const field = el();
  surface.children.push(field);
  dom.activeElement = a;
  const scope = focusScope();
  dom.activeElement = field;
  scope.leave(surface);
  assert.equal(dom.activeElement, a);
  assert.ok(a.focused);
  assert.ok(!field.focused);
});

void test("a saved element that is gone is not focused", () => {
  const a = el();
  const surface = el();
  const field = el();
  surface.children.push(field);
  dom.activeElement = a;
  const scope = focusScope();
  dom.activeElement = field;
  a.isConnected = false;
  scope.leave(surface);
  assert.ok(!a.focused);
});

void test("a focus that moved on meanwhile stays where it is", () => {
  const a = el();
  dom.activeElement = a;
  const scope = focusScope();
  const c = el();
  dom.activeElement = c;
  scope.leave();
  assert.ok(!a.focused);
  assert.equal(dom.activeElement, c);
});

void test("a focus inside the surface is given back, one outside it is not", () => {
  const a = el();
  const surface = el();
  const field = el();
  surface.children.push(field);

  dom.activeElement = a;
  const inside = focusScope();
  dom.activeElement = field;
  inside.leave(surface);
  assert.equal(dom.activeElement, a);
  assert.ok(a.focused);

  const b = el();
  const outside = el();
  dom.activeElement = b;
  const moved = focusScope();
  dom.activeElement = outside;
  moved.leave(surface);
  assert.equal(dom.activeElement, outside);
  assert.ok(!b.focused);
});

void test("nested scopes give the focus back inwards", () => {
  const a = el();
  const b = el();
  const outerSurface = el();
  const innerSurface = el();
  outerSurface.children.push(innerSurface, b);

  dom.activeElement = a;
  const outer = focusScope();
  dom.activeElement = b;
  const inner = focusScope();

  inner.leave(innerSurface);
  assert.equal(dom.activeElement, b);
  outer.leave(outerSurface);
  assert.equal(dom.activeElement, a);
});

void test("an outer scope does not steal the focus from an inner one still open", () => {
  const a = el();
  const c = el();
  const outerSurface = el();
  const innerSurface = el();
  innerSurface.children.push(c);
  outerSurface.children.push(innerSurface);

  dom.activeElement = a;
  const outer = focusScope();
  dom.activeElement = c;
  const inner = focusScope();

  outer.leave(outerSurface);
  assert.ok(!a.focused);
  inner.leave(innerSurface);
  assert.equal(dom.activeElement, c);
});

void test("cancel ends a scope without giving the focus back", () => {
  const a = el();
  dom.activeElement = a;
  const scope = focusScope();
  const c = el();
  dom.activeElement = c;
  scope.cancel();
  scope.leave();
  assert.ok(!a.focused);
  assert.equal(dom.activeElement, c);
});

void test("a named target is restored instead of the remembered element", () => {
  const a = el();
  dom.activeElement = a;
  let restored = false;
  const scope = focusScope({
    restore: () => {
      restored = true;
      dom.activeElement = null;
    },
  });
  scope.leave();
  assert.ok(restored);
  assert.ok(!a.focused);
});
