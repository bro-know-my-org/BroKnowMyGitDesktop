import assert from "node:assert/strict";
import test from "node:test";
import { buildCloneOptions, defaultCloneOptionsForm, validCloneDepth } from "./cloneOptions";

test("disabled shallow clone ignores a previously entered depth", () => {
  const form = { ...defaultCloneOptionsForm(), depth: null };
  assert.equal(validCloneDepth(form), true);
  assert.equal(buildCloneOptions(form).depth, undefined);
  form.shallow = true;
  assert.equal(validCloneDepth(form), false);
});

test("shallow depth must be a positive Git-sized integer", () => {
  for (const depth of [null, 0, -1, 1.5, Infinity, 2147483648]) {
    assert.equal(validCloneDepth({ ...defaultCloneOptionsForm(), shallow: true, depth }), false);
  }
  assert.equal(validCloneDepth({ ...defaultCloneOptionsForm(), shallow: true, depth: 10 }), true);
});

test("submodule flags require checkout and recursive cloning", () => {
  const form = { ...defaultCloneOptionsForm(), shallowSubmodules: true };
  assert.equal(buildCloneOptions(form).shallowSubmodules, false);
  form.recurseSubmodules = true;
  assert.equal(buildCloneOptions(form).shallowSubmodules, true);
  form.noCheckout = true;
  assert.equal(buildCloneOptions(form).recurseSubmodules, false);
  assert.equal(buildCloneOptions(form).shallowSubmodules, false);
});

test("branch and partial clone values are normalized", () => {
  const form = { ...defaultCloneOptionsForm(), branch: " release/v1 ", partial: true };
  assert.equal(buildCloneOptions(form).branch, "release/v1");
  assert.equal(buildCloneOptions(form).filter, "blob:none");
});
