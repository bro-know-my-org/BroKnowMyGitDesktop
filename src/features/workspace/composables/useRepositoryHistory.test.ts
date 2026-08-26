import assert from "node:assert/strict";
import test from "node:test";
import { nextTick, ref } from "vue";
import { api } from "../../../shared/api/tauri";
import { useRepositoryHistory } from "./useRepositoryHistory";

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
