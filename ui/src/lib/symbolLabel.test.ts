import assert from "node:assert/strict";
import { test } from "node:test";
import { fullName, nameOf, placeLabel, shortLabel } from "./symbolLabel.ts";

const label = (path: string, name: string[], modules = 0) => shortLabel({ id: { path, name }, modules });

// Names as the language servers list them (measured: roslyn-language-server, typescript-language-server,
// rust-analyzer, pyright).
const CS = "src/Device.cs";
const NAMESPACE = "HES.LCD500.Domain.Entities";

void test("a C# member is named without its signature, a type or namespace with it kept whole", () => {
  const name = (member: string) => nameOf({ path: CS, name: [NAMESPACE, "Device", member] });
  assert.equal(name("Create(int) : Device"), "Create");
  assert.equal(
    name("GetByGroupAsync(IReadOnlyList<int>, string) : Task<Dictionary<string, List<int>>>"),
    "GetByGroupAsync",
  );
  assert.equal(name("Map<T>(Func<int, T>) : T"), "Map<T>");
  assert.equal(name("operator +(Device, Device) : Device"), "operator +");
  assert.equal(name("this[int] : Device"), "this[int]");
  assert.equal(name("~Device()"), "~Device");
  assert.equal(name("Id : int"), "Id");
  assert.equal(name("Changed : Action"), "Changed");
  assert.equal(nameOf({ path: CS, name: [NAMESPACE, "Nested<T>"] }), "Nested<T>");
  assert.equal(nameOf({ path: CS, name: [NAMESPACE] }), NAMESPACE);
});

void test("names of other languages are kept as they are", () => {
  assert.equal(nameOf({ path: "src/a.ts", name: ["Box", "[Symbol.iterator]"] }), "[Symbol.iterator]");
  assert.equal(nameOf({ path: "src/a.ts", name: ["Box", '"quoted-name"'] }), '"quoted-name"');
  assert.equal(nameOf({ path: "src/a.ts", name: ["Box"] }), "Box");
  assert.equal(nameOf({ path: "src/a.ts", name: ["Invoice", "(anonymous)"] }), "(anonymous)");
  assert.equal(nameOf({ path: "src/a.ts", name: ["run", "map() callback"] }), "map() callback");
  // A colon is part of a Python or TypeScript name only where a server puts one.
  assert.equal(nameOf({ path: "src/a.ts", name: ["x : y"] }), "x : y");
  assert.equal(nameOf({ path: "m.py", name: ["Invoice", "__add__"] }), "__add__");
});

void test("a symbol is named with its container, outside the namespace", () => {
  assert.equal(label(CS, [NAMESPACE, "Device", "Create(int) : Device"], 1), "Device.Create");
  // A type directly in a namespace is just its name.
  assert.equal(label(CS, [NAMESPACE, "Device"], 1), "Device");
  // Namespaces nested in blocks: `Old.Style` > `Inner` > `K` > `M`.
  assert.equal(label(CS, ["Old.Style", "Inner", "K", "M(int) : void"], 2), "K.M");
  assert.equal(label(CS, ["Old.Style", "Inner", "K"], 2), "K");
  // The namespace itself is shown whole.
  assert.equal(label(CS, [NAMESPACE], 1), NAMESPACE);
  assert.equal(label(CS, ["Old.Style", "Inner"], 2), "Inner");
  // Without a namespace the container stays.
  assert.equal(label("src/Invoice.cs", ["Invoice", "Merge(int) : void"]), "Invoice.Merge");
  // Rust modules are namespaces as well, an impl block is named by its type.
  assert.equal(label("src/lib.rs", ["billing", "core", "impl Backoff for Invoice", "wait"], 2), "Invoice::wait");
  assert.equal(
    label("src/lib.rs", ["billing", "core", "impl<T: Clone> From<T> for Retry<T>", "from"], 2),
    "Retry<T>::from",
  );
  assert.equal(label("src/lib.rs", ["billing", "core", "free"], 2), "free");
  assert.equal(label("src/lib.rs", ["billing", "core", "impl Retry<T>"], 2), "Retry<T>");
  // A TypeScript namespace is one name with dots.
  assert.equal(label("src/a.ts", ["Acme.Billing", "Invoice", "merge"], 1), "Invoice.merge");
  assert.equal(label("src/a.ts", ["Box", "create"]), "Box.create");
  assert.equal(label("m.py", ["Invoice", "Inner", "go"]), "Inner.go");
  assert.equal(label("m.py", ["free"]), "free");
});

void test("the whole name keeps the namespace, as its language writes it, without signatures", () => {
  assert.equal(
    fullName({ path: CS, name: [NAMESPACE, "Device", "Create(int) : Device"] }),
    "HES.LCD500.Domain.Entities.Device.Create",
  );
  assert.equal(fullName({ path: "src/lib.rs", name: ["billing", "core", "free"] }), "billing::core::free");
});

void test("a place without a symbol is its file, whatever the separators of its path", () => {
  assert.equal(placeLabel(null, "src/billing/util.py"), "util.py");
  assert.equal(placeLabel(null, "/home/dep/lib/mod.rs"), "mod.rs");
  assert.equal(placeLabel(null, "C:\\dep\\Mod.cs"), "Mod.cs");
  assert.equal(placeLabel({ id: { path: "a.py", name: ["f"] }, modules: 0 }, "a.py"), "f");
});
