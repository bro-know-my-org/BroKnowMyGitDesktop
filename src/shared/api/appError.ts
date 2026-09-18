import { localizedErrorDetails, localizedErrorMessage } from "../i18n";
import type { AppError } from "../types/domain";

/** Only the backend's dedicated, redacted diagnostic field is displayed verbatim. */
export function asAppError(error: unknown): AppError {
  if (typeof error === "string") {
    try {
      return asAppError(JSON.parse(error));
    } catch {
      // Unstructured IPC errors have not passed through backend redaction.
    }
  }
  if (typeof error === "object" && error !== null && "message" in error) {
    const candidate = error as Partial<AppError>;
    const code = typeof candidate.code === "string" ? candidate.code : "command_failed";
    const diagnostic = typeof candidate.diagnostic === "string" ? candidate.diagnostic.trim() : "";
    const guidance = localizedErrorDetails(code);
    return {
      code,
      message: localizedErrorMessage(code),
      diagnostic: diagnostic || null,
      details: [guidance, diagnostic].filter(Boolean).join("\n\n") || null,
    };
  }
  return { code: "command_failed", message: localizedErrorMessage("command_failed"), details: null };
}
