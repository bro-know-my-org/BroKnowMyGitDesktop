import { t } from "./i18n";

export function nativePathInputHint(): string {
  const platform = typeof navigator === "undefined" ? "" : navigator.platform;
  if (/^Mac/u.test(platform)) return t("workspace.pathPicker.nativeHintMac");
  if (/^Win/u.test(platform)) return t("workspace.pathPicker.nativeHintWindows");
  return t("workspace.pathPicker.nativeHintLinux");
}
