type StartupTheme = "light" | "dark";

function resolveStartupTheme(): StartupTheme {
  try {
    const saved = window.localStorage.getItem("broknowmygitdesktop.color-mode");
    if (saved === "light" || saved === "dark") return saved;
  } catch {
    // The system preference remains a safe fallback when storage is blocked.
  }
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

document.documentElement.dataset.startupTheme = resolveStartupTheme();

function showStartupFailure(): void {
  const startup = document.querySelector<HTMLElement>("#bkm-startup");
  if (!startup) return;
  const content = startup.querySelector<HTMLElement>(".bkm-startup-content");
  if (!content) return;
  content.replaceChildren();
  const message = document.createElement("p");
  message.textContent = "应用启动失败，请重新启动。";
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
