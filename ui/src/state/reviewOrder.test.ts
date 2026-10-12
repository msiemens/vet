import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { ReviewOrder } from "./reviewOrder.ts";

void describe("ReviewOrder", () => {
  void it("ignores an update of a file that is older than the state held, however late it arrives", () => {
    const order = new ReviewOrder();
    order.load(1, ["a", "b"], false);
    assert.equal(order.file("a", 5), true);
    assert.equal(order.file("a", 4), false);
    assert.equal(order.file("b", 4), true, "another file's state is its own");
  });

  void it("applies an update once when the answer and the event carry the same state", () => {
    const order = new ReviewOrder();
    order.load(1, ["a"], false);
    assert.equal(order.file("a", 2), true);
    assert.equal(order.file("a", 2), false);
    assert.equal(order.completion(2), true);
    assert.equal(order.completion(2), false);
    assert.equal(order.changeset(2), true);
    assert.equal(order.changeset(2), false);
  });

  void it("keeps the state held when a manifest read before it arrives after it", () => {
    const order = new ReviewOrder();
    order.load(1, ["a", "b"], false);
    order.file("a", 4);
    order.completion(4);
    order.changeset(4);
    const newer = order.load(3, ["a", "b"], true);
    assert.deepEqual([...newer.files], ["a"]);
    assert.deepEqual([newer.completion, newer.changeset], [true, true]);
    assert.equal(order.file("a", 4), false, "the state kept still is the one at 4");
    assert.equal(order.file("b", 3), false, "the manifest's state of b is held at 3");
  });

  void it("lets a manifest replace what is older and what it is not of", () => {
    const order = new ReviewOrder();
    order.load(1, ["a"], false);
    order.file("a", 2);
    const newer = order.load(5, ["a"], true);
    assert.deepEqual([newer.files.size, newer.completion, newer.changeset], [0, false, false]);
    order.file("a", 9);
    const other = order.load(6, ["a"], false);
    assert.deepEqual([other.files.size, other.completion, other.changeset], [0, false, false]);
    assert.equal(order.file("a", 7), true, "a manifest of another view starts over");
  });

  void it("orders the view's completion and the changeset's separately", () => {
    const order = new ReviewOrder();
    order.load(1, [], false);
    assert.equal(order.completion(5), true);
    assert.equal(order.changeset(3), true, "the changeset's own counter has not seen 5");
  });
});
