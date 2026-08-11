import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

export function escapeReleaseNoteText(text) {
  return text
    .replace(/[\u0000-\u001f\u007f\u200b-\u200f\u202a-\u202e\u2060-\u206f\ufeff]/g, "�")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replace(/([\\`*_[\]{}()#+.!|~@-])/g, "\\$1");
}

const maxCommitNotesLength = 100_000;
const truncatedNotesMessage = "- …（更新记录过长，已截断；请查看下方完整版本对比）";
const maxCommitCount = 2_000;
const maxCommitSubjectColumns = 512;

export function parseGitLogRecords(
  output,
  githubRepository,
  maximumLength = maxCommitNotesLength,
) {
  if (!Number.isSafeInteger(maximumLength) || maximumLength < truncatedNotesMessage.length) {
    throw new RangeError(
      `maximumLength must be an integer of at least ${truncatedNotesMessage.length}.`,
    );
  }
  const fields = output.split("\0");
  if (fields.at(-1) === "") fields.pop();
  if (fields.length % 3 !== 0) throw new Error("Unexpected git log record count.");
  const recordCount = fields.length / 3;
  const wasCountTruncated = recordCount > maxCommitCount;

  const notes = [];
  for (let index = 0; index < Math.min(fields.length, maxCommitCount * 3); index += 3) {
    const [fullHash, shortHash, paddedSubject] = fields.slice(index, index + 3);
    if (!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i.test(fullHash) || !/^[0-9a-f]+$/i.test(shortHash)) {
      throw new Error("Unexpected git log hash field.");
    }
    const subject = paddedSubject.replace(/ +$/g, "");
    const note =
      `- ${escapeReleaseNoteText(subject)} ` +
      `([${shortHash}](https://github.com/${githubRepository}/commit/${fullHash}))`;
    notes.push(note);
  }
  const renderedNotes = notes.join("\n");
  if (
    !wasCountTruncated &&
    renderedNotes.length <= maximumLength
  ) return renderedNotes;
  if (
    wasCountTruncated &&
    renderedNotes.length + 1 + truncatedNotesMessage.length <= maximumLength
  ) return `${renderedNotes}\n${truncatedNotesMessage}`;

  const prefixLimit = maximumLength - truncatedNotesMessage.length - 1;
  const lastCompleteLine = renderedNotes.lastIndexOf("\n", prefixLimit);
  if (lastCompleteLine < 0) return truncatedNotesMessage;
  return `${renderedNotes.slice(0, lastCompleteLine)}\n${truncatedNotesMessage}`;
}

export function releaseLogArgs(commitRange) {
  return [
    "log",
    "-z",
    "--first-parent",
    `--max-count=${maxCommitCount + 1}`,
    `--pretty=format:%H%x00%h%x00%<(${maxCommitSubjectColumns},trunc)%s`,
    "--end-of-options",
    commitRange,
  ];
}

export function generateCommitNotes(
  repositoryRoot,
  commitRange,
  githubRepository,
  runGit = execFileSync,
) {
  try {
    const output = runGit(
      "git",
      releaseLogArgs(commitRange),
      { cwd: repositoryRoot, encoding: "utf8", maxBuffer: 4 * 1024 * 1024 },
    );
    return parseGitLogRecords(output, githubRepository);
  } catch (error) {
    if (error.code === "ENOBUFS") return truncatedNotesMessage;
    throw error;
  }
}

const invokedPath = process.argv[1]
  ? fileURLToPath(import.meta.url) === path.resolve(process.argv[1])
  : false;
if (invokedPath) {
  const [commitRange, githubRepository] = process.argv.slice(2);
  if (!commitRange || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(githubRepository ?? "")) {
    console.error("Usage: release-notes.mjs <commit-range> <owner/repository>");
    process.exit(1);
  }
  console.log(generateCommitNotes(process.cwd(), commitRange, githubRepository));
}
