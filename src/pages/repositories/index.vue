<script lang="ts">
import { defineComponent } from "vue";
import { NAlert, NButton, NCard, NGrid, NGridItem, NScrollbar, NSpace, NSpin, NTabPane, NTabs, NTag, NText } from "naive-ui";
import RepositoryBranchesPanel from "./components/BranchesPanel.vue";
import RepositoryChangesPanel from "./components/ChangesPanel.vue";
import RepositoryHistoryPanel from "./components/HistoryPanel.vue";
import RepositoryOverviewPanel from "./components/OverviewPanel.vue";
import { useWorkspaceControllerContext } from "../../features/workspace/composables/workspaceContext";
import { t } from "../../shared/i18n";

export default defineComponent({
  name: "RepositoriesPage",
  components: { NAlert, NButton, NCard, NGrid, NGridItem, NScrollbar, NSpace, NSpin, NTabPane, NTabs, NTag, NText, RepositoryChangesPanel, RepositoryHistoryPanel, RepositoryBranchesPanel, RepositoryOverviewPanel },
  setup() {
    return { ...useWorkspaceControllerContext(), t };
  },
});
</script>

<template>
<section v-if="snapshot" class="repository-page">
            <n-scrollbar v-if="snapshot.operation || conflictedChanges.length" class="operation-alert">
            <n-alert type="warning" :bordered="false">
              <template #header>
                {{ snapshot.operation ? t('repository.page.gitOperationNeedsAttention') : t('repository.page.unresolvedConflictsDetected') }}
              </template>
              <template v-if="snapshot.operation">
                {{ t('repository.page.operationCurrentlyInProgress', { operation: gitOperationLabel(snapshot.operation.kind) }) }}
              </template>
              <template v-if="conflictedChanges.length">
                {{ t('repository.page.resolveConflictsHelp', { count: conflictedChanges.length }) }}
              </template>
              <n-space v-if="snapshot.operation || conflictedChanges.length" size="small" class="operation-actions">
                <n-button v-if="conflictedChanges.length" size="tiny" type="primary" :loading="mutationBusy" @click="openConflictedFilesInExternalEditor">{{ externalEditor ? t('repository.page.resolveExternalEditor') : t('repository.page.configureExternalEditor') }}</n-button>
                <n-button v-if="conflictedChanges.length && externalEditor" size="tiny" tertiary :disabled="mutationBusy" @click="openExternalEditorModal">{{ t('repository.page.editorSettings') }}</n-button>
                <n-button v-if="snapshot.operation" size="tiny" :loading="mutationBusy" :disabled="snapshot.operation.hasConflicts || conflictedChanges.length > 0" @click="continueGitOperation">{{ t('repository.page.continue') }}</n-button>
                <n-button v-if="snapshot.operation" size="tiny" tertiary type="error" :loading="mutationBusy" @click="confirmAbortGitOperation">{{ t('repository.page.abort') }}</n-button>
              </n-space>
            </n-alert>
            </n-scrollbar>

            <n-tabs v-model:value="activeTab" type="line" :tabs-padding="12" :pane-style="{ padding: '0' }" class="repository-tabs">
              <n-tab-pane name="changes" :tab-props="{ onPointerdown: (event: PointerEvent) => activateRepositoryTabOnPointerDown('changes', event) }" display-directive="show:lazy">
                <template #tab>
                  <span class="repository-tab-label">{{ t('repository.page.changes') }}</span>
                  <span v-if="snapshot.status.entries.length" class="repository-tab-count">{{ snapshot.status.entries.length }}</span>
                </template>
                <RepositoryChangesPanel />
              </n-tab-pane>
              <n-tab-pane name="history" :tab="t('repository.page.history')" :tab-props="{ onPointerdown: (event: PointerEvent) => activateRepositoryTabOnPointerDown('history', event) }" display-directive="show:lazy">
                <RepositoryHistoryPanel v-if="repositoryTabContentReady.history" />
                <div v-else class="repository-tab-loading"><n-spin size="medium" /></div>
              </n-tab-pane>
              <n-tab-pane name="branches" :tab="t('repository.page.branchesRemotes')" :tab-props="{ onPointerdown: (event: PointerEvent) => activateRepositoryTabOnPointerDown('branches', event) }" display-directive="show:lazy">
                <div v-if="repositoryTabContentReady.branches" class="repository-tab-page">
                <n-card v-if="activeRemote" size="small" class="remote-summary-card">
                  <n-grid :cols="4" x-gap="18" y-gap="12" responsive="screen" item-responsive>
                    <n-grid-item span="4 m:1">
                      <div class="detail-label">{{ t('repository.page.remote') }}</div>
                      <strong>{{ activeRemote.name }}</strong>
                    </n-grid-item>
                    <n-grid-item span="4 m:1">
                      <div class="detail-label">{{ t('repository.page.gitService') }}</div>
                      <n-tag size="small" :bordered="false">{{ endpointKindLabel(activeRemote.codeHostKind) }}</n-tag>
                    </n-grid-item>
                    <n-grid-item span="4 m:1">
                      <div class="detail-label">{{ t('repository.page.accountSelection') }}</div>
                      <strong>{{ remoteBindingLabel(activeRemote) }}</strong>
                    </n-grid-item>
                    <n-grid-item span="4 m:1">
                      <div class="detail-label">{{ t('repository.page.authenticationStatus') }}</div>
                      <strong>{{ remoteAuthenticationLabel(activeRemote) }}</strong>
                    </n-grid-item>
                  </n-grid>
                  <template #action>
                    <n-space justify="space-between" align="center">
                      <n-text depth="3" class="remote-url">{{ activeRemote.fetchUrl }}</n-text>
                      <n-space size="small">
                        <n-button size="small" tertiary type="warning" :loading="networkBusy === 'push'" @click="confirmForcePush">{{ t('repository.page.forcePush') }}</n-button>
                        <n-button size="small" secondary @click="openBindingDrawer(activeRemote)">{{ t('repository.page.manageAccountBinding') }}</n-button>
                      </n-space>
                    </n-space>
                  </template>
                </n-card>
                <RepositoryBranchesPanel />
                </div>
                <div v-else class="repository-tab-loading"><n-spin size="medium" /></div>
              </n-tab-pane>
              <n-tab-pane name="overview" :tab="t('repository.page.details')" :tab-props="{ onPointerdown: (event: PointerEvent) => activateRepositoryTabOnPointerDown('overview', event) }" display-directive="show:lazy">
                <div v-if="repositoryTabContentReady.overview" class="repository-tab-page"><RepositoryOverviewPanel /></div>
                <div v-else class="repository-tab-loading"><n-spin size="medium" /></div>
              </n-tab-pane>
            </n-tabs>
          </section>
</template>

<style src="./index.css"></style>
