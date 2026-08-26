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

export interface RepositoryNetworkSecurityStatus {
  repositoryPath: string;
  unsafeKeys: string[];
  currentFingerprint: string | null;
  authorizationAllowed: boolean;
  trusted: boolean;
  authorizedKeys: string[];
  authorizedFingerprint: string | null;
  authorizationStale: boolean;
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
  repositoryFork: boolean;
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

/** App-local terminal used only by the explicit preferred-terminal action. */
export interface TerminalConfig {
  executable: string;
  arguments: string[];
}

export interface SystemTerminalInfo {
  executable: string;
  arguments: string[];
}

export interface GitStatus {
  headCommit: string | null;
  branch: string | null;
  upstream: string | null;
  upstreamCommit: string | null;
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
  tags: string[];
  tagsTruncated: boolean;
  pushed: boolean | null;
  authorName: string;
  authorEmail: string;
  authoredAt: string;
  summary: string;
}

export type CommitFileStatus =
  | "added"
  | "copied"
  | "deleted"
  | "modified"
  | "renamed"
  | "typeChanged"
  | "unmerged"
  | "unknown";

export interface CommitChangedFile {
  path: string;
  previousPath: string | null;
  status: CommitFileStatus;
  similarity: number | null;
}

export interface CommitFileDiff {
  text: string;
  truncated: boolean;
}

export interface StashEntry {
  reference: string;
  commitId: string;
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
  alias: string | null;
  group: string | null;
  host: string | null;
  namespace: string | null;
  lastOpenedAt: number | null;
}

export interface RepositoryDiscoveryResult {
  repositories: RepositoryRecord[];
  importLimit: number;
}

export interface RepositoryImportResult {
  repositories: RepositoryRecord[];
  rejectedCount: number;
  remainingCapacity: number;
  persistenceWarning?: boolean;
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

export interface CreateBranchResult {
  snapshot: RepositorySnapshot;
  stashedLocalChanges: boolean;
}

export interface NetworkOperationResult {
  summary: string;
}

export interface CleanUntrackedPreview {
  preview: string;
  confirmation: string;
}

export interface DiscardChangesPreview {
  confirmation: string;
  pathCount: number;
  untrackedCount: number;
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

export interface GitHubPublishOwner {
  login: string;
  name: string | null;
  kind: "user" | "organization";
  canCreateRepositories: boolean;
}

export interface GitHubForkSummary {
  fullName: string;
  ownerLogin: string;
  repositoryName: string;
  webUrl: string;
  cloneUrl: string;
  sshUrl: string;
  remoteName: string;
  sourceRemoteName: string;
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
  /** Save tracked and untracked worktree changes before creating and switching. */
  stashLocalChanges?: boolean;
}
