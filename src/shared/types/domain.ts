/**
 * Shared, non-secret IPC shapes.  These mirror the Rust domain model, but
 * deliberately never include a credential value or an askpass environment.
 */
export type CodeHostKind =
  | "gitHub"
  | "gitLab"
  | "gitee"
  | "gitea"
  | "forgejo"
  | "genericGit"
  | "unknown";

export type AuthenticationMethod =
  | "sshAgent"
  | "systemGitCredentialHelper"
  | "ghCli"
  | "glabCli"
  | "personalAccessToken";

export type AuthenticationStatus =
  | "unknown"
  | "available"
  | "valid"
  | "needsReauthentication"
  | "unavailable";

export type RemoteTransport = "https" | "http" | "ssh" | "git" | "local" | "other";
export type RemoteUrlKind = "fetch" | "push";

export interface AppError {
  code: string;
  message: string;
  details?: string | null;
}

export interface CodeHostEndpoint {
  id: string;
  kind: CodeHostKind;
  displayName: string;
  webUrl: string;
  apiUrl: string;
  gitHost: string;
  isBuiltin: boolean;
}

export interface CodeHostCapabilities {
  repositoryInfo: boolean;
  repositoryPublish: boolean;
  pullRequestsRead: boolean;
  pullRequestsWrite: boolean;
  accountImport: boolean;
  accountBindings: boolean;
  patKeychain: boolean;
}

export interface KeychainReference {
  service: string;
  account: string;
}

export interface AccountProfile {
  id: string;
  endpointId: string;
  login: string;
  displayName: string | null;
  authMethod: AuthenticationMethod;
  secretRef: KeychainReference | null;
  status: AuthenticationStatus;
  isDefaultCandidate: boolean;
}

export interface ParsedRemoteUrl {
  transport: RemoteTransport;
  host: string | null;
  port: number | null;
  namespace: string | null;
  repository: string | null;
}

export interface RemoteProfile {
  name: string;
  fetchUrl: string;
  pushUrl: string;
  parsed: ParsedRemoteUrl;
  endpointId: string | null;
  codeHostKind: CodeHostKind;
}

export type AccountSelection =
  | { mode: "auto" }
  | { mode: "profile"; accountProfileId: string };

export interface RemoteBinding {
  repositoryPath: string;
  remoteName: string;
  endpointId: string;
  accountSelection: AccountSelection;
}

export interface GitStatusEntry {
  path: string;
  originalPath: string | null;
  indexStatus: string;
  worktreeStatus: string;
}

/**
 * App-local external editor launch settings. Each argument is separate so
 * conflict paths never have to be interpreted as shell command text.
 */
export interface ExternalEditorConfig {
  executable: string;
  arguments: string[];
}

export interface GitStatus {
  branch: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  entries: GitStatusEntry[];
}

export interface BranchInfo {
    name: string;
    isCurrent: boolean;
    upstream: string | null;
}

export interface RemoteBranchInfo {
  remoteName: string;
  name: string;
}

export interface WorktreeInfo {
  path: string;
  head: string;
  branch: string | null;
  isMain: boolean;
  isDetached: boolean;
  isLocked: boolean;
  isPrunable: boolean;
}

export type SubmoduleStatus = "initialized" | "uninitialized" | "needsUpdate" | "conflicted";

export interface SubmoduleInfo {
  path: string;
  commit: string;
  status: SubmoduleStatus;
}

export interface LfsStatus {
  available: boolean;
  initialized: boolean;
  version: string | null;
  files: string[];
}

export interface CommitSummary {
  sha: string;
  shortSha: string;
  parents: string[];
  authorName: string;
  authorEmail: string;
  authoredAt: string;
  summary: string;
}

export interface StashEntry {
  reference: string;
  message: string;
}

export type ResetMode = "soft" | "mixed" | "hard";

export type GitOperationKind = "merge" | "rebase" | "cherryPick" | "revert";

export interface GitOperationState {
  kind: GitOperationKind;
  hasConflicts: boolean;
}

export interface TagInfo {
  name: string;
  target: string;
}

export interface RepositoryRecord {
  path: string;
  name: string;
  group: string | null;
  host: string | null;
  namespace: string | null;
  lastOpenedAt: number | null;
}

export interface RepositorySnapshot {
  repository: RepositoryRecord;
  hasHead: boolean;
  status: GitStatus;
  branches: BranchInfo[];
  remoteBranches: RemoteBranchInfo[];
  remotes: RemoteProfile[];
  tags: TagInfo[];
  bindings: RemoteBinding[];
  operation: GitOperationState | null;
}

export interface NetworkOperationResult {
  summary: string;
}

export interface CodeHostRepositoryInfo {
  fullName: string;
  description: string | null;
  defaultBranch: string | null;
  webUrl: string;
  isPrivate: boolean | null;
}

export interface PullRequestSummary {
  number: number;
  title: string;
  author: string;
  state: string;
  webUrl: string;
  updatedAt: string | null;
}

export interface CreatePullRequestInput {
  title: string;
  body: string;
  head: string;
  base: string;
  draft: boolean;
}

export interface CloneRepositoryInput {
  url: string;
  destinationPath: string;
  /** Optional explicit identity for a private clone. Omit to use normal Git behavior. */
  endpointId?: string;
  accountProfileId?: string;
}

export interface InitRepositoryInput {
  destinationPath: string;
  initialBranch?: string;
}

/** PAT is accepted only for the write request and must never be retained by UI state. */
export interface SavePatAccountInput {
  endpointId: string;
  login: string;
  displayName?: string;
  token: string;
}

export interface SaveAccountProfileInput {
  endpointId: string;
  login: string;
  displayName?: string;
  authMethod: Exclude<AuthenticationMethod, "personalAccessToken">;
}

export interface CreateBranchInput {
  repositoryPath: string;
  name: string;
  checkout: boolean;
}

export function endpointKindLabel(kind: CodeHostKind, locale: "zh-CN" | "en-US" = "zh-CN"): string {
  const labels: Record<CodeHostKind, [string, string]> = {
    gitHub: ["GitHub", "GitHub"],
    gitLab: ["GitLab", "GitLab"],
    gitee: ["Gitee", "Gitee"],
    gitea: ["Gitea", "Gitea"],
    forgejo: ["Forgejo", "Forgejo"],
    genericGit: ["通用 Git 服务", "Generic Git service"],
    unknown: ["未识别平台", "Unrecognized service"],
  };
  return labels[kind][locale === "zh-CN" ? 0 : 1];
}

export function authenticationMethodLabel(method: AuthenticationMethod, locale: "zh-CN" | "en-US" = "zh-CN"): string {
  const labels: Record<AuthenticationMethod, [string, string]> = {
    sshAgent: ["SSH agent", "SSH agent"],
    systemGitCredentialHelper: ["系统 Git credential helper", "System Git credential helper"],
    ghCli: ["GitHub CLI (gh)", "GitHub CLI (gh)"],
    glabCli: ["GitLab CLI (glab)", "GitLab CLI (glab)"],
    personalAccessToken: ["个人访问令牌 (PAT)", "Personal access token (PAT)"],
  };
  return labels[method][locale === "zh-CN" ? 0 : 1];
}

export function authenticationStatusLabel(status: AuthenticationStatus, locale: "zh-CN" | "en-US" = "zh-CN"): string {
  const labels: Record<AuthenticationStatus, [string, string]> = {
    unknown: ["未检测", "Unchecked"],
    available: ["可用", "Available"],
    valid: ["有效", "Valid"],
    needsReauthentication: ["需要重新认证", "Reauthentication required"],
    unavailable: ["不可用", "Unavailable"],
  };
  return labels[status][locale === "zh-CN" ? 0 : 1];
}
