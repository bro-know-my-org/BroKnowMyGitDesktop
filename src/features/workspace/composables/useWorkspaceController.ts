import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useDialog, useMessage } from "naive-ui";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, builtinEndpoints, isAppError, isBrowserDemo, toggleDevtools } from "../../../shared/api/tauri";
import { appLocale, authenticationMethodLabel, authenticationStatusLabel, endpointKindLabel, localizedErrorDetails, localizedErrorMessage, setAppLocale, t } from "../../../shared/i18n";
import { completeStartupPerformance, markPerformance, measurePerformance } from "../../../shared/performance";
import { colorMode, resetThemeColor, setColorMode, themeColor } from "../../../shared/theme";
import {
  type AppError,
  type CodeHostCapabilities,
  type RemoteBinding,
  type RemoteProfile,
  type RepositorySnapshot,
} from "../../../shared/types/domain";
import { createRepositoryPresentation } from "../../repository/utils/presentation";
import { useRepositoryCatalog } from "./useRepositoryCatalog";
import { useWorkspaceAccounts } from "./useWorkspaceAccounts";
import { useRepositoryPlatform } from "./useRepositoryPlatform";
import { useRepositoryAuxiliary } from "./useRepositoryAuxiliary";
import { useRepositoryHistory } from "./useRepositoryHistory";
import { useRepositoryOperations } from "./useRepositoryOperations";
import { useRepositoryChanges } from "./useRepositoryChanges";
import { handleDevtoolsShortcut, localeOptions, updateAppLocale, updateColorMode, updateThemeColor, useWorkspaceUiOptions } from "./workspaceUiActions";
import { useWorkspaceTables } from "./useWorkspaceTables";
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
const lastError = ref<AppError | null>(null);
const lastErrorAction = ref("");

const workspaceInitializing = ref(true);
const snapshotLoading = ref(false);
let repositoryTabLoadGeneration = 0;
const networkBusy = ref<NetworkAction | null>(null);
let networkOperationGeneration = 0;
const networkOperationsInFlight = new Set<string>();
const mutationBusy = ref(false);
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
} = useRepositoryAuxiliary({
  activeRepositoryPath,
  snapshot,
  mutationBusy,
  getSelectionGeneration: () => repositorySelectionGeneration,
  repositorySelectionIsCurrent,
  applySnapshot,
  loadRepositories,
  reportError,
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
  openReflog,
  loadCommitFiles,
  loadCommitFileDiff,
} = useRepositoryHistory({
  activeRepositoryPath,
  snapshot,
  getSelectionGeneration: () => repositorySelectionGeneration,
  repositorySelectionIsCurrent,
  isControllerMounted: () => controllerMounted,
  reportError,
});

const cloneModalShown = ref(false);
const initModalShown = ref(false);
const discoverModalShown = ref(false);
const remoteModalShown = ref(false);
const bindingDrawerShown = ref(false);
const bindingRemote = ref<RemoteProfile | null>(null);
const settingsModalShown = ref(false);

const cloneForm = reactive({ url: "", destinationPath: "", endpointId: "", accountId: "none" });
const initForm = reactive({ destinationPath: "", initialBranch: "main" });
const discoverForm = reactive({ rootPath: "" });
const remoteForm = reactive({ name: "", fetchUrl: "", pushUrl: "" });
const editedRemoteName = ref<string | null>(null);
const bindingForm = reactive({ endpointId: "", accountId: "auto" });
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
  pullRequests,
  platformLoading,
  pullRequestsLoading,
  publishOwnersLoading,
  pullRequestModalShown,
  publishModalShown,
  pullRequestForm,
  publishForm,
  publishOwners,
  publishOwnersError,
  pullRequestBaseOptions,
  publishEndpoints,
  publishAccounts,
  publishAccountOptions,
  publishOwnerOptions,
  canPublishToSelectedOwner,
  loadRepositoryInfo,
  loadPullRequests,
  openCreatePullRequest,
  createPullRequest,
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
  mutationBusy,
  capabilityForEndpoint,
  applySnapshot,
  loadCapabilities,
  reportError,
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
  mutationBusy,
  getSelectionGeneration: () => repositorySelectionGeneration,
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
  mutationBusy,
  getSelectionGeneration: () => repositorySelectionGeneration,
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
    return { repositoryInfo: !portScopedCli, repositoryPublish: !portScopedCli, pullRequestsRead: !portScopedCli, pullRequestsWrite: !portScopedCli, accountImport: !portScopedCli, accountBindings: true, patKeychain: true };
  }
  if (endpoint.kind === "gitLab") {
    return { repositoryInfo: false, repositoryPublish: false, pullRequestsRead: false, pullRequestsWrite: false, accountImport: !portScopedCli, accountBindings: true, patKeychain: true };
  }
  if (endpoint.kind === "gitee") {
    return { repositoryInfo: false, repositoryPublish: false, pullRequestsRead: false, pullRequestsWrite: false, accountImport: false, accountBindings: true, patKeychain: true };
  }
  if (endpoint.kind === "gitea" || endpoint.kind === "forgejo" || endpoint.kind === "genericGit") {
    return { repositoryInfo: false, repositoryPublish: false, pullRequestsRead: false, pullRequestsWrite: false, accountImport: false, accountBindings: true, patKeychain: true };
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

function reportError(error: unknown, action: string): void {
  const appError: AppError = isAppError(error)
    ? error
    : { code: "unexpected_error", message: t("workspace.controller.anUnexpectedErrorOccurred"), details: null };
  appError.message = localizedErrorMessage(appError.code);
  appError.details = localizedErrorDetails(appError.code) || null;
  lastError.value = appError;
  lastErrorAction.value = action;
  message.error(t("workspace.controller.actionMessage", { action, message: appError.message }));
  if (appError.code === "account_selection_required" && activeRemote.value) {
    message.warning(t("workspace.controller.thisRemoteHasMultipleOrNoUsableAccountsSelectOneExplicitlyAndRetry"));
    openBindingDrawer(activeRemote.value);
  }
}

function clearLastError(): void {
  lastError.value = null;
  lastErrorAction.value = "";
}

async function refreshBrowserDemoLocalizedData(): Promise<void> {
  if (!browserDemo || !controllerMounted) return;
  const refreshGeneration = ++browserDemoLocaleRefreshGeneration;
  const repositoryPath = activeRepositoryPath.value;
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
    && activeRepositoryPath.value === repositoryPath;
  clearLastError();

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
    if (refreshBlame && selectedPath) {
      const localizedBlame = await api.getFileBlame(repositoryPath, selectedPath);
      if (!refreshIsCurrent() || !blameModalShown.value || selectedChange.value?.path !== selectedPath) return;
      blameText.value = localizedBlame;
    }
    if (refreshReflog) {
      const localizedReflog = await api.listReflog(repositoryPath);
      if (!refreshIsCurrent() || !reflogModalShown.value) return;
      reflogText.value = localizedReflog;
    }
    if (publishModalShown.value && refreshIsCurrent()) await loadPublishOwners();
  } catch (error) {
    if (refreshGeneration === browserDemoLocaleRefreshGeneration && controllerMounted) {
      reportError(error, t("workspace.controller.couldNotRefreshLocalizedDemoData"));
    }
  }
}

function openRepositoryAction(value: string | number): void {
  if (value === "discover") discoverModalShown.value = true;
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
  if (!nextSnapshot.remotes.some((remote) => remote.name === selectedRemoteName.value)) {
    selectedRemoteName.value = nextSnapshot.remotes[0]?.name || "";
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

async function loadCapabilities(): Promise<void> {
  if (!activeRemote.value?.endpointId) {
    activeCapabilities.value = emptyCapabilities;
    return;
  }
  try {
    activeCapabilities.value = await api.getCodeHostCapabilities(activeRemote.value.endpointId);
  } catch {
    activeCapabilities.value = emptyCapabilities;
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
  networkBusy.value = null;
  resetRepositoryOperations();
  snapshot.value = null;
  repositoryInfo.value = null;
  pullRequests.value = [];
  activeRepositoryPath.value = repositoryPath;
  const selectionGeneration = ++repositorySelectionGeneration;
  section.value = "repositories";
  snapshotLoading.value = true;
  clearLastError();
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
      reportError(error, t("workspace.controller.couldNotOpenRepository"));
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
      reportError(error, t("workspace.controller.couldNotRefreshRepository"));
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
  const element = document.querySelector<HTMLElement>(".content-area > .n-layout-scroll-container");
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

async function discoverRepositories(): Promise<void> {
  if (!discoverForm.rootPath.trim()) {
    message.warning(t("workspace.controller.enterTheFolderPathToScan"));
    return;
  }
  mutationBusy.value = true;
  try {
    const found = await api.discoverRepositories(discoverForm.rootPath.trim());
    const paths = new Set(repositories.value.map((repository) => repository.path));
    repositories.value = [...repositories.value, ...found.filter((repository) => !paths.has(repository.path))];
    discoverModalShown.value = false;
    discoverForm.rootPath = "";
    message.success(found.length ? t("workspace.controller.foundCountRepositories", { count: found.length }) : t("workspace.controller.noGitRepositoriesFound"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotScanRepositories"));
  } finally {
    mutationBusy.value = false;
  }
}

async function cloneRepository(): Promise<void> {
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
    if (!repositories.value.some((item) => item.path === repository.path)) repositories.value = [...repositories.value, repository];
    cloneModalShown.value = false;
    cloneForm.url = "";
    cloneForm.destinationPath = "";
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
    if (!repositories.value.some((item) => item.path === repository.path)) repositories.value = [...repositories.value, repository];
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
  editedRemoteName.value = remote?.name ?? null;
  remoteForm.name = remote?.name ?? "";
  remoteForm.fetchUrl = remote?.fetchUrl ?? "";
  remoteForm.pushUrl = remote?.pushUrl ?? "";
  remoteModalShown.value = true;
}

async function saveRemote(): Promise<void> {
  if (!activeRepositoryPath.value || !remoteForm.name.trim() || !remoteForm.fetchUrl.trim()) {
    message.warning(t("workspace.controller.enterARemoteNameAndFetchUrl"));
    return;
  }

  mutationBusy.value = true;
  try {
    const repositoryPath = activeRepositoryPath.value;
    let currentName = editedRemoteName.value;
    if (!currentName) {
      await api.addRemote(repositoryPath, remoteForm.name.trim(), remoteForm.fetchUrl.trim());
      currentName = remoteForm.name.trim();
    } else if (currentName !== remoteForm.name.trim()) {
      await api.renameRemote(repositoryPath, currentName, remoteForm.name.trim());
      currentName = remoteForm.name.trim();
    }
    await api.setRemoteUrl(repositoryPath, currentName, remoteForm.fetchUrl.trim(), "fetch");
    if (remoteForm.pushUrl.trim()) {
      await api.setRemoteUrl(repositoryPath, currentName, remoteForm.pushUrl.trim(), "push");
    }
    remoteModalShown.value = false;
    await refreshSnapshot();
    message.success(t("workspace.controller.remoteSaved"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotSaveRemote"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmRemoveRemote(): void {
  if (!activeRepositoryPath.value || !editedRemoteName.value) return;
  const name = editedRemoteName.value;
  dialog.warning({
    title: t("workspace.controller.removeRemote"),
    content: t("workspace.controller.removeRemoteRemoteItsAssociatedAccountBindingWillAlsoBeRemoved", { remote: name }),
    positiveText: t("workspace.controller.remove"),
    negativeText: t("workspace.controller.cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        await api.removeRemote(activeRepositoryPath.value!, name);
        remoteModalShown.value = false;
        await refreshSnapshot();
        message.success(t("workspace.controller.remoteRemoved"));
      } catch (error) {
        reportError(error, t("workspace.controller.couldNotRemoveRemote"));
      } finally {
        mutationBusy.value = false;
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
  const remoteName = capturedTarget?.remoteName ?? activeRemote.value?.name;
  const selectionGeneration = capturedTarget?.selectionGeneration ?? repositorySelectionGeneration;
  if (!repositoryPath || !remoteName) {
    message.warning(t("workspace.controller.selectARemoteFirst"));
    return;
  }
  if (
    !repositorySelectionIsCurrent(repositoryPath, selectionGeneration) ||
    (capturedTarget && activeRemote.value?.name !== remoteName)
  ) {
    message.warning(t("workspace.controller.theRepositoryOrRemoteChangedStartTheOperationAgain"));
    return;
  }
  if (networkOperationsInFlight.has(repositoryPath)) {
    message.warning(t("workspace.controller.aNetworkOperationIsAlreadyInProgressWaitForItToFinish"));
    return;
  }
  networkOperationsInFlight.add(repositoryPath);
  networkBusy.value = action;
  const operationGeneration = ++networkOperationGeneration;
  clearLastError();
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
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      message.success(t("workspace.controller.actionCompleted", { action: actionLabel }));
      await refreshSnapshot();
    }
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      reportError(
        error,
        action === "fetch"
          ? t("workspace.controller.fetchFailed")
          : action === "pull"
            ? t("workspace.controller.pullFailed")
            : t("workspace.controller.pushFailed"),
      );
    }
  } finally {
    networkOperationsInFlight.delete(repositoryPath);
    if (operationGeneration === networkOperationGeneration) {
      networkBusy.value = null;
    }
  }
}

function confirmForcePush(): void {
  if (!activeRepositoryPath.value || !activeRemote.value) return;
  const target: NetworkActionTarget = {
    repositoryPath: activeRepositoryPath.value,
    remoteName: activeRemote.value.name,
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
  if (!snapshot.value || !bindingRemote.value || !bindingForm.endpointId) return;
  const selectedAccount = accounts.value.find((account) => account.id === bindingForm.accountId);
  if (bindingForm.accountId !== "auto" && (!selectedAccount || selectedAccount.endpointId !== bindingForm.endpointId)) {
    message.warning(t("workspace.controller.selectAnAccountThatBelongsToThisGitServiceEndpoint"));
    return;
  }
  if (selectedAccount && !supportsGitRemoteCredentials(selectedAccount)) {
    message.warning(t("workspace.controller.anAccountBoundRemoteSupportsOnlyAnAppManagedPatSshAgentAndTheSystemGitCredentialHelperWorkAutoma"));
    return;
  }
  const binding: RemoteBinding = {
    repositoryPath: snapshot.value.repository.path,
    remoteName: bindingRemote.value.name,
    endpointId: bindingForm.endpointId,
    accountSelection: bindingForm.accountId === "auto" ? { mode: "auto" } : { mode: "profile", accountProfileId: bindingForm.accountId },
  };
  mutationBusy.value = true;
  try {
    await api.saveRemoteBinding(binding);
    bindingDrawerShown.value = false;
    await refreshSnapshot();
    message.success(t("workspace.controller.remoteAccountBindingSavedFutureNetworkOperationsWillPreferIt"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotSaveBinding"));
  } finally {
    mutationBusy.value = false;
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
  repositoryInfo.value = null;
  pullRequests.value = [];
  void loadCapabilities();
});

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
  const startupDataPromise = Promise.all([loadAccounts(), loadEndpoints(), loadExternalEditor()])
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
    selectedCommit,
    commitChangedFiles,
    selectedCommitFile,
    commitDiffText,
    commitDiffVisibleLines,
    commitDiffHasMoreLines,
    commitDiffCanExpand,
    commitDiffLimitedAtMaximum,
    commitDiffLineLimitReached,
    lastError,
    lastErrorAction,
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
    blameModalShown,
    reflogModalShown,
    pullRequestModalShown,
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
    discoverForm,
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
    pullRequestForm,
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
    clearLastError,
    updateAppLocale,
    updateColorMode,
    updateThemeColor,
    resetThemeColor,
    openRepositoryAction,
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
    discoverRepositories,
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
    setDefaultCandidate,
    confirmRemoveAccount,
    loadRepositoryInfo,
    loadPullRequests,
    openCreatePullRequest,
    createPullRequest,
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
