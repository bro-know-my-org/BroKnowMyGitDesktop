import { computed, h } from "vue";
import { NText } from "naive-ui";
import { toggleDevtools } from "../../../shared/api/tauri";
import { setAppLocale, t, type AppLocale } from "../../../shared/i18n";
import { setColorMode, setThemeColor, type ColorMode } from "../../../shared/theme";
import { nativePathInputHint } from "../../../shared/nativePathInputHint";

export const localeOptions = [
  { label: "简体中文", value: "zh-CN" },
  { label: "English", value: "en-US" },
];

export function useWorkspaceUiOptions() {
  const colorModeOptions = computed(() => [
    { label: t("workspace.controller.followSystem"), key: "system" },
    { label: t("workspace.controller.light"), key: "light" },
    { label: t("workspace.controller.dark"), key: "dark" },
  ]);
  const repositoryActionOptions = computed(() => [
    { label: t("workspace.controller.scanFolder"), key: "discover" },
    { label: t("workspace.controller.newLocalRepository"), key: "init" },
    { label: t("workspace.controller.cloneRepository"), key: "clone" },
    { type: "divider" as const, key: "path-hint-divider" },
    {
      type: "render" as const,
      key: "path-hint",
      render: () => h(NText, {
        depth: 3,
        tag: "div",
        class: "repository-path-input-hint",
      }, { default: nativePathInputHint }),
    },
  ]);
  return { colorModeOptions, repositoryActionOptions };
}

export function updateAppLocale(value: string | number): void {
  if (value === "zh-CN" || value === "en-US") setAppLocale(value as AppLocale);
}

export function updateColorMode(value: string | number): void {
  if (value === "system" || value === "light" || value === "dark") setColorMode(value as ColorMode);
}

export function updateThemeColor(value: string | null): void {
  if (value) setThemeColor(value);
}

export function handleDevtoolsShortcut(event: KeyboardEvent): void {
  if (event.key !== "F12") return;
  event.preventDefault();
  void toggleDevtools().catch(() => undefined);
}
