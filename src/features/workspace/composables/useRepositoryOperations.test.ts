import assert from "node:assert/strict";
import test from "node:test";
import { computed, effectScope, ref } from "vue";
import { api } from "../../../shared/api/tauri";
import type { RepositorySnapshot } from "../../../shared/types/domain";
import { useRepositoryOperations } from "./useRepositoryOperations";

function fixture(names: string[], path = "/repo-a"): RepositorySnapshot {
  return {
    repository: { path },
    tags: names.map((name) => ({ name, target: "abc123" })),
    branches: [],
  } as unknown as RepositorySnapshot;
}

function setup(names: string[]) {
  const scope = effectScope();
  const activeRepositoryPath = ref<string | null>("/repo-a");
  const snapshot = ref<RepositorySnapshot | null>(fixture(names));
  const warnings: string[] = [];
  let refreshCount = 0;
  const operations = scope.run(() => useRepositoryOperations({
    activeRepositoryPath,
    snapshot,
    selectedChange: ref(null),
    conflictedChanges: computed(() => []),
    currentBranch: computed(() => "main"),
    commits: ref([]),
    getSelectionGeneration: () => 1,
    repositorySelectionIsCurrent: (path) => activeRepositoryPath.value === path,
    applySnapshot: () => true,
    syncSelectedDiff: async () => undefined,
    loadHistory: async () => undefined,
    refreshSnapshot: async () => { refreshCount += 1; },
    refreshAfterGitConflict: async () => undefined,
    reconcileSnapshotAfterMutationError: async () => undefined,
    gitOperationLabel: () => "",
    reportError: (error) => { throw error; },
    message: { warning: (text: string) => warnings.push(text), success: () => undefined } as never,
    dialog: {} as never,
  }))!;
  return { operations, activeRepositoryPath, snapshot, warnings, scope, refreshCount: () => refreshCount };
}

test("tag suggestions sort naturally, prioritize prefixes, and search beyond the display limit", (t) => {
  const state = setup(["release-v2", "v10", "v2", "V3", ...Array.from({ length: 30 }, (_, i) => `build-${i}`)]);
  t.after(() => state.scope.stop());
  const { operations } = state;
  operations.openTagModal();
  assert.equal(operations.tagNameOptions.value.length, 20);
  operations.tagForm.name = "v";
  assert.deepEqual(operations.tagNameOptions.value.map((option) => option.value), ["v2", "V3", "v10", "release-v2"]);
  operations.tagForm.name = "build-29";
  assert.deepEqual(operations.tagNameOptions.value.map((option) => option.value), ["build-29"]);
  operations.tagForm.name = "new-name";
  assert.equal(operations.tagNameOptions.value.length, 0);
  state.snapshot.value = fixture([]);
  operations.tagForm.name = "";
  assert.equal(operations.tagNameOptions.value.length, 0);
});

test("duplicate tags cannot reach the API; editing a suggestion preserves the chosen target", async (t) => {
  const state = setup(["v2"]);
  const original = api.createTag;
  const calls: unknown[][] = [];
  api.createTag = async (...args) => { calls.push(args); };
  t.after(() => { api.createTag = original; state.scope.stop(); });
  const { operations } = state;
  operations.openTagModal("chosen-commit");
  operations.tagForm.name = " v2 ";
  assert.equal(operations.tagNameExists.value, true);
  await operations.createTag();
  assert.equal(calls.length, 0);
  assert.equal(state.warnings.length, 1);
  assert.equal(operations.tagModalShown.value, true);
  operations.tagForm.name = "V2";
  assert.equal(operations.tagNameExists.value, false, "Git tag names are case-sensitive");
  operations.tagForm.name = "v3";
  await operations.createTag();
  assert.deepEqual(calls, [["/repo-a", "v3", "chosen-commit", undefined]]);
  assert.equal(state.refreshCount(), 1);
  assert.equal(operations.tagModalShown.value, false);
});

test("repository changes close the tag dialog and never suggest tags from a stale snapshot", async (t) => {
  const state = setup(["old-tag"]);
  const original = api.createTag;
  let calls = 0;
  api.createTag = async () => { calls += 1; };
  t.after(() => { api.createTag = original; state.scope.stop(); });
  const { operations } = state;
  operations.openTagModal("old-commit");
  operations.tagForm.name = "draft";
  state.activeRepositoryPath.value = "/repo-b";
  assert.equal(operations.tagModalShown.value, false);
  assert.equal(operations.tagForm.name, "");
  assert.equal(operations.tagNameOptions.value.length, 0);
  operations.tagForm.name = "stale-submit";
  await operations.createTag();
  assert.equal(calls, 0);
  state.snapshot.value = fixture(["new-tag"], "/repo-b");
  operations.openTagModal();
  assert.equal(operations.tagForm.target, "");
  assert.deepEqual(operations.tagNameOptions.value.map((option) => option.value), ["new-tag"]);
  state.snapshot.value = fixture(["fresh-tag"], "/repo-b");
  assert.deepEqual(operations.tagNameOptions.value.map((option) => option.value), ["fresh-tag"]);
});
