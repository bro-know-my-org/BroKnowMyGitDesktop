import { computed, reactive, ref, type ComputedRef, type Ref } from "vue";
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

interface RepositoryPlatformDependencies {
  activeRepositoryPath: Ref<string | null>;
  activeRemote: ComputedRef<RemoteProfile | null>;
  activeCapabilities: Ref<CodeHostCapabilities>;
  snapshot: Ref<RepositorySnapshot | null>;
  endpoints: Ref<CodeHostEndpoint[]>;
  accounts: Ref<AccountProfile[]>;
  selectedCommit: Ref<CommitSummary | null>;
  commits: Ref<CommitSummary[]>;
  mutationBusy: Ref<boolean>;
  capabilityForEndpoint: (endpointId: string | null) => CodeHostCapabilities;
  applySnapshot: (snapshot: RepositorySnapshot) => boolean;
  loadCapabilities: () => Promise<void>;
  reportError: (error: unknown, action: string) => void;
  message: MessageApi;
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
  mutationBusy,
  capabilityForEndpoint,
  applySnapshot,
  loadCapabilities,
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
  const pullRequestForm = reactive({ title: "", body: "", base: "", draft: false });
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
    if (!activeRepositoryPath.value || !activeRemote.value || !activeCapabilities.value.repositoryInfo) return;
    platformLoading.value = true;
    try {
      repositoryInfo.value = await api.getCodeHostRepositoryInfo(
        activeRepositoryPath.value,
        activeRemote.value.name,
      );
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
      message.success(t("workspace.controller.pullRequestNumberCreated", { number: pullRequest.number }));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotCreatePullRequest"));
    } finally {
      mutationBusy.value = false;
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
    publishForm.endpointId = publishEndpoints.value[0]?.id || "github.com";
    publishForm.accountId = publishAccounts.value[0]?.id || "";
    publishForm.ownerLogin = publishAccounts.value[0]?.login || "";
    publishForm.name = snapshot.value?.repository.name || "";
    publishForm.isPrivate = true;
    publishModalShown.value = true;
  }

  async function publishGitHubRepository(): Promise<void> {
    if (
      !activeRepositoryPath.value
      || !publishForm.endpointId
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
    mutationBusy.value = true;
    try {
      applySnapshot(await api.publishGitHubRepository(
        activeRepositoryPath.value,
        publishForm.endpointId,
        publishForm.accountId,
        publishForm.ownerLogin,
        publishForm.name.trim(),
        publishForm.isPrivate,
      ));
      publishModalShown.value = false;
      await loadCapabilities();
      message.success(t("workspace.controller.repositoryPublishedToGithubAndOriginConfigured"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotPublishGithubRepository"));
    } finally {
      mutationBusy.value = false;
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
    pullRequestForm,
    publishForm,
    publishOwners,
    publishOwnersError,
    pullRequestBaseOptions,
    publishEndpoints,
    publishAccounts,
    publishAccountOptions,
    publishOwnerOptions,
    canPublishToSelectedOwner,
    loadRepositoryInfo,
    loadPullRequests,
    openCreatePullRequest,
    createPullRequest,
    loadPublishOwners,
    openPublishModal,
    publishGitHubRepository,
    invalidatePublishOwnerRequests,
  };
}
