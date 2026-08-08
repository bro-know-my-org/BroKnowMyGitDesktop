import { reactive, ref, type Ref } from "vue";
import type { DialogApi, MessageApi } from "naive-ui";
import { api } from "../../../shared/api/tauri";
import { t } from "../../../shared/i18n";
import type {
  ExternalEditorConfig,
  LfsStatus,
  RepositorySnapshot,
  SubmoduleInfo,
  WorktreeInfo,
} from "../../../shared/types/domain";
import {
  lfsStatusesAreEqual,
  reuseEqualArray,
  submodulesAreEqual,
  worktreesAreEqual,
} from "./workspaceControllerSupport";
import { acquireRepositoryOperation } from "./repositoryOperationLock";

interface RepositoryAuxiliaryDependencies {
  activeRepositoryPath: Ref<string | null>;
  snapshot: Ref<RepositorySnapshot | null>;
  mutationBusy: Ref<boolean>;
  getSelectionGeneration: () => number;
  repositorySelectionIsCurrent: (repositoryPath: string, generation: number) => boolean;
  applySnapshot: (snapshot: RepositorySnapshot) => boolean;
  loadRepositories: () => Promise<void>;
  refreshSnapshot: () => Promise<void>;
  reportError: (error: unknown, action: string) => void;
  message: MessageApi;
  dialog: DialogApi;
}

export function useRepositoryAuxiliary({
  activeRepositoryPath,
  snapshot,
  mutationBusy,
  getSelectionGeneration,
  repositorySelectionIsCurrent,
  applySnapshot,
  loadRepositories,
  refreshSnapshot,
  reportError,
  message,
  dialog,
}: RepositoryAuxiliaryDependencies) {
  const worktrees = ref<WorktreeInfo[]>([]);
  const submodules = ref<SubmoduleInfo[]>([]);
  const lfsStatus = ref<LfsStatus | null>(null);
  const externalEditor = ref<ExternalEditorConfig | null>(null);
  const worktreesLoading = ref(false);
  const submodulesLoading = ref(false);
  const lfsLoading = ref(false);
  const worktreeModalShown = ref(false);
  const lfsTrackModalShown = ref(false);
  const externalEditorModalShown = ref(false);
  const worktreeForm = reactive({ path: "", mode: "new", branch: "" });
  const lfsTrackForm = reactive({ pattern: "" });
  const externalEditorForm = reactive({ executable: "", arguments: "" });
  let worktreesRequestGeneration = 0;
  let submodulesRequestGeneration = 0;
  let lfsRequestGeneration = 0;

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

  async function loadWorktrees(reportErrors = true, showLoading = true): Promise<void> {
    const repositoryPath = activeRepositoryPath.value;
    if (!repositoryPath) {
      worktrees.value = [];
      return;
    }
    const selectionGeneration = getSelectionGeneration();
    const requestGeneration = ++worktreesRequestGeneration;
    const shouldClearLoading = showLoading || worktreesLoading.value;
    if (showLoading) worktreesLoading.value = true;
    try {
      const nextWorktrees = await api.listWorktrees(repositoryPath);
      if (
        repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && requestGeneration === worktreesRequestGeneration
      ) worktrees.value = reuseEqualArray(worktrees.value, nextWorktrees, worktreesAreEqual);
    } catch (error) {
      if (
        repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && requestGeneration === worktreesRequestGeneration
        && reportErrors
      ) reportError(error, t("workspace.controller.couldNotLoadWorktrees"));
    } finally {
      if (
        shouldClearLoading
        && repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && requestGeneration === worktreesRequestGeneration
      ) worktreesLoading.value = false;
    }
  }

  async function loadSubmodules(reportErrors = true, showLoading = true): Promise<void> {
    const repositoryPath = activeRepositoryPath.value;
    if (!repositoryPath) {
      submodules.value = [];
      return;
    }
    const selectionGeneration = getSelectionGeneration();
    const requestGeneration = ++submodulesRequestGeneration;
    const shouldClearLoading = showLoading || submodulesLoading.value;
    if (showLoading) submodulesLoading.value = true;
    try {
      const nextSubmodules = await api.listSubmodules(repositoryPath);
      if (
        repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && requestGeneration === submodulesRequestGeneration
      ) submodules.value = reuseEqualArray(submodules.value, nextSubmodules, submodulesAreEqual);
    } catch (error) {
      if (
        repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && requestGeneration === submodulesRequestGeneration
        && reportErrors
      ) reportError(error, t("workspace.controller.couldNotLoadSubmodules"));
    } finally {
      if (
        shouldClearLoading
        && repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && requestGeneration === submodulesRequestGeneration
      ) submodulesLoading.value = false;
    }
  }

  async function loadLfsStatus(reportErrors = true, showLoading = true): Promise<void> {
    const repositoryPath = activeRepositoryPath.value;
    if (!repositoryPath) {
      lfsStatus.value = null;
      return;
    }
    const selectionGeneration = getSelectionGeneration();
    const requestGeneration = ++lfsRequestGeneration;
    const shouldClearLoading = showLoading || lfsLoading.value;
    if (showLoading) lfsLoading.value = true;
    try {
      const nextLfsStatus = await api.getLfsStatus(repositoryPath);
      if (
        repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && requestGeneration === lfsRequestGeneration
      ) {
        lfsStatus.value = lfsStatus.value
          ? (lfsStatusesAreEqual(lfsStatus.value, nextLfsStatus) ? lfsStatus.value : nextLfsStatus)
          : nextLfsStatus;
      }
    } catch (error) {
      if (
        repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && requestGeneration === lfsRequestGeneration
        && reportErrors
      ) reportError(error, t("workspace.controller.couldNotLoadGitLfsStatus"));
    } finally {
      if (
        shouldClearLoading
        && repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && requestGeneration === lfsRequestGeneration
      ) lfsLoading.value = false;
    }
  }

  async function loadExternalEditor(): Promise<void> {
    try {
      externalEditor.value = await api.getExternalEditor();
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotLoadExternalEditorConfiguration"));
    }
  }

  function openExternalEditorModal(): void {
    externalEditorForm.executable = externalEditor.value?.executable ?? "";
    externalEditorForm.arguments = externalEditor.value?.arguments.join("\n") ?? "";
    externalEditorModalShown.value = true;
  }

  async function saveExternalEditor(): Promise<void> {
    const executable = externalEditorForm.executable.trim();
    if (!executable) {
      message.warning(t("workspace.controller.enterTheExternalEditorSAbsoluteExecutablePath"));
      return;
    }
    const editorArguments = externalEditorForm.arguments
      .split(/\r?\n/u)
      .map((argument) => argument.trim())
      .filter(Boolean);
    mutationBusy.value = true;
    try {
      externalEditor.value = await api.saveExternalEditor({ executable, arguments: editorArguments });
      externalEditorModalShown.value = false;
      message.success(t("workspace.controller.externalEditorSaved"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotSaveExternalEditor"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function confirmClearExternalEditor(): void {
    dialog.warning({
      title: t("workspace.controller.clearExternalEditor"),
      content: t("workspace.controller.clearTheExternalEditorConfigurationSavedByThisAppThisDoesNotChangeSystemGitOrEditorSettings"),
      positiveText: t("workspace.controller.clear"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          externalEditor.value = await api.saveExternalEditor(null);
          externalEditorModalShown.value = false;
          message.success(t("workspace.controller.externalEditorConfigurationCleared"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotClearExternalEditor"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  async function openConflictedFilesInExternalEditor(): Promise<void> {
    if (!activeRepositoryPath.value) return;
    if (!externalEditor.value) {
      openExternalEditorModal();
      return;
    }
    mutationBusy.value = true;
    try {
      const paths = await api.openConflictedFilesInExternalEditor(activeRepositoryPath.value);
      message.info(t(
        "workspace.controller.openedCountConflictedFileSInTheExternalEditorWhenFinishedReturnHereToRefreshStageAndContinue",
        { count: paths.length },
      ));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotOpenExternalEditor"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function openWorktreeModal(): void {
    worktreeForm.path = "";
    worktreeForm.mode = "new";
    worktreeForm.branch = "";
    worktreeModalShown.value = true;
  }

  async function addWorktree(): Promise<void> {
    if (!activeRepositoryPath.value || !worktreeForm.path.trim() || !worktreeForm.branch.trim()) {
      message.warning(worktreeForm.mode === "new"
        ? t("workspace.controller.enterANewWorktreeFolderAndBranchName")
        : t("workspace.controller.enterAWorktreeFolderAndSelectAnExistingBranch"));
      return;
    }
    const repositoryPath = activeRepositoryPath.value;
    if (!repositoryPath) return;
    const selectionGeneration = getSelectionGeneration();
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const nextWorktrees = await api.addWorktree(
        repositoryPath,
        worktreeForm.path.trim(),
        worktreeForm.mode === "new" ? worktreeForm.branch.trim() : undefined,
        worktreeForm.mode === "existing" ? worktreeForm.branch.trim() : undefined,
      );
      if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
        worktreesRequestGeneration += 1;
        worktreesLoading.value = false;
        worktrees.value = nextWorktrees;
        worktreeModalShown.value = false;
      }
      await loadRepositories();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      await refreshSnapshot();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      message.success(t("workspace.controller.worktreeCreatedYouCanOpenItFromTheList"));
    } catch (error) {
      if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
        reportError(error, t("workspace.controller.couldNotCreateWorktree"));
      }
    } finally {
      finishMutation();
    }
  }

  function confirmRemoveWorktree(worktree: WorktreeInfo): void {
    if (!activeRepositoryPath.value || worktree.isMain || worktree.path === snapshot.value?.repository.path) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    dialog.error({
      title: t("workspace.controller.removeWorktree"),
      content: t(
        "workspace.controller.removeWorktreePathItsDirectoryAndUncommittedChangesWillBeDeletedGitRejectsTheOperationIfTheWorkt",
        { path: worktree.path },
      ),
      positiveText: t("workspace.controller.removeWorktree"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        const finishMutation = beginRepositoryMutation(repositoryPath);
        if (!finishMutation) return;
        try {
          const nextWorktrees = await api.removeWorktree(repositoryPath, worktree.path, false);
          if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
            worktreesRequestGeneration += 1;
            worktreesLoading.value = false;
            worktrees.value = nextWorktrees;
          }
          await loadRepositories();
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          message.success(t("workspace.controller.worktreeRemoved"));
        } catch (error) {
          if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
            reportError(error, t("workspace.controller.couldNotRemoveWorktree"));
          }
        } finally {
          finishMutation();
        }
      },
    });
  }

  function confirmUpdateSubmodules(): void {
    if (!activeRepositoryPath.value) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    dialog.warning({
      title: t("workspace.controller.initializeOrUpdateSubmodules"),
      content: t("workspace.controller.initializeOrUpdateAllSubmodulesRecursivelyUsingThisRepositorySCurrentConfigurationThisMayDownloa"),
      positiveText: t("workspace.controller.initializeAndUpdate"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
          return;
        }
        const finishMutation = beginRepositoryMutation(repositoryPath);
        if (!finishMutation) return;
        try {
          const nextSubmodules = await api.updateSubmodules(repositoryPath, true, true);
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          submodulesRequestGeneration += 1;
          submodulesLoading.value = false;
          submodules.value = nextSubmodules;
          await refreshSnapshot();
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          message.success(t("workspace.controller.submodulesInitializedOrUpdated"));
        } catch (error) {
          if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
            reportError(error, t("workspace.controller.couldNotUpdateSubmodules"));
          }
        } finally {
          finishMutation();
        }
      },
    });
  }

  async function initializeLfs(): Promise<void> {
    if (!activeRepositoryPath.value || !lfsStatus.value?.available) return;
    const repositoryPath = activeRepositoryPath.value;
    const selectionGeneration = getSelectionGeneration();
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const nextLfsStatus = await api.installLfs(repositoryPath);
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      lfsRequestGeneration += 1;
      lfsLoading.value = false;
      lfsStatus.value = nextLfsStatus;
      message.success(t("workspace.controller.gitLfsInitializedForThisRepository"));
    } catch (error) {
      if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
        reportError(error, t("workspace.controller.couldNotInitializeGitLfs"));
      }
    } finally {
      finishMutation();
    }
  }

  function openLfsTrackModal(): void {
    lfsTrackForm.pattern = "";
    lfsTrackModalShown.value = true;
  }

  async function trackLfsPattern(): Promise<void> {
    if (!activeRepositoryPath.value || !lfsTrackForm.pattern.trim()) {
      message.warning(t("workspace.controller.enterAFilePatternForGitLfsToTrack"));
      return;
    }
    const repositoryPath = activeRepositoryPath.value;
    if (!repositoryPath) return;
    const selectionGeneration = getSelectionGeneration();
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const nextSnapshot = await api.trackLfsPattern(repositoryPath, lfsTrackForm.pattern.trim());
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      applySnapshot(nextSnapshot);
      lfsTrackModalShown.value = false;
      await loadLfsStatus();
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      message.success(t("workspace.controller.theGitLfsRuleWasWrittenToGitattributesReviewAndCommitThatChange"));
    } catch (error) {
      if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
        reportError(error, t("workspace.controller.couldNotAddGitLfsRule"));
      }
    } finally {
      finishMutation();
    }
  }

  function resetRepositoryAuxiliary(): void {
    worktreesRequestGeneration += 1;
    submodulesRequestGeneration += 1;
    lfsRequestGeneration += 1;
    worktreesLoading.value = false;
    submodulesLoading.value = false;
    lfsLoading.value = false;
    worktreeModalShown.value = false;
    lfsTrackModalShown.value = false;
    worktrees.value = [];
    submodules.value = [];
    lfsStatus.value = null;
  }

  return {
    worktrees,
    submodules,
    lfsStatus,
    externalEditor,
    worktreesLoading,
    submodulesLoading,
    lfsLoading,
    worktreeModalShown,
    lfsTrackModalShown,
    externalEditorModalShown,
    worktreeForm,
    lfsTrackForm,
    externalEditorForm,
    loadWorktrees,
    loadSubmodules,
    loadLfsStatus,
    loadExternalEditor,
    openExternalEditorModal,
    saveExternalEditor,
    confirmClearExternalEditor,
    openConflictedFilesInExternalEditor,
    openWorktreeModal,
    addWorktree,
    confirmRemoveWorktree,
    confirmUpdateSubmodules,
    initializeLfs,
    openLfsTrackModal,
    trackLfsPattern,
    resetRepositoryAuxiliary,
  };
}
