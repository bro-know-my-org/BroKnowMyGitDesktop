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
  :native-scrollbar="false"
  content-class="content-scroll-body"
  :class="{
    'is-repository-panel-content': section === 'repositories' && (activeTab === 'changes' || activeTab === 'history'),
    'is-fixed-repository-content': section === 'repositories' && (activeTab === 'changes' || activeTab === 'history') && !browserDemo,
  }"
>
  <main class="content-inner">
<n-alert v-if="browserDemo" type="info" class="demo-alert" :bordered="false">
            {{ t('workspace.content.browserDemoModeThisUsesInteractiveLocalMockDataTauriCallsRustCommandsDemoPatsAreNeverSavedOrShown') }}
          </n-alert>

    <AccountsPage v-if="section === 'accounts'" />
<n-spin v-else-if="!snapshot && (workspaceInitializing || snapshotLoading)" size="large" class="loading-state" />
<section v-else-if="!snapshot" class="repository-empty-state">
            <n-empty :description="t('workspace.content.selectOrAddALocalGitRepository')" :show-icon="false">
              <template #extra>
                <n-space justify="center">
                  <n-button @click="openRepositoryDiscovery">{{ t('workspace.content.scanFolder') }}</n-button>
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
