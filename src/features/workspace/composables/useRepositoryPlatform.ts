import { computed, reactive, ref, watch, type ComputedRef, type Ref } from "vue";
import type { MessageApi } from "naive-ui";
import { api, isAppError } from "../../../shared/api/tauri";
import { authenticationStatusLabel, localizedErrorMessage, t } from "../../../shared/i18n";
import type {
  AccountProfile,
  AppError,
  CodeHostCapabilities,
  CodeHostEndpoint,
  CodeHostRepositoryInfo,
  CommitSummary,
  GitHubPublishOwner,
  PullRequestSummary,
  RemoteProfile,
  RepositorySnapshot,
} from "../../../shared/types/domain";
import { acquireRepositoryOperation } from "./repositoryOperationLock";

interface RepositoryPlatformDependencies {
  activeRepositoryPath: Ref<string | null>;
  activeRemote: ComputedRef<RemoteProfile | null>;
  activeCapabilities: Ref<CodeHostCapabilities>;
  snapshot: Ref<RepositorySnapshot | null>;
  endpoints: Ref<CodeHostEndpoint[]>;
  accounts: Ref<AccountProfile[]>;
  selectedCommit: Ref<CommitSummary | null>;
  commits: Ref<CommitSummary[]>;
  getSelectionGeneration: () => number;
  repositorySelectionIsCurrent: (repositoryPath: string, generation: number) => boolean;
  capabilityForEndpoint: (endpointId: string | null) => CodeHostCapabilities;
  selectedRemoteName: Ref<string>;
  applySnapshot: (snapshot: RepositorySnapshot) => boolean;
  loadCapabilities: (repositoryPath?: string, selectionGeneration?: number) => Promise<void>;
  refreshSnapshot: () => Promise<void>;
  reportError: (error: unknown, action: string) => void;
  message: MessageApi;
}

function getRemoteIdentity(remote: RemoteProfile | null): string | null {
  if (!remote) return null;
  return [
    remote.name,
    remote.fetchUrl,
    remote.pushUrl,
    remote.endpointId ?? "",
    remote.codeHostKind,
  ].join("\u0000");
}

export function useRepositoryPlatform({
  activeRepositoryPath,
  activeRemote,
  activeCapabilities,
  snapshot,
  endpoints,
  accounts,
  selectedCommit,
  commits,
  getSelectionGeneration,
  repositorySelectionIsCurrent,
  capabilityForEndpoint,
  selectedRemoteName,
  applySnapshot,
  loadCapabilities,
  refreshSnapshot,
  reportError,
  message,
}: RepositoryPlatformDependencies) {
  const repositoryInfo = ref<CodeHostRepositoryInfo | null>(null);
  const pullRequests = ref<PullRequestSummary[]>([]);
  const platformLoading = ref(false);
  const pullRequestsLoading = ref(false);
  const publishOwnersLoading = ref(false);
  const pullRequestModalShown = ref(false);
  const publishModalShown = ref(false);
  const forkModalShown = ref(false);
  const pullRequestForm = reactive({ title: "", body: "", base: "", draft: false });
  const pullRequestHead = ref<string | null>(null);
  const forkForm = reactive({
    accountId: "",
    remoteName: "fork",
    createContributionBranch: false,
    branchName: "",
    stashLocalChanges: false,
  });
  const publishForm = reactive({
    endpointId: "github.com",
    accountId: "",
    ownerLogin: "",
    name: "",
    isPrivate: true,
  });
  const publishOwners = ref<GitHubPublishOwner[]>([]);
  const publishOwnersError = ref("");
  let publishOwnersRequestGeneration = 0;
  let repositoryInfoRequestGeneration = 0;
  let pullRequestsRequestGeneration = 0;
  const pullRequestRepositoryPath = ref<string | null>(null);
  const pullRequestRemoteName = ref<string | null>(null);
  const pullRequestRemoteIdentity = ref<string | null>(null);
  const pullRequestSelectionGeneration = ref<number | null>(null);
  const publishRepositoryPath = ref<string | null>(null);
  const publishSelectionGeneration = ref<number | null>(null);
  const forkRepositoryPath = ref<string | null>(null);
  const forkRemoteName = ref<string | null>(null);
  const forkRemoteIdentity = ref<string | null>(null);
  const forkSelectionGeneration = ref<number | null>(null);

  function isDefaultContributionBranch(branch: string): boolean {
    const configuredDefaultBranch = repositoryInfo.value?.defaultBranch?.trim();
    if (configuredDefaultBranch) return configuredDefaultBranch === branch;
    return ["main", "master", "trunk"].includes(branch);
  }

  function branchNamePart(value: string): string {
    const normalized = value.normalize("NFKD").toLowerCase();
    return normalized.replace(/[^a-z0-9]+/gu, "-").replace(/^-+|-+$/gu, "").slice(0, 48);
  }

  function suggestedContributionBranchName(): string {
    const summary = selectedCommit.value?.summary || commits.value[0]?.summary || "change";
    const base = `contrib/${branchNamePart(summary) || "change"}`;
    const names = new Set(snapshot.value?.branches.map((branch) => branch.name) ?? []);
    if (!names.has(base)) return base;
    let suffix = 2;
    while (names.has(`${base}-${suffix}`)) suffix += 1;
    return `${base}-${suffix}`;
  }

  const forkContributionBranchRecommended = computed(() => {
    const branch = snapshot.value?.status.branch;
    return Boolean(branch && isDefaultContributionBranch(branch));
  });

  function beginRepositoryMutation(repositoryPath: string): (() => void) | null {
    const releaseOperation = acquireRepositoryOperation(repositoryPath);
    if (!releaseOperation) {
      message.warning(t("workspace.controller.aRepositoryOperationIsAlreadyInProgressWaitForItToFinish"));
      return null;
    }
    return () => {
      releaseOperation();
    };
  }

  const pullRequestBaseOptions = computed(() => {
    const branches = snapshot.value?.branches
      .filter((branch) => !branch.isCurrent)
      .map((branch) => branch.name) ?? [];
    const defaultBranch = repositoryInfo.value?.defaultBranch;
    const values = [...new Set([defaultBranch, ...branches].filter(
      (branch): branch is string => Boolean(branch),
    ))];
    return values.map((branch) => ({ label: branch, value: branch }));
  });
  const publishEndpoints = computed(() => endpoints.value.filter(
    (endpoint) => endpoint.kind === "gitHub" && capabilityForEndpoint(endpoint.id).repositoryPublish,
  ));
  const publishAccounts = computed(() => accounts.value.filter((account) =>
    account.endpointId === publishForm.endpointId
    && account.authMethod === "ghCli"
    && (account.status === "valid" || account.status === "available")));
  const publishAccountOptions = computed(() => publishAccounts.value.map((account) => ({
    label: (account.displayName || account.login) + " · " + authenticationStatusLabel(account.status),
    value: account.id,
  })));
  const forkAccounts = computed(() => accounts.value.filter((account) =>
    account.endpointId === activeRemote.value?.endpointId
    && account.authMethod === "ghCli"
    && (account.status === "valid" || account.status === "available"),
  ));
  const forkAccountOptions = computed(() => forkAccounts.value.map((account) => ({
    label: (account.displayName || account.login) + " · " + authenticationStatusLabel(account.status),
    value: account.id,
  })));
  const publishOwnerOptions = computed(() => publishOwners.value.map((owner) => ({
    label: (owner.name
      ? `${owner.name} (${owner.login}) · ${owner.kind === "organization" ? t("workspace.controller.organization") : t("workspace.controller.personal")}`
      : `${owner.login} · ${owner.kind === "organization" ? t("workspace.controller.organization") : t("workspace.controller.personal")}`)
      + (owner.canCreateRepositories ? "" : t("workspace.controller.noCreatePermission")),
    value: owner.login,
    disabled: !owner.canCreateRepositories,
  })));
  const canPublishToSelectedOwner = computed(() =>
    !publishOwnersLoading.value
    && !publishOwnersError.value
    && publishOwners.value.some((owner) =>
      owner.login === publishForm.ownerLogin && owner.canCreateRepositories));

  async function loadRepositoryInfo(): Promise<void> {
    const repositoryPath = activeRepositoryPath.value;
    const remote = activeRemote.value;
    const selectionGeneration = getSelectionGeneration();
    if (!repositoryPath || !remote || !activeCapabilities.value.repositoryInfo) return;
    const remoteName = remote.name;
    const remoteIdentity = getRemoteIdentity(remote);
    const requestGeneration = ++repositoryInfoRequestGeneration;
    platformLoading.value = true;
    try {
      const nextRepositoryInfo = await api.getCodeHostRepositoryInfo(repositoryPath, remoteName);
      if (
        requestGeneration !== repositoryInfoRequestGeneration
        || !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        || getRemoteIdentity(activeRemote.value) !== remoteIdentity
      ) return;
      repositoryInfo.value = nextRepositoryInfo;
    } catch (error) {
      if (
        requestGeneration === repositoryInfoRequestGeneration
        && repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && getRemoteIdentity(activeRemote.value) === remoteIdentity
      ) reportError(error, t("workspace.controller.couldNotLoadGitServiceRepositoryInformation"));
    } finally {
      if (requestGeneration === repositoryInfoRequestGeneration) platformLoading.value = false;
    }
  }

  async function loadPullRequests(): Promise<void> {
    const repositoryPath = activeRepositoryPath.value;
    const remote = activeRemote.value;
    const selectionGeneration = getSelectionGeneration();
    if (!repositoryPath || !remote || !activeCapabilities.value.pullRequestsRead) return;
    const remoteName = remote.name;
    const remoteIdentity = getRemoteIdentity(remote);
    const requestGeneration = ++pullRequestsRequestGeneration;
    pullRequestsLoading.value = true;
    try {
      const nextPullRequests = await api.listPullRequests(repositoryPath, remoteName);
      if (
        requestGeneration !== pullRequestsRequestGeneration
        || !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        || getRemoteIdentity(activeRemote.value) !== remoteIdentity
      ) return;
      pullRequests.value = nextPullRequests;
    } catch (error) {
      if (
        requestGeneration === pullRequestsRequestGeneration
        && repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && getRemoteIdentity(activeRemote.value) === remoteIdentity
      ) reportError(error, t("workspace.controller.couldNotLoadPullRequests"));
    } finally {
      if (requestGeneration === pullRequestsRequestGeneration) pullRequestsLoading.value = false;
    }
  }

  function invalidatePlatformRequests(): void {
    repositoryInfoRequestGeneration += 1;
    pullRequestsRequestGeneration += 1;
    platformLoading.value = false;
    pullRequestsLoading.value = false;
  }

  watch(
    () => getRemoteIdentity(activeRemote.value),
    (nextIdentity, previousIdentity) => {
      if (nextIdentity === previousIdentity) return;
      invalidatePlatformRequests();
      repositoryInfo.value = null;
      pullRequests.value = [];
      if (pullRequestModalShown.value) {
        pullRequestModalShown.value = false;
        pullRequestHead.value = null;
        pullRequestRepositoryPath.value = null;
        pullRequestRemoteName.value = null;
        pullRequestRemoteIdentity.value = null;
        pullRequestSelectionGeneration.value = null;
      }
    },
  );

  function prepareCreatePullRequest(headOverride?: string): void {
    const repositoryPath = activeRepositoryPath.value;
    const remote = activeRemote.value;
    const remoteName = remote?.name;
    const remoteIdentity = getRemoteIdentity(remote);
    if (!repositoryPath || !remoteName || !remoteIdentity) return;
    pullRequestRepositoryPath.value = repositoryPath;
    pullRequestRemoteName.value = remoteName;
    pullRequestRemoteIdentity.value = remoteIdentity;
    pullRequestSelectionGeneration.value = getSelectionGeneration();
    const suggestedBase = repositoryInfo.value?.defaultBranch || pullRequestBaseOptions.value[0]?.value || "";
    pullRequestForm.title = selectedCommit.value?.summary || commits.value[0]?.summary || "";
    pullRequestForm.body = "";
    pullRequestForm.base = suggestedBase;
    pullRequestForm.draft = false;
    pullRequestHead.value = headOverride?.trim() || null;
    pullRequestModalShown.value = true;
  }

  function openCreatePullRequest(): void {
    prepareCreatePullRequest();
  }

  function suggestedForkRemoteName(): string {
    const names = new Set(snapshot.value?.remotes.map((remote) => remote.name) ?? []);
    if (!names.has("fork")) return "fork";
    let suffix = 2;
    while (names.has(`fork-${suffix}`)) suffix += 1;
    return `fork-${suffix}`;
  }

  function openForkModal(): void {
    const repositoryPath = activeRepositoryPath.value;
    const remote = activeRemote.value;
    if (
      !repositoryPath
      || !remote
      || !activeCapabilities.value.repositoryFork
      || !snapshot.value?.hasHead
      || !snapshot.value.status.branch
    ) return;
    forkRepositoryPath.value = repositoryPath;
    forkRemoteName.value = remote.name;
    forkRemoteIdentity.value = getRemoteIdentity(remote);
    forkSelectionGeneration.value = getSelectionGeneration();
    forkForm.accountId = forkAccounts.value[0]?.id || "";
    forkForm.remoteName = suggestedForkRemoteName();
    forkForm.createContributionBranch = isDefaultContributionBranch(snapshot.value.status.branch);
    forkForm.branchName = suggestedContributionBranchName();
    forkForm.stashLocalChanges = false;
    forkModalShown.value = true;
  }

  async function forkGithubRepository(): Promise<void> {
    const repositoryPath = forkRepositoryPath.value;
    const sourceRemoteName = forkRemoteName.value;
    const sourceRemoteIdentity = forkRemoteIdentity.value;
    const selectionGeneration = forkSelectionGeneration.value;
    const remote = activeRemote.value;
    if (
      !repositoryPath
      || !sourceRemoteName
      || !sourceRemoteIdentity
      || selectionGeneration === null
      || !remote
      || !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
      || remote.name !== sourceRemoteName
      || getRemoteIdentity(remote) !== sourceRemoteIdentity
    ) {
      forkModalShown.value = false;
      message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
      return;
    }
    const accountId = forkForm.accountId;
    const requestedForkRemoteName = forkForm.remoteName.trim();
    const createContributionBranch = forkForm.createContributionBranch;
    const branchName = forkForm.branchName.trim();
    const stashLocalChanges = forkForm.stashLocalChanges;
    const endpointId = remote.endpointId;
    if (!endpointId || !accountId || !requestedForkRemoteName) {
      message.warning(t("workspace.controller.selectAGhAccountAndEnterAForkRemoteName"));
      return;
    }
    if (requestedForkRemoteName === sourceRemoteName) {
      message.warning(t("workspace.controller.chooseAForkRemoteNameDifferentFromTheSourceRemote"));
      return;
    }
    const account = forkAccounts.value.find((item) => item.id === accountId);
    if (!account) {
      message.warning(t("workspace.controller.selectAGhAccountAndEnterAForkRemoteName"));
      return;
    }
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    let contributionBranch = snapshot.value?.status.branch;
    if (!contributionBranch) {
      finishMutation();
      message.warning(t("workspace.controller.switchToALocalBranchBeforeCreatingAPullRequest"));
      return;
    }
    if (createContributionBranch && !branchName) {
      finishMutation();
      message.warning(t("workspace.controller.enterAContributionBranchName"));
      return;
    }
    try {
      if (createContributionBranch) {
        try {
          const branchResult = await api.createBranch({
            repositoryPath,
            name: branchName,
            checkout: true,
            stashLocalChanges,
          });
          const nextSnapshot = branchResult.snapshot;
          if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) return;
          selectedRemoteName.value = sourceRemoteName;
          contributionBranch = nextSnapshot.status.branch || branchName;
        } catch (error) {
          if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
            reportError(error, t("workspace.controller.couldNotCreateContributionBranch"));
          }
          return;
        }
      }
      if (
        !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        || getRemoteIdentity(activeRemote.value) !== sourceRemoteIdentity
        || snapshot.value?.status.branch !== contributionBranch
      ) {
        forkModalShown.value = false;
        message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
        return;
      }
      const fork = await api.forkGitHubRepository(
        repositoryPath,
        sourceRemoteName,
        endpointId,
        accountId,
        requestedForkRemoteName,
      );
      if (
        !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        || getRemoteIdentity(activeRemote.value) !== sourceRemoteIdentity
        || snapshot.value?.status.branch !== contributionBranch
      ) {
        forkModalShown.value = false;
        message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
        return;
      }
      selectedRemoteName.value = fork.remoteName;
      await refreshSnapshot();
      if (
        !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        || activeRemote.value?.name !== fork.remoteName
        || snapshot.value?.status.branch !== contributionBranch
      ) {
        forkModalShown.value = false;
        message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
        return;
      }
      try {
        await api.pushRemote(repositoryPath, fork.remoteName, false);
      } catch (error) {
        await refreshSnapshot().catch(() => undefined);
        if (
          repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
          && activeRemote.value?.name === fork.remoteName
        ) {
          reportError(error, t("workspace.controller.forkCreatedButCurrentBranchCouldNotBePushed"));
        }
        return;
      }
      await refreshSnapshot();
      if (
        !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        || activeRemote.value?.name !== fork.remoteName
        || snapshot.value?.status.branch !== contributionBranch
      ) {
        forkModalShown.value = false;
        message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
        return;
      }
      selectedRemoteName.value = fork.sourceRemoteName;
      forkModalShown.value = false;
      forkRepositoryPath.value = null;
      forkRemoteName.value = null;
      forkRemoteIdentity.value = null;
      forkSelectionGeneration.value = null;
      forkForm.createContributionBranch = false;
      forkForm.branchName = "";
      forkForm.stashLocalChanges = false;
      prepareCreatePullRequest(`${fork.ownerLogin}:${contributionBranch}`);
      message.success(t("workspace.controller.repositoryForkedAndBranchPushed", {
        branch: contributionBranch,
        remote: fork.remoteName,
      }));
    } catch (error) {
      if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
        reportError(error, t("workspace.controller.couldNotForkGithubRepository"));
      }
    } finally {
      finishMutation();
    }
  }

  async function createPullRequest(): Promise<void> {
    const repositoryPath = pullRequestRepositoryPath.value;
    const remoteName = pullRequestRemoteName.value;
    const remoteIdentity = pullRequestRemoteIdentity.value;
    const selectionGeneration = pullRequestSelectionGeneration.value;
    if (
      !repositoryPath
      || !remoteName
      || !remoteIdentity
      || selectionGeneration === null
      || !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
      || getRemoteIdentity(activeRemote.value) !== remoteIdentity
    ) {
      pullRequestModalShown.value = false;
      pullRequestHead.value = null;
      pullRequestRepositoryPath.value = null;
      pullRequestRemoteName.value = null;
      pullRequestRemoteIdentity.value = null;
      pullRequestSelectionGeneration.value = null;
      message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
      return;
    }
    if (!snapshot.value?.status.branch) {
      message.warning(t("workspace.controller.switchToALocalBranchBeforeCreatingAPullRequest"));
      return;
    }
    if (!pullRequestForm.title.trim() || !pullRequestForm.base.trim()) {
      message.warning(t("workspace.controller.enterAPullRequestTitleAndChooseABaseBranch"));
      return;
    }
    const headBranch = snapshot.value.status.branch;
    const head = pullRequestHead.value?.trim() || headBranch;
    if (!pullRequestHead.value?.trim() && pullRequestForm.base.trim() === headBranch) {
      message.warning(t("workspace.controller.theHeadBranchAndBaseBranchCannotBeTheSame"));
      return;
    }
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const pullRequest = await api.createPullRequest(repositoryPath, remoteName, {
        title: pullRequestForm.title.trim(),
        body: pullRequestForm.body,
        head,
        base: pullRequestForm.base.trim(),
        draft: pullRequestForm.draft,
      });
      if (
        !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        || getRemoteIdentity(activeRemote.value) !== remoteIdentity
      ) {
        pullRequestModalShown.value = false;
        pullRequestHead.value = null;
        pullRequestRepositoryPath.value = null;
        pullRequestRemoteName.value = null;
        pullRequestRemoteIdentity.value = null;
        pullRequestSelectionGeneration.value = null;
        return;
      }
      pullRequestsRequestGeneration += 1;
      pullRequestsLoading.value = false;
      pullRequests.value = [pullRequest, ...pullRequests.value.filter((item) => item.number !== pullRequest.number)];
      pullRequestModalShown.value = false;
      pullRequestHead.value = null;
      pullRequestRepositoryPath.value = null;
      pullRequestRemoteName.value = null;
      pullRequestRemoteIdentity.value = null;
      pullRequestSelectionGeneration.value = null;
      message.success(t("workspace.controller.pullRequestNumberCreated", { number: pullRequest.number }));
    } catch (error) {
      if (
        repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
        && getRemoteIdentity(activeRemote.value) === remoteIdentity
      ) reportError(error, t("workspace.controller.couldNotCreatePullRequest"));
    } finally {
      finishMutation();
    }
  }

  function fallbackPublishOwner(): GitHubPublishOwner | null {
    const account = publishAccounts.value.find((item) => item.id === publishForm.accountId);
    return account ? {
      login: account.login,
      name: account.displayName,
      kind: "user",
      canCreateRepositories: true,
    } : null;
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
    const repositoryPath = activeRepositoryPath.value;
    if (!repositoryPath) return;
    publishRepositoryPath.value = repositoryPath;
    publishSelectionGeneration.value = getSelectionGeneration();
    publishForm.endpointId = publishEndpoints.value[0]?.id || "github.com";
    publishForm.accountId = publishAccounts.value[0]?.id || "";
    publishForm.ownerLogin = publishAccounts.value[0]?.login || "";
    publishForm.name = snapshot.value?.repository.name || "";
    publishForm.isPrivate = true;
    publishModalShown.value = true;
  }

  async function publishGitHubRepository(): Promise<void> {
    const repositoryPath = publishRepositoryPath.value;
    const selectionGeneration = publishSelectionGeneration.value;
    if (
      !repositoryPath
      || selectionGeneration === null
      || !repositorySelectionIsCurrent(repositoryPath, selectionGeneration)
    ) {
      publishModalShown.value = false;
      publishRepositoryPath.value = null;
      publishSelectionGeneration.value = null;
      message.warning(t("workspace.controller.theActiveRepositoryChangedPleaseTryAgain"));
      return;
    }
    if (
      !publishForm.endpointId
      || !publishForm.accountId
      || !canPublishToSelectedOwner.value
      || !publishForm.name.trim()
    ) {
      message.warning(t("workspace.controller.selectAnEligibleOwnerAndEnterAGithubRepositoryName"));
      return;
    }
    if (publishForm.name.trim().includes("/")) {
      message.warning(t("workspace.controller.enterOnlyTheRepositoryNameChooseItsOwnerAbove"));
      return;
    }
    const finishMutation = beginRepositoryMutation(repositoryPath);
    if (!finishMutation) return;
    try {
      const nextSnapshot = await api.publishGitHubRepository(
        repositoryPath,
        publishForm.endpointId,
        publishForm.accountId,
        publishForm.ownerLogin,
        publishForm.name.trim(),
        publishForm.isPrivate,
      );
      if (!repositorySelectionIsCurrent(repositoryPath, selectionGeneration) || !applySnapshot(nextSnapshot)) {
        publishModalShown.value = false;
        publishRepositoryPath.value = null;
        publishSelectionGeneration.value = null;
        return;
      }
      publishModalShown.value = false;
      publishRepositoryPath.value = null;
      publishSelectionGeneration.value = null;
      message.success(t("workspace.controller.repositoryPublishedToGithubAndOriginConfigured"));
      await loadCapabilities(repositoryPath, selectionGeneration).catch(() => undefined);
    } catch (error) {
      if (repositorySelectionIsCurrent(repositoryPath, selectionGeneration)) {
        reportError(error, t("workspace.controller.couldNotPublishGithubRepository"));
      }
    } finally {
      finishMutation();
    }
  }

  function invalidatePublishOwnerRequests(): void {
    publishOwnersRequestGeneration += 1;
    publishOwnersLoading.value = false;
  }

  return {
    repositoryInfo,
    pullRequests,
    platformLoading,
    pullRequestsLoading,
    publishOwnersLoading,
    pullRequestModalShown,
    publishModalShown,
    forkModalShown,
    pullRequestForm,
    pullRequestHead,
    forkForm,
    forkContributionBranchRecommended,
    publishForm,
    publishOwners,
    publishOwnersError,
    pullRequestBaseOptions,
    publishEndpoints,
    publishAccounts,
    publishAccountOptions,
    forkAccounts,
    forkAccountOptions,
    publishOwnerOptions,
    canPublishToSelectedOwner,
    loadRepositoryInfo,
    loadPullRequests,
    invalidatePlatformRequests,
    openCreatePullRequest,
    createPullRequest,
    openForkModal,
    forkGithubRepository,
    loadPublishOwners,
    openPublishModal,
    publishGitHubRepository,
    invalidatePublishOwnerRequests,
  };
}
