import {
  isAppLocale,
  localeStorageKey,
  resolveSupportedLocale,
  type AppLocale,
} from "./shared/i18n/locale";

type StartupTheme = "light" | "dark";

const startupFailureMessages: Record<AppLocale, string> = {
  "zh-CN": "应用启动失败，请重新启动。",
  "en-US": "The application failed to start. Please restart it.",
};
const startupStatusMessages: Record<AppLocale, string> = {
  "zh-CN": "正在启动 BroKnowMyGitDesktop",
  "en-US": "Starting BroKnowMyGitDesktop",
};

function browserPreferredLanguages(): readonly string[] {
  if (typeof navigator === "undefined") return [];
  if (Array.isArray(navigator.languages) && navigator.languages.length) return navigator.languages;
  return navigator.language ? [navigator.language] : [];
}

function initialStartupLocale(): AppLocale {
  try {
    const saved = window.localStorage.getItem(localeStorageKey);
    if (isAppLocale(saved)) return saved;
  } catch {
    // Browser preferences remain a safe fallback when storage is blocked.
  }
  return resolveSupportedLocale(browserPreferredLanguages());
}

function resolveStartupTheme(): StartupTheme {
  try {
    const saved = window.localStorage.getItem("broknowmygitdesktop.color-mode");
    if (saved === "light" || saved === "dark") return saved;
  } catch {
    // The system preference remains a safe fallback when storage is blocked.
  }
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

const startupLocale = initialStartupLocale();
document.documentElement.lang = startupLocale;
document.documentElement.dataset.startupTheme = resolveStartupTheme();
document.querySelector<HTMLElement>("#bkm-startup")?.setAttribute("aria-label", startupStatusMessages[startupLocale]);

function showStartupFailure(): void {
  const startup = document.querySelector<HTMLElement>("#bkm-startup");
  if (!startup) return;
  const content = startup.querySelector<HTMLElement>(".bkm-startup-content");
  if (!content) return;
  startup.setAttribute("aria-label", startupFailureMessages[startupLocale]);
  content.replaceChildren();
  const message = document.createElement("p");
  message.textContent = startupFailureMessages[startupLocale];
  message.style.color = "#8994a6";
  message.style.fontSize = "14px";
  content.append(message);
}

function loadApplication(): void {
  void import("./main").catch(() => {
    showStartupFailure();
  });
}

// Let the static startup shell reach the compositor before Vue, Naive UI and
// the workspace controller begin parsing and mounting.
if (typeof requestAnimationFrame === "function") {
  requestAnimationFrame(() => requestAnimationFrame(loadApplication));
} else {
  setTimeout(loadApplication, 0);
}
