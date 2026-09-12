<script setup lang="ts">
import { computed, h, onBeforeUnmount, ref, toRef, watch } from "vue";
import { NAlert, NButton, NForm, NFormItem, NInput, NSelect, NSpace, NTabPane, NTabs, NText, type SelectRenderLabel } from "naive-ui";
import { selectPath } from "../../../shared/pathPicker";
import { t } from "../../../shared/i18n";
import { cloneDestination } from "../composables/cloneDestination";
import { useCloneRepositoryList } from "../composables/useCloneRepositoryList";
import { useWorkspaceControllerContext } from "../composables/workspaceContext";

const { accounts, endpoints, cloneForm, cloneAccountOptions, mutationBusy, reportError, detectCliAccounts, capabilityForEndpoint } = useWorkspaceControllerContext();
const githubEndpoints = computed(() => endpoints.value.filter((endpoint) => endpoint.kind === "gitHub" && capabilityForEndpoint(endpoint.id).accountImport));
const accountOptions = computed(() => accounts.value.filter((account) => account.authMethod === "ghCli"
  && (account.status === "valid" || account.status === "available")
  && githubEndpoints.value.some((endpoint) => endpoint.id === account.endpointId))
  .map((account) => ({ label: `${account.login} · ${account.endpointId}`, value: account.id })));
const accountId = ref<string | null>(null);
const mode = ref(cloneForm.url ? "url" : "account");
const detecting = ref(false);
const detectionFailed = ref(false);
let detectionGeneration = 0;
const selectedRepository = ref<string | null>(null);
const protocol = ref("https");
const { repositories, loading, failed, hasMore, loadRepositories } = useCloneRepositoryList(
  accountId, (error) => reportError(error, t("workspace.clone.loadFailed")),
);
const picking = ref(false);
const parent = toRef(cloneForm, "parentPath");
const repositoryDerivedRouting = toRef(cloneForm, "repositoryDerivedRouting");
let disposed = false;
onBeforeUnmount(() => { disposed = true; detectionGeneration += 1; });
const repositoryOptions = computed(() => repositories.value.map((repository) => ({
  label: repository.fullName,
  value: repository.fullName,
  isPrivate: repository.isPrivate,
})));
const repositoryEmptyStateKey = computed(() => {
  if (loading.value) return "workspace.clone.loading";
  if (failed.value) return "workspace.clone.loadFailed";
  return "workspace.clone.empty";
});
const renderRepositoryLabel: SelectRenderLabel = (option) => h("span", { class: "clone-repository-label" }, [
  option.isPrivate ? h("svg", { class: "clone-repository-lock", viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": 1.7, role: "img", "aria-label": t("workspace.clone.private") }, [
    h("title", t("workspace.clone.private")), h("rect", { x: 5, y: 10, width: 14, height: 11, rx: 2 }), h("path", { d: "M8 10V7a4 4 0 0 1 8 0v3m-4 5v2" }),
  ]) : null,
  h("span", { class: "clone-repository-name", title: String(option.label) }, String(option.label)),
]);

async function detectAccounts(): Promise<void> {
  const requestId = ++detectionGeneration;
  detecting.value = true;
  detectionFailed.value = false;
  const results = await Promise.allSettled(githubEndpoints.value.map((endpoint) => detectCliAccounts(endpoint.id)));
  if (disposed || requestId !== detectionGeneration) return;
  detecting.value = false;
  const detected = results.flatMap((result) => result.status === "fulfilled" ? result.value : []);
  detectionFailed.value = results.some((result) => result.status === "rejected");
  const availableIds = new Set(accountOptions.value.map((option) => option.value));
  if (!accountId.value || !availableIds.has(accountId.value)) {
    const detectedCandidates = detected.filter((account) => availableIds.has(account.id));
    const candidates = detectedCandidates.length ? detectedCandidates : accounts.value.filter((account) => availableIds.has(account.id));
    accountId.value = candidates.find((account) => account.isDefaultCandidate)?.id ?? candidates[0]?.id ?? null;
  } else {
    void loadRepositories(true);
  }
}
watch(() => githubEndpoints.value.map((endpoint) => endpoint.id).join("\n"), () => { void detectAccounts(); }, { immediate: true });

watch(accountId, () => {
  if (selectedRepository.value) cloneForm.url = "";
  selectedRepository.value = null;
});
watch(accountOptions, (options) => {
  if (accountId.value && !options.some((option) => option.value === accountId.value)) accountId.value = null;
});
watch([selectedRepository, protocol], ([slug], [previousSlug]) => {
  const account = accounts.value.find((item) => item.id === accountId.value);
  const endpoint = endpoints.value.find((item) => item.id === account?.endpointId);
  if (!endpoint || !slug || !/^[\w.-]+\/[\w.-]+$/u.test(slug)) return;
  cloneForm.url = protocol.value === "ssh" ? `ssh://git@${endpoint.gitHost}/${slug}.git` : `https://${endpoint.gitHost}/${slug}.git`;
  if (slug !== previousSlug) {
    cloneForm.endpointId = endpoint.id;
    cloneForm.accountId = "none";
    repositoryDerivedRouting.value = true;
  }
});
function editRemoteUrl(): void {
  selectedRepository.value = null;
  if (repositoryDerivedRouting.value) {
    cloneForm.endpointId = "";
    cloneForm.accountId = "none";
  }
  repositoryDerivedRouting.value = false;
}
watch(() => cloneForm.url, (url, previous) => {
  if (!cloneForm.destinationPath || cloneForm.destinationPath === cloneDestination(parent.value, previous)) {
    cloneForm.destinationPath = cloneDestination(parent.value, url);
  }
});
async function chooseParent(): Promise<void> {
  picking.value = true;
  try {
    const selected = await selectPath({ title: t("workspace.clone.chooseParent"), directory: true, initialPath: parent.value });
    if (!selected || disposed || mutationBusy.value) return;
    parent.value = selected;
    cloneForm.destinationPath = cloneDestination(selected, cloneForm.url);
  } catch (error) {
    if (!disposed) reportError(error, t("workspace.clone.chooseFolderFailed"));
  } finally { picking.value = false; }
}
</script>

<template>
  <n-form class="clone-repository-form clone-form-stack" label-placement="top" :show-feedback="false" :disabled="mutationBusy">
    <n-tabs v-model:value="mode" type="segment" :animated="false" :pane-style="{ padding: mode === 'account' ? '16px 0 0' : '0' }">
      <n-tab-pane name="account" :tab="t('workspace.clone.fromAccount')" :disabled="mutationBusy">
        <div class="clone-form-stack">
          <n-form-item :label="t('workspace.clone.account')">
            <n-select v-model:value="accountId" :options="accountOptions" :loading="detecting" :disabled="detecting || mutationBusy" :placeholder="t(detecting ? 'workspace.clone.detecting' : 'workspace.clone.selectAccount')" />
          </n-form-item>
          <n-space>
            <n-button size="small" :loading="detecting" :disabled="mutationBusy || detecting || loading" @click="detectAccounts">{{ t('workspace.clone.detectAgain') }}</n-button>
            <n-button size="small" :loading="loading" :disabled="mutationBusy || !accountId" @click="loadRepositories(true)">{{ t('workspace.clone.refresh') }}</n-button>
          </n-space>
          <n-text v-if="detectionFailed || (!detecting && !accountOptions.length)" depth="3">{{ t(detectionFailed ? 'workspace.controller.couldNotDetectCliAccounts' : 'workspace.clone.noGhAccount') }}</n-text>
          <n-form-item :label="t('workspace.clone.repository')">
            <n-select v-model:value="selectedRepository" filterable to="body" :options="repositoryOptions" :render-label="renderRepositoryLabel" :loading="loading" :disabled="!accountId || mutationBusy" :placeholder="t('workspace.clone.search')">
              <template #empty>{{ t(repositoryEmptyStateKey) }}</template>
            </n-select>
          </n-form-item>
          <n-space v-if="hasMore || failed"><n-button size="small" :loading="loading" :disabled="mutationBusy || loading" @click="loadRepositories()">{{ t(failed ? 'workspace.clone.retry' : 'workspace.clone.loadMore') }}</n-button></n-space>
          <n-form-item :label="t('workspace.clone.protocol')"><n-select v-model:value="protocol" :options="[{ label: 'HTTPS', value: 'https' }, { label: 'SSH', value: 'ssh' }]" /></n-form-item>
          <n-alert type="info" :bordered="false">{{ t('workspace.clone.credentialsHelp') }}</n-alert>
        </div>
      </n-tab-pane>
      <n-tab-pane name="url" :tab="t('workspace.dialogs.remoteUrl')" :disabled="mutationBusy" />
    </n-tabs>
    <n-form-item :label="t('workspace.dialogs.remoteUrl')"><n-input v-model:value="cloneForm.url" :placeholder="t('workspace.dialogs.httpsGitExampleComOrgRepositoryGitOrGitHostOrgRepositoryGit')" @update:value="editRemoteUrl" /></n-form-item>
    <n-form-item :label="t('workspace.dialogs.destinationFolder')">
      <n-space vertical style="width: 100%">
        <n-input v-model:value="cloneForm.destinationPath" :placeholder="t('workspace.dialogs.placeholderCloneDestination')" />
        <n-button :loading="picking" :disabled="mutationBusy || picking" @click="chooseParent">{{ t('workspace.clone.chooseParent') }}</n-button>
        <n-text depth="3">{{ t('workspace.clone.destinationHelp') }} {{ parent }}</n-text>
      </n-space>
    </n-form-item>
    <n-form-item :label="t('workspace.dialogs.gitServiceEndpointOptionalForPrivateRepositories')"><n-select v-model:value="cloneForm.endpointId" clearable :options="endpoints.map((endpoint) => ({ label: endpoint.displayName, value: endpoint.id }))" :placeholder="t('workspace.dialogs.leaveEmptyToUseNormalSystemGitBehavior')" @update:value="repositoryDerivedRouting = false" /></n-form-item>
    <n-form-item :label="t('workspace.dialogs.accountUsedForClone')"><n-select v-model:value="cloneForm.accountId" :options="cloneAccountOptions" @update:value="repositoryDerivedRouting = false" /></n-form-item>
  </n-form>
</template>

<style>
.clone-form-stack {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 16px;
}

.clone-repository-form {
  margin-bottom: 16px;
}

.clone-repository-label {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
}

.clone-repository-lock {
  width: 16px;
  height: 16px;
  flex: none;
}

.clone-repository-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
