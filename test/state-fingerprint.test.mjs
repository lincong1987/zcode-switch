import assert from "node:assert/strict";
import test from "node:test";
import { stateFingerprint } from "../src/state-fingerprint.js";

test("ignores object key order", () => {
  assert.equal(
    stateFingerprint({ b: 2, a: { d: 4, c: 3 } }),
    stateFingerprint({ a: { c: 3, d: 4 }, b: 2 }),
  );
});

test("detects changed values and array order", () => {
  const base = { accounts: [{ id: "a" }, { id: "b" }] };
  assert.notEqual(stateFingerprint(base), stateFingerprint({ accounts: [{ id: "a" }, { id: "c" }] }));
  assert.notEqual(stateFingerprint(base), stateFingerprint({ accounts: [{ id: "b" }, { id: "a" }] }));
});
