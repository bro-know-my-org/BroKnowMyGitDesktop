import assert from "node:assert/strict";
import test from "node:test";
import { ref } from "vue";
import type { RepositoryRecord } from "../../../shared/types/domain";
import { useRepositoryCatalog } from "./useRepositoryCatalog";

function repository(path: string): RepositoryRecord {
  return {
    path,
    name: path.split("/").filter(Boolean).pop() ?? "repository",
    alias: null,
    group: null,
    host: null,
    namespace: null,
    lastOpenedAt: null,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

function catalogHarness(
  listRepositories: (refreshOwners: boolean) => Promise<RepositoryRecord[]>,
) {
  const errors: string[] = [];
  const catalog = useRepositoryCatalog({
    snapshot: ref(null),
    mutationBusy: ref(false),
    listRepositories,
    reportError: (_error, action) => errors.push(action),
    showSuccess: () => undefined,
  });
  return { catalog, errors };
}

test("overlapping catalog loads coalesce and escalate owner refresh", async () => {
  const first = deferred<RepositoryRecord[]>();
  const second = deferred<RepositoryRecord[]>();
  const refreshOwners: boolean[] = [];
  const requests = [first, second];
  const { catalog } = catalogHarness(async (refreshOwnerRecords) => {
    refreshOwners.push(refreshOwnerRecords);
    return requests[refreshOwners.length - 1].promise;
  });

  const firstLoad = catalog.loadRepositories(false);
  await Promise.resolve();
  const secondLoad = catalog.loadRepositories(true);
  first.resolve([repository("/projects/stale")]);
  await Promise.resolve();
  second.resolve([repository("/projects/current")]);

  assert.equal(await firstLoad, true);
  assert.equal(await secondLoad, true);
  assert.deepEqual(refreshOwners, [false, true]);
  assert.deepEqual(catalog.repositories.value.map((item) => item.path), ["/projects/current"]);
});

test("a local catalog mutation during loading cannot be overwritten by a stale result", async () => {
  const first = deferred<RepositoryRecord[]>();
  const second = deferred<RepositoryRecord[]>();
  let requestCount = 0;
  const local = repository("/projects/local");
  const { catalog } = catalogHarness(async () => {
    requestCount += 1;
    return requestCount === 1 ? first.promise : second.promise;
  });

  const loading = catalog.loadRepositories(false);
  await Promise.resolve();
  catalog.replaceRepositoryRecord(local);
  first.resolve([repository("/projects/stale")]);
  await Promise.resolve();
  second.resolve([local]);

  assert.equal(await loading, true);
  assert.equal(requestCount, 2);
  assert.deepEqual(catalog.repositories.value, [local]);
});

test("a failed catalog request runs an already queued retry", async () => {
  const first = deferred<RepositoryRecord[]>();
  const second = deferred<RepositoryRecord[]>();
  let requestCount = 0;
  const current = repository("/projects/current");
  const { catalog, errors } = catalogHarness(async () => {
    requestCount += 1;
    return requestCount === 1 ? first.promise : second.promise;
  });

  const firstLoad = catalog.loadRepositories(false);
  await Promise.resolve();
  const queuedLoad = catalog.loadRepositories(false);
  first.reject(new Error("temporary failure"));
  await Promise.resolve();
  second.resolve([current]);

  assert.equal(await firstLoad, true);
  assert.equal(await queuedLoad, true);
  assert.equal(requestCount, 2);
  assert.deepEqual(errors, []);
  assert.deepEqual(catalog.repositories.value, [current]);
});
