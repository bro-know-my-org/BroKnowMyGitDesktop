import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const componentUrl = new URL("./WorkspaceHeader.vue", import.meta.url);
const dialogsUrl = new URL("./WorkspaceDialogs.vue", import.meta.url);
const tauriApiUrl = new URL("../../../shared/api/tauri.ts", import.meta.url);
const stylesheetUrl = new URL("./WorkspaceHeader.css", import.meta.url);

test("repository switcher keeps one accessible trigger for every interaction", async () => {
  const componentSource = await readFile(componentUrl, "utf8");
  const trigger = componentSource.match(
    /<template #trigger>([\s\S]*?)<\/template>/u,
  )?.[1];

  assert.ok(trigger, "repository picker trigger template is present");
  assert.equal(trigger.match(/<button\b/gu)?.length, 1);
  assert.match(trigger, /class="repository-switcher"/u);
  assert.match(trigger, /:aria-label="repositorySwitcherAriaLabel"/u);
  assert.match(trigger, /:aria-expanded="repositoryPickerOpen"/u);
  assert.match(trigger, /aria-haspopup="dialog"/u);
  assert.match(trigger, /aria-keyshortcuts="Shift\+F10"/u);
  assert.match(trigger, /@click\.stop="toggleRepositoryPicker\(\$event\)"/u);
  assert.match(trigger, /@contextmenu="[^"]*openRepositoryContextMenu/u);
  assert.match(
    trigger,
    /@keydown="[^"]*openRepositoryContextMenuFromKeyboard/u,
  );
  assert.match(trigger, /class="repository-switcher-caret"/u);
  assert.doesNotMatch(
    componentSource,
    /repository-switcher-(?:split|current|toggle)/u,
  );
});

test("repository switcher styles follow explicit state and preserve owner casing", async () => {
  const [componentSource, stylesheetSource] = await Promise.all([
    readFile(componentUrl, "utf8"),
    readFile(stylesheetUrl, "utf8"),
  ]);

  assert.match(
    componentSource,
    /'sidebar-group-heading-owner': item\.group\.key\.startsWith\('owner:'\)/u,
  );
  assert.match(
    stylesheetSource,
    /\.repository-switcher\[aria-expanded="true"\]/u,
  );
  assert.match(
    stylesheetSource,
    /\.sidebar-group-heading-owner\s*\{\s*text-transform: none;/u,
  );
  assert.doesNotMatch(stylesheetSource, /:has\(/u);
});

test("repository context menu lists configured IDEs and opens the selected one", async () => {
  const componentSource = await readFile(componentUrl, "utf8");

  assert.match(componentSource, /workspace\.repositoryEditors\.value\.map/u);
  assert.match(componentSource, /key: `open-repository-editor:\$\{editor\.name\}`/u);
  assert.match(componentSource, /api\.openRepositoryInEditor\(repository\.path, editor\.name\)/u);
  assert.match(componentSource, /workspace\.openRepositoryEditorsModal\(\)/u);
});

test("repository IDE manager exposes add, edit, and remove actions", async () => {
  const dialogsSource = await readFile(dialogsUrl, "utf8");

  assert.match(dialogsSource, /v-model:show="repositoryEditorsModalShown"/u);
  assert.match(dialogsSource, /v-for="\(editor, index\) in repositoryEditors"/u);
  assert.match(dialogsSource, /@click="editRepositoryEditor\(index\)"/u);
  assert.match(dialogsSource, /@click="confirmRemoveRepositoryEditor\(index\)"/u);
  assert.match(dialogsSource, /@click="saveRepositoryEditor"/u);
  assert.match(dialogsSource, /@click="selectRepositoryEditorApplication"/u);
});

test("repository IDE manager uses the Linux application registry with a native-file fallback", async () => {
  const [apiSource, dialogsSource] = await Promise.all([
    readFile(tauriApiUrl, "utf8"),
    readFile(dialogsUrl, "utf8"),
  ]);

  assert.match(apiSource, /listInstalledApplications\(\)/u);
  assert.match(apiSource, /request\("list_installed_applications"/u);
  assert.match(dialogsSource, /v-model:show="installedApplicationPickerShown"/u);
  assert.match(apiSource, /selectApplicationExecutable\(title: string[,)]/u);
  assert.match(apiSource, /openDialog\(\{[\s\S]*?directory: false,[\s\S]*?multiple: false,/u);
  assert.match(apiSource, /extensions: \["exe", "com"\]/u);
});
