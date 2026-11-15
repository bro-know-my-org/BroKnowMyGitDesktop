import assert from "node:assert/strict";
import test from "node:test";
import { api } from "./tauri";
import { mockSnapshot } from "./browserDemoFixtures";

test("demo history rejects stale page anchors after a branch tip changes", async () => {
  const path = mockSnapshot.repository.path;
  const before = await api.refreshRepositorySnapshot(path);
  const [first] = await api.listCommits(path, 1);
  const branch = before.branches.find((candidate) => !candidate.isCurrent);
  assert.ok(first && branch && before.status.branch);
  await api.listCommits(path, 1, 1, first.sha);
  try {
    await api.checkoutBranch(path, branch.name, true);
    const switched = await api.commitChanges(path, "Updated branch tip", true);
    assert.notEqual(switched.status.headCommit, first.sha);
    await assert.rejects(api.listCommits(path, 1, 1, first.sha), { code: "repository_changed" });
    const page = await api.listCommits(path, 1, 1, switched.status.headCommit!);
    assert.deepEqual(page, await api.listCommits(path, 1, 1));
  } finally {
    await api.checkoutBranch(path, before.status.branch, true);
  }
});
