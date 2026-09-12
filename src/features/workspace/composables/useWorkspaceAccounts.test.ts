import assert from "node:assert/strict";
import test from "node:test";
import { ref } from "vue";
import { api } from "../../../shared/api/tauri";
import type { AccountProfile } from "../../../shared/types/domain";
import { useWorkspaceAccounts } from "./useWorkspaceAccounts";

const account = (id: string): AccountProfile => ({
  id, endpointId: "github.com", login: id, displayName: null, authMethod: "ghCli",
  secretRef: null, status: "valid", isDefaultCandidate: false,
});
const createAccounts = (supported = true, onRemove?: (remove: () => Promise<void>) => void) => useWorkspaceAccounts({
  mutationBusy: ref(false), capabilityForEndpoint: () => ({ accountImport: supported }),
  reportError: (error: unknown) => { throw error; }, message: { success: () => {} },
  dialog: { warning: (options: { onPositiveClick: () => Promise<void> }) => onRemove?.(options.onPositiveClick) },
} as unknown as Parameters<typeof useWorkspaceAccounts>[0]);

for (const [operation, method] of [
  ["loadAccounts", "listAccounts"], ["refreshAccountStatuses", "refreshAccountStatuses"],
] as const) {
  test(`${operation} preserves detected profiles when an older snapshot completes`, async (context) => {
    let resolve!: (accounts: AccountProfile[]) => void;
    context.mock.method(api, method, () => new Promise<AccountProfile[]>((done) => { resolve = done; }));
    context.mock.method(api, "importCliAccounts", async () => [account("detected")]);
    const state = createAccounts();
    state.accounts.value = [{ ...account("detected"), status: "unavailable" }];
    const pending = state[operation]();
    await state.detectCliAccounts("github.com");
    resolve([account("persisted"), { ...account("detected"), status: "unavailable" }]);
    await pending;
    assert.deepEqual(state.accounts.value.map(({ id, status }) => ({ id, status })), [
      { id: "persisted", status: "valid" }, { id: "detected", status: "valid" },
    ]);
  });
  test(`${operation} cannot resurrect an account added and removed during loading`, async (context) => {
    let resolve!: (accounts: AccountProfile[]) => void;
    let remove!: () => Promise<void>;
    context.mock.method(api, method, () => new Promise<AccountProfile[]>((done) => { resolve = done; }));
    context.mock.method(api, "importCliAccounts", async () => [account("temporary")]);
    context.mock.method(api, "removeAccount", async () => {});
    const state = createAccounts(true, (action) => { remove = action; });
    const pending = state[operation]();
    await state.detectCliAccounts("github.com");
    state.confirmRemoveAccount(state.accounts.value[0]);
    await remove();
    resolve([account("temporary")]);
    await pending;
    assert.deepEqual(state.accounts.value, []);
  });
}

test("CLI detection coalesces concurrent requests, retries failures and skips unsupported endpoints", async (context) => {
  let reject!: (error: unknown) => void;
  const imported = context.mock.method(api, "importCliAccounts", () => new Promise<AccountProfile[]>((_resolve, fail) => { reject = fail; }));
  const state = createAccounts();
  const first = state.detectCliAccounts("github.com");
  assert.equal(first, state.detectCliAccounts("github.com"));
  assert.equal(imported.mock.callCount(), 1);
  reject(new Error("offline"));
  await assert.rejects(first, /offline/u);
  imported.mock.mockImplementation(async () => [account("retry")]);
  assert.equal((await state.detectCliAccounts("github.com"))[0].id, "retry");
  assert.equal(imported.mock.callCount(), 2);
  assert.deepEqual(await createAccounts(false).detectCliAccounts("unsupported:443"), []);
  assert.equal(imported.mock.callCount(), 2);
});
