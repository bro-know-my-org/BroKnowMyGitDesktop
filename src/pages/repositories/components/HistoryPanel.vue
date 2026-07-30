<script lang="ts">
import { computed, defineComponent, nextTick, ref, watch } from "vue";
import { NButton, NCard, NDataTable, NDropdown, NEmpty, NScrollbar, NSpace, NSpin, NTag, useMessage } from "naive-ui";
import RepositoryDiffViewer from "../../../features/repository/components/RepositoryDiffViewer.vue";
import { useRepositoryPanelSplit } from "../../../features/repository/composables/useRepositoryPanelSplit";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";
import { t } from "../../../shared/i18n";
import type { CommitChangedFile, CommitFileStatus, CommitSummary } from "../../../shared/types/domain";

export default defineComponent({
  name: "RepositoryHistoryPanel",
  components: { NButton, NCard, NDataTable, NDropdown, NEmpty, NScrollbar, NSpace, NSpin, NTag, RepositoryDiffViewer },
  setup() {
    const workspace = useWorkspaceControllerContext();
    const message = useMessage();
    const commitContextMenuOpen = ref(false);
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

    function writeClipboardTextWithSelection(text: string): boolean {
      const textarea = document.createElement("textarea");
      textarea.value = text;
      textarea.setAttribute("readonly", "");
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.appendChild(textarea);
      textarea.select();
      try {
        return document.execCommand("copy");
      } catch {
        return false;
      } finally {
        textarea.remove();
      }
    }

    async function writeClipboardText(text: string): Promise<void> {
      // Keep this synchronous attempt in the context-menu event call stack so
      // WebViews that require transient user activation can grant clipboard access.
      if (writeClipboardTextWithSelection(text)) return;
      if (!navigator.clipboard?.writeText) throw new Error("clipboard unavailable");
      await navigator.clipboard.writeText(text);
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
                      <n-space size="small">
                        <n-button size="tiny" tertiary :disabled="reflogLoading" @click="openReflog">{{ t('repository.history.viewReflog') }}</n-button>
                        <n-button size="tiny" tertiary :disabled="commits.length < 3" @click="openSquashModal">{{ t('repository.history.squashRecent') }}</n-button>
                        <n-button size="tiny" tertiary :loading="historyLoading" @click="loadHistory">{{ t('repository.history.refresh') }}</n-button>
                      </n-space>
                    </template>
                    <n-spin :show="historyLoading">
                      <n-scrollbar
                        v-if="commits.length"
                        class="history-commit-scroll"
                        content-class="history-commit-list"
                        :aria-label="t('repository.history.commitHistoryList')"
                      >
                        <button
                          v-for="commit in commits"
                          :key="commit.sha"
                          type="button"
                          class="history-commit-row"
                          :class="{ selected: selectedCommit?.sha === commit.sha }"
                          :aria-label="t('repository.history.viewCommitShaSummary', { sha: commit.shortSha, summary: commit.summary || '—' })"
                          @click="loadCommitFiles(commit)"
                          @contextmenu="openCommitContextMenu($event, commit)"
                        >
                          <span class="history-commit-primary">
                            <span class="history-commit-summary" :title="commit.summary || '—'">{{ commit.summary || '—' }}</span>
                            <span v-if="commit.tags.length || commit.tagsTruncated || commit.pushed === false" class="history-commit-tags">
                              <n-tag v-for="tag in commit.tags" :key="tag" size="tiny" type="primary" :bordered="false">{{ tag }}</n-tag>
                              <n-tag v-if="commit.tagsTruncated" size="tiny" type="warning" :bordered="false">{{ t('repository.history.tagsIncomplete') }}</n-tag>
                              <n-tag v-if="commit.pushed === false" size="tiny" type="warning" :bordered="false">{{ t('repository.history.notPushed') }}</n-tag>
                            </span>
                          </span>
                          <span class="history-commit-meta">
                            <span class="history-commit-author" :title="commit.authorName || '—'">{{ commit.authorName || '—' }}</span>
                            <span class="history-commit-sha">{{ commit.shortSha }}</span>
                            <span class="history-commit-date" :title="commit.authoredAt">{{ commit.authoredAt }}</span>
                          </span>
                        </button>
                      </n-scrollbar>
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
                      <n-empty v-if="!commits.length && !historyLoading" :description="t('repository.history.branchHasNoCommitsYet')" />
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
                    :aria-valuetext="t('repository.changes.widthPixels', { width: repositoryPanelAriaValue })"
                    :title="t('repository.changes.dragResizeDoubleClickRestoreDefault')"
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
                                  {{ t('repository.history.previousPath', { path: file.previousPath }) }}
                                </span>
                                <span v-else class="history-file-kind">{{ commitFileStatusLabel(file) }}</span>
                              </span>
                              <span class="history-file-status" :title="commitFileStatusLabel(file)">{{ commitFileStatusCode(file) }}</span>
                            </button>
                          </div>
                          <n-empty
                            v-else-if="!commitFilesLoading"
                            :description="selectedCommit ? t('repository.history.commitHasNoDisplayableFileChanges') : t('repository.history.selectCommitViewChangedFiles')"
                            size="small"
                          />
                        </n-spin>
                      </section>

                      <section class="history-file-diff-pane" :aria-label="t('repository.history.commitFileDiff')">
                        <header class="history-pane-header history-diff-header">
                          <strong :title="selectedCommitFile?.path">{{ selectedCommitFile?.path || t('repository.history.fileDiff') }}</strong>
                          <span v-if="selectedCommitFile" class="history-file-kind">{{ commitFileStatusLabel(selectedCommitFile) }}</span>
                        </header>
                        <div class="history-diff-content">
                          <n-spin v-if="commitDiffLoading" size="small" class="history-diff-loading" />
                          <template v-else-if="commitDiffText">
                            <RepositoryDiffViewer
                              :lines="commitDiffVisibleLines"
                              :line-class="diffLineClass"
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
                          />
                        </div>
                      </section>
                    </div>
                  </n-card>
                </div>

                <n-card size="small" :title="t('repository.history.stash')" class="commit-card">
                  <template #header-extra><n-button size="tiny" type="primary" :disabled="!snapshot.status.entries.length" @click="openStashModal">{{ t('repository.history.stashChanges') }}</n-button></template>
                  <n-data-table :columns="stashColumns" :data="stashes" :loading="stashesLoading" :bordered="false" />
                  <n-empty v-if="!stashesLoading && !stashes.length" :description="t('repository.history.stashEmpty')" />
                </n-card>
  </template>
</template>

<style src="./HistoryPanel.css"></style>
