import assert from "node:assert/strict";
import test from "node:test";
import { computed, nextTick, ref, watch } from "vue";
import { api } from "../../../shared/api/tauri";
import type { GitStatusEntry } from "../../../shared/types/domain";
import { useRepositoryChanges } from "./useRepositoryChanges";

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function change(path: string): GitStatusEntry {
  return { path, originalPath: null, indexStatus: "", worktreeStatus: "M" };
}

test("stale blame requests cannot update or clear newer repository state", async () => {
  const originalGetFileBlame = api.getFileBlame;
  const requests: Deferred<string>[] = [];
  api.getFileBlame = async () => {
    const request = deferred<string>();
    requests.push(request);
    return request.promise;
  };

  const activeRepositoryPath = ref<string | null>("/repo-a");
  let selectionGeneration = 1;
  const reportedErrors: unknown[] = [];
  const changes = useRepositoryChanges({
    activeRepositoryPath,
    snapshot: ref(null),
    currentBranch: computed(() => "main"),
    commits: ref([]),
    getSelectionGeneration: () => selectionGeneration,
    repositorySelectionIsCurrent: (path, generation) =>
      path === activeRepositoryPath.value && generation === selectionGeneration,
    applySnapshot: () => true,
    loadHistory: async () => undefined,
    isChangeFullyIncluded: () => false,
    isConflictedEntry: () => false,
    splitDiffHunks: () => [],
    splitDiffLines: () => [],
    reportError: (error) => reportedErrors.push(error),
    message: { warning: () => undefined, success: () => undefined, info: () => undefined } as never,
    dialog: {} as never,
  });

  try {
    changes.selectedChange.value = change("first.txt");
    const first = changes.openFileBlame();
    assert.equal(changes.blameLoading.value, true);

    changes.blameModalShown.value = false;
    await nextTick();
    requests[0]?.resolve("stale closed blame");
    await first;
    assert.equal(changes.blameText.value, "");
    assert.equal(changes.blameLoading.value, false);
    assert.deepEqual(reportedErrors, []);

    changes.selectedChange.value = change("first.txt");
    const oldFile = changes.openFileBlame();
    changes.selectedChange.value = change("second.txt");
    const newFile = changes.openFileBlame();
    requests[1]?.resolve("stale first-file blame");
    await oldFile;
    assert.equal(changes.blameText.value, "");
    assert.equal(changes.blameLoading.value, true);
    requests[2]?.resolve("current second-file blame");
    await newFile;
    assert.equal(changes.blameText.value, "current second-file blame");
    assert.equal(changes.blameLoading.value, false);

    changes.blameText.value = "previous localized blame";
    changes.blameModalShown.value = true;
    const silentRefresh = changes.loadFileBlame("/repo-a", "second.txt", selectionGeneration, false);
    requests[3]?.reject(new Error("silent locale refresh failure"));
    await silentRefresh;
    assert.equal(changes.blameText.value, "previous localized blame");
    assert.equal(changes.blameModalShown.value, true);
    assert.equal(changes.blameLoading.value, false);
    assert.deepEqual(reportedErrors, []);

    let localeGeneration = 1;
    const firstLocaleRefresh = changes.loadFileBlame(
      "/repo-a",
      "second.txt",
      selectionGeneration,
      false,
      () => localeGeneration === 1,
    );
    localeGeneration = 2;
    const secondLocaleRefresh = changes.loadFileBlame(
      "/repo-a",
      "second.txt",
      selectionGeneration,
      false,
      () => localeGeneration === 2,
    );
    requests[4]?.resolve("stale first locale blame");
    await firstLocaleRefresh;
    assert.equal(changes.blameText.value, "previous localized blame");
    requests[5]?.resolve("latest locale blame");
    await secondLocaleRefresh;
    assert.equal(changes.blameText.value, "latest locale blame");
    assert.equal(changes.blameLoading.value, false);

    changes.selectedChange.value = change("third.txt");
    const oldRepository = changes.openFileBlame();
    activeRepositoryPath.value = "/repo-b";
    selectionGeneration += 1;
    changes.clearDiffSelection();
    requests[6]?.reject(new Error("stale repository failure"));
    await oldRepository;
    assert.equal(changes.blameModalShown.value, false);
    assert.equal(changes.blameLoading.value, false);
    assert.deepEqual(reportedErrors, []);
  } finally {
    api.getFileBlame = originalGetFileBlame;
  }
});


test("same-path resource refresh renders loading and preserves the chosen comparison", async () => {
  const changes = useRepositoryChanges({
    activeRepositoryPath: ref("/repo"), snapshot: ref(null), currentBranch: computed(() => "main"), commits: ref([]),
    getSelectionGeneration: () => 1, repositorySelectionIsCurrent: () => true,
    applySnapshot: () => true, loadHistory: async () => undefined,
    isChangeFullyIncluded: () => false, isConflictedEntry: () => false,
    splitDiffHunks: () => [], splitDiffLines: () => [], reportError: (error) => assert.fail(String(error)),
    message: {} as never, dialog: {} as never,
  });
  const states: boolean[] = [];
  const stop = watch(changes.diffLoading, (value) => states.push(value), { flush: "post" });
  const entry = { ...change("icon.png"), indexStatus: "M" };
  await changes.loadDiff(entry, "/repo", true, true);
  await nextTick();
  await changes.loadDiff(entry);
  await nextTick();
  assert.deepEqual(states, [true, false, true, false]);
  assert.equal(changes.selectedDiffStaged.value, true);
  await changes.loadDiff(entry, "/repo", true, false);
  assert.equal(changes.selectedDiffStaged.value, false);
  stop();
});
