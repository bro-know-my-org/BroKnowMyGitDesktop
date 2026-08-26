import assert from "node:assert/strict";
import test from "node:test";
import {
  beginGitLockInspection,
  finishGitLockInspection,
  gitLockInventoryDisposition,
  gitLockInspectionIsCurrent,
  replaceGitLockRecoveryDialog,
  runOwnedGitLockInspection,
} from "./gitLockRecoveryState";

test("lock inventory disposition preserves inconclusive and reference-lock recovery guidance", () => {
  assert.equal(gitLockInventoryDisposition(1, false, false), "showLocks");
  assert.equal(gitLockInventoryDisposition(0, true, false), "inconclusive");
  assert.equal(gitLockInventoryDisposition(0, true, true), "inconclusive");
  assert.equal(gitLockInventoryDisposition(0, false, true), "emptyReference");
  assert.equal(gitLockInventoryDisposition(0, false, false), "empty");
});

test("duplicate lock inspections share one owner slot until the request finishes", () => {
  const busyOwners = new Set<symbol>();
  const owner = Symbol("lock-recovery");

  assert.equal(beginGitLockInspection(busyOwners, owner), true);
  assert.equal(beginGitLockInspection(busyOwners, owner), false);
  finishGitLockInspection(busyOwners, owner);
  assert.equal(beginGitLockInspection(busyOwners, owner), true);
});

test("inspection results are rejected after repository or notification ownership changes", () => {
  const owner = Symbol("lock-recovery");

  assert.equal(gitLockInspectionIsCurrent(true, owner, owner), true);
  assert.equal(gitLockInspectionIsCurrent(false, owner, owner), false);
  assert.equal(gitLockInspectionIsCurrent(true, Symbol("replacement"), owner), false);
  assert.equal(gitLockInspectionIsCurrent(true, null, owner), false);
});

test("pending inspection results cannot populate recovery UI after owner replacement", async () => {
  const firstOwner = Symbol("first");
  const secondOwner = Symbol("second");
  let activeOwner: symbol | null = firstOwner;
  let repositoryIsCurrent = true;
  let resolveInspection!: (locks: string[]) => void;
  const inspection = new Promise<string[]>((resolve) => {
    resolveInspection = resolve;
  });

  const result = inspection.then((locks) => gitLockInspectionIsCurrent(
    repositoryIsCurrent,
    activeOwner,
    firstOwner,
  ) ? locks : []);
  activeOwner = secondOwner;
  repositoryIsCurrent = false;
  resolveInspection(["stale.lock"]);

  assert.deepEqual(await result, []);
  assert.equal(gitLockInspectionIsCurrent(true, activeOwner, secondOwner), true);
});

test("owned inspections release busy state and suppress stale results and failures", async () => {
  const busyOwners = new Set<symbol>();
  const oldOwner = Symbol("old");
  const newOwner = Symbol("new");
  let activeOwner: symbol | null = oldOwner;
  let repositoryIsCurrent = true;
  let resolveOld!: (locks: string[]) => void;
  const oldRequest = runOwnedGitLockInspection(
    busyOwners,
    oldOwner,
    () => gitLockInspectionIsCurrent(repositoryIsCurrent, activeOwner, oldOwner),
    () => new Promise<string[]>((resolve) => { resolveOld = resolve; }),
  );
  assert.equal(busyOwners.has(oldOwner), true);

  activeOwner = newOwner;
  repositoryIsCurrent = false;
  resolveOld(["stale.lock"]);
  assert.equal(await oldRequest, null);
  assert.equal(busyOwners.has(oldOwner), false);

  activeOwner = oldOwner;
  repositoryIsCurrent = true;
  let rejectOld!: (error: unknown) => void;
  const staleFailure = runOwnedGitLockInspection(
    busyOwners,
    oldOwner,
    () => gitLockInspectionIsCurrent(repositoryIsCurrent, activeOwner, oldOwner),
    () => new Promise<string[]>((_resolve, reject) => { rejectOld = reject; }),
  );
  activeOwner = newOwner;
  rejectOld(new Error("stale failure"));
  assert.equal(await staleFailure, null);
  assert.equal(busyOwners.has(oldOwner), false);
  assert.equal(busyOwners.has(newOwner), false);
});

test("replacing or clearing a recovery dialog destroys the stale dialog", () => {
  let firstDestroyed = 0;
  let secondDestroyed = 0;
  const first = { destroy: () => { firstDestroyed += 1; } };
  const second = { destroy: () => { secondDestroyed += 1; } };

  const active = replaceGitLockRecoveryDialog(first, second);
  assert.equal(active, second);
  assert.equal(firstDestroyed, 1);
  assert.equal(secondDestroyed, 0);

  assert.equal(replaceGitLockRecoveryDialog(active, null), null);
  assert.equal(secondDestroyed, 1);
});
