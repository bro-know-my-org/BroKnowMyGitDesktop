import { invoke } from "@tauri-apps/api/core";
import { asAppError } from "./appError";
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
