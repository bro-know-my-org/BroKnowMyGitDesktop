import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const componentUrl = new URL("./WorkspaceHeader.vue", import.meta.url);
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
    /'sidebar-group-heading-owner': group\.key\.startsWith\('owner:'\)/u,
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
