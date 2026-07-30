<script lang="ts">
import { defineComponent } from "vue";
import { NButton, NCard, NDataTable, NDescriptions, NDescriptionsItem, NGrid, NGridItem, NSpace, NTag, NText } from "naive-ui";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";
import { t } from "../../../shared/i18n";

export default defineComponent({
  name: "RepositoryOverviewPanel",
  components: { NButton, NCard, NDataTable, NDescriptions, NDescriptionsItem, NGrid, NGridItem, NSpace, NTag, NText },
  setup() {
    return { ...useWorkspaceControllerContext(), t };
  },
});
</script>

<template>
  <template v-if="snapshot">
<n-grid :cols="3" x-gap="16" y-gap="16" responsive="screen" item-responsive>
                  <n-grid-item span="3 m:1">
                    <n-card size="small" :title="t('repository.page.changes')">
                      <div class="metric"><strong>{{ snapshot.status.entries.length }}</strong><span>{{ t('repository.overview.countChangedFiles', { count: snapshot.status.entries.length }) }}</span></div>
                      <n-space size="small"><n-tag size="small" type="warning">{{ t('repository.overview.countUnstaged', { count: unstagedCount }) }}</n-tag><n-tag size="small" type="success">{{ t('repository.overview.countStaged', { count: stagedCount }) }}</n-tag></n-space>
                      <template #action><n-button text type="primary" @click="activeTab = 'changes'">{{ t('repository.overview.viewChanges') }}</n-button></template>
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="3 m:1">
                    <n-card size="small" :title="t('repository.overview.branches')">
                      <div class="metric"><strong>{{ snapshot.branches.length }}</strong><span>{{ t('repository.overview.countLocalBranches', { count: snapshot.branches.length }) }}</span></div>
                      <n-text depth="3">{{ t('repository.overview.currentBranch', { branch: currentBranch }) }}</n-text>
                      <template #action><n-button text type="primary" @click="activeTab = 'branches'">{{ t('repository.overview.manageBranches') }}</n-button></template>
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="3 m:1">
                    <n-card size="small" :title="t('repository.overview.remotes')">
                      <div class="metric"><strong>{{ snapshot.remotes.length }}</strong><span>{{ t('repository.overview.countRemotes', { count: snapshot.remotes.length }) }}</span></div>
                      <n-text depth="3">{{ t('repository.overview.tagsCount', { count: snapshot.tags.length }) }}</n-text>
                      <template #action><n-button text type="primary" @click="activeTab = 'branches'">{{ t('repository.overview.viewRemotes') }}</n-button></template>
                    </n-card>
                  </n-grid-item>
                </n-grid>

                <n-card v-if="activeCapabilities.repositoryInfo || activeCapabilities.pullRequestsRead || activeCapabilities.pullRequestsWrite" size="small" :title="t('repository.overview.gitService')" class="platform-card">
                  <n-space>
                    <n-button v-if="activeCapabilities.repositoryInfo" :loading="platformLoading" @click="loadRepositoryInfo">{{ t('repository.overview.loadRepositoryInformation') }}</n-button>
                    <n-button v-if="activeCapabilities.pullRequestsRead" :loading="pullRequestsLoading" @click="loadPullRequests">{{ t('repository.overview.loadPullRequests') }}</n-button>
                    <n-button v-if="activeCapabilities.pullRequestsWrite" type="primary" @click="openCreatePullRequest">{{ t('repository.overview.createPullRequest') }}</n-button>
                  </n-space>
                  <n-descriptions v-if="repositoryInfo" :column="2" size="small" class="repository-info">
                    <n-descriptions-item :label="t('repository.overview.repository')">{{ repositoryInfo.fullName }}</n-descriptions-item>
                    <n-descriptions-item :label="t('repository.overview.defaultBranch')">{{ repositoryInfo.defaultBranch || '—' }}</n-descriptions-item>
                    <n-descriptions-item :label="t('repository.overview.visibility')">{{ repositoryInfo.isPrivate ? t('repository.overview.private') : t('repository.overview.public') }}</n-descriptions-item>
                    <n-descriptions-item :label="t('repository.overview.description')">{{ repositoryInfo.description || '—' }}</n-descriptions-item>
                  </n-descriptions>
                  <n-data-table v-if="pullRequests.length" :columns="pullRequestColumns" :data="pullRequests" size="small" :bordered="false" class="pull-request-table" />
                </n-card>
  </template>
</template>

<style src="./OverviewPanel.css"></style>
