import { computed, reactive, ref, type Ref } from "vue";
import type { DialogApi, MessageApi } from "naive-ui";
import { api, builtinEndpoints } from "../../../shared/api/tauri";
import { t } from "../../../shared/i18n";
import type {
  AccountProfile,
  AuthenticationMethod,
  CodeHostCapabilities,
  CodeHostEndpoint,
} from "../../../shared/types/domain";

interface WorkspaceAccountsDependencies {
  mutationBusy: Ref<boolean>;
  capabilityForEndpoint: (endpointId: string | null) => CodeHostCapabilities;
  reportError: (error: unknown, action: string) => void;
  message: MessageApi;
  dialog: DialogApi;
}

export function useWorkspaceAccounts({
  mutationBusy,
  capabilityForEndpoint,
  reportError,
  message,
  dialog,
}: WorkspaceAccountsDependencies) {
  const accounts = ref<AccountProfile[]>([]);
  const endpoints = ref<CodeHostEndpoint[]>([...builtinEndpoints]);
  const accountsLoading = ref(false);
  const accountStatusRefreshing = ref(false);
  const cliAccountDetections = new Map<string, Promise<AccountProfile[]>>();
  const pendingAccountRemovals = new Set<Set<string>>();
  const accountModalShown = ref(false);
  const customEndpointModalShown = ref(false);
  const accountForm = reactive({
    endpointId: "gitee.com",
    login: "",
    displayName: "",
    authMethod: "personalAccessToken" as AuthenticationMethod,
    token: "",
  });
  const endpointForm = reactive({
    id: "",
    kind: "genericGit",
    displayName: "",
    webUrl: "",
    apiUrl: "",
    gitHost: "",
  });
  let lastAccountStatusRefreshAt = 0;
  const accountStatusRefreshCacheMs = 60_000;

  const accountFormCapabilities = computed(() => capabilityForEndpoint(accountForm.endpointId));
  const accountUsesPat = computed(() => accountForm.authMethod === "personalAccessToken");
  const accountMethodOptions = computed(() => [
    {
      label: t("workspace.controller.personalAccessTokenPat"),
      value: "personalAccessToken",
      disabled: !accountFormCapabilities.value.patKeychain,
    },
    { label: t("common.authentication.method.sshAgent"), value: "sshAgent" },
  ]);

  async function updateAccountsFrom(load: () => Promise<AccountProfile[]>): Promise<void> {
    const before = new Map(accounts.value.map((account) => [account.id, account]));
    const removed = new Set<string>();
    pendingAccountRemovals.add(removed);
    try {
      const loaded = await load();
      const current = new Map(accounts.value.map((account) => [account.id, account]));
      // Preserve profiles changed locally while this older backend snapshot was in flight.
      const merged = new Map(loaded.filter((account) => !removed.has(account.id) && (!before.has(account.id) || current.has(account.id)))
        .map((account) => [account.id, account]));
      for (const account of current.values()) {
        if (account !== before.get(account.id)) merged.set(account.id, account);
      }
      accounts.value = [...merged.values()];
    } finally {
      pendingAccountRemovals.delete(removed);
    }
  }

  async function loadAccounts(): Promise<void> {
    accountsLoading.value = true;
    try {
      await updateAccountsFrom(() => api.listAccounts());
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotLoadAccounts"));
    } finally {
      accountsLoading.value = false;
    }
  }

  async function refreshAccountStatuses(force = false): Promise<void> {
    if (accountStatusRefreshing.value) return;
    if (!force && Date.now() - lastAccountStatusRefreshAt < accountStatusRefreshCacheMs) return;
    accountStatusRefreshing.value = true;
    try {
      await updateAccountsFrom(() => api.refreshAccountStatuses());
      lastAccountStatusRefreshAt = Date.now();
    } catch (error) {
      if (force) reportError(error, t("workspace.controller.couldNotRefreshAccountAuthenticationStatus"));
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
      reportError(error, t("workspace.controller.couldNotLoadGitServiceEndpoints"));
    }
  }

  async function saveAccount(): Promise<void> {
    if (accountUsesPat.value && !accountFormCapabilities.value.patKeychain) {
      message.warning(t("workspace.controller.thisEndpointDoesNotSupportAnAppManagedPatChooseASupportedAuthenticationMethod"));
      return;
    }
    if (!accountForm.login.trim() || (accountUsesPat.value && !accountForm.token.trim())) {
      message.warning(accountUsesPat.value
        ? t("workspace.controller.enterAnAccountLoginAndPersonalAccessToken")
        : t("workspace.controller.enterTheAccountLogin"));
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
      message.success(accountUsesPat.value
        ? t("workspace.controller.accountMetadataWasSavedAndThePatWasSentToTheSystemKeychainItWillNotBeShownAgain")
        : t("workspace.controller.accountProfileSavedNetworkGitOperationsWillUseThisMethodAccordingToTheRemoteBinding"));
      void refreshAccountStatuses(true);
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotAddAccount"));
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
      !endpointForm.id.trim()
      || !endpointForm.displayName.trim()
      || !endpointForm.gitHost.trim()
      || (requiresPlatformUrls && (!endpointForm.webUrl.trim() || !endpointForm.apiUrl.trim()))
    ) {
      message.warning(t("workspace.controller.completeTheRequiredGitServiceEndpointDetails"));
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
      endpoints.value = index === -1
        ? [...endpoints.value, endpoint]
        : endpoints.value.map((item, itemIndex) => itemIndex === index ? endpoint : item);
      customEndpointModalShown.value = false;
      message.success(t("workspace.controller.gitServiceEndpointSavedYouCanNowSelectItForARemote"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotSaveGitServiceEndpoint"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function detectCliAccounts(endpointId: string): Promise<AccountProfile[]> {
    if (!capabilityForEndpoint(endpointId).accountImport) return Promise.resolve([]);
    const existing = cliAccountDetections.get(endpointId);
    if (existing) return existing;
    const request = api.importCliAccounts(endpointId).then((imported) => {
      accounts.value = [...new Map([...accounts.value, ...imported].map((account) => [account.id, account])).values()];
      return imported;
    }).finally(() => cliAccountDetections.delete(endpointId));
    cliAccountDetections.set(endpointId, request);
    return request;
  }

  async function importCliAccounts(endpointId: string): Promise<void> {
    if (!capabilityForEndpoint(endpointId).accountImport) return;
    mutationBusy.value = true;
    try {
      const imported = await detectCliAccounts(endpointId);
      await loadAccounts();
      message.success(imported.length
        ? t("workspace.controller.detectedCountCliAccountS", { count: imported.length })
        : t("workspace.controller.noCliAccountsAreAvailableToImport"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotDetectCliAccounts"));
    } finally {
      mutationBusy.value = false;
    }
  }

  async function setDefaultCandidate(account: AccountProfile): Promise<void> {
    mutationBusy.value = true;
    try {
      accounts.value = await api.setDefaultAccount(account.id, !account.isDefaultCandidate);
      message.success(account.isDefaultCandidate
        ? t("workspace.controller.defaultCandidateRemoved")
        : t("workspace.controller.setAsDefaultCandidateItDoesNotOverrideAnExplicitRemoteBinding"));
    } catch (error) {
      reportError(error, t("workspace.controller.couldNotUpdateDefaultCandidate"));
    } finally {
      mutationBusy.value = false;
    }
  }

  function confirmRemoveAccount(account: AccountProfile): void {
    dialog.warning({
      title: t("workspace.controller.removeAccount"),
      content: t(
        "workspace.controller.removeTheLocalProfileForAccountThisRemovesTheAppSKeychainReferenceButNeverChangesGlobalGitGhOrGl",
        { account: account.displayName || account.login },
      ),
      positiveText: t("workspace.controller.remove"),
      negativeText: t("workspace.controller.cancel"),
      onPositiveClick: async () => {
        mutationBusy.value = true;
        try {
          await api.removeAccount(account.id);
          for (const removed of pendingAccountRemovals) removed.add(account.id);
          accounts.value = accounts.value.filter((item) => item.id !== account.id);
          message.success(t("workspace.controller.accountRemoved"));
        } catch (error) {
          reportError(error, t("workspace.controller.couldNotRemoveAccount"));
        } finally {
          mutationBusy.value = false;
        }
      },
    });
  }

  return {
    accounts,
    endpoints,
    accountsLoading,
    accountStatusRefreshing,
    accountModalShown,
    customEndpointModalShown,
    accountForm,
    endpointForm,
    lastAccountStatusRefreshAt,
    accountStatusRefreshCacheMs,
    accountFormCapabilities,
    accountUsesPat,
    accountMethodOptions,
    loadAccounts,
    refreshAccountStatuses,
    loadAccountsThenRefreshStatuses,
    loadEndpoints,
    saveAccount,
    openCustomEndpointModal,
    saveCustomEndpoint,
    importCliAccounts,
    detectCliAccounts,
    setDefaultCandidate,
    confirmRemoveAccount,
  };
}
