import assert from "node:assert/strict";
import test from "node:test";
import { api } from "./tauri";
import { mockSnapshot } from "./browserDemoFixtures";

test("demo tag pushes fail explicitly without mutating the branch or tags", async () => {
  const path = mockSnapshot.repository.path;
  const before = await api.refreshRepositorySnapshot(path);
  const remote = before.remotes[0].name;
  await assert.rejects(api.pushRemote(path, remote, true, true), { code: "push_tags_with_force_unsupported" });
  assert.deepEqual(await api.refreshRepositorySnapshot(path), before);
  await assert.rejects(api.pushRemote(path, remote, false, true), { code: "browser_demo_tag_push_unsupported" });
  assert.deepEqual(await api.refreshRepositorySnapshot(path), before);
  await api.pushRemote(path, remote);
});

test("native push IPC preserves opt-in tag and force flags", async (t) => {
  const original = Object.getOwnPropertyDescriptor(globalThis, "window");
  const calls: unknown[] = [];
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      __TAURI_INTERNALS__: {
        invoke: async (command: string, args: unknown) => {
          calls.push({ command, args });
          return { summary: "ok" };
        },
      },
    },
  });
  t.after(() => {
    if (original) Object.defineProperty(globalThis, "window", original);
    else Reflect.deleteProperty(globalThis, "window");
  });
  await api.pushRemote("/repo", "origin");
  await api.pushRemote("/repo", "origin", false, true);
  assert.deepEqual(calls, [
    { command: "push_remote", args: { repositoryPath: "/repo", remoteName: "origin", forceWithLease: false, includeTags: false } },
    { command: "push_remote", args: { repositoryPath: "/repo", remoteName: "origin", forceWithLease: false, includeTags: true } },
  ]);
});
