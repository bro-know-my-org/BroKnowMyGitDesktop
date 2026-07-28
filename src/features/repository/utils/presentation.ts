import type { tr as translate } from "../../../shared/i18n";
import type { AccountProfile, GitOperationKind, GitStatusEntry, SubmoduleInfo } from "../../../shared/types/domain";

export interface DiffHunk {
  header: string;
  patch: string;
}

type Translate = typeof translate;

export function createRepositoryPresentation(tr: Translate) {
  function authenticationTagType(status: AccountProfile["status"]): "default" | "success" | "warning" | "error" {
    if (status === "valid" || status === "available") return "success";
    if (status === "needsReauthentication") return "warning";
    if (status === "unavailable") return "error";
    return "default";
  }

  function changeStatusLabel(entry: GitStatusEntry): string {
    if (!entry.indexStatus && entry.worktreeStatus === "?") return tr("未跟踪", "Untracked");
    if (isConflictedEntry(entry)) return tr("有冲突", "Conflicted");
    if (entry.indexStatus && entry.worktreeStatus) return tr("部分已暂存", "Partially staged");
    if (entry.indexStatus) return tr("已暂存 · {status}", "Staged · {status}", { status: changeKindLabel(entry.indexStatus) });
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
      return tr("将 {path} 的所有改动加入下一次提交", "Include all changes in {path} in the next commit", { path: entry.path });
    }
    return isChangeFullyIncluded(entry)
      ? tr("从下一次提交中移除 {path}", "Remove {path} from the next commit", { path: entry.path })
      : tr("将 {path} 加入下一次提交", "Include {path} in the next commit", { path: entry.path });
  }

  function changeKindLabel(status: string): string {
    switch (status) {
      case "A":
        return tr("新增", "Added");
      case "M":
        return tr("已修改", "Modified");
      case "D":
        return tr("已删除", "Deleted");
      case "R":
        return tr("已重命名", "Renamed");
      case "C":
        return tr("已复制", "Copied");
      case "U":
        return tr("未合并", "Unmerged");
      default:
        return tr("有改动", "Changed");
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
      return { header: lines[start], patch: `${preamble}\n${hunk}` };
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
        return tr("合并", "Merge");
      case "rebase":
        return tr("变基", "Rebase");
      case "cherryPick":
        return tr("拣选提交", "Cherry-pick");
      case "revert":
        return tr("还原提交", "Revert");
    }
  }

  function submoduleStatusLabel(status: SubmoduleInfo["status"]): string {
    switch (status) {
      case "initialized":
        return tr("已就绪", "Ready");
      case "uninitialized":
        return tr("未初始化", "Uninitialized");
      case "needsUpdate":
        return tr("需要更新", "Needs update");
      case "conflicted":
        return tr("冲突", "Conflicted");
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
