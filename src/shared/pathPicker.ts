import { shallowRef } from "vue";

interface PathPickerOptions {
  title: string;
  directory: boolean;
  initialPath?: string;
}

export const pathPickerRequest = shallowRef<(PathPickerOptions & { id: number }) | null>(null);
let nextId = 0;
let resolveSelection: ((path: string | null) => void) | null = null;

export function selectPath(options: PathPickerOptions): Promise<string | null> {
  if (pathPickerRequest.value) finishPathSelection(pathPickerRequest.value.id, null);
  return new Promise((resolve) => {
    resolveSelection = resolve;
    pathPickerRequest.value = { ...options, id: ++nextId };
  });
}

export function finishPathSelection(id: number, path: string | null): void {
  if (pathPickerRequest.value?.id !== id) return;
  const resolve = resolveSelection;
  resolveSelection = null;
  pathPickerRequest.value = null;
  resolve?.(path);
}

export function normalizePickerPath(path: string): string {
  const trimmed = path.trim();
  return /^(["']).*\1$/u.test(trimmed) ? trimmed.slice(1, -1) : trimmed;
}
