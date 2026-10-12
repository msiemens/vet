import assert from "node:assert/strict";
import { test } from "node:test";
import { rowLabel } from "./symbolLabel.ts";

void test("a row names the symbol and the innermost container, not the namespace chain", () => {
  const label = (path: string, name: string[]) => rowLabel({ path, name }, path);
  assert.deepEqual(label("src/Invoice.ts", ["Acme.Billing.Core", "Invoice", "merge"]), {
    name: "merge",
    context: "Invoice",
  });
  assert.deepEqual(label("billing/core.rs", ["billing", "core", "impl Invoice", "merge"]), {
    name: "merge",
    context: "impl Invoice",
  });
  assert.deepEqual(label("billing/invoice.py", ["Invoice", "merge"]), { name: "merge", context: "Invoice" });
});

void test("a symbol at the top level of its file is placed in the file", () => {
  assert.deepEqual(rowLabel({ path: "src/billing/util.py", name: ["total"] }, "src/billing/util.py"), {
    name: "total",
    context: "util.py",
  });
});

void test("a place without a symbol is its file, whatever the separators of its path", () => {
  assert.deepEqual(rowLabel(null, "src/billing/util.py"), { name: "util.py", context: "" });
  assert.deepEqual(rowLabel(null, "/home/dep/lib/mod.rs"), { name: "mod.rs", context: "" });
  assert.deepEqual(rowLabel(null, "C:\\dep\\Mod.cs"), { name: "Mod.cs", context: "" });
});

void test("a C# method is named without its parameters and return type", () => {
  const id = {
    path: "src/Invoice.cs",
    name: ["Acme.Billing.Core", "Invoice", "Merge(Dictionary<string, List<Line>>, Task<Line>) : List<Line>"],
  };
  assert.deepEqual(rowLabel(id, id.path), { name: "Merge", context: "Invoice" });
  // The same goes for a method that contains the place, and for a constructor.
  const local = { path: id.path, name: ["Acme", "Invoice", "Merge(int x) : void", "Helper(int y) : int"] };
  assert.deepEqual(rowLabel(local, local.path), { name: "Helper", context: "Merge" });
  const constructor = { path: id.path, name: ["Acme", "Invoice", "Invoice(int id)"] };
  assert.deepEqual(rowLabel(constructor, constructor.path), { name: "Invoice", context: "Invoice" });
});

void test("parentheses in names of other languages are kept", () => {
  const id = { path: "src/a.ts", name: ["Invoice", "(anonymous)"] };
  assert.equal(rowLabel(id, id.path).name, "(anonymous)");
  const callback = { path: "src/a.ts", name: ["run", "map() callback"] };
  assert.equal(rowLabel(callback, callback.path).name, "map() callback");
});
