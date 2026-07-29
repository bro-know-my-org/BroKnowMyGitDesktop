import { invoke } from "@tauri-apps/api/core";
import type {
  AccountProfile,
  AppError,
  CloneRepositoryInput,
  CleanUntrackedPreview,
  CommitChangedFile,
  CommitSummary,
  CodeHostCapabilities,
  CodeHostEndpoint,
  CodeHostRepositoryInfo,
  CreatePullRequestInput,
  CreateBranchInput,
  ExternalEditorConfig,
  GitOperationKind,
  GitHubPublishOwner,
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

export const builtinEndpoints: CodeHostEndpoint[] = [
  {
    id: "github.com",
    kind: "gitHub",
    displayName: "GitHub.com",
    webUrl: "https://github.com",
    apiUrl: "https://api.github.com",
    gitHost: "github.com",
    isBuiltin: true,
  },
  {
    id: "gitlab.com",
    kind: "gitLab",
    displayName: "GitLab.com",
    webUrl: "https://gitlab.com",
    apiUrl: "https://gitlab.com/api/v4",
    gitHost: "gitlab.com",
    isBuiltin: true,
  },
  {
    id: "gitee.com",
    kind: "gitee",
    displayName: "Gitee.com",
    webUrl: "https://gitee.com",
    apiUrl: "https://gitee.com/api/v5",
    gitHost: "gitee.com",
    isBuiltin: true,
  },
];

const builtinCapabilities: Record<string, CodeHostCapabilities> = {
  "github.com": {
    repositoryInfo: true,
    repositoryPublish: true,
    pullRequestsRead: true,
    pullRequestsWrite: true,
    accountImport: true,
    accountBindings: true,
    patKeychain: true,
  },
  "gitlab.com": {
    repositoryInfo: false,
    repositoryPublish: false,
    pullRequestsRead: false,
    pullRequestsWrite: false,
    accountImport: true,
    accountBindings: true,
    patKeychain: true,
  },
  "gitee.com": {
    repositoryInfo: false,
    repositoryPublish: false,
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
      pullRequestsRead: false,
      pullRequestsWrite: false,
      accountImport: false,
      accountBindings: true,
      patKeychain: true,
    };
  }
  return unavailableCapabilities;
}

const sampleRepository: RepositoryRecord = {
  path: "/Users/you/Projects/broknowmygitdesktop",
  name: "BroKnowMyGitDesktop",
  alias: "桌面端主仓",
  group: "桌面客户端",
  host: "gitee.com",
  namespace: "broknowmyorg",
  lastOpenedAt: Date.now() - 1000 * 60 * 18,
};

const sampleEntries: GitStatusEntry[] = [
  {
    path: "src/App.vue",
    originalPath: null,
    indexStatus: "M",
    worktreeStatus: "M",
  },
  {
    path: "src-tauri/src/domain.rs",
    originalPath: null,
    indexStatus: "",
    worktreeStatus: "M",
  },
  {
    path: "docs/architecture.md",
    originalPath: null,
    indexStatus: "A",
    worktreeStatus: "",
  },
];

const mockSnapshot: RepositorySnapshot = {
  repository: sampleRepository,
  hasHead: true,
  status: {
    branch: "main",
    upstream: "origin/main",
    ahead: 1,
    behind: 0,
    entries: sampleEntries,
  },
  branches: [
    { name: "main", isCurrent: true, upstream: "origin/main" },
    { name: "feature/multi-account", isCurrent: false, upstream: "origin/feature/multi-account" },
  ],
  remoteBranches: [
    { remoteName: "origin", name: "feature/multi-account" },
    { remoteName: "origin", name: "main" },
    { remoteName: "upstream", name: "main" },
  ],
  remotes: [
    {
      name: "origin",
      fetchUrl: "git@gitee.com:broknowmyorg/broknowmygitdesktop.git",
      pushUrl: "git@gitee.com:broknowmyorg/broknowmygitdesktop.git",
      parsed: {
        transport: "ssh",
        host: "gitee.com",
        port: null,
        namespace: "broknowmyorg",
        repository: "broknowmygitdesktop",
      },
      endpointId: "gitee.com",
      codeHostKind: "gitee",
    },
    {
      name: "upstream",
      fetchUrl: "https://github.com/broknowmyorg/broknowmygitdesktop.git",
      pushUrl: "https://github.com/broknowmyorg/broknowmygitdesktop.git",
      parsed: {
        transport: "https",
        host: "github.com",
        port: null,
        namespace: "broknowmyorg",
        repository: "broknowmygitdesktop",
      },
      endpointId: "github.com",
      codeHostKind: "gitHub",
    },
  ],
  tags: [{ name: "v0.1.0", target: "main" }],
  bindings: [
    {
      repositoryPath: sampleRepository.path,
      remoteName: "origin",
      endpointId: "gitee.com",
      accountSelection: { mode: "profile", accountProfileId: "gitee-personal" },
    },
    {
      repositoryPath: sampleRepository.path,
      remoteName: "upstream",
      endpointId: "github.com",
      accountSelection: { mode: "auto" },
    },
  ],
  operation: null,
};

let browserSnapshot = copy(mockSnapshot);
let browserCommits: CommitSummary[] = [
  {
    sha: "9d7a4f0c2ea846105d1200f9cf6a4eb68d748301",
    shortSha: "9d7a4f0",
    parents: ["55b91f2b0e1a6540dcdb43f1f7db2e3a22d7db10"],
    tags: ["v0.1.0"],
    tagsTruncated: false,
    pushed: false,
    authorName: "BroKnowMy",
    authorEmail: "broknowmy@example.test",
    authoredAt: "2026-07-19T10:20:30+08:00",
    summary: "完善多账号远程绑定",
  },
  {
    sha: "55b91f2b0e1a6540dcdb43f1f7db2e3a22d7db10",
    shortSha: "55b91f2",
    parents: [],
    tags: [],
    tagsTruncated: false,
    pushed: true,
    authorName: "BroKnowMy",
    authorEmail: "broknowmy@example.test",
    authoredAt: "2026-07-18T09:00:00+08:00",
    summary: "初始化桌面客户端",
  },
];
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
  const commits = browserBranchCommits(snapshot.status.branch);
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
let browserWorktrees: WorktreeInfo[] = [
  {
    path: sampleRepository.path,
    head: browserCommits[0].sha,
    branch: "main",
    isMain: true,
    isDetached: false,
    isLocked: false,
    isPrunable: false,
  },
];
let browserSubmodules: SubmoduleInfo[] = [];
let browserLfsStatus: LfsStatus = {
  available: true,
  initialized: false,
  version: "git-lfs/3.6.1 (browser demo)",
  files: [],
};
let browserExternalEditor: ExternalEditorConfig | null = null;
let browserCleanUntrackedConfirmation: { repositoryPath: string; token: string; paths: Set<string> } | null = null;
type BrowserStash = StashEntry & { entries: GitStatusEntry[] };

let browserStashes: BrowserStash[] = [];

function renumberBrowserStashes(): void {
  browserStashes = browserStashes.map((stash, index) => ({
    ...stash,
    reference: `stash@{${index}}`,
  }));
}

function stashBrowserChanges(snapshot: RepositorySnapshot, message: string): void {
  if (!snapshot.status.entries.length) return;
  browserStashes = [{
    reference: "",
    commitId: `browser-stash-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    message: `On ${snapshot.status.branch || "HEAD"}: ${message}`,
    entries: copy(snapshot.status.entries),
  }, ...browserStashes];
  renumberBrowserStashes();
  snapshot.status.entries = [];
}
let browserPullRequests: PullRequestSummary[] = [
  {
    number: 12,
    title: "完善多账号远程绑定",
    author: "broknowmy",
    state: "OPEN",
    webUrl: "https://github.com/broknowmyorg/broknowmygitdesktop/pull/12",
    updatedAt: "刚刚",
  },
];
let browserAccounts: AccountProfile[] = [
  {
    id: "gitee-personal",
    endpointId: "gitee.com",
    login: "broknowmy",
    displayName: "BroKnowMy · 个人",
    authMethod: "personalAccessToken",
    secretRef: { service: "io.github.broknowmyorg.broknowmygitdesktop", account: "gitee-personal" },
    status: "valid",
    isDefaultCandidate: true,
  },
  {
    id: "github-cli",
    endpointId: "github.com",
    login: "broknowmy",
    displayName: "GitHub CLI 导入账号",
    authMethod: "ghCli",
    secretRef: null,
    status: "available",
    isDefaultCandidate: true,
  },
  {
    id: "gitlab-cli",
    endpointId: "gitlab.com",
    login: "broknowmy-work",
    displayName: "工作账号",
    authMethod: "glabCli",
    secretRef: null,
    status: "needsReauthentication",
    isDefaultCandidate: false,
  },
];

function browserGitHubPublishOwnersFor(account: AccountProfile): GitHubPublishOwner[] {
  const personalOwner: GitHubPublishOwner = {
    login: account.login,
    name: account.displayName,
    kind: "user",
    canCreateRepositories: true,
  };
  if (account.id !== "github-cli") return [personalOwner];
  return [
    personalOwner,
    {
      login: "broknowmyorg",
      name: "BroKnowMy Organization",
      kind: "organization",
      canCreateRepositories: true,
    },
  ];
}
let browserEndpoints = copy(builtinEndpoints);

function copy<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);
}

export function isBrowserDemo(): boolean {
  return !isTauriRuntime();
}

/** Tauri only exposes this built-in command in debug builds. */
export async function toggleDevtools(): Promise<void> {
  if (!isTauriRuntime()) return;
  await invoke("plugin:webview|internal_toggle_devtools");
}

function isEmptyBrowserDemo(): boolean {
  return import.meta.env.VITE_UI_EMPTY_STATE === "true";
}

function redact(value: string): string {
  return value
    .replace(/(https?:\/\/)[^\s/@]+@/giu, "$1")
    .replace(/(authorization\s*[:=]\s*bearer\s+)[^\s,;]+/giu, "$1[已隐藏]")
    .replace(/((?:token|password|authorization)\s*[:=]\s*)[^\s,;]+/giu, "$1[已隐藏]")
    .replace(/([?&](?:access_token|private_token|client_secret)=)[^&#\s]+/giu, "$1[已隐藏]");
}

function asAppError(error: unknown): AppError {
  if (typeof error === "object" && error !== null && "message" in error) {
    const candidate = error as Partial<AppError>;
    return {
      code: typeof candidate.code === "string" ? candidate.code : "command_failed",
      message: redact(String(candidate.message)),
      details: candidate.details ? redact(String(candidate.details)) : null,
    };
  }

  const raw = typeof error === "string" ? error : "操作未能完成。";
  try {
    const parsed = JSON.parse(raw) as Partial<AppError>;
    if (typeof parsed.message === "string") {
      return {
        code: typeof parsed.code === "string" ? parsed.code : "command_failed",
        message: redact(parsed.message),
        details: parsed.details ? redact(String(parsed.details)) : null,
      };
    }
  } catch {
    // Tauri also returns ordinary strings for some command failures.
  }
  return { code: "command_failed", message: redact(raw), details: null };
}

async function request<T>(command: string, args: object, browserFallback: () => T | Promise<T>): Promise<T> {
  if (!isTauriRuntime()) {
    return browserFallback();
  }

  try {
    return await invoke<T>(command, args as Record<string, unknown>);
  } catch (error) {
    throw asAppError(error);
  }
}

function updateMockSnapshot(updater: (snapshot: RepositorySnapshot) => void): RepositorySnapshot {
  updater(browserSnapshot);
  // Browser confirmations only retain path strings, unlike the native
  // fingerprinted flow. Any intervening mock mutation can therefore replace
  // an untracked entry at the same path, so it invalidates the preview.
  browserCleanUntrackedConfirmation = null;
  return copy(browserSnapshot);
}

function parseGitQuotedDiffPath(header: string, index: number): { path: string; end: number } | null {
  if (header[index] !== '"') return null;
  const bytes: number[] = [];
  for (let cursor = index + 1; cursor < header.length; cursor += 1) {
    const character = header[cursor];
    if (character === '"') {
      try {
        return { path: new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes)), end: cursor + 1 };
      } catch {
        return null;
      }
    }
    if (character === "\\") {
      const escaped = header[++cursor];
      if (escaped === undefined) return null;
      const escapedByte = escaped === "n" ? 0x0a
        : escaped === "r" ? 0x0d
          : escaped === "t" ? 0x09
            : escaped === "\\" || escaped === '"' ? escaped.charCodeAt(0)
              : null;
      if (escapedByte !== null) {
        bytes.push(escapedByte);
        continue;
      }
      if (!/[0-7]/u.test(escaped) || cursor + 2 >= header.length) return null;
      const octal = header.slice(cursor, cursor + 3);
      if (!/^[0-7]{3}$/u.test(octal)) return null;
      bytes.push(Number.parseInt(octal, 8));
      cursor += 2;
      continue;
    }
    const code = character.charCodeAt(0);
    if (code < 0x20) return null;
    if (code <= 0x7e) {
      bytes.push(code);
    } else {
      bytes.push(...new TextEncoder().encode(character));
    }
  }
  return null;
}

function parseGitDiffHeaderPaths(header: string): [string, string] | null {
  const prefix = "diff --git ";
  if (!header.startsWith(prefix)) return null;
  let index = prefix.length;
  const paths: string[] = [];
  for (let pathIndex = 0; pathIndex < 2; pathIndex += 1) {
    if (index >= header.length) return null;
    if (header[index] === '"') {
      const parsed = parseGitQuotedDiffPath(header, index);
      if (!parsed) return null;
      paths.push(parsed.path);
      index = parsed.end;
    } else {
      const end = header.indexOf(" ", index);
      const tokenEnd = end === -1 ? header.length : end;
      if (tokenEnd === index) return null;
      paths.push(header.slice(index, tokenEnd));
      index = tokenEnd;
    }
    if (pathIndex === 0) {
      if (header[index] !== " ") return null;
      index += 1;
    }
  }
  return index === header.length ? [paths[0], paths[1]] : null;
}

function requireBrowserSnapshot(repositoryPath: string): RepositorySnapshot {
  if (repositoryPath !== browserSnapshot.repository.path) {
    throw { code: "repository_not_found", message: "找不到这个示例仓库。" } satisfies AppError;
  }
  return browserSnapshot;
}

function requireBrowserRemote(snapshot: RepositorySnapshot, remoteName: string): void {
  if (!snapshot.remotes.some((remote) => remote.name === remoteName)) {
    throw { code: "remote_not_found", message: "找不到该 remote。" } satisfies AppError;
  }
}

function requireBrowserGitHubRemote(repositoryPath: string, remoteName: string): RemoteProfile {
  const snapshot = requireBrowserSnapshot(repositoryPath);
  const remote = snapshot.remotes.find((item) => item.name === remoteName);
  if (!remote || remote.codeHostKind !== "gitHub") {
    throw { code: "adapter_endpoint_mismatch", message: "该 remote 不支持 GitHub 平台功能。" } satisfies AppError;
  }
  return remote;
}

function requireBrowserAccountForEndpoint(accountProfileId: string, endpointId: string): AccountProfile {
  const account = browserAccounts.find((item) => item.id === accountProfileId);
  if (!account) {
    throw { code: "account_not_found", message: "找不到所选账号。" } satisfies AppError;
  }
  if (account.endpointId !== endpointId) {
    throw { code: "binding_endpoint_mismatch", message: "所选账号不属于该代码托管 Endpoint。" } satisfies AppError;
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

export const api = {
  listRepositories(refreshOwners = false): Promise<RepositoryRecord[]> {
    return request("list_repositories", { refreshOwners }, () => (isEmptyBrowserDemo() ? [] : [copy(browserSnapshot.repository)]));
  },

  discoverRepositories(rootPath: string): Promise<RepositoryRecord[]> {
    return request("discover_repositories", { rootPath }, () => (isEmptyBrowserDemo() ? [] : [copy(browserSnapshot.repository)]));
  },

  initRepository(input: InitRepositoryInput): Promise<RepositoryRecord> {
    return request("init_repository", input, () => {
      const destinationPath = input.destinationPath.replace(/[\\/]$/, "");
      if (!destinationPath) throw { code: "invalid_init_destination", message: "新仓库目录无效。" } satisfies AppError;
      const name = destinationPath.split(/[\\/]/u).filter(Boolean).pop() || "new-repository";
      browserSnapshot = copy({
        ...mockSnapshot,
        repository: { path: destinationPath, name, alias: null, group: null, host: null, namespace: null, lastOpenedAt: null },
        hasHead: false,
        status: { ...mockSnapshot.status, branch: input.initialBranch?.trim() || "main", upstream: null, ahead: 0, behind: 0, entries: [] },
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
      return copy(browserSnapshot.repository);
    });
  },

  cloneRepository(input: CloneRepositoryInput): Promise<RepositoryRecord> {
    return request("clone_repository", input, () => {
      const pathname = input.destinationPath.replace(/[\\/]$/, "");
      const name = pathname.split(/[\\/]/u).filter(Boolean).pop() || "cloned-repository";
      const branch = "main";
      browserSnapshot = copy({
        ...mockSnapshot,
        repository: { path: pathname, name, alias: null, group: null, host: null, namespace: null, lastOpenedAt: null },
        hasHead: false,
        status: { ...mockSnapshot.status, branch, upstream: null, ahead: 0, behind: 0, entries: [] },
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
      return copy(browserSnapshot.repository);
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
      }
      const ownerRemote = browserSnapshot.remotes.find(
        (remote) => remote.name === "origin" && remote.parsed.host && remote.parsed.namespace,
      ) ?? browserSnapshot.remotes.find((remote) => remote.parsed.host && remote.parsed.namespace);
      browserSnapshot.repository.host = ownerRemote?.parsed.host ?? null;
      browserSnapshot.repository.namespace = ownerRemote?.parsed.namespace ?? null;
      browserSnapshot.repository.lastOpenedAt = Date.now();
      return copy(requireBrowserSnapshot(repositoryPath));
    });
  },

  refreshRepositorySnapshot(repositoryPath: string): Promise<RepositorySnapshot> {
    return request("refresh_repository_snapshot", { repositoryPath }, () => copy(requireBrowserSnapshot(repositoryPath)));
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
        throw { code: "repository_group_invalid", message: "分组名称最多 80 个字符。" } satisfies AppError;
      }
      snapshot.repository.group = nextGroup;
      return copy(snapshot.repository);
    });
  },

  setRepositoryAlias(repositoryPath: string, alias: string | null): Promise<RepositoryRecord> {
    return request("set_repository_alias", { repositoryPath, alias }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      const nextAlias = alias?.trim() || null;
      if (nextAlias && Array.from(nextAlias).length > 80) {
        throw { code: "repository_alias_invalid", message: "仓库别名最多 80 个字符。" } satisfies AppError;
      }
      snapshot.repository.alias = nextAlias;
      return copy(snapshot.repository);
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
        "-  title: '旧的实现',",
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
        "55b91f2b0e1 (BroKnowMy 2026-07-18)  2) 示例 Blame 输出",
      ].join("\n");
    });
  },

  listReflog(repositoryPath: string, limit = 100): Promise<string> {
    return request("list_reflog", { repositoryPath, limit }, () => {
      requireBrowserSnapshot(repositoryPath);
      return [
        "9d7a4f0 HEAD@{0} commit: 完善多账号远程绑定 (BroKnowMy, 2026-07-19T10:20:30+08:00)",
        "55b91f2 HEAD@{1} commit (initial): 初始化桌面客户端 (BroKnowMy, 2026-07-18T09:00:00+08:00)",
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
        throw {
          code: "clean_untracked_confirmation_expired",
          message: "未跟踪文件预览已过期，请重新确认后再清理。",
        } satisfies AppError;
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
        throw { code: "external_editor_config_invalid", message: "外部编辑器配置无效。" } satisfies AppError;
      }
      browserExternalEditor = { executable, arguments: editorArguments };
      return copy(browserExternalEditor);
    });
  },

  openConflictedFilesInExternalEditor(repositoryPath: string): Promise<string[]> {
    return request("open_conflicted_files_in_external_editor", { repositoryPath }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!browserExternalEditor) {
        throw { code: "external_editor_not_configured", message: "请先配置用于解决冲突的外部编辑器。" } satisfies AppError;
      }
      const conflicts = browserSnapshot.status.entries
        .filter((entry) => ["DD", "AU", "UD", "UA", "DU", "AA", "UU"].includes(entry.indexStatus + entry.worktreeStatus))
        .map((entry) => entry.path);
      if (!conflicts.length) {
        throw { code: "no_conflicted_files", message: "当前仓库没有需要在外部编辑器中处理的冲突文件。" } satisfies AppError;
      }
      return conflicts;
    });
  },

  listCommits(repositoryPath: string, limit = 100): Promise<CommitSummary[]> {
    return request("list_commits", { repositoryPath, limit }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      return copy(browserHistoryCommits(snapshot).slice(0, limit));
    });
  },

  getCommitDiff(repositoryPath: string, revision: string): Promise<string> {
    return request("get_commit_diff", { repositoryPath, revision }, () => {
      requireBrowserSnapshot(repositoryPath);
      const commit = browserCommits.find((entry) => entry.sha === revision || entry.shortSha === revision);
      if (!commit) throw { code: "invalid_git_revision", message: "找不到该提交。" } satisfies AppError;
      return [
        `commit ${commit.sha}`,
        `Author: ${commit.authorName} <${commit.authorEmail}>`,
        `Date:   ${commit.authoredAt}`,
        "",
        `    ${commit.summary}`,
        "",
        "diff --git a/README.md b/README.md",
        "--- a/README.md",
        "+++ b/README.md",
        "@@ -1 +1,2 @@",
        " # BroKnowMyGitDesktop",
        "+历史记录浏览器演示",
      ].join("\n");
    });
  },

  listCommitChangedFiles(repositoryPath: string, revision: string): Promise<CommitChangedFile[]> {
    return request("list_commit_changed_files", { repositoryPath, revision }, () => {
      requireBrowserSnapshot(repositoryPath);
      const commit = browserCommits.find((entry) => entry.sha === revision || entry.shortSha === revision);
      if (!commit) throw { code: "invalid_git_revision", message: "找不到该提交。" } satisfies AppError;
      return copy(browserCommitChangedFiles(commit));
    });
  },

  getCommitFileDiff(
    repositoryPath: string,
    revision: string,
    path: string,
    previousPath: string | null = null,
  ): Promise<string> {
    return request("get_commit_file_diff", { repositoryPath, revision, path, previousPath }, () => {
      requireBrowserSnapshot(repositoryPath);
      const commit = browserCommits.find((entry) => entry.sha === revision || entry.shortSha === revision);
      if (!commit) throw { code: "invalid_git_revision", message: "找不到该提交。" } satisfies AppError;
      const changedFile = browserCommitChangedFiles(commit).find((file) => file.path === path);
      if (!changedFile || changedFile.previousPath !== previousPath) {
        throw { code: "commit_file_not_found", message: "所选文件不属于这个提交的改动。" } satisfies AppError;
      }
      if (previousPath) {
        return [
          `diff --git a/${previousPath} b/${path}`,
          `similarity index 92%`,
          `rename from ${previousPath}`,
          `rename to ${path}`,
        ].join("\n");
      }
      if (changedFile.status === "added") {
        return [
          `diff --git a/${path} b/${path}`,
          "new file mode 100644",
          "--- /dev/null",
          `+++ b/${path}`,
          "@@ -0,0 +1 @@",
          "+历史记录按文件浏览",
        ].join("\n");
      }
      return [
        `diff --git a/${path} b/${path}`,
        `--- a/${path}`,
        `+++ b/${path}`,
        "@@ -1 +1,2 @@",
        ` # ${path}`,
        "+历史记录按文件浏览",
      ].join("\n");
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
        throw { code: "invalid_git_patch", message: "代码块补丁无效。" } satisfies AppError;
      }
      const diffHeaders = patch.match(/^diff --git [^\r\n]+$/gmu) ?? [];
      const headerPaths = diffHeaders.length === 1 ? parseGitDiffHeaderPaths(diffHeaders[0]) : null;
      if (!headerPaths || headerPaths[0] !== `a/${path}` || headerPaths[1] !== `b/${path}`) {
        throw { code: "invalid_git_patch", message: "代码块补丁与当前选中的文件不匹配。" } satisfies AppError;
      }
      return updateMockSnapshot((snapshot) => {
        const entry = snapshot.status.entries.find((candidate) => candidate.path === path);
        if (!entry) throw { code: "invalid_git_patch", message: "找不到补丁对应的文件。" } satisfies AppError;
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

  discardChanges(repositoryPath: string, paths: string[]): Promise<RepositorySnapshot> {
    return request("discard_changes", { repositoryPath, paths }, () => {
      requireBrowserSnapshot(repositoryPath);
      const selected = new Set(paths);
      const found = browserSnapshot.status.entries.some((entry) => selected.has(entry.path));
      if (!found) throw { code: "nothing_to_discard", message: "没有可丢弃的改动。" } satisfies AppError;
      return updateMockSnapshot((snapshot) => {
        snapshot.status.entries = snapshot.status.entries.filter((entry) => !selected.has(entry.path));
      });
    });
  },

  commitChanges(repositoryPath: string, message: string, amend = false): Promise<RepositorySnapshot> {
    return request("commit_changes", { repositoryPath, message, amend }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!message.trim()) {
        throw { code: "commit_message_required", message: "请输入提交说明。" } satisfies AppError;
      }
      return updateMockSnapshot((snapshot) => {
        const stagedEntries = snapshot.status.entries.filter((entry) => Boolean(entry.indexStatus));
        if (!stagedEntries.length && !amend) {
          throw { code: "nothing_to_commit", message: "没有已暂存的改动可提交。" } satisfies AppError;
        }
        const sha = createBrowserMockSha();
        const currentTip = snapshot.status.branch ? browserBranchTips.get(snapshot.status.branch) : null;
        const currentCommit = currentTip ? browserCommits.find((commit) => commit.sha === currentTip) : undefined;
        if (amend && (!snapshot.hasHead || !currentCommit)) {
          throw { code: "no_commit_to_amend", message: "当前分支还没有可修改的提交。" } satisfies AppError;
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

  createBranch(input: CreateBranchInput): Promise<RepositorySnapshot> {
    return request("create_branch", input, () => {
      requireBrowserSnapshot(input.repositoryPath);
      if (!input.name.trim()) {
        throw { code: "branch_name_required", message: "请输入分支名称。" } satisfies AppError;
      }
      return updateMockSnapshot((snapshot) => {
        const name = input.name.trim();
        if (snapshot.branches.some((branch) => branch.name === name)) {
          throw { code: "branch_already_exists", message: "该分支已存在。" } satisfies AppError;
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
    });
  },

  checkoutBranch(repositoryPath: string, branch: string, stashLocalChanges = false): Promise<RepositorySnapshot> {
    return request("checkout_branch", { repositoryPath, branch, stashLocalChanges }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!browserSnapshot.branches.some((entry) => entry.name === branch)) {
        throw { code: "branch_not_found", message: "找不到该分支。" } satisfies AppError;
      }
      return updateMockSnapshot((snapshot) => {
        if (stashLocalChanges) stashBrowserChanges(snapshot, "切换分支前自动暂存");
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
        throw { code: "remote_branch_not_found", message: "找不到该远程分支；请先获取远程更新后重试。" } satisfies AppError;
      }
      return updateMockSnapshot((nextSnapshot) => {
        if (stashLocalChanges) stashBrowserChanges(nextSnapshot, "切换分支前自动暂存");
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
      if (!newName.trim()) throw { code: "invalid_git_reference", message: "新分支名称无效。" } satisfies AppError;
      return updateMockSnapshot((snapshot) => {
        const branch = snapshot.branches.find((entry) => entry.name === oldName);
        if (!branch) throw { code: "branch_not_found", message: "找不到该分支。" } satisfies AppError;
        const name = newName.trim();
        if (name !== oldName && snapshot.branches.some((entry) => entry.name === name)) {
          throw { code: "branch_already_exists", message: "该分支已存在。" } satisfies AppError;
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
        if (!branch) throw { code: "branch_not_found", message: "找不到该分支。" } satisfies AppError;
        if (branch.isCurrent) throw { code: "cannot_delete_current_branch", message: "不能删除当前分支。" } satisfies AppError;
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
      if (!target) throw { code: "invalid_worktree_path", message: "工作树路径无效。" } satisfies AppError;
      if (!createBranch?.trim() && !commitish?.trim()) {
        throw { code: "invalid_worktree_config", message: "请选择新分支或现有分支。" } satisfies AppError;
      }
      if (browserWorktrees.some((entry) => entry.path === target)) {
        throw { code: "git_command_failed", message: "该工作树路径已存在。" } satisfies AppError;
      }
      const newBranch = createBranch?.trim();
      if (newBranch && browserSnapshot.branches.some((entry) => entry.name === newBranch)) {
        throw { code: "branch_already_exists", message: "该分支已存在。" } satisfies AppError;
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
        throw { code: "invalid_git_revision", message: "找不到该提交。" } satisfies AppError;
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
      if (!worktree) throw { code: "worktree_not_found", message: "找不到该工作树。" } satisfies AppError;
      if (worktree.isMain) throw { code: "cannot_remove_main_worktree", message: "不能移除主工作树。" } satisfies AppError;
      if (worktree.path === browserSnapshot.repository.path) {
        throw { code: "cannot_remove_active_worktree", message: "不能移除当前工作树。" } satisfies AppError;
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
      if (!browserLfsStatus.available) throw { code: "lfs_unavailable", message: "系统未安装 Git LFS。" } satisfies AppError;
      browserLfsStatus.initialized = true;
      return copy(browserLfsStatus);
    });
  },

  trackLfsPattern(repositoryPath: string, pattern: string): Promise<RepositorySnapshot> {
    return request("track_lfs_pattern", { repositoryPath, pattern }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!browserLfsStatus.available) throw { code: "lfs_unavailable", message: "系统未安装 Git LFS。" } satisfies AppError;
      if (!pattern.trim() || pattern.trim().startsWith("-")) {
        throw { code: "invalid_lfs_pattern", message: "Git LFS 跟踪规则无效。" } satisfies AppError;
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
      if (!base.trim()) throw { code: "invalid_git_revision", message: "请选择变基目标。" } satisfies AppError;
      return updateMockSnapshot((snapshot) => {
        if (snapshot.operation) throw { code: "git_operation_in_progress", message: "当前已有 Git 操作尚未完成。" } satisfies AppError;
        snapshot.operation = { kind: "rebase", hasConflicts: false };
      });
    });
  },

  cherryPickCommits(repositoryPath: string, revisions: string[]): Promise<RepositorySnapshot> {
    return request("cherry_pick_commits", { repositoryPath, revisions }, () => {
      requireBrowserSnapshot(repositoryPath);
      const selected = revisions.map((revision) => revision.trim()).filter(Boolean);
      if (!selected.length) throw { code: "invalid_cherry_pick", message: "请输入要拣选的提交。" } satisfies AppError;
      if (browserSnapshot.operation) throw { code: "git_operation_in_progress", message: "当前已有 Git 操作尚未完成。" } satisfies AppError;
      const source = browserCommits.find((commit) => commit.sha === selected[0] || commit.shortSha === selected[0]);
      const currentTip = browserSnapshot.status.branch
        ? browserBranchTips.get(browserSnapshot.status.branch) ?? null
        : null;
      const sha = createBrowserMockSha();
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
          summary: source ? `Cherry-pick: ${source.summary}` : `Cherry-pick ${selected[0]}`,
        },
        ...browserCommits,
      ];
      return updateMockSnapshot((snapshot) => {
        setBrowserCurrentBranchTip(snapshot, sha);
        syncBrowserBranchStatus(snapshot);
      });
    });
  },

  continueGitOperation(repositoryPath: string, kind: GitOperationKind): Promise<RepositorySnapshot> {
    return request("continue_git_operation", { repositoryPath, kind }, () => {
      requireBrowserSnapshot(repositoryPath);
      return updateMockSnapshot((snapshot) => {
        if (!snapshot.operation || snapshot.operation.kind !== kind) {
          throw { code: "git_operation_not_in_progress", message: "没有可继续的 Git 操作。" } satisfies AppError;
        }
        if (snapshot.operation.hasConflicts) {
          throw { code: "git_merge_conflict", message: "请先解决全部冲突。" } satisfies AppError;
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
          throw { code: "git_operation_not_in_progress", message: "没有可中止的 Git 操作。" } satisfies AppError;
        }
        snapshot.operation = null;
      });
    });
  },

  listStashes(repositoryPath: string): Promise<StashEntry[]> {
    return request("list_stashes", { repositoryPath }, () => {
      requireBrowserSnapshot(repositoryPath);
      return copy(browserStashes.map(({ reference, commitId, message }) => ({ reference, commitId, message })));
    });
  },

  stashChanges(repositoryPath: string, message: string | undefined, includeUntracked: boolean): Promise<RepositorySnapshot> {
    return request("stash_changes", { repositoryPath, message, includeUntracked }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!browserSnapshot.status.entries.length) throw { code: "nothing_to_stash", message: "没有可暂存的改动。" } satisfies AppError;
      const suffix = message?.trim() || "WIP";
      return updateMockSnapshot((snapshot) => {
        stashBrowserChanges(snapshot, suffix);
      });
    });
  },

  popStash(repositoryPath: string, reference: string, expectedCommitId: string): Promise<RepositorySnapshot> {
    return request("pop_stash", { repositoryPath, reference, expectedCommitId }, () => {
      requireBrowserSnapshot(repositoryPath);
      const index = browserStashes.findIndex((entry) => entry.reference === reference && entry.commitId === expectedCommitId);
      if (index === -1) throw { code: "stash_not_found", message: "找不到该暂存栈条目。" } satisfies AppError;
      const [stash] = browserStashes.splice(index, 1);
      renumberBrowserStashes();
      return updateMockSnapshot((snapshot) => {
        snapshot.status.entries = copy(stash.entries);
      });
    });
  },

  dropStash(repositoryPath: string, reference: string, expectedCommitId: string): Promise<void> {
    return request("drop_stash", { repositoryPath, reference, expectedCommitId }, () => {
      requireBrowserSnapshot(repositoryPath);
      browserStashes = browserStashes.filter((entry) => entry.reference !== reference || entry.commitId !== expectedCommitId);
      renumberBrowserStashes();
    });
  },

  resetBranch(repositoryPath: string, target: string, mode: ResetMode): Promise<RepositorySnapshot> {
    return request("reset_branch", { repositoryPath, target, mode }, () => {
      requireBrowserSnapshot(repositoryPath);
      if (!browserCommits.some((commit) => commit.sha === target || commit.shortSha === target)) {
        throw { code: "invalid_git_revision", message: "找不到重置目标。" } satisfies AppError;
      }
      throw {
        code: "browser_demo_unsupported",
        message: `浏览器演示不执行 ${mode} reset，以免伪造仓库状态。`,
      } satisfies AppError;
    });
  },

  squashRecentCommits(repositoryPath: string, count: number, message: string): Promise<RepositorySnapshot> {
    return request("squash_recent_commits", { repositoryPath, count, message }, () => {
      requireBrowserSnapshot(repositoryPath);
      const branchCommits = browserBranchCommits(browserSnapshot.status.branch);
      if (!Number.isInteger(count) || count < 2 || count > 1000 || branchCommits.length <= count) {
        throw { code: "invalid_squash_count", message: "没有足够的提交可压缩。" } satisfies AppError;
      }
      if (browserSnapshot.status.entries.length) {
        throw { code: "working_tree_not_clean", message: "请先处理当前改动。" } satisfies AppError;
      }
      if (!message.trim()) throw { code: "commit_message_required", message: "请输入提交说明。" } satisfies AppError;
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
      if (!target) throw { code: "invalid_git_revision", message: "找不到该提交。" } satisfies AppError;
      const currentTip = browserSnapshot.status.branch
        ? browserBranchTips.get(browserSnapshot.status.branch) ?? null
        : null;
      const sha = createBrowserMockSha();
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
          summary: `Revert \"${target.summary}\"`,
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
      if (!name.trim()) throw { code: "invalid_git_reference", message: "标签名称无效。" } satisfies AppError;
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
            throw { code: "invalid_git_revision", message: "找不到该标签目标。" } satisfies AppError;
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
      if (!name.trim() || !url.trim()) throw { code: "invalid_remote_url", message: "远程仓库信息无效。" } satisfies AppError;
      updateMockSnapshot((snapshot) => {
        if (snapshot.remotes.some((remote) => remote.name === name.trim())) {
          throw { code: "remote_exists", message: "该 remote 已存在。" } satisfies AppError;
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
        if (!remote) throw { code: "remote_not_found", message: "找不到该 remote。" } satisfies AppError;
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
        if (!remote) throw { code: "remote_not_found", message: "找不到该 remote。" } satisfies AppError;
        if (snapshot.remotes.some((item) => item.name === newName && item !== remote)) {
          throw { code: "remote_exists", message: "该 remote 已存在。" } satisfies AppError;
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
      return { summary: `已从 ${remoteName} 获取最新引用（浏览器演示）。` };
    });
  },

  pullRemote(repositoryPath: string, remoteName: string): Promise<NetworkOperationResult> {
    return request("pull_remote", { repositoryPath, remoteName }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      requireBrowserRemote(snapshot, remoteName);
      return { summary: `已从 ${remoteName} 拉取变更（浏览器演示）。` };
    });
  },

  pushRemote(repositoryPath: string, remoteName: string, forceWithLease = false): Promise<NetworkOperationResult> {
    return request("push_remote", { repositoryPath, remoteName, forceWithLease }, () => {
      const snapshot = requireBrowserSnapshot(repositoryPath);
      requireBrowserRemote(snapshot, remoteName);
      updateMockSnapshot((snapshot) => {
        if (!snapshot.hasHead) {
          throw { code: "no_commit_to_push", message: "当前分支没有可推送的提交。" } satisfies AppError;
        }
        const branch = snapshot.status.branch;
        if (!branch) {
          throw { code: "detached_head", message: "游离 HEAD 无法建立分支上游。" } satisfies AppError;
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
      return { summary: `已推送到 ${remoteName}（浏览器演示）。` };
    });
  },

  listAccounts(): Promise<AccountProfile[]> {
    return request("list_accounts", {}, () => copy(browserAccounts));
  },

  refreshAccountStatuses(): Promise<AccountProfile[]> {
    return request("refresh_account_statuses", {}, () => copy(browserAccounts));
  },

  savePatAccount(input: SavePatAccountInput): Promise<AccountProfile> {
    return request("save_pat_account", input, () => {
      if (!input.token.trim()) {
        throw { code: "token_required", message: "请输入个人访问令牌。" } satisfies AppError;
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
      return copy(account);
    });
  },

  saveAccountProfile(input: SaveAccountProfileInput): Promise<AccountProfile> {
    return request("save_account_profile", input, () => {
      if (!input.login.trim()) {
        throw { code: "invalid_account", message: "账号资料无效。" } satisfies AppError;
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
      return copy(account);
    });
  },

  removeAccount(accountProfileId: string): Promise<void> {
    return request("remove_account", { accountProfileId }, () => {
      browserAccounts = browserAccounts.filter((account) => account.id !== accountProfileId);
    });
  },

  importCliAccounts(endpointId: string): Promise<AccountProfile[]> {
    return request("import_cli_accounts", { endpointId }, () => {
      const existing = browserAccounts.filter((account) => account.endpointId === endpointId && (account.authMethod === "ghCli" || account.authMethod === "glabCli"));
      return copy(existing);
    });
  },

  setDefaultAccount(accountProfileId: string, isDefaultCandidate: boolean): Promise<AccountProfile[]> {
    return request("set_default_account", { accountProfileId, isDefaultCandidate }, () => {
      const selected = browserAccounts.find((account) => account.id === accountProfileId);
      if (!selected) {
        throw { code: "account_not_found", message: "找不到这个账号。" } satisfies AppError;
      }
      browserAccounts = browserAccounts.map((account) =>
        account.endpointId === selected.endpointId ? { ...account, isDefaultCandidate: account.id === accountProfileId && isDefaultCandidate } : account,
      );
      return copy(browserAccounts);
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
        throw { code: "endpoint_not_found", message: "找不到该代码托管 Endpoint。" } satisfies AppError;
      }
      const remote = snapshot.remotes.find((item) => item.name === binding.remoteName);
      if (!remote) {
        throw { code: "remote_not_found", message: "找不到该 remote。" } satisfies AppError;
      }
      if (!browserRemoteMatchesEndpoint(remote, endpoint.id)) {
        throw { code: "endpoint_association_required", message: "该 remote 与所选代码托管 Endpoint 不匹配。" } satisfies AppError;
      }
      if (binding.accountSelection.mode === "profile") {
        const account = requireBrowserAccountForEndpoint(binding.accountSelection.accountProfileId, endpoint.id);
        if (account.authMethod !== "personalAccessToken") {
          throw { code: "authentication_method_mismatch", message: "指定账号绑定仅支持应用管理的 PAT。" } satisfies AppError;
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
        throw { code: "invalid_endpoint", message: "自托管 Endpoint 信息无效。" } satisfies AppError;
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
        description: "一个轻量、多账号的代码托管桌面客户端。",
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
        throw { code: "adapter_endpoint_mismatch", message: "只有 GitHub Endpoint 支持查询发布所有者。" } satisfies AppError;
      }
      const account = requireBrowserAccountForEndpoint(accountProfileId, endpointId);
      if (account.authMethod !== "ghCli") {
        throw { code: "github_publish_auth_unsupported", message: "查询 GitHub 发布所有者需要匹配的 gh CLI 账号。" } satisfies AppError;
      }
      return copy(browserGitHubPublishOwnersFor(account));
    });
  },

  listPullRequests(repositoryPath: string, remoteName: string): Promise<PullRequestSummary[]> {
    return request("list_pull_requests", { repositoryPath, remoteName }, () => {
      requireBrowserGitHubRemote(repositoryPath, remoteName);
      return copy(browserPullRequests);
    });
  },

  createPullRequest(
    repositoryPath: string,
    remoteName: string,
    input: CreatePullRequestInput,
  ): Promise<PullRequestSummary> {
    return request("create_pull_request", { repositoryPath, remoteName, input }, () => {
      requireBrowserGitHubRemote(repositoryPath, remoteName);
      if (!input.title.trim()) throw { code: "invalid_pull_request_title", message: "请输入 Pull Request 标题。" } satisfies AppError;
      if (!input.head.trim() || !input.base.trim()) {
        throw { code: "invalid_pull_request_branch", message: "请选择源分支和目标分支。" } satisfies AppError;
      }
      const number = (browserPullRequests[0]?.number ?? 0) + 1;
      const pullRequest: PullRequestSummary = {
        number,
        title: input.title.trim(),
        author: "broknowmy",
        state: input.draft ? "DRAFT" : "OPEN",
        webUrl: "https://github.com/broknowmyorg/broknowmygitdesktop/pull/" + number,
        updatedAt: "刚刚",
      };
      browserPullRequests = [pullRequest, ...browserPullRequests];
      return copy(pullRequest);
    });
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
          throw { code: "invalid_repository_name", message: "GitHub 仓库名称无效。" } satisfies AppError;
        }
        if (browserSnapshot.remotes.length) {
          throw { code: "repository_already_has_remote", message: "当前仓库已有 remote。" } satisfies AppError;
        }
        if (!browserSnapshot.hasHead) {
          throw { code: "nothing_to_publish", message: "请先创建本地提交。" } satisfies AppError;
        }
        const endpoint = browserEndpoints.find((entry) => entry.id === endpointId);
        if (!endpoint || endpoint.kind !== "gitHub") {
          throw { code: "adapter_endpoint_mismatch", message: "请选择 GitHub Endpoint。" } satisfies AppError;
        }
        const account = browserAccounts.find((entry) => entry.id === accountProfileId);
        if (!account) {
          throw { code: "account_not_found", message: "找不到发布所选的账号。" } satisfies AppError;
        }
        if (account.endpointId !== endpoint.id || account.authMethod !== "ghCli") {
          throw { code: "github_publish_auth_unsupported", message: "发布 GitHub 仓库需要匹配的 gh CLI 账号。" } satisfies AppError;
        }
        if (account.status !== "available" && account.status !== "valid") {
          throw { code: "cli_auth_unavailable", message: "当前 gh 登录与所选账号不匹配或已失效。" } satisfies AppError;
        }
        const owner = browserGitHubPublishOwnersFor(account).find(
          (entry) => entry.login.toLowerCase() === ownerLogin.toLowerCase() && entry.canCreateRepositories,
        );
        if (!owner) {
          throw { code: "github_organization_publish_forbidden", message: "所选所有者不允许当前账号创建仓库。" } satisfies AppError;
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

export function isAppError(error: unknown): error is AppError {
  return typeof error === "object" && error !== null && "code" in error && "message" in error;
}
