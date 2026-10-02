import assert from "node:assert/strict";
import test from "node:test";
import { latestWindowRead, pollWindowState, windowSubscriptions } from "./useWindowChrome";

test("subscriptions retry failures without duplicating healthy or pending listeners", async () => {
  const attempts = [0, 0, 0];
  const removed: number[] = [];
  let failures = 0;
  let finishPending: (unlisten: () => void) => void = () => {};
  const subscriptions = windowSubscriptions([
    async () => { attempts[0] += 1; return () => { removed.push(0); }; },
    async () => {
      attempts[1] += 1;
      if (attempts[1] === 1) throw new Error("transient registration failure");
      return () => { removed.push(1); };
    },
    () => { attempts[2] += 1; return new Promise((resolve) => { finishPending = resolve; }); },
  ], () => { failures += 1; });
  subscriptions.retry();
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(subscriptions.ready(), false);
  assert.equal(failures, 1);
  subscriptions.retry();
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(attempts, [1, 2, 1]);
  finishPending(() => { removed.push(2); });
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(subscriptions.ready(), true);
  subscriptions.retry();
  assert.deepEqual(attempts, [1, 2, 1]);
  subscriptions.dispose();
  assert.deepEqual(removed, [0, 1, 2]);
});

test("a pending registration cannot prevent cleanup and late listeners are removed", async () => {
  const removed: number[] = [];
  let finishPending: (unlisten: () => void) => void = () => {};
  const subscriptions = windowSubscriptions([
    async () => () => { removed.push(0); },
    () => new Promise((resolve) => { finishPending = resolve; }),
  ], () => assert.fail("no subscription should fail"));
  subscriptions.retry();
  await new Promise<void>((resolve) => setImmediate(resolve));
  subscriptions.dispose();
  assert.deepEqual(removed, [0]);
  finishPending(() => { removed.push(1); });
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(removed, [0, 1]);
});

test("a native focus event takes precedence over an in-flight focus query", async () => {
  let revision = 0;
  let focused = true;
  let resolveQuery: (value: boolean) => void = () => {};
  const read = latestWindowRead(
    () => new Promise<boolean>((resolve) => { resolveQuery = resolve; }),
    (value) => { focused = value; },
    () => revision,
  );
  const query = read();
  revision += 1;
  focused = false;
  resolveQuery(true);
  await query;
  assert.equal(focused, false);
});

test("a delayed resize read cannot overwrite a newer action read", async () => {
  const pending: Array<(value: boolean) => void> = [];
  const applied: boolean[] = [];
  const refresh = latestWindowRead(
    () => new Promise<boolean>((resolve) => { pending.push(resolve); }),
    (value) => { applied.push(value); },
  );
  const resize = refresh();
  const action = refresh();
  pending[1](true);
  await action;
  pending[0](false);
  await resize;
  assert.deepEqual(applied, [true]);
});

test("fallback polling retries failed state reads, coalesces slow reads and stops on disposal", async (context) => {
  context.mock.timers.enable({ apis: ["setInterval"] });
  let calls = 0;
  let rejectPending: (error: Error) => void = () => {};
  const stop = pollWindowState(() => {
    calls += 1;
    return new Promise<void>((_resolve, reject) => { rejectPending = reject; });
  });
  context.after(stop);
  context.mock.timers.tick(1_000);
  assert.equal(calls, 1);
  context.mock.timers.tick(3_000);
  assert.equal(calls, 1, "a slow native read must not accumulate more requests");
  rejectPending(new Error("transient bridge failure"));
  await new Promise<void>((resolve) => setImmediate(resolve));
  context.mock.timers.tick(1_000);
  assert.equal(calls, 2, "a failed read must not disable future synchronization");
  stop();
  rejectPending(new Error("disposed bridge"));
  await new Promise<void>((resolve) => setImmediate(resolve));
  context.mock.timers.tick(5_000);
  assert.equal(calls, 2, "disposing must cancel all future polling");
});
