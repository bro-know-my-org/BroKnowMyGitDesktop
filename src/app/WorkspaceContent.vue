<script lang="ts">
import { defineComponent } from "vue";
import { NAlert, NButton, NEmpty, NLayoutContent, NSpace, NSpin } from "naive-ui";
import AccountsPage from "../pages/accounts/index.vue";
import RepositoriesPage from "../pages/repositories/index.vue";
import { useWorkspaceControllerContext } from "../features/workspace/composables/workspaceContext";

export default defineComponent({
  name: "WorkspaceContent",
  components: { NAlert, NButton, NEmpty, NLayoutContent, NSpace, NSpin, AccountsPage, RepositoriesPage },
  setup() {
    return useWorkspaceControllerContext();
  },
});
</script>

<template>
<n-layout-content
  class="content-area"
  :class="{ 'is-fixed-repository-content': section === 'repositories' && (activeTab === 'changes' || activeTab === 'history') && !browserDemo && !lastError }"
>
  <main class="content-inner">
<n-alert v-if="browserDemo" type="info" class="demo-alert" :bordered="false">
            {{ t('workspace.content.browserDemoModeThisUsesInteractiveLocalMockDataTauriCallsRustCommandsDemoPatsAreNeverSavedOrShown') }}
          </n-alert>

          <n-alert v-if="lastError" type="error" closable class="error-alert" @close="clearLastError">
            <template #header>
              {{ lastErrorAction ? t('workspace.content.mostRecentOperationFailedAction', { action: lastErrorAction }) : t('workspace.content.mostRecentOperationFailed') }}
            </template>
            <strong>{{ lastError.message }}</strong>
            <div class="error-diagnostic-code">{{ t('workspace.content.errorCode') }}: <code>{{ lastError.code }}</code></div>
            <div v-if="lastError.details" class="error-diagnostic-guidance">{{ lastError.details }}</div>
            <template v-if="lastError.code === 'account_selection_required'">{{ t('workspace.content.selectAnExplicitAccountInTheRemoteBindingThenRetry') }}</template>
          </n-alert>
    <AccountsPage v-if="section === 'accounts'" />
<n-spin v-else-if="!snapshot && (workspaceInitializing || snapshotLoading)" size="large" class="loading-state" />
<section v-else-if="!snapshot" class="repository-empty-state">
            <n-empty :description="t('workspace.content.selectOrAddALocalGitRepository')" :show-icon="false">
              <template #extra>
                <n-space justify="center">
                  <n-button @click="discoverModalShown = true">{{ t('workspace.content.scanFolder') }}</n-button>
                  <n-button @click="initModalShown = true">{{ t('workspace.content.newLocalRepository') }}</n-button>
                  <n-button type="primary" @click="cloneModalShown = true">{{ t('workspace.content.cloneRepository') }}</n-button>
                </n-space>
              </template>
            </n-empty>
          </section>
    <RepositoriesPage v-else-if="snapshot" />
  </main>
</n-layout-content>
</template>

<style src="./WorkspaceContent.css"></style>
