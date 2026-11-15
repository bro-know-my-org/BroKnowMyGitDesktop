import assert from "node:assert/strict";
import test from "node:test";
import { nextTick, ref } from "vue";
import { api } from "../../../shared/api/tauri";
import { useRepositoryHistory } from "./useRepositoryHistory";
import type { CommitSummary } from "../../../shared/types/domain";

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

test("history loads beyond 100 commits, retries failed pages and rejects stale repository pages", async () => {
  const original = api.listCommits;
  const requests: { offset: number; head?: string; result: Deferred<CommitSummary[]> }[] = [];
  api.listCommits = async (_path, _limit, offset = 0, head) => {
    const result = deferred<CommitSummary[]>();
    requests.push({ offset, head, result });
    return result.promise;
  };
  const activeRepositoryPath = ref<string | null>("/repo-a");
  let generation = 1;
  const history = useRepositoryHistory({
    activeRepositoryPath, snapshot: ref(null), getSelectionGeneration: () => generation,
    repositorySelectionIsCurrent: (path, value) => path === activeRepositoryPath.value && value === generation,
    isControllerMounted: () => true, reportError: () => {},
  });
  const commits = Array.from({ length: 149 }, (_, index) => ({ sha: `sha-${index}` } as CommitSummary));
  try {
    const first = history.loadCommitHistoryWithOptions();
    requests[0]!.result.resolve(commits.slice(0, 100));
    await first;
    assert.equal(history.historyHasMore.value, true);
    const failed = history.loadMoreHistory();
    assert.equal(requests[1]!.offset, 100);
    assert.equal(requests[1]!.head, "sha-0");
    await history.loadMoreHistory();
    assert.equal(requests.length, 2, "scroll events cannot duplicate a pending page");
    requests[1]!.result.reject(new Error("temporary failure"));
    await failed;
    assert.equal(history.commits.value.length, 100);
    const retry = history.loadMoreHistory();
    requests[2]!.result.resolve(commits.slice(100));
    await retry;
    assert.equal(history.commits.value.length, 149);
    assert.equal(history.historyHasMore.value, false);
    const refresh = history.loadCommitHistoryWithOptions({ force: true, resetPagination: true });
    requests[3]!.result.resolve(commits.slice(0, 100));
    await refresh;
    const superseded = history.loadMoreHistory();
    const newerRefresh = history.loadCommitHistoryWithOptions({ force: true, resetPagination: true });
    requests[5]!.result.resolve(commits.slice(0, 100));
    await newerRefresh;
    requests[4]!.result.resolve(commits.slice(100));
    await superseded;
    assert.equal(history.commits.value.length, 100, "a page cannot overwrite a newer refresh");
    const stale = history.loadMoreHistory();
    activeRepositoryPath.value = "/repo-b";
    generation += 1;
    history.resetRepositoryHistory();
    requests[6]!.result.resolve(commits.slice(100));
    await stale;
    assert.equal(history.commits.value.length, 0);
    assert.equal(history.historyLoadingMore.value, false);
  } finally {
    api.listCommits = original;
    history.disposeRepositoryHistory();
  }
});

test("background refresh preserves loaded history and the selected older commit", async () => {
  const original = api.listCommits;
  const commits = Array.from({ length: 149 }, (_, index) => ({ sha: `sha-${index}` } as CommitSummary));
  const offsets: number[] = [];
  const limits: number[] = [];
  api.listCommits = async (_path, limit = 100, offset = 0) => {
    offsets.push(offset);
    limits.push(limit);
    return commits.slice(offset, offset + limit);
  };
  const history = useRepositoryHistory({
    activeRepositoryPath: ref("/repo-a"), snapshot: ref(null), getSelectionGeneration: () => 1,
    repositorySelectionIsCurrent: () => true, isControllerMounted: () => true,
    reportError: (error) => { throw error; },
  });
  try {
    await history.loadCommitHistoryWithOptions();
    await history.loadMoreHistory();
    history.selectedCommit.value = commits[148]!;
    history.invalidateCommitHistoryCache();
    await history.loadCommitHistoryWithOptions({ force: true, reportErrors: false });
    assert.equal(history.commits.value.length, 149);
    assert.equal(history.selectedCommit.value?.sha, "sha-148");
    assert.equal(history.historyHasMore.value, false);
    assert.deepEqual(offsets, [0, 100, 0]);
    assert.deepEqual(limits, [100, 100, 150]);
    await history.loadCommitHistoryWithOptions({ force: true, resetPagination: true });
    assert.equal(history.commits.value.length, 100);
  } finally {
    api.listCommits = original;
    history.disposeRepositoryHistory();
  }
});

test("retained history refresh falls back to bounded pages after an output budget failure", async () => {
  const original = api.listCommits;
  const commits = Array.from({ length: 3000 }, (_, index) => ({ sha: `sha-${index}`, summary: "x".repeat(2048) } as CommitSummary));
  const requests: { limit: number; offset: number; head?: string }[] = [];
  api.listCommits = async (_path, limit = 100, offset = 0, head) => {
    requests.push({ limit, offset, head });
    if (limit > 100) throw { code: "git_output_too_large", message: "output budget exceeded" };
    return commits.slice(offset, offset + limit);
  };
  const history = useRepositoryHistory({
    activeRepositoryPath: ref("/repo-a"), snapshot: ref(null), getSelectionGeneration: () => 1,
    repositorySelectionIsCurrent: () => true, isControllerMounted: () => true,
    reportError: (error) => { throw error; },
  });
  try {
    await history.loadCommitHistoryWithOptions();
    for (let page = 1; page < 30; page += 1) await history.loadMoreHistory();
    history.selectedCommit.value = commits[2999]!;
    const refreshStart = requests.length;
    await history.loadCommitHistoryWithOptions({ force: true });
    assert.equal(requests[refreshStart]?.limit, 3001);
    assert.ok(requests.slice(refreshStart + 1).every((request) => request.limit <= 100));
    assert.ok(requests.slice(refreshStart + 2).every((request) => request.head === "sha-0"));
    assert.equal(history.commits.value.length, 3000);
    assert.equal(history.selectedCommit.value?.sha, "sha-2999");
    assert.equal(history.historyHasMore.value, false);
    assert.equal(history.historyLoading.value, false);
  } finally {
    api.listCommits = original;
    history.disposeRepositoryHistory();
  }
});

test("reflog requests ignore closed and stale owners and preserve text after silent failures", async () => {
  const originalListReflog = api.listReflog;
  const requests: Deferred<string>[] = [];
  api.listReflog = async () => {
    const request = deferred<string>();
    requests.push(request);
    return request.promise;
  };

  const activeRepositoryPath = ref<string | null>("/repo-a");
  let selectionGeneration = 1;
  const reportedErrors: unknown[] = [];
  const history = useRepositoryHistory({
    activeRepositoryPath,
    snapshot: ref(null),
    getSelectionGeneration: () => selectionGeneration,
    repositorySelectionIsCurrent: (path, generation) =>
      path === activeRepositoryPath.value && generation === selectionGeneration,
    isControllerMounted: () => true,
    reportError: (error) => reportedErrors.push(error),
  });

  try {
    const closed = history.openReflog();
    assert.equal(history.reflogLoading.value, true);
    history.reflogModalShown.value = false;
    await nextTick();
    requests[0]?.resolve("stale closed reflog");
    await closed;
    assert.equal(history.reflogText.value, "");
    assert.equal(history.reflogLoading.value, false);

    history.reflogModalShown.value = true;
    history.reflogText.value = "previous localized reflog";
    const silentRefresh = history.loadReflog("/repo-a", selectionGeneration, false);
    requests[1]?.reject(new Error("silent locale refresh failure"));
    await silentRefresh;
    assert.equal(history.reflogText.value, "previous localized reflog");
    assert.equal(history.reflogModalShown.value, true);
    assert.equal(history.reflogLoading.value, false);

    let localeGeneration = 1;
    const firstLocaleRefresh = history.loadReflog(
      "/repo-a",
      selectionGeneration,
      false,
      () => localeGeneration === 1,
    );
    localeGeneration = 2;
    const secondLocaleRefresh = history.loadReflog(
      "/repo-a",
      selectionGeneration,
      false,
      () => localeGeneration === 2,
    );
    requests[2]?.resolve("stale first locale reflog");
    await firstLocaleRefresh;
    assert.equal(history.reflogText.value, "previous localized reflog");
    requests[3]?.resolve("latest locale reflog");
    await secondLocaleRefresh;
    assert.equal(history.reflogText.value, "latest locale reflog");
    assert.equal(history.reflogLoading.value, false);

    const staleRepository = history.loadReflog("/repo-a", selectionGeneration);
    activeRepositoryPath.value = "/repo-b";
    selectionGeneration += 1;
    history.resetRepositoryHistory();
    requests[4]?.reject(new Error("stale repository failure"));
    await staleRepository;
    assert.equal(history.reflogModalShown.value, false);
    assert.equal(history.reflogLoading.value, false);
    assert.deepEqual(reportedErrors, []);
  } finally {
    api.listReflog = originalListReflog;
  }
});
