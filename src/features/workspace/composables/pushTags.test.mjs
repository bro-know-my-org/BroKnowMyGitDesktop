import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { setImmediate } from "node:timers/promises";
import vm from "node:vm";
import test from "node:test";
import ts from "typescript";

// Execute the actual private controller functions with deterministic UI/API
// boundaries, without mounting the desktop window and its repository watchers.
const source = await readFile(new URL("./useWorkspaceController.ts", import.meta.url), "utf8");
const syntax = ts.createSourceFile("controller.ts", source, ts.ScriptTarget.Latest, true);
const names = new Set([
  "runNetworkAction", "reportUnsafeNetworkConfiguration", "approveNetworkConfigurationAndRetry",
]);
const functions = [];
function collect(node) {
  if (ts.isFunctionDeclaration(node) && names.has(node.name?.text)) functions.push(node.getText(syntax));
  ts.forEachChild(node, collect);
}
collect(syntax);
assert.equal(functions.length, names.size);
const script = ts.transpileModule(functions.join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

function fixture(failureCode, fingerprint = "config-version") {
  const calls = [];
  const errors = [];
  const prompts = [];
  const approvals = [];
  let notification;
  let held = false;
  const context = vm.createContext({
    activeRepositoryPath: { value: "/repo" },
    activeRemote: { value: { name: "origin" } },
    activeCapabilities: { value: { repositoryFork: true } },
    snapshot: { value: { status: { branch: "main" } } },
    repositorySelectionGeneration: 1,
    networkOperationGeneration: 0,
    networkBusy: { value: null },
    networkTrustActionBusyOwners: new Set(),
    activeErrorNotification: null,
    activeErrorNotificationOwner: null,
    remoteProfileIdentity: (remote) => remote.name,
    repositorySelectionIsCurrent: () => true,
    networkActionPromptIsCurrent: () => true,
    networkActionAttemptIsCurrent: () => true,
    acquireRepositoryOperation: () => {
      assert.equal(held, false);
      held = true;
      return () => { held = false; };
    },
    t: (key) => key,
    message: { warning() {}, error() {}, success() {}, info() {} },
    isAppError: (error) => Boolean(error?.code),
    normalizedAppError: (error) => error,
    reportError: (error) => errors.push(error),
    refreshSnapshot: async () => {},
    dialog: { warning: (options) => prompts.push(options) },
    notification: { error: (options) => { notification = options; return { destroy() {} }; } },
    destroyOwnedErrorNotification() {},
    NButton: "button",
    NSpace: "space",
    h: (type, props, slots) => ({ type, props, children: slots?.default?.() }),
    api: {
      pushRemote: async (...args) => {
        calls.push(args);
        if (failureCode && calls.length === 1) throw { code: failureCode, message: "Rejected" };
      },
      getRepositoryNetworkSecurity: async () => ({ currentFingerprint: fingerprint }),
      approveRepositoryNetworkConfiguration: async (...args) => { approvals.push(args); },
    },
  });
  vm.runInContext(script, context);
  return { context, calls, errors, prompts, approvals, notification: () => notification };
}

test("ordinary push excludes tags and the explicit tag mode reaches the API", async () => {
  const state = fixture();
  await state.context.runNetworkAction("push");
  await state.context.runNetworkAction("push", false, undefined, true);
  assert.deepEqual(state.calls, [["/repo", "origin", false, false], ["/repo", "origin", false, true]]);
  assert.equal(state.context.networkBusy.value, null);
});

for (const fingerprint of ["config-version", null]) {
  test(`tag mode survives security approval retry (${fingerprint ?? "configuration removed"})`, async () => {
    const state = fixture("unsafe_git_config", fingerprint);
    await state.context.runNetworkAction("push", false, undefined, true);
    const buttons = state.notification().action().children;
    const approve = buttons.find((button) => button.props.type === "primary");
    approve.props.onClick();
    await setImmediate();
    assert.deepEqual(state.errors, []);
    assert.deepEqual(state.calls, [["/repo", "origin", false, true], ["/repo", "origin", false, true]]);
    assert.deepEqual(state.approvals, fingerprint ? [["/repo", fingerprint]] : []);
    assert.equal(state.context.networkBusy.value, null);
    assert.equal(state.context.networkTrustActionBusyOwners.size, 0);
  });
}

test("tag permission failures never offer a fork recovery that would drop tags", async () => {
  const tagged = fixture("git_push_permission_denied");
  await tagged.context.runNetworkAction("push", false, undefined, true);
  assert.equal(tagged.prompts.length, 0);
  assert.equal(tagged.errors[0].code, "git_push_permission_denied");
  const ordinary = fixture("git_push_permission_denied");
  await ordinary.context.runNetworkAction("push");
  assert.equal(ordinary.prompts.length, 1);
  assert.equal(ordinary.errors.length, 0);
});
