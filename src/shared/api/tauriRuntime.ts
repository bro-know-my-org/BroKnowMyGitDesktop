import { invoke } from "@tauri-apps/api/core";
import { localizedErrorDetails, localizedErrorMessage } from "../i18n";
import { measureCommand, performanceProfilingEnabled } from "../performance";
import type { AppError } from "../types/domain";

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);
}

export function isBrowserDemo(): boolean {
  return !isTauriRuntime();
}

/** Tauri only exposes this built-in command in debug builds. */
export async function toggleDevtools(): Promise<void> {
  if (!isTauriRuntime()) return;
  await invoke("plugin:webview|internal_toggle_devtools");
}

function asAppError(error: unknown): AppError {
  if (typeof error === "object" && error !== null && "message" in error) {
    const candidate = error as Partial<AppError>;
    const code = typeof candidate.code === "string" ? candidate.code : "command_failed";
    const details = candidate.details ? localizedErrorDetails(code) : "";
    return {
      code,
      message: localizedErrorMessage(code),
      details: details || null,
    };
  }

  const raw = typeof error === "string" ? error : localizedErrorMessage("command_failed");
  try {
    const parsed = JSON.parse(raw) as Partial<AppError>;
    if (typeof parsed.message === "string") {
      const code = typeof parsed.code === "string" ? parsed.code : "command_failed";
      const details = parsed.details ? localizedErrorDetails(code) : "";
      return {
        code,
        message: localizedErrorMessage(code),
        details: details || null,
      };
    }
  } catch {
    // Tauri also returns ordinary strings for some command failures.
  }
  return { code: "command_failed", message: localizedErrorMessage("command_failed"), details: null };
}

export async function request<T>(
  command: string,
  args: object,
  browserFallback: () => T | Promise<T>,
): Promise<T> {
  const operation = async (): Promise<T> => {
    if (!isTauriRuntime()) {
      try {
        return await browserFallback();
      } catch (error) {
        throw asAppError(error);
      }
    }
    try {
      return await invoke<T>(command, args as Record<string, unknown>);
    } catch (error) {
      throw asAppError(error);
    }
  };
  return performanceProfilingEnabled ? measureCommand(command, operation) : operation();
}

export function isAppError(error: unknown): error is AppError {
  return typeof error === "object" && error !== null && "code" in error && "message" in error;
}
