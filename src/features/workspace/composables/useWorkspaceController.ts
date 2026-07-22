import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useDialog, useMessage } from "naive-ui";
import { api, builtinEndpoints, isAppError, isBrowserDemo, toggleDevtools } from "../../../shared/api/tauri";
import { appLocale, localizedErrorMessage, setAppLocale, tr, type AppLocale } from "../../../shared/i18n";
import { colorMode, isDarkTheme, setColorMode, type ColorMode } from "../../../shared/theme";
import {
  authenticationMethodLabel,
  authenticationStatusLabel,
  endpointKindLabel,
  type AccountProfile,
  type AuthenticationMethod,
  type AppError,
  type BranchInfo,
  type CommitSummary,
  type CodeHostCapabilities,
  type CodeHostEndpoint,
  type CodeHostRepositoryInfo,
  type GitStatusEntry,
  type LfsStatus,
  type PullRequestSummary,
  type RemoteBinding,
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

interface RepositorySidebarGroup {
  key: string;
  label: string;
  repositories: RepositoryRecord[];
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
const colorModeLabel = computed(() => {
  if (colorMode.value === "system") return tr("跟随系统", "Follow system");
  return colorMode.value === "dark" ? tr("深色", "Dark") : tr("浅色", "Light");
});
const themeButtonLabel = computed(() => tr("外观：{mode}", "Appearance: {mode}", { mode: colorModeLabel.value }));
const colorModeIcon = computed(() => (colorMode.value === "system" ? "◐" : isDarkTheme.value ? "☾" : "☀"));
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
const snapshot = ref<RepositorySnapshot | null>(null);
const accounts = ref<AccountProfile[]>([]);
const selectedRemoteName = ref("");
const activeCapabilities = ref<CodeHostCapabilities>(emptyCapabilities);
const repositoryInfo = ref<CodeHostRepositoryInfo | null>(null);
const pullRequests = ref<PullRequestSummary[]>([]);
const selectedChange = ref<GitStatusEntry | null>(null);
const diffText = ref("");
const selectedDiffStaged = ref(false);
const commits = ref<CommitSummary[]>([]);
const stashes = ref<StashEntry[]>([]);
const worktrees = ref<WorktreeInfo[]>([]);
const submodules = ref<SubmoduleInfo[]>([]);
const lfsStatus = ref<LfsStatus | null>(null);
const selectedCommit = ref<CommitSummary | null>(null);
const commitDiffText = ref("");
const lastError = ref<AppError | null>(null);

const repositoriesLoading = ref(false);
const snapshotLoading = ref(false);
const accountsLoading = ref(false);
const accountStatusRefreshing = ref(false);
const diffLoading = ref(false);
const platformLoading = ref(false);
const pullRequestsLoading = ref(false);
const historyLoading = ref(false);
const commitDiffLoading = ref(false);
const stashesLoading = ref(false);
const worktreesLoading = ref(false);
const submodulesLoading = ref(false);
const lfsLoading = ref(false);
const networkBusy = ref<NetworkAction | null>(null);
const mutationBusy = ref(false);

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
const pullRequestModalShown = ref(false);
const squashModalShown = ref(false);
const publishModalShown = ref(false);
const tagModalShown = ref(false);
const remoteModalShown = ref(false);
const bindingDrawerShown = ref(false);
const bindingRemote = ref<RemoteProfile | null>(null);
const repositoryGroupModalShown = ref(false);
const repositoryGroupTarget = ref<RepositoryRecord | null>(null);

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
const pullRequestForm = reactive({ title: "", body: "", base: "", draft: false });
const squashForm = reactive({ count: "2", message: "" });
const publishForm = reactive({ endpointId: "github.com", accountId: "", name: "", isPrivate: true });
const tagForm = reactive({ name: "", target: "", message: "" });
const remoteForm = reactive({ name: "", fetchUrl: "", pushUrl: "" });
const editedRemoteName = ref<string | null>(null);
const bindingForm = reactive({ endpointId: "", accountId: "auto" });
const repositoryGroupForm = reactive({ name: "" });
const endpoints = ref<CodeHostEndpoint[]>([...builtinEndpoints]);
let lastAccountStatusRefreshAt = 0;
const accountStatusRefreshCacheMs = 60_000;

const activeRemote = computed(() => snapshot.value?.remotes.find((remote) => remote.name === selectedRemoteName.value) ?? null);
const currentBranch = computed(() => snapshot.value?.status.branch ?? "—");
const stagedChanges = computed(() => snapshot.value?.status.entries.filter((entry) => Boolean(entry.indexStatus)) ?? []);
const stagedCount = computed(() => stagedChanges.value.length);
const unstagedCount = computed(() => snapshot.value?.status.entries.filter((entry) => Boolean(entry.worktreeStatus)).length ?? 0);
const workingChanges = computed(() => snapshot.value?.status.entries.filter((entry) => Boolean(entry.worktreeStatus)) ?? []);
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
const commitDiffLines = computed(() => splitDiffLines(commitDiffText.value));
const canAmendLatestCommit = computed(() => snapshot.value?.hasHead ?? false);
const isUnbornRepository = computed(() => Boolean(snapshot.value && !snapshot.value.hasHead));
const primaryNetworkAction = computed<NetworkAction | null>(() => {
  if (!activeRemote.value) return null;
  if ((snapshot.value?.status.behind ?? 0) > 0) return "pull";
  if ((snapshot.value?.status.ahead ?? 0) > 0) return "push";
  return "fetch";
});
const primaryNetworkActionLabel = computed(() => {
  switch (primaryNetworkAction.value) {
    case "pull":
      return tr("拉取更新", "Pull changes");
    case "push":
      return tr("推送提交", "Push commits");
    case "fetch":
      return tr("获取更新", "Fetch");
    default:
      return tr("同步", "Sync");
  }
});
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
const accountFormCapabilities = computed(() => capabilityForEndpoint(accountForm.endpointId));
const accountUsesPat = computed(() => accountForm.authMethod === "personalAccessToken");
const accountMethodOptions = computed(() => [
  { label: tr("个人访问令牌 (PAT)", "Personal access token (PAT)"), value: "personalAccessToken", disabled: !accountFormCapabilities.value.patKeychain },
  { label: "SSH agent", value: "sshAgent" },
]);
function supportsGitRemoteCredentials(account: AccountProfile): boolean {
  return account.authMethod === "personalAccessToken" || account.authMethod === "sshAgent";
}
const bindingAccounts = computed(() => accounts.value.filter(
  (account) => account.endpointId === bindingForm.endpointId && supportsGitRemoteCredentials(account),
));
const bindingAccountOptions = computed(() => [
  { label: tr("自动选择（唯一 PAT/SSH 候选；否则系统 Git）", "Auto-select (one PAT/SSH candidate; otherwise system Git)"), value: "auto" },
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
  (account) => account.endpointId === cloneForm.endpointId && supportsGitRemoteCredentials(account),
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
const tagTargetOptions = computed(() => [
  { label: tr("当前 HEAD", "Current HEAD"), value: "" },
  ...(snapshot.value?.branches.map((branch) => ({ label: branch.name, value: branch.name })) ?? []),
]);
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
  message.error(tr("{action}：{message}", "{action}: {message}", { action, message: appError.message }));
  if (appError.code === "account_selection_required" && activeRemote.value) {
    message.warning(tr("此 remote 有多个或没有可用账号，请明确选择一个账号后重试。", "This remote has multiple or no usable accounts. Select one explicitly and retry."));
    openBindingDrawer(activeRemote.value);
  }
}

function updateAppLocale(value: string | number): void {
  if (value === "zh-CN" || value === "en-US") setAppLocale(value as AppLocale);
}

function updateColorMode(value: string | number): void {
  if (value === "system" || value === "light" || value === "dark") setColorMode(value as ColorMode);
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

function applySnapshot(nextSnapshot: RepositorySnapshot): void {
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
}

function clearDiffSelection(): void {
  selectedChange.value = null;
  diffText.value = "";
  selectedDiffStaged.value = false;
}

async function syncSelectedDiff(nextSnapshot: RepositorySnapshot, selectedPath?: string, selectFirst = false): Promise<void> {
  const nextChange =
    (selectedPath ? nextSnapshot.status.entries.find((entry) => entry.path === selectedPath) : null) ??
    (selectFirst ? nextSnapshot.status.entries[0] : null);
  if (nextChange) {
    await loadDiff(nextChange);
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
  return left.name.localeCompare(right.name, appLocale.value, { sensitivity: "base" }) || left.path.localeCompare(right.path, appLocale.value);
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

async function selectRepository(repositoryPath: string): Promise<void> {
  activeRepositoryPath.value = repositoryPath;
  section.value = "repositories";
  snapshotLoading.value = true;
  lastError.value = null;
  try {
    clearDiffSelection();
    const nextSnapshot = await api.getRepositorySnapshot(repositoryPath);
    applySnapshot(nextSnapshot);
    activeTab.value = "changes";
    repositoryInfo.value = null;
    pullRequests.value = [];
    commits.value = [];
    stashes.value = [];
    worktrees.value = [];
    submodules.value = [];
    lfsStatus.value = null;
    selectedCommit.value = null;
    commitDiffText.value = "";
    amendLastCommit.value = false;
    await syncSelectedDiff(nextSnapshot, undefined, true);
    await Promise.all([loadCapabilities(), loadWorktrees(), loadSubmodules(), loadLfsStatus()]);
  } catch (error) {
    reportError(error, tr("打开仓库失败", "Could not open repository"));
  } finally {
    snapshotLoading.value = false;
  }
}

async function refreshSnapshot(): Promise<void> {
  if (!activeRepositoryPath.value) return;
  snapshotLoading.value = true;
  try {
    const selectedPath = selectedChange.value?.path;
    const nextSnapshot = await api.getRepositorySnapshot(activeRepositoryPath.value);
    applySnapshot(nextSnapshot);
    await syncSelectedDiff(nextSnapshot, selectedPath, !selectedPath);
    await Promise.all([loadWorktrees(), loadSubmodules(), loadLfsStatus()]);
  } catch (error) {
    reportError(error, tr("刷新仓库失败", "Could not refresh repository"));
  } finally {
    snapshotLoading.value = false;
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
    const repository = await api.cloneRepository({
      url: cloneForm.url.trim(),
      destinationPath: cloneForm.destinationPath.trim(),
      ...(cloneForm.accountId !== "none" ? { endpointId: cloneForm.endpointId } : {}),
      ...(cloneForm.accountId !== "none" && cloneForm.accountId !== "auto" ? { accountProfileId: cloneForm.accountId } : {}),
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

async function loadDiff(entry: GitStatusEntry): Promise<void> {
  if (!activeRepositoryPath.value) return;
  selectedChange.value = entry;
  selectedDiffStaged.value = Boolean(entry.indexStatus) && !entry.worktreeStatus;
  diffLoading.value = true;
  try {
    diffText.value = await api.getFileDiff(activeRepositoryPath.value, entry.path, Boolean(entry.indexStatus) && !entry.worktreeStatus);
  } catch (error) {
    reportError(error, tr("读取差异失败", "Could not load diff"));
  } finally {
    diffLoading.value = false;
  }
}

async function applyDiffHunk(hunk: DiffHunk): Promise<void> {
  if (!activeRepositoryPath.value || !selectedChange.value) return;
  const selectedPath = selectedChange.value.path;
  const reverse = selectedDiffStaged.value;
  mutationBusy.value = true;
  try {
    const nextSnapshot = await api.applyPatch(activeRepositoryPath.value, hunk.patch, reverse);
    applySnapshot(nextSnapshot);
    const nextEntry = nextSnapshot.status.entries.find((entry) => entry.path === selectedPath);
    if (nextEntry) {
      await loadDiff(nextEntry);
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
  if (!activeRepositoryPath.value) return;
  historyLoading.value = true;
  stashesLoading.value = true;
  try {
    const [nextCommits, nextStashes] = await Promise.all([
      api.listCommits(activeRepositoryPath.value),
      api.listStashes(activeRepositoryPath.value),
    ]);
    commits.value = nextCommits;
    stashes.value = nextStashes;
    if (selectedCommit.value && !nextCommits.some((commit) => commit.sha === selectedCommit.value?.sha)) {
      selectedCommit.value = null;
      commitDiffText.value = "";
    }
  } catch (error) {
    reportError(error, tr("读取历史记录失败", "Could not load repository history"));
  } finally {
    historyLoading.value = false;
    stashesLoading.value = false;
  }
}

async function loadWorktrees(): Promise<void> {
  if (!activeRepositoryPath.value) {
    worktrees.value = [];
    return;
  }
  worktreesLoading.value = true;
  try {
    worktrees.value = await api.listWorktrees(activeRepositoryPath.value);
  } catch (error) {
    reportError(error, tr("读取工作树失败", "Could not load worktrees"));
  } finally {
    worktreesLoading.value = false;
  }
}

async function loadSubmodules(): Promise<void> {
  if (!activeRepositoryPath.value) {
    submodules.value = [];
    return;
  }
  submodulesLoading.value = true;
  try {
    submodules.value = await api.listSubmodules(activeRepositoryPath.value);
  } catch (error) {
    reportError(error, tr("读取子模块失败", "Could not load submodules"));
  } finally {
    submodulesLoading.value = false;
  }
}

async function loadLfsStatus(): Promise<void> {
  if (!activeRepositoryPath.value) {
    lfsStatus.value = null;
    return;
  }
  lfsLoading.value = true;
  try {
    lfsStatus.value = await api.getLfsStatus(activeRepositoryPath.value);
  } catch (error) {
    reportError(error, tr("读取 Git LFS 状态失败", "Could not load Git LFS status"));
  } finally {
    lfsLoading.value = false;
  }
}

async function loadCommitDiff(commit: CommitSummary): Promise<void> {
  if (!activeRepositoryPath.value) return;
  selectedCommit.value = commit;
  commitDiffLoading.value = true;
  try {
    commitDiffText.value = await api.getCommitDiff(activeRepositoryPath.value, commit.sha);
  } catch (error) {
    reportError(error, tr("读取提交改动失败", "Could not load commit diff"));
  } finally {
    commitDiffLoading.value = false;
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

async function refreshAfterGitConflict(error: unknown): Promise<void> {
  if (isAppError(error) && error.code === "git_merge_conflict") await refreshSnapshot();
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

async function checkoutBranch(branch: BranchInfo): Promise<void> {
  if (!activeRepositoryPath.value || branch.isCurrent) return;
  mutationBusy.value = true;
  try {
    applySnapshot(await api.checkoutBranch(activeRepositoryPath.value, branch.name));
    message.success(tr("已切换到 {branch}。", "Switched to {branch}.", { branch: branch.name }));
  } catch (error) {
    reportError(error, tr("切换分支失败", "Could not switch branch"));
  } finally {
    mutationBusy.value = false;
  }
}

async function createBranch(): Promise<void> {
  if (!activeRepositoryPath.value || !branchForm.name.trim()) {
    message.warning(tr("请输入分支名称。", "Enter a branch name."));
    return;
  }
  mutationBusy.value = true;
  try {
    applySnapshot(
      await api.createBranch({ repositoryPath: activeRepositoryPath.value, name: branchForm.name.trim(), checkout: branchForm.checkout }),
    );
    branchModalShown.value = false;
    branchForm.name = "";
    branchForm.checkout = true;
    message.success(tr("分支已创建。", "Branch created."));
  } catch (error) {
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
        applySnapshot(await api.popStash(activeRepositoryPath.value!, stash.reference));
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
        await api.dropStash(activeRepositoryPath.value!, stash.reference);
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

function openTagModal(): void {
  tagForm.name = "";
  tagForm.target = "";
  tagForm.message = "";
  tagModalShown.value = true;
}

async function createTag(): Promise<void> {
  if (!activeRepositoryPath.value || !tagForm.name.trim()) {
    message.warning(tr("请输入标签名称。", "Enter a tag name."));
    return;
  }
  mutationBusy.value = true;
  try {
    await api.createTag(
      activeRepositoryPath.value,
      tagForm.name.trim(),
      tagForm.target.trim() || undefined,
      tagForm.message.trim() || undefined,
    );
    tagModalShown.value = false;
    await refreshSnapshot();
    message.success(tr("标签已创建。", "Tag created."));
  } catch (error) {
    reportError(error, tr("创建标签失败", "Could not create tag"));
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

async function runNetworkAction(action: NetworkAction, forceWithLease = false): Promise<void> {
  if (!activeRepositoryPath.value || !activeRemote.value) {
    message.warning(tr("请先选择一个 remote。", "Select a remote first."));
    return;
  }
  networkBusy.value = action;
  lastError.value = null;
  try {
    await (action === "fetch"
      ? api.fetchRemote(activeRepositoryPath.value, activeRemote.value.name)
      : action === "pull"
        ? api.pullRemote(activeRepositoryPath.value, activeRemote.value.name)
        : api.pushRemote(activeRepositoryPath.value, activeRemote.value.name, forceWithLease));
    const actionLabel =
      action === "fetch"
        ? tr("获取", "Fetch")
        : action === "pull"
          ? tr("拉取", "Pull")
          : forceWithLease
            ? tr("强制推送（带 lease）", "Force push with lease")
            : tr("推送", "Push");
    message.success(tr("{action} 已完成。", "{action} completed.", { action: actionLabel }));
    await refreshSnapshot();
  } catch (error) {
    reportError(
      error,
      action === "fetch"
        ? tr("获取失败", "Fetch failed")
        : action === "pull"
          ? tr("拉取失败", "Pull failed")
          : tr("推送失败", "Push failed"),
    );
  } finally {
    networkBusy.value = null;
  }
}

async function runPrimaryNetworkAction(): Promise<void> {
  if (primaryNetworkAction.value) await runNetworkAction(primaryNetworkAction.value);
}

function confirmForcePush(): void {
  if (!activeRemote.value) return;
  dialog.error({
    title: tr("强制推送（带 lease）", "Force push with lease"),
    content: tr(
      "向 {remote} 使用 force-with-lease 推送？它会改写远程分支历史，但在远程分支已被他人更新时会拒绝覆盖。请先确认团队协作约定。",
      "Force-push to {remote} with lease? It rewrites remote branch history but refuses to overwrite a branch changed by someone else. Confirm your team workflow first.",
      { remote: activeRemote.value.name },
    ),
    positiveText: tr("强制推送", "Force push"),
    negativeText: tr("取消", "Cancel"),
    onPositiveClick: () => void runNetworkAction("push", true),
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
    message.warning(tr("指定账号绑定仅支持 PAT 或 SSH agent。系统 Git credential helper 会按 remote URL 自动处理，无需绑定账号。", "An account-bound remote supports only a PAT or SSH agent. The system Git credential helper works automatically by remote URL and needs no account binding."));
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

function openPublishModal(): void {
  publishForm.endpointId = publishEndpoints.value[0]?.id || "github.com";
  publishForm.accountId = publishAccounts.value[0]?.id || "";
  publishForm.name = snapshot.value?.repository.name || "";
  publishForm.isPrivate = true;
  publishModalShown.value = true;
}

async function publishGitHubRepository(): Promise<void> {
  if (!activeRepositoryPath.value || !publishForm.endpointId || !publishForm.accountId || !publishForm.name.trim()) {
    message.warning(tr("请选择匹配的 gh 账号并填写 GitHub 仓库名称。", "Select a matching gh account and enter a GitHub repository name."));
    return;
  }
  mutationBusy.value = true;
  try {
    applySnapshot(
      await api.publishGitHubRepository(
        activeRepositoryPath.value,
        publishForm.endpointId,
        publishForm.accountId,
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
  }
}

watch(selectedRemoteName, () => {
  repositoryInfo.value = null;
  pullRequests.value = [];
  void loadCapabilities();
});

watch(activeTab, (tab) => {
  if (tab === "history") void loadHistory();
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
    if (!publishAccounts.value.some((account) => account.id === publishForm.accountId)) {
      publishForm.accountId = publishAccounts.value[0]?.id || "";
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
  if (!browserDemo) window.addEventListener("keydown", handleDevtoolsShortcut);
  await Promise.all([loadRepositories(), loadAccounts(), loadEndpoints()]);
  if (repositories.value[0]) await selectRepository(repositories.value[0].path);
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", handleDevtoolsShortcut);
});

  return {
    message,
    dialog,
    browserDemo,
    repositoryGroupingStorageKey,
    initialRepositoryGrouping,
    emptyCapabilities,
    localeOptions,
    colorModeLabel,
    themeButtonLabel,
    colorModeIcon,
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
    commits,
    stashes,
    worktrees,
    submodules,
    lfsStatus,
    selectedCommit,
    commitDiffText,
    lastError,
    repositoriesLoading,
    snapshotLoading,
    accountsLoading,
    accountStatusRefreshing,
    diffLoading,
    platformLoading,
    pullRequestsLoading,
    historyLoading,
    commitDiffLoading,
    stashesLoading,
    worktreesLoading,
    submodulesLoading,
    lfsLoading,
    networkBusy,
    mutationBusy,
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
    pullRequestModalShown,
    squashModalShown,
    publishModalShown,
    tagModalShown,
    remoteModalShown,
    bindingDrawerShown,
    bindingRemote,
    repositoryGroupModalShown,
    repositoryGroupTarget,
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
    pullRequestForm,
    squashForm,
    publishForm,
    tagForm,
    remoteForm,
    editedRemoteName,
    bindingForm,
    repositoryGroupForm,
    endpoints,
    lastAccountStatusRefreshAt,
    accountStatusRefreshCacheMs,
    activeRemote,
    currentBranch,
    stagedChanges,
    stagedCount,
    unstagedCount,
    workingChanges,
    allChangesIncluded,
    someChangesIncluded,
    conflictedChanges,
    diffHunks,
    diffLines,
    hasDiffText,
    commitDiffLines,
    canAmendLatestCommit,
    isUnbornRepository,
    primaryNetworkAction,
    primaryNetworkActionLabel,
    commitActionLabel,
    commitSelectionSummary,
    rebaseBaseOptions,
    worktreeExistingBranchOptions,
    pullRequestBaseOptions,
    publishEndpoints,
    publishAccounts,
    publishAccountOptions,
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
    updateAppLocale,
    updateColorMode,
    openRepositoryAction,
    handleDevtoolsShortcut,
    applySnapshot,
    clearDiffSelection,
    syncSelectedDiff,
    loadCapabilities,
    compareRepositoryRecords,
    replaceRepositoryRecord,
    openRepositoryGroupEditor,
    saveRepositoryGroup,
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
    applyDiffHunk,
    loadHistory,
    loadWorktrees,
    loadSubmodules,
    loadLfsStatus,
    loadCommitDiff,
    setChangeIncluded,
    stageAll,
    unstageAll,
    confirmDiscardChanges,
    discardSelectedChange,
    refreshAfterGitConflict,
    commitMessage,
    amendLastCommit,
    setAmendLastCommit,
    commitChanges,
    checkoutBranch,
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
    runPrimaryNetworkAction,
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
    isDarkTheme,
    setColorMode,
    authenticationMethodLabel,
    authenticationStatusLabel,
    endpointKindLabel,
  };
}

export type WorkspaceController = ReturnType<typeof useWorkspaceController>;
