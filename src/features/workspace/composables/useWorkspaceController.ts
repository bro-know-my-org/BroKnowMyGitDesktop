import { computed, h, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { NButton, useDialog, useMessage } from "naive-ui";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, builtinEndpoints, isAppError, isBrowserDemo, toggleDevtools } from "../../../shared/api/tauri";
import { appLocale, localizedErrorMessage, setAppLocale, tr, type AppLocale } from "../../../shared/i18n";
import { colorMode, resetThemeColor, setColorMode, setThemeColor, themeColor, type ColorMode } from "../../../shared/theme";
import {
  authenticationMethodLabel,
  authenticationStatusLabel,
  endpointKindLabel,
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
}

interface RefreshSnapshotOptions {
  includeAuxiliary?: boolean;
  reportErrors?: boolean;
  silent?: boolean;
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
} = createRepositoryPresentation(tr);
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
  { label: tr("跟随系统", "Follow system"), key: "system" },
  { label: tr("浅色", "Light"), key: "light" },
  { label: tr("深色", "Dark"), key: "dark" },
]);
const repositoryActionOptions = computed(() => [
  { label: tr("扫描文件夹", "Scan folder"), key: "discover" },
  { label: tr("新建本地仓库", "New local repository"), key: "init" },
  { label: tr("Clone 仓库", "Clone repository"), key: "clone" },
]);

const section = ref<AppSection>("repositories");
const activeTab = ref<RepositoryTab>("changes");
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
let automaticRefreshDirty = false;
let automaticRefreshInFlight = false;
let windowFocused = typeof document === "undefined" || document.hasFocus();
let controllerMounted = false;
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
  if (amendLastCommit.value) return tr("修改最近一次提交", "Amend last commit");
  if (!stagedCount.value) return tr("选择要提交的文件", "Select files to commit");
  return tr(
    "提交 {count} 个文件到 {branch}",
    stagedCount.value === 1 ? "Commit 1 file to {branch}" : "Commit {count} files to {branch}",
    { count: stagedCount.value, branch: currentBranch.value },
  );
});
const commitSelectionSummary = computed(() => {
  if (amendLastCommit.value) {
    return tr(
      "会用此说明重写最近一次提交，并包含当前已加入的 {count} 个文件。",
      "This rewrites the latest commit and includes the {count} file(s) currently selected.",
      { count: stagedCount.value },
    );
  }
  return tr(
    "已选择 {count} 个文件；Git hooks 将照常运行。",
    stagedCount.value === 1 ? "1 file selected; Git hooks run as usual." : "{count} files selected; Git hooks run as usual.",
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
    label: (account.displayName || account.login) + " · " + authenticationStatusLabel(account.status, appLocale.value),
    value: account.id,
  })),
);
const publishOwnerOptions = computed(() =>
  publishOwners.value.map((owner) => ({
    label: (owner.name
      ? `${owner.name} (${owner.login}) · ${owner.kind === "organization" ? tr("组织", "Organization") : tr("个人", "Personal")}`
      : `${owner.login} · ${owner.kind === "organization" ? tr("组织", "Organization") : tr("个人", "Personal")}`)
      + (owner.canCreateRepositories ? "" : tr(" · 无创建权限", " · No create permission")),
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
  { label: tr("个人访问令牌 (PAT)", "Personal access token (PAT)"), value: "personalAccessToken", disabled: !accountFormCapabilities.value.patKeychain },
  { label: "SSH agent", value: "sshAgent" },
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
  { label: tr("自动选择（唯一 PAT 候选；否则系统 Git）", "Auto-select (one PAT candidate; otherwise system Git)"), value: "auto" },
  ...bindingAccounts.value.map((account) => ({
    label: `${account.displayName || account.login} · ${authenticationStatusLabel(account.status, appLocale.value)}`,
    value: account.id,
    disabled: account.status === "needsReauthentication" || account.status === "unavailable",
  })),
]);
const bindingEndpointOptions = computed(() => {
  const options = endpoints.value.map((endpoint) => ({ label: endpoint.displayName, value: endpoint.id }));
  const remoteEndpointId = bindingRemote.value?.endpointId;
  if (remoteEndpointId && !options.some((option) => option.value === remoteEndpointId)) {
    options.push({ label: `${remoteEndpointId} (${tr("手动关联", "manually associated")})`, value: remoteEndpointId });
  }
  return options;
});
const cloneAccounts = computed(() => accounts.value.filter(
  (account) => account.endpointId === cloneForm.endpointId && supportsCloneCredentials(account),
));
const cloneAccountOptions = computed(() => [
  { label: tr("使用系统 Git / 不指定账号", "Use system Git / no app account"), value: "none" },
  ...(cloneForm.endpointId
    ? [
        { label: tr("自动选择（仅唯一可用候选时）", "Auto-select (only one usable candidate)"), value: "auto", disabled: !cloneAccounts.value.some((account) => account.status === "valid" || account.status === "available") },
        ...cloneAccounts.value.map((account) => ({
          label: `${account.displayName || account.login} · ${authenticationStatusLabel(account.status, appLocale.value)}`,
          value: account.id,
          disabled: account.status === "needsReauthentication" || account.status === "unavailable",
        })),
      ]
    : []),
]);
const tagTargetOptions = computed(() => {
  const options = [
    { label: tr("当前 HEAD", "Current HEAD"), value: "" },
    ...(snapshot.value?.branches.map((branch) => ({ label: branch.name, value: branch.name })) ?? []),
  ];
  const target = tagForm.target.trim();
  if (target && !options.some((option) => option.value === target)) {
    options.push({
      label: tr("提交 {sha}", "Commit {sha}", { sha: target.slice(0, 12) }),
      value: target,
    });
  }
  return options;
});
const repositoryGroupingOptions = computed(() => [
  { label: tr("手动分组", "Manual groups"), value: "manual" },
  { label: tr("远端组织 / 用户", "Remote owner"), value: "owner" },
  { label: tr("最近使用", "Recently used"), value: "recent" },
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
      ? [...namedGroups, { key: "manual:ungrouped", label: tr("未分组", "Ungrouped"), repositories: ungrouped }]
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
      ? [...ownerGroups, { key: "owner:unknown", label: tr("未识别所有者", "Unrecognized owner"), repositories: unknown }]
      : ownerGroups;
  }

  const now = Date.now();
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const todayStart = today.getTime();
  const weekStart = now - 7 * 24 * 60 * 60 * 1000;
  const buckets: RepositorySidebarGroup[] = [
    { key: "recent:today", label: tr("今天", "Today"), repositories: [] },
    { key: "recent:week", label: tr("最近 7 天", "Last 7 days"), repositories: [] },
    { key: "recent:older", label: tr("更早", "Earlier"), repositories: [] },
    { key: "recent:never", label: tr("尚未打开", "Not opened yet"), repositories: [] },
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
  tr,
  appLocale,
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
  if (!binding) return tr("自动选择", "Auto-select");
  const selection = binding.accountSelection;
  if (selection.mode === "auto") return tr("自动选择", "Auto-select");
  const account = accounts.value.find((item) => item.id === selection.accountProfileId);
  return account
    ? `${account.displayName || account.login} · ${authenticationMethodLabel(account.authMethod, appLocale.value)}`
    : tr("已绑定的账号不可用", "The bound account is unavailable");
}

function remoteAuthenticationLabel(remote: RemoteProfile): string {
  const binding = getBinding(remote.name);
  const selection = binding?.accountSelection;
  if (!selection || selection.mode === "auto") return tr("Auto：操作前解析可用账号", "Auto: resolve a usable account before the operation");
  const account = accounts.value.find((item) => item.id === selection.accountProfileId);
  if (!account) return tr("绑定账号已移除", "The bound account was removed");
  return `${authenticationMethodLabel(account.authMethod, appLocale.value)} · ${authenticationStatusLabel(account.status, appLocale.value)}`;
}

function reportError(error: unknown, action: string): void {
  const appError: AppError = isAppError(error)
    ? error
    : { code: "unexpected_error", message: tr("发生未预期错误。", "An unexpected error occurred."), details: null };
  appError.message = localizedErrorMessage(appError.code, appError.message);
  lastError.value = appError;
  lastErrorAction.value = action;
  message.error(tr("{action}：{message}", "{action}: {message}", { action, message: appError.message }));
  if (appError.code === "account_selection_required" && activeRemote.value) {
    message.warning(tr("此 remote 有多个或没有可用账号，请明确选择一个账号后重试。", "This remote has multiple or no usable accounts. Select one explicitly and retry."));
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

function repositorySelectionIsCurrent(repositoryPath: string, generation: number): boolean {
  return activeRepositoryPath.value === repositoryPath && repositorySelectionGeneration === generation;
}

function applySnapshot(nextSnapshot: RepositorySnapshot, invalidateRefreshRequests = true): boolean {
  // Every backend snapshot is scoped to one repository. Never let a late
  // mutation response overwrite the currently selected repository.
  if (activeRepositoryPath.value !== nextSnapshot.repository.path) return false;
  snapshotApplicationGeneration += 1;
  if (invalidateRefreshRequests) snapshotRefreshRequestGeneration += 1;
  const selectedPath = selectedChange.value?.path;
  snapshot.value = nextSnapshot;
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
    message.success(name ? tr("仓库分组已保存。", "Repository group saved.") : tr("仓库已移到未分组。", "Repository moved to ungrouped."));
  } catch (error) {
    reportError(error, tr("保存仓库分组失败", "Could not save repository group"));
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
    message.success(name ? tr("仓库别名已保存。", "Repository alias saved.") : tr("仓库别名已移除。", "Repository alias removed."));
  } catch (error) {
    reportError(error, tr("保存仓库别名失败", "Could not save repository alias"));
  } finally {
    mutationBusy.value = false;
  }
}

async function loadRepositories(refreshOwners = repositoryGrouping.value === "owner"): Promise<void> {
  repositoriesLoading.value = true;
  try {
    repositories.value = await api.listRepositories(refreshOwners);
  } catch (error) {
    reportError(error, tr("读取仓库列表失败", "Could not load repositories"));
  } finally {
    repositoriesLoading.value = false;
  }
}

async function loadAccounts(): Promise<void> {
  accountsLoading.value = true;
  try {
    accounts.value = await api.listAccounts();
  } catch (error) {
    reportError(error, tr("读取账号失败", "Could not load accounts"));
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
    if (force) reportError(error, tr("刷新账号认证状态失败", "Could not refresh account authentication status"));
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
    reportError(error, tr("读取代码托管 Endpoint 失败", "Could not load Git service endpoints"));
  }
}

function installRepositoryWatcher(repositoryPath: string): void {
  if (repositoryWatcherRetryTimer) clearTimeout(repositoryWatcherRetryTimer);
  repositoryWatcherRetryTimer = null;
  const requestGeneration = ++repositoryWatchRequestGeneration;
  void api.watchRepositoryChanges(repositoryPath, requestGeneration)
    .then(() => {
      if (
        controllerMounted &&
        repositoryWatchRequestGeneration === requestGeneration &&
        activeRepositoryPath.value === repositoryPath
      ) {
        repositoryWatcherRetryAttempt = 0;
        repositoryWatcherUnavailable = false;
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
          },
        });
      }
    });
}

async function selectRepository(repositoryPath: string): Promise<void> {
  if (repositoryWatcherRetryTimer) clearTimeout(repositoryWatcherRetryTimer);
  repositoryWatcherRetryTimer = null;
  repositoryWatcherRetryAttempt = 0;
  repositoryWatcherUnavailable = false;
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
    applySnapshot(nextSnapshot);
    installRepositoryWatcher(nextSnapshot.repository.path);
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
    await syncSelectedDiff(nextSnapshot, undefined, true);
    await Promise.all([loadCapabilities(), loadWorktrees(), loadSubmodules(), loadLfsStatus()]);
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      reportError(error, tr("打开仓库失败", "Could not open repository"));
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
    applySnapshot(nextSnapshot, false);
    const appliedApplicationGeneration = snapshotApplicationGeneration;
    if (selectedDiffGeneration === diffRequestGeneration) {
      await syncSelectedDiff(nextSnapshot, selectedPath, !selectedPath, repositoryPath, reportErrors);
    }
    if (
      includeAuxiliary &&
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      appliedApplicationGeneration === snapshotApplicationGeneration &&
      refreshGeneration === snapshotRefreshRequestGeneration
    ) {
      await Promise.all([
        loadWorktrees(reportErrors),
        loadSubmodules(reportErrors),
        loadLfsStatus(reportErrors),
      ]);
    }
  } catch (error) {
    if (
      reportErrors &&
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      applicationGeneration === snapshotApplicationGeneration &&
      refreshGeneration === snapshotRefreshRequestGeneration
    ) {
      reportError(error, tr("刷新仓库失败", "Could not refresh repository"));
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
  if (automaticRefreshInFlight || snapshotLoading.value || mutationBusy.value || networkBusy.value) {
    scheduleAutomaticRefresh(automaticRefreshRetryMs);
    return;
  }

  automaticRefreshDirty = false;
  automaticRefreshInFlight = true;
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
    automaticRefreshInFlight = false;
    if (automaticRefreshDirty) scheduleAutomaticRefresh();
  }
}

function handleRepositoryChanged(event: { payload: RepositoryChangedPayload }): void {
  if (
    event.payload.repositoryPath !== activeRepositoryPath.value ||
    event.payload.requestGeneration !== repositoryWatchRequestGeneration
  ) return;
  if (event.payload.historyObjectsChanged) {
    invalidateCommitHistoryCache();
    invalidateCommitDetailCaches();
  }
  if (event.payload.stashesChanged) invalidateStashesCache();
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
      installRepositoryWatcher(event.payload.repositoryPath);
    }
  }, retryDelay);
}

function handleWindowFocus(): void {
  windowFocused = true;
  if (repositoryWatcherUnavailable && activeRepositoryPath.value) {
    repositoryWatcherRetryAttempt = 0;
    installRepositoryWatcher(activeRepositoryPath.value);
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
    message.warning(tr("请输入要扫描的文件夹路径。", "Enter the folder path to scan."));
    return;
  }
  mutationBusy.value = true;
  try {
    const found = await api.discoverRepositories(discoverForm.rootPath.trim());
    const paths = new Set(repositories.value.map((repository) => repository.path));
    repositories.value = [...repositories.value, ...found.filter((repository) => !paths.has(repository.path))];
    discoverModalShown.value = false;
    discoverForm.rootPath = "";
    message.success(found.length ? tr("发现 {count} 个仓库。", "Found {count} repositories.", { count: found.length }) : tr("没有发现 Git 仓库。", "No Git repositories found."));
  } catch (error) {
    reportError(error, tr("扫描仓库失败", "Could not scan repositories"));
  } finally {
    mutationBusy.value = false;
  }
}

async function cloneRepository(): Promise<void> {
  if (!cloneForm.url.trim() || !cloneForm.destinationPath.trim()) {
    message.warning(tr("请输入远程地址和目标目录。", "Enter a remote URL and destination folder."));
    return;
  }
  if (cloneForm.accountId !== "none" && !cloneForm.endpointId) {
    message.warning(tr("选择应用账号时，请同时选择对应的 CodeHost Endpoint。", "When choosing an app account, also choose its Git service endpoint."));
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
    message.success(tr("仓库已 clone，正在打开。", "Repository cloned. Opening it now."));
    await selectRepository(repository.path);
  } catch (error) {
    reportError(error, tr("Clone 失败", "Clone failed"));
  } finally {
    mutationBusy.value = false;
  }
}

async function initRepository(): Promise<void> {
  if (!initForm.destinationPath.trim()) {
    message.warning(tr("请输入新仓库目录。", "Enter a destination folder for the new repository."));
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
    message.success(tr("本地 Git 仓库已创建，正在打开。", "Local Git repository created. Opening it now."));
    await selectRepository(repository.path);
  } catch (error) {
    reportError(error, tr("创建本地仓库失败", "Could not create local repository"));
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
      if (reportErrors) reportError(error, tr("读取差异失败", "Could not load diff"));
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
    reportError(error, tr("读取文件 Blame 失败", "Could not load file blame"));
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
    message.success(reverse ? tr("代码块已取消暂存。", "Hunk unstaged.") : tr("代码块已暂存。", "Hunk staged."));
  } catch (error) {
    reportError(error, reverse ? tr("取消暂存代码块失败", "Could not unstage hunk") : tr("暂存代码块失败", "Could not stage hunk"));
  } finally {
    mutationBusy.value = false;
  }
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
      if (reportErrors) reportError(error, tr("读取提交历史失败", "Could not load commit history"));
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
      if (reportErrors) reportError(error, tr("读取暂存栈失败", "Could not load stashes"));
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
    reportError(error, tr("读取 Reflog 失败", "Could not load reflog"));
  } finally {
    reflogLoading.value = false;
  }
}

async function loadWorktrees(reportErrors = true): Promise<void> {
  const repositoryPath = activeRepositoryPath.value;
  if (!repositoryPath) {
    worktrees.value = [];
    return;
  }
  const selectionGeneration = repositorySelectionGeneration;
  const requestGeneration = ++worktreesRequestGeneration;
  worktreesLoading.value = true;
  try {
    const nextWorktrees = await api.listWorktrees(repositoryPath);
    if (
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      requestGeneration === worktreesRequestGeneration
    ) worktrees.value = nextWorktrees;
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === worktreesRequestGeneration) {
      if (reportErrors) reportError(error, tr("读取工作树失败", "Could not load worktrees"));
    }
  } finally {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === worktreesRequestGeneration) worktreesLoading.value = false;
  }
}

async function loadSubmodules(reportErrors = true): Promise<void> {
  const repositoryPath = activeRepositoryPath.value;
  if (!repositoryPath) {
    submodules.value = [];
    return;
  }
  const selectionGeneration = repositorySelectionGeneration;
  const requestGeneration = ++submodulesRequestGeneration;
  submodulesLoading.value = true;
  try {
    const nextSubmodules = await api.listSubmodules(repositoryPath);
    if (
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      requestGeneration === submodulesRequestGeneration
    ) submodules.value = nextSubmodules;
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === submodulesRequestGeneration) {
      if (reportErrors) reportError(error, tr("读取子模块失败", "Could not load submodules"));
    }
  } finally {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === submodulesRequestGeneration) submodulesLoading.value = false;
  }
}

async function loadLfsStatus(reportErrors = true): Promise<void> {
  const repositoryPath = activeRepositoryPath.value;
  if (!repositoryPath) {
    lfsStatus.value = null;
    return;
  }
  const selectionGeneration = repositorySelectionGeneration;
  const requestGeneration = ++lfsRequestGeneration;
  lfsLoading.value = true;
  try {
    const nextLfsStatus = await api.getLfsStatus(repositoryPath);
    if (
      repositorySelectionIsCurrent(repositoryPath, selectionGeneration) &&
      requestGeneration === lfsRequestGeneration
    ) lfsStatus.value = nextLfsStatus;
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === lfsRequestGeneration) {
      if (reportErrors) reportError(error, tr("读取 Git LFS 状态失败", "Could not load Git LFS status"));
    }
  } finally {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration) && requestGeneration === lfsRequestGeneration) lfsLoading.value = false;
  }
}

async function loadExternalEditor(): Promise<void> {
  try {
    externalEditor.value = await api.getExternalEditor();
  } catch (error) {
    reportError(error, tr("读取外部编辑器配置失败", "Could not load external editor configuration"));
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
      reportError(error, tr("读取提交文件失败", "Could not load changed files"));
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
      reportError(error, tr("读取提交文件改动失败", "Could not load the file diff"));
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
    reportError(error, included ? tr("暂存失败", "Could not stage changes") : tr("取消暂存失败", "Could not unstage changes"));
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
    message.success(tr("已暂存全部工作区改动。", "All working-tree changes are staged."));
  } catch (error) {
    reportError(error, tr("暂存失败", "Could not stage changes"));
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
    message.success(tr("已将全部文件移出暂存区。", "All files are removed from the staging area."));
  } catch (error) {
    reportError(error, tr("取消暂存失败", "Could not unstage changes"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmDiscardChanges(paths: string[]): void {
  if (!activeRepositoryPath.value || !paths.length) return;
  const uniquePaths = [...new Set(paths)];
  const includesUntracked = snapshot.value?.status.entries.some(
    (entry) => uniquePaths.includes(entry.path) && !entry.indexStatus && entry.worktreeStatus === "?",
  );
  dialog.error({
    title: tr("丢弃改动", "Discard changes"),
    content: tr(
      "永久丢弃 {count} 个文件的工作区和暂存区改动？此操作不能撤销。{untracked}",
      "Permanently discard working-tree and staged changes in {count} file(s)? This cannot be undone.{untracked}",
      {
        count: uniquePaths.length,
        untracked: includesUntracked
          ? tr("其中的未跟踪文件会从磁盘删除。", " Untracked files among them will be deleted from disk.")
          : "",
      },
    ),
    positiveText: tr("丢弃改动", "Discard changes"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        applySnapshot(await api.discardChanges(activeRepositoryPath.value!, uniquePaths));
        if (selectedChange.value && uniquePaths.includes(selectedChange.value.path)) {
          clearDiffSelection();
        }
        message.success(tr("改动已丢弃。", "Changes discarded."));
      } catch (error) {
        reportError(error, tr("丢弃改动失败", "Could not discard changes"));
      } finally {
        mutationBusy.value = false;
      }
    },
  });
}

function discardSelectedChange(): void {
  if (selectedChange.value) confirmDiscardChanges([selectedChange.value.path]);
}

async function confirmCleanUntracked(): Promise<void> {
  if (!activeRepositoryPath.value) return;
  mutationBusy.value = true;
  let cleanPreview: Awaited<ReturnType<typeof api.previewCleanUntracked>>;
  try {
    cleanPreview = await api.previewCleanUntracked(activeRepositoryPath.value);
  } catch (error) {
    reportError(error, tr("预览未跟踪文件清理失败", "Could not preview untracked file cleanup"));
    return;
  } finally {
    mutationBusy.value = false;
  }
  const entries = cleanPreview.preview.split(/\r?\n/u).filter(Boolean);
  if (!entries.length) {
    message.info(tr("没有可清理的未跟踪文件。", "There are no untracked files to clean."));
    return;
  }
  const count = entries.length;
  dialog.error({
    title: tr("清理未跟踪文件", "Clean untracked files"),
    content: tr(
      "永久删除 {count} 个未跟踪的文件或目录？已忽略的文件不会删除，此操作不能撤销。",
      "Permanently delete {count} untracked file(s) or folder(s)? Ignored files are not removed and this cannot be undone.",
      { count },
    ),
    positiveText: tr("永久删除", "Delete permanently"),
    negativeText: tr("取消", "Cancel"),
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
        message.success(tr("未跟踪文件已清理。", "Untracked files cleaned."));
      } catch (error) {
        reportError(error, tr("清理未跟踪文件失败", "Could not clean untracked files"));
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
    message.warning(tr("请输入外部编辑器的绝对可执行文件路径。", "Enter the external editor's absolute executable path."));
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
    message.success(tr("外部编辑器已保存。", "External editor saved."));
  } catch (error) {
    reportError(error, tr("保存外部编辑器失败", "Could not save external editor"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmClearExternalEditor(): void {
  dialog.warning({
    title: tr("清除外部编辑器", "Clear external editor"),
    content: tr(
      "清除应用保存的外部编辑器配置？这不会修改系统、Git 或编辑器本身的设置。",
      "Clear the external editor configuration saved by this app? This does not change system, Git, or editor settings.",
    ),
    positiveText: tr("清除", "Clear"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        externalEditor.value = await api.saveExternalEditor(null);
        externalEditorModalShown.value = false;
        message.success(tr("外部编辑器配置已清除。", "External editor configuration cleared."));
      } catch (error) {
        reportError(error, tr("清除外部编辑器失败", "Could not clear external editor"));
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
      tr(
        "已在外部编辑器中打开 {count} 个冲突文件。完成后回到这里刷新、暂存并继续操作。",
        "Opened {count} conflicted file(s) in the external editor. When finished, return here to refresh, stage, and continue.",
        { count: paths.length },
      ),
    );
  } catch (error) {
    reportError(error, tr("打开外部编辑器失败", "Could not open external editor"));
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
    message.warning(tr("当前分支还没有可修改的提交。", "This branch has no commit to amend yet."));
    return;
  }
  amendLastCommit.value = true;
  await loadHistory();
  if (!commitMessage.value.trim() && commits.value[0]) commitMessage.value = commits.value[0].summary;
}

async function commitChanges(): Promise<void> {
  if (!activeRepositoryPath.value) return;
  if (!commitMessage.value.trim()) {
    message.warning(tr("请输入提交说明。", "Enter a commit message."));
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
    message.success(amended ? tr("最近一次提交已修改。", "Latest commit amended.") : tr("提交已创建。", "Commit created."));
  } catch (error) {
    reportError(error, tr("提交失败", "Commit failed"));
  } finally {
    mutationBusy.value = false;
  }
}

function requestBranchSwitch(
  target: string,
  execute: (stashLocalChanges: boolean) => Promise<void>,
): void {
  if (snapshot.value?.operation || conflictedChanges.value.length) {
    message.warning(tr("请先完成或中止当前 Git 操作，再切换分支。", "Finish or abort the current Git operation before switching branches."));
    return;
  }
  const changeCount = snapshot.value?.status.entries.length ?? 0;
  if (!changeCount) {
    void execute(false);
    return;
  }
  const branchSwitchDialog = dialog.warning({
    title: tr("切换分支前暂存改动？", "Stash changes before switching?"),
    content: tr(
      "当前有 {count} 个未提交改动，即将切换到 {branch}。暂存后切换会保存已暂存、未暂存和未跟踪文件；直接切换会保留它们，但若会覆盖文件，Git 会拒绝切换。",
      "There are {count} uncommitted change(s) before switching to {branch}. Stash and switch saves staged, unstaged, and untracked files; switch directly keeps them, but Git refuses if a file would be overwritten.",
      { count: changeCount, branch: target },
    ),
    closable: true,
    action: () => [
      h(
        NButton,
        { onClick: () => branchSwitchDialog.destroy() },
        { default: () => tr("取消", "Cancel") },
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
        { default: () => tr("直接切换", "Switch directly") },
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
        { default: () => tr("暂存后切换", "Stash and switch") },
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
        ? tr("改动已暂存，并切换到 {branch}。", "Changes stashed and switched to {branch}.", { branch: branch.name })
        : tr("已切换到 {branch}。", "Switched to {branch}.", { branch: branch.name }),
    );
  } catch (error) {
    await reconcileSnapshotAfterMutationError(repositoryPath);
    if (stashLocalChanges && isAppError(error) && error.code === "branch_switch_changes_stashed") {
      await loadHistory().catch(() => undefined);
    }
    reportError(error, tr("切换分支失败", "Could not switch branch"));
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
        ? tr("改动已暂存，并切换到 {branch}。", "Changes stashed and switched to {branch}.", { branch: branchLabel })
        : tr("已切换到 {branch}。", "Switched to {branch}.", { branch: branchLabel }),
    );
  } catch (error) {
    await reconcileSnapshotAfterMutationError(repositoryPath);
    if (stashLocalChanges && isAppError(error) && error.code === "branch_switch_changes_stashed") {
      await loadHistory().catch(() => undefined);
    }
    reportError(error, tr("切换远程分支失败", "Could not switch remote branch"));
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
    message.warning(tr("请输入分支名称。", "Enter a branch name."));
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
    message.success(tr("分支已创建。", "Branch created."));
  } catch (error) {
    await reconcileSnapshotAfterMutationError(repositoryPath);
    reportError(error, tr("创建分支失败", "Could not create branch"));
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
    message.warning(tr("请输入新的分支名称。", "Enter a new branch name."));
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
    message.success(tr("分支已重命名。", "Branch renamed."));
  } catch (error) {
    reportError(error, tr("重命名分支失败", "Could not rename branch"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmDeleteBranch(branch: BranchInfo): void {
  if (!activeRepositoryPath.value || branch.isCurrent) return;
  dialog.warning({
    title: tr("删除分支", "Delete branch"),
    content: tr(
      "删除本地分支 {branch}？未合并的分支会被 Git 拒绝，以防止意外丢失提交。",
      "Delete local branch {branch}? Git will reject an unmerged branch to prevent accidental loss of commits.",
      { branch: branch.name },
    ),
    positiveText: tr("删除", "Delete"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        applySnapshot(await api.deleteBranch(activeRepositoryPath.value!, branch.name, false));
        message.success(tr("分支已删除。", "Branch deleted."));
      } catch (error) {
        reportError(error, tr("删除分支失败", "Could not delete branch"));
      } finally {
        mutationBusy.value = false;
      }
    },
  });
}

function confirmMergeBranch(branch: BranchInfo): void {
  if (!activeRepositoryPath.value) return;
  dialog.warning({
    title: tr("合并分支", "Merge branch"),
    content: tr(
      "将 {branch} 合并到当前分支 {current}？如有冲突，Git 会保留工作区供你处理。",
      "Merge {branch} into the current branch {current}? If conflicts occur, Git leaves the working tree available for resolution.",
      { branch: branch.name, current: currentBranch.value },
    ),
    positiveText: tr("合并", "Merge"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        await api.mergeBranch(activeRepositoryPath.value!, branch.name);
        await refreshSnapshot();
        message.success(tr("已合并 {branch}。", "Merged {branch}.", { branch: branch.name }));
      } catch (error) {
        reportError(error, tr("合并分支失败", "Could not merge branch"));
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
    message.warning(tr("请选择要变基到的本地分支。", "Select a local branch to rebase onto."));
    return;
  }
  mutationBusy.value = true;
  try {
    applySnapshot(await api.startRebase(activeRepositoryPath.value, rebaseForm.base));
    rebaseModalShown.value = false;
    await loadHistory();
    message.success(snapshot.value?.operation ? tr("变基已开始；完成冲突处理后可继续。", "Rebase started; continue after resolving any conflicts.") : tr("变基已完成。", "Rebase completed."));
  } catch (error) {
    reportError(error, tr("变基失败", "Could not rebase branch"));
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
    message.warning(tr("请输入要拣选的提交 SHA 或引用。", "Enter a commit SHA or revision to cherry-pick."));
    return;
  }
  mutationBusy.value = true;
  try {
    applySnapshot(await api.cherryPickCommits(activeRepositoryPath.value, [cherryPickForm.revision.trim()]));
    cherryPickModalShown.value = false;
    await loadHistory();
    message.success(tr("提交已拣选。", "Commit cherry-picked."));
  } catch (error) {
    reportError(error, tr("拣选提交失败", "Could not cherry-pick commit"));
    await refreshAfterGitConflict(error);
  } finally {
    mutationBusy.value = false;
  }
}

async function continueGitOperation(): Promise<void> {
  if (!activeRepositoryPath.value || !snapshot.value?.operation) return;
  if (snapshot.value.operation.hasConflicts || conflictedChanges.value.length) {
    message.warning(tr("请先解决并暂存全部冲突文件。", "Resolve and stage all conflicted files first."));
    return;
  }
  const operation = snapshot.value.operation;
  mutationBusy.value = true;
  try {
    applySnapshot(await api.continueGitOperation(activeRepositoryPath.value, operation.kind));
    await loadHistory();
    message.success(tr("Git 操作已继续。", "Git operation continued."));
  } catch (error) {
    reportError(error, tr("继续 Git 操作失败", "Could not continue Git operation"));
    await refreshAfterGitConflict(error);
  } finally {
    mutationBusy.value = false;
  }
}

function confirmAbortGitOperation(): void {
  if (!activeRepositoryPath.value || !snapshot.value?.operation) return;
  const operation = snapshot.value.operation;
  dialog.error({
    title: tr("中止 Git 操作", "Abort Git operation"),
    content: tr(
      "中止当前 {operation}？Git 会尝试恢复该操作开始前的状态，冲突解决中的未提交改动可能会丢失。",
      "Abort the current {operation}? Git will try to restore the state from before it started; unresolved conflict work may be lost.",
      { operation: gitOperationLabel(operation.kind) },
    ),
    positiveText: tr("中止操作", "Abort operation"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        applySnapshot(await api.abortGitOperation(activeRepositoryPath.value!, operation.kind));
        await loadHistory();
        message.success(tr("Git 操作已中止。", "Git operation aborted."));
      } catch (error) {
        reportError(error, tr("中止 Git 操作失败", "Could not abort Git operation"));
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
        ? tr("请输入新工作树目录和新分支名称。", "Enter a new worktree folder and branch name.")
        : tr("请输入工作树目录并选择现有分支。", "Enter a worktree folder and select an existing branch."),
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
    message.success(tr("工作树已创建，可从列表打开。", "Worktree created. You can open it from the list."));
  } catch (error) {
    reportError(error, tr("创建工作树失败", "Could not create worktree"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmRemoveWorktree(worktree: WorktreeInfo): void {
  if (!activeRepositoryPath.value || worktree.isMain || worktree.path === snapshot.value?.repository.path) return;
  dialog.error({
    title: tr("移除工作树", "Remove worktree"),
    content: tr(
      "移除工作树 {path}？该目录及其中未提交的改动会被删除；若工作树不干净，Git 会拒绝此操作。",
      "Remove worktree {path}? Its directory and uncommitted changes will be deleted; Git rejects the operation if the worktree is not clean.",
      { path: worktree.path },
    ),
    positiveText: tr("移除工作树", "Remove worktree"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        worktrees.value = await api.removeWorktree(activeRepositoryPath.value!, worktree.path, false);
        await loadRepositories();
        message.success(tr("工作树已移除。", "Worktree removed."));
      } catch (error) {
        reportError(error, tr("移除工作树失败", "Could not remove worktree"));
      } finally {
        mutationBusy.value = false;
      }
    },
  });
}

function confirmUpdateSubmodules(): void {
  if (!activeRepositoryPath.value) return;
  dialog.warning({
    title: tr("初始化或更新子模块", "Initialize or update submodules"),
    content: tr(
      "按仓库当前配置递归初始化或更新全部子模块？这可能会从子模块远程仓库下载内容。",
      "Initialize or update all submodules recursively using this repository's current configuration? This may download content from submodule remotes.",
    ),
    positiveText: tr("初始化并更新", "Initialize and update"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        submodules.value = await api.updateSubmodules(activeRepositoryPath.value!, true, true);
        message.success(tr("子模块已初始化或更新。", "Submodules initialized or updated."));
      } catch (error) {
        reportError(error, tr("更新子模块失败", "Could not update submodules"));
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
    message.success(tr("Git LFS 已在此仓库初始化。", "Git LFS initialized for this repository."));
  } catch (error) {
    reportError(error, tr("初始化 Git LFS 失败", "Could not initialize Git LFS"));
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
    message.warning(tr("请输入要由 Git LFS 跟踪的文件模式。", "Enter a file pattern for Git LFS to track."));
    return;
  }
  mutationBusy.value = true;
  try {
    applySnapshot(await api.trackLfsPattern(activeRepositoryPath.value, lfsTrackForm.pattern.trim()));
    lfsTrackModalShown.value = false;
    await loadLfsStatus();
    message.success(tr("Git LFS 规则已写入 .gitattributes，请审查并提交该改动。", "The Git LFS rule was written to .gitattributes. Review and commit that change."));
  } catch (error) {
    reportError(error, tr("添加 Git LFS 规则失败", "Could not add Git LFS rule"));
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
    message.success(tr("改动已保存到暂存栈。", "Changes saved to stash."));
  } catch (error) {
    reportError(error, tr("保存暂存栈失败", "Could not stash changes"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmPopStash(stash: StashEntry): void {
  if (!activeRepositoryPath.value) return;
  dialog.warning({
    title: tr("恢复暂存栈", "Pop stash"),
    content: tr(
      "恢复 {reference} 到当前工作区并从暂存栈移除？若产生冲突，Git 会保留工作区供你处理。",
      "Apply {reference} to the current working tree and remove it from the stash? If conflicts occur, Git leaves the working tree available for resolution.",
      { reference: stash.reference },
    ),
    positiveText: tr("恢复", "Pop"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        applySnapshot(await api.popStash(activeRepositoryPath.value!, stash.reference, stash.commitId));
        await loadHistory();
        message.success(tr("暂存栈已恢复。", "Stash restored."));
      } catch (error) {
        reportError(error, tr("恢复暂存栈失败", "Could not pop stash"));
      } finally {
        mutationBusy.value = false;
      }
    },
  });
}

function confirmDropStash(stash: StashEntry): void {
  if (!activeRepositoryPath.value) return;
  dialog.warning({
    title: tr("删除暂存栈", "Drop stash"),
    content: tr(
      "永久删除 {reference}？此操作不能撤销。",
      "Permanently delete {reference}? This cannot be undone.",
      { reference: stash.reference },
    ),
    positiveText: tr("删除", "Drop"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        await api.dropStash(activeRepositoryPath.value!, stash.reference, stash.commitId);
        await loadHistory();
        message.success(tr("暂存栈已删除。", "Stash dropped."));
      } catch (error) {
        reportError(error, tr("删除暂存栈失败", "Could not drop stash"));
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
      ? tr("Soft：保留暂存区和工作区", "Soft: keep index and working tree")
      : mode === "mixed"
        ? tr("Mixed：保留工作区，取消暂存", "Mixed: keep working tree, unstage changes")
        : tr("Hard：丢弃暂存区和工作区改动", "Hard: discard index and working-tree changes");
  dialog.error({
    title: tr("重置分支", "Reset branch"),
    content: tr(
      "将当前分支重置到 {commit}？{mode}。Hard reset 可能永久丢失未提交改动。",
      "Reset the current branch to {commit}? {mode}. A hard reset can permanently discard uncommitted changes.",
      { commit: commit.shortSha, mode: modeLabel },
    ),
    positiveText: tr("确认重置", "Reset branch"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        applySnapshot(await api.resetBranch(activeRepositoryPath.value!, commit.sha, mode));
        await loadHistory();
        message.success(tr("分支已重置。", "Branch reset."));
      } catch (error) {
        reportError(error, tr("重置分支失败", "Could not reset branch"));
      } finally {
        mutationBusy.value = false;
      }
    },
  });
}

function openSquashModal(): void {
  if (commits.value.length < 3) {
    message.warning(tr("至少需要三个提交，才能保留一个基线提交并压缩最近提交。", "At least three commits are needed to preserve a base commit while squashing recent commits."));
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
    message.warning(tr("请输入有效的提交数量（至少 2，且保留一个基线提交）和提交说明。", "Enter a valid count (at least 2 while keeping a base commit) and commit message."));
    return;
  }
  mutationBusy.value = true;
  try {
    applySnapshot(await api.squashRecentCommits(activeRepositoryPath.value, count, squashForm.message.trim()));
    squashModalShown.value = false;
    await loadHistory();
    message.success(tr("最近提交已压缩。", "Recent commits squashed."));
  } catch (error) {
    reportError(error, tr("压缩提交失败", "Could not squash commits"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmRevertCommit(commit: CommitSummary): void {
  if (!activeRepositoryPath.value) return;
  dialog.warning({
    title: tr("还原提交", "Revert commit"),
    content: tr(
      "创建一个新的提交来还原 {commit}？这不会改写现有历史。",
      "Create a new commit that reverts {commit}? This does not rewrite existing history.",
      { commit: commit.shortSha },
    ),
    positiveText: tr("创建还原提交", "Create revert commit"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        applySnapshot(await api.revertCommit(activeRepositoryPath.value!, commit.sha));
        await loadHistory();
        message.success(tr("已创建还原提交。", "Revert commit created."));
      } catch (error) {
        reportError(error, tr("还原提交失败", "Could not revert commit"));
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
    message.warning(tr("请输入标签名称。", "Enter a tag name."));
    return;
  }
  const repositoryPath = tagRepositoryPath.value;
  if (!repositoryPath || activeRepositoryPath.value !== repositoryPath) {
    tagModalShown.value = false;
    tagRepositoryPath.value = null;
    message.warning(tr("仓库已切换，请重新选择要创建标签的提交。", "The repository changed. Select the tag target again."));
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
      message.success(tr("标签已创建。", "Tag created."));
    }
  } catch (error) {
    if (activeRepositoryPath.value === repositoryPath) {
      reportError(error, tr("创建标签失败", "Could not create tag"));
    }
  } finally {
    mutationBusy.value = false;
  }
}

function confirmDeleteTag(name: string): void {
  if (!activeRepositoryPath.value) return;
  dialog.warning({
    title: tr("删除本地标签", "Delete local tag"),
    content: tr("删除本地标签 {tag}？这不会删除远程标签。", "Delete local tag {tag}? This does not delete a remote tag.", { tag: name }),
    positiveText: tr("删除", "Delete"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        await api.deleteTag(activeRepositoryPath.value!, name);
        await refreshSnapshot();
        message.success(tr("标签已删除。", "Tag deleted."));
      } catch (error) {
        reportError(error, tr("删除标签失败", "Could not delete tag"));
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
    message.warning(tr("请输入 remote 名称和 fetch 地址。", "Enter a remote name and fetch URL."));
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
    message.success(tr("远程仓库已保存。", "Remote saved."));
  } catch (error) {
    reportError(error, tr("保存远程仓库失败", "Could not save remote"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmRemoveRemote(): void {
  if (!activeRepositoryPath.value || !editedRemoteName.value) return;
  const name = editedRemoteName.value;
  dialog.warning({
    title: tr("移除远程仓库", "Remove remote"),
    content: tr("移除远程仓库 {remote}？关联的账号绑定也会被删除。", "Remove remote {remote}? Its associated account binding will also be removed.", { remote: name }),
    positiveText: tr("移除", "Remove"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        await api.removeRemote(activeRepositoryPath.value!, name);
        remoteModalShown.value = false;
        await refreshSnapshot();
        message.success(tr("远程仓库已移除。", "Remote removed."));
      } catch (error) {
        reportError(error, tr("移除远程仓库失败", "Could not remove remote"));
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
    message.warning(tr("请先选择一个 remote。", "Select a remote first."));
    return;
  }
  if (
    !repositorySelectionIsCurrent(repositoryPath, selectionGeneration) ||
    (capturedTarget && activeRemote.value?.name !== remoteName)
  ) {
    message.warning(tr("仓库或 remote 已切换，请重新执行操作。", "The repository or remote changed. Start the operation again."));
    return;
  }
  if (networkOperationsInFlight.has(repositoryPath)) {
    message.warning(tr("已有网络操作正在进行，请等待完成。", "A network operation is already in progress. Wait for it to finish."));
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
    const actionLabel =
      action === "fetch"
        ? tr("获取", "Fetch")
        : action === "pull"
          ? tr("拉取", "Pull")
          : forceWithLease
            ? tr("强制推送（带 lease）", "Force push with lease")
            : tr("推送", "Push");
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      message.success(tr("{action} 已完成。", "{action} completed.", { action: actionLabel }));
      await refreshSnapshot();
    }
  } catch (error) {
    if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
      reportError(
        error,
        action === "fetch"
          ? tr("获取失败", "Fetch failed")
          : action === "pull"
            ? tr("拉取失败", "Pull failed")
            : tr("推送失败", "Push failed"),
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
    title: tr("强制推送（带 lease）", "Force push with lease"),
    content: tr(
      "向 {remote} 使用 force-with-lease 推送？它会改写远程分支历史，但在远程分支已被他人更新时会拒绝覆盖。请先确认团队协作约定。",
      "Force-push to {remote} with lease? It rewrites remote branch history but refuses to overwrite a branch changed by someone else. Confirm your team workflow first.",
      { remote: target.remoteName },
    ),
    positiveText: tr("强制推送", "Force push"),
    negativeText: tr("取消", "Cancel"),
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
    message.warning(tr("请选择属于该代码托管 Endpoint 的账号。", "Select an account that belongs to this Git service endpoint."));
    return;
  }
  if (selectedAccount && !supportsGitRemoteCredentials(selectedAccount)) {
    message.warning(tr("指定账号绑定仅支持应用管理的 PAT。SSH agent 与系统 Git credential helper 会按 remote URL 自动处理，无需绑定账号。", "An account-bound remote supports only an app-managed PAT. SSH agent and the system Git credential helper work automatically by remote URL and need no account binding."));
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
    message.success(tr("已保存 remote 账号绑定。下次网络操作将优先采用该绑定。", "Remote account binding saved. Future network operations will prefer it."));
  } catch (error) {
    reportError(error, tr("保存绑定失败", "Could not save binding"));
  } finally {
    mutationBusy.value = false;
  }
}

async function saveAccount(): Promise<void> {
  if (accountUsesPat.value && !accountFormCapabilities.value.patKeychain) {
    message.warning(tr("此 Endpoint 尚未实现应用钥匙串 PAT。请使用已支持的认证方式。", "This endpoint does not support an app-managed PAT. Choose a supported authentication method."));
    return;
  }
  if (!accountForm.login.trim() || (accountUsesPat.value && !accountForm.token.trim())) {
    message.warning(accountUsesPat.value ? tr("请输入登录名和个人访问令牌。", "Enter an account login and personal access token.") : tr("请输入账号登录名。", "Enter the account login."));
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
        ? tr("账号元数据已保存，PAT 已交给系统钥匙串。应用不会再次读取或展示它。", "Account metadata was saved and the PAT was sent to the system keychain. It will not be shown again.")
        : tr("账号资料已保存。网络 Git 操作会按 remote 绑定使用此认证方式。", "Account profile saved. Network Git operations will use this method according to the remote binding."),
    );
    void refreshAccountStatuses(true);
  } catch (error) {
    reportError(error, tr("添加账号失败", "Could not add account"));
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
    message.warning(tr("请填写完整的自托管 Endpoint 信息。", "Complete the required Git service endpoint details."));
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
    message.success(tr("自托管 Endpoint 已保存；现在可为 remote 选择它。", "Git service endpoint saved. You can now select it for a remote."));
  } catch (error) {
    reportError(error, tr("保存自托管 Endpoint 失败", "Could not save Git service endpoint"));
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
    message.success(imported.length ? tr("已检测到 {count} 个 CLI 账号。", "Detected {count} CLI account(s).", { count: imported.length }) : tr("没有可导入的 CLI 账号。", "No CLI accounts are available to import."));
} catch (error) {
    reportError(error, tr("检测 CLI 账号失败", "Could not detect CLI accounts"));
  } finally {
    mutationBusy.value = false;
  }
}

async function setDefaultCandidate(account: AccountProfile): Promise<void> {
  mutationBusy.value = true;
  try {
    accounts.value = await api.setDefaultAccount(account.id, !account.isDefaultCandidate);
    message.success(account.isDefaultCandidate ? tr("已取消默认候选。", "Default candidate removed.") : tr("已设为默认候选。它不会覆盖 remote 的明确绑定。", "Set as default candidate. It does not override an explicit remote binding."));
  } catch (error) {
    reportError(error, tr("更新默认候选失败", "Could not update default candidate"));
  } finally {
    mutationBusy.value = false;
  }
}

function confirmRemoveAccount(account: AccountProfile): void {
  dialog.warning({
    title: tr("移除账号", "Remove account"),
    content: tr(
      "移除 {account} 的本地账号资料？这会移除应用使用的钥匙串引用，但不会修改 Git、gh 或 glab 的全局配置。",
      "Remove the local profile for {account}? This removes the app's keychain reference but never changes global Git, gh, or glab configuration.",
      { account: account.displayName || account.login },
    ),
    positiveText: tr("移除", "Remove"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: async () => {
      mutationBusy.value = true;
      try {
        await api.removeAccount(account.id);
        accounts.value = accounts.value.filter((item) => item.id !== account.id);
        message.success(tr("账号已移除。", "Account removed."));
      } catch (error) {
        reportError(error, tr("移除账号失败", "Could not remove account"));
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
    reportError(error, tr("读取代码托管仓库信息失败", "Could not load Git service repository information"));
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
    reportError(error, tr("读取 Pull Request 失败", "Could not load pull requests"));
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
    message.warning(tr("请先切换到一个本地分支。", "Switch to a local branch before creating a pull request."));
    return;
  }
  if (!pullRequestForm.title.trim() || !pullRequestForm.base.trim()) {
    message.warning(tr("请输入 Pull Request 标题并选择目标分支。", "Enter a pull request title and choose a base branch."));
    return;
  }
  if (pullRequestForm.base.trim() === snapshot.value.status.branch) {
    message.warning(tr("源分支和目标分支不能相同。", "The head branch and base branch cannot be the same."));
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
      tr(
        "Pull Request #{number} 已创建。",
        "Pull request #{number} created.",
        { number: pullRequest.number },
      ),
    );
  } catch (error) {
    reportError(error, tr("创建 Pull Request 失败", "Could not create pull request"));
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
      : { code: "unexpected_error", message: tr("发生未预期错误。", "An unexpected error occurred."), details: null };
    publishOwnersError.value = localizedErrorMessage(appError.code, appError.message);
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
    message.warning(tr("请选择可发布的所有者并填写 GitHub 仓库名称。", "Select an eligible owner and enter a GitHub repository name."));
    return;
  }
  if (publishForm.name.trim().includes("/")) {
    message.warning(tr("仓库名称只填写名称即可，所有者请使用上方选项。", "Enter only the repository name; choose its owner above."));
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
    message.success(tr("仓库已发布到 GitHub，并已配置 origin。", "Repository published to GitHub and origin configured."));
  } catch (error) {
    reportError(error, tr("发布 GitHub 仓库失败", "Could not publish GitHub repository"));
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

watch(selectedRemoteName, () => {
  repositoryInfo.value = null;
  pullRequests.value = [];
  void loadCapabilities();
});

watch(activeTab, (tab) => {
  if (tab === "history") void loadHistoryWithOptions();
});

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
  controllerMounted = true;
  if (!browserDemo) window.addEventListener("keydown", handleDevtoolsShortcut);
  window.addEventListener("focus", handleWindowFocus);
  window.addEventListener("blur", handleWindowBlur);
  document.addEventListener("visibilitychange", handleVisibilityChange);
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
  await Promise.all([loadRepositories(), loadAccounts(), loadEndpoints(), loadExternalEditor()]);
  if (!controllerMounted) return;
  const storedPath = storedLastRepositoryPath();
  const storedRepository = storedPath
    ? repositories.value.find((repository) => repository.path === storedPath)
    : undefined;
  const mostRecentRepository = [...repositories.value].sort(
    (left, right) => (right.lastOpenedAt ?? 0) - (left.lastOpenedAt ?? 0),
  )[0];
  const startupRepository = storedRepository ?? mostRecentRepository;
  if (startupRepository) await selectRepository(startupRepository.path);
});

onBeforeUnmount(() => {
  controllerMounted = false;
  if (commitFilesDebounceTimer) clearTimeout(commitFilesDebounceTimer);
  if (commitDiffDebounceTimer) clearTimeout(commitDiffDebounceTimer);
  commitFilesDebounceTimer = null;
  commitDiffDebounceTimer = null;
  pendingCommitDiffRequest = null;
  commitFilesRequestGeneration += 1;
  commitDiffRequestGeneration += 1;
  const stopWatcherGeneration = ++repositoryWatchRequestGeneration;
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
    tr,
    colorMode,
    setColorMode,
    themeColor,
    authenticationMethodLabel,
    authenticationStatusLabel,
    endpointKindLabel,
  };
}

export type WorkspaceController = ReturnType<typeof useWorkspaceController>;
