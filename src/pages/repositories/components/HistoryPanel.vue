<script lang="ts">
import { defineComponent } from "vue";
import { NButton, NCard, NDataTable, NEmpty, NSpace, NSpin } from "naive-ui";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";

export default defineComponent({
  name: "RepositoryHistoryPanel",
  components: { NButton, NCard, NDataTable, NEmpty, NSpace, NSpin },
  setup() {
    return useWorkspaceControllerContext();
  },
});
</script>

<template>
  <template v-if="snapshot">
<div class="changes-grid">
                  <n-card size="small" :title="tr('提交历史', 'Commit history')">
                    <template #header-extra>
                      <n-space size="small">
                        <n-button size="tiny" tertiary :loading="reflogLoading" @click="openReflog">{{ tr("查看 Reflog", "View reflog") }}</n-button>
                        <n-button size="tiny" tertiary :disabled="commits.length < 3" @click="openSquashModal">{{ tr("压缩最近提交", "Squash recent") }}</n-button>
                        <n-button size="tiny" tertiary :loading="historyLoading" @click="loadHistory">{{ tr("刷新", "Refresh") }}</n-button>
                      </n-space>
                    </template>
                    <n-spin :show="historyLoading">
                      <div v-if="commits.length" class="history-commit-list" :aria-label="tr('提交历史列表', 'Commit history list')">
                        <button
                          v-for="commit in commits"
                          :key="commit.sha"
                          type="button"
                          class="history-commit-row"
                          :class="{ selected: selectedCommit?.sha === commit.sha }"
                          :aria-label="tr('查看提交 {sha}：{summary}', 'View commit {sha}: {summary}', { sha: commit.shortSha, summary: commit.summary || '—' })"
                          @click="loadCommitDiff(commit)"
                        >
                          <span class="history-commit-primary">
                            <span class="history-commit-sha">{{ commit.shortSha }}</span>
                            <span class="history-commit-summary" :title="commit.summary || '—'">{{ commit.summary || '—' }}</span>
                          </span>
                          <span class="history-commit-meta">
                            <span class="history-commit-author" :title="commit.authorName || '—'">{{ commit.authorName || '—' }}</span>
                            <span class="history-commit-date" :title="commit.authoredAt">{{ commit.authoredAt }}</span>
                          </span>
                        </button>
                      </div>
                      <n-empty v-else-if="!historyLoading" :description="tr('这个分支还没有提交。', 'This branch has no commits yet.')" />
                    </n-spin>
                  </n-card>
                  <n-card size="small" :title="selectedCommit ? `${selectedCommit.shortSha} · ${selectedCommit.summary}` : tr('提交改动', 'Commit changes')" class="diff-card">
                    <template #header-extra>
                      <n-space v-if="selectedCommit" size="small">
                        <n-button size="tiny" tertiary @click="confirmRevertCommit(selectedCommit)">{{ tr("还原", "Revert") }}</n-button>
                        <n-button size="tiny" tertiary @click="confirmResetCommit(selectedCommit, 'soft')">{{ tr("Soft 重置", "Soft reset") }}</n-button>
                        <n-button size="tiny" tertiary @click="confirmResetCommit(selectedCommit, 'mixed')">{{ tr("Mixed 重置", "Mixed reset") }}</n-button>
                        <n-button size="tiny" type="error" tertiary @click="confirmResetCommit(selectedCommit, 'hard')">{{ tr("Hard 重置", "Hard reset") }}</n-button>
                      </n-space>
                    </template>
                    <n-spin :show="commitDiffLoading">
                      <pre v-if="commitDiffText" class="diff-output"><span v-for="(line, index) in commitDiffLines" :key="index" class="diff-line" :class="diffLineClass(line)">{{ line }}</span></pre>
                      <n-empty v-else :description="tr('选择一个提交以查看改动。', 'Select a commit to view its changes.')" size="small" />
                    </n-spin>
                  </n-card>
                </div>

                <n-card size="small" :title="tr('暂存栈', 'Stash')" class="commit-card">
                  <template #header-extra><n-button size="tiny" type="primary" :disabled="!snapshot.status.entries.length" @click="openStashModal">{{ tr("保存改动", "Stash changes") }}</n-button></template>
                  <n-data-table :columns="stashColumns" :data="stashes" :loading="stashesLoading" :bordered="false" />
                  <n-empty v-if="!stashesLoading && !stashes.length" :description="tr('暂存栈为空。', 'Stash is empty.')" />
                </n-card>
  </template>
</template>
