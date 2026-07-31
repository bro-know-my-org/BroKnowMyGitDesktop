import { computed, ref, type ComputedRef, type Ref } from "vue";
import type { DialogApi, MessageApi } from "naive-ui";
import { api } from "../../../shared/api/tauri";
import { t } from "../../../shared/i18n";
import type { CommitSummary, GitStatusEntry, RepositorySnapshot } from "../../../shared/types/domain";
import type { DiffHunk } from "../../repository/utils/presentation";

interface RepositoryChangesDependencies {
  activeRepositoryPath: Ref<string | null>;
  snapshot: Ref<RepositorySnapshot | null>;
  currentBranch: ComputedRef<string>;
  commits: Ref<CommitSummary[]>;
  mutationBusy: Ref<boolean>;
  getSelectionGeneration: () => number;
  repositorySelectionIsCurrent: (repositoryPath: string, generation: number) => boolean;
  applySnapshot: (snapshot: RepositorySnapshot) => boolean;
  loadHistory: () => Promise<void>;
  isChangeFullyIncluded: (entry: GitStatusEntry) => boolean;
  isConflictedEntry: (entry: GitStatusEntry) => boolean;
  splitDiffHunks: (diff: string) => DiffHunk[];
  splitDiffLines: (diff: string) => string[];
  reportError: (error: unknown, action: string) => void;
  message: MessageApi;
  dialog: DialogApi;
}

export function useRepositoryChanges({
  activeRepositoryPath,
  snapshot,
  currentBranch,
  commits,
  mutationBusy,
  getSelectionGeneration,
  repositorySelectionIsCurrent,
  applySnapshot,
  loadHistory,
  isChangeFullyIncluded,
  isConflictedEntry,
  splitDiffHunks,
  splitDiffLines,
  reportError,
  message,
  dialog,
}: RepositoryChangesDependencies) {
  const selectedChange = ref<GitStatusEntry | null>(null);
  const diffText = ref("");
  const selectedDiffStaged = ref(false);
  const blameText = ref("");
  const diffLoading = ref(false);
  const blameLoading = ref(false);
  const blameModalShown = ref(false);
  const commitMessage = ref("");
  const amendLastCommit = ref(false);
  let diffRequestGeneration = 0;

  const stagedChanges = computed(() =>
    snapshot.value?.status.entries.filter((entry) => Boolean(entry.indexStatus)) ?? []);
  const stagedCount = computed(() => stagedChanges.value.length);
  const unstagedCount = computed(() =>
    snapshot.value?.status.entries.filter((entry) => Boolean(entry.worktreeStatus)).length ?? 0);
  const workingChanges = computed(() =>
    snapshot.value?.status.entries.filter((entry) => Boolean(entry.worktreeStatus)) ?? []);
  const untrackedChanges = computed(() => snapshot.value?.status.entries.filter(
    (entry) => !entry.indexStatus && entry.worktreeStatus === "?",
  ) ?? []);
  const untrackedCount = computed(() => untrackedChanges.value.length);
  const allChangesIncluded = computed(() => {
    const entries = snapshot.value?.status.entries ?? [];
    return entries.length > 0 && entries.every(isChangeFullyIncluded);
  });
  const someChangesIncluded = computed(() => {
    const entries = snapshot.value?.status.entries ?? [];
    return !allChangesIncluded.value && entries.some((entry) => Boolean(entry.indexStatus));
  });
  const conflictedChanges = computed(() =>
    snapshot.value?.status.entries.filter(isConflictedEntry) ?? []);
  const diffHunks = computed(() => splitDiffHunks(diffText.value));
  const diffHunksByLine = computed(() =>
    new Map(diffHunks.value.map((hunk) => [hunk.lineIndex, hunk])));
  const diffLines = computed(() => splitDiffLines(diffText.value));
  const hasDiffText = computed(() => diffText.value.trim().length > 0);
  const canAmendLatestCommit = computed(() => snapshot.value?.hasHead ?? false);
  const isUnbornRepository = computed(() => Boolean(snapshot.value && !snapshot.value.hasHead));
  const commitActionLabel = computed(() => {
    if (amendLastCommit.value) return t("workspace.controller.amendLastCommit");
    if (!stagedCount.value) return t("workspace.controller.selectFilesToCommit");
    return t(
      stagedCount.value === 1
        ? "workspace.controller.commitOneFileToBranch"
        : "workspace.controller.commitFilesToBranch",
      { count: stagedCount.value, branch: currentBranch.value },
    );
  });
  const commitSelectionSummary = computed(() => {
    if (amendLastCommit.value) {
      return t(
        "workspace.controller.thisRewritesTheLatestCommitAndIncludesTheCountFileSCurrentlySelected",
        { count: stagedCount.value },
      );
    }
    return t(
      stagedCount.value === 1
        ? "workspace.controller.oneFileSelectedGitHooksRunAsUsual"
        : "workspace.controller.filesSelectedGitHooksRunAsUsual",
      { count: stagedCount.value },
    );
  });

  function clearDiffSelection(): void {
    diffRequestGeneration += 1;
    selectedChange.value = null;
    diffText.value = "";
    selectedDiffStaged.value = false;
    diffLoading.value = false;
  }

  function getDiffRequestGeneration(): number {
    return diffRequestGeneration;
  }

  async function loadDiff(
    entry: GitStatusEntry,
    repositoryPath = activeRepositoryPath.value,
    reportErrors = true,
  ): Promise<void> {
    if (!repositoryPath || activeRepositoryPath.value !== repositoryPath) return;
    const requestGeneration = ++diffRequestGeneration;
    selectedChange.value = entry;
    selectedDiffStaged.value = Boolean(entry.indexStatus) && !entry.worktreeStatus;
    diffLoading.value = true;
    try {
      const diff = await api.getFileDiff(
        repositoryPath,
        entry.path,
        Boolean(entry.indexStatus) && !entry.worktreeStatus,
      );
      if (
        activeRepositoryPath.value !== repositoryPath
        || selectedChange.value?.path !== entry.path
        || requestGeneration !== diffRequestGeneration
      ) return;
      diffText.value = diff;
    } catch (error) {
      if (activeRepositoryPath.value === repositoryPath && requestGeneration === diffRequestGeneration && reportErrors) {
        reportError(error, t("workspace.controller.couldNotLoadDiff"));
      }
    } finally {
      if (activeRepositoryPath.value === repositoryPath && requestGeneration === diffRequestGeneration) {
        diffLoading.value = false;
      }
    }
  }

  async function syncSelectedDiff(
    nextSnapshot: RepositorySnapshot,
    selectedPath?: string,
    selectFirst = false,
    repositoryPath = activeRepositoryPath.value,
    reportErrors = true,
  ): Promise<void> {
    if (!repositoryPath || activeRepositoryPath.value !== repositoryPath) return;
    const nextChange = (selectedPath
      ? nextSnapshot.status.entries.find((entry) => entry.path === selectedPath)
      : null) ?? (selectFirst ? nextSnapshot.status.entries[0] : null);
    if (nextChange) await loadDiff(nextChange, repositoryPath, reportErrors);
    else clearDiffSelection();
  }

  async function openFileBlame(): Promise<void> {
    if (!activeRepositoryPath.value || !selectedChange.value) return;
    blameModalShown.value = true;
    blameText.value = "";
    blameLoading.value = true;
    try {
      blameText.value = await api.getFileBlame(activeRepositoryPath.value, selectedChange.value.path);
    } catch (error) {
      blameModalShown.value = false;
      reportError(error, t("workspace.controller.couldNotLoadFileBlame"));
    } finally {
      blameLoading.value = false;
    }
  }

  async function applyDiffHunk(hunk: DiffHunk): Promise<void> {
    if (!activeRepositoryPath.value || !selectedChange.value) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    const selectedPath = selectedChange.value.path;
    const reverse = selectedDiffStaged.value;
    mutationBusy.value = true;
    try {
      const nextSnapshot = await api.applyPatch(repositoryPath, selectedPath, hunk.patch, reverse);
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
      const nextEntry = nextSnapshot.status.entries.find((entry) => entry.path === selectedPath);
      if (nextEntry) await loadDiff(nextEntry, repositoryPath);
      else clearDiffSelection();
      message.success(reverse ? t("workspace.controller.hunkUnstaged") : t("workspace.controller.hunkStaged"));
    } catch (error) {
      reportError(error, reverse
        ? t("workspace.controller.couldNotUnstageHunk")
        : t("workspace.controller.couldNotStageHunk"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function diffHunkAtLine(lineIndex: number): DiffHunk | undefined {
    return diffHunksByLine.value.get(lineIndex);
  }

  async function applyDiffHunkAtLine(lineIndex: number): Promise<void> {
    const hunk = diffHunkAtLine(lineIndex);
    if (hunk) await applyDiffHunk(hunk);
  }

  function updateChangeInclusion(entry: GitStatusEntry, included: boolean): void {
    void setChangeIncluded(entry, included);
  }

  function updateAllChangeInclusion(included: boolean): void {
    void (included ? stageAll() : unstageAll());
  }

  async function setChangeIncluded(entry: GitStatusEntry, included: boolean): Promise<void> {
    if (!activeRepositoryPath.value) return;
    mutationBusy.value = true;
    try {
      const wasSelected = selectedChange.value?.path === entry.path;
      const nextSnapshot = included
        ? await api.stagePaths(activeRepositoryPath.value, [entry.path])
        : await api.unstagePaths(activeRepositoryPath.value, [entry.path]);
      applySnapshot(nextSnapshot);
      if (wasSelected) await syncSelectedDiff(nextSnapshot, entry.path);
    } catch (error) {
      reportError(error, included
        ? t("workspace.controller.couldNotStageChanges")
        : t("workspace.controller.couldNotUnstageChanges"));
    } finally {
      mutationBusy.value = false;
    }
  }

  async function stageAll(): Promise<void> {
    if (!activeRepositoryPath.value || !workingChanges.value.length) return;
    mutationBusy.value = true;
    try {
      const selectedPath = selectedChange.value?.path;
      const nextSnapshot = await api.stagePaths(
        activeRepositoryPath.value,
        workingChanges.value.map((entry) => entry.path),
      );
      applySnapshot(nextSnapshot);
      if (selectedPath) await syncSelectedDiff(nextSnapshot, selectedPath);
      message.success(t("workspace.controller.allWorkingTreeChangesAreStaged"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotStageChanges"));
    } finally {
      mutationBusy.value = false;
    }
  }

  async function unstageAll(): Promise<void> {
    if (!activeRepositoryPath.value || !stagedChanges.value.length) return;
    mutationBusy.value = true;
    try {
      const selectedPath = selectedChange.value?.path;
      const nextSnapshot = await api.unstagePaths(
        activeRepositoryPath.value,
        stagedChanges.value.map((entry) => entry.path),
      );
      applySnapshot(nextSnapshot);
      if (selectedPath) await syncSelectedDiff(nextSnapshot, selectedPath);
      message.success(t("workspace.controller.allFilesAreRemovedFromTheStagingArea"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotUnstageChanges"));
    } finally {
      mutationBusy.value = false;
    }
  }

  async function confirmDiscardChanges(paths: string[]): Promise<void> {
    if (!activeRepositoryPath.value || !paths.length) return;
    const repositoryPath = activeRepositoryPath.value;
    const uniquePaths = [...new Set(paths)];
    mutationBusy.value = true;
    let discardPreview: Awaited<ReturnType<typeof api.previewDiscardChanges>>;
    try {
      discardPreview = await api.previewDiscardChanges(repositoryPath, uniquePaths);
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotPrepareToDiscardChanges"));
      return;
    } finally {
      mutationBusy.value = false;
    }
    if (activeRepositoryPath.value !== repositoryPath) return;
    dialog.error({
      title: t("workspace.controller.discardChanges"),
      content: t(
        "workspace.controller.permanentlyDiscardWorkingTreeAndStagedChangesInCountFileSThisCannotBeUndoneUntracked",
        {
          count: discardPreview.pathCount,
          untracked: discardPreview.untrackedCount > 0
            ? t("workspace.controller.untrackedFilesAmongThemWillBeDeletedFromDisk")
            : "",
        },
      ),
      positiveText: t("workspace.controller.discardChanges"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        if (activeRepositoryPath.value !== repositoryPath) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        mutationBusy.value = true;
        try {
          applySnapshot(await api.discardChanges(repositoryPath, discardPreview.confirmation));
          if (selectedChange.value && uniquePaths.includes(selectedChange.value.path)) clearDiffSelection();
          message.success(t("workspace.controller.changesDiscarded"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotDiscardChanges"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  function discardSelectedChange(): void {
    if (selectedChange.value) void confirmDiscardChanges([selectedChange.value.path]);
  }

  async function confirmCleanUntracked(): Promise<void> {
    if (!activeRepositoryPath.value) return;
    mutationBusy.value = true;
    let cleanPreview: Awaited<ReturnType<typeof api.previewCleanUntracked>>;
    try {
      cleanPreview = await api.previewCleanUntracked(activeRepositoryPath.value);
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotPreviewUntrackedFileCleanup"));
      return;
    } finally {
      mutationBusy.value = false;
    }
    const entries = cleanPreview.preview.split(/\r?\n/u).filter(Boolean);
    if (!entries.length) {
      message.info(t("workspace.controller.thereAreNoUntrackedFilesToClean"));
      return;
    }
    dialog.error({
      title: t("workspace.controller.cleanUntrackedFiles"),
      content: t(
        "workspace.controller.permanentlyDeleteCountUntrackedFileSOrFolderSIgnoredFilesAreNotRemovedAndThisCannotBeUndone",
        { count: entries.length },
      ),
      positiveText: t("workspace.controller.deletePermanently"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          const selectedPath = selectedChange.value?.path;
          const nextSnapshot = await api.cleanUntracked(activeRepositoryPath.value!, cleanPreview.confirmation);
          applySnapshot(nextSnapshot);
          if (selectedPath && !nextSnapshot.status.entries.some((entry) => entry.path === selectedPath)) {
            clearDiffSelection();
          }
          message.success(t("workspace.controller.untrackedFilesCleaned"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotCleanUntrackedFiles"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  async function setAmendLastCommit(value: boolean): Promise<void> {
    if (!value) {
      amendLastCommit.value = false;
      return;
    }
    if (!canAmendLatestCommit.value) {
      message.warning(t("workspace.controller.thisBranchHasNoCommitToAmendYet"));
      return;
    }
    amendLastCommit.value = true;
    await loadHistory();
    if (!commitMessage.value.trim() && commits.value[0]) commitMessage.value = commits.value[0].summary;
  }

  async function commitChanges(): Promise<void> {
    if (!activeRepositoryPath.value) return;
    if (!commitMessage.value.trim()) {
      message.warning(t("workspace.controller.enterACommitMessage"));
      return;
    }
    mutationBusy.value = true;
    try {
      const selectedPath = selectedChange.value?.path;
      const nextSnapshot = await api.commitChanges(
        activeRepositoryPath.value,
        commitMessage.value.trim(),
        amendLastCommit.value,
      );
      applySnapshot(nextSnapshot);
      await syncSelectedDiff(nextSnapshot, selectedPath);
      commitMessage.value = "";
      const amended = amendLastCommit.value;
      amendLastCommit.value = false;
      await loadHistory();
      message.success(amended
        ? t("workspace.controller.latestCommitAmended")
        : t("workspace.controller.commitCreated"));
    } catch (error) {
      reportError(error, t("workspace.controller.commitFailed"));
    } finally {
      mutationBusy.value = false;
    }
  }

  return {
    selectedChange,
    diffText,
    selectedDiffStaged,
    blameText,
    diffLoading,
    blameLoading,
    blameModalShown,
    commitMessage,
    amendLastCommit,
    stagedChanges,
    stagedCount,
    unstagedCount,
    workingChanges,
    untrackedChanges,
    untrackedCount,
    allChangesIncluded,
    someChangesIncluded,
    conflictedChanges,
    diffHunks,
    diffLines,
    hasDiffText,
    canAmendLatestCommit,
    isUnbornRepository,
    commitActionLabel,
    commitSelectionSummary,
    clearDiffSelection,
    getDiffRequestGeneration,
    syncSelectedDiff,
    loadDiff,
    openFileBlame,
    applyDiffHunk,
    diffHunkAtLine,
    applyDiffHunkAtLine,
    updateChangeInclusion,
    updateAllChangeInclusion,
    setChangeIncluded,
    stageAll,
    unstageAll,
    confirmDiscardChanges,
    discardSelectedChange,
    confirmCleanUntracked,
    setAmendLastCommit,
    commitChanges,
  };
}
