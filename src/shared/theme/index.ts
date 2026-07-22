import { computed, ref, watch } from "vue";
import { setTheme as setTauriTheme } from "@tauri-apps/api/app";

export type ColorMode = "system" | "light" | "dark";

const colorModeStorageKey = "broknowmygitdesktop.color-mode";

function initialColorMode(): ColorMode {
  if (typeof window === "undefined") return "system";
  try {
    const saved = window.localStorage.getItem(colorModeStorageKey);
    if (saved === "system" || saved === "light" || saved === "dark") return saved;
  } catch {
    // A disabled storage area must not prevent the UI from starting.
  }
  return "system";
}

const systemColorScheme = typeof window === "undefined" || typeof window.matchMedia !== "function"
  ? null
  : window.matchMedia("(prefers-color-scheme: dark)");
const systemPrefersDark = ref(systemColorScheme?.matches ?? false);
let nativeThemeUpdate = Promise.resolve();

systemColorScheme?.addEventListener("change", (event) => {
  systemPrefersDark.value = event.matches;
});

export const colorMode = ref<ColorMode>(initialColorMode());
export const isDarkTheme = computed(() => colorMode.value === "dark" || (colorMode.value === "system" && systemPrefersDark.value));

function applyDocumentTheme(isDark: boolean): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = isDark ? "dark" : "light";
  document.documentElement.style.colorScheme = isDark ? "dark" : "light";
  if (typeof window !== "undefined" && ("__TAURI_INTERNALS__" in window || "__TAURI__" in window)) {
    const theme = colorMode.value === "system" ? null : isDark ? "dark" : "light";
    // Native IPC is asynchronous. Queue updates so a slow older request can
    // never finish after and overwrite a newer color-mode selection.
    nativeThemeUpdate = nativeThemeUpdate
      .then(() => setTauriTheme(theme))
      .catch((error: unknown) => {
        // Keep the web theme usable, but make a native title-bar mismatch
        // diagnosable instead of silently claiming synchronization succeeded.
        console.error("Failed to synchronize the native window theme.", error);
      });
  }
}

// A system-mode change can leave the resolved boolean unchanged (for example
// dark system → explicit dark). Watch the selected mode as well so the native
// window theme stays in sync with the document theme.
watch([isDarkTheme, colorMode], ([isDark]) => applyDocumentTheme(isDark), { immediate: true });

export function setColorMode(mode: ColorMode): void {
  colorMode.value = mode;
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(colorModeStorageKey, mode);
  } catch {
    // The current session can still use the selected theme if storage is unavailable.
  }
}

