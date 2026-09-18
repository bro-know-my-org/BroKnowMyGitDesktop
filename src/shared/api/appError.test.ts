import assert from "node:assert/strict";
import test from "node:test";
import { asAppError } from "./appError";

test("preserves redacted diagnostics for object and serialized IPC errors", () => {
  const input = {
    code: "git_push_rejected_by_policy",
    message: "backend title",
    details: "backend guidance",
    diagnostic: "remote: GH013\nremote: Changes must be made through a pull request.",
  };
  for (const value of [input, JSON.stringify(input)]) {
    const error = asAppError(value);
    assert.equal(error.diagnostic, input.diagnostic);
    assert.ok(error.details?.includes(input.diagnostic));
    assert.notEqual(error.message, input.message);
    assert.deepEqual(asAppError(error), error);
  }
});

test("unknown failures retain diagnostics without forwarding unredacted fallback fields", () => {
  const error = asAppError({ code: ["custom", "error"].join("_"), message: "secret", details: "secret", diagnostic: "hook declined: missing ticket" });
  assert.equal(error.details, "hook declined: missing ticket");
  assert.ok(!JSON.stringify(error).includes("secret"));
  assert.equal(asAppError("fatal: unredacted secret").details, null);
  assert.equal(asAppError(null).details, null);
});

test("normalization is idempotent when backend guidance is absent", () => {
  for (const diagnostic of [null, "remote: rejected by branch protection"]) {
    const input = { code: "git_push_rejected_by_policy", message: "backend title", details: null, diagnostic };
    const error = asAppError(input);
    assert.deepEqual(asAppError(error), error);
    assert.deepEqual(asAppError(JSON.stringify(error)), error);
  }
});
