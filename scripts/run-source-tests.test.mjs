import assert from "node:assert/strict";
import test from "node:test";
import path from "node:path";
import { runSourceTests, sourceTests } from "./run-source-tests.mjs";

test("source test traversal failures are propagated", async () => {
  await assert.rejects(
    sourceTests("src", async () => {
      throw new Error("unreadable");
    }),
    /unreadable/u,
  );
});

test("source test discovery recurses, filters extensions, and sorts paths", async () => {
  const directoryEntry = (name) => ({ name, isDirectory: () => true, isFile: () => false });
  const fileEntry = (name) => ({ name, isDirectory: () => false, isFile: () => true });
  const nestedDirectory = path.join("src", "nested");
  const tree = new Map([
    ["src", [fileEntry("z.test.ts"), directoryEntry("nested"), fileEntry("ignored.ts")]],
    [nestedDirectory, [fileEntry("b.test.mjs"), fileEntry("a.test.ts"), fileEntry("ignored.js")]],
  ]);

  const tests = await sourceTests("src", async (directory) => tree.get(directory) ?? []);

  assert.deepEqual(tests, [
    path.join(nestedDirectory, "a.test.ts"),
    path.join(nestedDirectory, "b.test.mjs"),
    path.join("src", "z.test.ts"),
  ]);
});

test("source test runner rejects empty discovery", async () => {
  const errors = [];
  const status = await runSourceTests({
    discover: async () => [],
    stderr: (...parts) => errors.push(parts.join(" ")),
  });

  assert.equal(status, 1);
  assert.match(errors[0], /No source tests found/u);
});

test("source test runner reports discovery failures", async () => {
  const errors = [];
  const status = await runSourceTests({
    discover: async () => {
      throw new Error("unreadable");
    },
    stderr: (...parts) => errors.push(parts.join(" ")),
  });

  assert.equal(status, 1);
  assert.match(errors[0], /Failed to discover source tests/u);
});

test("source test runner propagates spawn failures", async () => {
  const errors = [];
  const status = await runSourceTests({
    discover: async () => ["src/example.test.ts"],
    spawn: () => ({ error: new Error("spawn unavailable") }),
    stderr: (...parts) => errors.push(parts.join(" ")),
  });

  assert.equal(status, 1);
  assert.match(errors[0], /Failed to start source tests/u);
});

test("source test runner propagates signal termination", async () => {
  const errors = [];
  const status = await runSourceTests({
    discover: async () => ["src/example.test.ts"],
    spawn: () => ({ signal: "SIGTERM", status: null }),
    stderr: (...parts) => errors.push(parts.join(" ")),
  });

  assert.equal(status, 1);
  assert.match(errors[0], /SIGTERM/u);
});

test("source test runner returns a nonzero child status", async () => {
  const status = await runSourceTests({
    discover: async () => ["src/example.test.ts"],
    spawn: () => ({ signal: null, status: 7 }),
  });

  assert.equal(status, 7);
});
