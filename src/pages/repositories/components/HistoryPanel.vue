<script lang="ts">
import { computed, defineComponent, nextTick, ref, watch } from "vue";
import { NButton, NCard, NDataTable, NDropdown, NEmpty, NSpace, NSpin, NTag } from "naive-ui";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";
import type { CommitSummary } from "../../../shared/types/domain";

export default defineComponent({
  name: "RepositoryHistoryPanel",
  components: { NButton, NCard, NDataTable, NDropdown, NEmpty, NSpace, NSpin, NTag },
  setup() {
    const workspace = useWorkspaceControllerContext();
    const commitContextMenuOpen = ref(false);
    const commitContextMenuX = ref(0);
    const commitContextMenuY = ref(0);
    const commitContextTarget = ref<{ repositoryPath: string; sha: string } | null>(null);
    let commitContextMenuGeneration = 0;
    const commitContextMenuOptions = computed(() => [
      { label: workspace.tr("在此提交创建标签…", "Create tag at this commit…"), key: "create-tag" },
    ]);

    function openCommitContextMenu(event: MouseEvent, commit: CommitSummary): void {
      event.preventDefault();
      const repositoryPath = workspace.activeRepositoryPath.value;
      if (!repositoryPath) return;
      const requestGeneration = ++commitContextMenuGeneration;
      commitContextMenuOpen.value = false;
      commitContextTarget.value = { repositoryPath, sha: commit.sha };
      commitContextMenuX.value = event.clientX;
      commitContextMenuY.value = event.clientY;
      void nextTick(() => {
        if (
          requestGeneration === commitContextMenuGeneration &&
          commitContextTarget.value?.repositoryPath === workspace.activeRepositoryPath.value &&
          commitContextTarget.value?.sha === commit.sha &&
          workspace.commits.value.some((candidate) => candidate.sha === commit.sha)
        ) {
          commitContextMenuOpen.value = true;
        }
      });
    }

    function selectCommitContextMenuItem(value: string | number): void {
      const target = commitContextTarget.value;
      commitContextMenuGeneration += 1;
      commitContextMenuOpen.value = false;
      commitContextTarget.value = null;
      if (
        value === "create-tag" &&
        target &&
        target.repositoryPath === workspace.activeRepositoryPath.value &&
        workspace.commits.value.some((commit) => commit.sha === target.sha)
      ) {
        workspace.openTagModal(target.sha);
      }
    }

    watch(
      [workspace.activeRepositoryPath, workspace.commits],
      () => {
        commitContextMenuGeneration += 1;
        commitContextMenuOpen.value = false;
        commitContextTarget.value = null;
      },
    );

    return {
      ...workspace,
      commitContextMenuOpen,
      commitContextMenuX,
      commitContextMenuY,
      commitContextMenuOptions,
      openCommitContextMenu,
      selectCommitContextMenuItem,
    };
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
                          @contextmenu="openCommitContextMenu($event, commit)"
                        >
                          <span class="history-commit-primary">
                            <span class="history-commit-summary" :title="commit.summary || '—'">{{ commit.summary || '—' }}</span>
                            <span v-if="commit.tags.length || commit.tagsTruncated || commit.pushed === false" class="history-commit-tags">
                              <n-tag v-for="tag in commit.tags" :key="tag" size="tiny" type="success" :bordered="false">{{ tag }}</n-tag>
                              <n-tag v-if="commit.tagsTruncated" size="tiny" type="warning" :bordered="false">{{ tr("标签未完整加载", "Tags incomplete") }}</n-tag>
                              <n-tag v-if="commit.pushed === false" size="tiny" type="warning" :bordered="false">{{ tr("未推送", "Not pushed") }}</n-tag>
                            </span>
                          </span>
                          <span class="history-commit-meta">
                            <span class="history-commit-author" :title="commit.authorName || '—'">{{ commit.authorName || '—' }}</span>
                            <span class="history-commit-sha">{{ commit.shortSha }}</span>
                            <span class="history-commit-date" :title="commit.authoredAt">{{ commit.authoredAt }}</span>
                          </span>
                        </button>
                      </div>
                      <n-dropdown
                        v-model:show="commitContextMenuOpen"
                        trigger="manual"
                        placement="bottom-start"
                        :x="commitContextMenuX"
                        :y="commitContextMenuY"
                        :options="commitContextMenuOptions"
                        @clickoutside="commitContextMenuOpen = false"
                        @select="selectCommitContextMenuItem"
                      />
                      <n-empty v-if="!commits.length && !historyLoading" :description="tr('这个分支还没有提交。', 'This branch has no commits yet.')" />
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
