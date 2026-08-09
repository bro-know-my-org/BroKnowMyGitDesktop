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
import { acquireRepositoryOperation, isRepositoryOperationInFlight } from "./repositoryOperationLock";

interface RepositoryOperationsDependencies {
  activeRepositoryPath: Ref<string | null>;
  snapshot: Ref<RepositorySnapshot | null>;
  selectedChange: Ref<GitStatusEntry | null>;
  conflictedChanges: ComputedRef<GitStatusEntry[]>;
  currentBranch: ComputedRef<string>;
  commits: Ref<CommitSummary[]>;
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
  const branchForm = reactive({ name: "", checkout: true, stashLocalChanges: false });
  const renameBranchForm = reactive({ oldName: "", newName: "" });
  const stashForm = reactive({ message: "", includeUntracked: true });
  const rebaseForm = reactive({ base: "" });
  const cherryPickForm = reactive({ revision: "" });
  const squashForm = reactive({ count: "2", message: "" });
  const tagForm = reactive({ name: "", target: "", message: "" });
  const tagRepositoryPath = ref<string | null>(null);
  const tagRepositorySelectionGeneration = ref<number | null>(null);

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

  function currentRepositoryOperationInFlight(): boolean {
    const repositoryPath = activeRepositoryPath.value;
    return Boolean(repositoryPath && isRepositoryOperationInFlight(repositoryPath));
  }

  function beginRepositoryMutation(repositoryPath: string): (() => void) | null {
    const releaseOperation = acquireRepositoryOperation(repositoryPath);
    if (!releaseOperation) {
      message.warning(t("workspace.controller.aRepositoryOperationIsAlreadyInProgressWaitForItToFinish"));
      return null;
    }
    return () => {
      releaseOperation();
    };
  }

  function requestBranchSwitch(
    target: string,
    repositoryPath: string,
    selectionGeneration: number,
    execute: (stashLocalChanges: boolean) => Promise<void>,
  ): void {
    if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
      return;
    }
    if (currentRepositoryOperationInFlight()) {
      message.warning(t("workspace.controller.aRepositoryOperationIsAlreadyInProgressWaitForItToFinish"));
      return;
    }
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

  async function switchLocalBranch(
    branch: BranchInfo,
    stashLocalChanges: boolean,
    repositoryPath: string,
    selectionGeneration: number,
  ): Promise<void> {
    if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
      return;
    }
    if (branch.isCurrent) return;
    if (currentRepositoryOperationInFlight()) {
      message.warning(t("workspace.controller.aRepositoryOperationIsAlreadyInProgressWaitForItToFinish"));
      return;
    }
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const selectedPath = selectedChange.value?.path;
      const nextSnapshot = await api.checkoutBranch(repositoryPath, branch.name, stashLocalChanges);
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
      await syncSelectedDiff(nextSnapshot, selectedPath);
      if (stashLocalChanges) await loadHistory();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      message.success(stashLocalChanges
        ? t("workspace.controller.changesStashedAndSwitchedToBranch", { branch: branch.name })
        : t("workspace.controller.switchedToBranch", { branch: branch.name }));
    } catch (error) {
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      await reconcileSnapshotAfterMutationError(repositoryPath);
      if (stashLocalChanges && isAppError(error) && error.code === "branch_switch_changes_stashed") {
        await loadHistory().catch(() => undefined);
      }
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      reportError(error, t("workspace.controller.couldNotSwitchBranch"));
    } finally {
      finishMutation();
    }
  }

  function checkoutBranch(branch: BranchInfo): void {
    if (!activeRepositoryPath.value || branch.isCurrent) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    requestBranchSwitch(
      branch.name,
      repositoryPath,
      selectionGeneration,
      (stashLocalChanges) => switchLocalBranch(branch, stashLocalChanges, repositoryPath, selectionGeneration),
    );
  }

  async function switchRemoteBranch(
    remoteBranch: RemoteBranchInfo,
    stashLocalChanges: boolean,
    repositoryPath: string,
    selectionGeneration: number,
  ): Promise<void> {
    if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
      return;
    }
    if (currentRepositoryOperationInFlight()) {
      message.warning(t("workspace.controller.aRepositoryOperationIsAlreadyInProgressWaitForItToFinish"));
      return;
    }
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const selectedPath = selectedChange.value?.path;
      const nextSnapshot = await api.checkoutRemoteBranch(
        repositoryPath,
        remoteBranch.remoteName,
        remoteBranch.name,
        stashLocalChanges,
      );
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
      await syncSelectedDiff(nextSnapshot, selectedPath);
      if (stashLocalChanges) await loadHistory();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      const branchLabel = `${remoteBranch.remoteName}/${remoteBranch.name}`;
      message.success(stashLocalChanges
        ? t("workspace.controller.changesStashedAndSwitchedToBranch", { branch: branchLabel })
        : t("workspace.controller.switchedToBranch", { branch: branchLabel }));
    } catch (error) {
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      await reconcileSnapshotAfterMutationError(repositoryPath);
      if (stashLocalChanges && isAppError(error) && error.code === "branch_switch_changes_stashed") {
        await loadHistory().catch(() => undefined);
      }
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      reportError(error, t("workspace.controller.couldNotSwitchRemoteBranch"));
    } finally {
      finishMutation();
    }
  }

  function checkoutRemoteBranch(remoteBranch: RemoteBranchInfo): void {
    if (!activeRepositoryPath.value) return;
    const repositoryPath = activeRepositoryPath.value;
    const matchingLocalBranch = snapshot.value?.branches.find((branch) => branch.name === remoteBranch.name);
    if (matchingLocalBranch?.isCurrent) return;
    const selectionGeneration = getSelectionGeneration();
    requestBranchSwitch(
      `${remoteBranch.remoteName}/${remoteBranch.name}`,
      repositoryPath,
      selectionGeneration,
      (stashLocalChanges) => switchRemoteBranch(remoteBranch, stashLocalChanges, repositoryPath, selectionGeneration),
    );
  }

  async function createBranch(): Promise<void> {
    if (!activeRepositoryPath.value || !branchForm.name.trim()) {
      message.warning(t("workspace.controller.enterABranchName"));
      return;
    }
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    const branchName = branchForm.name.trim();
    const checkedOutNewBranch = branchForm.checkout;
    const stashedLocalChanges = checkedOutNewBranch && branchForm.stashLocalChanges;
    try {
      const branchResult = await api.createBranch({
        repositoryPath,
        name: branchName,
        checkout: checkedOutNewBranch,
        stashLocalChanges: stashedLocalChanges,
      });
      const nextSnapshot = branchResult.snapshot;
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
      branchModalShown.value = false;
      branchForm.name = "";
      branchForm.checkout = true;
      branchForm.stashLocalChanges = false;
      if (branchResult.stashedLocalChanges) await loadHistory();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      let successMessage = t("workspace.controller.branchCreated");
      if (branchResult.stashedLocalChanges) {
        successMessage = t("workspace.controller.branchCreatedAndChangesStashed");
      } else if (checkedOutNewBranch) {
        successMessage = t("workspace.controller.branchCreatedAndSwitched");
      }
      message.success(successMessage);
    } catch (error) {
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      await reconcileSnapshotAfterMutationError(repositoryPath);
      if (isAppError(error) && error.code === "branch_switch_changes_stashed") {
        await loadHistory().catch(() => undefined);
      }
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      reportError(error, t("workspace.controller.couldNotCreateBranch"));
    } finally {
      finishMutation();
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
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    if (!repositoryPath) return;
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const nextSnapshot = await api.renameBranch(
        repositoryPath,
        renameBranchForm.oldName,
        renameBranchForm.newName.trim(),
      );
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
      renameBranchModalShown.value = false;
      message.success(t("workspace.controller.branchRenamed"));
    } catch (error) {
      if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
        reportError(error, t("workspace.controller.couldNotRenameBranch"));
      }
    } finally {
      finishMutation();
    }
  }

  function confirmDeleteBranch(branch: BranchInfo): void {
    if (!activeRepositoryPath.value || branch.isCurrent) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    dialog.warning({
      title: t("workspace.controller.deleteBranch"),
      content: t(
        "workspace.controller.deleteLocalBranchBranchGitWillRejectAnUnmergedBranchToPreventAccidentalLossOfCommits",
        { branch: branch.name },
      ),
      positiveText: t("workspace.controller.delete"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        const finishMutation = beginRepositoryMutation(repositoryPath);
        if (!finishMutation) return;
        try {
          const nextSnapshot = await api.deleteBranch(repositoryPath, branch.name, false);
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
          message.success(t("workspace.controller.branchDeleted"));
        } catch (error) {
          if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
            reportError(error, t("workspace.controller.couldNotDeleteBranch"));
          }
        } finally {
          finishMutation();
        }
      },
    });
  }

  function confirmMergeBranch(branch: BranchInfo): void {
    if (!activeRepositoryPath.value) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    dialog.warning({
      title: t("workspace.controller.mergeBranch"),
      content: t(
        "workspace.controller.mergeBranchIntoTheCurrentBranchCurrentIfConflictsOccurGitLeavesTheWorkingTreeAvailableForResolut",
        { branch: branch.name, current: currentBranch.value },
      ),
      positiveText: t("workspace.controller.merge"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        const finishMutation = beginRepositoryMutation(repositoryPath);
        if (!finishMutation) return;
        try {
          await api.mergeBranch(repositoryPath, branch.name);
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          await refreshSnapshot();
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          message.success(t("workspace.controller.mergedBranch", { branch: branch.name }));
        } catch (error) {
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          reportError(error, t("workspace.controller.couldNotMergeBranch"));
          await refreshAfterGitConflict(error);
        } finally {
          finishMutation();
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
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    if (!repositoryPath) return;
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const nextSnapshot = await api.startRebase(repositoryPath, rebaseForm.base);
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
      rebaseModalShown.value = false;
      await loadHistory();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      message.success(snapshot.value?.operation
        ? t("workspace.controller.rebaseStartedContinueAfterResolvingAnyConflicts")
        : t("workspace.controller.rebaseCompleted"));
    } catch (error) {
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      reportError(error, t("workspace.controller.couldNotRebaseBranch"));
      await refreshAfterGitConflict(error);
    } finally {
      finishMutation();
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
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    if (!repositoryPath) return;
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const nextSnapshot = await api.cherryPickCommits(repositoryPath, [cherryPickForm.revision.trim()]);
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
      cherryPickModalShown.value = false;
      await loadHistory();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      message.success(t("workspace.controller.commitCherryPicked"));
    } catch (error) {
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      reportError(error, t("workspace.controller.couldNotCherryPickCommit"));
      await refreshAfterGitConflict(error);
    } finally {
      finishMutation();
    }
  }

  async function continueGitOperation(): Promise<void> {
    if (!activeRepositoryPath.value || !snapshot.value?.operation) return;
    if (snapshot.value.operation.hasConflicts || conflictedChanges.value.length) {
      message.warning(t("workspace.controller.resolveAndStageAllConflictedFilesFirst"));
      return;
    }
    const operation = snapshot.value.operation;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    if (!repositoryPath) return;
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const nextSnapshot = await api.continueGitOperation(repositoryPath, operation.kind);
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
      await loadHistory();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      message.success(t("workspace.controller.gitOperationContinued"));
    } catch (error) {
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      reportError(error, t("workspace.controller.couldNotContinueGitOperation"));
      await refreshAfterGitConflict(error);
    } finally {
      finishMutation();
    }
  }

  function confirmAbortGitOperation(): void {
    if (!activeRepositoryPath.value || !snapshot.value?.operation) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
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
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        const finishMutation = beginRepositoryMutation(repositoryPath);
        if (!finishMutation) return;
        try {
          const nextSnapshot = await api.abortGitOperation(repositoryPath, operation.kind);
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
          await loadHistory();
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          message.success(t("workspace.controller.gitOperationAborted"));
        } catch (error) {
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          reportError(error, t("workspace.controller.couldNotAbortGitOperation"));
          await refreshAfterGitConflict(error);
        } finally {
          finishMutation();
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
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const nextSnapshot = await api.stashChanges(
        repositoryPath,
        stashForm.message.trim() || undefined,
        stashForm.includeUntracked,
      );
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
      stashModalShown.value = false;
      await loadHistory();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      message.success(t("workspace.controller.changesSavedToStash"));
    } catch (error) {
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      reportError(error, t("workspace.controller.couldNotStashChanges"));
    } finally {
      finishMutation();
    }
  }

  function confirmPopStash(stash: StashEntry): void {
    if (!activeRepositoryPath.value) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    dialog.warning({
      title: t("workspace.controller.popStash"),
      content: t(
        "workspace.controller.applyReferenceToTheCurrentWorkingTreeAndRemoveItFromTheStashIfConflictsOccurGitLeavesTheWorkingT",
        { reference: stash.reference },
      ),
      positiveText: t("workspace.controller.pop"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        const finishMutation = beginRepositoryMutation(repositoryPath);
        if (!finishMutation) return;
        try {
          const nextSnapshot = await api.popStash(repositoryPath, stash.reference, stash.commitId);
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
          await loadHistory();
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          message.success(t("workspace.controller.stashRestored"));
        } catch (error) {
          if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
            reportError(error, t("workspace.controller.couldNotPopStash"));
          }
        } finally {
          finishMutation();
        }
      },
    });
  }

  function confirmDropStash(stash: StashEntry): void {
    if (!activeRepositoryPath.value) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    dialog.warning({
      title: t("workspace.controller.dropStash"),
      content: t("workspace.controller.permanentlyDeleteReferenceThisCannotBeUndone", { reference: stash.reference }),
      positiveText: t("workspace.controller.drop"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        const finishMutation = beginRepositoryMutation(repositoryPath);
        if (!finishMutation) return;
        try {
          await api.dropStash(repositoryPath, stash.reference, stash.commitId);
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          await loadHistory();
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          message.success(t("workspace.controller.stashDropped"));
        } catch (error) {
          if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
            reportError(error, t("workspace.controller.couldNotDropStash"));
          }
        } finally {
          finishMutation();
        }
      },
    });
  }

  function confirmResetCommit(commit: CommitSummary, mode: ResetMode): void {
    if (!activeRepositoryPath.value) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
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
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        const finishMutation = beginRepositoryMutation(repositoryPath);
        if (!finishMutation) return;
        try {
          const nextSnapshot = await api.resetBranch(repositoryPath, commit.sha, mode);
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
          await loadHistory();
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          message.success(t("workspace.controller.branchReset"));
        } catch (error) {
          if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
            reportError(error, t("workspace.controller.couldNotResetBranch"));
          }
        } finally {
          finishMutation();
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
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    if (!repositoryPath) return;
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const nextSnapshot = await api.squashRecentCommits(repositoryPath, count, squashForm.message.trim());
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
      squashModalShown.value = false;
      await loadHistory();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      message.success(t("workspace.controller.recentCommitsSquashed"));
    } catch (error) {
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      reportError(error, t("workspace.controller.couldNotSquashCommits"));
    } finally {
      finishMutation();
    }
  }

  function confirmRevertCommit(commit: CommitSummary): void {
    if (!activeRepositoryPath.value) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    dialog.warning({
      title: t("workspace.controller.revertCommit"),
      content: t(
        "workspace.controller.createANewCommitThatRevertsCommitThisDoesNotRewriteExistingHistory",
        { commit: commit.shortSha },
      ),
      positiveText: t("workspace.controller.createRevertCommit"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        const finishMutation = beginRepositoryMutation(repositoryPath);
        if (!finishMutation) return;
        try {
          const nextSnapshot = await api.revertCommit(repositoryPath, commit.sha);
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
          await loadHistory();
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          message.success(t("workspace.controller.revertCommitCreated"));
        } catch (error) {
          if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
            reportError(error, t("workspace.controller.couldNotRevertCommit"));
          }
        } finally {
          finishMutation();
        }
      },
    });
  }

  function openTagModal(target = ""): void {
    if (!activeRepositoryPath.value) return;
    tagRepositoryPath.value = activeRepositoryPath.value;
    tagRepositorySelectionGeneration.value = getSelectionGeneration();
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
    const selectionGeneration = tagRepositorySelectionGeneration.value;
    if (
      !repositoryPath
      || selectionGeneration === null
      || !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
    ) {
      tagModalShown.value = false;
      tagRepositoryPath.value = null;
      tagRepositorySelectionGeneration.value = null;
      message.warning(t("workspace.controller.theRepositoryChangedSelectTheTagTargetAgain"));
      return;
    }
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      await api.createTag(
        repositoryPath,
        tagForm.name.trim(),
        tagForm.target.trim() || undefined,
        tagForm.message.trim() || undefined,
      );
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      tagModalShown.value = false;
      tagRepositoryPath.value = null;
      tagRepositorySelectionGeneration.value = null;
      await refreshSnapshot();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      message.success(t("workspace.controller.tagCreated"));
    } catch (error) {
      if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
        reportError(error, t("workspace.controller.couldNotCreateTag"));
      }
    } finally {
      finishMutation();
    }
  }

  function confirmDeleteTag(name: string): void {
    if (!activeRepositoryPath.value) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    dialog.warning({
      title: t("workspace.controller.deleteLocalTag"),
      content: t("workspace.controller.deleteLocalTagTagThisDoesNotDeleteARemoteTag", { tag: name }),
      positiveText: t("workspace.controller.delete"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        const finishMutation = beginRepositoryMutation(repositoryPath);
        if (!finishMutation) return;
        try {
          await api.deleteTag(repositoryPath, name);
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          await refreshSnapshot();
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          message.success(t("workspace.controller.tagDeleted"));
        } catch (error) {
          if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
            reportError(error, t("workspace.controller.couldNotDeleteTag"));
          }
        } finally {
          finishMutation();
        }
      },
    });
  }

  function resetRepositoryOperations(): void {
    branchModalShown.value = false;
    renameBranchModalShown.value = false;
    stashModalShown.value = false;
    rebaseModalShown.value = false;
    cherryPickModalShown.value = false;
    squashModalShown.value = false;
    tagModalShown.value = false;
    tagRepositoryPath.value = null;
    tagRepositorySelectionGeneration.value = null;
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
