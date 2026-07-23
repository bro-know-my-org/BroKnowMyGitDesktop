<script lang="ts">
import { defineComponent } from "vue";
import { NAlert, NButton, NCard, NGrid, NGridItem, NSpace, NTabPane, NTabs, NTag, NText } from "naive-ui";
import RepositoryBranchesPanel from "./components/BranchesPanel.vue";
import RepositoryChangesPanel from "./components/ChangesPanel.vue";
import RepositoryHistoryPanel from "./components/HistoryPanel.vue";
import RepositoryOverviewPanel from "./components/OverviewPanel.vue";
import { useWorkspaceControllerContext } from "../../features/workspace/composables/workspaceContext";

export default defineComponent({
  name: "RepositoriesPage",
  components: { NAlert, NButton, NCard, NGrid, NGridItem, NSpace, NTabPane, NTabs, NTag, NText, RepositoryChangesPanel, RepositoryHistoryPanel, RepositoryBranchesPanel, RepositoryOverviewPanel },
  setup() {
    return useWorkspaceControllerContext();
  },
});
</script>

<template>
<section v-if="snapshot" class="repository-page">
            <n-card v-if="activeRemote && activeTab === 'branches'" size="small" class="remote-summary-card">
              <n-grid :cols="4" x-gap="18" y-gap="12" responsive="screen" item-responsive>
                <n-grid-item span="4 m:1">
                  <div class="detail-label">{{ tr("远程仓库", "Remote") }}</div>
                  <strong>{{ activeRemote.name }}</strong>
                </n-grid-item>
                <n-grid-item span="4 m:1">
                  <div class="detail-label">{{ tr("Git 服务", "Git service") }}</div>
                  <n-tag size="small" :bordered="false">{{ endpointKindLabel(activeRemote.codeHostKind, appLocale) }}</n-tag>
                </n-grid-item>
                <n-grid-item span="4 m:1">
                  <div class="detail-label">{{ tr("账号选择", "Account selection") }}</div>
                  <strong>{{ remoteBindingLabel(activeRemote) }}</strong>
                </n-grid-item>
                <n-grid-item span="4 m:1">
                  <div class="detail-label">{{ tr("认证状态", "Authentication status") }}</div>
                  <strong>{{ remoteAuthenticationLabel(activeRemote) }}</strong>
                </n-grid-item>
              </n-grid>
              <template #action>
                <n-space justify="space-between" align="center">
                  <n-text depth="3" class="remote-url">{{ activeRemote.fetchUrl }}</n-text>
                  <n-space size="small">
                    <n-button size="small" tertiary type="warning" :loading="networkBusy === 'push'" @click="confirmForcePush">{{ tr("强制推送…", "Force push…") }}</n-button>
                    <n-button size="small" secondary @click="openBindingDrawer(activeRemote)">{{ tr("管理账号绑定", "Manage account binding") }}</n-button>
                  </n-space>
                </n-space>
              </template>
            </n-card>

            <n-alert v-if="snapshot.operation || conflictedChanges.length" type="warning" class="operation-alert" :bordered="false">
              <template #header>
                {{ snapshot.operation ? tr("Git 操作需要处理", "Git operation needs attention") : tr("检测到未解决的冲突", "Unresolved conflicts detected") }}
              </template>
              <template v-if="snapshot.operation">
                {{ tr("当前正在进行 {operation}。", "A {operation} is currently in progress.", { operation: gitOperationLabel(snapshot.operation.kind) }) }}
              </template>
              <template v-if="conflictedChanges.length">
                {{ tr("请在外部编辑器中解决并暂存 {count} 个冲突文件，然后继续；也可中止操作。", "Resolve and stage {count} conflicted file(s) in an external editor, then continue; you can also abort the operation.", { count: conflictedChanges.length }) }}
              </template>
              <n-space v-if="snapshot.operation || conflictedChanges.length" size="small" class="operation-actions">
                <n-button v-if="conflictedChanges.length" size="tiny" type="primary" :loading="mutationBusy" @click="openConflictedFilesInExternalEditor">{{ externalEditor ? tr("在外部编辑器中解决", "Resolve in external editor") : tr("配置外部编辑器", "Configure external editor") }}</n-button>
                <n-button v-if="conflictedChanges.length && externalEditor" size="tiny" tertiary :disabled="mutationBusy" @click="openExternalEditorModal">{{ tr("编辑器设置", "Editor settings") }}</n-button>
                <n-button v-if="snapshot.operation" size="tiny" :loading="mutationBusy" :disabled="snapshot.operation.hasConflicts || conflictedChanges.length > 0" @click="continueGitOperation">{{ tr("继续", "Continue") }}</n-button>
                <n-button v-if="snapshot.operation" size="tiny" tertiary type="error" :loading="mutationBusy" @click="confirmAbortGitOperation">{{ tr("中止", "Abort") }}</n-button>
              </n-space>
            </n-alert>

            <n-tabs v-model:value="activeTab" type="line" animated class="repository-tabs">
              <n-tab-pane name="changes" :tab="tr('改动', 'Changes')"><RepositoryChangesPanel /></n-tab-pane>
              <n-tab-pane name="history" :tab="tr('历史', 'History')"><RepositoryHistoryPanel /></n-tab-pane>
              <n-tab-pane name="branches" :tab="tr('分支与远程', 'Branches & remotes')"><RepositoryBranchesPanel /></n-tab-pane>
              <n-tab-pane name="overview" :tab="tr('详情', 'Details')"><RepositoryOverviewPanel /></n-tab-pane>
            </n-tabs>
          </section>
</template>
