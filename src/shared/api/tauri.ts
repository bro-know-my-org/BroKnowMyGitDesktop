import { localizedErrorMessage, t, type MessageKey } from "../i18n";
import { builtinEndpoints } from "./builtinEndpoints";
import { request } from "./tauriRuntime";
import { boundedBrowserCommitFileDiff, parseGitDiffHeaderPaths } from "./browserDemoDiff";
export { isAppError, isBrowserDemo, toggleDevtools } from "./tauriRuntime";
import {
  demoMarkers,
  initialBrowserAccounts,
  initialBrowserCommits,
  initialBrowserLfsStatus,
  initialBrowserPullRequests,
  initialBrowserSubmodules,
  initialBrowserWorktrees,
  mockSnapshot,
} from "./browserDemoFixtures";
export { builtinEndpoints } from "./builtinEndpoints";
import type {
  AccountProfile,
  AppError,
  CloneRepositoryInput,
  CleanUntrackedPreview,
  CommitChangedFile,
  CommitFileDiff,
  CommitSummary,
  CodeHostCapabilities,
  CodeHostEndpoint,
  CodeHostRepositoryInfo,
  CreatePullRequestInput,
  CreateBranchInput,
  CreateBranchResult,
  DiscardChangesPreview,
  ExternalEditorConfig,
  GitOperationKind,
  GitHubPublishOwner,
  GitHubForkSummary,
  GitStatusEntry,
  InitRepositoryInput,
  LfsStatus,
  NetworkOperationResult,
  PullRequestSummary,
  RemoteProfile,
  RemoteUrlKind,
  RemoteBinding,
  RepositoryRecord,
  RepositorySnapshot,
  SaveAccountProfileInput,
  SavePatAccountInput,
  ResetMode,
  StashEntry,
  SubmoduleInfo,
  WorktreeInfo,
} from "../types/domain";

type DemoCommitSummaryDescriptor =
  | { kind: "message"; key: MessageKey }
  | { kind: "cherryPick"; sourceSha: string }
  | { kind: "cherryPickRevision"; revision: string }
  | { kind: "revert"; sourceSha: string };

interface DemoStashMessageDescriptor {
  branch: string;
  message: string;
  messageKey?: MessageKey;
}

let browserRepositoryAliasKey: MessageKey | null = "demo.repository.alias";
let browserRepositoryGroupKey: MessageKey | null = "demo.repository.group";
const browserCommitSummaryDescriptors = new Map<string, DemoCommitSummaryDescriptor>([
  ["9d7a4f0c2ea846105d1200f9cf6a4eb68d748301", { kind: "message", key: "demo.commit.multiAccount" }],
  ["55b91f2b0e1a6540dcdb43f1f7db2e3a22d7db10", { kind: "message", key: "demo.commit.initial" }],
]);
const browserAccountDisplayNameKeys = new Map<string, MessageKey>([
  ["gitee-personal", "demo.account.personal"],
  ["github-cli", "demo.account.githubCli"],
  ["gitlab-cli", "demo.account.work"],
]);
const browserPullRequestTitleKeys = new Map<number, MessageKey>([[12, "demo.commit.multiAccount"]]);
const browserPullRequestUpdatedAtKeys = new Map<number, MessageKey>([[12, "demo.pullRequest.updatedNow"]]);
const browserStashMessageDescriptors = new Map<string, DemoStashMessageDescriptor>();

const builtinCapabilities: Record<string, CodeHostCapabilities> = {
  "github.com": {
    repositoryInfo: true,
    repositoryPublish: true,
    repositoryFork: true,
    pullRequestsRead: true,
    pullRequestsWrite: true,
    accountImport: true,
    accountBindings: true,
    patKeychain: true,
  },
  "gitlab.com": {
    repositoryInfo: false,
    repositoryPublish: false,
    repositoryFork: false,
    pullRequestsRead: false,
    pullRequestsWrite: false,
    accountImport: true,
    accountBindings: true,
    patKeychain: true,
  },
  "gitee.com": {
    repositoryInfo: false,
    repositoryPublish: false,
    repositoryFork: false,
    pullRequestsRead: false,
    pullRequestsWrite: false,
    accountImport: false,
    accountBindings: true,
    patKeychain: true,
  },
};

const unavailableCapabilities: CodeHostCapabilities = {
  repositoryInfo: false,
  repositoryPublish: false,
  repositoryFork: false,
  pullRequestsRead: false,
  pullRequestsWrite: false,
  accountImport: false,
  accountBindings: true,
  patKeychain: false,
};

function browserCapabilities(endpointId: string): CodeHostCapabilities {
  const builtin = builtinCapabilities[endpointId];
  if (builtin) return builtin;
  const endpoint = browserEndpoints.find((item) => item.id === endpointId);
  if (!endpoint) return unavailableCapabilities;
  if (endpoint.kind === "gitea" || endpoint.kind === "forgejo" || endpoint.kind === "genericGit") {
    return {
      repositoryInfo: false,
      repositoryPublish: false,
      repositoryFork: false,
      pullRequestsRead: false,
      pullRequestsWrite: false,
      accountImport: false,
      accountBindings: true,
      patKeychain: true,
    };
  }
  return unavailableCapabilities;
}

let browserSnapshot = rawCopy(mockSnapshot);
let browserCommits = rawCopy(initialBrowserCommits);
let browserBranchTips = new Map<string, string | null>([
  ["main", browserCommits[0].sha],
  ["feature/multi-account", browserCommits[0].sha],
]);
let browserRemoteBranchTips = new Map<string, string | null>([
  ["origin/main", browserCommits[1].sha],
  ["origin/feature/multi-account", browserCommits[0].sha],
  ["upstream/main", browserCommits[0].sha],
]);
let browserRevisionCounter = 0;

function createBrowserMockSha(): string {
  const counter = (++browserRevisionCounter).toString(16).padStart(7, "0");
  const timestamp = Date.now().toString(16).padStart(12, "0");
  return `${counter}${timestamp}${"0".repeat(21)}`;
}

function browserBranchCommits(branch: string | null): CommitSummary[] {
  if (!branch) return [];
  const tip = browserBranchTips.get(branch);
  if (!tip) return [];
  const commitsBySha = new Map(browserCommits.map((commit) => [commit.sha, commit]));
  const reachable = new Set<string>();
  const pending = [tip];
  while (pending.length) {
    const sha = pending.pop()!;
    if (reachable.has(sha)) continue;
    const commit = commitsBySha.get(sha);
    if (!commit) continue;
    reachable.add(sha);
    pending.push(...commit.parents);
  }
  return browserCommits.filter((commit) => reachable.has(commit.sha));
}

function browserHistoryCommits(snapshot: RepositorySnapshot): CommitSummary[] {
  const commits = snapshot.status.branch
    ? browserBranchCommits(snapshot.status.branch)
    : browserBranchCommitsFromTip(snapshot.status.headCommit);
  const upstreamTip = snapshot.status.upstream
    ? browserRemoteBranchTips.get(snapshot.status.upstream) ?? null
    : null;
  const pushed = upstreamTip
    ? new Set(browserBranchCommitsFromTip(upstreamTip).map((commit) => commit.sha))
    : null;
  return commits.map((commit) => ({
    ...commit,
    pushed: pushed ? pushed.has(commit.sha) : null,
  }));
}

function browserCommitChangedFiles(commit: CommitSummary): CommitChangedFile[] {
  if (commit.parents.length === 0) {
    return [
      { path: "README.md", previousPath: null, status: "added", similarity: null },
      { path: "src/main.ts", previousPath: null, status: "added", similarity: null },
    ];
  }
  return [
    { path: "README.md", previousPath: null, status: "modified", similarity: null },
    { path: "src/shared/api/tauri.ts", previousPath: null, status: "modified", similarity: null },
    { path: "src/accounts.ts", previousPath: "src/account.ts", status: "renamed", similarity: 92 },
  ];
}

function browserBranchCommitsFromTip(tip: string | null): CommitSummary[] {
  if (!tip) return [];
  const commitsBySha = new Map(browserCommits.map((commit) => [commit.sha, commit]));
  const reachable = new Set<string>();
  const pending = [tip];
  while (pending.length) {
    const sha = pending.pop()!;
    if (reachable.has(sha)) continue;
    const commit = commitsBySha.get(sha);
    if (!commit) continue;
    reachable.add(sha);
    pending.push(...commit.parents);
  }
  return browserCommits.filter((commit) => reachable.has(commit.sha));
}

function syncBrowserBranchStatus(snapshot: RepositorySnapshot): void {
  const branch = snapshot.status.branch;
  const localTip = branch ? browserBranchTips.get(branch) ?? null : null;
  const upstreamTip = snapshot.status.upstream
    ? browserRemoteBranchTips.get(snapshot.status.upstream) ?? null
    : null;
  const localCommits = new Set(browserBranchCommitsFromTip(localTip).map((commit) => commit.sha));
  const upstreamCommits = new Set(browserBranchCommitsFromTip(upstreamTip).map((commit) => commit.sha));
  snapshot.hasHead = localTip !== null;
  snapshot.status.headCommit = localTip;
  snapshot.status.upstreamCommit = upstreamTip;
  snapshot.status.ahead = snapshot.status.upstream
    ? [...localCommits].filter((sha) => !upstreamCommits.has(sha)).length
    : 0;
  snapshot.status.behind = snapshot.status.upstream
    ? [...upstreamCommits].filter((sha) => !localCommits.has(sha)).length
    : 0;
}

function setBrowserCurrentBranchTip(snapshot: RepositorySnapshot, sha: string | null): void {
  if (snapshot.status.branch) browserBranchTips.set(snapshot.status.branch, sha);
}
let browserWorktrees = rawCopy(initialBrowserWorktrees);
let browserSubmodules = rawCopy(initialBrowserSubmodules);
let browserLfsStatus = rawCopy(initialBrowserLfsStatus);
let browserExternalEditor: ExternalEditorConfig | null = null;
let browserCleanUntrackedConfirmation: { repositoryPath: string; token: string; paths: Set<string> } | null = null;
const browserDiscardConfirmationTtlMs = 60_000;
let browserDiscardChangesConfirmation: {
  repositoryPath: string;
  token: string;
  paths: Set<string>;
  issuedAt: number;
} | null = null;
type BrowserStash = StashEntry & { entries: GitStatusEntry[] };

let browserStashes: BrowserStash[] = [];

function renumberBrowserStashes(): void {
  browserStashes = browserStashes.map((stash, index) => ({
    ...stash,
    reference: `stash@{${index}}`,
  }));
}

function isBrowserUntrackedEntry(entry: GitStatusEntry): boolean {
  return !entry.indexStatus && entry.worktreeStatus === "?";
}

function stashBrowserChanges(
  snapshot: RepositorySnapshot,
  message: string,
  messageKey?: MessageKey,
  includeUntracked = true,
): boolean {
  const entries = includeUntracked
    ? snapshot.status.entries
    : snapshot.status.entries.filter((entry) => !isBrowserUntrackedEntry(entry));
  if (!entries.length) return false;
  const commitId = `browser-stash-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  browserStashMessageDescriptors.set(commitId, {
    branch: snapshot.status.branch || "HEAD",
    message,
    ...(messageKey ? { messageKey } : {}),
  });
  browserStashes = [{
    reference: "",
    commitId,
    message,
    entries: copy(entries),
  }, ...browserStashes];
  renumberBrowserStashes();
  snapshot.status.entries = includeUntracked
    ? []
    : snapshot.status.entries.filter(isBrowserUntrackedEntry);
  return true;
}
let browserPullRequests = rawCopy(initialBrowserPullRequests);
let browserAccounts = rawCopy(initialBrowserAccounts);

function browserGitHubPublishOwnersFor(account: AccountProfile): GitHubPublishOwner[] {
  const localizedAccount = localizeBrowserAccount(account);
  const personalOwner: GitHubPublishOwner = {
    login: account.login,
    name: localizedAccount.displayName,
    kind: "user",
    canCreateRepositories: true,
  };
  if (account.id !== "github-cli") return [personalOwner];
  return [
    personalOwner,
    {
      login: "broknowmyorg",
      name: t("demo.account.organization"),
      kind: "organization",
      canCreateRepositories: true,
    },
  ];
}
let browserEndpoints = rawCopy(builtinEndpoints);

function rawCopy<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function copy<T>(value: T): T {
  return rawCopy(value);
}

function localizeBrowserRepository(record: RepositoryRecord): RepositoryRecord {
  const localized = rawCopy(record);
  if (browserRepositoryAliasKey && record.alias === demoMarkers.repositoryAlias) {
    localized.alias = t(browserRepositoryAliasKey);
  }
  if (browserRepositoryGroupKey && record.group === demoMarkers.repositoryGroup) {
    localized.group = t(browserRepositoryGroupKey);
  }
  return localized;
}

function localizeBrowserSnapshot(snapshot: RepositorySnapshot): RepositorySnapshot {
  const localized = rawCopy(snapshot);
  localized.repository = localizeBrowserRepository(snapshot.repository);
  return localized;
}

function localizeBrowserCommitSummary(commit: CommitSummary, visited = new Set<string>()): string {
  const descriptor = browserCommitSummaryDescriptors.get(commit.sha);
  if (!descriptor || visited.has(commit.sha)) return commit.summary;
  if (descriptor.kind === "message") return t(descriptor.key);
  if (descriptor.kind === "cherryPickRevision") {
    return t("demo.commit.cherryPickRevision", { revision: descriptor.revision });
  }
  visited.add(commit.sha);
  const source = browserCommits.find((candidate) => candidate.sha === descriptor.sourceSha);
  const summary = source ? localizeBrowserCommitSummary(source, visited) : descriptor.sourceSha;
  return descriptor.kind === "cherryPick"
    ? t("demo.commit.cherryPick", { summary })
    : t("demo.commit.revert", { summary });
}

function localizeBrowserCommits(commits: CommitSummary[]): CommitSummary[] {
  return rawCopy(commits).map((commit) => ({ ...commit, summary: localizeBrowserCommitSummary(commit) }));
}

function localizeBrowserAccount(account: AccountProfile): AccountProfile {
  const localized = rawCopy(account);
  const key = browserAccountDisplayNameKeys.get(account.id);
  if (key) localized.displayName = t(key);
  return localized;
}

function localizeBrowserAccounts(accounts: AccountProfile[]): AccountProfile[] {
  return accounts.map(localizeBrowserAccount);
}

function localizeBrowserPullRequest(pullRequest: PullRequestSummary): PullRequestSummary {
  const localized = rawCopy(pullRequest);
  const titleKey = browserPullRequestTitleKeys.get(pullRequest.number);
  const updatedAtKey = browserPullRequestUpdatedAtKeys.get(pullRequest.number);
  if (titleKey) localized.title = t(titleKey);
  if (updatedAtKey) localized.updatedAt = t(updatedAtKey);
  return localized;
}

function localizeBrowserPullRequests(pullRequests: PullRequestSummary[]): PullRequestSummary[] {
  return pullRequests.map(localizeBrowserPullRequest);
}

function localizeBrowserStashes(stashes: StashEntry[]): StashEntry[] {
  return rawCopy(stashes).map((stash) => {
    const descriptor = browserStashMessageDescriptors.get(stash.commitId);
    if (!descriptor) return stash;
    const message = descriptor.messageKey ? t(descriptor.messageKey) : descriptor.message;
    return { ...stash, message: t("demo.stash.onBranch", { branch: descriptor.branch, message }) };
  });
}

function isEmptyBrowserDemo(): boolean {
  return import.meta.env.VITE_UI_EMPTY_STATE === "true";
}

function browserError(code: string): AppError {
  return {
    code,
    message: localizedErrorMessage(code),
  };
}

function updateMockSnapshot(updater: (snapshot: RepositorySnapshot) => void): RepositorySnapshot {
  updater(browserSnapshot);
  // Browser confirmations only retain path strings, unlike the native
  // fingerprinted flow. Any intervening mock mutation can therefore replace
  // an untracked entry at the same path, so it invalidates the preview.
  browserCleanUntrackedConfirmation = null;
  browserDiscardChangesConfirmation = null;
  return localizeBrowserSnapshot(browserSnapshot);
}

function requireBrowserSnapshot(repositoryPath: string): RepositorySnapshot {
  if (repositoryPath !== browserSnapshot.repository.path) {
    throw browserError("repository_not_found");
  }
  return browserSnapshot;
}

function requireBrowserRemote(snapshot: RepositorySnapshot, remoteName: string): void {
  if (!snapshot.remotes.some((remote) => remote.name === remoteName)) {
    throw browserError("remote_not_found");
  }
}

function requireBrowserGitHubRemote(repositoryPath: string, remoteName: string): RemoteProfile {
  const snapshot = requireBrowserSnapshot(repositoryPath);
  const remote = snapshot.remotes.find((item) => item.name === remoteName);
  if (!remote || remote.codeHostKind !== "gitHub") {
    throw browserError("adapter_endpoint_mismatch");
  }
  return remote;
}

function requireBrowserAccountForEndpoint(accountProfileId: string, endpointId: string): AccountProfile {
  const account = browserAccounts.find((item) => item.id === accountProfileId);
  if (!account) {
    throw browserError("account_not_found");
  }
  if (account.endpointId !== endpointId) {
    throw browserError("binding_endpoint_mismatch");
  }
  return account;
}

// Browsers normalize an explicitly written scheme-default port away from
// `URL.port`. Endpoint identity intentionally does not: a user may configure
// `git.example:443` separately from `git.example`, so retain the raw URL
// authority's explicit port for the browser-demo validation path as well.
function explicitPortFromUrlAuthority(authority: string): string {
  const hostPort = authority.slice(authority.lastIndexOf("@") + 1);
  const bracketedPort = /^\[[^\]]+\]:(\d+)$/u.exec(hostPort)?.[1];
  return bracketedPort ?? /:(\d+)$/u.exec(hostPort)?.[1] ?? "";
}

function browserRemoteProfile(name: string, fetchUrl: string, pushUrl = fetchUrl): RemoteProfile {
  const ssh = fetchUrl.match(/^(?:[^@/]+@)?(\[[^\]]+\]|[^:/]+):(.+)$/u);
  const sshUrl = fetchUrl.match(/^ssh:\/\/(?:[^@/]+@)?(\[[^\]]+\]|[^/:]+)(?::(\d+))?\/(.+)$/u);
  const http = fetchUrl.match(/^(https?):\/\/([^/]+)\/(.+)$/u);
  const git = fetchUrl.match(/^git:\/\/([^/]+)\/(.+)$/u);
  const local = fetchUrl.startsWith("/")
    || fetchUrl.startsWith("./")
    || fetchUrl.startsWith("../")
    || fetchUrl.startsWith("file://")
    || (!fetchUrl.includes(":") && !fetchUrl.includes("@"));
  let httpUrl: URL | null = null;
  if (http) {
    try {
      httpUrl = new URL(fetchUrl);
    } catch {
      // Keep malformed demo input as an unrecognized remote instead of
      // letting the browser-only fallback throw during a form update.
    }
  }
  const transport = http?.[1] === "https" ? "https" : http?.[1] === "http" ? "http" : ssh || sshUrl ? "ssh" : git ? "git" : local ? "local" : "other";
  const host = httpUrl?.hostname ?? sshUrl?.[1] ?? ssh?.[1] ?? git?.[1] ?? null;
  const portText = http ? explicitPortFromUrlAuthority(http[2]) : sshUrl?.[2] ?? "";
  const parsedPort = /^\d+$/u.test(portText) ? Number(portText) : null;
  const port = parsedPort !== null && parsedPort <= 65535 ? parsedPort : null;
  const path = http?.[3] ?? sshUrl?.[3] ?? ssh?.[2] ?? git?.[2] ?? (local ? fetchUrl.replace(/^file:\/\//u, "") : "");
  const pathParts = path.replace(/\.git$/u, "").split("/").filter(Boolean);
  const repository = pathParts.pop() ?? null;
  const namespace = pathParts.length ? pathParts.join("/") : null;
  const authority = host ? `${host}${port !== null ? `:${port}` : ""}` : null;
  const endpoint = browserEndpoints.find((item) => item.gitHost.toLowerCase() === authority?.toLowerCase());
  return {
    name,
    fetchUrl,
    pushUrl,
    parsed: {
      transport,
      host,
      port,
      namespace,
      repository,
    },
    endpointId: endpoint?.id ?? null,
    codeHostKind: endpoint?.kind ?? "unknown",
  };
}

function browserRemoteUrlMatchesEndpoint(url: string, endpointId: string): boolean {
  const parsed = browserRemoteProfile("", url);
  return (parsed.parsed.transport === "https" || parsed.parsed.transport === "ssh")
    && parsed.endpointId === endpointId;
}

function browserRemoteMatchesEndpoint(remote: RemoteProfile, endpointId: string): boolean {
  return browserRemoteUrlMatchesEndpoint(remote.fetchUrl, endpointId)
    && browserRemoteUrlMatchesEndpoint(remote.pushUrl, endpointId);
}

function isValidBrowserRemoteName(value: string): boolean {
  return value.length > 0
    && value.trim() === value
    && !value.startsWith("-")
    && value !== "@"
    && !/[\u0000-\u001F\u007F-\u009F]/u.test(value)
    && !value.includes(" ")
    && !/[~^:?*[\\]/u.test(value)
    && !value.includes("..")
    && !value.includes("@{")
    && !value.startsWith("/")
    && !value.endsWith("/")
    && value.split("/").every((component) =>
      component.length > 0
      && !component.startsWith(".")
      && !component.endsWith(".")
      && !component.endsWith(".lock"));
}

function isValidBrowserGitHubRouteSegment(value: string): boolean {
  return value.length > 0
    && value.length <= 100
    && value !== "."
    && value !== ".."
    && /^[A-Za-z0-9._-]+$/u.test(value);
}

export const api = {
  async openExternalUrl(url: string): Promise<void> {
    await request("open_external_url", { url }, () => {
      const trimmedUrl = url.trim();
      const parsed = new URL(trimmedUrl);
      const serialized = parsed.toString();
      if (
        !(["http:", "https:"] as string[]).includes(parsed.protocol)
        || !parsed.hostname
        || parsed.username
        || parsed.password
        || serialized !== `${parsed.origin}${parsed.pathname}`
        || new TextEncoder().encode(trimmedUrl).length > 4096
      ) {
        throw browserError("command_failed");
      }
      window.open(serialized, "_blank", "noopener,noreferrer");
    });
  },

  listRepositories(refreshOwners = false): Promise<RepositoryRecord[]> {
    return request("list_repositories", { refreshOwners }, () => (isEmptyBrowserDemo() ? [] : [localizeBrowserRepository(browserSnapshot.repository)]));
  },

  discoverRepositories(rootPath: string): Promise<RepositoryRecord[]> {
    return request("discover_repositories", { rootPath }, () => (isEmptyBrowserDemo() ? [] : [localizeBrowserRepository(browserSnapshot.repository)]));
  },

  initRepository(input: InitRepositoryInput): Promise<RepositoryRecord> {
    return request("init_repository", input, () => {
      const destinationPath = input.destinationPath.replace(/[\\/]$/, "");
      if (!destinationPath) throw browserError("invalid_init_destination");
      const name = destinationPath.split(/[\\/]/u).filter(Boolean).pop() || "new-repository";
      browserSnapshot = rawCopy({
        ...mockSnapshot,
        repository: { path: destinationPath, name, alias: null, group: null, host: null, namespace: null, lastOpenedAt: null },
        hasHead: false,
        status: { ...mockSnapshot.status, headCommit: null, branch: input.initialBranch?.trim() || "main", upstream: null, upstreamCommit: null, ahead: 0, behind: 0, entries: [] },
        branches: [{ name: input.initialBranch?.trim() || "main", isCurrent: true, upstream: null }],
        remoteBranches: [],
        remotes: [],
        bindings: [],
        tags: [],
      });
      browserCommits = [];
      browserBranchTips = new Map([[input.initialBranch?.trim() || "main", null]]);
      browserRemoteBranchTips = new Map();
      browserWorktrees = [
        {
          path: destinationPath,
          head: "",
          branch: input.initialBranch?.trim() || "main",
          isMain: true,
          isDetached: false,
          isLocked: false,
          isPrunable: false,
        },
      ];
      browserSubmodules = [];
      browserLfsStatus = { available: true, initialized: false, version: "git-lfs/3.6.1 (browser demo)", files: [] };
      browserStashes = [];
      browserPullRequests = [];
      browserRepositoryAliasKey = null;
      browserRepositoryGroupKey = null;
      browserCommitSummaryDescriptors.clear();
      browserPullRequestTitleKeys.clear();
      browserPullRequestUpdatedAtKeys.clear();
      browserStashMessageDescriptors.clear();
      return localizeBrowserRepository(browserSnapshot.repository);
    });
  },

  cloneRepository(input: CloneRepositoryInput): Promise<RepositoryRecord> {
    return request("clone_repository", input, () => {
      const pathname = input.destinationPath.replace(/[\\/]$/, "");
      const name = pathname.split(/[\\/]/u).filter(Boolean).pop() || "cloned-repository";
      const branch = "main";
      browserSnapshot = rawCopy({
        ...mockSnapshot,
        repository: { path: pathname, name, alias: null, group: null, host: null, namespace: null, lastOpenedAt: null },
        hasHead: false,
        status: { ...mockSnapshot.status, headCommit: null, branch, upstream: null, upstreamCommit: null, ahead: 0, behind: 0, entries: [] },
        branches: [{ name: branch, isCurrent: true, upstream: null }],
        remoteBranches: [{ remoteName: "origin", name: branch }],
        remotes: [browserRemoteProfile("origin", input.url)],
        bindings: [],
        tags: [],
        operation: null,
      });
      browserCommits = [];
      browserBranchTips = new Map([[branch, null]]);
      browserRemoteBranchTips = new Map([[`origin/${branch}`, null]]);
      browserWorktrees = [
        {
          path: pathname,
          head: "",
          branch,
          isMain: true,
          isDetached: false,
          isLocked: false,
          isPrunable: false,
        },
      ];
      browserSubmodules = [];
      browserLfsStatus = { available: true, initialized: false, version: "git-lfs/3.6.1 (browser demo)", files: [] };
      browserStashes = [];
      browserPullRequests = [];
      browserRepositoryAliasKey = null;
      browserRepositoryGroupKey = null;
      browserCommitSummaryDescriptors.clear();
      browserPullRequestTitleKeys.clear();
      browserPullRequestUpdatedAtKeys.clear();
      browserStashMessageDescriptors.clear();
      return localizeBrowserRepository(browserSnapshot.repository);
    });
  },

  getRepositorySnapshot(repositoryPath: string): Promise<RepositorySnapshot> {
    return request("get_repository_snapshot", { repositoryPath }, () => {
      const worktree = browserWorktrees.find((entry) => entry.path === repositoryPath);
      if (worktree && browserSnapshot.repository.path !== repositoryPath) {
        browserSnapshot.repository = {
          path: worktree.path,
          name: worktree.path.split(/[\\/]/u).filter(Boolean).pop() || "worktree",
          alias: null,
          group: null,
          host: null,
          namespace: null,
          lastOpenedAt: null,
        };
        browserSnapshot.status.branch = worktree.branch;
        browserSnapshot.branches.forEach((branch) => {
          branch.isCurrent = branch.name === worktree.branch;
        });
        browserSnapshot.status.upstream = browserSnapshot.branches.find((branch) => branch.isCurrent)?.upstream ?? null;
        browserRepositoryAliasKey = null;
        browserRepositoryGroupKey = null;
        syncBrowserBranchStatus(browserSnapshot);
        if (worktree.isDetached) {
          browserSnapshot.hasHead = Boolean(worktree.head);
          browserSnapshot.status.headCommit = worktree.head || null;
          browserSnapshot.status.upstream = null;
          browserSnapshot.status.upstreamCommit = null;
          browserSnapshot.status.ahead = 0;
          browserSnapshot.status.behind = 0;
        }
      }
      const ownerRemote = browserSnapshot.remotes.find(
        (remote) => remote.name === "origin" && remote.parsed.host && remote.parsed.namespace,
      ) ?? browserSnapshot.remotes.find((remote) => remote.parsed.host && remote.parsed.namespace);
      browserSnapshot.repository.host = ownerRemote?.parsed.host ?? null;
      browserSnapshot.repository.namespace = ownerRemote?.parsed.namespace ?? null;
      browserSnapshot.repository.lastOpenedAt = Date.now();
      return localizeBrowserSnapshot(requireBrowserSnapshot(repositoryPath));
    });
  },

  refreshRepositorySnapshot(repositoryPath: string): Promise<RepositorySnapshot> {
    return request("refresh_repository_snapshot", { repositoryPath }, () => localizeBrowserSnapshot(requireBrowserSnapshot(repositoryPath)));
  },

  watchRepositoryChanges(repositoryPath: string, requestGeneration: number): Promise<void> {
    return request("watch_repository_changes", { repositoryPath, requestGeneration }, () => undefined);
  },

  stopRepositoryWatcher(requestGeneration: number): Promise<void> {
    return request("stop_repository_watcher", { requestGeneration }, () => undefined);
  },

  setRepositoryGroup(repositoryPath: string, group: string | null): Promise<RepositoryRecord> {
    return request("set_repository_group", { repositoryPath, group }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      const nextGroup = group?.trim() || null;
      if (nextGroup && Array.from(nextGroup).length > 80) {
        throw browserError("repository_group_invalid");
      }
      snapshot.repository.group = nextGroup;
      browserRepositoryGroupKey = null;
      return localizeBrowserRepository(snapshot.repository);
    });
  },

  setRepositoryAlias(repositoryPath: string, alias: string | null): Promise<RepositoryRecord> {
    return request("set_repository_alias", { repositoryPath, alias }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      const nextAlias = alias?.trim() || null;
      if (nextAlias && Array.from(nextAlias).length > 80) {
        throw browserError("repository_alias_invalid");
      }
      snapshot.repository.alias = nextAlias;
      browserRepositoryAliasKey = null;
      return localizeBrowserRepository(snapshot.repository);
    });
  },

  getFileDiff(repositoryPath: string, path: string, staged = false): Promise<string> {
    return request("get_file_diff", { repositoryPath, path, staged }, () => {
      requireBrowserSnapshot(repositoryPath);
      const target = path || "README.md";
      return [
        `diff --git a/${target} b/${target}`,
        `--- a/${target}`,
        `+++ b/${target}`,
        "@@ -1,3 +1,5 @@",
        " export default {",
        `-  title: '${t("demo.diff.oldImplementation")}',`,
        "+  title: 'BroKnowMyGitDesktop',",
        "+  multiAccount: true,",
        " }",
      ].join("\n");
    });
  },

  getFileBlame(repositoryPath: string, path: string): Promise<string> {
    return request("get_file_blame", { repositoryPath, path }, () => {
      requireBrowserSnapshot(repositoryPath);
      const target = path || "README.md";
      return [
        "9d7a4f0c2ea (BroKnowMy 2026-07-19)  1) # " + target,
        `55b91f2b0e1 (BroKnowMy 2026-07-18)  2) ${t("demo.blame.sample")}`,
      ].join("\n");
    });
  },

  listReflog(repositoryPath: string, limit = 100): Promise<string> {
    return request("list_reflog", { repositoryPath, limit }, () => {
      requireBrowserSnapshot(repositoryPath);
      return [
        `9d7a4f0 HEAD@{0} commit: ${t("demo.commit.multiAccount")} (BroKnowMy, 2026-07-19T10:20:30+08:00)`,
        `55b91f2 HEAD@{1} commit (initial): ${t("demo.commit.initial")} (BroKnowMy, 2026-07-18T09:00:00+08:00)`,
      ].join("\n");
    });
  },

  previewCleanUntracked(repositoryPath: string): Promise<CleanUntrackedPreview> {
    return request("preview_clean_untracked", { repositoryPath }, () => {
      requireBrowserSnapshot(repositoryPath);
      const confirmation = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
      const paths = new Set(
        browserSnapshot.status.entries
          .filter((entry) => !entry.indexStatus && entry.worktreeStatus === "?")
          .map((entry) => entry.path),
      );
      browserCleanUntrackedConfirmation = { repositoryPath, token: confirmation, paths };
      return {
        preview: [...paths].map((path) => "Would remove " + path).join("\n"),
        confirmation,
      };
    });
  },

  cleanUntracked(repositoryPath: string, confirmation: string): Promise<RepositorySnapshot> {
    return request("clean_untracked", { repositoryPath, confirmation }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (
        !confirmation
        || confirmation !== browserCleanUntrackedConfirmation?.token
        || repositoryPath !== browserCleanUntrackedConfirmation?.repositoryPath
      ) {
        throw browserError("clean_untracked_confirmation_expired");
      }
      const previewedPaths = browserCleanUntrackedConfirmation.paths;
      browserCleanUntrackedConfirmation = null;
      return updateMockSnapshot((snapshot) => {
        snapshot.status.entries = snapshot.status.entries.filter(
          (entry) => entry.indexStatus || entry.worktreeStatus !== "?" || !previewedPaths.has(entry.path),
        );
      });
    });
  },

  getExternalEditor(): Promise<ExternalEditorConfig | null> {
    return request("get_external_editor", {}, () => (browserExternalEditor ? copy(browserExternalEditor) : null));
  },

  saveExternalEditor(editor: ExternalEditorConfig | null): Promise<ExternalEditorConfig | null> {
    return request("save_external_editor", { editor }, () => {
      if (editor === null) {
        browserExternalEditor = null;
        return null;
      }
      const executable = editor.executable.trim();
      const editorArguments = editor.arguments.map((argument) => argument.trim()).filter(Boolean);
      const isAbsoluteExecutable = /^(?:[A-Za-z]:[\\/]|\\\\[^\\/]+[\\/][^\\/]+|\/)/u.test(executable);
      if (!executable || !isAbsoluteExecutable || editorArguments.length > 32) {
        throw browserError("external_editor_config_invalid");
      }
      browserExternalEditor = { executable, arguments: editorArguments };
      return copy(browserExternalEditor);
    });
  },

  openConflictedFilesInExternalEditor(repositoryPath: string): Promise<string[]> {
    return request("open_conflicted_files_in_external_editor", { repositoryPath }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!browserExternalEditor) {
        throw browserError("external_editor_not_configured");
      }
      const conflicts = browserSnapshot.status.entries
        .filter((entry) => ["DD", "AU", "UD", "UA", "DU", "AA", "UU"].includes(entry.indexStatus + entry.worktreeStatus))
        .map((entry) => entry.path);
      if (!conflicts.length) {
        throw browserError("no_conflicted_files");
      }
      return conflicts;
    });
  },

  listCommits(repositoryPath: string, limit = 100): Promise<CommitSummary[]> {
    return request("list_commits", { repositoryPath, limit }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      return localizeBrowserCommits(browserHistoryCommits(snapshot).slice(0, limit));
    });
  },

  getCommitDiff(repositoryPath: string, revision: string): Promise<string> {
    return request("get_commit_diff", { repositoryPath, revision }, () => {
      requireBrowserSnapshot(repositoryPath);
      const commit = browserCommits.find((entry) => entry.sha === revision || entry.shortSha === revision);
      if (!commit) throw browserError("invalid_git_revision");
      return [
        `commit ${commit.sha}`,
        `Author: ${commit.authorName} <${commit.authorEmail}>`,
        `Date:   ${commit.authoredAt}`,
        "",
        `    ${localizeBrowserCommitSummary(commit)}`,
        "",
        "diff --git a/README.md b/README.md",
        "--- a/README.md",
        "+++ b/README.md",
        "@@ -1 +1,2 @@",
        " # BroKnowMyGitDesktop",
        `+${t("demo.diff.historyDemo")}`,
      ].join("\n");
    });
  },

  listCommitChangedFiles(repositoryPath: string, revision: string): Promise<CommitChangedFile[]> {
    return request("list_commit_changed_files", { repositoryPath, revision }, () => {
      requireBrowserSnapshot(repositoryPath);
      const commit = browserCommits.find((entry) => entry.sha === revision || entry.shortSha === revision);
      if (!commit) throw browserError("invalid_git_revision");
      return copy(browserCommitChangedFiles(commit));
    });
  },

  getCommitFileDiff(
    repositoryPath: string,
    revision: string,
    path: string,
    previousPath: string | null = null,
    maximumBytes = 512 * 1024,
  ): Promise<CommitFileDiff> {
    return request("get_commit_file_diff", { repositoryPath, revision, path, previousPath, maximumBytes }, () => {
      requireBrowserSnapshot(repositoryPath);
      const commit = browserCommits.find((entry) => entry.sha === revision || entry.shortSha === revision);
      if (!commit) throw browserError("invalid_git_revision");
      const changedFile = browserCommitChangedFiles(commit).find((file) => file.path === path);
      if (!changedFile || changedFile.previousPath !== previousPath) {
        throw browserError("commit_file_not_found");
      }
      if (previousPath) {
        return boundedBrowserCommitFileDiff([
          `diff --git a/${previousPath} b/${path}`,
          `similarity index 92%`,
          `rename from ${previousPath}`,
          `rename to ${path}`,
        ].join("\n"), maximumBytes);
      }
      if (changedFile.status === "added") {
        return boundedBrowserCommitFileDiff([
          `diff --git a/${path} b/${path}`,
          "new file mode 100644",
          "--- /dev/null",
          `+++ b/${path}`,
          "@@ -0,0 +1 @@",
          `+${t("demo.diff.historyByFile")}`,
        ].join("\n"), maximumBytes);
      }
      return boundedBrowserCommitFileDiff([
        `diff --git a/${path} b/${path}`,
        `--- a/${path}`,
        `+++ b/${path}`,
        "@@ -1 +1,2 @@",
        ` # ${path}`,
        `+${t("demo.diff.historyByFile")}`,
      ].join("\n"), maximumBytes);
    });
  },

  stagePaths(repositoryPath: string, paths: string[]): Promise<RepositorySnapshot> {
    return request("stage_paths", { repositoryPath, paths }, () => {
      requireBrowserSnapshot(repositoryPath);
      return updateMockSnapshot((snapshot) => {
        for (const entry of snapshot.status.entries) {
          if (paths.includes(entry.path)) {
            entry.indexStatus = entry.indexStatus || "M";
            entry.worktreeStatus = "";
          }
        }
      });
    });
  },

  unstagePaths(repositoryPath: string, paths: string[]): Promise<RepositorySnapshot> {
    return request("unstage_paths", { repositoryPath, paths }, () => {
      requireBrowserSnapshot(repositoryPath);
      return updateMockSnapshot((snapshot) => {
        for (const entry of snapshot.status.entries) {
          if (paths.includes(entry.path) && entry.indexStatus) {
            entry.worktreeStatus = entry.indexStatus;
            entry.indexStatus = "";
          }
        }
      });
    });
  },

  applyPatch(repositoryPath: string, path: string, patch: string, reverse: boolean): Promise<RepositorySnapshot> {
    return request("apply_patch", { repositoryPath, path, patch, reverse }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!patch.startsWith("diff --git ")) {
        throw browserError("invalid_git_patch");
      }
      const diffHeaders = patch.match(/^diff --git [^\r\n]+$/gmu) ?? [];
      const headerPaths = diffHeaders.length === 1 ? parseGitDiffHeaderPaths(diffHeaders[0]) : null;
      if (!headerPaths || headerPaths[0] !== `a/${path}` || headerPaths[1] !== `b/${path}`) {
        throw browserError("invalid_git_patch");
      }
      return updateMockSnapshot((snapshot) => {
        const entry = snapshot.status.entries.find((candidate) => candidate.path === path);
        if (!entry) throw browserError("invalid_git_patch");
        if (reverse) {
          entry.worktreeStatus = entry.worktreeStatus || entry.indexStatus || "M";
          entry.indexStatus = "";
        } else {
          entry.indexStatus = entry.indexStatus || "M";
          entry.worktreeStatus = "";
        }
      });
    });
  },

  previewDiscardChanges(repositoryPath: string, paths: string[]): Promise<DiscardChangesPreview> {
    return request("preview_discard_changes", { repositoryPath, paths }, () => {
      requireBrowserSnapshot(repositoryPath);
      const selected = new Set(paths);
      const entries = browserSnapshot.status.entries.filter((entry) => selected.has(entry.path));
      if (!entries.length) throw browserError("nothing_to_discard");
      const confirmation = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
      browserDiscardChangesConfirmation = {
        repositoryPath,
        token: confirmation,
        paths: selected,
        issuedAt: Date.now(),
      };
      return {
        confirmation,
        pathCount: entries.length,
        untrackedCount: entries.filter((entry) => !entry.indexStatus && entry.worktreeStatus === "?").length,
      };
    });
  },

  discardChanges(repositoryPath: string, confirmation: string): Promise<RepositorySnapshot> {
    return request("discard_changes", { repositoryPath, confirmation }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (
        !confirmation
        || confirmation !== browserDiscardChangesConfirmation?.token
        || repositoryPath !== browserDiscardChangesConfirmation?.repositoryPath
      ) {
        throw browserError("discard_confirmation_required");
      }
      if (Date.now() - browserDiscardChangesConfirmation.issuedAt > browserDiscardConfirmationTtlMs) {
        browserDiscardChangesConfirmation = null;
        throw browserError("discard_confirmation_expired");
      }
      const selected = browserDiscardChangesConfirmation.paths;
      browserDiscardChangesConfirmation = null;
      return updateMockSnapshot((snapshot) => {
        snapshot.status.entries = snapshot.status.entries.filter((entry) => !selected.has(entry.path));
      });
    });
  },

  commitChanges(repositoryPath: string, message: string, amend = false): Promise<RepositorySnapshot> {
    return request("commit_changes", { repositoryPath, message, amend }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!message.trim()) {
        throw browserError("commit_message_required");
      }
      return updateMockSnapshot((snapshot) => {
        const stagedEntries = snapshot.status.entries.filter((entry) => Boolean(entry.indexStatus));
        if (!stagedEntries.length && !amend) {
          throw browserError("nothing_to_commit");
        }
        const sha = createBrowserMockSha();
        const currentTip = snapshot.status.branch ? browserBranchTips.get(snapshot.status.branch) : null;
        const currentCommit = currentTip ? browserCommits.find((commit) => commit.sha === currentTip) : undefined;
        if (amend && (!snapshot.hasHead || !currentCommit)) {
          throw browserError("no_commit_to_amend");
        }
        const nextCommit: CommitSummary = {
          sha,
          shortSha: sha.slice(0, 7),
          parents: amend && currentCommit ? currentCommit.parents : currentTip ? [currentTip] : [],
          tags: [],
          tagsTruncated: false,
          pushed: false,
          authorName: "BroKnowMy",
          authorEmail: "broknowmy@example.test",
          authoredAt: new Date().toISOString(),
          summary: message.trim().split(/\r?\n/u, 1)[0],
        };
        browserCommits = [nextCommit, ...browserCommits];
        setBrowserCurrentBranchTip(snapshot, nextCommit.sha);
        snapshot.status.entries = snapshot.status.entries.flatMap((entry) => {
          if (!entry.indexStatus) return [entry];
          return entry.worktreeStatus ? [{ ...entry, indexStatus: "" }] : [];
        });
        snapshot.hasHead = true;
        syncBrowserBranchStatus(snapshot);
      });
    });
  },

  createBranch(input: CreateBranchInput): Promise<CreateBranchResult> {
    return request("create_branch", input, () => {
      requireBrowserSnapshot(input.repositoryPath);
      if (!input.name.trim()) {
        throw browserError("branch_name_required");
      }
      let stashedLocalChanges = false;
      const snapshot = updateMockSnapshot((snapshot) => {
        const name = input.name.trim();
        if (snapshot.branches.some((branch) => branch.name === name)) {
          throw browserError("branch_already_exists");
        }
        if (input.checkout && input.stashLocalChanges) {
          stashedLocalChanges = stashBrowserChanges(snapshot, "", "demo.stash.autoBeforeSwitch");
        }
        const sourceTip = snapshot.status.branch ? browserBranchTips.get(snapshot.status.branch) ?? null : null;
        if (input.checkout) {
          snapshot.branches.forEach((branch) => {
            branch.isCurrent = false;
          });
          snapshot.status.branch = name;
          snapshot.status.upstream = null;
        }
        snapshot.branches.push({ name, isCurrent: input.checkout, upstream: null });
        browserBranchTips.set(name, sourceTip);
        if (input.checkout) syncBrowserBranchStatus(snapshot);
      });
      return { snapshot, stashedLocalChanges };
    });
  },

  checkoutBranch(repositoryPath: string, branch: string, stashLocalChanges = false): Promise<RepositorySnapshot> {
    return request("checkout_branch", { repositoryPath, branch, stashLocalChanges }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!browserSnapshot.branches.some((entry) => entry.name === branch)) {
        throw browserError("branch_not_found");
      }
      return updateMockSnapshot((snapshot) => {
        if (stashLocalChanges) stashBrowserChanges(snapshot, "", "demo.stash.autoBeforeSwitch");
        snapshot.branches.forEach((entry) => {
          entry.isCurrent = entry.name === branch;
        });
        snapshot.status.branch = branch;
        snapshot.status.upstream = snapshot.branches.find((entry) => entry.name === branch)?.upstream ?? null;
        syncBrowserBranchStatus(snapshot);
      });
    });
  },

  checkoutRemoteBranch(
    repositoryPath: string,
    remoteName: string,
    branch: string,
    stashLocalChanges = false,
  ): Promise<RepositorySnapshot> {
    return request("checkout_remote_branch", { repositoryPath, remoteName, branch, stashLocalChanges }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      requireBrowserRemote(snapshot, remoteName);
      if (!snapshot.remoteBranches.some((entry) => entry.remoteName === remoteName && entry.name === branch)) {
        throw browserError("remote_branch_not_found");
      }
      return updateMockSnapshot((nextSnapshot) => {
        if (stashLocalChanges) stashBrowserChanges(nextSnapshot, "", "demo.stash.autoBeforeSwitch");
        let localBranch = nextSnapshot.branches.find((entry) => entry.name === branch);
        if (!localBranch) {
          localBranch = { name: branch, isCurrent: false, upstream: `${remoteName}/${branch}` };
          nextSnapshot.branches.push(localBranch);
          browserBranchTips.set(branch, browserRemoteBranchTips.get(`${remoteName}/${branch}`) ?? null);
        }
        nextSnapshot.branches.forEach((entry) => {
          entry.isCurrent = entry === localBranch;
        });
        nextSnapshot.status.branch = localBranch.name;
        nextSnapshot.status.upstream = localBranch.upstream;
        syncBrowserBranchStatus(nextSnapshot);
      });
    });
  },

  renameBranch(repositoryPath: string, oldName: string, newName: string): Promise<RepositorySnapshot> {
    return request("rename_branch", { repositoryPath, oldName, newName }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!newName.trim()) throw browserError("invalid_git_reference");
      return updateMockSnapshot((snapshot) => {
        const branch = snapshot.branches.find((entry) => entry.name === oldName);
        if (!branch) throw browserError("branch_not_found");
        const name = newName.trim();
        if (name !== oldName && snapshot.branches.some((entry) => entry.name === name)) {
          throw browserError("branch_already_exists");
        }
        branch.name = name;
        const tip = browserBranchTips.get(oldName) ?? null;
        browserBranchTips.delete(oldName);
        browserBranchTips.set(name, tip);
        if (snapshot.status.branch === oldName) snapshot.status.branch = name;
      });
    });
  },

  deleteBranch(repositoryPath: string, name: string, force: boolean): Promise<RepositorySnapshot> {
    return request("delete_branch", { repositoryPath, name, force }, () => {
      requireBrowserSnapshot(repositoryPath);
      return updateMockSnapshot((snapshot) => {
        const branch = snapshot.branches.find((entry) => entry.name === name);
        if (!branch) throw browserError("branch_not_found");
        if (branch.isCurrent) throw browserError("cannot_delete_current_branch");
        snapshot.branches = snapshot.branches.filter((entry) => entry.name !== name);
        browserBranchTips.delete(name);
      });
    });
  },

  mergeBranch(repositoryPath: string, branch: string): Promise<void> {
    return request("merge_branch", { repositoryPath, branch }, () => {
      requireBrowserSnapshot(repositoryPath);
    });
  },

  listWorktrees(repositoryPath: string): Promise<WorktreeInfo[]> {
    return request("list_worktrees", { repositoryPath }, () => {
      requireBrowserSnapshot(repositoryPath);
      return copy(browserWorktrees);
    });
  },

  addWorktree(
    repositoryPath: string,
    worktreePath: string,
    createBranch: string | undefined,
    commitish: string | undefined,
  ): Promise<WorktreeInfo[]> {
    return request("add_worktree", { repositoryPath, worktreePath, createBranch, commitish }, () => {
      requireBrowserSnapshot(repositoryPath);
      const target = worktreePath.trim();
      if (!target) throw browserError("invalid_worktree_path");
      if (!createBranch?.trim() && !commitish?.trim()) {
        throw browserError("invalid_worktree_config");
      }
      if (browserWorktrees.some((entry) => entry.path === target)) {
        throw browserError("git_command_failed");
      }
      const newBranch = createBranch?.trim();
      if (newBranch && browserSnapshot.branches.some((entry) => entry.name === newBranch)) {
        throw browserError("branch_already_exists");
      }
      const currentTip = browserSnapshot.status.branch
        ? browserBranchTips.get(browserSnapshot.status.branch) ?? null
        : null;
      const normalizedCommitish = commitish?.trim();
      const startTip = normalizedCommitish
        ? browserBranchTips.get(normalizedCommitish)
          ?? browserRemoteBranchTips.get(normalizedCommitish)
          ?? browserSnapshot.tags.find((tag) => tag.name === normalizedCommitish)?.target
          ?? browserCommits.find(
            (commit) => commit.sha === normalizedCommitish || commit.shortSha === normalizedCommitish,
          )?.sha
          ?? null
        : currentTip;
      if (normalizedCommitish && !startTip) {
        throw browserError("invalid_git_revision");
      }
      const branch = newBranch
        ?? (normalizedCommitish && browserBranchTips.has(normalizedCommitish)
          ? normalizedCommitish
          : null);
      if (newBranch) {
        browserSnapshot.branches.push({ name: newBranch, isCurrent: false, upstream: null });
        browserBranchTips.set(newBranch, startTip);
      }
      browserWorktrees = [
        ...browserWorktrees,
        {
          path: target,
          head: startTip || "",
          branch,
          isMain: false,
          isDetached: branch === null,
          isLocked: false,
          isPrunable: false,
        },
      ];
      return copy(browserWorktrees);
    });
  },

  removeWorktree(repositoryPath: string, worktreePath: string, force: boolean): Promise<WorktreeInfo[]> {
    return request("remove_worktree", { repositoryPath, worktreePath, force }, () => {
      requireBrowserSnapshot(repositoryPath);
      const worktree = browserWorktrees.find((entry) => entry.path === worktreePath);
      if (!worktree) throw browserError("worktree_not_found");
      if (worktree.isMain) throw browserError("cannot_remove_main_worktree");
      if (worktree.path === browserSnapshot.repository.path) {
        throw browserError("cannot_remove_active_worktree");
      }
      browserWorktrees = browserWorktrees.filter((entry) => entry.path !== worktreePath);
      return copy(browserWorktrees);
    });
  },

  listSubmodules(repositoryPath: string): Promise<SubmoduleInfo[]> {
    return request("list_submodules", { repositoryPath }, () => {
      requireBrowserSnapshot(repositoryPath);
      return copy(browserSubmodules);
    });
  },

  updateSubmodules(repositoryPath: string, initialize: boolean, recursive: boolean): Promise<SubmoduleInfo[]> {
    return request("update_submodules", { repositoryPath, initialize, recursive }, () => {
      requireBrowserSnapshot(repositoryPath);
      return copy(browserSubmodules);
    });
  },

  getLfsStatus(repositoryPath: string): Promise<LfsStatus> {
    return request("get_lfs_status", { repositoryPath }, () => {
      requireBrowserSnapshot(repositoryPath);
      return copy(browserLfsStatus);
    });
  },

  installLfs(repositoryPath: string): Promise<LfsStatus> {
    return request("install_lfs", { repositoryPath }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!browserLfsStatus.available) throw browserError("lfs_unavailable");
      browserLfsStatus.initialized = true;
      return copy(browserLfsStatus);
    });
  },

  trackLfsPattern(repositoryPath: string, pattern: string): Promise<RepositorySnapshot> {
    return request("track_lfs_pattern", { repositoryPath, pattern }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!browserLfsStatus.available) throw browserError("lfs_unavailable");
      if (!pattern.trim() || pattern.trim().startsWith("-")) {
        throw browserError("invalid_lfs_pattern");
      }
      return updateMockSnapshot((snapshot) => {
        if (!snapshot.status.entries.some((entry) => entry.path === ".gitattributes")) {
          snapshot.status.entries.push({ path: ".gitattributes", originalPath: null, indexStatus: "", worktreeStatus: "M" });
        }
      });
    });
  },

  startRebase(repositoryPath: string, base: string): Promise<RepositorySnapshot> {
    return request("start_rebase", { repositoryPath, base }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!base.trim()) throw browserError("invalid_git_revision");
      return updateMockSnapshot((snapshot) => {
        if (snapshot.operation) throw browserError("git_operation_in_progress");
        snapshot.operation = { kind: "rebase", hasConflicts: false };
      });
    });
  },

  cherryPickCommits(repositoryPath: string, revisions: string[]): Promise<RepositorySnapshot> {
    return request("cherry_pick_commits", { repositoryPath, revisions }, () => {
      requireBrowserSnapshot(repositoryPath);
      const selected = revisions.map((revision) => revision.trim()).filter(Boolean);
      if (!selected.length || selected.length > 100) throw browserError("invalid_cherry_pick");
      if (browserSnapshot.operation) throw browserError("git_operation_in_progress");
      let currentTip = browserSnapshot.status.branch
        ? browserBranchTips.get(browserSnapshot.status.branch) ?? null
        : null;
      for (const revision of selected) {
        const source = browserCommits.find((commit) => commit.sha === revision || commit.shortSha === revision);
        const sha = createBrowserMockSha();
        browserCommitSummaryDescriptors.set(sha, source
          ? { kind: "cherryPick", sourceSha: source.sha }
          : { kind: "cherryPickRevision", revision });
        const commit: CommitSummary = {
          sha,
          shortSha: sha.slice(0, 7),
          parents: currentTip ? [currentTip] : [],
          tags: [],
          tagsTruncated: false,
          pushed: false,
          authorName: "BroKnowMy",
          authorEmail: "broknowmy@example.test",
          authoredAt: new Date().toISOString(),
          summary: `Cherry-pick ${source?.shortSha ?? revision}`,
        };
        browserCommits = [commit, ...browserCommits];
        currentTip = sha;
      }
      return updateMockSnapshot((snapshot) => {
        setBrowserCurrentBranchTip(snapshot, currentTip!);
        syncBrowserBranchStatus(snapshot);
      });
    });
  },

  continueGitOperation(repositoryPath: string, kind: GitOperationKind): Promise<RepositorySnapshot> {
    return request("continue_git_operation", { repositoryPath, kind }, () => {
      requireBrowserSnapshot(repositoryPath);
      return updateMockSnapshot((snapshot) => {
        if (!snapshot.operation || snapshot.operation.kind !== kind) {
          throw browserError("git_operation_not_in_progress");
        }
        if (snapshot.operation.hasConflicts) {
          throw browserError("git_merge_conflict");
        }
        snapshot.operation = null;
      });
    });
  },

  abortGitOperation(repositoryPath: string, kind: GitOperationKind): Promise<RepositorySnapshot> {
    return request("abort_git_operation", { repositoryPath, kind }, () => {
      requireBrowserSnapshot(repositoryPath);
      return updateMockSnapshot((snapshot) => {
        if (!snapshot.operation || snapshot.operation.kind !== kind) {
          throw browserError("git_operation_not_in_progress");
        }
        snapshot.operation = null;
      });
    });
  },

  listStashes(repositoryPath: string): Promise<StashEntry[]> {
    return request("list_stashes", { repositoryPath }, () => {
      requireBrowserSnapshot(repositoryPath);
      return localizeBrowserStashes(browserStashes.map(({ reference, commitId, message }) => ({ reference, commitId, message })));
    });
  },

  stashChanges(repositoryPath: string, message: string | undefined, includeUntracked: boolean): Promise<RepositorySnapshot> {
    return request("stash_changes", { repositoryPath, message, includeUntracked }, () => {
      requireBrowserSnapshot(repositoryPath);
      const hasStashableChanges = browserSnapshot.status.entries.some(
        (entry) => includeUntracked || !isBrowserUntrackedEntry(entry),
      );
      if (!hasStashableChanges) throw browserError("nothing_to_stash");
      const suffix = message?.trim() || "WIP";
      return updateMockSnapshot((snapshot) => {
        stashBrowserChanges(snapshot, suffix, undefined, includeUntracked);
      });
    });
  },

  popStash(repositoryPath: string, reference: string, expectedCommitId: string): Promise<RepositorySnapshot> {
    return request("pop_stash", { repositoryPath, reference, expectedCommitId }, () => {
      requireBrowserSnapshot(repositoryPath);
      const index = browserStashes.findIndex((entry) => entry.reference === reference && entry.commitId === expectedCommitId);
      if (index === -1) throw browserError("stash_not_found");
      const stashedPaths = new Set(browserStashes[index].entries.map((entry) => entry.path));
      if (browserSnapshot.status.entries.some((entry) => stashedPaths.has(entry.path))) {
        throw browserError("git_merge_conflict");
      }
      const [stash] = browserStashes.splice(index, 1);
      browserStashMessageDescriptors.delete(stash.commitId);
      renumberBrowserStashes();
      return updateMockSnapshot((snapshot) => {
        snapshot.status.entries = [...copy(stash.entries), ...snapshot.status.entries];
      });
    });
  },

  dropStash(repositoryPath: string, reference: string, expectedCommitId: string): Promise<void> {
    return request("drop_stash", { repositoryPath, reference, expectedCommitId }, () => {
      requireBrowserSnapshot(repositoryPath);
      for (const stash of browserStashes) {
        if (stash.reference === reference && stash.commitId === expectedCommitId) {
          browserStashMessageDescriptors.delete(stash.commitId);
        }
      }
      browserStashes = browserStashes.filter((entry) => entry.reference !== reference || entry.commitId !== expectedCommitId);
      renumberBrowserStashes();
    });
  },

  resetBranch(repositoryPath: string, target: string, mode: ResetMode): Promise<RepositorySnapshot> {
    return request("reset_branch", { repositoryPath, target, mode }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!browserCommits.some((commit) => commit.sha === target || commit.shortSha === target)) {
        throw browserError("invalid_git_revision");
      }
      throw browserError("browser_demo_unsupported");
    });
  },

  squashRecentCommits(repositoryPath: string, count: number, message: string): Promise<RepositorySnapshot> {
    return request("squash_recent_commits", { repositoryPath, count, message }, () => {
      requireBrowserSnapshot(repositoryPath);
      const branchCommits = browserBranchCommits(browserSnapshot.status.branch);
      if (!Number.isInteger(count) || count < 2 || count > 1000 || branchCommits.length <= count) {
        throw browserError("invalid_squash_count");
      }
      if (browserSnapshot.status.entries.length) {
        throw browserError("working_tree_not_clean");
      }
      if (!message.trim()) throw browserError("commit_message_required");
      const sha = createBrowserMockSha();
      browserCommits = [
        {
          sha,
          shortSha: sha.slice(0, 7),
          parents: [branchCommits[count].sha],
          tags: [],
          tagsTruncated: false,
          pushed: false,
          authorName: "BroKnowMy",
          authorEmail: "broknowmy@example.test",
          authoredAt: new Date().toISOString(),
          summary: message.trim().split(/\r?\n/u, 1)[0],
        },
        ...browserCommits,
      ];
      return updateMockSnapshot((snapshot) => {
        setBrowserCurrentBranchTip(snapshot, sha);
        syncBrowserBranchStatus(snapshot);
      });
    });
  },

  revertCommit(repositoryPath: string, revision: string): Promise<RepositorySnapshot> {
    return request("revert_commit", { repositoryPath, revision }, () => {
      requireBrowserSnapshot(repositoryPath);
      const target = browserCommits.find((commit) => commit.sha === revision || commit.shortSha === revision);
      if (!target) throw browserError("invalid_git_revision");
      const currentTip = browserSnapshot.status.branch
        ? browserBranchTips.get(browserSnapshot.status.branch) ?? null
        : null;
      const sha = createBrowserMockSha();
      browserCommitSummaryDescriptors.set(sha, { kind: "revert", sourceSha: target.sha });
      browserCommits = [
        {
          sha,
          shortSha: sha.slice(0, 7),
          parents: currentTip ? [currentTip] : [],
          tags: [],
          tagsTruncated: false,
          pushed: false,
          authorName: "BroKnowMy",
          authorEmail: "broknowmy@example.test",
          authoredAt: new Date().toISOString(),
          summary: `Revert ${target.shortSha}`,
        },
        ...browserCommits,
      ];
      return updateMockSnapshot((snapshot) => {
        setBrowserCurrentBranchTip(snapshot, sha);
        syncBrowserBranchStatus(snapshot);
      });
    });
  },

  createTag(repositoryPath: string, name: string, target?: string, message?: string): Promise<void> {
    return request("create_tag", { repositoryPath, name, target, message }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!name.trim()) throw browserError("invalid_git_reference");
      updateMockSnapshot((snapshot) => {
        if (!snapshot.tags.some((tag) => tag.name === name.trim())) {
          const normalizedTarget = target?.trim();
          const currentTip = snapshot.status.branch
            ? browserBranchTips.get(snapshot.status.branch) ?? null
            : null;
          const targetTip = !normalizedTarget || normalizedTarget === "HEAD" || normalizedTarget === snapshot.status.branch
            ? currentTip
            : browserBranchTips.get(normalizedTarget)
              ?? browserRemoteBranchTips.get(normalizedTarget)
              ?? snapshot.tags.find((tag) => tag.name === normalizedTarget)?.target
              ?? normalizedTarget;
          const commit = browserCommits.find((entry) => entry.sha === targetTip || entry.shortSha === targetTip);
          if (!commit) {
            throw browserError("invalid_git_revision");
          }
          snapshot.tags.unshift({ name: name.trim(), target: commit.sha });
          if (!commit.tags.includes(name.trim())) commit.tags.push(name.trim());
        }
      });
    });
  },

  deleteTag(repositoryPath: string, name: string): Promise<void> {
    return request("delete_tag", { repositoryPath, name }, () => {
      requireBrowserSnapshot(repositoryPath);
      updateMockSnapshot((snapshot) => {
        snapshot.tags = snapshot.tags.filter((tag) => tag.name !== name);
        for (const commit of browserCommits) commit.tags = commit.tags.filter((tag) => tag !== name);
      });
    });
  },

  addRemote(repositoryPath: string, name: string, url: string): Promise<void> {
    return request("add_remote", { repositoryPath, name, url }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!name.trim() || !url.trim()) throw browserError("invalid_remote_url");
      updateMockSnapshot((snapshot) => {
        if (snapshot.remotes.some((remote) => remote.name === name.trim())) {
          throw browserError("remote_exists");
        }
        snapshot.remotes.push(browserRemoteProfile(name.trim(), url.trim()));
      });
    });
  },

  setRemoteUrl(repositoryPath: string, name: string, url: string, kind: RemoteUrlKind): Promise<void> {
    return request("set_remote_url", { repositoryPath, name, url, kind }, () => {
      requireBrowserSnapshot(repositoryPath);
      updateMockSnapshot((snapshot) => {
        const remote = snapshot.remotes.find((item) => item.name === name);
        if (!remote) throw browserError("remote_not_found");
        const next = browserRemoteProfile(remote.name, kind === "fetch" ? url.trim() : remote.fetchUrl, kind === "push" ? url.trim() : remote.pushUrl);
        Object.assign(remote, next);
        const bindingIndex = snapshot.bindings.findIndex((binding) => binding.remoteName === name);
        if (bindingIndex !== -1 && !browserRemoteMatchesEndpoint(next, snapshot.bindings[bindingIndex].endpointId)) {
          snapshot.bindings.splice(bindingIndex, 1);
        }
      });
    });
  },

  renameRemote(repositoryPath: string, oldName: string, newName: string): Promise<void> {
    return request("rename_remote", { repositoryPath, oldName, newName }, () => {
      requireBrowserSnapshot(repositoryPath);
      updateMockSnapshot((snapshot) => {
        const remote = snapshot.remotes.find((item) => item.name === oldName);
        if (!remote) throw browserError("remote_not_found");
        if (snapshot.remotes.some((item) => item.name === newName && item !== remote)) {
          throw browserError("remote_exists");
        }
        const renamedReferences = snapshot.remoteBranches
          .filter((branch) => branch.remoteName === oldName)
          .map((branch) => [`${oldName}/${branch.name}`, `${newName}/${branch.name}`] as const);
        for (const [oldReference, newReference] of renamedReferences) {
          const tip = browserRemoteBranchTips.get(oldReference) ?? null;
          browserRemoteBranchTips.delete(oldReference);
          browserRemoteBranchTips.set(newReference, tip);
          snapshot.branches.forEach((branch) => {
            if (branch.upstream === oldReference) branch.upstream = newReference;
          });
          if (snapshot.status.upstream === oldReference) snapshot.status.upstream = newReference;
        }
        remote.name = newName;
        snapshot.remoteBranches.forEach((branch) => {
          if (branch.remoteName === oldName) branch.remoteName = newName;
        });
        const binding = snapshot.bindings.find((item) => item.remoteName === oldName);
        if (binding) binding.remoteName = newName;
        syncBrowserBranchStatus(snapshot);
      });
    });
  },

  removeRemote(repositoryPath: string, name: string): Promise<void> {
    return request("remove_remote", { repositoryPath, name }, () => {
      requireBrowserSnapshot(repositoryPath);
      updateMockSnapshot((snapshot) => {
        const removedReferences = snapshot.remoteBranches
          .filter((branch) => branch.remoteName === name)
          .map((branch) => `${name}/${branch.name}`);
        for (const reference of removedReferences) {
          browserRemoteBranchTips.delete(reference);
          snapshot.branches.forEach((branch) => {
            if (branch.upstream === reference) branch.upstream = null;
          });
          if (snapshot.status.upstream === reference) snapshot.status.upstream = null;
        }
        snapshot.remotes = snapshot.remotes.filter((remote) => remote.name !== name);
        snapshot.remoteBranches = snapshot.remoteBranches.filter((branch) => branch.remoteName !== name);
        snapshot.bindings = snapshot.bindings.filter((binding) => binding.remoteName !== name);
        syncBrowserBranchStatus(snapshot);
      });
    });
  },

  fetchRemote(repositoryPath: string, remoteName: string): Promise<NetworkOperationResult> {
    return request("fetch_remote", { repositoryPath, remoteName }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      requireBrowserRemote(snapshot, remoteName);
      updateMockSnapshot((nextSnapshot) => {
        if (!nextSnapshot.remoteBranches.some((branch) => branch.remoteName === remoteName)) {
          nextSnapshot.remoteBranches.push({ remoteName, name: "main" });
        }
        const remoteReference = `${remoteName}/main`;
        if (!browserRemoteBranchTips.has(remoteReference)) {
          const currentTip = nextSnapshot.status.branch
            ? browserBranchTips.get(nextSnapshot.status.branch) ?? null
            : null;
          browserRemoteBranchTips.set(
            remoteReference,
            currentTip ?? browserBranchTips.get("main") ?? browserCommits[0]?.sha ?? null,
          );
        }
      });
      return { summary: t("demo.network.fetch", { remote: remoteName }) };
    });
  },

  pullRemote(repositoryPath: string, remoteName: string): Promise<NetworkOperationResult> {
    return request("pull_remote", { repositoryPath, remoteName }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      requireBrowserRemote(snapshot, remoteName);
      return { summary: t("demo.network.pull", { remote: remoteName }) };
    });
  },

  pushRemote(repositoryPath: string, remoteName: string, forceWithLease = false): Promise<NetworkOperationResult> {
    return request("push_remote", { repositoryPath, remoteName, forceWithLease }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      requireBrowserRemote(snapshot, remoteName);
      updateMockSnapshot((snapshot) => {
        if (!snapshot.hasHead) {
          throw browserError("no_commit_to_push");
        }
        const branch = snapshot.status.branch;
        if (!branch) {
          throw browserError("detached_head");
        }
        const isUpstreamRemote = snapshot.status.upstream?.startsWith(`${remoteName}/`) ?? false;
        const establishesUpstream = !snapshot.status.upstream;
        if (!snapshot.status.upstream) {
          snapshot.status.upstream = `${remoteName}/${branch}`;
          const currentBranch = snapshot.branches.find((candidate) => candidate.isCurrent);
          if (currentBranch) currentBranch.upstream = snapshot.status.upstream;
        }
        const destination = isUpstreamRemote ? snapshot.status.upstream! : `${remoteName}/${branch}`;
        browserRemoteBranchTips.set(destination, browserBranchTips.get(branch) ?? null);
        const remoteBranchName = isUpstreamRemote
          ? destination.slice(remoteName.length + 1)
          : branch;
        if (!snapshot.remoteBranches.some((entry) => entry.remoteName === remoteName && entry.name === remoteBranchName)) {
          snapshot.remoteBranches.push({ remoteName, name: remoteBranchName });
        }
        if (establishesUpstream || isUpstreamRemote) syncBrowserBranchStatus(snapshot);
      });
      return { summary: t("demo.network.push", { remote: remoteName }) };
    });
  },

  listAccounts(): Promise<AccountProfile[]> {
    return request("list_accounts", {}, () => localizeBrowserAccounts(browserAccounts));
  },

  refreshAccountStatuses(): Promise<AccountProfile[]> {
    return request("refresh_account_statuses", {}, () => localizeBrowserAccounts(browserAccounts));
  },

  savePatAccount(input: SavePatAccountInput): Promise<AccountProfile> {
    return request("save_pat_account", input, () => {
      if (!input.token.trim()) {
        throw browserError("token_required");
      }
      const account: AccountProfile = {
        id: `${input.endpointId}-${input.login}-${Date.now()}`,
        endpointId: input.endpointId,
        login: input.login.trim(),
        displayName: input.displayName?.trim() || null,
        authMethod: "personalAccessToken",
        secretRef: { service: "browser-demo", account: input.login.trim() },
        status: "valid",
        isDefaultCandidate: false,
      };
      browserAccounts = [...browserAccounts, account];
      return localizeBrowserAccount(account);
    });
  },

  saveAccountProfile(input: SaveAccountProfileInput): Promise<AccountProfile> {
    return request("save_account_profile", input, () => {
      if (!input.login.trim()) {
        throw browserError("invalid_account");
      }
      const account: AccountProfile = {
        id: `${input.endpointId}-${input.authMethod}-${input.login}-${Date.now()}`,
        endpointId: input.endpointId,
        login: input.login.trim(),
        displayName: input.displayName?.trim() || null,
        authMethod: input.authMethod,
        secretRef: null,
        status: input.authMethod === "sshAgent" || input.authMethod === "systemGitCredentialHelper" ? "available" : "unknown",
        isDefaultCandidate: false,
      };
      browserAccounts = [...browserAccounts, account];
      return localizeBrowserAccount(account);
    });
  },

  removeAccount(accountProfileId: string): Promise<void> {
    return request("remove_account", { accountProfileId }, () => {
      browserAccounts = browserAccounts.filter((account) => account.id !== accountProfileId);
      browserAccountDisplayNameKeys.delete(accountProfileId);
    });
  },

  importCliAccounts(endpointId: string): Promise<AccountProfile[]> {
    return request("import_cli_accounts", { endpointId }, () => {
      const existing = browserAccounts.filter((account) => account.endpointId === endpointId && (account.authMethod === "ghCli" || account.authMethod === "glabCli"));
      return localizeBrowserAccounts(existing);
    });
  },

  setDefaultAccount(accountProfileId: string, isDefaultCandidate: boolean): Promise<AccountProfile[]> {
    return request("set_default_account", { accountProfileId, isDefaultCandidate }, () => {
      const selected = browserAccounts.find((account) => account.id === accountProfileId);
      if (!selected) {
        throw browserError("account_not_found");
      }
      browserAccounts = browserAccounts.map((account) =>
        account.endpointId === selected.endpointId ? { ...account, isDefaultCandidate: account.id === accountProfileId && isDefaultCandidate } : account,
      );
      return localizeBrowserAccounts(browserAccounts);
    });
  },

  getRemoteBinding(repositoryPath: string, remoteName: string): Promise<RemoteBinding | null> {
    return request("get_remote_binding", { repositoryPath, remoteName }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      return copy(snapshot.bindings.find((binding) => binding.remoteName === remoteName) ?? null);
    });
  },

  saveRemoteBinding(binding: RemoteBinding): Promise<RemoteBinding> {
    return request("save_remote_binding", { binding }, () => {
      const snapshot = requireBrowserSnapshot(binding.repositoryPath);
      const endpoint = browserEndpoints.find((item) => item.id === binding.endpointId);
      if (!endpoint) {
        throw browserError("endpoint_not_found");
      }
      const remote = snapshot.remotes.find((item) => item.name === binding.remoteName);
      if (!remote) {
        throw browserError("remote_not_found");
      }
      if (!browserRemoteMatchesEndpoint(remote, endpoint.id)) {
        throw browserError("endpoint_association_required");
      }
      if (binding.accountSelection.mode === "profile") {
        const account = requireBrowserAccountForEndpoint(binding.accountSelection.accountProfileId, endpoint.id);
        if (account.authMethod !== "personalAccessToken") {
          throw browserError("authentication_method_mismatch");
        }
      }
      updateMockSnapshot((snapshot) => {
        const index = snapshot.bindings.findIndex((entry) => entry.remoteName === binding.remoteName);
        if (index === -1) {
          snapshot.bindings.push(copy(binding));
        } else {
          snapshot.bindings[index] = copy(binding);
        }
      });
      return copy(binding);
    });
  },

  getCodeHostCapabilities(endpointId: string): Promise<CodeHostCapabilities> {
    return request("get_codehost_capabilities", { endpointId }, () => copy(browserCapabilities(endpointId)));
  },

  listEndpoints(): Promise<CodeHostEndpoint[]> {
    return request("list_endpoints", {}, () => copy(browserEndpoints));
  },

  saveCustomEndpoint(endpoint: CodeHostEndpoint): Promise<CodeHostEndpoint> {
    return request("save_custom_endpoint", { endpoint }, () => {
      const id = endpoint.id.trim().toLowerCase();
      if (!id || endpoint.isBuiltin || builtinEndpoints.some((item) => item.id.toLowerCase() === id)) {
        throw browserError("invalid_endpoint");
      }
      const next = copy({ ...endpoint, id, isBuiltin: false });
      const index = browserEndpoints.findIndex((item) => item.id === next.id);
      if (index === -1) browserEndpoints = [...browserEndpoints, next];
      else browserEndpoints = browserEndpoints.map((item, itemIndex) => (itemIndex === index ? next : item));
      return next;
    });
  },

  getCodeHostRepositoryInfo(repositoryPath: string, remoteName: string): Promise<CodeHostRepositoryInfo> {
    return request("get_codehost_repository_info", { repositoryPath, remoteName }, () => {
      requireBrowserGitHubRemote(repositoryPath, remoteName);
      return {
        fullName: "broknowmyorg/broknowmygitdesktop",
        description: t("demo.repository.description"),
        defaultBranch: "main",
        webUrl: "https://github.com/broknowmyorg/broknowmygitdesktop",
        isPrivate: false,
      };
    });
  },

  listGitHubPublishOwners(endpointId: string, accountProfileId: string): Promise<GitHubPublishOwner[]> {
    return request("list_github_publish_owners", { endpointId, accountProfileId }, () => {
      const endpoint = browserEndpoints.find((item) => item.id === endpointId);
      if (!endpoint || endpoint.kind !== "gitHub") {
        throw browserError("adapter_endpoint_mismatch");
      }
      const account = requireBrowserAccountForEndpoint(accountProfileId, endpointId);
      if (account.authMethod !== "ghCli") {
        throw browserError("github_publish_auth_unsupported");
      }
      return copy(browserGitHubPublishOwnersFor(account));
    });
  },

  listPullRequests(repositoryPath: string, remoteName: string): Promise<PullRequestSummary[]> {
    return request("list_pull_requests", { repositoryPath, remoteName }, () => {
      requireBrowserGitHubRemote(repositoryPath, remoteName);
      return localizeBrowserPullRequests(browserPullRequests);
    });
  },

  createPullRequest(
    repositoryPath: string,
    remoteName: string,
    input: CreatePullRequestInput,
  ): Promise<PullRequestSummary> {
    return request("create_pull_request", { repositoryPath, remoteName, input }, () => {
      requireBrowserGitHubRemote(repositoryPath, remoteName);
      if (!input.title.trim()) throw browserError("invalid_pull_request_title");
      if (!input.head.trim() || !input.base.trim()) {
        throw browserError("invalid_pull_request_branch");
      }
      const number = (browserPullRequests[0]?.number ?? 0) + 1;
      const pullRequest: PullRequestSummary = {
        number,
        title: input.title.trim(),
        author: "broknowmy",
        state: input.draft ? "DRAFT" : "OPEN",
        webUrl: "https://github.com/broknowmyorg/broknowmygitdesktop/pull/" + number,
        updatedAt: demoMarkers.updatedNow,
      };
      browserPullRequestUpdatedAtKeys.set(number, "demo.pullRequest.updatedNow");
      browserPullRequests = [pullRequest, ...browserPullRequests];
      return localizeBrowserPullRequest(pullRequest);
    });
  },

  forkGitHubRepository(
    repositoryPath: string,
    remoteName: string,
    endpointId: string,
    accountProfileId: string,
    forkRemoteName: string,
  ): Promise<GitHubForkSummary> {
    return request(
      "fork_github_repository",
      { repositoryPath, remoteName, endpointId, accountProfileId, forkRemoteName },
      () => {
        const snapshot = requireBrowserSnapshot(repositoryPath);
        const sourceRemote = requireBrowserGitHubRemote(repositoryPath, remoteName);
        const endpoint = browserEndpoints.find((entry) => entry.id === endpointId);
        if (
          !endpoint
          || endpoint.kind !== "gitHub"
          || !browserRemoteMatchesEndpoint(sourceRemote, endpointId)
        ) throw browserError("adapter_endpoint_mismatch");
        const account = requireBrowserAccountForEndpoint(accountProfileId, endpointId);
        if (account.authMethod !== "ghCli") throw browserError("github_fork_auth_unsupported");
        if (account.status !== "available" && account.status !== "valid") {
          throw browserError("cli_auth_unavailable");
        }
        const normalizedRemoteName = forkRemoteName.trim();
        if (!isValidBrowserRemoteName(normalizedRemoteName)) throw browserError("invalid_remote_name");
        if (normalizedRemoteName === sourceRemote.name) throw browserError("remote_already_exists");
        if (!snapshot.status.branch || !snapshot.hasHead) throw browserError("no_commit_to_push");
        const sourceOwner = sourceRemote.parsed.namespace;
        const repositoryName = sourceRemote.parsed.repository;
        if (
          !sourceOwner
          || sourceOwner.includes("/")
          || !repositoryName
          || !isValidBrowserGitHubRouteSegment(sourceOwner)
          || !isValidBrowserGitHubRouteSegment(repositoryName)
        ) throw browserError("repository_slug_unavailable");
        const ownerLogin = account.login;
        if (!isValidBrowserGitHubRouteSegment(ownerLogin) || ownerLogin.startsWith("-")) {
          throw browserError("invalid_repository_name");
        }
        const cloneUrl = `https://${endpoint.gitHost}/${ownerLogin}/${repositoryName}.git`;
        const sshUrl = `git@${endpoint.gitHost}:${ownerLogin}/${repositoryName}.git`;
        const forkUrl = sourceRemote.parsed.transport === "ssh" ? sshUrl : cloneUrl;
        const existing = snapshot.remotes.find((remote) => remote.name === normalizedRemoteName);
        if (existing && (existing.fetchUrl !== forkUrl || existing.pushUrl !== forkUrl)) {
          throw browserError("remote_already_exists");
        }
        updateMockSnapshot((nextSnapshot) => {
          if (!existing) nextSnapshot.remotes.push(browserRemoteProfile(normalizedRemoteName, forkUrl));
        });
        return {
          fullName: `${ownerLogin}/${repositoryName}`,
          ownerLogin,
          repositoryName,
          webUrl: `https://${endpoint.gitHost}/${ownerLogin}/${repositoryName}`,
          cloneUrl,
          sshUrl,
          remoteName: normalizedRemoteName,
          sourceRemoteName: sourceRemote.name,
        };
      },
    );
  },

  publishGitHubRepository(
    repositoryPath: string,
    endpointId: string,
    accountProfileId: string,
    ownerLogin: string,
    name: string,
    isPrivate: boolean,
  ): Promise<RepositorySnapshot> {
    return request(
      "publish_github_repository",
      { repositoryPath, endpointId, accountProfileId, ownerLogin, name, isPrivate },
      () => {
        requireBrowserSnapshot(repositoryPath);
        const repositoryName = name.trim();
        if (
          !repositoryName
          || repositoryName.length > 100
          || repositoryName.startsWith("-")
          || repositoryName === "."
          || repositoryName === ".."
          || !/^[A-Za-z0-9._-]+$/u.test(repositoryName)
        ) {
          throw browserError("invalid_repository_name");
        }
        if (browserSnapshot.remotes.length) {
          throw browserError("repository_already_has_remote");
        }
        if (!browserSnapshot.hasHead) {
          throw browserError("nothing_to_publish");
        }
        const endpoint = browserEndpoints.find((entry) => entry.id === endpointId);
        if (!endpoint || endpoint.kind !== "gitHub") {
          throw browserError("adapter_endpoint_mismatch");
        }
        const account = browserAccounts.find((entry) => entry.id === accountProfileId);
        if (!account) {
          throw browserError("account_not_found");
        }
        if (account.endpointId !== endpoint.id || account.authMethod !== "ghCli") {
          throw browserError("github_publish_auth_unsupported");
        }
        if (account.status !== "available" && account.status !== "valid") {
          throw browserError("cli_auth_unavailable");
        }
        const owner = browserGitHubPublishOwnersFor(account).find(
          (entry) => entry.login.toLowerCase() === ownerLogin.toLowerCase() && entry.canCreateRepositories,
        );
        if (!owner) {
          throw browserError("github_organization_publish_forbidden");
        }
        return updateMockSnapshot((snapshot) => {
          const remote = browserRemoteProfile("origin", "https://" + endpoint.gitHost + "/" + owner.login + "/" + repositoryName + ".git");
          snapshot.remotes.push(remote);
          snapshot.bindings.push({
            repositoryPath: snapshot.repository.path,
            remoteName: "origin",
            endpointId,
            accountSelection: { mode: "profile", accountProfileId },
          });
        });
      },
    );
  },
};
