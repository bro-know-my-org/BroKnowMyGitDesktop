<script lang="ts">
import { defineComponent } from "vue";
import { NButton, NCard, NDataTable, NDescriptions, NDescriptionsItem, NGrid, NGridItem, NSpace, NTag, NText } from "naive-ui";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";

export default defineComponent({
  name: "RepositoryOverviewPanel",
  components: { NButton, NCard, NDataTable, NDescriptions, NDescriptionsItem, NGrid, NGridItem, NSpace, NTag, NText },
  setup() {
    return useWorkspaceControllerContext();
  },
});
</script>

<template>
  <template v-if="snapshot">
<n-grid :cols="3" x-gap="16" y-gap="16" responsive="screen" item-responsive>
                  <n-grid-item span="3 m:1">
                    <n-card size="small" :title="tr('改动', 'Changes')">
                      <div class="metric"><strong>{{ snapshot.status.entries.length }}</strong><span>{{ tr("{count} 个文件有改动", "{count} changed files", { count: snapshot.status.entries.length }) }}</span></div>
                      <n-space size="small"><n-tag size="small" type="warning">{{ tr("{count} 未暂存", "{count} unstaged", { count: unstagedCount }) }}</n-tag><n-tag size="small" type="success">{{ tr("{count} 已暂存", "{count} staged", { count: stagedCount }) }}</n-tag></n-space>
                      <template #action><n-button text type="primary" @click="activeTab = 'changes'">{{ tr("查看改动", "View changes") }}</n-button></template>
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="3 m:1">
                    <n-card size="small" :title="tr('分支', 'Branches')">
                      <div class="metric"><strong>{{ snapshot.branches.length }}</strong><span>{{ tr("{count} 个本地分支", "{count} local branches", { count: snapshot.branches.length }) }}</span></div>
                      <n-text depth="3">{{ tr("当前：{branch}", "Current: {branch}", { branch: currentBranch }) }}</n-text>
                      <template #action><n-button text type="primary" @click="activeTab = 'branches'">{{ tr("管理分支", "Manage branches") }}</n-button></template>
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="3 m:1">
                    <n-card size="small" :title="tr('远程仓库', 'Remotes')">
                      <div class="metric"><strong>{{ snapshot.remotes.length }}</strong><span>{{ tr("{count} 个 remote", "{count} remotes", { count: snapshot.remotes.length }) }}</span></div>
                      <n-text depth="3">{{ tr("标签：{count}", "Tags: {count}", { count: snapshot.tags.length }) }}</n-text>
                      <template #action><n-button text type="primary" @click="activeTab = 'branches'">{{ tr("查看 remote", "View remotes") }}</n-button></template>
                    </n-card>
                  </n-grid-item>
                </n-grid>

                <n-card v-if="activeCapabilities.repositoryInfo || activeCapabilities.pullRequestsRead || activeCapabilities.pullRequestsWrite" size="small" :title="tr('代码托管平台', 'Git service')" class="platform-card">
                  <n-space>
                    <n-button v-if="activeCapabilities.repositoryInfo" :loading="platformLoading" @click="loadRepositoryInfo">{{ tr("读取仓库信息", "Load repository information") }}</n-button>
                    <n-button v-if="activeCapabilities.pullRequestsRead" :loading="pullRequestsLoading" @click="loadPullRequests">{{ tr("读取 Pull Requests", "Load pull requests") }}</n-button>
                    <n-button v-if="activeCapabilities.pullRequestsWrite" type="primary" @click="openCreatePullRequest">{{ tr("创建 Pull Request", "Create pull request") }}</n-button>
                  </n-space>
                  <n-descriptions v-if="repositoryInfo" :column="2" size="small" class="repository-info">
                    <n-descriptions-item :label="tr('仓库', 'Repository')">{{ repositoryInfo.fullName }}</n-descriptions-item>
                    <n-descriptions-item :label="tr('默认分支', 'Default branch')">{{ repositoryInfo.defaultBranch || '—' }}</n-descriptions-item>
                    <n-descriptions-item :label="tr('可见性', 'Visibility')">{{ repositoryInfo.isPrivate ? tr('私有', 'Private') : tr('公开', 'Public') }}</n-descriptions-item>
                    <n-descriptions-item :label="tr('说明', 'Description')">{{ repositoryInfo.description || '—' }}</n-descriptions-item>
                  </n-descriptions>
                  <n-data-table v-if="pullRequests.length" :columns="pullRequestColumns" :data="pullRequests" size="small" :bordered="false" class="pull-request-table" />
                </n-card>
  </template>
</template>
