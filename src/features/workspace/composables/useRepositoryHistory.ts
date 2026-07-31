import { computed, ref, type Ref } from "vue";
import { api } from "../../../shared/api/tauri";
import { t } from "../../../shared/i18n";
import type {
  CommitChangedFile,
  CommitFileDiff,
  CommitSummary,
  RepositorySnapshot,
  StashEntry,
} from "../../../shared/types/domain";
import {
  commitCacheLimit,
  commitDiffCacheMaximumWeight,
  commitDiffInitialLineCount,
  commitDiffLineIncrement,
  commitDiffMaximumBytes,
  commitDiffMaximumDisplayedLines,
  commitDiffPreviewBytes,
  commitFilesCacheMaximumWeight,
  commitSelectionDebounceMs,
  historySnapshotSignature,
  splitVisibleDiffLines,
  type HistoryLoadOptions,
} from "./workspaceControllerSupport";

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

interface RepositoryHistoryDependencies {
  activeRepositoryPath: Ref<string | null>;
  snapshot: Ref<RepositorySnapshot | null>;
  getSelectionGeneration: () => number;
  repositorySelectionIsCurrent: (repositoryPath: string, generation: number) => boolean;
  isControllerMounted: () => boolean;
  reportError: (error: unknown, action: string) => void;
}

export function useRepositoryHistory({
  activeRepositoryPath,
  snapshot,
  getSelectionGeneration,
  repositorySelectionIsCurrent,
  isControllerMounted,
  reportError,
}: RepositoryHistoryDependencies) {
  const commits = ref<CommitSummary[]>([]);
  const stashes = ref<StashEntry[]>([]);
  const reflogText = ref("");
  const selectedCommit = ref<CommitSummary | null>(null);
  const commitChangedFiles = ref<CommitChangedFile[]>([]);
  const selectedCommitFile = ref<CommitChangedFile | null>(null);
  const commitDiffText = ref("");
  const commitDiffTruncated = ref(false);
  const commitDiffMaximumReached = ref(false);
  const historyLoading = ref(false);
  const reflogLoading = ref(false);
  const commitFilesLoading = ref(false);
  const commitDiffLoading = ref(false);
  const stashesLoading = ref(false);
  const reflogModalShown = ref(false);
  let historyRequestGeneration = 0;
  let historyLoadedRepositoryPath: string | null = null;
  let historyLoadedSignature: string | null = null;
  let stashesRequestGeneration = 0;
  let stashesLoadedRepositoryPath: string | null = null;
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

  const commitDiffVisibleLineCount = ref(commitDiffInitialLineCount);
  const commitDiffVisibleChunk = computed(() =>
    splitVisibleDiffLines(commitDiffText.value, commitDiffVisibleLineCount.value));
  const commitDiffVisibleLines = computed(() => commitDiffVisibleChunk.value.lines);
  const commitDiffHasMoreLines = computed(() => commitDiffVisibleChunk.value.hasMore);
  const commitDiffLineLimitReached = computed(() =>
    commitDiffHasMoreLines.value && commitDiffVisibleLineCount.value >= commitDiffMaximumDisplayedLines);
  const commitDiffCanExpand = computed(() =>
    (!commitDiffLineLimitReached.value && commitDiffHasMoreLines.value)
    || (commitDiffTruncated.value && !commitDiffMaximumReached.value));
  const commitDiffLimitedAtMaximum = computed(() =>
    commitDiffTruncated.value && commitDiffMaximumReached.value);

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

  function showMoreCommitDiffLines(): void {
    if (commitDiffLineLimitReached.value && commitDiffTruncated.value && !commitDiffMaximumReached.value) {
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

  function invalidateCommitDiffCache(): void {
    commitDiffCache.clear();
    commitDiffCacheWeight = 0;
  }

  function historyWasLoaded(repositoryPath: string): boolean {
    return historyLoadedRepositoryPath === repositoryPath || stashesLoadedRepositoryPath === repositoryPath;
  }

  function resetRepositoryHistory(): void {
    invalidateCommitHistoryCache();
    invalidateStashesCache();
    clearCommitSelection();
    commits.value = [];
    stashes.value = [];
  }

  function disposeRepositoryHistory(): void {
    if (commitFilesDebounceTimer) clearTimeout(commitFilesDebounceTimer);
    if (commitDiffDebounceTimer) clearTimeout(commitDiffDebounceTimer);
    commitFilesDebounceTimer = null;
    commitDiffDebounceTimer = null;
    pendingCommitDiffRequest = null;
    commitFilesRequestGeneration += 1;
    commitDiffRequestGeneration += 1;
  }

  async function loadHistory(): Promise<void> {
    await loadHistoryWithOptions({ force: true });
  }

  async function loadHistoryWithOptions(options: HistoryLoadOptions = {}): Promise<void> {
    await Promise.all([loadCommitHistoryWithOptions(options), loadStashesWithOptions(options)]);
  }

  async function loadCommitHistoryWithOptions(options: HistoryLoadOptions = {}): Promise<void> {
    const repositoryPath = activeRepositoryPath.value;
    if (!repositoryPath) return;
    const { force = false, reportErrors = true, silent = false } = options;
    const requestHistorySignature = historySnapshotSignature(snapshot.value);
    if (!force && historyLoadedRepositoryPath === repositoryPath && historyLoadedSignature === requestHistorySignature) return;
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
        activeRepositoryPath.value !== repositoryPath
        || requestGeneration !== historyRequestGeneration
        || historySnapshotSignature(snapshot.value) !== requestHistorySignature
      ) return;
      commits.value = nextCommits;
      historyLoadedRepositoryPath = repositoryPath;
      historyLoadedSignature = requestHistorySignature;
      if (selectedCommit.value && !nextCommits.some((commit) => commit.sha === selectedCommit.value?.sha)) {
        clearCommitSelection();
      }
    } catch (error) {
      if (activeRepositoryPath.value === repositoryPath && requestGeneration === historyRequestGeneration && reportErrors) {
        reportError(error, t("workspace.controller.couldNotLoadCommitHistory"));
      }
    } finally {
      if (!silent && activeRepositoryPath.value === repositoryPath && requestGeneration === historyRequestGeneration) {
        historyLoading.value = false;
      }
    }
  }

  async function loadStashesWithOptions(options: HistoryLoadOptions = {}): Promise<void> {
    const repositoryPath = activeRepositoryPath.value;
    if (!repositoryPath) return;
    const { force = false, reportErrors = true, silent = false } = options;
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
      if (activeRepositoryPath.value === repositoryPath && requestGeneration === stashesRequestGeneration && reportErrors) {
        reportError(error, t("workspace.controller.couldNotLoadStashes"));
      }
    } finally {
      if (!silent && activeRepositoryPath.value === repositoryPath && requestGeneration === stashesRequestGeneration) {
        stashesLoading.value = false;
      }
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

  function loadCommitFiles(commit: CommitSummary): void {
    const repositoryPath = activeRepositoryPath.value;
    if (!repositoryPath) return;
    const selectionGeneration = getSelectionGeneration();
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
        repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && selectedCommit.value?.sha === commit.sha
        && requestGeneration === commitFilesRequestGeneration
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
    const selectionGeneration = getSelectionGeneration();
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
    const selectionGeneration = getSelectionGeneration();
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
      while (isControllerMounted() && pendingCommitDiffRequest) {
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
        repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && selectedCommit.value?.sha === commit.sha
        && selectedCommitFile.value?.path === file.path
        && requestGeneration === commitDiffRequestGeneration
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

  return {
    commits,
    stashes,
    reflogText,
    selectedCommit,
    commitChangedFiles,
    selectedCommitFile,
    commitDiffText,
    commitDiffTruncated,
    commitDiffMaximumReached,
    historyLoading,
    reflogLoading,
    commitFilesLoading,
    commitDiffLoading,
    stashesLoading,
    reflogModalShown,
    commitDiffVisibleLineCount,
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
  };
}
