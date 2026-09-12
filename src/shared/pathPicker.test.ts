import assert from "node:assert/strict";
import test from "node:test";
import { finishPathSelection, normalizePickerPath, pathPickerRequest, selectPath } from "./pathPicker";

test("replacing a path picker cancels its caller and ignores late native results", async () => {
  const first = selectPath({ title: "Folder", directory: true });
  const firstId = pathPickerRequest.value!.id;
  const second = selectPath({ title: "IDE", directory: false });
  const secondId = pathPickerRequest.value!.id;
  assert.equal(await first, null);
  finishPathSelection(firstId, "/stale/path");
  assert.equal(pathPickerRequest.value?.id, secondId);
  finishPathSelection(secondId, "/Applications/Visual Studio Code.app");
  assert.equal(await second, "/Applications/Visual Studio Code.app");
  assert.equal(pathPickerRequest.value, null);
});

test("paths copied with quotes preserve spaces, Unicode, Windows drives and UNC shares", () => {
  assert.equal(normalizePickerPath('  "C:\\Program Files\\Code.exe"  '), "C:\\Program Files\\Code.exe");
  assert.equal(normalizePickerPath('"\\\\server\\share\\工作 目录"'), "\\\\server\\share\\工作 目录");
  assert.equal(normalizePickerPath("'/home/me/工作 目录'"), "/home/me/工作 目录");
  assert.equal(normalizePickerPath("/home/me/O'Brien"), "/home/me/O'Brien");
});
