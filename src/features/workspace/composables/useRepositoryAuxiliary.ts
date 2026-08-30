import { computed, reactive, ref, watch, type Ref } from "vue";
import type { DialogApi, MessageApi } from "naive-ui";
import {
  api,
  isAbsoluteExecutablePath,
  repositoryEditorLaunchShapeIsValid,
} from "../../../shared/api/tauri";
import { isAppError } from "../../../shared/api/tauriRuntime";
import { t } from "../../../shared/i18n";
import type {
  ExternalEditorConfig,
  LfsStatus,
  RepositoryEditorConfig,
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
  reportError: (
    error: unknown,
    action: string,
    repositoryPath?: string,
    selectionGeneration?: number,
  ) => void;
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
  const repositoryEditors = ref<RepositoryEditorConfig[]>([]);
  const repositoryEditorsLoaded = ref(false);
  const worktreesLoading = ref(false);
  const submodulesLoading = ref(false);
  const lfsLoading = ref(false);
  const worktreeModalShown = ref(false);
  const lfsTrackModalShown = ref(false);
  const externalEditorModalShown = ref(false);
  const repositoryEditorsModalShown = ref(false);
  const installedApplicationPickerShown = ref(false);
  const worktreeForm = reactive({ path: "", mode: "new", branch: "" });
  const lfsTrackForm = reactive({ pattern: "" });
  const externalEditorForm = reactive({ executable: "", arguments: "" });
  const repositoryEditorForm = reactive({ name: "", executable: "", arguments: "" });
  const repositoryEditorEditingIndex = ref<number | null>(null);
  const repositoryEditorEditingName = ref<string | null>(null);
  const repositoryEditorPickerBusy = ref(false);
  const installedApplications = ref<RepositoryEditorConfig[]>([]);
  const installedApplicationOptions = computed(() => installedApplications.value.map(
    (application, index) => ({ label: application.name, value: index }),
  ));
  const installedApplicationSelection = ref<number | null>(null);
  let installedApplicationsLoaded = false;
  let installedApplicationsRequest: Promise<RepositoryEditorConfig[]> | null = null;
  let worktreesRequestGeneration = 0;
  let submodulesRequestGeneration = 0;
  let lfsRequestGeneration = 0;
  let repositoryEditorPickerGeneration = 0;
  let repositoryEditorsLoadGeneration = 0;
  let repositoryEditorsRequest: Promise<RepositoryEditorConfig[]> | null = null;

  function invalidateRepositoryEditorPicker(): void {
    repositoryEditorPickerGeneration += 1;
    repositoryEditorPickerBusy.value = false;
    installedApplicationPickerShown.value = false;
  }

  watch(repositoryEditorsModalShown, (shown) => {
    if (shown) return;
    invalidateRepositoryEditorPicker();
  });

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
      ) reportError(error, t("workspace.controller.couldNotLoadWorktrees"), repositoryPath, selectionGeneration);
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
      ) reportError(error, t("workspace.controller.couldNotLoadSubmodules"), repositoryPath, selectionGeneration);
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
      ) reportError(error, t("workspace.controller.couldNotLoadGitLfsStatus"), repositoryPath, selectionGeneration);
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

  async function loadRepositoryEditors(): Promise<boolean> {
    const requestGeneration = repositoryEditorsLoadGeneration;
    const request = repositoryEditorsRequest ?? api.getRepositoryEditors();
    repositoryEditorsRequest = request;
    try {
      const editors = await request;
      if (requestGeneration !== repositoryEditorsLoadGeneration) return false;
      repositoryEditors.value = editors;
      repositoryEditorsLoaded.value = true;
      return true;
    } catch (error) {
      if (requestGeneration !== repositoryEditorsLoadGeneration) return false;
      repositoryEditorsLoaded.value = false;
      reportError(error, t("workspace.controller.couldNotLoadRepositoryEditors"));
      return false;
    } finally {
      if (repositoryEditorsRequest === request) repositoryEditorsRequest = null;
    }
  }

  function resetRepositoryEditorForm(): void {
    invalidateRepositoryEditorPicker();
    repositoryEditorEditingIndex.value = null;
    repositoryEditorEditingName.value = null;
    repositoryEditorForm.name = "";
    repositoryEditorForm.executable = "";
    repositoryEditorForm.arguments = "";
  }

  async function openRepositoryEditorsModal(): Promise<void> {
    if (!repositoryEditorsLoaded.value && !(await loadRepositoryEditors())) return;
    installedApplicationsLoaded = false;
    resetRepositoryEditorForm();
    repositoryEditorsModalShown.value = true;
  }

  function editRepositoryEditor(index: number): void {
    const editor = repositoryEditors.value[index];
    if (!editor) return;
    invalidateRepositoryEditorPicker();
    repositoryEditorEditingIndex.value = index;
    repositoryEditorEditingName.value = editor.name;
    repositoryEditorForm.name = editor.name;
    repositoryEditorForm.executable = editor.executable;
    repositoryEditorForm.arguments = editor.arguments.join("\n");
  }

  async function browseRepositoryEditorExecutable(): Promise<void> {
    if (repositoryEditorPickerBusy.value) return;
    repositoryEditorPickerBusy.value = true;
    const requestGeneration = ++repositoryEditorPickerGeneration;
    installedApplicationPickerShown.value = false;
    try {
      const executable = await api.selectApplicationExecutable(
        t("workspace.dialogs.selectIdeApplication"),
      );
      if (
        executable
        && requestGeneration === repositoryEditorPickerGeneration
        && repositoryEditorsModalShown.value
      ) repositoryEditorForm.executable = executable;
    } catch (error) {
      if (
        requestGeneration === repositoryEditorPickerGeneration
        && repositoryEditorsModalShown.value
      ) reportError(error, t("workspace.controller.couldNotSelectIdeApplication"));
    } finally {
      if (requestGeneration === repositoryEditorPickerGeneration) {
        repositoryEditorPickerBusy.value = false;
      }
    }
  }

  async function selectRepositoryEditorApplication(): Promise<void> {
    if (repositoryEditorPickerBusy.value) return;
    repositoryEditorPickerBusy.value = true;
    const requestGeneration = ++repositoryEditorPickerGeneration;
    try {
      if (!installedApplicationsLoaded) {
        try {
          if (!installedApplicationsRequest) {
            const request = api.listInstalledApplications();
            installedApplicationsRequest = request;
            const clearRequest = () => {
              if (installedApplicationsRequest === request) installedApplicationsRequest = null;
            };
            void request.then(clearRequest, clearRequest);
          }
          const applications = await installedApplicationsRequest;
          if (
            requestGeneration !== repositoryEditorPickerGeneration
            || !repositoryEditorsModalShown.value
          ) return;
          installedApplications.value = applications;
          installedApplicationsLoaded = true;
        } catch (error) {
          installedApplications.value = [];
          if (
            requestGeneration === repositoryEditorPickerGeneration
            && repositoryEditorsModalShown.value
          ) reportError(error, t("workspace.controller.couldNotSelectIdeApplication"));
        }
      }
      if (
        requestGeneration !== repositoryEditorPickerGeneration
        || !repositoryEditorsModalShown.value
      ) return;
      installedApplicationSelection.value = null;
      if (installedApplications.value.length) installedApplicationPickerShown.value = true;
      else {
        const executable = await api.selectApplicationExecutable(
          t("workspace.dialogs.selectIdeApplication"),
        );
        if (
          executable
          && requestGeneration === repositoryEditorPickerGeneration
          && repositoryEditorsModalShown.value
        ) repositoryEditorForm.executable = executable;
      }
    } catch (pickerError) {
      if (
        requestGeneration === repositoryEditorPickerGeneration
        && repositoryEditorsModalShown.value
      ) {
          reportError(pickerError, t("workspace.controller.couldNotSelectIdeApplication"));
      }
    } finally {
      if (requestGeneration === repositoryEditorPickerGeneration) {
        repositoryEditorPickerBusy.value = false;
      }
    }
  }

  function applyInstalledApplication(): void {
    const index = installedApplicationSelection.value;
    const application = index === null ? undefined : installedApplications.value[index];
    if (!application) return;
    repositoryEditorForm.name = application.name;
    repositoryEditorForm.executable = application.executable;
    repositoryEditorForm.arguments = application.arguments.join("\n");
    installedApplicationPickerShown.value = false;
  }

  async function saveRepositoryEditor(): Promise<void> {
    if (mutationBusy.value) return;
    const name = repositoryEditorForm.name.trim();
    const executable = repositoryEditorForm.executable.trim();
    if (!name || !executable || Array.from(name).length > 80 || /[\u0000-\u001f\u007f-\u009f]/u.test(name)) {
      message.warning(t("workspace.controller.enterTheIdeNameAndAbsoluteExecutablePath"));
      return;
    }
    if (!isAbsoluteExecutablePath(executable)) {
      message.warning(t("workspace.controller.ideExecutablePathMustBeAbsolute"));
      return;
    }
    const editingName = repositoryEditorEditingName.value;
    if (repositoryEditors.value.some((editor) =>
      editor.name !== editingName && editor.name.toLowerCase() === name.toLowerCase()
    )) {
      message.warning(t("workspace.controller.ideNameMustBeUnique"));
      return;
    }
    if (editingName === null && repositoryEditors.value.length >= 16) {
      message.warning(t("workspace.controller.tooManyRepositoryEditors"));
      return;
    }
    const argumentsList = repositoryEditorForm.arguments
      .split(/\r?\n/u)
      .map((argument) => argument.trim())
      .filter(Boolean);
    if (argumentsList.length > 32) {
      message.warning(t("workspace.controller.tooManyIdeArguments"));
      return;
    }
    if (!repositoryEditorLaunchShapeIsValid(executable, argumentsList)) {
      message.warning(t("workspace.controller.ideLaunchConfigurationInvalid"));
      return;
    }
    const editor: RepositoryEditorConfig = {
      name,
      executable,
      arguments: argumentsList,
    };
    const nextEditors = [...repositoryEditors.value];
    if (editingName === null) nextEditors.push(editor);
    else {
      const currentIndex = nextEditors.findIndex((candidate) => candidate.name === editingName);
      if (currentIndex < 0) {
        message.warning(t("workspace.controller.repositoryEditorListChanged"));
        return;
      }
      nextEditors[currentIndex] = editor;
    }
    invalidateRepositoryEditorPicker();
    mutationBusy.value = true;
    repositoryEditorsLoadGeneration += 1;
    repositoryEditorsRequest = null;
    try {
      repositoryEditors.value = await api.saveRepositoryEditors(nextEditors, repositoryEditors.value);
      repositoryEditorsLoadGeneration += 1;
      repositoryEditorsRequest = null;
      repositoryEditorsLoaded.value = true;
      resetRepositoryEditorForm();
      message.success(t("workspace.controller.repositoryEditorsSaved"));
    } catch (error) {
      if (isAppError(error) && error.code === "repository_editors_changed") {
        repositoryEditorsLoaded.value = false;
        if (await loadRepositoryEditors()) resetRepositoryEditorForm();
        else repositoryEditorsModalShown.value = false;
        message.warning(t("workspace.controller.repositoryEditorListChanged"));
      } else reportError(error, t("workspace.controller.couldNotSaveRepositoryEditors"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function confirmRemoveRepositoryEditor(index: number): void {
    const editor = repositoryEditors.value[index];
    if (!editor) return;
    dialog.warning({
      title: t("workspace.controller.removeRepositoryEditor"),
      content: t("workspace.controller.removeRepositoryEditorConfirmation", { name: editor.name }),
      positiveText: t("workspace.controller.remove"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        if (mutationBusy.value) return false;
        mutationBusy.value = true;
        repositoryEditorsLoadGeneration += 1;
        repositoryEditorsRequest = null;
        invalidateRepositoryEditorPicker();
        try {
          const expectedEditors = repositoryEditors.value;
          repositoryEditors.value = await api.saveRepositoryEditors(
            expectedEditors.filter((candidate) => candidate.name !== editor.name),
            expectedEditors,
          );
          repositoryEditorsLoadGeneration += 1;
          repositoryEditorsRequest = null;
          repositoryEditorsLoaded.value = true;
          if (repositoryEditorEditingName.value === editor.name) {
            resetRepositoryEditorForm();
          } else if (repositoryEditorEditingName.value !== null) {
            repositoryEditorEditingIndex.value = repositoryEditors.value.findIndex(
              (candidate) => candidate.name === repositoryEditorEditingName.value,
            );
          }
          message.success(t("workspace.controller.repositoryEditorRemoved"));
        } catch (error) {
          if (isAppError(error) && error.code === "repository_editors_changed") {
            repositoryEditorsLoaded.value = false;
            if (await loadRepositoryEditors()) resetRepositoryEditorForm();
            else repositoryEditorsModalShown.value = false;
            message.warning(t("workspace.controller.repositoryEditorListChanged"));
          } else reportError(error, t("workspace.controller.couldNotSaveRepositoryEditors"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
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
    const repositoryPath = activeRepositoryPath.value;
    if (!repositoryPath) return;
    const selectionGeneration = getSelectionGeneration();
    if (!externalEditor.value) {
      openExternalEditorModal();
      return;
    }
    mutationBusy.value = true;
    try {
      const paths = await api.openConflictedFilesInExternalEditor(repositoryPath);
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
      message.info(t(
        "workspace.controller.openedCountConflictedFileSInTheExternalEditorWhenFinishedReturnHereToRefreshStageAndContinue",
        { count: paths.length },
      ));
    } catch (error) {
      if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
        reportError(
          error,
          t("workspace.controller.couldNotOpenExternalEditor"),
          repositoryPath,
          selectionGeneration,
        );
      }
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
        reportError(error, t("workspace.controller.couldNotCreateWorktree"), repositoryPath, selectionGeneration);
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
            reportError(error, t("workspace.controller.couldNotRemoveWorktree"), repositoryPath, selectionGeneration);
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
            reportError(error, t("workspace.controller.couldNotUpdateSubmodules"), repositoryPath, selectionGeneration);
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
        reportError(error, t("workspace.controller.couldNotInitializeGitLfs"), repositoryPath, selectionGeneration);
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
        reportError(error, t("workspace.controller.couldNotAddGitLfsRule"), repositoryPath, selectionGeneration);
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
    repositoryEditors,
    repositoryEditorsLoaded,
    worktreesLoading,
    submodulesLoading,
    lfsLoading,
    worktreeModalShown,
    lfsTrackModalShown,
    externalEditorModalShown,
    repositoryEditorsModalShown,
    installedApplicationPickerShown,
    worktreeForm,
    lfsTrackForm,
    externalEditorForm,
    repositoryEditorForm,
    repositoryEditorEditingIndex,
    repositoryEditorPickerBusy,
    installedApplications,
    installedApplicationOptions,
    installedApplicationSelection,
    loadWorktrees,
    loadSubmodules,
    loadLfsStatus,
    loadExternalEditor,
    loadRepositoryEditors,
    openExternalEditorModal,
    saveExternalEditor,
    confirmClearExternalEditor,
    openRepositoryEditorsModal,
    resetRepositoryEditorForm,
    editRepositoryEditor,
    selectRepositoryEditorApplication,
    browseRepositoryEditorExecutable,
    applyInstalledApplication,
    saveRepositoryEditor,
    confirmRemoveRepositoryEditor,
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
