import assert from "node:assert/strict";
import test from "node:test";
import { ref } from "vue";
import type { RepositoryRecord } from "../../../shared/types/domain";
import { useRepositoryPickerPreferences } from "./useRepositoryPickerPreferences";

const repository = (path: string): RepositoryRecord => ({
  path, name: path, alias: null, group: null, host: null, namespace: null, lastOpenedAt: null,
});
const groups = () => ref([
  { key: "owner:team", label: "team", repositories: [repository("/a"), repository("/b")] },
  { key: "manual:tools", label: "tools", repositories: [repository("/c")] },
]);

test("stars pin globally without duplication; hidden stars can be recovered in show-all mode", () => {
  const picker = useRepositoryPickerPreferences(groups(), null);
  picker.toggleStar("/c");
  picker.toggleHidden("/c");
  assert.deepEqual(picker.pickerGroups.value.map((group) => group.key), ["owner:team"]);
  picker.showAllRepositories.value = true;
  assert.deepEqual(picker.pickerGroups.value.map((group) => group.key), ["starred", "owner:team"]);
  assert.deepEqual(picker.pickerGroups.value.flatMap((group) => group.repositories.map((r) => r.path)), ["/c", "/a", "/b"]);
  picker.toggleHidden("/c");
  picker.showAllRepositories.value = false;
  assert.equal(picker.pickerGroups.value[0].repositories[0].path, "/c");
  picker.toggleStar("/c");
  assert.equal(picker.pickerGroups.value[1].key, "manual:tools");
});

test("preferences survive recreation and group modes have independent collapse keys", () => {
  const values = new Map<string, string>();
  const storage = { getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); } };
  const first = useRepositoryPickerPreferences(groups(), storage);
  first.toggleStar("/b");
  first.toggleHidden("/a");
  first.toggleCollapsed("owner:team");
  const restored = useRepositoryPickerPreferences(groups(), storage);
  assert.equal(restored.isStarred("/b"), true);
  assert.equal(restored.isHidden("/a"), true);
  assert.equal(restored.isCollapsed("owner:team"), true);
  assert.equal(restored.isCollapsed("manual:team"), false);
  assert.equal(restored.showAllRepositories.value, false);
});

test("corrupt and disabled storage cannot prevent picker interactions", () => {
  const corrupt = useRepositoryPickerPreferences(groups(), { getItem: () => '{"starred":[null,42,"/a","/a"]}', setItem() {} });
  assert.deepEqual(corrupt.pickerGroups.value[0].repositories.map((r) => r.path), ["/a"]);
  const disabled = useRepositoryPickerPreferences(groups(), { getItem() { throw Error("disabled"); }, setItem() { throw Error("disabled"); } });
  disabled.toggleHidden("/b");
  assert.equal(disabled.isHidden("/b"), true);
});
