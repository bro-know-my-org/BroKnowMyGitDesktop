import { computed, h, reactive, ref, type ComputedRef, type Ref } from "vue";
import { NButton, type DialogApi, type MessageApi } from "naive-ui";
import { api, isAppError } from "../../../shared/api/tauri";
import { t } from "../../../shared/i18n";
import type {
  BranchInfo,
  CommitSummary,
  GitOperationKind,
  GitStatusEntry,
  RemoteBranchInfo,
  RepositorySnapshot,
  ResetMode,
  StashEntry,
} from "../../../shared/types/domain";

interface RepositoryOperationsDependencies {
  activeRepositoryPath: Ref<string | null>;
  snapshot: Ref<RepositorySnapshot | null>;
  selectedChange: Ref<GitStatusEntry | null>;
  conflictedChanges: ComputedRef<GitStatusEntry[]>;
  currentBranch: ComputedRef<string>;
  commits: Ref<CommitSummary[]>;
  mutationBusy: Ref<boolean>;
  getSelectionGeneration: () => number;
  repositorySelectionIsCurrent: (repositoryPath: string, generation: number) => boolean;
  applySnapshot: (snapshot: RepositorySnapshot) => boolean;
  syncSelectedDiff: (snapshot: RepositorySnapshot, selectedPath?: string) => Promise<void>;
  loadHistory: () => Promise<void>;
  refreshSnapshot: () => Promise<void>;
  refreshAfterGitConflict: (error: unknown) => Promise<void>;
  reconcileSnapshotAfterMutationError: (repositoryPath: string) => Promise<void>;
  gitOperationLabel: (kind: GitOperationKind) => string;
  reportError: (error: unknown, action: string) => void;
  message: MessageApi;
  dialog: DialogApi;
}

export function useRepositoryOperations({
  activeRepositoryPath,
  snapshot,
  selectedChange,
  conflictedChanges,
  currentBranch,
  commits,
  mutationBusy,
  getSelectionGeneration,
  repositorySelectionIsCurrent,
  applySnapshot,
  syncSelectedDiff,
  loadHistory,
  refreshSnapshot,
  refreshAfterGitConflict,
  reconcileSnapshotAfterMutationError,
  gitOperationLabel,
  reportError,
  message,
  dialog,
}: RepositoryOperationsDependencies) {
  const branchModalShown = ref(false);
  const renameBranchModalShown = ref(false);
  const stashModalShown = ref(false);
  const rebaseModalShown = ref(false);
  const cherryPickModalShown = ref(false);
  const squashModalShown = ref(false);
  const tagModalShown = ref(false);
  const branchForm = reactive({ name: "", checkout: true });
  const renameBranchForm = reactive({ oldName: "", newName: "" });
  const stashForm = reactive({ message: "", includeUntracked: true });
  const rebaseForm = reactive({ base: "" });
  const cherryPickForm = reactive({ revision: "" });
  const squashForm = reactive({ count: "2", message: "" });
  const tagForm = reactive({ name: "", target: "", message: "" });
  const tagRepositoryPath = ref<string | null>(null);

  const rebaseBaseOptions = computed(() => snapshot.value?.branches
    .filter((branch) => !branch.isCurrent)
    .map((branch) => ({ label: branch.name, value: branch.name })) ?? []);
  const tagTargetOptions = computed(() => {
    const options = [
      { label: t("workspace.controller.currentHead"), value: "" },
      ...(snapshot.value?.branches.map((branch) => ({ label: branch.name, value: branch.name })) ?? []),
    ];
    const target = tagForm.target.trim();
    if (target && !options.some((option) => option.value === target)) {
      options.push({
        label: t("workspace.controller.commitSha", { sha: target.slice(0, 12) }),
        value: target,
      });
    }
    return options;
  });

  function requestBranchSwitch(target: string, execute: (stashLocalChanges: boolean) => Promise<void>): void {
    if (snapshot.value?.operation || conflictedChanges.value.length) {
      message.warning(t("workspace.controller.finishOrAbortTheCurrentGitOperationBeforeSwitchingBranches"));
      return;
    }
    const changeCount = snapshot.value?.status.entries.length ?? 0;
    if (!changeCount) {
      void execute(false);
      return;
    }
    const branchSwitchDialog = dialog.warning({
      title: t("workspace.controller.stashChangesBeforeSwitching"),
      content: t(
        "workspace.controller.thereAreCountUncommittedChangeSBeforeSwitchingToBranchStashAndSwitchSavesStagedUnstagedAndUntrac",
        { count: changeCount, branch: target },
      ),
      closable: true,
      action: () => [
        h(NButton, { onClick: () => branchSwitchDialog.destroy() }, { default: () => t("workspace.controller.cancel") }),
        h(NButton, {
          tertiary: true,
          onClick: () => {
            branchSwitchDialog.destroy();
            void execute(false);
          },
        }, { default: () => t("workspace.controller.switchDirectly") }),
        h(NButton, {
          type: "primary",
          onClick: () => {
            branchSwitchDialog.destroy();
            void execute(true);
          },
        }, { default: () => t("workspace.controller.stashAndSwitch") }),
      ],
    });
  }

  async function switchLocalBranch(branch: BranchInfo, stashLocalChanges: boolean): Promise<void> {
    if (!activeRepositoryPath.value || branch.isCurrent) return;
    const repositoryPath = activeRepositoryPath.value;
    mutationBusy.value = true;
    try {
      const selectedPath = selectedChange.value?.path;
      const nextSnapshot = await api.checkoutBranch(repositoryPath, branch.name, stashLocalChanges);
      if (activeRepositoryPath.value !== repositoryPath) return;
      applySnapshot(nextSnapshot);
      await syncSelectedDiff(nextSnapshot, selectedPath);
      if (stashLocalChanges) await loadHistory();
      message.success(stashLocalChanges
        ? t("workspace.controller.changesStashedAndSwitchedToBranch", { branch: branch.name })
        : t("workspace.controller.switchedToBranch", { branch: branch.name }));
    } catch (error) {
      await reconcileSnapshotAfterMutationError(repositoryPath);
      if (stashLocalChanges && isAppError(error) && error.code === "branch_switch_changes_stashed") {
        await loadHistory().catch(() => undefined);
      }
      reportError(error, t("workspace.controller.couldNotSwitchBranch"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function checkoutBranch(branch: BranchInfo): void {
    if (!activeRepositoryPath.value || branch.isCurrent) return;
    requestBranchSwitch(branch.name, (stashLocalChanges) => switchLocalBranch(branch, stashLocalChanges));
  }

  async function switchRemoteBranch(remoteBranch: RemoteBranchInfo, stashLocalChanges: boolean): Promise<void> {
    if (!activeRepositoryPath.value) return;
    const repositoryPath = activeRepositoryPath.value;
    mutationBusy.value = true;
    try {
      const selectedPath = selectedChange.value?.path;
      const nextSnapshot = await api.checkoutRemoteBranch(
        repositoryPath,
        remoteBranch.remoteName,
        remoteBranch.name,
        stashLocalChanges,
      );
      if (activeRepositoryPath.value !== repositoryPath) return;
      applySnapshot(nextSnapshot);
      await syncSelectedDiff(nextSnapshot, selectedPath);
      if (stashLocalChanges) await loadHistory();
      const branchLabel = `${remoteBranch.remoteName}/${remoteBranch.name}`;
      message.success(stashLocalChanges
        ? t("workspace.controller.changesStashedAndSwitchedToBranch", { branch: branchLabel })
        : t("workspace.controller.switchedToBranch", { branch: branchLabel }));
    } catch (error) {
      await reconcileSnapshotAfterMutationError(repositoryPath);
      if (stashLocalChanges && isAppError(error) && error.code === "branch_switch_changes_stashed") {
        await loadHistory().catch(() => undefined);
      }
      reportError(error, t("workspace.controller.couldNotSwitchRemoteBranch"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function checkoutRemoteBranch(remoteBranch: RemoteBranchInfo): void {
    if (!activeRepositoryPath.value) return;
    const matchingLocalBranch = snapshot.value?.branches.find((branch) => branch.name === remoteBranch.name);
    if (matchingLocalBranch?.isCurrent) return;
    requestBranchSwitch(
      `${remoteBranch.remoteName}/${remoteBranch.name}`,
      (stashLocalChanges) => switchRemoteBranch(remoteBranch, stashLocalChanges),
    );
  }

  async function createBranch(): Promise<void> {
    if (!activeRepositoryPath.value || !branchForm.name.trim()) {
      message.warning(t("workspace.controller.enterABranchName"));
      return;
    }
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    mutationBusy.value = true;
    try {
      const nextSnapshot = await api.createBranch({
        repositoryPath,
        name: branchForm.name.trim(),
        checkout: branchForm.checkout,
      });
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      applySnapshot(nextSnapshot);
      branchModalShown.value = false;
      branchForm.name = "";
      branchForm.checkout = true;
      message.success(t("workspace.controller.branchCreated"));
    } catch (error) {
      await reconcileSnapshotAfterMutationError(repositoryPath);
      reportError(error, t("workspace.controller.couldNotCreateBranch"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function openRenameBranchModal(branch: BranchInfo): void {
    renameBranchForm.oldName = branch.name;
    renameBranchForm.newName = branch.name;
    renameBranchModalShown.value = true;
  }

  async function renameBranch(): Promise<void> {
    if (!activeRepositoryPath.value || !renameBranchForm.newName.trim()) {
      message.warning(t("workspace.controller.enterANewBranchName"));
      return;
    }
    mutationBusy.value = true;
    try {
      applySnapshot(await api.renameBranch(
        activeRepositoryPath.value,
        renameBranchForm.oldName,
        renameBranchForm.newName.trim(),
      ));
      renameBranchModalShown.value = false;
      message.success(t("workspace.controller.branchRenamed"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotRenameBranch"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function confirmDeleteBranch(branch: BranchInfo): void {
    if (!activeRepositoryPath.value || branch.isCurrent) return;
    dialog.warning({
      title: t("workspace.controller.deleteBranch"),
      content: t(
        "workspace.controller.deleteLocalBranchBranchGitWillRejectAnUnmergedBranchToPreventAccidentalLossOfCommits",
        { branch: branch.name },
      ),
      positiveText: t("workspace.controller.delete"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          applySnapshot(await api.deleteBranch(activeRepositoryPath.value!, branch.name, false));
          message.success(t("workspace.controller.branchDeleted"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotDeleteBranch"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  function confirmMergeBranch(branch: BranchInfo): void {
    if (!activeRepositoryPath.value) return;
    dialog.warning({
      title: t("workspace.controller.mergeBranch"),
      content: t(
        "workspace.controller.mergeBranchIntoTheCurrentBranchCurrentIfConflictsOccurGitLeavesTheWorkingTreeAvailableForResolut",
        { branch: branch.name, current: currentBranch.value },
      ),
      positiveText: t("workspace.controller.merge"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          await api.mergeBranch(activeRepositoryPath.value!, branch.name);
          await refreshSnapshot();
          message.success(t("workspace.controller.mergedBranch", { branch: branch.name }));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotMergeBranch"));
          await refreshAfterGitConflict(error);
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  function openRebaseModal(): void {
    rebaseForm.base = rebaseBaseOptions.value[0]?.value ?? "";
    rebaseModalShown.value = true;
  }

  async function startRebase(): Promise<void> {
    if (!activeRepositoryPath.value || !rebaseForm.base) {
      message.warning(t("workspace.controller.selectALocalBranchToRebaseOnto"));
      return;
    }
    mutationBusy.value = true;
    try {
      applySnapshot(await api.startRebase(activeRepositoryPath.value, rebaseForm.base));
      rebaseModalShown.value = false;
      await loadHistory();
      message.success(snapshot.value?.operation
        ? t("workspace.controller.rebaseStartedContinueAfterResolvingAnyConflicts")
        : t("workspace.controller.rebaseCompleted"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotRebaseBranch"));
      await refreshAfterGitConflict(error);
    } finally {
      mutationBusy.value = false;
    }
  }

  function openCherryPickModal(revision = ""): void {
    cherryPickForm.revision = revision;
    cherryPickModalShown.value = true;
  }

  async function cherryPickCommits(): Promise<void> {
    if (!activeRepositoryPath.value || !cherryPickForm.revision.trim()) {
      message.warning(t("workspace.controller.enterACommitShaOrRevisionToCherryPick"));
      return;
    }
    mutationBusy.value = true;
    try {
      applySnapshot(await api.cherryPickCommits(activeRepositoryPath.value, [cherryPickForm.revision.trim()]));
      cherryPickModalShown.value = false;
      await loadHistory();
      message.success(t("workspace.controller.commitCherryPicked"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotCherryPickCommit"));
      await refreshAfterGitConflict(error);
    } finally {
      mutationBusy.value = false;
    }
  }

  async function continueGitOperation(): Promise<void> {
    if (!activeRepositoryPath.value || !snapshot.value?.operation) return;
    if (snapshot.value.operation.hasConflicts || conflictedChanges.value.length) {
      message.warning(t("workspace.controller.resolveAndStageAllConflictedFilesFirst"));
      return;
    }
    const operation = snapshot.value.operation;
    mutationBusy.value = true;
    try {
      applySnapshot(await api.continueGitOperation(activeRepositoryPath.value, operation.kind));
      await loadHistory();
      message.success(t("workspace.controller.gitOperationContinued"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotContinueGitOperation"));
      await refreshAfterGitConflict(error);
    } finally {
      mutationBusy.value = false;
    }
  }

  function confirmAbortGitOperation(): void {
    if (!activeRepositoryPath.value || !snapshot.value?.operation) return;
    const operation = snapshot.value.operation;
    dialog.error({
      title: t("workspace.controller.abortGitOperation"),
      content: t(
        "workspace.controller.abortTheCurrentOperationGitWillTryToRestoreTheStateFromBeforeItStartedUnresolvedConflictWorkMayB",
        { operation: gitOperationLabel(operation.kind) },
      ),
      positiveText: t("workspace.controller.abortOperation"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          applySnapshot(await api.abortGitOperation(activeRepositoryPath.value!, operation.kind));
          await loadHistory();
          message.success(t("workspace.controller.gitOperationAborted"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotAbortGitOperation"));
          await refreshAfterGitConflict(error);
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  function openStashModal(): void {
    stashForm.message = "";
    stashForm.includeUntracked = true;
    stashModalShown.value = true;
  }

  async function stashChanges(): Promise<void> {
    if (!activeRepositoryPath.value) return;
    mutationBusy.value = true;
    try {
      applySnapshot(await api.stashChanges(
        activeRepositoryPath.value,
        stashForm.message.trim() || undefined,
        stashForm.includeUntracked,
      ));
      stashModalShown.value = false;
      await loadHistory();
      message.success(t("workspace.controller.changesSavedToStash"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotStashChanges"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function confirmPopStash(stash: StashEntry): void {
    if (!activeRepositoryPath.value) return;
    dialog.warning({
      title: t("workspace.controller.popStash"),
      content: t(
        "workspace.controller.applyReferenceToTheCurrentWorkingTreeAndRemoveItFromTheStashIfConflictsOccurGitLeavesTheWorkingT",
        { reference: stash.reference },
      ),
      positiveText: t("workspace.controller.pop"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          applySnapshot(await api.popStash(activeRepositoryPath.value!, stash.reference, stash.commitId));
          await loadHistory();
          message.success(t("workspace.controller.stashRestored"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotPopStash"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  function confirmDropStash(stash: StashEntry): void {
    if (!activeRepositoryPath.value) return;
    dialog.warning({
      title: t("workspace.controller.dropStash"),
      content: t("workspace.controller.permanentlyDeleteReferenceThisCannotBeUndone", { reference: stash.reference }),
      positiveText: t("workspace.controller.drop"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          await api.dropStash(activeRepositoryPath.value!, stash.reference, stash.commitId);
          await loadHistory();
          message.success(t("workspace.controller.stashDropped"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotDropStash"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  function confirmResetCommit(commit: CommitSummary, mode: ResetMode): void {
    if (!activeRepositoryPath.value) return;
    const modeLabel = mode === "soft"
      ? t("workspace.controller.softKeepIndexAndWorkingTree")
      : mode === "mixed"
        ? t("workspace.controller.mixedKeepWorkingTreeUnstageChanges")
        : t("workspace.controller.hardDiscardIndexAndWorkingTreeChanges");
    dialog.error({
      title: t("workspace.controller.resetBranch"),
      content: t(
        "workspace.controller.resetTheCurrentBranchToCommitModeAHardResetCanPermanentlyDiscardUncommittedChanges",
        { commit: commit.shortSha, mode: modeLabel },
      ),
      positiveText: t("workspace.controller.confirmResetBranch"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          applySnapshot(await api.resetBranch(activeRepositoryPath.value!, commit.sha, mode));
          await loadHistory();
          message.success(t("workspace.controller.branchReset"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotResetBranch"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  function openSquashModal(): void {
    if (commits.value.length < 3) {
      message.warning(t("workspace.controller.atLeastThreeCommitsAreNeededToPreserveABaseCommitWhileSquashingRecentCommits"));
      return;
    }
    squashForm.count = "2";
    squashForm.message = commits.value[0]?.summary || "";
    squashModalShown.value = true;
  }

  async function squashRecentCommits(): Promise<void> {
    if (!activeRepositoryPath.value) return;
    const count = Number(squashForm.count);
    if (!Number.isInteger(count) || count < 2 || count >= commits.value.length || !squashForm.message.trim()) {
      message.warning(t("workspace.controller.enterAValidCountAtLeast2WhileKeepingABaseCommitAndCommitMessage"));
      return;
    }
    mutationBusy.value = true;
    try {
      applySnapshot(await api.squashRecentCommits(activeRepositoryPath.value, count, squashForm.message.trim()));
      squashModalShown.value = false;
      await loadHistory();
      message.success(t("workspace.controller.recentCommitsSquashed"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotSquashCommits"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function confirmRevertCommit(commit: CommitSummary): void {
    if (!activeRepositoryPath.value) return;
    dialog.warning({
      title: t("workspace.controller.revertCommit"),
      content: t(
        "workspace.controller.createANewCommitThatRevertsCommitThisDoesNotRewriteExistingHistory",
        { commit: commit.shortSha },
      ),
      positiveText: t("workspace.controller.createRevertCommit"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          applySnapshot(await api.revertCommit(activeRepositoryPath.value!, commit.sha));
          await loadHistory();
          message.success(t("workspace.controller.revertCommitCreated"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotRevertCommit"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  function openTagModal(target = ""): void {
    if (!activeRepositoryPath.value) return;
    tagRepositoryPath.value = activeRepositoryPath.value;
    tagForm.name = "";
    tagForm.target = target;
    tagForm.message = "";
    tagModalShown.value = true;
  }

  async function createTag(): Promise<void> {
    if (!tagForm.name.trim()) {
      message.warning(t("workspace.controller.enterATagName"));
      return;
    }
    const repositoryPath = tagRepositoryPath.value;
    if (!repositoryPath || activeRepositoryPath.value !== repositoryPath) {
      tagModalShown.value = false;
      tagRepositoryPath.value = null;
      message.warning(t("workspace.controller.theRepositoryChangedSelectTheTagTargetAgain"));
      return;
    }
    mutationBusy.value = true;
    try {
      await api.createTag(
        repositoryPath,
        tagForm.name.trim(),
        tagForm.target.trim() || undefined,
        tagForm.message.trim() || undefined,
      );
      tagModalShown.value = false;
      tagRepositoryPath.value = null;
      if (activeRepositoryPath.value === repositoryPath) {
        await refreshSnapshot();
        message.success(t("workspace.controller.tagCreated"));
      }
    } catch (error) {
      if (activeRepositoryPath.value === repositoryPath) {
        reportError(error, t("workspace.controller.couldNotCreateTag"));
      }
    } finally {
      mutationBusy.value = false;
    }
  }

  function confirmDeleteTag(name: string): void {
    if (!activeRepositoryPath.value) return;
    dialog.warning({
      title: t("workspace.controller.deleteLocalTag"),
      content: t("workspace.controller.deleteLocalTagTagThisDoesNotDeleteARemoteTag", { tag: name }),
      positiveText: t("workspace.controller.delete"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          await api.deleteTag(activeRepositoryPath.value!, name);
          await refreshSnapshot();
          message.success(t("workspace.controller.tagDeleted"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotDeleteTag"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  function resetRepositoryOperations(): void {
    tagModalShown.value = false;
    tagRepositoryPath.value = null;
  }

  return {
    branchModalShown,
    renameBranchModalShown,
    stashModalShown,
    rebaseModalShown,
    cherryPickModalShown,
    squashModalShown,
    tagModalShown,
    branchForm,
    renameBranchForm,
    stashForm,
    rebaseForm,
    cherryPickForm,
    squashForm,
    tagForm,
    tagRepositoryPath,
    rebaseBaseOptions,
    tagTargetOptions,
    checkoutBranch,
    checkoutRemoteBranch,
    createBranch,
    openRenameBranchModal,
    renameBranch,
    confirmDeleteBranch,
    confirmMergeBranch,
    openRebaseModal,
    startRebase,
    openCherryPickModal,
    cherryPickCommits,
    continueGitOperation,
    confirmAbortGitOperation,
    openStashModal,
    stashChanges,
    confirmPopStash,
    confirmDropStash,
    confirmResetCommit,
    openSquashModal,
    squashRecentCommits,
    confirmRevertCommit,
    openTagModal,
    createTag,
    confirmDeleteTag,
    resetRepositoryOperations,
  };
}
