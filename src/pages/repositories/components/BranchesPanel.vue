<script lang="ts">
import { defineComponent } from "vue";
import { NAlert, NButton, NCard, NDataTable, NDescriptions, NDescriptionsItem, NDivider, NEmpty, NGrid, NGridItem, NSpace, NSpin, NTag, NText } from "naive-ui";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";

export default defineComponent({
  name: "RepositoryBranchesPanel",
  components: { NAlert, NButton, NCard, NDataTable, NDescriptions, NDescriptionsItem, NDivider, NEmpty, NGrid, NGridItem, NSpace, NSpin, NTag, NText },
  setup() {
    return useWorkspaceControllerContext();
  },
});
</script>

<template>
  <template v-if="snapshot">
<n-grid :cols="2" x-gap="16" y-gap="16" responsive="screen" item-responsive>
                  <n-grid-item span="2 m:1">
                    <n-card size="small" :title="tr('本地分支', 'Local branches')">
                      <template #header-extra>
                        <n-space size="small">
                          <n-button size="tiny" tertiary :disabled="!rebaseBaseOptions.length" @click="openRebaseModal">{{ tr("变基当前分支", "Rebase current") }}</n-button>
                          <n-button size="tiny" tertiary @click="openCherryPickModal()">{{ tr("拣选提交", "Cherry-pick") }}</n-button>
                          <n-button size="tiny" type="primary" @click="branchModalShown = true">{{ tr("新建分支", "New branch") }}</n-button>
                        </n-space>
                      </template>
                      <n-data-table :columns="branchColumns" :data="snapshot.branches" :bordered="false" />
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="2 m:1">
                    <n-card size="small" :title="tr('Remote 与账号绑定', 'Remotes & account bindings')">
                      <template #header-extra>
                        <n-space size="small">
                          <n-button size="tiny" tertiary @click="openRemoteEditor()">{{ tr("添加 remote", "Add remote") }}</n-button>
                          <n-button size="tiny" type="primary" @click="openTagModal()">{{ tr("新建标签", "New tag") }}</n-button>
                        </n-space>
                      </template>
                      <n-data-table :columns="remoteColumns" :data="snapshot.remotes" :bordered="false" />
                      <n-divider />
                      <div class="tags-row"><n-text depth="3">{{ tr("标签", "Tags") }}</n-text><n-tag v-for="tag in snapshot.tags" :key="tag.name" size="small" closable @close="confirmDeleteTag(tag.name)">{{ tag.name }}</n-tag><n-text v-if="!snapshot.tags.length" depth="3">{{ tr("暂无标签", "No tags") }}</n-text></div>
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="2">
                    <n-card size="small" :title="tr('远程分支', 'Remote branches')">
                      <template #header-extra>
                        <n-button size="tiny" tertiary :disabled="!activeRemote" :loading="networkBusy === 'fetch'" @click="runNetworkAction('fetch')">
                          {{ activeRemote ? tr('获取 {remote}', 'Fetch {remote}', { remote: activeRemote.name }) : tr('获取更新', 'Fetch') }}
                        </n-button>
                      </template>
                      <n-data-table :columns="remoteBranchColumns" :data="snapshot.remoteBranches" :bordered="false" />
                      <n-empty
                        v-if="!snapshot.remoteBranches.length"
                        :description="activeRemote ? tr('尚未获取到远程分支。点击右上角按钮获取更新。', 'No remote branches have been fetched yet. Use the button above to fetch.') : tr('请先添加并选择一个 remote。', 'Add and select a remote first.')"
                      />
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="2">
                    <n-card size="small" :title="tr('工作树', 'Worktrees')">
                      <template #header-extra><n-button size="tiny" type="primary" @click="openWorktreeModal">{{ tr("新建工作树", "New worktree") }}</n-button></template>
                      <n-data-table :columns="worktreeColumns" :data="worktrees" :loading="worktreesLoading" :bordered="false" />
                      <n-empty v-if="!worktreesLoading && !worktrees.length" :description="tr('没有可用工作树。', 'No worktrees found.')" />
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="2">
                    <n-card size="small" :title="tr('子模块', 'Submodules')">
                      <template #header-extra><n-button size="tiny" tertiary :loading="mutationBusy" @click="confirmUpdateSubmodules">{{ tr("初始化 / 更新全部", "Initialize / update all") }}</n-button></template>
                      <n-data-table :columns="submoduleColumns" :data="submodules" :loading="submodulesLoading" :bordered="false" />
                      <n-empty v-if="!submodulesLoading && !submodules.length" :description="tr('此仓库没有子模块。', 'This repository has no submodules.')" />
                    </n-card>
                  </n-grid-item>
                  <n-grid-item span="2">
                    <n-card size="small" :title="tr('Git LFS', 'Git LFS')">
                      <template #header-extra>
                        <n-space size="small">
                          <n-button size="tiny" tertiary :loading="mutationBusy" :disabled="!lfsStatus?.available || lfsStatus.initialized" @click="initializeLfs">{{ tr("在此仓库初始化", "Initialize here") }}</n-button>
                          <n-button size="tiny" type="primary" :disabled="!lfsStatus?.available" @click="openLfsTrackModal">{{ tr("添加跟踪规则", "Add tracking rule") }}</n-button>
                        </n-space>
                      </template>
                      <n-spin :show="lfsLoading">
                        <n-alert v-if="lfsStatus && !lfsStatus.available" type="warning" :bordered="false">{{ tr("未检测到 Git LFS。安装 Git LFS 后刷新仓库即可使用大文件跟踪。", "Git LFS was not detected. Install Git LFS, then refresh this repository to use large-file tracking.") }}</n-alert>
                        <template v-else-if="lfsStatus">
                          <n-descriptions :column="3" size="small">
                            <n-descriptions-item :label="tr('状态', 'Status')"><n-tag size="small" :type="lfsStatus.initialized ? 'success' : 'warning'">{{ lfsStatus.initialized ? tr("已初始化", "Initialized") : tr("未初始化", "Not initialized") }}</n-tag></n-descriptions-item>
                            <n-descriptions-item :label="tr('版本', 'Version')">{{ lfsStatus.version || '—' }}</n-descriptions-item>
                            <n-descriptions-item :label="tr('LFS 文件', 'LFS files')">{{ lfsStatus.files.length }}</n-descriptions-item>
                          </n-descriptions>
                          <n-space v-if="lfsStatus.files.length" size="small" wrap class="lfs-file-list"><n-tag v-for="file in lfsStatus.files" :key="file" size="small">{{ file }}</n-tag></n-space>
                          <n-text v-else depth="3">{{ tr("尚未发现由 Git LFS 管理的文件。", "No Git LFS-managed files found yet.") }}</n-text>
                        </template>
                      </n-spin>
                    </n-card>
                  </n-grid-item>
                </n-grid>
  </template>
</template>
