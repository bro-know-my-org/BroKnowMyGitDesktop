<script lang="ts">
import { defineComponent } from "vue";
import { NAlert, NButton, NCard, NDataTable, NDescriptions, NDescriptionsItem, NDivider, NEmpty, NGrid, NGridItem, NSpace, NSpin, NTag, NText } from "naive-ui";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";
import { t } from "../../../shared/i18n";

export default defineComponent({
  name: "RepositoryBranchesPanel",
  components: { NAlert, NButton, NCard, NDataTable, NDescriptions, NDescriptionsItem, NDivider, NEmpty, NGrid, NGridItem, NSpace, NSpin, NTag, NText },
  setup() {
    return { ...useWorkspaceControllerContext(), t };
  },
});
</script>

<template>
  <template v-if="snapshot">
<n-grid :cols="2" x-gap="16" y-gap="16" responsive="screen" item-responsive>
                  <n-grid-item span="2 m:1">
                    <n-card size="small" :title="t('repository.branches.localBranches')">
                      <template #header-extra>
                        <n-space size="small">
                          <n-button size="tiny" tertiary :disabled="!rebaseBaseOptions.length" @click="openRebaseModal">{{ t('repository.branches.rebaseCurrent') }}</n-button>
                          <n-button size="tiny" tertiary @click="openCherryPickModal()">{{ t('repository.branches.cherryPick') }}</n-button>
                          <n-button size="tiny" type="primary" @click="branchModalShown = true">{{ t('repository.branches.newBranch') }}</n-button>
                        </n-space>
                      </template>
                      <n-data-table :columns="branchColumns" :data="snapshot.branches" :bordered="false" table-layout="fixed" />
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="2 m:1">
                    <n-card size="small" :title="t('repository.branches.remotesAccountBindings')">
                      <template #header-extra>
                        <n-space size="small">
                          <n-button size="tiny" tertiary @click="openRemoteEditor()">{{ t('repository.branches.addRemote') }}</n-button>
                          <n-button size="tiny" type="primary" @click="openTagModal()">{{ t('repository.branches.newTag') }}</n-button>
                        </n-space>
                      </template>
                      <n-data-table :columns="remoteColumns" :data="snapshot.remotes" :bordered="false" />
                      <n-divider />
                      <div class="tags-row"><n-text depth="3">{{ t('repository.branches.tags') }}</n-text><n-tag v-for="tag in snapshot.tags" :key="tag.name" size="small" closable @close="confirmDeleteTag(tag.name)">{{ tag.name }}</n-tag><n-text v-if="!snapshot.tags.length" depth="3">{{ t('repository.branches.noTags') }}</n-text></div>
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="2">
                    <n-card size="small" :title="t('repository.branches.remoteBranches')">
                      <template #header-extra>
                        <n-button size="tiny" tertiary :disabled="!activeRemote" :loading="networkBusy === 'fetch'" @click="runNetworkAction('fetch')">
                          {{ activeRemote ? t('repository.branches.fetchRemote', { remote: activeRemote.name }) : t('repository.branches.fetch') }}
                        </n-button>
                      </template>
                      <n-data-table :columns="remoteBranchColumns" :data="snapshot.remoteBranches" :bordered="false" table-layout="fixed" />
                      <n-empty
                        v-if="!snapshot.remoteBranches.length"
                        :description="activeRemote ? t('repository.branches.noRemoteBranches') : t('repository.branches.addSelectRemoteFirst')"
                        :show-icon="false"
                      />
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="2">
                    <n-card size="small" :title="t('repository.branches.worktrees')">
                      <template #header-extra><n-button size="tiny" type="primary" @click="openWorktreeModal">{{ t('repository.branches.newWorktree') }}</n-button></template>
                      <n-data-table :columns="worktreeColumns" :data="worktrees" :loading="worktreesLoading" :bordered="false" />
                      <n-empty v-if="!worktreesLoading && !worktrees.length" :description="t('repository.branches.noWorktreesFound')" :show-icon="false" />
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="2">
                    <n-card size="small" :title="t('repository.branches.submodules')">
                      <template #header-extra><n-button size="tiny" tertiary :loading="mutationBusy" @click="confirmUpdateSubmodules">{{ t('repository.branches.initializeUpdateAll') }}</n-button></template>
                      <n-data-table :columns="submoduleColumns" :data="submodules" :loading="submodulesLoading" :bordered="false" />
                      <n-empty v-if="!submodulesLoading && !submodules.length" :description="t('repository.branches.repositoryHasNoSubmodules')" :show-icon="false" />
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="2">
                    <n-card size="small" :title="t('repository.branches.lfs')">
                      <template #header-extra>
                        <n-space size="small">
                          <n-button size="tiny" tertiary :loading="mutationBusy" :disabled="!lfsStatus?.available || lfsStatus.initialized" @click="initializeLfs">{{ t('repository.branches.initializeHere') }}</n-button>
                          <n-button size="tiny" type="primary" :disabled="!lfsStatus?.available" @click="openLfsTrackModal">{{ t('repository.branches.addTrackingRule') }}</n-button>
                        </n-space>
                      </template>
                      <n-spin :show="lfsLoading">
                        <n-alert v-if="lfsStatus && !lfsStatus.available" type="warning" :bordered="false">{{ t('repository.branches.lfsUnavailableHelp') }}</n-alert>
                        <template v-else-if="lfsStatus">
                          <n-descriptions :column="3" size="small">
                            <n-descriptions-item :label="t('repository.branches.status')"><n-tag size="small" :type="lfsStatus.initialized ? 'success' : 'warning'">{{ lfsStatus.initialized ? t('repository.branches.initialized') : t('repository.branches.notInitialized') }}</n-tag></n-descriptions-item>
                            <n-descriptions-item :label="t('repository.branches.version')">{{ lfsStatus.version || '—' }}</n-descriptions-item>
                            <n-descriptions-item :label="t('repository.branches.lfsFiles')">{{ lfsStatus.files.length }}</n-descriptions-item>
                          </n-descriptions>
                          <n-space v-if="lfsStatus.files.length" size="small" wrap class="lfs-file-list"><n-tag v-for="file in lfsStatus.files" :key="file" size="small">{{ file }}</n-tag></n-space>
                          <n-text v-else depth="3">{{ t('repository.branches.noLfsManagedFilesFoundYet') }}</n-text>
                        </template>
                      </n-spin>
                    </n-card>
                  </n-grid-item>
                </n-grid>
  </template>
</template>

<style src="./BranchesPanel.css"></style>
