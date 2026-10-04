import { computed, ref, watch } from "vue";
import { setTheme as setTauriTheme } from "@tauri-apps/api/app";

export type ColorMode = "system" | "light" | "dark";

const colorModeStorageKey = "broknowmygitdesktop.color-mode";
const themeColorStorageKey = "broknowmygitdesktop.theme-color";
export const defaultThemeColor = "#0A84FFFF";

interface RgbaColor {
  red: number;
  green: number;
  blue: number;
  alpha: number;
}

function parseHexColor(value: string): RgbaColor | null {
  const match = /^#([\da-f]{6})([\da-f]{2})?$/i.exec(value.trim());
  if (!match) return null;
  const rgb = match[1];
  return {
    red: Number.parseInt(rgb.slice(0, 2), 16),
    green: Number.parseInt(rgb.slice(2, 4), 16),
    blue: Number.parseInt(rgb.slice(4, 6), 16),
    alpha: match[2] ? Number.parseInt(match[2], 16) / 255 : 1,
  };
}

function toHex(color: RgbaColor): string {
  const channel = (value: number) => Math.round(value).toString(16).padStart(2, "0");
  return `#${channel(color.red)}${channel(color.green)}${channel(color.blue)}`.toUpperCase();
}

function toHexa(color: RgbaColor): string {
  return `${toHex(color)}${Math.round(color.alpha * 255).toString(16).padStart(2, "0")}`.toUpperCase();
}

function mix(color: RgbaColor, target: number, amount: number): RgbaColor {
  return {
    red: color.red + (target - color.red) * amount,
    green: color.green + (target - color.green) * amount,
    blue: color.blue + (target - color.blue) * amount,
    alpha: color.alpha,
  };
}

function relativeLuminance(color: RgbaColor): number {
  const linearChannel = (value: number) => {
    const channel = value / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linearChannel(color.red) + 0.7152 * linearChannel(color.green) + 0.0722 * linearChannel(color.blue);
}

function contrastRatio(left: RgbaColor, right: RgbaColor): number {
  const lighter = Math.max(relativeLuminance(left), relativeLuminance(right));
  const darker = Math.min(relativeLuminance(left), relativeLuminance(right));
  return (lighter + 0.05) / (darker + 0.05);
}

function ensureContrast(color: RgbaColor, background: RgbaColor, target: number, minimumRatio = 4.5): RgbaColor {
  if (contrastRatio(color, background) >= minimumRatio) return color;
  let insufficientAmount = 0;
  let sufficientAmount = 1;
  for (let iteration = 0; iteration < 12; iteration += 1) {
    const amount = (insufficientAmount + sufficientAmount) / 2;
    if (contrastRatio(mix(color, target, amount), background) >= minimumRatio) sufficientAmount = amount;
    else insufficientAmount = amount;
  }
  return mix(color, target, sufficientAmount);
}

function readableForeground(color: RgbaColor): string {
  const black: RgbaColor = { red: 0, green: 0, blue: 0, alpha: 1 };
  const white: RgbaColor = { red: 255, green: 255, blue: 255, alpha: 1 };
  return contrastRatio(color, black) >= contrastRatio(color, white) ? "#000000" : "#FFFFFF";
}

function composite(foreground: RgbaColor, background: RgbaColor, opacity: number): RgbaColor {
  return {
    red: foreground.red * opacity + background.red * (1 - opacity),
    green: foreground.green * opacity + background.green * (1 - opacity),
    blue: foreground.blue * opacity + background.blue * (1 - opacity),
    alpha: 1,
  };
}

function rgba(color: RgbaColor, opacity: number): string {
  return `rgba(${Math.round(color.red)}, ${Math.round(color.green)}, ${Math.round(color.blue)}, ${Math.max(0, Math.min(1, opacity)).toFixed(3)})`;
}

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

function initialThemeColor(): string {
  if (typeof window === "undefined") return defaultThemeColor;
  try {
    const saved = window.localStorage.getItem(themeColorStorageKey);
    const parsed = saved ? parseHexColor(saved) : null;
    if (parsed) return toHexa(parsed);
  } catch {
    // A disabled storage area must not prevent the UI from starting.
  }
  return defaultThemeColor;
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
export const themeColor = ref(initialThemeColor());
export const themePalette = computed(() => {
  const selected = parseHexColor(themeColor.value) ?? parseHexColor(defaultThemeColor)!;
  const surface: RgbaColor = isDarkTheme.value
    ? { red: 44, green: 44, blue: 46, alpha: 1 }
    : { red: 255, green: 255, blue: 255, alpha: 1 };
  const contrastTarget = isDarkTheme.value ? 255 : 0;
  const softAlpha = selected.alpha;
  const selectedOpacity = isDarkTheme.value ? 0.08 + softAlpha * 0.12 : 0.04 + softAlpha * 0.05;
  const borderOpacity = isDarkTheme.value ? 0.25 + softAlpha * 0.25 : 0.18 + softAlpha * 0.12;
  const focusOpacity = isDarkTheme.value ? 0.18 + softAlpha * 0.12 : 0.12 + softAlpha * 0.08;
  const adapted = isDarkTheme.value ? mix(selected, 255, 0.36) : selected;
  let primary = ensureContrast(adapted, surface, contrastTarget);
  // Selected rows tint the surface with the primary color. Re-check against
  // that real composite so their accent text keeps the same WCAG contrast.
  for (let iteration = 0; iteration < 4; iteration += 1) {
    primary = ensureContrast(primary, composite(primary, surface, selectedOpacity), contrastTarget);
  }
  const hover = ensureContrast(mix(primary, 255, isDarkTheme.value ? 0.16 : 0.13), surface, contrastTarget);
  const pressed = ensureContrast(mix(primary, 0, isDarkTheme.value ? 0.12 : 0.16), surface, contrastTarget);
  return {
    primary: toHex(primary),
    hover: toHex(hover),
    pressed: toHex(pressed),
    primaryText: readableForeground(primary),
    hoverText: readableForeground(hover),
    pressedText: readableForeground(pressed),
    selectedBackground: rgba(primary, selectedOpacity),
    selectedBorder: rgba(primary, borderOpacity),
    focusRing: rgba(primary, focusOpacity),
  };
});

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

watch(themePalette, (palette) => {
  if (typeof document === "undefined") return;
  const root = document.documentElement.style;
  root.setProperty("--app-accent", palette.primary);
  root.setProperty("--app-accent-strong", palette.primary);
  root.setProperty("--app-accent-bright", palette.hover);
  root.setProperty("--app-on-accent", palette.primaryText);
  root.setProperty("--app-row-selected", palette.selectedBackground);
  root.setProperty("--app-selected-border", palette.selectedBorder);
  root.setProperty("--app-focus", palette.primary);
  root.setProperty("--app-focus-ring", palette.focusRing);
}, { immediate: true });

export function setColorMode(mode: ColorMode): void {
  colorMode.value = mode;
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(colorModeStorageKey, mode);
  } catch {
    // The current session can still use the selected theme if storage is unavailable.
  }
}

export function setThemeColor(value: string): void {
  const parsed = parseHexColor(value);
  if (!parsed) return;
  const normalized = toHexa(parsed);
  themeColor.value = normalized;
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(themeColorStorageKey, normalized);
  } catch {
    // The current session can still use the selected color if storage is unavailable.
  }
}

export function resetThemeColor(): void {
  setThemeColor(defaultThemeColor);
}
