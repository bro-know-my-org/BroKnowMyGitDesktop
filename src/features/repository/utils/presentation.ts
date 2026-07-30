import { t } from "../../../shared/i18n";
import type { AccountProfile, GitOperationKind, GitStatusEntry, SubmoduleInfo } from "../../../shared/types/domain";

export interface DiffHunk {
  header: string;
  lineIndex: number;
  patch: string;
}

export function createRepositoryPresentation() {
  function authenticationTagType(status: AccountProfile["status"]): "default" | "success" | "warning" | "error" {
    if (status === "valid" || status === "available") return "success";
    if (status === "needsReauthentication") return "warning";
    if (status === "unavailable") return "error";
    return "default";
  }

  function changeStatusLabel(entry: GitStatusEntry): string {
    if (!entry.indexStatus && entry.worktreeStatus === "?") return t("repository.presentation.untracked");
    if (isConflictedEntry(entry)) return t("repository.presentation.conflicted");
    if (entry.indexStatus && entry.worktreeStatus) return t("repository.presentation.partiallyStaged");
    if (entry.indexStatus) return t("repository.presentation.stagedStatus", { status: changeKindLabel(entry.indexStatus) });
    if (entry.worktreeStatus === "M") return "";
    return changeKindLabel(entry.worktreeStatus);
  }

  function changeStatusTagType(entry: GitStatusEntry): "default" | "success" | "warning" | "error" {
    if (isConflictedEntry(entry)) return "error";
    if (entry.indexStatus && entry.worktreeStatus) return "warning";
    if (entry.indexStatus) return "success";
    return entry.worktreeStatus ? "warning" : "default";
  }

  function isChangeFullyIncluded(entry: GitStatusEntry): boolean {
    return Boolean(entry.indexStatus) && !entry.worktreeStatus;
  }

  function isChangePartiallyIncluded(entry: GitStatusEntry): boolean {
    return Boolean(entry.indexStatus) && Boolean(entry.worktreeStatus);
  }

  function changeIncludeLabel(entry: GitStatusEntry): string {
    if (isChangePartiallyIncluded(entry)) {
      return t("repository.presentation.includeAllChangesPathNextCommit", { path: entry.path });
    }
    return isChangeFullyIncluded(entry)
      ? t("repository.presentation.removePathNextCommit", { path: entry.path })
      : t("repository.presentation.includePathNextCommit", { path: entry.path });
  }

  function changeKindLabel(status: string): string {
    switch (status) {
      case "A":
        return t("repository.history.added");
      case "M":
        return t("repository.presentation.modified");
      case "D":
        return t("repository.presentation.deleted");
      case "R":
        return t("repository.presentation.renamed");
      case "C":
        return t("repository.presentation.copied");
      case "U":
        return t("repository.history.unmerged");
      default:
        return t("repository.presentation.changed");
    }
  }

  function isConflictedEntry(entry: GitStatusEntry): boolean {
    return ["DD", "AU", "UD", "UA", "DU", "AA", "UU"].includes(`${entry.indexStatus}${entry.worktreeStatus}`);
  }

  function splitDiffHunks(diff: string): DiffHunk[] {
    const lines = diff.split("\n");
    const hunkStarts = lines.reduce<number[]>((indexes, line, index) => {
      if (line.startsWith("@@ ")) indexes.push(index);
      return indexes;
    }, []);
    if (!hunkStarts.length) return [];
    const preamble = lines.slice(0, hunkStarts[0]).join("\n");
    return hunkStarts.map((start, index) => {
      const end = hunkStarts[index + 1] ?? lines.length;
      const hunk = lines.slice(start, end).join("\n");
      return { header: lines[start], lineIndex: start, patch: `${preamble}\n${hunk}` };
    });
  }

  function splitDiffLines(diff: string): string[] {
    const lines = diff.split("\n");
    if (lines[lines.length - 1] === "") lines.pop();
    return lines;
  }

  function diffLineClass(line: string): string {
    if (line.startsWith("diff --git ")) return "is-file-header";
    if (line.startsWith("@@ ")) return "is-hunk";
    if (line.startsWith("+++ ")) return "is-add-file";
    if (line.startsWith("--- ")) return "is-remove-file";
    if (line.startsWith("+")) return "is-add";
    if (line.startsWith("-")) return "is-remove";
    if (line.startsWith("\\ No newline at end of file")) return "is-no-newline";
    if (/^(?:index |new file mode |deleted file mode |similarity index |dissimilarity index |rename (?:from|to) |Binary files |GIT binary patch)/u.test(line)) return "is-meta";
    return "is-context";
  }

  function gitOperationLabel(kind: GitOperationKind): string {
    switch (kind) {
      case "merge":
        return t("repository.presentation.merge");
      case "rebase":
        return t("repository.presentation.rebase");
      case "cherryPick":
        return t("repository.branches.cherryPick");
      case "revert":
        return t("repository.presentation.revert");
    }
  }

  function submoduleStatusLabel(status: SubmoduleInfo["status"]): string {
    switch (status) {
      case "initialized":
        return t("repository.presentation.ready");
      case "uninitialized":
        return t("repository.presentation.uninitialized");
      case "needsUpdate":
        return t("repository.presentation.needsUpdate");
      case "conflicted":
        return t("repository.presentation.submoduleConflicted");
    }
  }

  function submoduleStatusTagType(status: SubmoduleInfo["status"]): "success" | "warning" | "error" {
    return status === "initialized" ? "success" : status === "conflicted" ? "error" : "warning";
  }

  return {
    authenticationTagType,
    changeStatusLabel,
    changeStatusTagType,
    isChangeFullyIncluded,
    isChangePartiallyIncluded,
    changeIncludeLabel,
    changeKindLabel,
    isConflictedEntry,
    splitDiffHunks,
    splitDiffLines,
    diffLineClass,
    gitOperationLabel,
    submoduleStatusLabel,
    submoduleStatusTagType,
  };
}
