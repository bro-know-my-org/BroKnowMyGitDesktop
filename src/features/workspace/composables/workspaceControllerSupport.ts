import type {
  AccountProfile,
  BranchInfo,
  CodeHostCapabilities,
  LfsStatus,
  RemoteBinding,
  RemoteBranchInfo,
  RemoteProfile,
  RepositoryRecord,
  RepositorySnapshot,
  SubmoduleInfo,
  WorktreeInfo,
} from "../../../shared/types/domain";

export type AppSection = "repositories" | "accounts";
export type RepositoryTab = "overview" | "changes" | "history" | "branches";
export type NetworkAction = "fetch" | "pull" | "push";
export type RepositoryGrouping = "manual" | "owner" | "recent";

export interface NetworkActionTarget {
  repositoryPath: string;
  remoteName: string;
  remoteIdentity: string;
  selectionGeneration: number;
}

export interface RepositorySidebarGroup {
  key: string;
  label: string;
  repositories: RepositoryRecord[];
}

export function repositoryRemoteLabel(repository: RepositoryRecord): string {
  if (!repository.remoteRepository) return "";
  return repository.remoteNamespace ? `${repository.remoteNamespace}/${repository.remoteRepository}` : repository.remoteRepository;
}

export function repositoryStarAriaLabel(repository: RepositoryRecord, action: string): string {
  return `${action}: ${repository.alias || repository.name} (${repository.path})`;
}

export function filterRepositoryGroups(
  groups: RepositorySidebarGroup[],
  rawQuery: string,
): RepositorySidebarGroup[] {
  const query = rawQuery.trim().toLowerCase();
  if (!query) return groups;
  return groups
    .map((group) => ({
      ...group,
      repositories: group.repositories.filter((repository) =>
        [repository.alias, repository.name, repository.path, repositoryRemoteLabel(repository)]
          .some((value) => value?.toLowerCase().includes(query))),
    }))
    .filter((group) => group.repositories.length);
}

export interface RepositoryChangedPayload {
  repositoryPath: string;
  requestGeneration: number;
  errorKind: string | null;
  retryable: boolean;
  historyObjectsChanged: boolean;
  stashesChanged: boolean;
  watchPlanChanged: boolean;
}

export interface RefreshSnapshotOptions {
  includeAuxiliary?: boolean;
  reportErrors?: boolean;
  silent?: boolean;
}

export interface RepositoryViewportSnapshot {
  element: HTMLElement;
  scrollTop: number;
  scrollLeft: number;
  interactionGeneration: number;
  repositoryPath: string | null;
  tab: RepositoryTab;
}

export interface HistoryLoadOptions {
  force?: boolean;
  reportErrors?: boolean;
  silent?: boolean;
  resetPagination?: boolean;
}

export const commitCacheLimit = 20;
export const commitSelectionDebounceMs = 75;
export const commitDiffInitialLineCount = 2_000;
export const commitDiffLineIncrement = 2_000;
export const commitDiffMaximumDisplayedLines = 10_000;
export const commitDiffPreviewBytes = 512 * 1024;
export const commitDiffMaximumBytes = 4 * 1024 * 1024;
export const commitFilesCacheMaximumWeight = 16 * 1024 * 1024;
export const commitDiffCacheMaximumWeight = 16 * 1024 * 1024;

export const repositoryGroupingStorageKey = "broknowmygitdesktop.repository-grouping";
const lastRepositoryStorageKey = "broknowmygitdesktop.last-repository";
const repositorySnapshotSignatures = new WeakMap<RepositorySnapshot, string>();

export const emptyCapabilities: CodeHostCapabilities = {
  repositoryInfo: false,
  repositoryPublish: false,
  repositoryFork: false,
  pullRequestsRead: false,
  pullRequestsWrite: false,
  accountImport: false,
  accountBindings: false,
  patKeychain: false,
};

export function repositorySnapshotSignature(repositorySnapshot: RepositorySnapshot): string {
  const cached = repositorySnapshotSignatures.get(repositorySnapshot);
  if (cached !== undefined) return cached;
  const signature = JSON.stringify(repositorySnapshot);
  repositorySnapshotSignatures.set(repositorySnapshot, signature);
  return signature;
}

export function initialRepositoryGrouping(): RepositoryGrouping {
  if (typeof window === "undefined") return "manual";
  try {
    const saved = window.localStorage.getItem(repositoryGroupingStorageKey);
    if (saved === "manual" || saved === "owner" || saved === "recent") return saved;
  } catch {
    // Disabled browser storage should not prevent the repository list from loading.
  }
  return "manual";
}

export function storedLastRepositoryPath(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(lastRepositoryStorageKey);
  } catch {
    return null;
  }
}

export function rememberLastRepositoryPath(repositoryPath: string): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(lastRepositoryStorageKey, repositoryPath);
  } catch {
    // The repository remains selected for this session if storage is unavailable.
  }
}

export function supportsGitRemoteCredentials(account: AccountProfile): boolean {
  // A saved binding is an explicit account-selection promise. Only the
  // PAT and gh's per-user token lookup can keep that promise; an SSH agent picks
  // its identity from the user's SSH configuration at connection time.
  return account.authMethod === "personalAccessToken" || account.authMethod === "ghCli";
}

export function supportsCloneCredentials(account: AccountProfile): boolean {
  // Clone has no durable remote binding yet, so an SSH-agent account remains
  // useful as a UI hint while Git/SSH selects the actual key.
  return supportsGitRemoteCredentials(account) || account.authMethod === "sshAgent";
}

export function endpointHasExplicitPort(gitHost: string): boolean {
  const value = gitHost.trim();
  const closingBracket = value.lastIndexOf("]");
  if (value.startsWith("[") && closingBracket !== -1) return /^:\d+$/u.test(value.slice(closingBracket + 1));
  return /:\d+$/u.test(value);
}

export function reuseEqualArray<T>(
  currentValue: T[],
  nextValue: T[],
  itemsAreEqual: (left: T, right: T) => boolean,
): T[] {
  return currentValue.length === nextValue.length
    && currentValue.every((item, index) => itemsAreEqual(item, nextValue[index]))
    ? currentValue
    : nextValue;
}

export function branchesAreEqual(left: BranchInfo, right: BranchInfo): boolean {
  return left.name === right.name && left.isCurrent === right.isCurrent && left.upstream === right.upstream;
}

export function remoteBranchesAreEqual(left: RemoteBranchInfo, right: RemoteBranchInfo): boolean {
  return left.remoteName === right.remoteName && left.name === right.name;
}

export function remotesAreEqual(left: RemoteProfile, right: RemoteProfile): boolean {
  return left.name === right.name
    && left.fetchUrl === right.fetchUrl
    && left.pushUrl === right.pushUrl
    && left.endpointId === right.endpointId
    && left.codeHostKind === right.codeHostKind
    && left.parsed.transport === right.parsed.transport
    && left.parsed.host === right.parsed.host
    && left.parsed.port === right.parsed.port
    && left.parsed.namespace === right.parsed.namespace
    && left.parsed.repository === right.parsed.repository;
}

export function bindingsAreEqual(left: RemoteBinding, right: RemoteBinding): boolean {
  return left.repositoryPath === right.repositoryPath
    && left.remoteName === right.remoteName
    && left.endpointId === right.endpointId
    && left.accountSelection.mode === right.accountSelection.mode
    && (left.accountSelection.mode === "auto" || (
      right.accountSelection.mode === "profile"
      && left.accountSelection.accountProfileId === right.accountSelection.accountProfileId
    ));
}

export function worktreesAreEqual(left: WorktreeInfo, right: WorktreeInfo): boolean {
  return left.path === right.path
    && left.head === right.head
    && left.branch === right.branch
    && left.isMain === right.isMain
    && left.isDetached === right.isDetached
    && left.isLocked === right.isLocked
    && left.isPrunable === right.isPrunable;
}

export function submodulesAreEqual(left: SubmoduleInfo, right: SubmoduleInfo): boolean {
  return left.path === right.path && left.commit === right.commit && left.status === right.status;
}

export function lfsStatusesAreEqual(left: LfsStatus, right: LfsStatus): boolean {
  return left.available === right.available
    && left.initialized === right.initialized
    && left.version === right.version
    && left.files.length === right.files.length
    && left.files.every((file, index) => file === right.files[index]);
}

export function splitVisibleDiffLines(diff: string, maximumLines: number): { lines: string[]; hasMore: boolean } {
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

export function historySnapshotSignature(repositorySnapshot: RepositorySnapshot | null): string {
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
