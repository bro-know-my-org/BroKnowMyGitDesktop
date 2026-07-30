import { computed, ref } from "vue";
import { dateEnUS, dateZhCN, enUS, zhCN } from "naive-ui";
import type { AuthenticationMethod, AuthenticationStatus, CodeHostKind } from "../types/domain";
import { enUSMessages, zhCNMessages, type MessageKey } from "./locales";
import {
  defaultAppLocale,
  isAppLocale,
  localeStorageKey,
  resolveSupportedLocale,
  type AppLocale,
} from "./locale";

export type { MessageKey } from "./locales";
export { supportedAppLocales, type AppLocale } from "./locale";

function initialLocale(): AppLocale {
  if (typeof window === "undefined") return defaultAppLocale;
  try {
    const saved = window.localStorage.getItem(localeStorageKey);
    if (isAppLocale(saved)) return saved;
  } catch {
    // A disabled storage area must not prevent the UI from starting.
  }
  let preferredLanguages: readonly string[] = [];
  if (typeof navigator !== "undefined") {
    if (Array.isArray(navigator.languages) && navigator.languages.length) {
      preferredLanguages = navigator.languages;
    } else if (navigator.language) {
      preferredLanguages = [navigator.language];
    }
  }
  return resolveSupportedLocale(preferredLanguages);
}

export const appLocale = ref<AppLocale>(initialLocale());

function applyDocumentLocale(locale: AppLocale): void {
  if (typeof document !== "undefined") document.documentElement.lang = locale;
}

applyDocumentLocale(appLocale.value);

export function setAppLocale(locale: AppLocale): void {
  appLocale.value = locale;
  if (typeof window !== "undefined") {
    try {
      window.localStorage.setItem(localeStorageKey, locale);
    } catch {
      // The current session can still use the selected locale.
    }
  }
  applyDocumentLocale(locale);
}

export type TranslationValues = Record<string, string | number>;

const messagesByLocale = {
  "zh-CN": zhCNMessages,
  "en-US": enUSMessages,
} satisfies Record<AppLocale, Record<MessageKey, string>>;

function interpolate(source: string, values: TranslationValues): string {
  return source.replace(/\{([A-Za-z][A-Za-z0-9_]*)\}/gu, (_, key: string) => String(values[key] ?? `{${key}}`));
}

/** Translate a stable UI message key from the statically bundled locale catalogs. */
export function t(key: MessageKey, values: TranslationValues = {}): string {
  const localized = messagesByLocale[appLocale.value][key];
  const source = localized ?? zhCNMessages[key] ?? key;
  return interpolate(source, values);
}

export function hasTranslation(key: string): key is MessageKey {
  return Object.prototype.hasOwnProperty.call(zhCNMessages, key);
}

const endpointKindKeys: Record<CodeHostKind, MessageKey> = {
  gitHub: "common.endpoint.kind.gitHub",
  gitLab: "common.endpoint.kind.gitLab",
  gitee: "common.endpoint.kind.gitee",
  gitea: "common.endpoint.kind.gitea",
  forgejo: "common.endpoint.kind.forgejo",
  genericGit: "common.endpoint.kind.genericGit",
  unknown: "common.endpoint.kind.unknown",
};

const authenticationMethodKeys: Record<AuthenticationMethod, MessageKey> = {
  sshAgent: "common.authentication.method.sshAgent",
  systemGitCredentialHelper: "common.authentication.method.systemGitCredentialHelper",
  ghCli: "common.authentication.method.ghCli",
  glabCli: "common.authentication.method.glabCli",
  personalAccessToken: "common.authentication.method.personalAccessToken",
};

const authenticationStatusKeys: Record<AuthenticationStatus, MessageKey> = {
  unknown: "common.authentication.status.unknown",
  available: "common.authentication.status.available",
  valid: "common.authentication.status.valid",
  needsReauthentication: "common.authentication.status.needsReauthentication",
  unavailable: "common.authentication.status.unavailable",
};

export function endpointKindLabel(kind: CodeHostKind): string {
  return t(endpointKindKeys[kind] ?? "common.endpoint.kind.unknown");
}

export function authenticationMethodLabel(method: AuthenticationMethod): string {
  return t(authenticationMethodKeys[method] ?? "common.authentication.method.unknown");
}

export function authenticationStatusLabel(status: AuthenticationStatus): string {
  return t(authenticationStatusKeys[status] ?? "common.authentication.status.unknown");
}

const dateFormatters = new Map<string, Intl.DateTimeFormat>();
const numberFormatters = new Map<string, Intl.NumberFormat>();
const relativeTimeFormatters = new Map<string, Intl.RelativeTimeFormat>();
const listFormatters = new Map<string, Intl.ListFormat>();

function formatterKey(locale: AppLocale, options: object): string {
  return `${locale}:${JSON.stringify(options)}`;
}

export function formatDate(
  value: Date | number,
  options: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" },
): string {
  const key = formatterKey(appLocale.value, options);
  let formatter = dateFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(appLocale.value, options);
    dateFormatters.set(key, formatter);
  }
  return formatter.format(value);
}

export function formatNumber(value: number, options: Intl.NumberFormatOptions = {}): string {
  const key = formatterKey(appLocale.value, options);
  let formatter = numberFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.NumberFormat(appLocale.value, options);
    numberFormatters.set(key, formatter);
  }
  return formatter.format(value);
}

export function formatRelativeTime(
  value: number,
  unit: Intl.RelativeTimeFormatUnit,
  options: Intl.RelativeTimeFormatOptions = { numeric: "auto" },
): string {
  const key = formatterKey(appLocale.value, options);
  let formatter = relativeTimeFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.RelativeTimeFormat(appLocale.value, options);
    relativeTimeFormatters.set(key, formatter);
  }
  return formatter.format(value, unit);
}

export function formatList(values: Iterable<string>, options: Intl.ListFormatOptions = {}): string {
  const key = formatterKey(appLocale.value, options);
  let formatter = listFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.ListFormat(appLocale.value, options);
    listFormatters.set(key, formatter);
  }
  return formatter.format(values);
}

/** Translate curated backend error codes without rendering untrusted fallback text. */
export function localizedErrorMessage(code: string): string {
  const key = `errors.${code}`;
  return hasTranslation(key) ? t(key) : t("errors.command_failed");
}

/** Return only curated remediation text; unknown details stay hidden. */
export function localizedErrorDetails(code: string): string {
  const key = `errors.${code}.details`;
  return hasTranslation(key) ? t(key) : "";
}

export const naiveUiLocale = computed(() => (appLocale.value === "zh-CN" ? zhCN : enUS));
export const naiveUiDateLocale = computed(() => (appLocale.value === "zh-CN" ? dateZhCN : dateEnUS));
