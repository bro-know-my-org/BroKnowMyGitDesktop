export type ResourceKind = "image" | "audio" | "video" | "pdf" | "binary";
const extensions: Record<string, ResourceKind> = {
  png: "image", jpg: "image", jpeg: "image", gif: "image", webp: "image", avif: "image", bmp: "image", ico: "image", svg: "image",
  mp3: "audio", wav: "audio", ogg: "audio", oga: "audio", flac: "audio", m4a: "audio", aac: "audio", opus: "audio",
  mp4: "video", webm: "video", mov: "video", m4v: "video", ogv: "video",
  pdf: "pdf",
};
export const resourceExtension = (path: string) => {
  const name = path.split("/").pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1).toLowerCase() : "";
};
export const resourceKind = (path: string): ResourceKind => extensions[resourceExtension(path)] ?? "binary";
export const isVisualResource = (path: string) => resourceKind(path) !== "binary";
export const isResourceDiff = (path: string, diff: string, previousPath?: string | null) =>
  isVisualResource(path) || Boolean(previousPath && isVisualResource(previousPath)) || /^(?:Binary files |GIT binary patch)/mu.test(diff);
export const skipResourceTextDiff = (path: string, previousPath?: string | null) =>
  isVisualResource(path) && resourceExtension(path) !== "svg" && resourceExtension(previousPath ?? "") !== "svg";
export const formatResourceSize = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`;
  let unit = 3;
  if (bytes < 1024 ** 2) unit = 1;
  else if (bytes < 1024 ** 3) unit = 2;
  return `${(bytes / 1024 ** unit).toFixed(1)} ${["B", "KiB", "MiB", "GiB"][unit]}`;
};

/** Highlight changed visible color/alpha. Ignore invisible RGB in fully transparent pixels. */
export function pixelDifference(before: Uint8ClampedArray, after: Uint8ClampedArray): Uint8ClampedArray {
  if (before.length !== after.length || before.length % 4) throw new Error("Invalid image buffers");
  const result = new Uint8ClampedArray(before.length);
  for (let index = 0; index < before.length; index += 4) {
    const oldAlpha = before[index + 3] / 255, newAlpha = after[index + 3] / 255;
    let delta = Math.abs(before[index + 3] - after[index + 3]);
    for (let channel = 0; channel < 3; channel++) {
      delta = Math.max(delta, Math.abs(before[index + channel] * oldAlpha - after[index + channel] * newAlpha));
    }
    if (delta > 0) { result[index] = 255; result[index + 2] = 100; result[index + 3] = Math.max(80, delta); }
  }
  return result;
}
