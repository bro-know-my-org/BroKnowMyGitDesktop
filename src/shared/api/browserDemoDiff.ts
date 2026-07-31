import { localizedErrorMessage } from "../i18n";
import type { AppError, CommitFileDiff } from "../types/domain";

function browserDiffError(): AppError {
  return { code: "invalid_diff_limit", message: localizedErrorMessage("invalid_diff_limit") };
}

export function boundedBrowserCommitFileDiff(text: string, maximumBytes: number): CommitFileDiff {
  if (!Number.isSafeInteger(maximumBytes) || maximumBytes < 64 * 1024 || maximumBytes > 4 * 1024 * 1024) {
    throw browserDiffError();
  }
  const bytes = new TextEncoder().encode(text);
  if (bytes.length <= maximumBytes) return { text, truncated: false };
  let end = maximumBytes;
  const decoder = new TextDecoder("utf-8", { fatal: true });
  while (end > 0) {
    try {
      return { text: decoder.decode(bytes.slice(0, end)), truncated: true };
    } catch {
      end -= 1;
    }
  }
  return {
    text: "",
    truncated: true,
  };
}

function parseGitQuotedDiffPath(header: string, index: number): { path: string; end: number } | null {
  if (header[index] !== '"') return null;
  const bytes: number[] = [];
  for (let cursor = index + 1; cursor < header.length; cursor += 1) {
    const character = header[cursor];
    if (character === '"') {
      try {
        return { path: new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes)), end: cursor + 1 };
      } catch {
        return null;
      }
    }
    if (character === "\\") {
      const escaped = header[++cursor];
      if (escaped === undefined) return null;
      const escapedByte = escaped === "n" ? 0x0a
        : escaped === "r" ? 0x0d
          : escaped === "t" ? 0x09
            : escaped === "\\" || escaped === '"' ? escaped.charCodeAt(0)
              : null;
      if (escapedByte !== null) {
        bytes.push(escapedByte);
        continue;
      }
      if (!/[0-7]/u.test(escaped) || cursor + 2 >= header.length) return null;
      const octal = header.slice(cursor, cursor + 3);
      if (!/^[0-7]{3}$/u.test(octal)) return null;
      bytes.push(Number.parseInt(octal, 8));
      cursor += 2;
      continue;
    }
    const code = character.charCodeAt(0);
    if (code < 0x20) return null;
    if (code <= 0x7e) {
      bytes.push(code);
    } else {
      bytes.push(...new TextEncoder().encode(character));
    }
  }
  return null;
}

export function parseGitDiffHeaderPaths(header: string): [string, string] | null {
  const prefix = "diff --git ";
  if (!header.startsWith(prefix)) return null;
  let index = prefix.length;
  const paths: string[] = [];
  for (let pathIndex = 0; pathIndex < 2; pathIndex += 1) {
    if (index >= header.length) return null;
    if (header[index] === '"') {
      const parsed = parseGitQuotedDiffPath(header, index);
      if (!parsed) return null;
      paths.push(parsed.path);
      index = parsed.end;
    } else {
      const end = header.indexOf(" ", index);
      const tokenEnd = end === -1 ? header.length : end;
      if (tokenEnd === index) return null;
      paths.push(header.slice(index, tokenEnd));
      index = tokenEnd;
    }
    if (pathIndex === 0) {
      if (header[index] !== " ") return null;
      index += 1;
    }
  }
  return index === header.length ? [paths[0], paths[1]] : null;
}

