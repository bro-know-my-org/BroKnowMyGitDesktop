import { computed, ref, watch } from "vue";
import { request } from "../api/tauriRuntime";

export interface SystemFontFamily {
  family: string;
  monospaced: boolean;
}

interface SystemFontPreferences {
  interfaceFamily: string | null;
  codeFamily: string | null;
  interfaceSize: number | null;
  codeSize: number | null;
  interfaceWeight: number | null;
  codeWeight: number | null;
}

export const systemFontSelection = "__system__";

const interfaceFontStorageKey = "broknowmygitdesktop.interface-font";
const codeFontStorageKey = "broknowmygitdesktop.code-font";
const interfaceFontSizeStorageKey = "broknowmygitdesktop.interface-font-size";
const codeFontSizeStorageKey = "broknowmygitdesktop.code-font-size";
const interfaceFontWeightStorageKey = "broknowmygitdesktop.interface-font-weight";
const codeFontWeightStorageKey = "broknowmygitdesktop.code-font-weight";
const codeLigaturesStorageKey = "broknowmygitdesktop.code-ligatures";
const fallbackInterfaceFontStack = "system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif";
const fallbackCodeFontStack = "ui-monospace, SFMono-Regular, Menlo, Monaco, \"Cascadia Mono\", Consolas, \"Liberation Mono\", monospace";

function isValidFontValue(value: string): boolean {
  return value.trim().length > 0 && value.length <= 200 && !/[\u0000-\u001F\u007F-\u009F]/u.test(value);
}

function initialFont(storageKey: string): string {
  if (typeof window === "undefined") return systemFontSelection;
  try {
    const saved = window.localStorage.getItem(storageKey)?.trim();
    if (saved && isValidFontValue(saved)) return saved;
  } catch {
    // Storage availability must not prevent the application from starting.
  }
  return systemFontSelection;
}

function initialNumericPreference(storageKey: string, minimum: number, maximum: number, step: number): string {
  const saved = initialFont(storageKey);
  if (saved === systemFontSelection) return saved;
  const value = Number(saved);
  return Number.isInteger(value) && value >= minimum && value <= maximum && (value - minimum) % step === 0
    ? saved
    : systemFontSelection;
}

function initialCodeLigatures(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(codeLigaturesStorageKey) === "true";
  } catch {
    return false;
  }
}

function persist(storageKey: string, value: string): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(storageKey, value);
  } catch {
    // Keep the current session usable when storage is unavailable.
  }
}

function quoteFontFamily(family: string): string {
  return `"${family.replace(/\\/g, "\\\\").replace(/"/g, "\\\"")}"`;
}

function selectedFontStack(selected: string, fallback: string): string {
  return selected === systemFontSelection ? fallback : `${quoteFontFamily(selected)}, ${fallback}`;
}

function systemFontStack(selected: string | null, fallback: string): string {
  if (!selected || !isValidFontValue(selected)) return fallback;
  if (selected === "-apple-system" || selected === "system-ui" || selected === "ui-monospace") {
    return `${selected}, ${fallback}`;
  }
  return `${quoteFontFamily(selected)}, ${fallback}`;
}

export const interfaceFont = ref(initialFont(interfaceFontStorageKey));
export const codeFont = ref(initialFont(codeFontStorageKey));
export const interfaceFontSize = ref(initialNumericPreference(interfaceFontSizeStorageKey, 10, 24, 1));
export const codeFontSize = ref(initialNumericPreference(codeFontSizeStorageKey, 9, 24, 1));
export const interfaceFontWeight = ref(initialNumericPreference(interfaceFontWeightStorageKey, 100, 900, 100));
export const codeFontWeight = ref(initialNumericPreference(codeFontWeightStorageKey, 100, 900, 100));
export const codeLigatures = ref(initialCodeLigatures());
export const systemFonts = ref<SystemFontFamily[]>([]);
export const systemFontsLoading = ref(false);
export const systemInterfaceFontName = ref<string | null>(null);
export const systemCodeFontName = ref<string | null>(null);
export const systemInterfaceFontSize = ref<number | null>(null);
export const systemCodeFontSize = ref<number | null>(null);
export const systemInterfaceFontWeight = ref<number | null>(null);
export const systemCodeFontWeight = ref<number | null>(null);
const systemInterfaceFontStack = computed(() => systemFontStack(systemInterfaceFontName.value, fallbackInterfaceFontStack));
const systemCodeFontStack = computed(() => systemFontStack(systemCodeFontName.value, fallbackCodeFontStack));
export const interfaceFontFamily = computed(() => selectedFontStack(interfaceFont.value, systemInterfaceFontStack.value));
export const codeFontFamily = computed(() => selectedFontStack(codeFont.value, systemCodeFontStack.value));
function resolvedNumber(selected: string, systemValue: number | null, fallback: number, minimum: number, maximum: number): number {
  const value = selected === systemFontSelection ? systemValue : Number(selected);
  return Math.min(maximum, Math.max(minimum, Number.isFinite(value) && value !== null ? value : fallback));
}
export const resolvedInterfaceFontSize = computed(() => resolvedNumber(interfaceFontSize.value, systemInterfaceFontSize.value, 14, 10, 24));
export const resolvedCodeFontSize = computed(() => resolvedNumber(codeFontSize.value, systemCodeFontSize.value, 12, 9, 24));
export const resolvedDiffLineHeight = computed(() => resolvedCodeFontSize.value * 1.62);
export const resolvedInterfaceFontWeight = computed(() => resolvedNumber(interfaceFontWeight.value, systemInterfaceFontWeight.value, 400, 100, 900));
export const resolvedCodeFontWeight = computed(() => resolvedNumber(codeFontWeight.value, systemCodeFontWeight.value, 400, 100, 900));

let systemFontsRequest: Promise<void> | null = null;
let systemFontPreferencesRequest: Promise<void> | null = null;

export function loadSystemFontPreferences(): Promise<void> {
  if (systemFontPreferencesRequest) return systemFontPreferencesRequest;
  systemFontPreferencesRequest = request<SystemFontPreferences>("get_system_font_preferences", {}, () => ({
    interfaceFamily: null,
    codeFamily: null,
    interfaceSize: null,
    codeSize: null,
    interfaceWeight: null,
    codeWeight: null,
  }))
    .then((preferences) => {
      systemInterfaceFontName.value = preferences.interfaceFamily && isValidFontValue(preferences.interfaceFamily)
        ? preferences.interfaceFamily
        : null;
      systemCodeFontName.value = preferences.codeFamily && isValidFontValue(preferences.codeFamily)
        ? preferences.codeFamily
        : null;
      systemInterfaceFontSize.value = preferences.interfaceSize;
      systemCodeFontSize.value = preferences.codeSize;
      systemInterfaceFontWeight.value = preferences.interfaceWeight;
      systemCodeFontWeight.value = preferences.codeWeight;
    })
    .catch(() => {
      systemFontPreferencesRequest = null;
    });
  return systemFontPreferencesRequest;
}

export function setInterfaceFont(value: string | number): void {
  if (typeof value !== "string" || !isValidFontValue(value)) return;
  interfaceFont.value = value;
  persist(interfaceFontStorageKey, value);
}

export function setCodeFont(value: string | number): void {
  if (typeof value !== "string" || !isValidFontValue(value)) return;
  codeFont.value = value;
  persist(codeFontStorageKey, value);
}

function setNumericPreference(
  target: { value: string },
  storageKey: string,
  value: string | number,
  minimum: number,
  maximum: number,
  step: number,
): void {
  const normalized = String(value);
  if (normalized !== systemFontSelection) {
    const number = Number(normalized);
    if (!Number.isInteger(number) || number < minimum || number > maximum || (number - minimum) % step !== 0) return;
  }
  target.value = normalized;
  persist(storageKey, normalized);
}

export function setInterfaceFontSize(value: string | number): void {
  setNumericPreference(interfaceFontSize, interfaceFontSizeStorageKey, value, 10, 24, 1);
}

export function setCodeFontSize(value: string | number): void {
  setNumericPreference(codeFontSize, codeFontSizeStorageKey, value, 9, 24, 1);
}

export function setInterfaceFontWeight(value: string | number): void {
  setNumericPreference(interfaceFontWeight, interfaceFontWeightStorageKey, value, 100, 900, 100);
}

export function setCodeFontWeight(value: string | number): void {
  setNumericPreference(codeFontWeight, codeFontWeightStorageKey, value, 100, 900, 100);
}

export function setCodeLigatures(value: boolean): void {
  codeLigatures.value = value;
  persist(codeLigaturesStorageKey, String(value));
}

export function resetFonts(): void {
  setInterfaceFont(systemFontSelection);
  setCodeFont(systemFontSelection);
  setInterfaceFontSize(systemFontSelection);
  setCodeFontSize(systemFontSelection);
  setInterfaceFontWeight(systemFontSelection);
  setCodeFontWeight(systemFontSelection);
  setCodeLigatures(false);
}

export function loadSystemFonts(): Promise<void> {
  if (systemFontsRequest) return systemFontsRequest;
  systemFontsLoading.value = true;
  systemFontsRequest = request<SystemFontFamily[]>("list_system_fonts", {}, () => [])
    .then((fonts) => {
      const validFonts = fonts.filter((font) => isValidFontValue(font.family));
      systemFonts.value = validFonts;
      if (validFonts.length === 0) return;
      const installed = new Set(validFonts.map((font) => font.family));
      if (interfaceFont.value !== systemFontSelection && !installed.has(interfaceFont.value)) {
        setInterfaceFont(systemFontSelection);
      }
      if (codeFont.value !== systemFontSelection && !installed.has(codeFont.value)) {
        setCodeFont(systemFontSelection);
      }
    })
    .catch(() => {
      systemFontsRequest = null;
    })
    .finally(() => {
      systemFontsLoading.value = false;
    });
  return systemFontsRequest;
}

watch([
  interfaceFontFamily,
  codeFontFamily,
  resolvedInterfaceFontSize,
  resolvedCodeFontSize,
  resolvedInterfaceFontWeight,
  resolvedCodeFontWeight,
  codeLigatures,
], ([interfaceFamily, codeFamily, interfaceSize, codeSize, interfaceWeight, codeWeight, ligatures]) => {
  if (typeof document === "undefined") return;
  const root = document.documentElement.style;
  root.setProperty("--app-font-family", interfaceFamily);
  root.setProperty("--app-monospace-font-family", codeFamily);
  root.setProperty("--app-font-scale", String(interfaceSize / 14));
  root.setProperty("--app-code-font-scale", String(codeSize / 12));
  root.setProperty("--app-diff-line-height", `${resolvedDiffLineHeight.value}px`);
  root.setProperty("--app-font-weight", String(interfaceWeight));
  root.setProperty("--app-code-font-weight", String(codeWeight));
  root.setProperty("--app-font-weight-offset", String(interfaceWeight - 400));
  root.setProperty("--app-code-font-weight-offset", String(codeWeight - 400));
  root.setProperty("--app-code-font-feature-settings", ligatures
    ? "\"calt\" 1, \"liga\" 1, \"clig\" 1, \"dlig\" 1"
    : "\"calt\" 0, \"liga\" 0, \"clig\" 0, \"dlig\" 0");
  root.setProperty("--app-code-font-variant-ligatures", ligatures
    ? "common-ligatures contextual discretionary-ligatures"
    : "none");
}, { immediate: true });

void loadSystemFontPreferences();
