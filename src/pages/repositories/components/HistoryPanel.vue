<script lang="ts">
import { computed, defineComponent, nextTick, ref, watch } from "vue";
import { NButton, NCard, NDataTable, NDropdown, NEmpty, NSpace, NSpin, NTag } from "naive-ui";
import { useRepositoryPanelSplit } from "../../../features/repository/composables/useRepositoryPanelSplit";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";
import type { CommitChangedFile, CommitFileStatus, CommitSummary } from "../../../shared/types/domain";

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
    const commitFileStatusCodes: Record<CommitFileStatus, string> = {
      added: "A",
      copied: "C",
      deleted: "D",
      modified: "M",
      renamed: "R",
      typeChanged: "T",
      unmerged: "U",
      unknown: "?",
    };

    function commitFileStatusCode(file: CommitChangedFile): string {
      return commitFileStatusCodes[file.status];
    }

    function commitFileStatusClass(file: CommitChangedFile): string {
      return `is-${file.status === "typeChanged" ? "type-changed" : file.status}`;
    }

    function commitFileStatusLabel(file: CommitChangedFile): string {
      const labels: Record<CommitFileStatus, string> = {
        added: workspace.tr("新增", "Added"),
        copied: workspace.tr("复制", "Copied"),
        deleted: workspace.tr("删除", "Deleted"),
        modified: workspace.tr("修改", "Modified"),
        renamed: workspace.tr("重命名", "Renamed"),
        typeChanged: workspace.tr("类型变更", "Type changed"),
        unmerged: workspace.tr("未合并", "Unmerged"),
        unknown: workspace.tr("其他变更", "Other change"),
      };
      const similarity = file.similarity === null ? "" : ` · ${file.similarity}%`;
      return `${labels[file.status]}${similarity}`;
    }

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
      ...useRepositoryPanelSplit(),
      commitContextMenuOpen,
      commitContextMenuX,
      commitContextMenuY,
      commitContextMenuOptions,
      commitFileStatusCode,
      commitFileStatusClass,
      commitFileStatusLabel,
      openCommitContextMenu,
      selectCommitContextMenuItem,
    };
  },
});
</script>

<template>
  <template v-if="snapshot">
<div class="history-workspace repository-split-layout" :style="repositoryPanelStyle">
                  <n-card size="small" :title="tr('提交历史', 'Commit history')" class="history-commit-card">
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
                          @click="loadCommitFiles(commit)"
                          @contextmenu="openCommitContextMenu($event, commit)"
                        >
                          <span class="history-commit-primary">
                            <span class="history-commit-summary" :title="commit.summary || '—'">{{ commit.summary || '—' }}</span>
                            <span v-if="commit.tags.length || commit.tagsTruncated || commit.pushed === false" class="history-commit-tags">
                              <n-tag v-for="tag in commit.tags" :key="tag" size="tiny" type="primary" :bordered="false">{{ tag }}</n-tag>
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
                  <div
                    ref="repositoryPanelResizer"
                    class="repository-column-resizer"
                    role="separator"
                    tabindex="0"
                    aria-orientation="vertical"
                    :aria-label="tr('调整历史列表宽度', 'Resize history list')"
                    :aria-valuemin="240"
                    :aria-valuenow="repositoryPanelAriaValue"
                    :aria-valuemax="repositoryPanelAriaMax"
                    :aria-valuetext="tr('{width} 像素', '{width} pixels', { width: repositoryPanelAriaValue })"
                    :title="tr('拖动调整宽度，双击恢复默认值', 'Drag to resize; double-click to restore the default')"
                    @pointerdown="startRepositoryPanelResize"
                    @keydown="resizeRepositoryPanelWithKeyboard"
                    @dblclick="resetRememberedRepositoryPanelWidth"
                  />
                  <n-card size="small" :title="selectedCommit ? `${selectedCommit.shortSha} · ${selectedCommit.summary}` : tr('提交改动', 'Commit changes')" class="history-detail-card">
                    <template #header-extra>
                      <n-space v-if="selectedCommit" size="small">
                        <n-button size="tiny" tertiary @click="confirmRevertCommit(selectedCommit)">{{ tr("还原", "Revert") }}</n-button>
                        <n-button size="tiny" tertiary @click="confirmResetCommit(selectedCommit, 'soft')">{{ tr("Soft 重置", "Soft reset") }}</n-button>
                        <n-button size="tiny" tertiary @click="confirmResetCommit(selectedCommit, 'mixed')">{{ tr("Mixed 重置", "Mixed reset") }}</n-button>
                        <n-button size="tiny" type="error" tertiary @click="confirmResetCommit(selectedCommit, 'hard')">{{ tr("Hard 重置", "Hard reset") }}</n-button>
                      </n-space>
                    </template>
                    <div class="history-detail-grid">
                      <section class="history-files-pane" :aria-label="tr('提交变动文件', 'Files changed by commit')">
                        <header class="history-pane-header">
                          <strong>{{ tr("变动文件", "Changed files") }}</strong>
                          <span v-if="selectedCommit" class="history-pane-count">{{ commitChangedFiles.length }}</span>
                        </header>
                        <n-spin :show="commitFilesLoading" class="history-files-spin">
                          <div v-if="commitChangedFiles.length" class="history-file-list">
                            <button
                              v-for="file in commitChangedFiles"
                              :key="`${file.previousPath || ''}\u0000${file.path}`"
                              type="button"
                              class="history-file-row"
                              :class="[{ selected: selectedCommitFile?.path === file.path }, commitFileStatusClass(file)]"
                              :aria-label="`${commitFileStatusLabel(file)}：${file.path}`"
                              @click="loadCommitFileDiff(file)"
                            >
                              <span class="history-file-copy">
                                <span class="history-file-path" :title="file.path">{{ file.path }}</span>
                                <span v-if="file.previousPath" class="history-file-previous" :title="file.previousPath">
                                  {{ tr("原路径：{path}", "From: {path}", { path: file.previousPath }) }}
                                </span>
                                <span v-else class="history-file-kind">{{ commitFileStatusLabel(file) }}</span>
                              </span>
                              <span class="history-file-status" :title="commitFileStatusLabel(file)">{{ commitFileStatusCode(file) }}</span>
                            </button>
                          </div>
                          <n-empty
                            v-else-if="!commitFilesLoading"
                            :description="selectedCommit ? tr('这个提交没有可显示的文件改动。', 'This commit has no displayable file changes.') : tr('选择一个提交以查看变动文件。', 'Select a commit to view changed files.')"
                            size="small"
                          />
                        </n-spin>
                      </section>

                      <section class="history-file-diff-pane" :aria-label="tr('提交文件差异', 'Commit file diff')">
                        <header class="history-pane-header history-diff-header">
                          <strong :title="selectedCommitFile?.path">{{ selectedCommitFile?.path || tr("文件差异", "File diff") }}</strong>
                          <span v-if="selectedCommitFile" class="history-file-kind">{{ commitFileStatusLabel(selectedCommitFile) }}</span>
                        </header>
                        <div class="history-diff-content">
                          <n-spin v-if="commitDiffLoading" size="small" class="history-diff-loading" />
                          <pre v-else-if="commitDiffText" class="diff-output"><span v-for="(line, index) in commitDiffLines" :key="index" class="diff-line" :class="diffLineClass(line)">{{ line }}</span></pre>
                          <n-empty
                            v-else
                            :description="selectedCommitFile ? tr('此文件没有可显示的文本 diff。', 'No text diff is available for this file.') : tr('选择一个文件以查看 diff。', 'Select a file to view its diff.')"
                            size="small"
                          />
                        </div>
                      </section>
                    </div>
                  </n-card>
                </div>

                <n-card size="small" :title="tr('暂存栈', 'Stash')" class="commit-card">
                  <template #header-extra><n-button size="tiny" type="primary" :disabled="!snapshot.status.entries.length" @click="openStashModal">{{ tr("保存改动", "Stash changes") }}</n-button></template>
                  <n-data-table :columns="stashColumns" :data="stashes" :loading="stashesLoading" :bordered="false" />
                  <n-empty v-if="!stashesLoading && !stashes.length" :description="tr('暂存栈为空。', 'Stash is empty.')" />
                </n-card>
  </template>
</template>

<style src="./HistoryPanel.css"></style>
