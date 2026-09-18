import { computed, h, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { NButton, NScrollbar, NSpace, useDialog, useMessage, useNotification, type NotificationReactive } from "naive-ui";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, builtinEndpoints, isAppError, isBrowserDemo, toggleDevtools } from "../../../shared/api/tauri";
import { appLocale, authenticationMethodLabel, authenticationStatusLabel, endpointKindLabel, formatDate, localizedErrorDetails, localizedErrorMessage, setAppLocale, t } from "../../../shared/i18n";
import { completeStartupPerformance, markPerformance, measurePerformance } from "../../../shared/performance";
import { colorMode, resetThemeColor, setColorMode, themeColor } from "../../../shared/theme";
import { settingsSection } from "../../../shared/settingsNavigation";
import {
  type AppError,
  type CodeHostCapabilities,
  type RemoteBinding,
  type RemoteProfile,
  type RepositoryLockInfo,
  type RepositorySnapshot,
} from "../../../shared/types/domain";
import { createRepositoryPresentation } from "../../repository/utils/presentation";
import { useRepositoryCatalog } from "./useRepositoryCatalog";
import { useRepositoryDiscovery } from "./useRepositoryDiscovery";
import { useWorkspaceAccounts } from "./useWorkspaceAccounts";
import { useRepositoryPlatform } from "./useRepositoryPlatform";
import { useRepositoryAuxiliary } from "./useRepositoryAuxiliary";
import { useRepositoryHistory } from "./useRepositoryHistory";
import { useRepositoryOperations } from "./useRepositoryOperations";
import { useRepositoryChanges } from "./useRepositoryChanges";
import { acquireRepositoryOperation, repositoryOperationsBusy } from "./repositoryOperationLock";
import { handleDevtoolsShortcut, localeOptions, updateAppLocale, updateColorMode, updateThemeColor, useWorkspaceUiOptions } from "./workspaceUiActions";
import { useWorkspaceTables } from "./useWorkspaceTables";
import {
  gitLockInventoryDisposition,
  gitLockInspectionIsCurrent,
  replaceGitLockRecoveryDialog,
  runOwnedGitLockInspection,
} from "./gitLockRecoveryState";
import {
  bindingsAreEqual,
  branchesAreEqual,
  emptyCapabilities,
  endpointHasExplicitPort,
  historySnapshotSignature,
  initialRepositoryGrouping,
  rememberLastRepositoryPath,
  remoteBranchesAreEqual,
  remotesAreEqual,
  repositoryGroupingStorageKey,
  repositorySnapshotSignature,
  reuseEqualArray,
  storedLastRepositoryPath,
  supportsCloneCredentials,
  supportsGitRemoteCredentials,
  type AppSection,
  type NetworkAction,
  type NetworkActionTarget,
  type RefreshSnapshotOptions,
  type RepositoryChangedPayload,
  type RepositoryTab,
  type RepositoryViewportSnapshot,
} from "./workspaceControllerSupport";

export function useWorkspaceController() {
const message = useMessage();
const dialog = useDialog();
const notification = useNotification();
const browserDemo = isBrowserDemo();
const {
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
} = createRepositoryPresentation();
const { colorModeOptions, repositoryActionOptions } = useWorkspaceUiOptions();

const section = ref<AppSection>("repositories");
const activeTab = ref<RepositoryTab>("changes");
const repositoryTabContentReady = reactive<Record<RepositoryTab, boolean>>({
  changes: true,
  history: false,
  branches: false,
  overview: false,
});
const activeRepositoryPath = ref<string | null>(null);
let repositorySelectionGeneration = 0;
let repositoryWatchRequestGeneration = Date.now();
let snapshotApplicationGeneration = 0;
let snapshotRefreshRequestGeneration = 0;
const snapshot = ref<RepositorySnapshot | null>(null);
const selectedRemoteName = ref("");
const activeCapabilities = ref<CodeHostCapabilities>(emptyCapabilities);
let activeErrorNotification: NotificationReactive | null = null;
let activeErrorNotificationOwner: symbol | null = null;
let activeGitLockRecoveryDialog: ReturnType<typeof dialog.warning> | null = null;
let activeGitLockRecoveryDialogOwner: symbol | null = null;
let activeGitLockRecoveryDialogTarget: RepositoryErrorTarget | null = null;

const workspaceInitializing = ref(true);
const snapshotLoading = ref(false);
let repositoryTabLoadGeneration = 0;
const networkBusy = ref<NetworkAction | null>(null);
const networkTrustActionBusyOwners = reactive(new Set<symbol>());
const gitLockRecoveryBusyOwners = reactive(new Set<symbol>());
let networkOperationGeneration = 0;
const nonRepositoryMutationBusy = ref(false);
const mutationBusy = computed({
  get: () => nonRepositoryMutationBusy.value || repositoryOperationsBusy.value,
  set: (value: boolean) => {
    nonRepositoryMutationBusy.value = value;
  },
});
const {
  repositories,
  repositoryGrouping,
  repositoriesLoading,
  repositoryGroups,
  repositoryGroupModalShown,
  repositoryAliasModalShown,
  repositoryGroupTarget,
  repositoryAliasTarget,
  repositoryGroupForm,
  repositoryAliasForm,
  compareRepositoryRecords,
  replaceRepositoryRecord,
  openRepositoryGroupEditor,
  saveRepositoryGroup,
  openRepositoryAliasEditor,
  saveRepositoryAlias,
  loadRepositories,
} = useRepositoryCatalog({
  snapshot,
  mutationBusy,
  reportError,
  showSuccess: (content) => message.success(content),
});
const {
  discoverModalShown,
  discoveryRootPath,
  repositoryImportLimit,
  discoveredRepositories,
  selectedDiscoveredRepositoryPaths,
  discoveryScanning,
  discoveryAdding,
  discoveryBusy,
  selectableDiscoveredRepositoryCount,
  selectedDiscoveredRepositoryCount,
  allDiscoveredRepositoriesSelected,
  someDiscoveredRepositoriesSelected,
  openRepositoryDiscovery,
  closeRepositoryDiscovery,
  disposeRepositoryDiscovery,
  updateRepositoryDiscoveryShown,
  discoveredRepositoryAlreadyAdded,
  discoveredRepositorySelected,
  discoveredRepositorySelectionDisabled,
  toggleDiscoveredRepositorySelection,
  updateDiscoveredRepositorySelection,
  updateAllDiscoveredRepositorySelections,
  addSelectedDiscoveredRepositories,
} = useRepositoryDiscovery({
  repositories,
  replaceRepositoryRecord,
  loadRepositories: () => loadRepositories(repositoryGrouping.value === "owner", false, false),
  operations: {
    selectDirectory: () => api.selectRepositoryDiscoveryDirectory(t("workspace.dialogs.selectFolderToScan")),
    discoverRepositories: (rootPath) => api.discoverRepositories(rootPath),
    addRepositories: (repositoryPaths) => api.addDiscoveredRepositories(repositoryPaths),
  },
  messages: {
    pickerFailed: () => t("workspace.controller.couldNotSelectFolder"),
    scanFailed: () => t("workspace.controller.couldNotScanRepositories"),
    addFailed: () => t("workspace.controller.couldNotAddRepositories"),
    noRepositoriesFound: () => t("workspace.controller.noGitRepositoriesFound"),
    addedRepositories: (count) => t("workspace.controller.addedCountRepositories", { count }),
    partiallyAddedRepositories: (added, selected) =>
      t("workspace.controller.addedCountOfSelectedRepositories", { added, selected }),
    importPersistenceWarning: (added, selected) =>
      t("workspace.controller.repositoryImportPersistenceWarning", { added, selected }),
    importCommittedRefreshFailed: (added, selected) =>
      t("workspace.controller.repositoryImportCommittedRefreshFailed", { added, selected }),
    selectionLimitReached: (limit) =>
      t("workspace.controller.repositorySelectionLimitReached", { limit }),
  },
  reportError,
  showSuccess: (content) => message.success(content),
  showWarning: (content) => message.warning(content),
});
const {
  accounts,
  endpoints,
  accountsLoading,
  accountStatusRefreshing,
  accountModalShown,
  customEndpointModalShown,
  accountForm,
  endpointForm,
  lastAccountStatusRefreshAt,
  accountStatusRefreshCacheMs,
  accountFormCapabilities,
  accountUsesPat,
  accountMethodOptions,
  loadAccounts,
  refreshAccountStatuses,
  loadAccountsThenRefreshStatuses,
  loadEndpoints,
  saveAccount,
  openCustomEndpointModal,
  saveCustomEndpoint,
  importCliAccounts,
  detectCliAccounts,
  setDefaultCandidate,
  confirmRemoveAccount,
} = useWorkspaceAccounts({
  mutationBusy,
  capabilityForEndpoint,
  reportError,
  message,
  dialog,
});
const {
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
} = useRepositoryAuxiliary({
  activeRepositoryPath,
  snapshot,
  mutationBusy,
  getSelectionGeneration: () => repositorySelectionGeneration,
  repositorySelectionIsCurrent,
  applySnapshot,
  loadRepositories: async () => {
    await loadRepositories();
  },
  refreshSnapshot,
  reportError: reportCapturedRepositoryError,
  message,
  dialog,
});
const {
  commits,
  stashes,
  reflogText,
  selectedCommit,
  commitChangedFiles,
  selectedCommitFile,
  commitDiffText,
  historyLoading,
  reflogLoading,
  commitFilesLoading,
  commitDiffLoading,
  stashesLoading,
  reflogModalShown,
  commitDiffVisibleLines,
  commitDiffHasMoreLines,
  commitDiffLineLimitReached,
  commitDiffCanExpand,
  commitDiffLimitedAtMaximum,
  showMoreCommitDiffLines,
  clearCommitSelection,
  invalidateCommitHistoryCache,
  invalidateStashesCache,
  invalidateCommitDetailCaches,
  invalidateCommitDiffCache,
  historyWasLoaded,
  resetRepositoryHistory,
  disposeRepositoryHistory,
  loadHistory,
  loadHistoryWithOptions,
  loadCommitHistoryWithOptions,
  loadStashesWithOptions,
  loadReflog,
  openReflog,
  loadCommitFiles,
  loadCommitFileDiff,
} = useRepositoryHistory({
  activeRepositoryPath,
  snapshot,
  getSelectionGeneration: () => repositorySelectionGeneration,
  repositorySelectionIsCurrent,
  isControllerMounted: () => controllerMounted,
  reportError: reportCapturedRepositoryError,
});

const cloneModalShown = ref(false);
const initModalShown = ref(false);
const remoteModalShown = ref(false);
const bindingDrawerShown = ref(false);
const bindingRemote = ref<RemoteProfile | null>(null);
const settingsModalShown = ref(false);

const cloneForm = reactive({ url: "", destinationPath: "", parentPath: "", endpointId: "", accountId: "none", repositoryDerivedRouting: false });
const initForm = reactive({ destinationPath: "", initialBranch: "main" });
const remoteForm = reactive({ name: "", fetchUrl: "", pushUrl: "" });
const editedRemoteName = ref<string | null>(null);
const remoteEditorRepositoryPath = ref<string | null>(null);
const remoteEditorSelectionGeneration = ref<number | null>(null);
const bindingForm = reactive({ endpointId: "", accountId: "auto" });
const bindingRepositoryPath = ref<string | null>(null);
const bindingSelectionGeneration = ref<number | null>(null);
const automaticRefreshDebounceMs = 500;
const automaticRefreshRetryMs = 750;
const repositoryWatcherMaxRetryDelayMs = 12_000;
const repositoryWatcherMaxRetryAttempts = 5;
let automaticRefreshTimer: ReturnType<typeof setTimeout> | null = null;
let repositoryWatcherRetryTimer: ReturnType<typeof setTimeout> | null = null;
let repositoryWatcherRetryAttempt = 0;
let repositoryWatcherUnavailable = false;
const repositoryWatchSupersededGenerations = new Set<number>();
let repositoryWatchPlanChangeGeneration = 0;
let repositoryWatchPlanAppliedGeneration = 0;
let automaticRefreshDirty = false;
let automaticRefreshInFlight = false;
let repositoryViewportInteractionGeneration = 0;
let windowFocused = typeof document === "undefined" || document.hasFocus();
let controllerMounted = false;
let browserDemoLocaleRefreshGeneration = 0;
let stopRepositoryChangeListener: UnlistenFn | null = null;
let stopRepositoryWatchErrorListener: UnlistenFn | null = null;
let stopWindowFocusListener: UnlistenFn | null = null;

const activeRemote = computed(() => snapshot.value?.remotes.find((remote) => remote.name === selectedRemoteName.value) ?? null);
function remoteProfileIdentity(remote: RemoteProfile | null): string | null {
  if (!remote) return null;
  return JSON.stringify([
    remote.name,
    remote.fetchUrl,
    remote.pushUrl,
    remote.endpointId,
    remote.codeHostKind,
  ]);
}

const canPullFromActiveRemote = computed(() => {
  const upstream = snapshot.value?.status.upstream;
  return Boolean(
    snapshot.value?.hasHead &&
    snapshot.value.status.branch &&
    upstream &&
    activeRemote.value &&
    upstream.startsWith(`${activeRemote.value.name}/`),
  );
});
const {
  repositoryInfo,
  repositoryWebUrl,
  pullRequests,
  platformLoading,
  pullRequestsLoading,
  publishOwnersLoading,
  pullRequestModalShown,
  publishModalShown,
  forkModalShown,
  pullRequestForm,
  pullRequestHead,
  forkForm,
  publishForm,
  publishOwners,
  publishOwnersError,
  pullRequestBaseOptions,
  publishEndpoints,
  publishAccounts,
  publishAccountOptions,
  forkAccounts,
  forkAccountOptions,
  forkContributionBranchRecommended,
  publishOwnerOptions,
  canPublishToSelectedOwner,
  loadRepositoryInfo,
  openCodeHostUrl,
  loadPullRequests,
  invalidatePlatformRequests,
  openCreatePullRequest,
  createPullRequest,
  openForkModal,
  forkGithubRepository,
  loadPublishOwners,
  openPublishModal,
  publishGitHubRepository,
  invalidatePublishOwnerRequests,
} = useRepositoryPlatform({
  activeRepositoryPath,
  activeRemote,
  activeCapabilities,
  snapshot,
  endpoints,
  accounts,
  selectedCommit,
  commits,
  getSelectionGeneration: () => repositorySelectionGeneration,
  repositorySelectionIsCurrent,
  capabilityForEndpoint,
  selectedRemoteName,
  applySnapshot,
  loadCapabilities,
  refreshSnapshot,
  reportError: reportCapturedRepositoryError,
  message,
});
const currentBranch = computed(() => snapshot.value?.status.branch ?? "—");
const {
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
  loadFileBlame,
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
} = useRepositoryChanges({
  activeRepositoryPath,
  snapshot,
  currentBranch,
  commits,
  getSelectionGeneration: () => repositorySelectionGeneration,
  repositorySelectionIsCurrent,
  applySnapshot,
  loadHistory,
  isChangeFullyIncluded,
  isConflictedEntry,
  splitDiffHunks,
  splitDiffLines,
  reportError: reportCapturedRepositoryError,
  message,
  dialog,
});
const {
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
  rebaseBaseOptions,
  tagTargetOptions,
  tagNameOptions,
  tagNameExists,
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
} = useRepositoryOperations({
  activeRepositoryPath,
  snapshot,
  selectedChange,
  conflictedChanges,
  currentBranch,
  commits,
  getSelectionGeneration: () => repositorySelectionGeneration,
  repositorySelectionIsCurrent,
  applySnapshot,
  syncSelectedDiff,
  loadHistory,
  refreshSnapshot,
  refreshAfterGitConflict,
  reconcileSnapshotAfterMutationError,
  gitOperationLabel,
  reportError: reportCapturedRepositoryError,
  message,
  dialog,
});
const worktreeExistingBranchOptions = computed(() => {
  const checkedOut = new Set(worktrees.value.map((worktree) => worktree.branch).filter(Boolean));
  return (snapshot.value?.branches ?? [])
    .filter((branch) => !branch.isCurrent)
    .map((branch) => ({ label: branch.name, value: branch.name, disabled: checkedOut.has(branch.name) }));
});
const bindingAccounts = computed(() => accounts.value.filter(
  (account) => account.endpointId === bindingForm.endpointId && supportsGitRemoteCredentials(account),
));
const bindingAccountOptions = computed(() => [
  { label: t("workspace.controller.autoSelectOnePatCandidateOtherwiseSystemGit"), value: "auto" },
  ...bindingAccounts.value.map((account) => ({
    label: `${account.displayName || account.login} · ${authenticationStatusLabel(account.status)}`,
    value: account.id,
    disabled: account.status === "needsReauthentication" || account.status === "unavailable",
  })),
]);
const bindingEndpointOptions = computed(() => {
  const options = endpoints.value.map((endpoint) => ({ label: endpoint.displayName, value: endpoint.id }));
  const remoteEndpointId = bindingRemote.value?.endpointId;
  if (remoteEndpointId && !options.some((option) => option.value === remoteEndpointId)) {
    options.push({ label: `${remoteEndpointId} (${t("workspace.controller.manuallyAssociated")})`, value: remoteEndpointId });
  }
  return options;
});
const cloneAccounts = computed(() => accounts.value.filter(
  (account) => account.endpointId === cloneForm.endpointId && supportsCloneCredentials(account),
));
const cloneAccountOptions = computed(() => [
  { label: t("workspace.controller.useSystemGitNoAppAccount"), value: "none" },
  ...(cloneForm.endpointId
    ? [
        { label: t("workspace.controller.autoSelectOnlyOneUsableCandidate"), value: "auto", disabled: !cloneAccounts.value.some((account) => account.status === "valid" || account.status === "available") },
        ...cloneAccounts.value.map((account) => ({
          label: `${account.displayName || account.login} · ${authenticationStatusLabel(account.status)}`,
          value: account.id,
          disabled: account.status === "needsReauthentication" || account.status === "unavailable",
        })),
      ]
    : []),
]);
const repositoryGroupingOptions = computed(() => [
  { label: t("workspace.controller.manualGroups"), value: "manual" },
  { label: t("workspace.controller.remoteOwner"), value: "owner" },
  { label: t("workspace.controller.recentlyUsed"), value: "recent" },
]);
const {
  branchColumns,
  remoteBranchColumns,
  worktreeColumns,
  submoduleColumns,
  remoteColumns,
  accountColumns,
  pullRequestColumns,
  stashColumns,
} = useWorkspaceTables({
  t,
  snapshot,
  checkoutBranch,
  checkoutRemoteBranch,
  confirmMergeBranch,
  openRenameBranchModal,
  confirmDeleteBranch,
  selectRepository,
  confirmRemoveWorktree,
  submoduleStatusLabel,
  submoduleStatusTagType,
  remoteBindingLabel,
  openBindingDrawer,
  openRemoteEditor,
  endpointLabel,
  authenticationTagType,
  setDefaultCandidate,
  confirmRemoveAccount,
  openCodeHostUrl,
  confirmPopStash,
  confirmDropStash,
});

function capabilityForEndpoint(endpointId: string | null): CodeHostCapabilities {
  if (!endpointId) return emptyCapabilities;
  if (activeRemote.value?.endpointId === endpointId) return activeCapabilities.value;
  const endpoint = endpoints.value.find((item) => item.id === endpointId);
  if (!endpoint) return emptyCapabilities;
  const portScopedCli = endpointHasExplicitPort(endpoint.gitHost);
  if (endpoint.kind === "gitHub") {
    return { repositoryInfo: !portScopedCli, repositoryPublish: !portScopedCli, repositoryFork: !portScopedCli, pullRequestsRead: !portScopedCli, pullRequestsWrite: !portScopedCli, accountImport: !portScopedCli, accountBindings: true, patKeychain: true };
  }
  if (endpoint.kind === "gitLab") {
    return { repositoryInfo: false, repositoryPublish: false, repositoryFork: false, pullRequestsRead: false, pullRequestsWrite: false, accountImport: !portScopedCli, accountBindings: true, patKeychain: true };
  }
  if (endpoint.kind === "gitee") {
    return { repositoryInfo: false, repositoryPublish: false, repositoryFork: false, pullRequestsRead: false, pullRequestsWrite: false, accountImport: false, accountBindings: true, patKeychain: true };
  }
  if (endpoint.kind === "gitea" || endpoint.kind === "forgejo" || endpoint.kind === "genericGit") {
    return { repositoryInfo: false, repositoryPublish: false, repositoryFork: false, pullRequestsRead: false, pullRequestsWrite: false, accountImport: false, accountBindings: true, patKeychain: true };
  }
  return emptyCapabilities;
}

function endpointLabel(endpointId: string): string {
  return endpoints.value.find((endpoint) => endpoint.id === endpointId)?.displayName || endpointId;
}

function getBinding(remoteName: string): RemoteBinding | null {
  return snapshot.value?.bindings.find((binding) => binding.remoteName === remoteName) ?? null;
}

function remoteBindingLabel(remote: RemoteProfile): string {
  const binding = getBinding(remote.name);
  if (!binding) return t("workspace.controller.autoSelect");
  const selection = binding.accountSelection;
  if (selection.mode === "auto") return t("workspace.controller.autoSelect");
  const account = accounts.value.find((item) => item.id === selection.accountProfileId);
  return account
    ? `${account.displayName || account.login} · ${authenticationMethodLabel(account.authMethod)}`
    : t("workspace.controller.theBoundAccountIsUnavailable");
}

function remoteAuthenticationLabel(remote: RemoteProfile): string {
  const binding = getBinding(remote.name);
  const selection = binding?.accountSelection;
  if (!selection || selection.mode === "auto") return t("workspace.controller.autoResolveAUsableAccountBeforeTheOperation");
  const account = accounts.value.find((item) => item.id === selection.accountProfileId);
  if (!account) return t("workspace.controller.theBoundAccountWasRemoved");
  return `${authenticationMethodLabel(account.authMethod)} · ${authenticationStatusLabel(account.status)}`;
}

function normalizedAppError(error: unknown): AppError {
  const appError: AppError = isAppError(error)
    ? error
    : { code: "unexpected_error", message: t("workspace.controller.anUnexpectedErrorOccurred"), details: null };
  return {
    ...appError,
    message: localizedErrorMessage(appError.code),
    details: localizedErrorDetails(appError.code) || null,
  };
}

const recoverableGitLockCodes = new Set([
  "git_config_locked",
  "git_index_locked",
  "git_reference_locked",
  "git_lock_detected",
  "git_lock_changed",
]);

interface RepositoryErrorTarget {
  repositoryPath: string;
  selectionGeneration: number;
}

function repositoryLockKindLabel(lock: RepositoryLockInfo): string {
  if (lock.kind === "configuration") return t("workspace.controller.gitConfigurationLock");
  if (lock.kind === "index") return t("workspace.controller.gitIndexLock");
  if (lock.kind === "reference") return t("workspace.controller.gitReferenceLock");
  if (lock.kind === "shallow") return t("workspace.controller.gitShallowLock");
  return t("workspace.controller.otherGitLock");
}

function repositoryLockDescription(lock: RepositoryLockInfo): string {
  const modifiedDate = lock.modifiedAtMs === null ? null : new Date(lock.modifiedAtMs);
  const modified = modifiedDate === null || Number.isNaN(modifiedDate.getTime())
    ? t("workspace.controller.unknownModificationTime")
    : formatDate(modifiedDate);
  return `${repositoryLockKindLabel(lock)} · ${modified}\n${lock.path}`;
}

function lockRecoveryIsCurrent(repositoryPath: string, selectionGeneration: number): boolean {
  return repositorySelectionIsCurrent(repositoryPath, selectionGeneration);
}

function clearActiveGitLockRecoveryDialog(): void {
  activeGitLockRecoveryDialog = replaceGitLockRecoveryDialog(
    activeGitLockRecoveryDialog,
    null,
  );
  activeGitLockRecoveryDialogOwner = null;
  activeGitLockRecoveryDialogTarget = null;
}

function clearStaleGitLockRecoveryDialog(): void {
  const target = activeGitLockRecoveryDialogTarget;
  if (
    target
    && !repositorySelectionIsCurrent(target.repositoryPath, target.selectionGeneration)
  ) {
    clearActiveGitLockRecoveryDialog();
  }
}

function releaseOwnedGitLockRecoveryDialog(dialogOwner: symbol): void {
  if (activeGitLockRecoveryDialogOwner !== dialogOwner) return;
  activeGitLockRecoveryDialog = null;
  activeGitLockRecoveryDialogOwner = null;
  activeGitLockRecoveryDialogTarget = null;
}

async function openLockDirectoryForRecovery(
  repositoryPath: string,
  selectionGeneration: number,
  lock: RepositoryLockInfo,
  dialogOwner: symbol,
): Promise<void> {
  if (
    !lockRecoveryIsCurrent(repositoryPath, selectionGeneration)
    || activeGitLockRecoveryDialogOwner !== dialogOwner
  ) {
    message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
    return;
  }
  try {
    await api.openRepositoryLockDirectory(repositoryPath, lock.id);
  } catch (error) {
    if (
      lockRecoveryIsCurrent(repositoryPath, selectionGeneration)
      && activeGitLockRecoveryDialogOwner === dialogOwner
    ) {
      const appError = normalizedAppError(error);
      if (appError.code === "git_lock_changed") {
        reportError(error, t("workspace.controller.couldNotOpenGitMetadataDirectory"), {
          repositoryPath,
          selectionGeneration,
        });
      } else {
        message.error(`${t("workspace.controller.couldNotOpenGitMetadataDirectory")}: ${appError.message}`);
      }
    }
  }
}

async function openGitDirectoriesForLockRecovery(
  repositoryPath: string,
  selectionGeneration: number,
  notificationOwner: symbol,
): Promise<void> {
  if (
    !lockRecoveryIsCurrent(repositoryPath, selectionGeneration)
    || activeErrorNotificationOwner !== notificationOwner
  ) {
    message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
    return;
  }
  try {
    const result = await api.openRepositoryGitDirectories(repositoryPath);
    if (
      result.failedCount > 0
      && lockRecoveryIsCurrent(repositoryPath, selectionGeneration)
      && activeErrorNotificationOwner === notificationOwner
    ) {
      message.warning(t("workspace.controller.someGitMetadataDirectoriesCouldNotBeOpened", {
        opened: result.openedCount,
        failed: result.failedCount,
      }));
    }
  } catch (error) {
    if (
      lockRecoveryIsCurrent(repositoryPath, selectionGeneration)
      && activeErrorNotificationOwner === notificationOwner
    ) {
      const appError = normalizedAppError(error);
      message.error(`${t("workspace.controller.couldNotOpenGitMetadataDirectory")}: ${appError.message}`);
    }
  }
}

async function inspectRepositoryLocks(
  repositoryPath: string,
  selectionGeneration: number,
  notificationOwner: symbol,
  lockErrorCode: string,
): Promise<void> {
  if (!gitLockInspectionIsCurrent(
    lockRecoveryIsCurrent(repositoryPath, selectionGeneration),
    activeErrorNotificationOwner,
    notificationOwner,
  )) {
    message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
    return;
  }
  try {
    const lockResult = await runOwnedGitLockInspection(
      gitLockRecoveryBusyOwners,
      notificationOwner,
      () => gitLockInspectionIsCurrent(
        lockRecoveryIsCurrent(repositoryPath, selectionGeneration),
        activeErrorNotificationOwner,
        notificationOwner,
      ),
      () => api.listRepositoryLocks(repositoryPath),
    );
    if (!lockResult) return;
    const { locks } = lockResult;
    const disposition = gitLockInventoryDisposition(
      locks.length,
      lockResult.incomplete,
      lockErrorCode === "git_reference_locked",
    );
    if (lockResult.incomplete) {
      message.warning(t("workspace.controller.someGitLockPathsCouldNotBeInspected"));
      if (disposition === "inconclusive") return;
    }
    if (disposition === "empty" || disposition === "emptyReference") {
      if (disposition === "empty") {
        destroyOwnedErrorNotification(notificationOwner);
      }
      message.info(t("workspace.controller.noGitLocksFoundRetryTheOperation"));
      return;
    }
    destroyOwnedErrorNotification(notificationOwner);
    const dialogOwner = Symbol("git-lock-recovery-dialog");
    activeGitLockRecoveryDialogOwner = dialogOwner;
    activeGitLockRecoveryDialogTarget = { repositoryPath, selectionGeneration };
    activeGitLockRecoveryDialog = replaceGitLockRecoveryDialog(
      activeGitLockRecoveryDialog,
      dialog.warning({
      title: t("workspace.controller.gitLockRecovery"),
      content: () => h("div", { class: "git-lock-recovery" }, [
        h("div", t("workspace.controller.closeOtherGitClientsBeforeCleaningLocks")),
        h(NScrollbar, {
          class: "git-lock-recovery-list",
          contentClass: "git-lock-recovery-list-content",
        }, { default: () => locks.map((lock) => h("div", {
            key: lock.id,
            class: "git-lock-recovery-item",
          }, [
            h("pre", {
              class: "git-lock-recovery-path",
            }, repositoryLockDescription(lock)),
            h(NButton, {
              size: "tiny",
              tertiary: true,
              onClick: () => void openLockDirectoryForRecovery(
                repositoryPath,
                selectionGeneration,
                lock,
                dialogOwner,
              ),
            }, { default: () => t("workspace.controller.openGitMetadataDirectory") }),
          ])),
        }),
      ]),
      negativeText: t("workspace.controller.close"),
      onNegativeClick: () => releaseOwnedGitLockRecoveryDialog(dialogOwner),
      onClose: () => releaseOwnedGitLockRecoveryDialog(dialogOwner),
      }),
    );
  } catch (error) {
    if (
      lockRecoveryIsCurrent(repositoryPath, selectionGeneration)
      && activeErrorNotificationOwner === notificationOwner
    ) {
      reportError(error, t("workspace.controller.couldNotInspectGitLocks"), {
        repositoryPath,
        selectionGeneration,
      });
    }
  }
}

function reportRepositoryLockError(
  appError: AppError,
  action: string,
  repositoryPath: string,
  selectionGeneration: number,
): void {
  clearActiveGitLockRecoveryDialog();
  const notificationOwner = Symbol("git-lock-recovery");
  activeErrorNotification?.destroy();
  activeErrorNotificationOwner = notificationOwner;
  activeErrorNotification = notification.error({
    title: action,
    content: appError.message,
    meta: `${t("workspace.content.errorCode")}: ${appError.code}`,
    description: appError.details || undefined,
    action: () => h(NSpace, { size: 8 }, {
      default: () => [
        h(NButton, {
          size: "small",
          tertiary: true,
          onClick: () => void openGitDirectoriesForLockRecovery(
            repositoryPath,
            selectionGeneration,
            notificationOwner,
          ),
        }, { default: () => t("workspace.controller.openGitMetadataDirectory") }),
        h(NButton, {
          size: "small",
          type: "primary",
          loading: gitLockRecoveryBusyOwners.has(notificationOwner),
          disabled: gitLockRecoveryBusyOwners.has(notificationOwner),
          onClick: () => void inspectRepositoryLocks(
            repositoryPath,
            selectionGeneration,
            notificationOwner,
            appError.code,
          ),
        }, { default: () => t("workspace.controller.inspectGitLocks") }),
      ],
    }),
    duration: 0,
    closable: true,
    onClose: () => {
      if (activeErrorNotificationOwner === notificationOwner) {
        activeErrorNotification = null;
        activeErrorNotificationOwner = null;
      }
    },
  });
}

function reportError(
  error: unknown,
  action: string,
  repositoryTarget?: RepositoryErrorTarget,
): void {
  const appError = normalizedAppError(error);
  if (
    repositoryTarget
    && recoverableGitLockCodes.has(appError.code)
    && repositorySelectionIsCurrent(
      repositoryTarget.repositoryPath,
      repositoryTarget.selectionGeneration,
    )
  ) {
    reportRepositoryLockError(
      appError,
      action,
      repositoryTarget.repositoryPath,
      repositoryTarget.selectionGeneration,
    );
    return;
  }
  clearStaleGitLockRecoveryDialog();
  activeErrorNotification?.destroy();
  activeErrorNotificationOwner = null;
  activeErrorNotification = notification.error({
    title: action,
    content: appError.message,
    meta: `${t("workspace.content.errorCode")}: ${appError.code}`,
    description: appError.details || undefined,
    duration: 0,
    closable: true,
  });
  if (appError.code === "account_selection_required" && activeRemote.value) {
    message.warning(t("workspace.controller.thisRemoteHasMultipleOrNoUsableAccountsSelectOneExplicitlyAndRetry"));
    openBindingDrawer(activeRemote.value);
  }
}

function reportCapturedRepositoryError(
  error: unknown,
  action: string,
  repositoryPath?: string,
  selectionGeneration?: number,
): void {
  if (!repositoryPath || selectionGeneration === undefined) {
    reportError(error, action);
    return;
  }
  reportError(error, action, {
    repositoryPath,
    selectionGeneration,
  });
}

function networkActionTargetIsCurrent(target: NetworkActionTarget): boolean {
  return repositorySelectionIsCurrent(target.repositoryPath, target.selectionGeneration)
    && activeRemote.value?.name === target.remoteName
    && remoteProfileIdentity(activeRemote.value) === target.remoteIdentity;
}

function networkActionAttemptIsCurrent(
  target: NetworkActionTarget,
  operationGeneration: number,
): boolean {
  return controllerMounted
    && operationGeneration === networkOperationGeneration
    && networkActionTargetIsCurrent(target);
}

function networkActionPromptIsCurrent(
  target: NetworkActionTarget,
  operationGeneration: number,
  owner: symbol,
): boolean {
  return activeErrorNotificationOwner === owner
    && networkActionAttemptIsCurrent(target, operationGeneration);
}

function destroyOwnedErrorNotification(owner: symbol): void {
  if (activeErrorNotificationOwner !== owner) return;
  activeErrorNotification?.destroy();
  activeErrorNotification = null;
  activeErrorNotificationOwner = null;
}

function clearActiveErrorNotification(): void {
  activeErrorNotification?.destroy();
  activeErrorNotification = null;
  activeErrorNotificationOwner = null;
}

function openRepositorySecuritySettings(
  target: NetworkActionTarget,
  operationGeneration: number,
  owner: symbol,
): void {
  if (!networkActionPromptIsCurrent(target, operationGeneration, owner)) {
    message.warning(t("workspace.controller.theRepositoryOrRemoteChangedStartTheOperationAgain"));
    return;
  }
  destroyOwnedErrorNotification(owner);
  settingsSection.value = "repository";
  settingsModalShown.value = true;
}

async function approveNetworkConfigurationAndRetry(
  action: NetworkAction,
  forceWithLease: boolean,
  target: NetworkActionTarget,
  operationGeneration: number,
  notificationOwner: symbol,
): Promise<void> {
  if (networkTrustActionBusyOwners.has(notificationOwner)) return;
  if (!networkActionPromptIsCurrent(target, operationGeneration, notificationOwner)) {
    message.warning(t("workspace.controller.theRepositoryOrRemoteChangedStartTheOperationAgain"));
    return;
  }
  networkTrustActionBusyOwners.add(notificationOwner);
  try {
    const status = await api.getRepositoryNetworkSecurity(target.repositoryPath);
    if (!networkActionPromptIsCurrent(target, operationGeneration, notificationOwner)) {
      message.warning(t("workspace.controller.theRepositoryOrRemoteChangedStartTheOperationAgain"));
      return;
    }
    if (!status.currentFingerprint) {
      destroyOwnedErrorNotification(notificationOwner);
      message.info(t("workspace.controller.dangerousGitConfigurationNoLongerPresentRetrying"));
      await runNetworkAction(action, forceWithLease, target);
      return;
    }
    await api.approveRepositoryNetworkConfiguration(
      target.repositoryPath,
      status.currentFingerprint,
    );
    if (!networkActionAttemptIsCurrent(target, operationGeneration)) {
      destroyOwnedErrorNotification(notificationOwner);
      message.warning(t("workspace.controller.repositoryConfigurationAuthorizedButSelectionChanged"));
      return;
    }
    destroyOwnedErrorNotification(notificationOwner);
    message.success(t("workspace.controller.currentDangerousGitConfigurationAuthorized"));
    await runNetworkAction(action, forceWithLease, target);
  } catch (error) {
    if (!networkActionAttemptIsCurrent(target, operationGeneration)) {
      destroyOwnedErrorNotification(notificationOwner);
      message.warning(t("workspace.controller.theRepositoryOrRemoteChangedStartTheOperationAgain"));
    } else if (activeErrorNotificationOwner === notificationOwner) {
      reportError(error, t("workspace.controller.couldNotAuthorizeRepositoryGitConfiguration"), {
        repositoryPath: target.repositoryPath,
        selectionGeneration: target.selectionGeneration,
      });
    } else {
      message.error(t("workspace.controller.couldNotAuthorizeRepositoryGitConfiguration"));
    }
  } finally {
    networkTrustActionBusyOwners.delete(notificationOwner);
  }
}

function reportUnsafeNetworkConfiguration(
  error: unknown,
  actionLabel: string,
  action: NetworkAction,
  forceWithLease: boolean,
  target: NetworkActionTarget,
  operationGeneration: number,
): void {
  const appError = normalizedAppError(error);
  const notificationOwner = Symbol("unsafe-network-configuration");
  activeErrorNotification?.destroy();
  activeErrorNotificationOwner = notificationOwner;
  activeErrorNotification = notification.error({
    title: actionLabel,
    content: appError.message,
    meta: `${t("workspace.content.errorCode")}: ${appError.code}`,
    description: appError.details || undefined,
    action: () => h(NSpace, { size: 8 }, {
      default: () => [
        h(NButton, {
          size: "small",
          tertiary: true,
          onClick: () => openRepositorySecuritySettings(
            target,
            operationGeneration,
            notificationOwner,
          ),
        }, { default: () => t("workspace.controller.goToSecuritySettings") }),
        h(NButton, {
          size: "small",
          type: "primary",
          loading: networkTrustActionBusyOwners.has(notificationOwner),
          disabled: networkTrustActionBusyOwners.has(notificationOwner),
          onClick: () => void approveNetworkConfigurationAndRetry(
            action,
            forceWithLease,
            target,
            operationGeneration,
            notificationOwner,
          ),
        }, { default: () => t("workspace.controller.allowAndRetry") }),
      ],
    }),
    duration: 0,
    closable: true,
    onClose: () => {
      if (activeErrorNotificationOwner === notificationOwner) {
        activeErrorNotification = null;
        activeErrorNotificationOwner = null;
      }
    },
  });
}

async function refreshBrowserDemoLocalizedData(): Promise<void> {
  if (!browserDemo || !controllerMounted) return;
  const refreshGeneration = ++browserDemoLocaleRefreshGeneration;
  const repositoryPath = activeRepositoryPath.value;
  const selectionGeneration = repositorySelectionGeneration;
  const selectedPath = selectedChange.value?.path;
  const selectedCommitSha = selectedCommit.value?.sha;
  const selectedCommitFilePath = selectedCommitFile.value?.path;
  const selectedCommitFilePreviousPath = selectedCommitFile.value?.previousPath;
  const refreshRepositoryInfo = Boolean(repositoryInfo.value) || activeTab.value === "overview";
  const refreshPullRequests = Boolean(pullRequests.value.length) || activeTab.value === "overview";
  const refreshBlame = blameModalShown.value && Boolean(selectedPath);
  const refreshReflog = reflogModalShown.value;
  const refreshIsCurrent = () => refreshGeneration === browserDemoLocaleRefreshGeneration
    && controllerMounted
    && activeRepositoryPath.value === repositoryPath
    && (!repositoryPath || repositorySelectionIsCurrent(repositoryPath, selectionGeneration));
  try {
    const [localizedRepositories, localizedAccounts] = await Promise.all([
      api.listRepositories(false),
      api.listAccounts(),
    ]);
    if (refreshGeneration !== browserDemoLocaleRefreshGeneration || !controllerMounted) return;
    repositories.value = localizedRepositories;
    accounts.value = localizedAccounts;
    if (!repositoryPath || !refreshIsCurrent()) return;

    await refreshSnapshotWithOptions({ includeAuxiliary: false, reportErrors: false, silent: true });
    if (!refreshIsCurrent()) return;

    if (selectedPath) {
      const entry = snapshot.value?.status.entries.find((candidate) => candidate.path === selectedPath);
      if (entry && selectedChange.value?.path === selectedPath) {
        await loadDiff(entry, repositoryPath, false);
      }
      if (!refreshIsCurrent()) return;
    }

    const shouldRefreshHistory = activeTab.value === "history" || historyWasLoaded(repositoryPath);
    if (shouldRefreshHistory) {
      await loadHistoryWithOptions({ force: true, reportErrors: false, silent: true });
    }
    if (!refreshIsCurrent()) return;

    if (
      selectedCommitSha &&
      selectedCommit.value?.sha === selectedCommitSha &&
      selectedCommitFile.value?.path === selectedCommitFilePath &&
      selectedCommitFile.value?.previousPath === selectedCommitFilePreviousPath
    ) {
      const localizedCommit = commits.value.find((commit) => commit.sha === selectedCommitSha);
      if (localizedCommit) {
        selectedCommit.value = localizedCommit;
        const localizedFile = commitChangedFiles.value.find((file) =>
          file.path === selectedCommitFilePath && file.previousPath === selectedCommitFilePreviousPath);
        if (localizedFile) {
          invalidateCommitDiffCache();
          loadCommitFileDiff(localizedFile, localizedCommit, repositoryPath);
        }
      }
    }

    const remoteName = activeRemote.value?.name;
    if (remoteName && refreshRepositoryInfo && activeCapabilities.value.repositoryInfo) {
      const localizedRepositoryInfo = await api.getCodeHostRepositoryInfo(repositoryPath, remoteName);
      if (!refreshIsCurrent() || activeRemote.value?.name !== remoteName) return;
      repositoryInfo.value = localizedRepositoryInfo;
    }
    if (remoteName && refreshPullRequests && activeCapabilities.value.pullRequestsRead) {
      const localizedPullRequests = await api.listPullRequests(repositoryPath, remoteName);
      if (!refreshIsCurrent() || activeRemote.value?.name !== remoteName) return;
      pullRequests.value = localizedPullRequests;
    }
    if (
      refreshBlame
      && selectedPath
      && blameModalShown.value
      && selectedChange.value?.path === selectedPath
    ) {
      await loadFileBlame(
        repositoryPath,
        selectedPath,
        selectionGeneration,
        false,
        refreshIsCurrent,
      );
      if (!refreshIsCurrent()) return;
    }
    if (refreshReflog && reflogModalShown.value) {
      await loadReflog(repositoryPath, selectionGeneration, false, refreshIsCurrent);
      if (!refreshIsCurrent()) return;
    }
    if (publishModalShown.value && refreshIsCurrent()) await loadPublishOwners();
  } catch (error) {
    if (refreshGeneration === browserDemoLocaleRefreshGeneration && controllerMounted) {
      reportError(error, t("workspace.controller.couldNotRefreshLocalizedDemoData"));
    }
  }
}

function openRepositoryAction(value: string | number): void {
  if (value === "discover") void openRepositoryDiscovery();
  else if (value === "init") initModalShown.value = true;
  else if (value === "clone") cloneModalShown.value = true;
}

function handleRepositoryViewportKeydown(event: KeyboardEvent): void {
  if (["ArrowDown", "ArrowUp", "End", "Home", "PageDown", "PageUp", " "].includes(event.key)) {
    markRepositoryViewportInteraction();
  }
}

function repositorySelectionIsCurrent(repositoryPath: string, generation: number): boolean {
  return controllerMounted
    && activeRepositoryPath.value === repositoryPath
    && repositorySelectionGeneration === generation;
}

function preferredRemoteName(repositorySnapshot: RepositorySnapshot): string {
  const upstream = repositorySnapshot.status.upstream;
  const upstreamRemote = upstream
    ? [...repositorySnapshot.remotes]
        .sort((left, right) => right.name.length - left.name.length)
        .find((remote) => upstream.startsWith(`${remote.name}/`))
    : undefined;
  return upstreamRemote?.name
    ?? repositorySnapshot.remotes.find((remote) => remote.name === "origin")?.name
    ?? repositorySnapshot.remotes[0]?.name
    ?? "";
}

function applySnapshot(nextSnapshot: RepositorySnapshot, invalidateRefreshRequests = true): boolean {
  // Every backend snapshot is scoped to one repository. Never let a late
  // mutation response overwrite the currently selected repository.
  if (activeRepositoryPath.value !== nextSnapshot.repository.path) return false;
  if (
    snapshot.value &&
    repositorySnapshotSignature(snapshot.value) === repositorySnapshotSignature(nextSnapshot)
  ) {
    if (invalidateRefreshRequests) snapshotRefreshRequestGeneration += 1;
    return true;
  }
  snapshotApplicationGeneration += 1;
  if (invalidateRefreshRequests) snapshotRefreshRequestGeneration += 1;
  const selectedPath = selectedChange.value?.path;
  const previousSnapshot = snapshot.value;
  snapshot.value = previousSnapshot
    ? {
        ...nextSnapshot,
        branches: reuseEqualArray(previousSnapshot.branches, nextSnapshot.branches, branchesAreEqual),
        remoteBranches: reuseEqualArray(previousSnapshot.remoteBranches, nextSnapshot.remoteBranches, remoteBranchesAreEqual),
        remotes: reuseEqualArray(previousSnapshot.remotes, nextSnapshot.remotes, remotesAreEqual),
        tags: reuseEqualArray(previousSnapshot.tags, nextSnapshot.tags, (left, right) =>
          left.name === right.name && left.target === right.target),
        bindings: reuseEqualArray(previousSnapshot.bindings, nextSnapshot.bindings, bindingsAreEqual),
      }
    : nextSnapshot;
  replaceRepositoryRecord(nextSnapshot.repository);
  const branchTargetChanged = !previousSnapshot
    || previousSnapshot.status.branch !== nextSnapshot.status.branch
    || previousSnapshot.status.upstream !== nextSnapshot.status.upstream;
  if (
    branchTargetChanged
    || !nextSnapshot.remotes.some((remote) => remote.name === selectedRemoteName.value)
  ) {
    selectedRemoteName.value = preferredRemoteName(nextSnapshot);
  }
  if (selectedPath) {
    const nextSelectedChange = nextSnapshot.status.entries.find((entry) => entry.path === selectedPath) ?? null;
    if (nextSelectedChange) {
      selectedChange.value = nextSelectedChange;
    } else {
      clearDiffSelection();
    }
  }
  return true;
}

async function loadCapabilities(
  capturedRepositoryPath = activeRepositoryPath.value,
  capturedSelectionGeneration = repositorySelectionGeneration,
): Promise<void> {
  const endpointId = activeRemote.value?.endpointId;
  if (!capturedRepositoryPath || !repositorySelectionIsCurrent(capturedRepositoryPath, capturedSelectionGeneration)) return;
  if (!endpointId) {
    activeCapabilities.value = emptyCapabilities;
    return;
  }
  try {
    const capabilities = await api.getCodeHostCapabilities(endpointId);
    if (
      !repositorySelectionIsCurrent(capturedRepositoryPath, capturedSelectionGeneration)
      || activeRemote.value?.endpointId !== endpointId
    ) return;
    activeCapabilities.value = capabilities;
  } catch {
    if (
      repositorySelectionIsCurrent(capturedRepositoryPath, capturedSelectionGeneration)
      && activeRemote.value?.endpointId === endpointId
    ) activeCapabilities.value = emptyCapabilities;
  }
}

function installRepositoryWatcher(repositoryPath: string): Promise<void> {
  if (repositoryWatcherRetryTimer) clearTimeout(repositoryWatcherRetryTimer);
  repositoryWatcherRetryTimer = null;
  const previousGeneration = repositoryWatchRequestGeneration;
  const requestGeneration = ++repositoryWatchRequestGeneration;
  if (previousGeneration > 0) {
    repositoryWatchSupersededGenerations.add(previousGeneration);
    if (repositoryWatchSupersededGenerations.size > 32) {
      const oldestGeneration = repositoryWatchSupersededGenerations.values().next().value;
      if (oldestGeneration !== undefined) repositoryWatchSupersededGenerations.delete(oldestGeneration);
    }
  }
  const watchPlanGeneration = repositoryWatchPlanChangeGeneration;
  return api.watchRepositoryChanges(repositoryPath, requestGeneration)
    .then(() => {
      if (
        controllerMounted &&
        repositoryWatchRequestGeneration === requestGeneration &&
        activeRepositoryPath.value === repositoryPath
      ) {
        repositoryWatcherRetryAttempt = 0;
        repositoryWatcherUnavailable = false;
        repositoryWatchPlanAppliedGeneration = Math.max(
          repositoryWatchPlanAppliedGeneration,
          watchPlanGeneration,
        );
      }
    })
    .catch((error) => {
      if (
        controllerMounted &&
        repositoryWatchRequestGeneration === requestGeneration &&
        activeRepositoryPath.value === repositoryPath
      ) {
        handleRepositoryWatchError({
          payload: {
            repositoryPath,
            requestGeneration,
            errorKind: isAppError(error) ? error.code : "installation_failed",
            retryable: !isAppError(error) || error.code !== "repository_watch_failed_permanent",
            historyObjectsChanged: false,
            stashesChanged: false,
            watchPlanChanged: false,
          },
        });
      }
    });
}

async function selectRepository(
  repositoryPath: string,
  reportOpenError = true,
  deferWatcherUntilInteractive = false,
): Promise<void> {
  clearActiveGitLockRecoveryDialog();
  invalidatePlatformRequests();
  remoteModalShown.value = false;
  bindingDrawerShown.value = false;
  pullRequestModalShown.value = false;
  publishModalShown.value = false;
  if (repositoryWatcherRetryTimer) clearTimeout(repositoryWatcherRetryTimer);
  repositoryWatcherRetryTimer = null;
  repositoryWatcherRetryAttempt = 0;
  repositoryWatcherUnavailable = false;
  repositoryWatchPlanChangeGeneration = 0;
  repositoryWatchPlanAppliedGeneration = 0;
  repositoryWatchSupersededGenerations.clear();
  snapshotApplicationGeneration += 1;
  snapshotRefreshRequestGeneration += 1;
  resetRepositoryHistory();
  resetRepositoryAuxiliary();
  networkOperationGeneration += 1;
  clearActiveErrorNotification();
  networkBusy.value = null;
  resetRepositoryOperations();
  snapshot.value = null;
  repositoryInfo.value = null;
  pullRequests.value = [];
  activeRepositoryPath.value = repositoryPath;
  const selectionGeneration = ++repositorySelectionGeneration;
  section.value = "repositories";
  snapshotLoading.value = true;
  try {
    clearDiffSelection();
    const nextSnapshot = await api.getRepositorySnapshot(repositoryPath);
    if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
    if (deferWatcherUntilInteractive) markPerformance("workspace:snapshot-apply:start");
    applySnapshot(nextSnapshot);
    if (deferWatcherUntilInteractive) {
      markPerformance("workspace:snapshot-apply:end");
      measurePerformance("Snapshot application (sync)", "workspace:snapshot-apply:start", "workspace:snapshot-apply:end");
      if (typeof requestAnimationFrame === "function") {
        requestAnimationFrame(() => {
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
          markPerformance("workspace:snapshot-rendered-frame");
          measurePerformance(
            "Snapshot application to rendered frame",
            "workspace:snapshot-apply:start",
            "workspace:snapshot-rendered-frame",
          );
        });
      }
    }
    rememberLastRepositoryPath(nextSnapshot.repository.path);
    activeTab.value = "changes";
    repositoryInfo.value = null;
    pullRequests.value = [];
    commits.value = [];
    stashes.value = [];
    clearCommitSelection();
    amendLastCommit.value = false;
    await Promise.all([syncSelectedDiff(nextSnapshot, undefined, true), loadCapabilities()]);
    if (
      !deferWatcherUntilInteractive
      && repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
    ) {
      // Recursive watcher installation can be expensive for repositories
      // containing large dependency or build trees. Its initial event already
      // schedules a reconciling refresh, so install it after the first visible
      // Diff instead of competing with the startup critical path.
      void installRepositoryWatcher(nextSnapshot.repository.path);
    }
  } catch (error) {
    if (reportOpenError && repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      reportError(error, t("workspace.controller.couldNotOpenRepository"), {
        repositoryPath,
        selectionGeneration,
      });
    }
  } finally {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      snapshotLoading.value = false;
    }
  }
}

async function refreshSnapshot(): Promise<void> {
  const repositoryPath = activeRepositoryPath.value;
  const previousApplicationGeneration = snapshotApplicationGeneration;
  const previousHistorySignature = historySnapshotSignature(snapshot.value);
  await refreshSnapshotWithOptions();
  if (
    !repositoryPath ||
    activeRepositoryPath.value !== repositoryPath ||
    snapshotApplicationGeneration === previousApplicationGeneration
  ) return;
  invalidateStashesCache();
  if (historySnapshotSignature(snapshot.value) !== previousHistorySignature) {
    invalidateCommitHistoryCache();
    invalidateCommitDetailCaches();
  }
  if (activeTab.value === "history") await loadHistoryWithOptions({ force: true });
}

async function refreshSnapshotWithOptions(options: RefreshSnapshotOptions = {}): Promise<void> {
  if (!activeRepositoryPath.value) return;
  const {
    includeAuxiliary = true,
    reportErrors = true,
    silent = false,
  } = options;
  const repositoryPath = activeRepositoryPath.value;
  const selectionGeneration = repositorySelectionGeneration;
  const applicationGeneration = snapshotApplicationGeneration;
  const refreshGeneration = ++snapshotRefreshRequestGeneration;
  if (!silent) snapshotLoading.value = true;
  try {
    const selectedPath = selectedChange.value?.path;
    const selectedDiffGeneration = getDiffRequestGeneration();
    const nextSnapshot = await api.refreshRepositorySnapshot(repositoryPath);
    if (
      !repositorySelectionIsCurrent(repositoryPath, selectionGeneration) ||
      applicationGeneration !== snapshotApplicationGeneration ||
      refreshGeneration !== snapshotRefreshRequestGeneration
    ) return;
    if (!applySnapshot(nextSnapshot, false)) return;
    const appliedApplicationGeneration = snapshotApplicationGeneration;
    const snapshotChanged = appliedApplicationGeneration !== applicationGeneration;
    if (snapshotChanged && selectedDiffGeneration === getDiffRequestGeneration()) {
      await syncSelectedDiff(nextSnapshot, selectedPath, !selectedPath, repositoryPath, reportErrors);
    }
    if (
      includeAuxiliary &&
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      appliedApplicationGeneration === snapshotApplicationGeneration &&
      refreshGeneration === snapshotRefreshRequestGeneration
    ) {
      await Promise.all([
        loadWorktrees(reportErrors, !silent),
        loadSubmodules(reportErrors, !silent),
        loadLfsStatus(reportErrors, !silent),
      ]);
    }
  } catch (error) {
    if (
      reportErrors &&
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      applicationGeneration === snapshotApplicationGeneration &&
      refreshGeneration === snapshotRefreshRequestGeneration
    ) {
      reportError(error, t("workspace.controller.couldNotRefreshRepository"), {
        repositoryPath,
        selectionGeneration,
      });
    }
  } finally {
    if (
      !silent &&
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      refreshGeneration === snapshotRefreshRequestGeneration
    ) {
      snapshotLoading.value = false;
    }
  }
}

function applicationCanAutoRefresh(): boolean {
  return controllerMounted && windowFocused && (typeof document === "undefined" || !document.hidden);
}

function markRepositoryViewportInteraction(): void {
  repositoryViewportInteractionGeneration += 1;
}

function captureRepositoryViewport(): RepositoryViewportSnapshot | null {
  if (typeof document === "undefined") return null;
  const element = document.querySelector<HTMLElement>(".content-area > .n-scrollbar > .n-scrollbar-container");
  if (!element) return null;
  return {
    element,
    scrollTop: element.scrollTop,
    scrollLeft: element.scrollLeft,
    interactionGeneration: repositoryViewportInteractionGeneration,
    repositoryPath: activeRepositoryPath.value,
    tab: activeTab.value,
  };
}

async function restoreRepositoryViewport(viewport: RepositoryViewportSnapshot | null): Promise<void> {
  if (!viewport) return;
  await nextTick();
  if (
    repositoryViewportInteractionGeneration !== viewport.interactionGeneration ||
    activeRepositoryPath.value !== viewport.repositoryPath ||
    activeTab.value !== viewport.tab ||
    !viewport.element.isConnected
  ) return;
  viewport.element.scrollTop = viewport.scrollTop;
  viewport.element.scrollLeft = viewport.scrollLeft;
}

function scheduleAutomaticRefresh(delay = automaticRefreshDebounceMs): void {
  automaticRefreshDirty = true;
  if (!applicationCanAutoRefresh() || section.value !== "repositories" || !activeRepositoryPath.value) return;
  if (automaticRefreshTimer) clearTimeout(automaticRefreshTimer);
  automaticRefreshTimer = setTimeout(() => {
    automaticRefreshTimer = null;
    void runAutomaticRefresh();
  }, delay);
}

async function runAutomaticRefresh(): Promise<void> {
  if (!automaticRefreshDirty || !applicationCanAutoRefresh() || section.value !== "repositories") return;
  if (
    automaticRefreshInFlight ||
    snapshotLoading.value ||
    mutationBusy.value ||
    networkBusy.value ||
    (typeof document !== "undefined" && document.documentElement.classList.contains("is-resizing-repository-columns"))
  ) {
    scheduleAutomaticRefresh(automaticRefreshRetryMs);
    return;
  }

  automaticRefreshDirty = false;
  automaticRefreshInFlight = true;
  const viewport = captureRepositoryViewport();
  const includeAuxiliary = activeTab.value === "branches";
  const repositoryPath = activeRepositoryPath.value;
  const previousHistorySignature = historySnapshotSignature(snapshot.value);
  try {
    await refreshSnapshotWithOptions({
      includeAuxiliary,
      reportErrors: false,
      silent: true,
    });
    if (activeRepositoryPath.value !== repositoryPath) return;
    const historyChanged = historySnapshotSignature(snapshot.value) !== previousHistorySignature;
    if (historyChanged) invalidateCommitHistoryCache();
    if (activeTab.value === "history" && activeRepositoryPath.value === repositoryPath) {
      await Promise.all([
        loadCommitHistoryWithOptions({
          force: repositoryWatcherUnavailable,
          reportErrors: false,
        }),
        loadStashesWithOptions({
          force: repositoryWatcherUnavailable,
          reportErrors: false,
        }),
      ]);
    }
  } finally {
    await restoreRepositoryViewport(viewport);
    automaticRefreshInFlight = false;
    if (
      controllerMounted
      && repositoryPath
      && activeRepositoryPath.value === repositoryPath
      && repositoryWatchPlanAppliedGeneration < repositoryWatchPlanChangeGeneration
    ) {
      void installRepositoryWatcher(repositoryPath);
    }
    if (automaticRefreshDirty) scheduleAutomaticRefresh();
  }
}

function handleRepositoryChanged(event: { payload: RepositoryChangedPayload }): void {
  const supersededPlanChange = event.payload.watchPlanChanged
    && repositoryWatchSupersededGenerations.has(event.payload.requestGeneration);
  if (
    event.payload.repositoryPath !== activeRepositoryPath.value ||
    (event.payload.requestGeneration !== repositoryWatchRequestGeneration && !supersededPlanChange)
  ) return;
  if (event.payload.historyObjectsChanged) {
    invalidateCommitHistoryCache();
    invalidateCommitDetailCaches();
  }
  if (event.payload.stashesChanged) invalidateStashesCache();
  if (event.payload.watchPlanChanged) repositoryWatchPlanChangeGeneration += 1;
  scheduleAutomaticRefresh();
}

function handleRepositoryWatchError(event: { payload: RepositoryChangedPayload }): void {
  if (
    event.payload.repositoryPath !== activeRepositoryPath.value ||
    event.payload.requestGeneration !== repositoryWatchRequestGeneration
  ) return;
  scheduleAutomaticRefresh();
  repositoryWatcherUnavailable = true;
  if (repositoryWatcherRetryTimer) clearTimeout(repositoryWatcherRetryTimer);
  if (!event.payload.retryable || repositoryWatcherRetryAttempt >= repositoryWatcherMaxRetryAttempts) {
    repositoryWatcherRetryTimer = null;
    return;
  }
  repositoryWatcherRetryAttempt += 1;
  const requestGeneration = event.payload.requestGeneration;
  const retryDelay = Math.min(
    automaticRefreshRetryMs * (2 ** (repositoryWatcherRetryAttempt - 1)),
    repositoryWatcherMaxRetryDelayMs,
  );
  repositoryWatcherRetryTimer = setTimeout(() => {
    repositoryWatcherRetryTimer = null;
    if (
      controllerMounted &&
      repositoryWatchRequestGeneration === requestGeneration &&
      activeRepositoryPath.value === event.payload.repositoryPath
    ) {
      void installRepositoryWatcher(event.payload.repositoryPath);
    }
  }, retryDelay);
}

function handleWindowFocus(): void {
  windowFocused = true;
  if (repositoryWatcherUnavailable && activeRepositoryPath.value) {
    repositoryWatcherRetryAttempt = 0;
    void installRepositoryWatcher(activeRepositoryPath.value);
  }
  if (section.value === "accounts") {
    void refreshAccountStatuses();
  } else {
    scheduleAutomaticRefresh(0);
  }
}

function handleWindowBlur(): void {
  windowFocused = false;
}

function handleVisibilityChange(): void {
  if (document.hidden) {
    windowFocused = false;
    return;
  }
  windowFocused = document.hasFocus();
  if (windowFocused) handleWindowFocus();
}

// A backend mutation can succeed before its follow-up snapshot read fails.
// Reconcile silently on an error path so the UI does not invite the user to
// retry a non-idempotent action from stale state.
async function reconcileSnapshotAfterMutationError(repositoryPath: string): Promise<void> {
  try {
    const selectedPath = selectedChange.value?.path;
    const selectionGeneration = repositorySelectionGeneration;
    const nextSnapshot = await api.getRepositorySnapshot(repositoryPath);
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      applySnapshot(nextSnapshot);
      await syncSelectedDiff(nextSnapshot, selectedPath, !selectedPath, repositoryPath);
    }
  } catch {
    // Preserve the original mutation error; a best-effort reconciliation must
    // not replace it with a secondary refresh failure.
  }
}

async function cloneRepository(): Promise<void> {
  if (mutationBusy.value) return;
  if (!cloneForm.url.trim() || !cloneForm.destinationPath.trim()) {
    message.warning(t("workspace.controller.enterARemoteUrlAndDestinationFolder"));
    return;
  }
  if (cloneForm.accountId !== "none" && !cloneForm.endpointId) {
    message.warning(t("workspace.controller.whenChoosingAnAppAccountAlsoChooseItsGitServiceEndpoint"));
    return;
  }
  mutationBusy.value = true;
  try {
    const selectedCloneAccount = cloneForm.accountId === "none" || cloneForm.accountId === "auto"
      ? null
      : accounts.value.find((account) => account.id === cloneForm.accountId) ?? null;
    // An SSH agent chooses its key from the user's SSH configuration, so its
    // profile cannot be pinned into Clone's durable origin binding. Keep the
    // selection as a UI hint only and let normal system Git/SSH handle it.
    const usesPinnedCloneAccount = selectedCloneAccount?.authMethod === "personalAccessToken";
    const usesAutomaticCloneAccount = cloneForm.accountId === "auto";
    const repository = await api.cloneRepository({
      url: cloneForm.url.trim(),
      destinationPath: cloneForm.destinationPath.trim(),
      ...(usesPinnedCloneAccount || usesAutomaticCloneAccount ? { endpointId: cloneForm.endpointId } : {}),
      ...(usesPinnedCloneAccount ? { accountProfileId: cloneForm.accountId } : {}),
    });
    const catalogRefreshed = await loadRepositories(repositoryGrouping.value === "owner", false, false);
    if (!catalogRefreshed || !repositories.value.some((item) => item.path === repository.path)) {
      replaceRepositoryRecord(repository);
    }
    if (!catalogRefreshed) {
      message.warning(t("workspace.controller.repositoryClonedCatalogRefreshFailed"));
    }
    cloneModalShown.value = false;
    cloneForm.url = "";
    cloneForm.destinationPath = "";
    cloneForm.parentPath = "";
    cloneForm.repositoryDerivedRouting = false;
    cloneForm.endpointId = "";
    cloneForm.accountId = "none";
    message.success(t("workspace.controller.repositoryClonedOpeningItNow"));
    await selectRepository(repository.path);
  } catch (error) {
    reportError(error, t("workspace.controller.cloneFailed"));
  } finally {
    mutationBusy.value = false;
  }
}

async function initRepository(): Promise<void> {
  if (!initForm.destinationPath.trim()) {
    message.warning(t("workspace.controller.enterADestinationFolderForTheNewRepository"));
    return;
  }
  mutationBusy.value = true;
  try {
    const repository = await api.initRepository({
      destinationPath: initForm.destinationPath.trim(),
      initialBranch: initForm.initialBranch.trim() || undefined,
    });
    const catalogRefreshed = await loadRepositories(repositoryGrouping.value === "owner", false, false);
    if (!catalogRefreshed || !repositories.value.some((item) => item.path === repository.path)) {
      replaceRepositoryRecord(repository);
    }
    if (!catalogRefreshed) {
      message.warning(t("workspace.controller.repositoryInitializedCatalogRefreshFailed"));
    }
    initModalShown.value = false;
    initForm.destinationPath = "";
    initForm.initialBranch = "main";
    message.success(t("workspace.controller.localGitRepositoryCreatedOpeningItNow"));
    await selectRepository(repository.path);
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotCreateLocalRepository"));
  } finally {
    mutationBusy.value = false;
  }
}

async function refreshAfterGitConflict(error: unknown): Promise<void> {
  if (isAppError(error) && error.code === "git_merge_conflict") await refreshSnapshot();
}

function openRemoteEditor(remote?: RemoteProfile): void {
  remoteEditorRepositoryPath.value = activeRepositoryPath.value;
  remoteEditorSelectionGeneration.value = repositorySelectionGeneration;
  editedRemoteName.value = remote?.name ?? null;
  remoteForm.name = remote?.name ?? "";
  remoteForm.fetchUrl = remote?.fetchUrl ?? "";
  remoteForm.pushUrl = remote?.pushUrl ?? "";
  remoteModalShown.value = true;
}

async function saveRemote(): Promise<void> {
  const repositoryPath = remoteEditorRepositoryPath.value;
  const selectionGeneration = remoteEditorSelectionGeneration.value;
  if (
    !repositoryPath
    || selectionGeneration === null
    || !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
  ) {
    message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
    return;
  }
  if (!remoteForm.name.trim() || !remoteForm.fetchUrl.trim()) {
    message.warning(t("workspace.controller.enterARemoteNameAndFetchUrl"));
    return;
  }

  const originalRemoteName = editedRemoteName.value;
  const nextRemoteName = remoteForm.name.trim();
  const fetchUrl = remoteForm.fetchUrl.trim();
  const pushUrl = remoteForm.pushUrl.trim();
  const releaseOperation = acquireRepositoryOperation(repositoryPath);
  if (!releaseOperation) {
    message.warning(t("workspace.controller.aRepositoryOperationIsAlreadyInProgressWaitForItToFinish"));
    return;
  }
  try {
    let currentName = originalRemoteName;
    if (!currentName) {
      await api.addRemote(repositoryPath, nextRemoteName, fetchUrl);
      currentName = nextRemoteName;
    } else if (currentName !== nextRemoteName) {
      await api.renameRemote(repositoryPath, currentName, nextRemoteName);
      currentName = nextRemoteName;
    }
    await api.setRemoteUrl(repositoryPath, currentName, fetchUrl, "fetch");
    if (pushUrl) {
      await api.setRemoteUrl(repositoryPath, currentName, pushUrl, "push");
    }
    if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
    remoteModalShown.value = false;
    remoteEditorRepositoryPath.value = null;
    remoteEditorSelectionGeneration.value = null;
    await refreshSnapshot();
    if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
    message.success(t("workspace.controller.remoteSaved"));
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      reportError(error, t("workspace.controller.couldNotSaveRemote"), {
        repositoryPath,
        selectionGeneration,
      });
    }
  } finally {
    releaseOperation();
  }
}

function confirmRemoveRemote(): void {
  const repositoryPath = remoteEditorRepositoryPath.value;
  const selectionGeneration = remoteEditorSelectionGeneration.value;
  if (!repositoryPath || selectionGeneration === null || !editedRemoteName.value) return;
  const name = editedRemoteName.value;
  dialog.warning({
    title: t("workspace.controller.removeRemote"),
    content: t("workspace.controller.removeRemoteRemoteItsAssociatedAccountBindingWillAlsoBeRemoved", { remote: name }),
    positiveText: t("workspace.controller.remove"),
    negativeText: t("workspace.controller.cancel"),
    onPositiveClick: async () => {
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
        message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
        return;
      }
      const releaseOperation = acquireRepositoryOperation(repositoryPath);
      if (!releaseOperation) {
        message.warning(t("workspace.controller.aRepositoryOperationIsAlreadyInProgressWaitForItToFinish"));
        return;
      }
      try {
        await api.removeRemote(repositoryPath, name);
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
        remoteModalShown.value = false;
        remoteEditorRepositoryPath.value = null;
        remoteEditorSelectionGeneration.value = null;
        await refreshSnapshot();
        if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
        message.success(t("workspace.controller.remoteRemoved"));
      } catch (error) {
        if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
          reportError(error, t("workspace.controller.couldNotRemoveRemote"), {
            repositoryPath,
            selectionGeneration,
          });
        }
      } finally {
        releaseOperation();
      }
    },
  });
}

async function runNetworkAction(
  action: NetworkAction,
  forceWithLease = false,
  capturedTarget?: NetworkActionTarget,
): Promise<void> {
  const repositoryPath = capturedTarget?.repositoryPath ?? activeRepositoryPath.value;
  const currentRemote = activeRemote.value;
  const remoteName = capturedTarget?.remoteName ?? currentRemote?.name;
  const remoteIdentity = capturedTarget?.remoteIdentity ?? remoteProfileIdentity(currentRemote);
  const selectionGeneration = capturedTarget?.selectionGeneration ?? repositorySelectionGeneration;
  if (!repositoryPath || !remoteName || !remoteIdentity) {
    message.warning(t("workspace.controller.selectARemoteFirst"));
    return;
  }
  const activeRemoteMatchesTarget = () =>
    activeRemote.value?.name === remoteName
    && remoteProfileIdentity(activeRemote.value) === remoteIdentity;
  if (
    !repositorySelectionIsCurrent(repositoryPath, selectionGeneration) ||
    (capturedTarget && !activeRemoteMatchesTarget())
  ) {
    message.warning(t("workspace.controller.theRepositoryOrRemoteChangedStartTheOperationAgain"));
    return;
  }
  const target: NetworkActionTarget = {
    repositoryPath,
    remoteName,
    remoteIdentity,
    selectionGeneration,
  };
  const releaseOperation = acquireRepositoryOperation(repositoryPath);
  if (!releaseOperation) {
    message.warning(t("workspace.controller.aRepositoryOperationIsAlreadyInProgressWaitForItToFinish"));
    return;
  }
  networkBusy.value = action;
  const operationGeneration = ++networkOperationGeneration;
  try {
    await (action === "fetch"
      ? api.fetchRemote(repositoryPath, remoteName)
      : action === "pull"
        ? api.pullRemote(repositoryPath, remoteName)
        : api.pushRemote(repositoryPath, remoteName, forceWithLease));
    let actionLabel: string;
    if (action === "fetch") {
      actionLabel = t("workspace.controller.fetch");
    } else if (action === "pull") {
      actionLabel = t("workspace.controller.pull");
    } else {
      actionLabel = forceWithLease
        ? t("workspace.controller.forcePushWithLease")
        : t("workspace.controller.push");
    }
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && activeRemoteMatchesTarget()) {
      message.success(t("workspace.controller.actionCompleted", { action: actionLabel }));
      await refreshSnapshot();
    }
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && activeRemoteMatchesTarget()) {
      let failureAction: string;
      if (action === "fetch") {
        failureAction = t("workspace.controller.fetchFailed");
      } else if (action === "pull") {
        failureAction = t("workspace.controller.pullFailed");
      } else {
        failureAction = t("workspace.controller.pushFailed");
      }
      const canOfferFork = action === "push"
        && isAppError(error)
        && error.code === "git_push_permission_denied"
        && activeCapabilities.value.repositoryFork
        && activeRemoteMatchesTarget();
      if (
        operationGeneration === networkOperationGeneration
        && isAppError(error)
        && error.code === "unsafe_git_config"
      ) {
        reportUnsafeNetworkConfiguration(
          error,
          failureAction,
          action,
          forceWithLease,
          target,
          operationGeneration,
        );
      } else if (!canOfferFork) {
        reportError(error, failureAction, { repositoryPath, selectionGeneration });
      } else {
        const deniedBranch = snapshot.value?.status.branch;
        let originalPushErrorReported = false;
        const reportOriginalPushError = () => {
          if (
            originalPushErrorReported
            || !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
            || !activeRemoteMatchesTarget()
          ) return;
          originalPushErrorReported = true;
          reportError(error, failureAction, { repositoryPath, selectionGeneration });
        };
        dialog.warning({
          title: t("workspace.controller.pushPermissionDenied"),
          content: t("workspace.controller.pushPermissionDeniedOfferFork", { remote: remoteName }),
          positiveText: t("workspace.controller.forkAndPush"),
          negativeText: t("workspace.controller.cancel"),
          onPositiveClick: () => {
            if (
              !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
              || !activeRemoteMatchesTarget()
              || !activeCapabilities.value.repositoryFork
              || !deniedBranch
              || snapshot.value?.status.branch !== deniedBranch
            ) {
              message.warning(t("workspace.controller.theRepositoryOrRemoteChangedStartTheOperationAgain"));
              return;
            }
            openForkModal();
          },
          onNegativeClick: reportOriginalPushError,
          onClose: reportOriginalPushError,
        });
      }
    }
  } finally {
    releaseOperation();
    if (operationGeneration === networkOperationGeneration) {
      networkBusy.value = null;
    }
  }
}

function confirmForcePush(): void {
  const remote = activeRemote.value;
  const remoteIdentity = remoteProfileIdentity(remote);
  if (!activeRepositoryPath.value || !remote || !remoteIdentity) return;
  const target: NetworkActionTarget = {
    repositoryPath: activeRepositoryPath.value,
    remoteName: remote.name,
    remoteIdentity,
    selectionGeneration: repositorySelectionGeneration,
  };
  dialog.error({
    title: t("workspace.controller.forcePushWithLease"),
    content: t("workspace.controller.forcePushToRemoteWithLeaseItRewritesRemoteBranchHistoryButRefusesToOverwriteABranchChangedBySome",
      { remote: target.remoteName },
    ),
    positiveText: t("workspace.controller.forcePush"),
    negativeText: t("workspace.controller.cancel"),
    onPositiveClick: () => void runNetworkAction("push", true, target),
  });
}

function openBindingDrawer(remote: RemoteProfile): void {
  bindingRepositoryPath.value = activeRepositoryPath.value;
  bindingSelectionGeneration.value = repositorySelectionGeneration;
  bindingRemote.value = remote;
  const binding = getBinding(remote.name);
  bindingForm.endpointId = binding?.endpointId || remote.endpointId || "gitee.com";
  const selection = binding?.accountSelection;
  const boundAccount = selection?.mode === "profile"
    ? accounts.value.find((account) => account.id === selection.accountProfileId)
    : undefined;
  bindingForm.accountId = boundAccount && supportsGitRemoteCredentials(boundAccount) ? boundAccount.id : "auto";
  bindingDrawerShown.value = true;
}

async function saveBinding(): Promise<void> {
  const repositoryPath = bindingRepositoryPath.value;
  const selectionGeneration = bindingSelectionGeneration.value;
  const remote = bindingRemote.value;
  if (
    !repositoryPath
    || selectionGeneration === null
    || !remote
    || !bindingForm.endpointId
    || !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
  ) {
    message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
    return;
  }
  const endpointId = bindingForm.endpointId;
  const accountId = bindingForm.accountId;
  const selectedAccount = accounts.value.find((account) => account.id === accountId);
  if (accountId !== "auto" && (!selectedAccount || selectedAccount.endpointId !== endpointId)) {
    message.warning(t("workspace.controller.selectAnAccountThatBelongsToThisGitServiceEndpoint"));
    return;
  }
  if (selectedAccount && !supportsGitRemoteCredentials(selectedAccount)) {
    message.warning(t("workspace.controller.anAccountBoundRemoteSupportsOnlyAnAppManagedPatSshAgentAndTheSystemGitCredentialHelperWorkAutoma"));
    return;
  }
  const releaseOperation = acquireRepositoryOperation(repositoryPath);
  if (!releaseOperation) {
    message.warning(t("workspace.controller.aRepositoryOperationIsAlreadyInProgressWaitForItToFinish"));
    return;
  }
  const binding: RemoteBinding = {
    repositoryPath,
    remoteName: remote.name,
    endpointId,
    accountSelection: accountId === "auto" ? { mode: "auto" } : { mode: "profile", accountProfileId: accountId },
  };
  try {
    await api.saveRemoteBinding(binding);
    if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
    bindingDrawerShown.value = false;
    bindingRepositoryPath.value = null;
    bindingSelectionGeneration.value = null;
    await refreshSnapshot();
    if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) return;
    message.success(t("workspace.controller.remoteAccountBindingSavedFutureNetworkOperationsWillPreferIt"));
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      reportError(error, t("workspace.controller.couldNotSaveBinding"), {
        repositoryPath,
        selectionGeneration,
      });
    }
  } finally {
    releaseOperation();
  }
}

function updateSection(value: string | number): void {
  section.value = value === "accounts" ? "accounts" : "repositories";
  if (section.value === "accounts") {
    void loadAccountsThenRefreshStatuses();
  } else if (automaticRefreshDirty) {
    scheduleAutomaticRefresh(0);
  }
}

function activateRepositoryTabOnPointerDown(tab: RepositoryTab, event: PointerEvent): void {
  if (event.button !== 0 || !event.isPrimary) return;
  const tabElement = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
  queueMicrotask(() => {
    if (
      controllerMounted &&
      !event.defaultPrevented &&
      tabElement?.isConnected &&
      tabElement.dataset.name === tab
    ) activeTab.value = tab;
  });
}

function scheduleRepositoryTabLoad(tab: RepositoryTab): void {
  const loadGeneration = ++repositoryTabLoadGeneration;
  const selectionGeneration = repositorySelectionGeneration;
  const revealAndLoad = async () => {
    if (
      !controllerMounted ||
      loadGeneration !== repositoryTabLoadGeneration ||
      selectionGeneration !== repositorySelectionGeneration ||
      activeTab.value !== tab
    ) return;
    repositoryTabContentReady[tab] = true;
    await nextTick();
    if (
      !controllerMounted ||
      loadGeneration !== repositoryTabLoadGeneration ||
      selectionGeneration !== repositorySelectionGeneration ||
      activeTab.value !== tab
    ) return;
    if (tab === "history") void loadHistoryWithOptions();
    else if (tab === "branches") {
      void Promise.all([loadWorktrees(), loadSubmodules(), loadLfsStatus()]);
    }
  };

  // First present the selected tab with its lightweight shell. Mount the full
  // panel and start repository I/O only after that frame is visible, so neither
  // initial component work nor Tauri command dispatch can delay click feedback.
  if (typeof requestAnimationFrame !== "function") {
    setTimeout(() => void revealAndLoad(), 0);
    return;
  }
  requestAnimationFrame(() => requestAnimationFrame(() => void revealAndLoad()));
}

watch(appLocale, () => {
  void refreshBrowserDemoLocalizedData().catch(() => undefined);
});

watch(selectedRemoteName, () => {
  networkOperationGeneration += 1;
  clearActiveErrorNotification();
  invalidatePlatformRequests();
  repositoryInfo.value = null;
  pullRequests.value = [];
  void loadCapabilities();
});

watch(
  () => activeRemote.value ? remoteProfileIdentity(activeRemote.value) : null,
  (identity, previousIdentity) => {
    if (identity === previousIdentity) return;
    networkOperationGeneration += 1;
    clearActiveErrorNotification();
  },
);

watch(activeTab, scheduleRepositoryTabLoad, { flush: "post" });

watch(
  () => bindingForm.endpointId,
  () => {
    if (bindingForm.accountId !== "auto" && !bindingAccounts.value.some((account) => account.id === bindingForm.accountId)) {
      bindingForm.accountId = "auto";
    }
  },
);

watch(
  () => cloneForm.endpointId,
  () => {
    if (!cloneForm.endpointId) {
      cloneForm.accountId = "none";
    } else if (cloneForm.accountId !== "none" && cloneForm.accountId !== "auto" && !cloneAccounts.value.some((account) => account.id === cloneForm.accountId)) {
      cloneForm.accountId = cloneForm.endpointId ? "auto" : "none";
    }
  },
);

watch(
  () => publishForm.endpointId,
  () => {
    invalidatePublishOwnerRequests();
    publishOwners.value = [];
    publishOwnersError.value = "";
    if (!publishAccounts.value.some((account) => account.id === publishForm.accountId)) {
      publishForm.accountId = publishAccounts.value[0]?.id || "";
    }
  },
);

watch(
  () => [publishModalShown.value, publishForm.endpointId, publishForm.accountId] as const,
  ([shown]) => {
    if (shown) void loadPublishOwners();
    else {
      invalidatePublishOwnerRequests();
    }
  },
);

watch(
  () => accountForm.authMethod,
  () => {
    if (!accountUsesPat.value) accountForm.token = "";
  },
);

watch(repositoryGrouping, (grouping) => {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(repositoryGroupingStorageKey, grouping);
  } catch {
    // Keep the choice for this session if browser storage is unavailable.
  }
  if (grouping === "owner") void loadRepositories(true);
});

onMounted(async () => {
  markPerformance("workspace:mounted");
  controllerMounted = true;
  if (!browserDemo) window.addEventListener("keydown", handleDevtoolsShortcut);
  window.addEventListener("keydown", handleRepositoryViewportKeydown);
  window.addEventListener("pointerdown", markRepositoryViewportInteraction, { passive: true });
  window.addEventListener("wheel", markRepositoryViewportInteraction, { passive: true });
  window.addEventListener("focus", handleWindowFocus);
  window.addEventListener("blur", handleWindowBlur);
  document.addEventListener("visibilitychange", handleVisibilityChange);
  markPerformance("workspace:startup-data:start");
  const startupDataPromise = Promise.all([
    loadAccounts(),
    loadEndpoints(),
    loadExternalEditor(),
    loadRepositoryEditors(),
  ])
    .then((result) => {
      markPerformance("workspace:startup-data:end");
      measurePerformance("Startup settings requests", "workspace:startup-data:start", "workspace:startup-data:end");
      return result;
    });
  const storedPath = storedLastRepositoryPath();
  const storedRepositoryPromise = storedPath
    ? (async () => {
        markPerformance("workspace:stored-repository:start");
        await selectRepository(storedPath, false, true);
        markPerformance("workspace:stored-repository:end");
        measurePerformance("Stored repository open", "workspace:stored-repository:start", "workspace:stored-repository:end");
      })()
    : Promise.resolve();
  if (!browserDemo) {
    const [repositoryListener, watchErrorListener, focusListener] = await Promise.allSettled([
      listen<RepositoryChangedPayload>("repository-changed", handleRepositoryChanged),
      listen<RepositoryChangedPayload>("repository-watch-error", handleRepositoryWatchError),
      getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (focused) handleWindowFocus();
        else handleWindowBlur();
      }),
    ]);
    if (!controllerMounted) {
      if (repositoryListener.status === "fulfilled") repositoryListener.value();
      if (watchErrorListener.status === "fulfilled") watchErrorListener.value();
      if (focusListener.status === "fulfilled") focusListener.value();
      return;
    }
    // Repository data must still load when a platform cannot expose one of
    // the optional native focus or file-change event streams.
    if (repositoryListener.status === "fulfilled") stopRepositoryChangeListener = repositoryListener.value;
    if (watchErrorListener.status === "fulfilled") stopRepositoryWatchErrorListener = watchErrorListener.value;
    if (focusListener.status === "fulfilled") stopWindowFocusListener = focusListener.value;
  }
  markPerformance("workspace:listeners-ready");
  measurePerformance("Workspace listener setup", "workspace:mounted", "workspace:listeners-ready");
  if (!controllerMounted) return;
  // Watcher installation is deferred until the first interactive frame, so
  // opening the remembered repository does not depend on event listeners.
  // Starting it alongside the bridge/listener warm-up removes that fixed
  // startup round trip from the repository critical path.
  await storedRepositoryPromise;
  if (!controllerMounted) return;
  if (snapshot.value) workspaceInitializing.value = false;

  let repositoriesLoadedDuringStartup = false;
  if (!snapshot.value) {
    await loadRepositories();
    repositoriesLoadedDuringStartup = true;
  }
  await startupDataPromise;
  if (!controllerMounted) return;
  const storedRepository = storedPath
    ? repositories.value.find((repository) => repository.path === storedPath)
    : undefined;
  const mostRecentRepository = [...repositories.value].sort(
    (left, right) => (right.lastOpenedAt ?? 0) - (left.lastOpenedAt ?? 0),
  )[0];
  const startupRepository = storedRepository ?? mostRecentRepository;
  if (!snapshot.value && startupRepository) {
    markPerformance("workspace:fallback-repository:start");
    await selectRepository(startupRepository.path, true, true);
    markPerformance("workspace:fallback-repository:end");
    measurePerformance("Fallback repository open", "workspace:fallback-repository:start", "workspace:fallback-repository:end");
  }
  if (controllerMounted) {
    workspaceInitializing.value = false;
    completeStartupPerformance(() => {
      if (!controllerMounted) return;
      const repositoryPath = activeRepositoryPath.value;
      const loadDeferredRepositories = () => {
        if (controllerMounted && !repositoriesLoadedDuringStartup) void loadRepositories();
      };
      if (repositoryPath && activeRepositoryPath.value === repositoryPath) {
        void installRepositoryWatcher(repositoryPath);
      }
      loadDeferredRepositories();
    });
  }
});

onBeforeUnmount(() => {
  controllerMounted = false;
  disposeRepositoryDiscovery();
  networkOperationGeneration += 1;
  clearActiveErrorNotification();
  clearActiveGitLockRecoveryDialog();
  repositoryTabLoadGeneration += 1;
  disposeRepositoryHistory();
  const stopWatcherGeneration = ++repositoryWatchRequestGeneration;
  repositoryWatchSupersededGenerations.clear();
  void api.stopRepositoryWatcher(stopWatcherGeneration).catch(() => undefined);
  if (automaticRefreshTimer) clearTimeout(automaticRefreshTimer);
  if (repositoryWatcherRetryTimer) clearTimeout(repositoryWatcherRetryTimer);
  automaticRefreshTimer = null;
  repositoryWatcherRetryTimer = null;
  stopRepositoryChangeListener?.();
  stopRepositoryWatchErrorListener?.();
  stopWindowFocusListener?.();
  stopRepositoryChangeListener = null;
  stopRepositoryWatchErrorListener = null;
  stopWindowFocusListener = null;
  window.removeEventListener("keydown", handleDevtoolsShortcut);
  window.removeEventListener("keydown", handleRepositoryViewportKeydown);
  window.removeEventListener("pointerdown", markRepositoryViewportInteraction);
  window.removeEventListener("wheel", markRepositoryViewportInteraction);
  window.removeEventListener("focus", handleWindowFocus);
  window.removeEventListener("blur", handleWindowBlur);
  document.removeEventListener("visibilitychange", handleVisibilityChange);
});

  return {
    message,
    dialog,
    browserDemo,
    repositoryGroupingStorageKey,
    initialRepositoryGrouping,
    emptyCapabilities,
    localeOptions,
    colorModeOptions,
    repositoryActionOptions,
    section,
    activeTab,
    activateRepositoryTabOnPointerDown,
    repositoryTabContentReady,
    repositories,
    repositoryGrouping,
    activeRepositoryPath,
    snapshot,
    accounts,
    selectedRemoteName,
    activeCapabilities,
    repositoryInfo,
    repositoryWebUrl,
    pullRequests,
    selectedChange,
    diffText,
    selectedDiffStaged,
    blameText,
    commits,
    stashes,
    reflogText,
    worktrees,
    submodules,
    lfsStatus,
    externalEditor,
    repositoryEditors,
    repositoryEditorsLoaded,
    selectedCommit,
    commitChangedFiles,
    selectedCommitFile,
    commitDiffText,
    commitDiffVisibleLines,
    commitDiffHasMoreLines,
    commitDiffCanExpand,
    commitDiffLimitedAtMaximum,
    commitDiffLineLimitReached,
    workspaceInitializing,
    repositoriesLoading,
    snapshotLoading,
    accountsLoading,
    accountStatusRefreshing,
    diffLoading,
    blameLoading,
    platformLoading,
    pullRequestsLoading,
    historyLoading,
    reflogLoading,
    commitFilesLoading,
    commitDiffLoading,
    stashesLoading,
    worktreesLoading,
    submodulesLoading,
    lfsLoading,
    networkBusy,
    mutationBusy,
    publishOwnersLoading,
    cloneModalShown,
    initModalShown,
    discoverModalShown,
    discoveryRootPath,
    repositoryImportLimit,
    discoveredRepositories,
    selectedDiscoveredRepositoryPaths,
    discoveryScanning,
    discoveryAdding,
    discoveryBusy,
    selectableDiscoveredRepositoryCount,
    selectedDiscoveredRepositoryCount,
    allDiscoveredRepositoriesSelected,
    someDiscoveredRepositoriesSelected,
    accountModalShown,
    customEndpointModalShown,
    branchModalShown,
    renameBranchModalShown,
    stashModalShown,
    rebaseModalShown,
    cherryPickModalShown,
    worktreeModalShown,
    lfsTrackModalShown,
    externalEditorModalShown,
    repositoryEditorsModalShown,
    installedApplicationPickerShown,
    blameModalShown,
    reflogModalShown,
    pullRequestModalShown,
    forkModalShown,
    squashModalShown,
    publishModalShown,
    tagModalShown,
    remoteModalShown,
    bindingDrawerShown,
    bindingRemote,
    repositoryGroupModalShown,
    repositoryAliasModalShown,
    settingsModalShown,
    repositoryGroupTarget,
    repositoryAliasTarget,
    cloneForm,
    initForm,
    accountForm,
    endpointForm,
    branchForm,
    renameBranchForm,
    stashForm,
    rebaseForm,
    cherryPickForm,
    worktreeForm,
    lfsTrackForm,
    externalEditorForm,
    repositoryEditorForm,
    repositoryEditorEditingIndex,
    repositoryEditorPickerBusy,
    installedApplications,
    installedApplicationOptions,
    installedApplicationSelection,
    pullRequestForm,
    pullRequestHead,
    forkForm,
    squashForm,
    publishForm,
    tagForm,
    remoteForm,
    editedRemoteName,
    bindingForm,
    repositoryGroupForm,
    repositoryAliasForm,
    endpoints,
    lastAccountStatusRefreshAt,
    accountStatusRefreshCacheMs,
    activeRemote,
    canPullFromActiveRemote,
    currentBranch,
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
    diffHunkAtLine,
    diffLines,
    hasDiffText,
    canAmendLatestCommit,
    isUnbornRepository,
    commitActionLabel,
    commitSelectionSummary,
    rebaseBaseOptions,
    worktreeExistingBranchOptions,
    pullRequestBaseOptions,
    publishEndpoints,
    publishAccounts,
    publishAccountOptions,
    forkAccounts,
    forkAccountOptions,
    forkContributionBranchRecommended,
    publishOwnerOptions,
    canPublishToSelectedOwner,
    publishOwnersError,
    accountFormCapabilities,
    accountUsesPat,
    accountMethodOptions,
    supportsGitRemoteCredentials,
    bindingAccounts,
    bindingAccountOptions,
    bindingEndpointOptions,
    cloneAccounts,
    cloneAccountOptions,
    tagTargetOptions,
    tagNameOptions,
    tagNameExists,
    repositoryGroupingOptions,
    repositoryGroups,
    branchColumns,
    remoteBranchColumns,
    worktreeColumns,
    submoduleColumns,
    remoteColumns,
    accountColumns,
    pullRequestColumns,
    stashColumns,
    endpointHasExplicitPort,
    capabilityForEndpoint,
    endpointLabel,
    authenticationTagType,
    changeStatusLabel,
    changeStatusTagType,
    isChangeFullyIncluded,
    isChangePartiallyIncluded,
    changeIncludeLabel,
    updateChangeInclusion,
    updateAllChangeInclusion,
    changeKindLabel,
    isConflictedEntry,
    splitDiffHunks,
    applyDiffHunkAtLine,
    splitDiffLines,
    diffLineClass,
    gitOperationLabel,
    submoduleStatusLabel,
    submoduleStatusTagType,
    getBinding,
    remoteBindingLabel,
    remoteAuthenticationLabel,
    reportError,
    updateAppLocale,
    updateColorMode,
    updateThemeColor,
    resetThemeColor,
    openRepositoryAction,
    openRepositoryDiscovery,
    closeRepositoryDiscovery,
    updateRepositoryDiscoveryShown,
    discoveredRepositoryAlreadyAdded,
    discoveredRepositorySelected,
    discoveredRepositorySelectionDisabled,
    toggleDiscoveredRepositorySelection,
    updateDiscoveredRepositorySelection,
    updateAllDiscoveredRepositorySelections,
    handleDevtoolsShortcut,
    applySnapshot,
    clearDiffSelection,
    clearCommitSelection,
    syncSelectedDiff,
    loadCapabilities,
    compareRepositoryRecords,
    replaceRepositoryRecord,
    openRepositoryGroupEditor,
    saveRepositoryGroup,
    openRepositoryAliasEditor,
    saveRepositoryAlias,
    loadRepositories,
    loadAccounts,
    refreshAccountStatuses,
    loadAccountsThenRefreshStatuses,
    loadEndpoints,
    selectRepository,
    refreshSnapshot,
    addSelectedDiscoveredRepositories,
    cloneRepository,
    initRepository,
    loadDiff,
    openFileBlame,
    applyDiffHunk,
    loadHistory,
    openReflog,
    loadWorktrees,
    loadSubmodules,
    loadLfsStatus,
    loadExternalEditor,
    loadRepositoryEditors,
    loadCommitFiles,
    loadCommitFileDiff,
    showMoreCommitDiffLines,
    setChangeIncluded,
    stageAll,
    unstageAll,
    confirmDiscardChanges,
    discardSelectedChange,
    confirmCleanUntracked,
    refreshAfterGitConflict,
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
    commitMessage,
    amendLastCommit,
    setAmendLastCommit,
    commitChanges,
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
    openWorktreeModal,
    addWorktree,
    confirmRemoveWorktree,
    confirmUpdateSubmodules,
    initializeLfs,
    openLfsTrackModal,
    trackLfsPattern,
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
    openRemoteEditor,
    saveRemote,
    confirmRemoveRemote,
    runNetworkAction,
    confirmForcePush,
    openBindingDrawer,
    saveBinding,
    saveAccount,
    openCustomEndpointModal,
    saveCustomEndpoint,
    importCliAccounts,
    detectCliAccounts,
    setDefaultCandidate,
    confirmRemoveAccount,
    loadRepositoryInfo,
    openCodeHostUrl,
    loadPullRequests,
    openCreatePullRequest,
    createPullRequest,
    openForkModal,
    forkGithubRepository,
    openPublishModal,
    loadPublishOwners,
    publishGitHubRepository,
    updateSection,
    api,
    builtinEndpoints,
    isAppError,
    isBrowserDemo,
    toggleDevtools,
    appLocale,
    localizedErrorMessage,
    setAppLocale,
    t,
    colorMode,
    setColorMode,
    themeColor,
    authenticationMethodLabel,
    authenticationStatusLabel,
    endpointKindLabel,
  };
}

export type WorkspaceController = ReturnType<typeof useWorkspaceController>;
