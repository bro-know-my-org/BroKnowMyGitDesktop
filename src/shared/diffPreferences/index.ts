import { ref } from "vue";

const diffLineWrappingStorageKey = "broknowmygitdesktop.diff-line-wrapping";

function initialDiffLineWrapping(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(diffLineWrappingStorageKey) === "true";
  } catch {
    // Storage availability must not prevent the application from starting.
    return false;
  }
}

export const diffLineWrapping = ref(initialDiffLineWrapping());

export function setDiffLineWrapping(value: boolean): void {
  diffLineWrapping.value = value;
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(diffLineWrappingStorageKey, String(value));
  } catch {
    // Keep the current session preference when storage is unavailable.
  }
}
