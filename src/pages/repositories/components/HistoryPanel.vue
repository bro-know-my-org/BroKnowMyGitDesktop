<script lang="ts">
import { computed, defineComponent, nextTick, ref, watch } from "vue";
import { NButton, NCard, NDataTable, NDrawer, NDrawerContent, NDropdown, NEmpty, NSpace, NSpin, NTag, useMessage } from "naive-ui";
import RepositoryFileDiff from "../../../features/repository/components/RepositoryFileDiff.vue";
import RepositoryDiffViewer from "../../../features/repository/components/RepositoryDiffViewer.vue";
import { useRepositoryPanelSplit } from "../../../features/repository/composables/useRepositoryPanelSplit";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";
import { resolvedInterfaceFontSize } from "../../../shared/fonts";
import { t } from "../../../shared/i18n";
import { writeClipboardText } from "../../../shared/clipboard";
import type { CommitChangedFile, CommitFileStatus, CommitSummary } from "../../../shared/types/domain";
import FixedVirtualList from "../../../shared/virtual-list/FixedVirtualList.vue";

export default defineComponent({
  name: "RepositoryHistoryPanel",
  components: { FixedVirtualList, NButton, NCard, NDataTable, NDrawer, NDrawerContent, NDropdown, NEmpty, NSpace, NSpin, NTag, RepositoryDiffViewer, RepositoryFileDiff },
  setup() {
    const workspace = useWorkspaceControllerContext();
    const message = useMessage();
    const commitContextMenuOpen = ref(false);
    const stashDrawerShown = ref(false);
    const commitContextMenuX = ref(0);
    const commitContextMenuY = ref(0);
    const commitContextTarget = ref<{ repositoryPath: string; sha: string } | null>(null);
    let commitContextMenuGeneration = 0;
    const commitContextMenuOptions = computed(() => [
      { label: t('repository.history.copyFullSha'), key: "copy-sha" },
      { label: t('repository.history.copyCommitSummary'), key: "copy-summary" },
      { label: t('repository.history.copyCommitDetails'), key: "copy-details" },
      { type: "divider", key: "copy-divider" },
      { label: t('repository.history.createTagCommit'), key: "create-tag" },
    ]);
    const historyActionOptions = computed(() => [
      { label: t('repository.history.refresh'), key: "refresh", disabled: workspace.historyLoading.value },
      { label: t('repository.history.viewReflog'), key: "reflog", disabled: workspace.reflogLoading.value },
      { label: t('repository.history.squashRecent'), key: "squash", disabled: workspace.commits.value.length < 3 },
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
    const historyFileRowHeight = computed(() => Math.ceil((25.3 * resolvedInterfaceFontSize.value / 14) + 3));
    const historyCommitRowHeight = computed(() => Math.ceil(40 * resolvedInterfaceFontSize.value / 14 + 24));
    const commitVirtualKeys = computed(() => workspace.commits.value.map((commit) => commit.sha));
    const commitFileVirtualKeys = computed(() => workspace.commitChangedFiles.value.map(
      (file) => `${file.previousPath || ""}\u0000${file.path}`,
    ));

    function commitFileStatusCode(file: CommitChangedFile): string {
      return commitFileStatusCodes[file.status];
    }

    function commitFileStatusClass(file: CommitChangedFile): string {
      return `is-${file.status === "typeChanged" ? "type-changed" : file.status}`;
    }

    function commitFileStatusLabel(file: CommitChangedFile): string {
      const labels: Record<CommitFileStatus, string> = {
        added: t('repository.history.added'),
        copied: t('repository.history.copied'),
        deleted: t('repository.history.deleted'),
        modified: t('repository.history.modified'),
        renamed: t('repository.history.renamed'),
        typeChanged: t('repository.history.typeChanged'),
        unmerged: t('repository.history.unmerged'),
        unknown: t('repository.history.otherChange'),
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

    function commitDetails(commit: CommitSummary): string {
      const author = commit.authorEmail
        ? `${commit.authorName || t('repository.history.unknownAuthor')} <${commit.authorEmail}>`
        : commit.authorName || t('repository.history.unknownAuthor');
      return t('repository.history.commitDetails', {
        sha: commit.sha,
        author,
        date: commit.authoredAt,
        summary: commit.summary || "—",
      });
    }

    async function copyCommitInformation(commit: CommitSummary, value: string | number): Promise<void> {
      let text: string;
      let successText: string;
      if (value === "copy-sha") {
        text = commit.sha;
        successText = t('repository.history.fullShaCopied');
      } else if (value === "copy-summary") {
        text = commit.summary;
        successText = t('repository.history.commitSummaryCopied');
      } else if (value === "copy-details") {
        text = commitDetails(commit);
        successText = t('repository.history.commitDetailsCopied');
      } else {
        return;
      }

      try {
        await writeClipboardText(text);
        message.success(successText);
      } catch {
        message.error(t('repository.history.couldNotCopyClipboard'));
      }
    }

    function selectCommitContextMenuItem(value: string | number): void {
      const target = commitContextTarget.value;
      commitContextMenuGeneration += 1;
      commitContextMenuOpen.value = false;
      commitContextTarget.value = null;
      if (!target || target.repositoryPath !== workspace.activeRepositoryPath.value) return;
      const commit = workspace.commits.value.find((candidate) => candidate.sha === target.sha);
      if (!commit) return;
      if (value === "create-tag") {
        workspace.openTagModal(target.sha);
      } else {
        void copyCommitInformation(commit, value);
      }
    }

    function selectHistoryAction(value: string | number): void {
      if (value === "refresh") {
        void workspace.loadHistory();
      } else if (value === "reflog") {
        workspace.openReflog();
      } else if (value === "squash") {
        workspace.openSquashModal();
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

    watch(workspace.activeRepositoryPath, () => {
      stashDrawerShown.value = false;
    });

    return {
      ...workspace,
      ...useRepositoryPanelSplit(),
      commitContextMenuOpen,
      stashDrawerShown,
      commitContextMenuX,
      commitContextMenuY,
      commitContextMenuOptions,
      historyActionOptions,
      historyFileRowHeight,
      historyCommitRowHeight,
      commitVirtualKeys,
      commitFileVirtualKeys,
      commitFileStatusCode,
      commitFileStatusClass,
      commitFileStatusLabel,
      openCommitContextMenu,
      selectCommitContextMenuItem,
      selectHistoryAction,
      t,
    };
  },
});
</script>

<template>
  <template v-if="snapshot">
<div class="history-workspace repository-split-layout" :style="repositoryPanelStyle">
                  <n-card size="small" :title="t('repository.history.commitHistory')" class="history-commit-card">
                    <template #header-extra>
                      <n-space size="small" :wrap="false">
                        <n-button
                          size="tiny"
                          tertiary
                          :loading="stashesLoading"
                          :aria-label="t('repository.history.stashCount', { count: stashes.length })"
                          :title="t('repository.history.stashCount', { count: stashes.length })"
                          @click="stashDrawerShown = true"
                        >
                          <svg class="history-header-action-icon" viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M4 8h16v12H4zM3 4h18v4H3zm6 8h6" />
                          </svg>
                          <span v-if="stashes.length" class="history-stash-count">{{ stashes.length }}</span>
                        </n-button>
                        <n-dropdown trigger="click" :options="historyActionOptions" @select="selectHistoryAction">
                          <n-button size="tiny" tertiary :aria-label="t('repository.history.moreActions')" :title="t('repository.history.moreActions')">
                            <svg class="history-header-action-icon is-filled" viewBox="0 0 24 24" aria-hidden="true">
                              <circle cx="5" cy="12" r="2" />
                              <circle cx="12" cy="12" r="2" />
                              <circle cx="19" cy="12" r="2" />
                            </svg>
                          </n-button>
                        </n-dropdown>
                      </n-space>
                    </template>
                    <n-spin :show="historyLoading">
                      <FixedVirtualList
                        v-if="commits.length"
                        class="history-commit-scroll"
                        :item-count="commits.length"
                        :item-height="historyCommitRowHeight"
                        :item-keys="commitVirtualKeys"
                        :loading="historyLoading || historyLoadingMore"
                        :reset-key="activeRepositoryPath"
                        :aria-label="t('repository.history.commitHistoryList')"
                        @end-reached="loadMoreHistory"
                      >
                        <template #default="{ index, tabIndex }">
                        <button
                          data-virtual-list-focus
                          :tabindex="tabIndex"
                          type="button"
                          class="history-commit-row"
                          :class="{ selected: selectedCommit?.sha === commits[index].sha }"
                          :aria-label="t('repository.history.viewCommitShaSummary', { sha: commits[index].shortSha, summary: commits[index].summary || '—' })"
                          @click="loadCommitFiles(commits[index])"
                          @contextmenu="openCommitContextMenu($event, commits[index])"
                        >
                          <span class="history-commit-primary">
                            <span class="history-commit-summary" :title="commits[index].summary || '—'">{{ commits[index].summary || '—' }}</span>
                            <span v-if="commits[index].tags.length || commits[index].tagsTruncated || commits[index].pushed === false" class="history-commit-tags">
                              <n-tag v-for="tag in commits[index].tags" :key="tag" size="tiny" type="primary" :bordered="false">{{ tag }}</n-tag>
                              <n-tag v-if="commits[index].tagsTruncated" size="tiny" type="warning" :bordered="false">{{ t('repository.history.tagsIncomplete') }}</n-tag>
                              <n-tag v-if="commits[index].pushed === false" size="tiny" type="warning" :bordered="false">{{ t('repository.history.notPushed') }}</n-tag>
                            </span>
                          </span>
                          <span class="history-commit-meta">
                            <span class="history-commit-author" :title="commits[index].authorName || '—'">{{ commits[index].authorName || '—' }}</span>
                            <span class="history-commit-sha">{{ commits[index].shortSha }}</span>
                            <span class="history-commit-date" :title="commits[index].authoredAt">{{ commits[index].authoredAt }}</span>
                          </span>
                        </button>
                        </template>
                      </FixedVirtualList>
                      <div v-if="commits.length" class="history-pagination" aria-live="polite">
                        <span>{{ t('repository.history.loadedCommits', { count: commits.length }) }}</span>
                        <n-button v-if="historyHasMore" size="tiny" text :loading="historyLoadingMore" :disabled="historyLoading" @click="loadMoreHistory">
                          {{ t('repository.history.loadMore') }}
                        </n-button>
                        <span v-else>{{ t('repository.history.allLoaded') }}</span>
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
                      <n-empty v-if="!commits.length && !historyLoading" :description="t('repository.history.branchHasNoCommitsYet')" :show-icon="false" />
                    </n-spin>
                  </n-card>
                  <div
                    ref="repositoryPanelResizer"
                    class="repository-column-resizer"
                    role="separator"
                    tabindex="0"
                    aria-orientation="vertical"
                    :aria-label="t('repository.history.resizeHistoryList')"
                    :aria-valuemin="240"
                    :aria-valuenow="repositoryPanelAriaValue"
                    :aria-valuemax="repositoryPanelAriaMax"
                    :aria-valuetext="t('repository.history.widthPixels', { width: repositoryPanelAriaValue })"
                    :title="t('repository.history.dragResizeDoubleClickRestoreDefault')"
                    @pointerdown="startRepositoryPanelResize"
                    @keydown="resizeRepositoryPanelWithKeyboard"
                    @dblclick="resetRememberedRepositoryPanelWidth"
                  />
                  <n-card size="small" :title="selectedCommit ? `${selectedCommit.shortSha} · ${selectedCommit.summary}` : t('repository.history.commitChanges')" class="history-detail-card">
                    <template #header-extra>
                      <n-space v-if="selectedCommit" size="small">
                        <n-button size="tiny" tertiary @click="confirmRevertCommit(selectedCommit)">{{ t('repository.history.revert') }}</n-button>
                        <n-button size="tiny" tertiary @click="confirmResetCommit(selectedCommit, 'soft')">{{ t('repository.history.softReset') }}</n-button>
                        <n-button size="tiny" tertiary @click="confirmResetCommit(selectedCommit, 'mixed')">{{ t('repository.history.mixedReset') }}</n-button>
                        <n-button size="tiny" type="error" tertiary @click="confirmResetCommit(selectedCommit, 'hard')">{{ t('repository.history.hardReset') }}</n-button>
                      </n-space>
                    </template>
                    <div class="history-detail-grid">
                      <section class="history-files-pane" :aria-label="t('repository.history.filesChangedCommit')">
                        <header class="history-pane-header">
                          <strong>{{ t('repository.history.changedFiles') }}</strong>
                          <span v-if="selectedCommit" class="history-pane-count">{{ commitChangedFiles.length }}</span>
                        </header>
                        <n-spin :show="commitFilesLoading" class="history-files-spin">
                          <FixedVirtualList
                            v-if="commitChangedFiles.length"
                            class="history-file-list"
                            :item-count="commitChangedFiles.length"
                            :item-height="historyFileRowHeight"
                            :item-keys="commitFileVirtualKeys"
                            :overscan-viewports="3"
                            :reset-key="selectedCommit?.sha"
                          >
                            <template #default="{ index, tabIndex }">
                            <button
                              type="button"
                              class="history-file-row"
                              data-virtual-list-focus
                              :tabindex="tabIndex"
                              :class="[{ selected: selectedCommitFile?.path === commitChangedFiles[index].path }, commitFileStatusClass(commitChangedFiles[index])]"
                              :aria-label="`${commitFileStatusLabel(commitChangedFiles[index])}：${commitChangedFiles[index].path}`"
                              @click="loadCommitFileDiff(commitChangedFiles[index])"
                            >
                              <span class="history-file-copy">
                                <span class="history-file-path" :title="commitChangedFiles[index].path">{{ commitChangedFiles[index].path }}</span>
                                <span v-if="commitChangedFiles[index].previousPath" class="history-file-previous" :title="commitChangedFiles[index].previousPath || ''">
                                  {{ t('repository.history.previousPath', { path: commitChangedFiles[index].previousPath || '' }) }}
                                </span>
                                <span v-else class="history-file-kind">{{ commitFileStatusLabel(commitChangedFiles[index]) }}</span>
                              </span>
                              <span class="history-file-status" :title="commitFileStatusLabel(commitChangedFiles[index])">{{ commitFileStatusCode(commitChangedFiles[index]) }}</span>
                            </button>
                            </template>
                          </FixedVirtualList>
                          <n-empty
                            v-else-if="!commitFilesLoading"
                            :description="selectedCommit ? t('repository.history.commitHasNoDisplayableFileChanges') : t('repository.history.selectCommitViewChangedFiles')"
                            size="small"
                            :show-icon="false"
                          />
                        </n-spin>
                      </section>

                      <section class="history-file-diff-pane" :aria-label="t('repository.history.commitFileDiff')">
                        <header class="history-pane-header history-diff-header">
                          <strong :title="selectedCommitFile?.path">{{ selectedCommitFile?.path || t('repository.history.fileDiff') }}</strong>
                          <span v-if="selectedCommitFile" class="history-file-kind">{{ commitFileStatusLabel(selectedCommitFile) }}</span>
                        </header>
                        <div class="history-diff-content">
                          <RepositoryFileDiff :repository-path="activeRepositoryPath" :comparison="selectedCommitFile && selectedCommit ? { path: selectedCommitFile.path, previousPath: selectedCommitFile.previousPath, staged: false, revision: selectedCommit.sha } : null" :diff-text="commitDiffText" :loading="commitDiffLoading">
                          <n-spin v-if="commitDiffLoading" size="small" class="history-diff-loading" />
                          <template v-else-if="commitDiffText">
                            <RepositoryDiffViewer
                              :lines="commitDiffVisibleLines"
                              :line-class="diffLineClass"
                              :file-path="selectedCommitFile?.path || ''"
                              :reset-key="`${selectedCommit?.sha || ''}\u0000${selectedCommitFile?.previousPath || ''}\u0000${selectedCommitFile?.path || ''}`"
                              :label="t('repository.history.commitFileDiff')"
                            />
                            <div v-if="commitDiffCanExpand" class="history-diff-more">
                              <n-button size="small" tertiary @click="showMoreCommitDiffLines">
                                {{ commitDiffHasMoreLines && !commitDiffLineLimitReached ? t('repository.history.show2000MoreLines') : t('repository.history.loadUpTo4MiB') }}
                              </n-button>
                            </div>
                            <div v-else-if="commitDiffLineLimitReached" class="history-diff-limit">
                              {{ t('repository.history.diffLineLimit') }}
                            </div>
                            <div v-else-if="commitDiffLimitedAtMaximum" class="history-diff-limit">
                              {{ t('repository.history.diffSizeLimit') }}
                            </div>
                          </template>
                          <n-empty
                            v-else
                            :description="selectedCommitFile ? t('repository.history.noTextDiffAvailableFile') : t('repository.history.selectFileViewItsDiff')"
                            size="small"
                            :show-icon="false"
                          />
                          </RepositoryFileDiff>
                        </div>
                      </section>
                    </div>
                  </n-card>
                </div>

                <n-drawer v-model:show="stashDrawerShown" width="min(560px, 94vw)" placement="right">
                  <n-drawer-content :title="t('repository.history.stash')" :native-scrollbar="false" closable>
                    <div class="history-stash-toolbar">
                      <span class="history-stash-summary">{{ t('repository.history.stashCount', { count: stashes.length }) }}</span>
                      <n-button type="primary" :disabled="!snapshot.status.entries.length" @click="openStashModal">
                        {{ t('repository.history.stashChanges') }}
                      </n-button>
                    </div>
                    <n-data-table
                      v-if="stashesLoading || stashes.length"
                      :columns="stashColumns"
                      :data="stashes"
                      :loading="stashesLoading"
                      :bordered="false"
                      size="small"
                    />
                    <n-empty v-else class="history-stash-empty" :description="t('repository.history.stashEmpty')" :show-icon="false" />
                  </n-drawer-content>
                </n-drawer>
  </template>
</template>

<style src="./HistoryPanel.css"></style>
