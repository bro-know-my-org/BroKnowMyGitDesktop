export const supportedAppLocales = ["zh-CN", "en-US"] as const;
export type AppLocale = (typeof supportedAppLocales)[number];

export const localeStorageKey = "broknowmygitdesktop.locale";
export const defaultAppLocale: AppLocale = "en-US";

export function isAppLocale(value: string | null): value is AppLocale {
  return supportedAppLocales.some((locale) => locale === value);
}

export function resolveSupportedLocale(preferredLanguages: readonly string[]): AppLocale {
  for (const language of preferredLanguages) {
    const normalized = language.toLowerCase();
    const exactMatch = supportedAppLocales.find((locale) => locale.toLowerCase() === normalized);
    if (exactMatch) return exactMatch;
    if (normalized === "zh" || normalized.startsWith("zh-")) return "zh-CN";
    if (normalized === "en" || normalized.startsWith("en-")) return "en-US";
  }
  return defaultAppLocale;
}
