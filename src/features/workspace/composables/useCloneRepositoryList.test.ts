import assert from "node:assert/strict";
import test from "node:test";
import { effectScope, nextTick, ref } from "vue";
import type { CodeHostRepositoryInfo } from "../../../shared/types/domain";
import { useCloneRepositoryList } from "./useCloneRepositoryList";

const repository = (name: string): CodeHostRepositoryInfo => ({
  fullName: `owner/${name}`, description: null, defaultBranch: "main", webUrl: `https://github.com/owner/${name}`, isPrivate: true,
});
test("account changes and disposal discard old responses and errors", async () => {
  const account = ref<string | null>("first");
  const pending: { resolve: (value: CodeHostRepositoryInfo[]) => void; reject: (error: unknown) => void }[] = [];
  const errors: unknown[] = [];
  const scope = effectScope();
  const list = scope.run(() => useCloneRepositoryList(account, (error) => errors.push(error),
    () => new Promise((resolve, reject) => pending.push({ resolve, reject }))))!;
  account.value = "second";
  pending[0].resolve([repository("old")]);
  await nextTick();
  assert.equal(list.repositories.value.length, 0);
  pending[1].resolve([repository("current")]);
  await nextTick();
  assert.equal(list.repositories.value[0].fullName, "owner/current");
  const refresh = list.loadRepositories(true);
  scope.stop();
  pending[2].reject(new Error("closed"));
  await refresh;
  assert.deepEqual(errors, []);
});

test("pagination retries the failed page, preserves results and deduplicates repositories", async () => {
  const calls: number[] = [];
  const scope = effectScope();
  const list = scope.run(() => useCloneRepositoryList(ref("account"), () => undefined, async (_account, page) => {
    calls.push(page);
    if (calls.length === 2) throw new Error("offline");
    return page === 1 ? Array.from({ length: 100 }, (_, index) => repository(String(index))) : [repository("99"), repository("100")];
  }))!;
  await nextTick();
  assert.equal(list.hasMore.value, true);
  await list.loadRepositories();
  assert.equal(list.failed.value, true);
  assert.equal(list.repositories.value.length, 100);
  await list.loadRepositories();
  assert.deepEqual(calls, [1, 2, 2]);
  assert.equal(list.repositories.value.length, 101);
  assert.equal(list.hasMore.value, false);
  scope.stop();
});
