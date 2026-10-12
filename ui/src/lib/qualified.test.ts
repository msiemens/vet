import assert from "node:assert/strict";
import { test } from "node:test";
import { splitQualified } from "./qualified.ts";

void test("a qualified name splits after its last separator, whichever the language uses", () => {
  assert.deepEqual(splitQualified("HES.LCD500.Api.Api.DataLoaders"), {
    qualifier: "HES.LCD500.Api.Api.",
    name: "DataLoaders",
  });
  assert.deepEqual(splitQualified("crate::billing::core"), { qualifier: "crate::billing::", name: "core" });
  assert.deepEqual(splitQualified("Acme.Billing::Invoice"), { qualifier: "Acme.Billing::", name: "Invoice" });
});

void test("a single segment has no qualifier, and a single colon is no separator", () => {
  assert.deepEqual(splitQualified("Invoice"), { qualifier: "", name: "Invoice" });
  assert.deepEqual(splitQualified(""), { qualifier: "", name: "" });
  assert.deepEqual(splitQualified("key: value"), { qualifier: "", name: "key: value" });
});

void test("a separator inside type arguments or parameters does not split", () => {
  assert.deepEqual(splitQualified("List<Foo.Bar>"), { qualifier: "", name: "List<Foo.Bar>" });
  assert.deepEqual(splitQualified("Acme.Repo<Foo.Bar>"), { qualifier: "Acme.", name: "Repo<Foo.Bar>" });
  assert.deepEqual(splitQualified("Acme.Repo<Foo.Bar>.Load(A.B)"), {
    qualifier: "Acme.Repo<Foo.Bar>.",
    name: "Load(A.B)",
  });
  assert.deepEqual(splitQualified("std::vec::Vec<std::string::String>"), {
    qualifier: "std::vec::",
    name: "Vec<std::string::String>",
  });
});

void test("a description with spaces stays whole", () => {
  assert.deepEqual(splitQualified("impl crate::Foo for Bar"), { qualifier: "", name: "impl crate::Foo for Bar" });
  assert.deepEqual(splitQualified("Merge(A.B) : List<Line>"), { qualifier: "", name: "Merge(A.B) : List<Line>" });
});
