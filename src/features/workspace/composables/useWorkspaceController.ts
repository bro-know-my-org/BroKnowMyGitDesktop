import { computed, h, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { NButton, useDialog, useMessage } from "naive-ui";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, builtinEndpoints, isAppError, isBrowserDemo, toggleDevtools } from "../../../shared/api/tauri";
import { appLocale, authenticationMethodLabel, authenticationStatusLabel, endpointKindLabel, localizedErrorDetails, localizedErrorMessage, setAppLocale, t, type AppLocale } from "../../../shared/i18n";
import { completeStartupPerformance, markPerformance, measurePerformance } from "../../../shared/performance";
import { colorMode, resetThemeColor, setColorMode, setThemeColor, themeColor, type ColorMode } from "../../../shared/theme";
import {
  type AccountProfile,
  type AuthenticationMethod,
  type AppError,
  type BranchInfo,
  type CommitChangedFile,
  type CommitFileDiff,
  type CommitSummary,
  type CodeHostCapabilities,
  type CodeHostEndpoint,
  type CodeHostRepositoryInfo,
  type ExternalEditorConfig,
  type GitStatusEntry,
  type GitHubPublishOwner,
  type LfsStatus,
  type PullRequestSummary,
  type RemoteBinding,
  type RemoteBranchInfo,
  type RemoteProfile,
  type RepositoryRecord,
  type RepositorySnapshot,
  type ResetMode,
  type StashEntry,
  type SubmoduleInfo,
  type WorktreeInfo,
} from "../../../shared/types/domain";
import { createRepositoryPresentation, type DiffHunk } from "../../repository/utils/presentation";
import { useWorkspaceTables } from "./useWorkspaceTables";

export function useWorkspaceController() {
type AppSection = "repositories" | "accounts";
type RepositoryTab = "overview" | "changes" | "history" | "branches";
type NetworkAction = "fetch" | "pull" | "push";
type RepositoryGrouping = "manual" | "owner" | "recent";

interface NetworkActionTarget {
  repositoryPath: string;
  remoteName: string;
  selectionGeneration: number;
}

interface RepositorySidebarGroup {
  key: string;
  label: string;
  repositories: RepositoryRecord[];
}

interface RepositoryChangedPayload {
  repositoryPath: string;
  requestGeneration: number;
  errorKind: string | null;
  retryable: boolean;
  historyObjectsChanged: boolean;
  stashesChanged: boolean;
  watchPlanChanged: boolean;
}

interface RefreshSnapshotOptions {
  includeAuxiliary?: boolean;
  reportErrors?: boolean;
  silent?: boolean;
}

interface RepositoryViewportSnapshot {
  element: HTMLElement;
  scrollTop: number;
  scrollLeft: number;
  interactionGeneration: number;
  repositoryPath: string | null;
  tab: RepositoryTab;
}

interface HistoryLoadOptions {
  force?: boolean;
  reportErrors?: boolean;
  silent?: boolean;
}

interface CachedCommitFileDiff extends CommitFileDiff {
  maximumReached: boolean;
}

interface CachedCommitFiles {
  files: CommitChangedFile[];
  weight: number;
}

interface CommitFileDiffRequest {
  repositoryPath: string;
  commit: CommitSummary;
  file: CommitChangedFile;
  selectionGeneration: number;
  requestGeneration: number;
  cacheKey: string;
  maximumBytes: number;
}

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
const repositoryGroupingStorageKey = "broknowmygitdesktop.repository-grouping";
const lastRepositoryStorageKey = "broknowmygitdesktop.last-repository";
const commitCacheLimit = 20;
const commitSelectionDebounceMs = 75;
const commitDiffInitialLineCount = 2_000;
const commitDiffLineIncrement = 2_000;
const commitDiffMaximumDisplayedLines = 10_000;
const commitDiffPreviewBytes = 512 * 1024;
const commitDiffMaximumBytes = 4 * 1024 * 1024;
const commitFilesCacheMaximumWeight = 16 * 1024 * 1024;
const commitDiffCacheMaximumWeight = 16 * 1024 * 1024;
const repositorySnapshotSignatures = new WeakMap<RepositorySnapshot, string>();

function repositorySnapshotSignature(repositorySnapshot: RepositorySnapshot): string {
  const cached = repositorySnapshotSignatures.get(repositorySnapshot);
  if (cached !== undefined) return cached;
  const signature = JSON.stringify(repositorySnapshot);
  repositorySnapshotSignatures.set(repositorySnapshot, signature);
  return signature;
}

function initialRepositoryGrouping(): RepositoryGrouping {
  if (typeof window === "undefined") return "manual";
  try {
    const saved = window.localStorage.getItem(repositoryGroupingStorageKey);
    if (saved === "manual" || saved === "owner" || saved === "recent") return saved;
  } catch {
    // Disabled browser storage should not prevent the repository list from loading.
  }
  return "manual";
}

function storedLastRepositoryPath(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(lastRepositoryStorageKey);
  } catch {
    return null;
  }
}

function rememberLastRepositoryPath(repositoryPath: string): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(lastRepositoryStorageKey, repositoryPath);
  } catch {
    // The repository remains selected for this session if storage is unavailable.
  }
}

const emptyCapabilities: CodeHostCapabilities = {
  repositoryInfo: false,
  repositoryPublish: false,
  pullRequestsRead: false,
  pullRequestsWrite: false,
  accountImport: false,
  accountBindings: false,
  patKeychain: false,
};

const localeOptions = [
  { label: "简体中文", value: "zh-CN" },
  { label: "English", value: "en-US" },
];
const colorModeOptions = computed(() => [
  { label: t("workspace.controller.followSystem"), key: "system" },
  { label: t("workspace.controller.light"), key: "light" },
  { label: t("workspace.controller.dark"), key: "dark" },
]);
const repositoryActionOptions = computed(() => [
  { label: t("workspace.controller.scanFolder"), key: "discover" },
  { label: t("workspace.controller.newLocalRepository"), key: "init" },
  { label: t("workspace.controller.cloneRepository"), key: "clone" },
]);

const section = ref<AppSection>("repositories");
const activeTab = ref<RepositoryTab>("changes");
const repositoryTabContentReady = reactive<Record<RepositoryTab, boolean>>({
  changes: true,
  history: false,
  branches: false,
  overview: false,
});
const repositories = ref<RepositoryRecord[]>([]);
const repositoryGrouping = ref<RepositoryGrouping>(initialRepositoryGrouping());
const activeRepositoryPath = ref<string | null>(null);
let repositorySelectionGeneration = 0;
let repositoryWatchRequestGeneration = Date.now();
let snapshotApplicationGeneration = 0;
let snapshotRefreshRequestGeneration = 0;
const snapshot = ref<RepositorySnapshot | null>(null);
const accounts = ref<AccountProfile[]>([]);
const selectedRemoteName = ref("");
const activeCapabilities = ref<CodeHostCapabilities>(emptyCapabilities);
const repositoryInfo = ref<CodeHostRepositoryInfo | null>(null);
const pullRequests = ref<PullRequestSummary[]>([]);
const selectedChange = ref<GitStatusEntry | null>(null);
const diffText = ref("");
const selectedDiffStaged = ref(false);
let diffRequestGeneration = 0;
let worktreesRequestGeneration = 0;
let submodulesRequestGeneration = 0;
let lfsRequestGeneration = 0;
let commitFilesRequestGeneration = 0;
let commitDiffRequestGeneration = 0;
let commitFilesDebounceTimer: ReturnType<typeof setTimeout> | null = null;
let commitDiffDebounceTimer: ReturnType<typeof setTimeout> | null = null;
let commitDiffRequestInFlight = false;
let pendingCommitDiffRequest: CommitFileDiffRequest | null = null;
const commitFilesCache = new Map<string, CachedCommitFiles>();
let commitFilesCacheWeight = 0;
const commitDiffCache = new Map<string, CachedCommitFileDiff>();
let commitDiffCacheWeight = 0;
const blameText = ref("");
const commits = ref<CommitSummary[]>([]);
const stashes = ref<StashEntry[]>([]);
const reflogText = ref("");
const worktrees = ref<WorktreeInfo[]>([]);
const submodules = ref<SubmoduleInfo[]>([]);
const lfsStatus = ref<LfsStatus | null>(null);
const externalEditor = ref<ExternalEditorConfig | null>(null);
const selectedCommit = ref<CommitSummary | null>(null);
const commitChangedFiles = ref<CommitChangedFile[]>([]);
const selectedCommitFile = ref<CommitChangedFile | null>(null);
const commitDiffText = ref("");
const commitDiffTruncated = ref(false);
const commitDiffMaximumReached = ref(false);
const lastError = ref<AppError | null>(null);
const lastErrorAction = ref("");

const workspaceInitializing = ref(true);
const repositoriesLoading = ref(false);
const snapshotLoading = ref(false);
const accountsLoading = ref(false);
const accountStatusRefreshing = ref(false);
const diffLoading = ref(false);
const blameLoading = ref(false);
const platformLoading = ref(false);
const pullRequestsLoading = ref(false);
const historyLoading = ref(false);
let historyRequestGeneration = 0;
let historyLoadedRepositoryPath: string | null = null;
let historyLoadedSignature: string | null = null;
let repositoryTabLoadGeneration = 0;
const reflogLoading = ref(false);
const commitFilesLoading = ref(false);
const commitDiffLoading = ref(false);
const stashesLoading = ref(false);
let stashesRequestGeneration = 0;
let stashesLoadedRepositoryPath: string | null = null;
const worktreesLoading = ref(false);
const submodulesLoading = ref(false);
const lfsLoading = ref(false);
const networkBusy = ref<NetworkAction | null>(null);
let networkOperationGeneration = 0;
const networkOperationsInFlight = new Set<string>();
const mutationBusy = ref(false);
const publishOwnersLoading = ref(false);

const cloneModalShown = ref(false);
const initModalShown = ref(false);
const discoverModalShown = ref(false);
const accountModalShown = ref(false);
const customEndpointModalShown = ref(false);
const branchModalShown = ref(false);
const renameBranchModalShown = ref(false);
const stashModalShown = ref(false);
const rebaseModalShown = ref(false);
const cherryPickModalShown = ref(false);
const worktreeModalShown = ref(false);
const lfsTrackModalShown = ref(false);
const externalEditorModalShown = ref(false);
const blameModalShown = ref(false);
const reflogModalShown = ref(false);
const pullRequestModalShown = ref(false);
const squashModalShown = ref(false);
const publishModalShown = ref(false);
const tagModalShown = ref(false);
const remoteModalShown = ref(false);
const bindingDrawerShown = ref(false);
const bindingRemote = ref<RemoteProfile | null>(null);
const repositoryGroupModalShown = ref(false);
const repositoryAliasModalShown = ref(false);
const settingsModalShown = ref(false);
const repositoryGroupTarget = ref<RepositoryRecord | null>(null);
const repositoryAliasTarget = ref<RepositoryRecord | null>(null);

const cloneForm = reactive({ url: "", destinationPath: "", endpointId: "", accountId: "none" });
const initForm = reactive({ destinationPath: "", initialBranch: "main" });
const discoverForm = reactive({ rootPath: "" });
const accountForm = reactive({ endpointId: "gitee.com", login: "", displayName: "", authMethod: "personalAccessToken" as AuthenticationMethod, token: "" });
const endpointForm = reactive({ id: "", kind: "genericGit", displayName: "", webUrl: "", apiUrl: "", gitHost: "" });
const branchForm = reactive({ name: "", checkout: true });
const renameBranchForm = reactive({ oldName: "", newName: "" });
const stashForm = reactive({ message: "", includeUntracked: true });
const rebaseForm = reactive({ base: "" });
const cherryPickForm = reactive({ revision: "" });
const worktreeForm = reactive({ path: "", mode: "new", branch: "" });
const lfsTrackForm = reactive({ pattern: "" });
const externalEditorForm = reactive({ executable: "", arguments: "" });
const pullRequestForm = reactive({ title: "", body: "", base: "", draft: false });
const squashForm = reactive({ count: "2", message: "" });
const publishForm = reactive({ endpointId: "github.com", accountId: "", ownerLogin: "", name: "", isPrivate: true });
const publishOwners = ref<GitHubPublishOwner[]>([]);
const publishOwnersError = ref("");
let publishOwnersRequestGeneration = 0;
const tagForm = reactive({ name: "", target: "", message: "" });
const tagRepositoryPath = ref<string | null>(null);
const remoteForm = reactive({ name: "", fetchUrl: "", pushUrl: "" });
const editedRemoteName = ref<string | null>(null);
const bindingForm = reactive({ endpointId: "", accountId: "auto" });
const repositoryGroupForm = reactive({ name: "" });
const repositoryAliasForm = reactive({ name: "" });
const endpoints = ref<CodeHostEndpoint[]>([...builtinEndpoints]);
let lastAccountStatusRefreshAt = 0;
const accountStatusRefreshCacheMs = 60_000;
const automaticRefreshDebounceMs = 500;
const automaticRefreshRetryMs = 750;
const repositoryWatcherMaxRetryDelayMs = 12_000;
const repositoryWatcherMaxRetryAttempts = 5;
let automaticRefreshTimer: ReturnType<typeof setTimeout> | null = null;
let repositoryWatcherRetryTimer: ReturnType<typeof setTimeout> | null = null;
let repositoryWatcherRetryAttempt = 0;
let repositoryWatcherUnavailable = false;
let repositoriesLoadPromise: Promise<void> | null = null;
let repositoriesReloadPending = false;
let repositoriesRefreshOwnersPending = false;
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
const currentBranch = computed(() => snapshot.value?.status.branch ?? "—");
const stagedChanges = computed(() => snapshot.value?.status.entries.filter((entry) => Boolean(entry.indexStatus)) ?? []);
const stagedCount = computed(() => stagedChanges.value.length);
const unstagedCount = computed(() => snapshot.value?.status.entries.filter((entry) => Boolean(entry.worktreeStatus)).length ?? 0);
const workingChanges = computed(() => snapshot.value?.status.entries.filter((entry) => Boolean(entry.worktreeStatus)) ?? []);
const untrackedChanges = computed(() =>
  snapshot.value?.status.entries.filter((entry) => !entry.indexStatus && entry.worktreeStatus === "?") ?? [],
);
const untrackedCount = computed(() => untrackedChanges.value.length);
const allChangesIncluded = computed(() => {
  const entries = snapshot.value?.status.entries ?? [];
  return entries.length > 0 && entries.every(isChangeFullyIncluded);
});
const someChangesIncluded = computed(() => {
  const entries = snapshot.value?.status.entries ?? [];
  return !allChangesIncluded.value && entries.some((entry) => Boolean(entry.indexStatus));
});
const conflictedChanges = computed(() => snapshot.value?.status.entries.filter(isConflictedEntry) ?? []);
const diffHunks = computed(() => splitDiffHunks(diffText.value));
const diffHunksByLine = computed(() => new Map(diffHunks.value.map((hunk) => [hunk.lineIndex, hunk])));
const diffLines = computed(() => splitDiffLines(diffText.value));
const hasDiffText = computed(() => diffText.value.trim().length > 0);
const commitDiffVisibleLineCount = ref(commitDiffInitialLineCount);
const commitDiffVisibleChunk = computed(() => splitVisibleDiffLines(commitDiffText.value, commitDiffVisibleLineCount.value));
const commitDiffVisibleLines = computed(() => commitDiffVisibleChunk.value.lines);
const commitDiffHasMoreLines = computed(() => commitDiffVisibleChunk.value.hasMore);
const commitDiffLineLimitReached = computed(() =>
  commitDiffHasMoreLines.value && commitDiffVisibleLineCount.value >= commitDiffMaximumDisplayedLines,
);
const commitDiffCanExpand = computed(() =>
  (!commitDiffLineLimitReached.value && commitDiffHasMoreLines.value) ||
  (commitDiffTruncated.value && !commitDiffMaximumReached.value),
);
const commitDiffLimitedAtMaximum = computed(() => commitDiffTruncated.value && commitDiffMaximumReached.value);
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
    return t("workspace.controller.thisRewritesTheLatestCommitAndIncludesTheCountFileSCurrentlySelected",
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
const rebaseBaseOptions = computed(() =>
  snapshot.value?.branches
    .filter((branch) => !branch.isCurrent)
    .map((branch) => ({ label: branch.name, value: branch.name })) ?? [],
);
const worktreeExistingBranchOptions = computed(() => {
  const checkedOut = new Set(worktrees.value.map((worktree) => worktree.branch).filter(Boolean));
  return (snapshot.value?.branches ?? [])
    .filter((branch) => !branch.isCurrent)
    .map((branch) => ({ label: branch.name, value: branch.name, disabled: checkedOut.has(branch.name) }));
});
const pullRequestBaseOptions = computed(() => {
  const branches = snapshot.value?.branches.filter((branch) => !branch.isCurrent).map((branch) => branch.name) ?? [];
  const defaultBranch = repositoryInfo.value?.defaultBranch;
  const values = [...new Set([defaultBranch, ...branches].filter((branch): branch is string => Boolean(branch)))];
  return values.map((branch) => ({ label: branch, value: branch }));
});
const publishEndpoints = computed(() =>
  endpoints.value.filter((endpoint) => endpoint.kind === "gitHub" && capabilityForEndpoint(endpoint.id).repositoryPublish),
);
const publishAccounts = computed(() =>
  accounts.value.filter(
    (account) =>
      account.endpointId === publishForm.endpointId &&
      account.authMethod === "ghCli" &&
      (account.status === "valid" || account.status === "available"),
  ),
);
const publishAccountOptions = computed(() =>
  publishAccounts.value.map((account) => ({
    label: (account.displayName || account.login) + " · " + authenticationStatusLabel(account.status),
    value: account.id,
  })),
);
const publishOwnerOptions = computed(() =>
  publishOwners.value.map((owner) => ({
    label: (owner.name
      ? `${owner.name} (${owner.login}) · ${owner.kind === "organization" ? t("workspace.controller.organization") : t("workspace.controller.personal")}`
      : `${owner.login} · ${owner.kind === "organization" ? t("workspace.controller.organization") : t("workspace.controller.personal")}`)
      + (owner.canCreateRepositories ? "" : t("workspace.controller.noCreatePermission")),
    value: owner.login,
    disabled: !owner.canCreateRepositories,
  })),
);
const canPublishToSelectedOwner = computed(() =>
  !publishOwnersLoading.value && !publishOwnersError.value && publishOwners.value.some(
    (owner) => owner.login === publishForm.ownerLogin && owner.canCreateRepositories,
  ),
);
const accountFormCapabilities = computed(() => capabilityForEndpoint(accountForm.endpointId));
const accountUsesPat = computed(() => accountForm.authMethod === "personalAccessToken");
const accountMethodOptions = computed(() => [
  { label: t("workspace.controller.personalAccessTokenPat"), value: "personalAccessToken", disabled: !accountFormCapabilities.value.patKeychain },
  { label: t("common.authentication.method.sshAgent"), value: "sshAgent" },
]);
function supportsGitRemoteCredentials(account: AccountProfile): boolean {
  // A saved binding is an explicit account-selection promise. Only the
  // app-managed PAT implementation can keep that promise; an SSH agent picks
  // its identity from the user's SSH configuration at connection time.
  return account.authMethod === "personalAccessToken";
}
function supportsCloneCredentials(account: AccountProfile): boolean {
  // Clone has no durable remote binding yet, so an SSH-agent account remains
  // useful as a UI hint while Git/SSH selects the actual key.
  return account.authMethod === "personalAccessToken" || account.authMethod === "sshAgent";
}
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
const repositoryGroupingOptions = computed(() => [
  { label: t("workspace.controller.manualGroups"), value: "manual" },
  { label: t("workspace.controller.remoteOwner"), value: "owner" },
  { label: t("workspace.controller.recentlyUsed"), value: "recent" },
]);
const repositoryGroups = computed<RepositorySidebarGroup[]>(() => {
  const sorted = [...repositories.value].sort(compareRepositoryRecords);

  if (repositoryGrouping.value === "manual") {
    const groups = new Map<string, RepositorySidebarGroup>();
    const ungrouped: RepositoryRecord[] = [];
    for (const repository of sorted) {
      const name = repository.group?.trim();
      if (!name) {
        ungrouped.push(repository);
        continue;
      }
      const key = name.toLocaleLowerCase();
      const group = groups.get(key);
      if (group) group.repositories.push(repository);
      else groups.set(key, { key: `manual:${key}`, label: name, repositories: [repository] });
    }
    const namedGroups = [...groups.values()].sort((left, right) => left.label.localeCompare(right.label, appLocale.value));
    return ungrouped.length
      ? [...namedGroups, { key: "manual:ungrouped", label: t("workspace.controller.ungrouped"), repositories: ungrouped }]
      : namedGroups;
  }

  if (repositoryGrouping.value === "owner") {
    const groups = new Map<string, RepositorySidebarGroup>();
    const unknown: RepositoryRecord[] = [];
    for (const repository of sorted) {
      const host = repository.host?.trim();
      const namespace = repository.namespace?.trim();
      if (!host || !namespace) {
        unknown.push(repository);
        continue;
      }
      const key = `${host.toLocaleLowerCase()}/${namespace.toLocaleLowerCase()}`;
      const group = groups.get(key);
      if (group) group.repositories.push(repository);
      else groups.set(key, { key: `owner:${key}`, label: `${host} / ${namespace}`, repositories: [repository] });
    }
    const ownerGroups = [...groups.values()].sort((left, right) => left.label.localeCompare(right.label, appLocale.value));
    return unknown.length
      ? [...ownerGroups, { key: "owner:unknown", label: t("workspace.controller.unrecognizedOwner"), repositories: unknown }]
      : ownerGroups;
  }

  const now = Date.now();
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const todayStart = today.getTime();
  const weekStart = now - 7 * 24 * 60 * 60 * 1000;
  const buckets: RepositorySidebarGroup[] = [
    { key: "recent:today", label: t("workspace.controller.today"), repositories: [] },
    { key: "recent:week", label: t("workspace.controller.last7Days"), repositories: [] },
    { key: "recent:older", label: t("workspace.controller.earlier"), repositories: [] },
    { key: "recent:never", label: t("workspace.controller.notOpenedYet"), repositories: [] },
  ];
  for (const repository of sorted) {
    const openedAt = repository.lastOpenedAt;
    if (typeof openedAt !== "number" || !Number.isFinite(openedAt)) buckets[3].repositories.push(repository);
    else if (openedAt >= todayStart) buckets[0].repositories.push(repository);
    else if (openedAt >= weekStart) buckets[1].repositories.push(repository);
    else buckets[2].repositories.push(repository);
  }
  for (const bucket of buckets) {
    bucket.repositories.sort((left, right) => (right.lastOpenedAt ?? 0) - (left.lastOpenedAt ?? 0) || compareRepositoryRecords(left, right));
  }
  return buckets.filter((bucket) => bucket.repositories.length);
});

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

function endpointHasExplicitPort(gitHost: string): boolean {
  const value = gitHost.trim();
  const closingBracket = value.lastIndexOf("]");
  if (value.startsWith("[") && closingBracket !== -1) return /^:\d+$/u.test(value.slice(closingBracket + 1));
  return /:\d+$/u.test(value);
}

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

function updateChangeInclusion(entry: GitStatusEntry, included: boolean): void {
  void setChangeIncluded(entry, included);
}

function updateAllChangeInclusion(included: boolean): void {
  void (included ? stageAll() : unstageAll());
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

function updateAppLocale(value: string | number): void {
  if (value === "zh-CN" || value === "en-US") setAppLocale(value as AppLocale);
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

    const historyWasLoaded = activeTab.value === "history"
      || historyLoadedRepositoryPath === repositoryPath
      || stashesLoadedRepositoryPath === repositoryPath;
    if (historyWasLoaded) {
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
          commitDiffCache.clear();
          commitDiffCacheWeight = 0;
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

function updateColorMode(value: string | number): void {
  if (value === "system" || value === "light" || value === "dark") setColorMode(value as ColorMode);
}

function updateThemeColor(value: string | null): void {
  if (value) setThemeColor(value);
}

function openRepositoryAction(value: string | number): void {
  if (value === "discover") discoverModalShown.value = true;
  else if (value === "init") initModalShown.value = true;
  else if (value === "clone") cloneModalShown.value = true;
}

function handleDevtoolsShortcut(event: KeyboardEvent): void {
  if (event.key !== "F12") return;
  event.preventDefault();
  void toggleDevtools().catch(() => undefined);
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

function reuseEqualArray<T>(currentValue: T[], nextValue: T[], itemsAreEqual: (left: T, right: T) => boolean): T[] {
  return currentValue.length === nextValue.length &&
    currentValue.every((item, index) => itemsAreEqual(item, nextValue[index]))
    ? currentValue
    : nextValue;
}

function branchesAreEqual(left: BranchInfo, right: BranchInfo): boolean {
  return left.name === right.name && left.isCurrent === right.isCurrent && left.upstream === right.upstream;
}

function remoteBranchesAreEqual(left: RemoteBranchInfo, right: RemoteBranchInfo): boolean {
  return left.remoteName === right.remoteName && left.name === right.name;
}

function remotesAreEqual(left: RemoteProfile, right: RemoteProfile): boolean {
  return left.name === right.name &&
    left.fetchUrl === right.fetchUrl &&
    left.pushUrl === right.pushUrl &&
    left.endpointId === right.endpointId &&
    left.codeHostKind === right.codeHostKind &&
    left.parsed.transport === right.parsed.transport &&
    left.parsed.host === right.parsed.host &&
    left.parsed.port === right.parsed.port &&
    left.parsed.namespace === right.parsed.namespace &&
    left.parsed.repository === right.parsed.repository;
}

function bindingsAreEqual(left: RemoteBinding, right: RemoteBinding): boolean {
  return left.repositoryPath === right.repositoryPath &&
    left.remoteName === right.remoteName &&
    left.endpointId === right.endpointId &&
    left.accountSelection.mode === right.accountSelection.mode &&
    (left.accountSelection.mode === "auto" || (
      right.accountSelection.mode === "profile" &&
      left.accountSelection.accountProfileId === right.accountSelection.accountProfileId
    ));
}

function worktreesAreEqual(left: WorktreeInfo, right: WorktreeInfo): boolean {
  return left.path === right.path &&
    left.head === right.head &&
    left.branch === right.branch &&
    left.isMain === right.isMain &&
    left.isDetached === right.isDetached &&
    left.isLocked === right.isLocked &&
    left.isPrunable === right.isPrunable;
}

function submodulesAreEqual(left: SubmoduleInfo, right: SubmoduleInfo): boolean {
  return left.path === right.path && left.commit === right.commit && left.status === right.status;
}

function lfsStatusesAreEqual(left: LfsStatus, right: LfsStatus): boolean {
  return left.available === right.available &&
    left.initialized === right.initialized &&
    left.version === right.version &&
    left.files.length === right.files.length &&
    left.files.every((file, index) => file === right.files[index]);
}

function clearDiffSelection(): void {
  diffRequestGeneration += 1;
  selectedChange.value = null;
  diffText.value = "";
  selectedDiffStaged.value = false;
  diffLoading.value = false;
}

function commitFilesCacheKey(repositoryPath: string, revision: string): string {
  return `${repositoryPath}\u0000${revision}`;
}

function commitDiffCacheKey(
  repositoryPath: string,
  revision: string,
  path: string,
  previousPath: string | null,
): string {
  return `${repositoryPath}\u0000${revision}\u0000${previousPath ?? ""}\u0000${path}`;
}

function readLruCache<T>(cache: Map<string, T>, key: string): T | undefined {
  const value = cache.get(key);
  if (value === undefined) return undefined;
  cache.delete(key);
  cache.set(key, value);
  return value;
}

function writeCommitDiffCache(key: string, value: CachedCommitFileDiff): void {
  const previous = commitDiffCache.get(key);
  if (previous) commitDiffCacheWeight -= previous.text.length * 2;
  commitDiffCache.delete(key);
  const weight = value.text.length * 2;
  if (weight > commitDiffCacheMaximumWeight) return;
  commitDiffCache.set(key, value);
  commitDiffCacheWeight += weight;
  while (commitDiffCache.size > commitCacheLimit || commitDiffCacheWeight > commitDiffCacheMaximumWeight) {
    const oldestKey = commitDiffCache.keys().next().value;
    if (oldestKey === undefined) break;
    const oldest = commitDiffCache.get(oldestKey);
    if (oldest) commitDiffCacheWeight -= oldest.text.length * 2;
    commitDiffCache.delete(oldestKey);
  }
}

function writeCommitFilesCache(key: string, files: CommitChangedFile[]): void {
  const previous = commitFilesCache.get(key);
  if (previous) commitFilesCacheWeight -= previous.weight;
  commitFilesCache.delete(key);
  const weight = files.reduce(
    (total, file) => total + 64 + (file.path.length + (file.previousPath?.length ?? 0)) * 2,
    0,
  );
  if (weight > commitFilesCacheMaximumWeight) return;
  commitFilesCache.set(key, { files, weight });
  commitFilesCacheWeight += weight;
  while (commitFilesCache.size > commitCacheLimit || commitFilesCacheWeight > commitFilesCacheMaximumWeight) {
    const oldestKey = commitFilesCache.keys().next().value;
    if (oldestKey === undefined) break;
    const oldest = commitFilesCache.get(oldestKey);
    if (oldest) commitFilesCacheWeight -= oldest.weight;
    commitFilesCache.delete(oldestKey);
  }
}

function resetCommitDiffPresentation(): void {
  commitDiffVisibleLineCount.value = commitDiffInitialLineCount;
  commitDiffTruncated.value = false;
  commitDiffMaximumReached.value = false;
}

function splitVisibleDiffLines(diff: string, maximumLines: number): { lines: string[]; hasMore: boolean } {
  const lines: string[] = [];
  let start = 0;
  while (start < diff.length && lines.length < maximumLines) {
    const newline = diff.indexOf("\n", start);
    if (newline === -1) {
      lines.push(diff.slice(start));
      return { lines, hasMore: false };
    }
    lines.push(diff.slice(start, newline));
    start = newline + 1;
  }
  return { lines, hasMore: start < diff.length };
}

function showMoreCommitDiffLines(): void {
  if (
    commitDiffLineLimitReached.value &&
    commitDiffTruncated.value &&
    !commitDiffMaximumReached.value
  ) {
    loadFullCommitFileDiff();
    return;
  }
  if (commitDiffHasMoreLines.value) {
    commitDiffVisibleLineCount.value = Math.min(
      commitDiffVisibleLineCount.value + commitDiffLineIncrement,
      commitDiffMaximumDisplayedLines,
    );
    return;
  }
  if (commitDiffTruncated.value && !commitDiffMaximumReached.value) loadFullCommitFileDiff();
}

function clearCommitSelection(): void {
  if (commitFilesDebounceTimer) clearTimeout(commitFilesDebounceTimer);
  if (commitDiffDebounceTimer) clearTimeout(commitDiffDebounceTimer);
  commitFilesDebounceTimer = null;
  commitDiffDebounceTimer = null;
  pendingCommitDiffRequest = null;
  commitFilesRequestGeneration += 1;
  commitDiffRequestGeneration += 1;
  selectedCommit.value = null;
  commitChangedFiles.value = [];
  selectedCommitFile.value = null;
  commitDiffText.value = "";
  resetCommitDiffPresentation();
  commitFilesLoading.value = false;
  commitDiffLoading.value = false;
}

async function syncSelectedDiff(
  nextSnapshot: RepositorySnapshot,
  selectedPath?: string,
  selectFirst = false,
  repositoryPath = activeRepositoryPath.value,
  reportErrors = true,
): Promise<void> {
  if (!repositoryPath || activeRepositoryPath.value !== repositoryPath) return;
  const nextChange =
    (selectedPath ? nextSnapshot.status.entries.find((entry) => entry.path === selectedPath) : null) ??
    (selectFirst ? nextSnapshot.status.entries[0] : null);
  if (nextChange) {
    await loadDiff(nextChange, repositoryPath, reportErrors);
  } else {
    clearDiffSelection();
  }
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

function compareRepositoryRecords(left: RepositoryRecord, right: RepositoryRecord): number {
  const leftLabel = left.alias?.trim() || left.name;
  const rightLabel = right.alias?.trim() || right.name;
  return leftLabel.localeCompare(rightLabel, appLocale.value, { sensitivity: "base" }) || left.path.localeCompare(right.path, appLocale.value);
}

function replaceRepositoryRecord(repository: RepositoryRecord): void {
  const index = repositories.value.findIndex((item) => item.path === repository.path);
  if (index === -1) repositories.value = [...repositories.value, repository];
  else repositories.value = repositories.value.map((item) => (item.path === repository.path ? repository : item));
}

function openRepositoryGroupEditor(repository: RepositoryRecord): void {
  repositoryGroupTarget.value = repository;
  repositoryGroupForm.name = repository.group || "";
  repositoryGroupModalShown.value = true;
}

async function saveRepositoryGroup(): Promise<void> {
  const target = repositoryGroupTarget.value;
  if (!target) return;
  mutationBusy.value = true;
  try {
    const name = repositoryGroupForm.name.trim();
    const updated = await api.setRepositoryGroup(target.path, name || null);
    replaceRepositoryRecord(updated);
    if (snapshot.value?.repository.path === updated.path) {
      snapshot.value = { ...snapshot.value, repository: updated };
    }
    repositoryGroupModalShown.value = false;
    message.success(name ? t("workspace.controller.repositoryGroupSaved") : t("workspace.controller.repositoryMovedToUngrouped"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotSaveRepositoryGroup"));
  } finally {
    mutationBusy.value = false;
  }
}

function openRepositoryAliasEditor(repository: RepositoryRecord): void {
  repositoryAliasTarget.value = repository;
  repositoryAliasForm.name = repository.alias || "";
  repositoryAliasModalShown.value = true;
}

async function saveRepositoryAlias(): Promise<void> {
  const target = repositoryAliasTarget.value;
  if (!target) return;
  mutationBusy.value = true;
  try {
    const name = repositoryAliasForm.name.trim();
    const updated = await api.setRepositoryAlias(target.path, name || null);
    replaceRepositoryRecord(updated);
    if (snapshot.value?.repository.path === updated.path) {
      snapshot.value = { ...snapshot.value, repository: updated };
    }
    repositoryAliasModalShown.value = false;
    message.success(name ? t("workspace.controller.repositoryAliasSaved") : t("workspace.controller.repositoryAliasRemoved"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotSaveRepositoryAlias"));
  } finally {
    mutationBusy.value = false;
  }
}

async function loadRepositories(refreshOwners = repositoryGrouping.value === "owner"): Promise<void> {
  if (repositoriesLoadPromise) {
    repositoriesReloadPending = true;
    repositoriesRefreshOwnersPending ||= refreshOwners;
    return repositoriesLoadPromise;
  }
  repositoriesLoadPromise = (async () => {
    repositoriesLoading.value = true;
    let shouldRefreshOwners = refreshOwners;
    try {
      do {
        repositoriesReloadPending = false;
        repositoriesRefreshOwnersPending = false;
        try {
          repositories.value = await api.listRepositories(shouldRefreshOwners);
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotLoadRepositories"));
          if (!repositoriesReloadPending) break;
        }
        shouldRefreshOwners = repositoriesRefreshOwnersPending;
      } while (repositoriesReloadPending);
    } finally {
      repositoriesLoading.value = false;
      repositoriesLoadPromise = null;
    }
  })();
  return repositoriesLoadPromise;
}

async function loadAccounts(): Promise<void> {
  accountsLoading.value = true;
  try {
    accounts.value = await api.listAccounts();
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotLoadAccounts"));
  } finally {
    accountsLoading.value = false;
  }
}

async function refreshAccountStatuses(force = false): Promise<void> {
  if (accountStatusRefreshing.value) return;
  if (!force && Date.now() - lastAccountStatusRefreshAt < accountStatusRefreshCacheMs) return;
  accountStatusRefreshing.value = true;
  try {
    accounts.value = await api.refreshAccountStatuses();
    lastAccountStatusRefreshAt = Date.now();
  } catch (error) {
    if (force) reportError(error, t("workspace.controller.couldNotRefreshAccountAuthenticationStatus"));
  } finally {
    accountStatusRefreshing.value = false;
  }
}

async function loadAccountsThenRefreshStatuses(): Promise<void> {
  await loadAccounts();
  void refreshAccountStatuses();
}

async function loadEndpoints(): Promise<void> {
  try {
    endpoints.value = await api.listEndpoints();
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotLoadGitServiceEndpoints"));
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
  invalidateCommitHistoryCache();
  invalidateStashesCache();
  worktreesRequestGeneration += 1;
  submodulesRequestGeneration += 1;
  lfsRequestGeneration += 1;
  if (commitFilesDebounceTimer) clearTimeout(commitFilesDebounceTimer);
  if (commitDiffDebounceTimer) clearTimeout(commitDiffDebounceTimer);
  commitFilesDebounceTimer = null;
  commitDiffDebounceTimer = null;
  pendingCommitDiffRequest = null;
  commitFilesRequestGeneration += 1;
  commitDiffRequestGeneration += 1;
  commitFilesLoading.value = false;
  commitDiffLoading.value = false;
  worktreesLoading.value = false;
  submodulesLoading.value = false;
  lfsLoading.value = false;
  networkOperationGeneration += 1;
  networkBusy.value = null;
  commits.value = [];
  stashes.value = [];
  selectedCommit.value = null;
  commitChangedFiles.value = [];
  selectedCommitFile.value = null;
  commitDiffText.value = "";
  tagModalShown.value = false;
  tagRepositoryPath.value = null;
  snapshot.value = null;
  repositoryInfo.value = null;
  pullRequests.value = [];
  worktrees.value = [];
  submodules.value = [];
  lfsStatus.value = null;
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
    worktrees.value = [];
    submodules.value = [];
    lfsStatus.value = null;
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
    const selectedDiffGeneration = diffRequestGeneration;
    const nextSnapshot = await api.refreshRepositorySnapshot(repositoryPath);
    if (
      !repositorySelectionIsCurrent(repositoryPath, selectionGeneration) ||
      applicationGeneration !== snapshotApplicationGeneration ||
      refreshGeneration !== snapshotRefreshRequestGeneration
    ) return;
    if (!applySnapshot(nextSnapshot, false)) return;
    const appliedApplicationGeneration = snapshotApplicationGeneration;
    const snapshotChanged = appliedApplicationGeneration !== applicationGeneration;
    if (snapshotChanged && selectedDiffGeneration === diffRequestGeneration) {
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

function historySnapshotSignature(repositorySnapshot: RepositorySnapshot | null): string {
  if (!repositorySnapshot) return "";
  return JSON.stringify([
    repositorySnapshot.status.headCommit,
    repositorySnapshot.status.branch,
    repositorySnapshot.status.upstream,
    repositorySnapshot.status.upstreamCommit,
    repositorySnapshot.status.ahead,
    repositorySnapshot.status.behind,
    repositorySnapshot.tags.map((tag) => [tag.name, tag.target]),
  ]);
}

function invalidateCommitHistoryCache(): void {
  historyRequestGeneration += 1;
  historyLoadedRepositoryPath = null;
  historyLoadedSignature = null;
  historyLoading.value = false;
}

function invalidateStashesCache(): void {
  stashesRequestGeneration += 1;
  stashesLoadedRepositoryPath = null;
  stashesLoading.value = false;
}

function invalidateCommitDetailCaches(): void {
  commitFilesCache.clear();
  commitFilesCacheWeight = 0;
  commitDiffCache.clear();
  commitDiffCacheWeight = 0;
  clearCommitSelection();
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
    const diff = await api.getFileDiff(repositoryPath, entry.path, Boolean(entry.indexStatus) && !entry.worktreeStatus);
    if (
      activeRepositoryPath.value !== repositoryPath ||
      selectedChange.value?.path !== entry.path ||
      requestGeneration !== diffRequestGeneration
    ) {
      return;
    }
    diffText.value = diff;
  } catch (error) {
    if (activeRepositoryPath.value === repositoryPath && requestGeneration === diffRequestGeneration) {
      if (reportErrors) reportError(error, t("workspace.controller.couldNotLoadDiff"));
    }
  } finally {
    if (activeRepositoryPath.value === repositoryPath && requestGeneration === diffRequestGeneration) {
      diffLoading.value = false;
    }
  }
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
  const selectionGeneration = repositorySelectionGeneration;
  const selectedPath = selectedChange.value.path;
  const reverse = selectedDiffStaged.value;
  mutationBusy.value = true;
  try {
    const nextSnapshot = await api.applyPatch(repositoryPath, selectedPath, hunk.patch, reverse);
    if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
    const nextEntry = nextSnapshot.status.entries.find((entry) => entry.path === selectedPath);
    if (nextEntry) {
      await loadDiff(nextEntry, repositoryPath);
    } else {
      selectedChange.value = null;
      diffText.value = "";
      selectedDiffStaged.value = false;
    }
    message.success(reverse ? t("workspace.controller.hunkUnstaged") : t("workspace.controller.hunkStaged"));
  } catch (error) {
    reportError(error, reverse ? t("workspace.controller.couldNotUnstageHunk") : t("workspace.controller.couldNotStageHunk"));
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

async function loadHistory(): Promise<void> {
  await loadHistoryWithOptions({ force: true });
}

async function loadHistoryWithOptions(options: HistoryLoadOptions = {}): Promise<void> {
  await Promise.all([
    loadCommitHistoryWithOptions(options),
    loadStashesWithOptions(options),
  ]);
}

async function loadCommitHistoryWithOptions(options: HistoryLoadOptions = {}): Promise<void> {
  const repositoryPath = activeRepositoryPath.value;
  if (!repositoryPath) return;
  const {
    force = false,
    reportErrors = true,
    silent = false,
  } = options;
  const requestHistorySignature = historySnapshotSignature(snapshot.value);
  if (
    !force &&
    historyLoadedRepositoryPath === repositoryPath &&
    historyLoadedSignature === requestHistorySignature
  ) return;
  if (force) {
    historyLoadedRepositoryPath = null;
    historyLoadedSignature = null;
  }
  if (!force && historyLoading.value) return;
  const requestGeneration = ++historyRequestGeneration;
  if (!silent) historyLoading.value = true;
  try {
    const nextCommits = await api.listCommits(repositoryPath);
    if (
      activeRepositoryPath.value !== repositoryPath ||
      requestGeneration !== historyRequestGeneration ||
      historySnapshotSignature(snapshot.value) !== requestHistorySignature
    ) return;
    commits.value = nextCommits;
    historyLoadedRepositoryPath = repositoryPath;
    historyLoadedSignature = requestHistorySignature;
    if (selectedCommit.value && !nextCommits.some((commit) => commit.sha === selectedCommit.value?.sha)) {
      clearCommitSelection();
    }
  } catch (error) {
    if (activeRepositoryPath.value === repositoryPath && requestGeneration === historyRequestGeneration) {
      if (reportErrors) reportError(error, t("workspace.controller.couldNotLoadCommitHistory"));
    }
  } finally {
    if (!silent && activeRepositoryPath.value === repositoryPath && requestGeneration === historyRequestGeneration) historyLoading.value = false;
  }
}

async function loadStashesWithOptions(options: HistoryLoadOptions = {}): Promise<void> {
  const repositoryPath = activeRepositoryPath.value;
  if (!repositoryPath) return;
  const {
    force = false,
    reportErrors = true,
    silent = false,
  } = options;
  if (!force && stashesLoadedRepositoryPath === repositoryPath) return;
  if (force) stashesLoadedRepositoryPath = null;
  if (!force && stashesLoading.value) return;
  const requestGeneration = ++stashesRequestGeneration;
  if (!silent) stashesLoading.value = true;
  try {
    const nextStashes = await api.listStashes(repositoryPath);
    if (activeRepositoryPath.value !== repositoryPath || requestGeneration !== stashesRequestGeneration) return;
    stashes.value = nextStashes;
    stashesLoadedRepositoryPath = repositoryPath;
  } catch (error) {
    if (activeRepositoryPath.value === repositoryPath && requestGeneration === stashesRequestGeneration) {
      if (reportErrors) reportError(error, t("workspace.controller.couldNotLoadStashes"));
    }
  } finally {
    if (!silent && activeRepositoryPath.value === repositoryPath && requestGeneration === stashesRequestGeneration) stashesLoading.value = false;
  }
}

async function openReflog(): Promise<void> {
  if (!activeRepositoryPath.value) return;
  reflogModalShown.value = true;
  reflogText.value = "";
  reflogLoading.value = true;
  try {
    reflogText.value = await api.listReflog(activeRepositoryPath.value);
  } catch (error) {
    reflogModalShown.value = false;
    reportError(error, t("workspace.controller.couldNotLoadReflog"));
  } finally {
    reflogLoading.value = false;
  }
}

async function loadWorktrees(reportErrors = true, showLoading = true): Promise<void> {
  const repositoryPath = activeRepositoryPath.value;
  if (!repositoryPath) {
    worktrees.value = [];
    return;
  }
  const selectionGeneration = repositorySelectionGeneration;
  const requestGeneration = ++worktreesRequestGeneration;
  const shouldClearLoading = showLoading || worktreesLoading.value;
  if (showLoading) worktreesLoading.value = true;
  try {
    const nextWorktrees = await api.listWorktrees(repositoryPath);
    if (
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      requestGeneration === worktreesRequestGeneration
    ) worktrees.value = reuseEqualArray(worktrees.value, nextWorktrees, worktreesAreEqual);
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === worktreesRequestGeneration) {
      if (reportErrors) reportError(error, t("workspace.controller.couldNotLoadWorktrees"));
    }
  } finally {
    if (
      shouldClearLoading &&
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      requestGeneration === worktreesRequestGeneration
    ) worktreesLoading.value = false;
  }
}

async function loadSubmodules(reportErrors = true, showLoading = true): Promise<void> {
  const repositoryPath = activeRepositoryPath.value;
  if (!repositoryPath) {
    submodules.value = [];
    return;
  }
  const selectionGeneration = repositorySelectionGeneration;
  const requestGeneration = ++submodulesRequestGeneration;
  const shouldClearLoading = showLoading || submodulesLoading.value;
  if (showLoading) submodulesLoading.value = true;
  try {
    const nextSubmodules = await api.listSubmodules(repositoryPath);
    if (
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      requestGeneration === submodulesRequestGeneration
    ) submodules.value = reuseEqualArray(submodules.value, nextSubmodules, submodulesAreEqual);
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === submodulesRequestGeneration) {
      if (reportErrors) reportError(error, t("workspace.controller.couldNotLoadSubmodules"));
    }
  } finally {
    if (
      shouldClearLoading &&
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      requestGeneration === submodulesRequestGeneration
    ) submodulesLoading.value = false;
  }
}

async function loadLfsStatus(reportErrors = true, showLoading = true): Promise<void> {
  const repositoryPath = activeRepositoryPath.value;
  if (!repositoryPath) {
    lfsStatus.value = null;
    return;
  }
  const selectionGeneration = repositorySelectionGeneration;
  const requestGeneration = ++lfsRequestGeneration;
  const shouldClearLoading = showLoading || lfsLoading.value;
  if (showLoading) lfsLoading.value = true;
  try {
    const nextLfsStatus = await api.getLfsStatus(repositoryPath);
    if (
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      requestGeneration === lfsRequestGeneration
    ) lfsStatus.value = lfsStatus.value
      ? (lfsStatusesAreEqual(lfsStatus.value, nextLfsStatus) ? lfsStatus.value : nextLfsStatus)
      : nextLfsStatus;
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === lfsRequestGeneration) {
      if (reportErrors) reportError(error, t("workspace.controller.couldNotLoadGitLfsStatus"));
    }
  } finally {
    if (
      shouldClearLoading &&
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      requestGeneration === lfsRequestGeneration
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

function loadCommitFiles(commit: CommitSummary): void {
  const repositoryPath = activeRepositoryPath.value;
  if (!repositoryPath) return;
  const selectionGeneration = repositorySelectionGeneration;
  const requestGeneration = ++commitFilesRequestGeneration;
  if (commitFilesDebounceTimer) clearTimeout(commitFilesDebounceTimer);
  if (commitDiffDebounceTimer) clearTimeout(commitDiffDebounceTimer);
  commitFilesDebounceTimer = null;
  commitDiffDebounceTimer = null;
  commitDiffRequestGeneration += 1;
  selectedCommit.value = commit;
  commitChangedFiles.value = [];
  selectedCommitFile.value = null;
  commitDiffText.value = "";
  resetCommitDiffPresentation();
  commitDiffLoading.value = false;
  const cacheKey = commitFilesCacheKey(repositoryPath, commit.sha);
  const cachedFiles = readLruCache(commitFilesCache, cacheKey);
  if (cachedFiles) {
    commitChangedFiles.value = cachedFiles.files;
    const firstFile = cachedFiles.files[0];
    if (firstFile) loadCommitFileDiff(firstFile, commit, repositoryPath);
    commitFilesLoading.value = false;
    return;
  }
  commitFilesLoading.value = true;
  commitFilesDebounceTimer = setTimeout(() => {
    commitFilesDebounceTimer = null;
    void fetchCommitFiles(repositoryPath, commit, selectionGeneration, requestGeneration, cacheKey);
  }, commitSelectionDebounceMs);
}

async function fetchCommitFiles(
  repositoryPath: string,
  commit: CommitSummary,
  selectionGeneration: number,
  requestGeneration: number,
  cacheKey: string,
): Promise<void> {
  try {
    const nextFiles = await api.listCommitChangedFiles(repositoryPath, commit.sha);
    if (
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      selectedCommit.value?.sha === commit.sha &&
      requestGeneration === commitFilesRequestGeneration
    ) {
      writeCommitFilesCache(cacheKey, nextFiles);
      commitChangedFiles.value = nextFiles;
      const firstFile = nextFiles[0];
      if (firstFile) loadCommitFileDiff(firstFile, commit, repositoryPath);
    }
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === commitFilesRequestGeneration) {
      reportError(error, t("workspace.controller.couldNotLoadChangedFiles"));
    }
  } finally {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === commitFilesRequestGeneration) {
      commitFilesLoading.value = false;
    }
  }
}

function loadCommitFileDiff(
  file: CommitChangedFile,
  commit = selectedCommit.value,
  repositoryPath = activeRepositoryPath.value,
): void {
  if (!repositoryPath || !commit || activeRepositoryPath.value !== repositoryPath) return;
  const selectionGeneration = repositorySelectionGeneration;
  const requestGeneration = ++commitDiffRequestGeneration;
  pendingCommitDiffRequest = null;
  if (commitDiffDebounceTimer) clearTimeout(commitDiffDebounceTimer);
  commitDiffDebounceTimer = null;
  selectedCommitFile.value = file;
  commitDiffText.value = "";
  resetCommitDiffPresentation();
  const cacheKey = commitDiffCacheKey(repositoryPath, commit.sha, file.path, file.previousPath);
  const cachedDiff = readLruCache(commitDiffCache, cacheKey);
  if (cachedDiff !== undefined) {
    commitDiffText.value = cachedDiff.text;
    commitDiffTruncated.value = cachedDiff.truncated;
    commitDiffMaximumReached.value = cachedDiff.maximumReached;
    commitDiffLoading.value = false;
    return;
  }
  commitDiffLoading.value = true;
  commitDiffDebounceTimer = setTimeout(() => {
    commitDiffDebounceTimer = null;
    queueCommitFileDiff({
      repositoryPath,
      commit,
      file,
      selectionGeneration,
      requestGeneration,
      cacheKey,
      maximumBytes: commitDiffPreviewBytes,
    });
  }, commitSelectionDebounceMs);
}

function loadFullCommitFileDiff(): void {
  const repositoryPath = activeRepositoryPath.value;
  const commit = selectedCommit.value;
  const file = selectedCommitFile.value;
  if (!repositoryPath || !commit || !file || !commitDiffTruncated.value || commitDiffLoading.value) return;
  const selectionGeneration = repositorySelectionGeneration;
  const requestGeneration = ++commitDiffRequestGeneration;
  commitDiffLoading.value = true;
  queueCommitFileDiff({
    repositoryPath,
    commit,
    file,
    selectionGeneration,
    requestGeneration,
    cacheKey: commitDiffCacheKey(repositoryPath, commit.sha, file.path, file.previousPath),
    maximumBytes: commitDiffMaximumBytes,
  });
}

function queueCommitFileDiff(request: CommitFileDiffRequest): void {
  pendingCommitDiffRequest = request;
  if (!commitDiffRequestInFlight) void drainCommitFileDiffRequests();
}

async function drainCommitFileDiffRequests(): Promise<void> {
  if (commitDiffRequestInFlight) return;
  commitDiffRequestInFlight = true;
  try {
    while (controllerMounted && pendingCommitDiffRequest) {
      const request = pendingCommitDiffRequest;
      pendingCommitDiffRequest = null;
      await fetchCommitFileDiff(
        request.repositoryPath,
        request.commit,
        request.file,
        request.selectionGeneration,
        request.requestGeneration,
        request.cacheKey,
        request.maximumBytes,
      );
    }
  } finally {
    commitDiffRequestInFlight = false;
  }
}

async function fetchCommitFileDiff(
  repositoryPath: string,
  commit: CommitSummary,
  file: CommitChangedFile,
  selectionGeneration: number,
  requestGeneration: number,
  cacheKey: string,
  maximumBytes: number,
): Promise<void> {
  try {
    const nextDiff = await api.getCommitFileDiff(
      repositoryPath,
      commit.sha,
      file.path,
      file.previousPath,
      maximumBytes,
    );
    if (
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      selectedCommit.value?.sha === commit.sha &&
      selectedCommitFile.value?.path === file.path &&
      requestGeneration === commitDiffRequestGeneration
    ) {
      const maximumReached = maximumBytes === commitDiffMaximumBytes && nextDiff.truncated;
      writeCommitDiffCache(cacheKey, { ...nextDiff, maximumReached });
      resetCommitDiffPresentation();
      commitDiffText.value = nextDiff.text;
      commitDiffTruncated.value = nextDiff.truncated;
      commitDiffMaximumReached.value = maximumReached;
    }
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === commitDiffRequestGeneration) {
      reportError(error, t("workspace.controller.couldNotLoadTheFileDiff"));
    }
  } finally {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === commitDiffRequestGeneration) {
      commitDiffLoading.value = false;
    }
  }
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
    reportError(error, included ? t("workspace.controller.couldNotStageChanges") : t("workspace.controller.couldNotUnstageChanges"));
  } finally {
    mutationBusy.value = false;
  }
}

async function stageAll(): Promise<void> {
  if (!activeRepositoryPath.value || !workingChanges.value.length) return;
  mutationBusy.value = true;
  try {
    const selectedPath = selectedChange.value?.path;
    const nextSnapshot = await api.stagePaths(activeRepositoryPath.value, workingChanges.value.map((entry) => entry.path));
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
    const nextSnapshot = await api.unstagePaths(activeRepositoryPath.value, stagedChanges.value.map((entry) => entry.path));
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
    content: t("workspace.controller.permanentlyDiscardWorkingTreeAndStagedChangesInCountFileSThisCannotBeUndoneUntracked",
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
        if (selectedChange.value && uniquePaths.includes(selectedChange.value.path)) {
          clearDiffSelection();
        }
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
  const count = entries.length;
  dialog.error({
    title: t("workspace.controller.cleanUntrackedFiles"),
    content: t("workspace.controller.permanentlyDeleteCountUntrackedFileSOrFolderSIgnoredFilesAreNotRemovedAndThisCannotBeUndone",
      { count },
    ),
    positiveText: t("workspace.controller.deletePermanently"),
    negativeText: t("workspace.controller.cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        const selectedPath = selectedChange.value?.path;
        const nextSnapshot = await api.cleanUntracked(
          activeRepositoryPath.value!,
          cleanPreview.confirmation,
        );
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

async function refreshAfterGitConflict(error: unknown): Promise<void> {
  if (isAppError(error) && error.code === "git_merge_conflict") await refreshSnapshot();
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
    content: t("workspace.controller.clearTheExternalEditorConfigurationSavedByThisAppThisDoesNotChangeSystemGitOrEditorSettings",
    ),
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
    message.info(
      t("workspace.controller.openedCountConflictedFileSInTheExternalEditorWhenFinishedReturnHereToRefreshStageAndContinue",
        { count: paths.length },
      ),
    );
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotOpenExternalEditor"));
  } finally {
    mutationBusy.value = false;
  }
}

const commitMessage = ref("");
const amendLastCommit = ref(false);

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
    const nextSnapshot = await api.commitChanges(activeRepositoryPath.value, commitMessage.value.trim(), amendLastCommit.value);
    applySnapshot(nextSnapshot);
    await syncSelectedDiff(nextSnapshot, selectedPath);
    commitMessage.value = "";
    const amended = amendLastCommit.value;
    amendLastCommit.value = false;
    await loadHistory();
    message.success(amended ? t("workspace.controller.latestCommitAmended") : t("workspace.controller.commitCreated"));
  } catch (error) {
    reportError(error, t("workspace.controller.commitFailed"));
  } finally {
    mutationBusy.value = false;
  }
}

function requestBranchSwitch(
  target: string,
  execute: (stashLocalChanges: boolean) => Promise<void>,
): void {
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
    content: t("workspace.controller.thereAreCountUncommittedChangeSBeforeSwitchingToBranchStashAndSwitchSavesStagedUnstagedAndUntrac",
      { count: changeCount, branch: target },
    ),
    closable: true,
    action: () => [
      h(
        NButton,
        { onClick: () => branchSwitchDialog.destroy() },
        { default: () => t("workspace.controller.cancel") },
      ),
      h(
        NButton,
        {
          tertiary: true,
          onClick: () => {
            branchSwitchDialog.destroy();
            void execute(false);
          },
        },
        { default: () => t("workspace.controller.switchDirectly") },
      ),
      h(
        NButton,
        {
          type: "primary",
          onClick: () => {
            branchSwitchDialog.destroy();
            void execute(true);
          },
        },
        { default: () => t("workspace.controller.stashAndSwitch") },
      ),
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
    message.success(
      stashLocalChanges
        ? t("workspace.controller.changesStashedAndSwitchedToBranch", { branch: branch.name })
        : t("workspace.controller.switchedToBranch", { branch: branch.name }),
    );
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
    message.success(
      stashLocalChanges
        ? t("workspace.controller.changesStashedAndSwitchedToBranch", { branch: branchLabel })
        : t("workspace.controller.switchedToBranch", { branch: branchLabel }),
    );
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
  const selectionGeneration = repositorySelectionGeneration;
  mutationBusy.value = true;
  try {
    const nextSnapshot = await api.createBranch({ repositoryPath, name: branchForm.name.trim(), checkout: branchForm.checkout });
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
    applySnapshot(
      await api.renameBranch(
        activeRepositoryPath.value,
        renameBranchForm.oldName,
        renameBranchForm.newName.trim(),
      ),
    );
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
    content: t("workspace.controller.deleteLocalBranchBranchGitWillRejectAnUnmergedBranchToPreventAccidentalLossOfCommits",
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
    content: t("workspace.controller.mergeBranchIntoTheCurrentBranchCurrentIfConflictsOccurGitLeavesTheWorkingTreeAvailableForResolut",
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
    message.success(snapshot.value?.operation ? t("workspace.controller.rebaseStartedContinueAfterResolvingAnyConflicts") : t("workspace.controller.rebaseCompleted"));
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
    content: t("workspace.controller.abortTheCurrentOperationGitWillTryToRestoreTheStateFromBeforeItStartedUnresolvedConflictWorkMayB",
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

function openWorktreeModal(): void {
  worktreeForm.path = "";
  worktreeForm.mode = "new";
  worktreeForm.branch = "";
  worktreeModalShown.value = true;
}

async function addWorktree(): Promise<void> {
  if (!activeRepositoryPath.value || !worktreeForm.path.trim() || !worktreeForm.branch.trim()) {
    message.warning(
      worktreeForm.mode === "new"
        ? t("workspace.controller.enterANewWorktreeFolderAndBranchName")
        : t("workspace.controller.enterAWorktreeFolderAndSelectAnExistingBranch"),
    );
    return;
  }
  mutationBusy.value = true;
  try {
    worktrees.value = await api.addWorktree(
      activeRepositoryPath.value,
      worktreeForm.path.trim(),
      worktreeForm.mode === "new" ? worktreeForm.branch.trim() : undefined,
      worktreeForm.mode === "existing" ? worktreeForm.branch.trim() : undefined,
    );
    worktreeModalShown.value = false;
    await loadRepositories();
    message.success(t("workspace.controller.worktreeCreatedYouCanOpenItFromTheList"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotCreateWorktree"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmRemoveWorktree(worktree: WorktreeInfo): void {
  if (!activeRepositoryPath.value || worktree.isMain || worktree.path === snapshot.value?.repository.path) return;
  dialog.error({
    title: t("workspace.controller.removeWorktree"),
    content: t("workspace.controller.removeWorktreePathItsDirectoryAndUncommittedChangesWillBeDeletedGitRejectsTheOperationIfTheWorkt",
      { path: worktree.path },
    ),
    positiveText: t("workspace.controller.removeWorktree"),
    negativeText: t("workspace.controller.cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        worktrees.value = await api.removeWorktree(activeRepositoryPath.value!, worktree.path, false);
        await loadRepositories();
        message.success(t("workspace.controller.worktreeRemoved"));
      } catch (error) {
        reportError(error, t("workspace.controller.couldNotRemoveWorktree"));
      } finally {
        mutationBusy.value = false;
      }
    },
  });
}

function confirmUpdateSubmodules(): void {
  if (!activeRepositoryPath.value) return;
  dialog.warning({
    title: t("workspace.controller.initializeOrUpdateSubmodules"),
    content: t("workspace.controller.initializeOrUpdateAllSubmodulesRecursivelyUsingThisRepositorySCurrentConfigurationThisMayDownloa",
    ),
    positiveText: t("workspace.controller.initializeAndUpdate"),
    negativeText: t("workspace.controller.cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        submodules.value = await api.updateSubmodules(activeRepositoryPath.value!, true, true);
        message.success(t("workspace.controller.submodulesInitializedOrUpdated"));
      } catch (error) {
        reportError(error, t("workspace.controller.couldNotUpdateSubmodules"));
      } finally {
        mutationBusy.value = false;
      }
    },
  });
}

async function initializeLfs(): Promise<void> {
  if (!activeRepositoryPath.value || !lfsStatus.value?.available) return;
  mutationBusy.value = true;
  try {
    lfsStatus.value = await api.installLfs(activeRepositoryPath.value);
    message.success(t("workspace.controller.gitLfsInitializedForThisRepository"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotInitializeGitLfs"));
  } finally {
    mutationBusy.value = false;
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
  mutationBusy.value = true;
  try {
    applySnapshot(await api.trackLfsPattern(activeRepositoryPath.value, lfsTrackForm.pattern.trim()));
    lfsTrackModalShown.value = false;
    await loadLfsStatus();
    message.success(t("workspace.controller.theGitLfsRuleWasWrittenToGitattributesReviewAndCommitThatChange"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotAddGitLfsRule"));
  } finally {
    mutationBusy.value = false;
  }
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
    applySnapshot(
      await api.stashChanges(
        activeRepositoryPath.value,
        stashForm.message.trim() || undefined,
        stashForm.includeUntracked,
      ),
    );
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
    content: t("workspace.controller.applyReferenceToTheCurrentWorkingTreeAndRemoveItFromTheStashIfConflictsOccurGitLeavesTheWorkingT",
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
    content: t("workspace.controller.permanentlyDeleteReferenceThisCannotBeUndone",
      { reference: stash.reference },
    ),
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
  const modeLabel =
    mode === "soft"
      ? t("workspace.controller.softKeepIndexAndWorkingTree")
      : mode === "mixed"
        ? t("workspace.controller.mixedKeepWorkingTreeUnstageChanges")
        : t("workspace.controller.hardDiscardIndexAndWorkingTreeChanges");
  dialog.error({
    title: t("workspace.controller.resetBranch"),
    content: t("workspace.controller.resetTheCurrentBranchToCommitModeAHardResetCanPermanentlyDiscardUncommittedChanges",
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
    content: t("workspace.controller.createANewCommitThatRevertsCommitThisDoesNotRewriteExistingHistory",
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

async function saveAccount(): Promise<void> {
  if (accountUsesPat.value && !accountFormCapabilities.value.patKeychain) {
    message.warning(t("workspace.controller.thisEndpointDoesNotSupportAnAppManagedPatChooseASupportedAuthenticationMethod"));
    return;
  }
  if (!accountForm.login.trim() || (accountUsesPat.value && !accountForm.token.trim())) {
    message.warning(accountUsesPat.value ? t("workspace.controller.enterAnAccountLoginAndPersonalAccessToken") : t("workspace.controller.enterTheAccountLogin"));
    return;
  }
  // Read the token once. It is immediately cleared from reactive UI state and is never logged or displayed.
  const token = accountForm.token;
  accountForm.token = "";
  mutationBusy.value = true;
  try {
    const account = accountUsesPat.value
      ? await api.savePatAccount({
          endpointId: accountForm.endpointId,
          login: accountForm.login.trim(),
          displayName: accountForm.displayName.trim() || undefined,
          token,
        })
      : await api.saveAccountProfile({
          endpointId: accountForm.endpointId,
          login: accountForm.login.trim(),
          displayName: accountForm.displayName.trim() || undefined,
          authMethod: accountForm.authMethod as Exclude<AuthenticationMethod, "personalAccessToken">,
        });
    accounts.value = [...accounts.value, account];
    accountModalShown.value = false;
    accountForm.login = "";
    accountForm.displayName = "";
    message.success(
      accountUsesPat.value
        ? t("workspace.controller.accountMetadataWasSavedAndThePatWasSentToTheSystemKeychainItWillNotBeShownAgain")
        : t("workspace.controller.accountProfileSavedNetworkGitOperationsWillUseThisMethodAccordingToTheRemoteBinding"),
    );
    void refreshAccountStatuses(true);
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotAddAccount"));
  } finally {
    accountForm.token = "";
    mutationBusy.value = false;
  }
}

function openCustomEndpointModal(): void {
  endpointForm.id = "";
  endpointForm.kind = "genericGit";
  endpointForm.displayName = "";
  endpointForm.webUrl = "";
  endpointForm.apiUrl = "";
  endpointForm.gitHost = "";
  customEndpointModalShown.value = true;
}

async function saveCustomEndpoint(): Promise<void> {
  const requiresPlatformUrls = endpointForm.kind !== "genericGit";
  if (
    !endpointForm.id.trim() ||
    !endpointForm.displayName.trim() ||
    !endpointForm.gitHost.trim() ||
    (requiresPlatformUrls && (!endpointForm.webUrl.trim() || !endpointForm.apiUrl.trim()))
  ) {
    message.warning(t("workspace.controller.completeTheRequiredGitServiceEndpointDetails"));
    return;
  }
  mutationBusy.value = true;
  try {
    const endpoint = await api.saveCustomEndpoint({
      id: endpointForm.id.trim(),
      kind: endpointForm.kind as CodeHostEndpoint["kind"],
      displayName: endpointForm.displayName.trim(),
      webUrl: endpointForm.webUrl.trim(),
      apiUrl: endpointForm.apiUrl.trim(),
      gitHost: endpointForm.gitHost.trim().toLowerCase(),
      isBuiltin: false,
    });
    const index = endpoints.value.findIndex((item) => item.id === endpoint.id);
    endpoints.value = index === -1 ? [...endpoints.value, endpoint] : endpoints.value.map((item, itemIndex) => (itemIndex === index ? endpoint : item));
    customEndpointModalShown.value = false;
    message.success(t("workspace.controller.gitServiceEndpointSavedYouCanNowSelectItForARemote"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotSaveGitServiceEndpoint"));
  } finally {
    mutationBusy.value = false;
  }
}

async function importCliAccounts(endpointId: string): Promise<void> {
  if (!capabilityForEndpoint(endpointId).accountImport) return;
  mutationBusy.value = true;
  try {
    const imported = await api.importCliAccounts(endpointId);
    await loadAccounts();
    message.success(imported.length ? t("workspace.controller.detectedCountCliAccountS", { count: imported.length }) : t("workspace.controller.noCliAccountsAreAvailableToImport"));
} catch (error) {
    reportError(error, t("workspace.controller.couldNotDetectCliAccounts"));
  } finally {
    mutationBusy.value = false;
  }
}

async function setDefaultCandidate(account: AccountProfile): Promise<void> {
  mutationBusy.value = true;
  try {
    accounts.value = await api.setDefaultAccount(account.id, !account.isDefaultCandidate);
    message.success(account.isDefaultCandidate ? t("workspace.controller.defaultCandidateRemoved") : t("workspace.controller.setAsDefaultCandidateItDoesNotOverrideAnExplicitRemoteBinding"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotUpdateDefaultCandidate"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmRemoveAccount(account: AccountProfile): void {
  dialog.warning({
    title: t("workspace.controller.removeAccount"),
    content: t("workspace.controller.removeTheLocalProfileForAccountThisRemovesTheAppSKeychainReferenceButNeverChangesGlobalGitGhOrGl",
      { account: account.displayName || account.login },
    ),
    positiveText: t("workspace.controller.remove"),
    negativeText: t("workspace.controller.cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        await api.removeAccount(account.id);
        accounts.value = accounts.value.filter((item) => item.id !== account.id);
        message.success(t("workspace.controller.accountRemoved"));
      } catch (error) {
        reportError(error, t("workspace.controller.couldNotRemoveAccount"));
      } finally {
        mutationBusy.value = false;
      }
    },
  });
}

async function loadRepositoryInfo(): Promise<void> {
  if (!activeRepositoryPath.value || !activeRemote.value || !activeCapabilities.value.repositoryInfo) return;
  platformLoading.value = true;
  try {
    repositoryInfo.value = await api.getCodeHostRepositoryInfo(activeRepositoryPath.value, activeRemote.value.name);
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotLoadGitServiceRepositoryInformation"));
  } finally {
    platformLoading.value = false;
  }
}

async function loadPullRequests(): Promise<void> {
  if (!activeRepositoryPath.value || !activeRemote.value || !activeCapabilities.value.pullRequestsRead) return;
  pullRequestsLoading.value = true;
  try {
    pullRequests.value = await api.listPullRequests(activeRepositoryPath.value, activeRemote.value.name);
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotLoadPullRequests"));
  } finally {
    pullRequestsLoading.value = false;
  }
}

function openCreatePullRequest(): void {
  const suggestedBase = repositoryInfo.value?.defaultBranch || pullRequestBaseOptions.value[0]?.value || "";
  pullRequestForm.title = selectedCommit.value?.summary || commits.value[0]?.summary || "";
  pullRequestForm.body = "";
  pullRequestForm.base = suggestedBase;
  pullRequestForm.draft = false;
  pullRequestModalShown.value = true;
}

async function createPullRequest(): Promise<void> {
  if (!activeRepositoryPath.value || !activeRemote.value || !snapshot.value?.status.branch) {
    message.warning(t("workspace.controller.switchToALocalBranchBeforeCreatingAPullRequest"));
    return;
  }
  if (!pullRequestForm.title.trim() || !pullRequestForm.base.trim()) {
    message.warning(t("workspace.controller.enterAPullRequestTitleAndChooseABaseBranch"));
    return;
  }
  if (pullRequestForm.base.trim() === snapshot.value.status.branch) {
    message.warning(t("workspace.controller.theHeadBranchAndBaseBranchCannotBeTheSame"));
    return;
  }
  mutationBusy.value = true;
  try {
    const pullRequest = await api.createPullRequest(activeRepositoryPath.value, activeRemote.value.name, {
      title: pullRequestForm.title.trim(),
      body: pullRequestForm.body,
      head: snapshot.value.status.branch,
      base: pullRequestForm.base.trim(),
      draft: pullRequestForm.draft,
    });
    pullRequests.value = [pullRequest, ...pullRequests.value.filter((item) => item.number !== pullRequest.number)];
    pullRequestModalShown.value = false;
    message.success(
      t("workspace.controller.pullRequestNumberCreated",
        { number: pullRequest.number },
      ),
    );
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotCreatePullRequest"));
  } finally {
    mutationBusy.value = false;
  }
}

function fallbackPublishOwner(): GitHubPublishOwner | null {
  const account = publishAccounts.value.find((item) => item.id === publishForm.accountId);
  return account
    ? {
        login: account.login,
        name: account.displayName,
        kind: "user",
        canCreateRepositories: true,
      }
    : null;
}

async function loadPublishOwners(): Promise<void> {
  const requestGeneration = ++publishOwnersRequestGeneration;
  const fallback = fallbackPublishOwner();
  publishOwners.value = fallback ? [fallback] : [];
  publishOwnersError.value = "";
  if (!publishForm.endpointId || !publishForm.accountId) {
    publishOwnersLoading.value = false;
    publishForm.ownerLogin = "";
    return;
  }
  if (!publishOwners.value.some((owner) => owner.login === publishForm.ownerLogin)) {
    publishForm.ownerLogin = fallback?.login || "";
  }
  publishOwnersLoading.value = true;
  try {
    const owners = await api.listGitHubPublishOwners(publishForm.endpointId, publishForm.accountId);
    if (requestGeneration !== publishOwnersRequestGeneration) return;
    publishOwners.value = owners;
    if (!owners.some((owner) => owner.login === publishForm.ownerLogin && owner.canCreateRepositories)) {
      publishForm.ownerLogin = owners.find((owner) => owner.canCreateRepositories)?.login || "";
    }
  } catch (error) {
    if (requestGeneration !== publishOwnersRequestGeneration) return;
    const appError: AppError = isAppError(error)
      ? error
      : { code: "unexpected_error", message: t("workspace.controller.anUnexpectedErrorOccurred"), details: null };
    publishOwnersError.value = localizedErrorMessage(appError.code);
  } finally {
    if (requestGeneration === publishOwnersRequestGeneration) publishOwnersLoading.value = false;
  }
}

function openPublishModal(): void {
  publishForm.endpointId = publishEndpoints.value[0]?.id || "github.com";
  publishForm.accountId = publishAccounts.value[0]?.id || "";
  publishForm.ownerLogin = publishAccounts.value[0]?.login || "";
  publishForm.name = snapshot.value?.repository.name || "";
  publishForm.isPrivate = true;
  publishModalShown.value = true;
}

async function publishGitHubRepository(): Promise<void> {
  if (!activeRepositoryPath.value || !publishForm.endpointId || !publishForm.accountId || !canPublishToSelectedOwner.value || !publishForm.name.trim()) {
    message.warning(t("workspace.controller.selectAnEligibleOwnerAndEnterAGithubRepositoryName"));
    return;
  }
  if (publishForm.name.trim().includes("/")) {
    message.warning(t("workspace.controller.enterOnlyTheRepositoryNameChooseItsOwnerAbove"));
    return;
  }
  mutationBusy.value = true;
  try {
    applySnapshot(
      await api.publishGitHubRepository(
        activeRepositoryPath.value,
        publishForm.endpointId,
        publishForm.accountId,
        publishForm.ownerLogin,
        publishForm.name.trim(),
        publishForm.isPrivate,
      ),
    );
    publishModalShown.value = false;
    await loadCapabilities();
    message.success(t("workspace.controller.repositoryPublishedToGithubAndOriginConfigured"));
  } catch (error) {
    reportError(error, t("workspace.controller.couldNotPublishGithubRepository"));
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
    publishOwnersRequestGeneration += 1;
    publishOwners.value = [];
    publishOwnersError.value = "";
    publishOwnersLoading.value = false;
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
      publishOwnersRequestGeneration += 1;
      publishOwnersLoading.value = false;
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
  if (commitFilesDebounceTimer) clearTimeout(commitFilesDebounceTimer);
  if (commitDiffDebounceTimer) clearTimeout(commitDiffDebounceTimer);
  commitFilesDebounceTimer = null;
  commitDiffDebounceTimer = null;
  pendingCommitDiffRequest = null;
  commitFilesRequestGeneration += 1;
  commitDiffRequestGeneration += 1;
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
