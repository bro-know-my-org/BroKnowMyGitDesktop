<script lang="ts">
import { defineComponent } from "vue";
import { NAlert, NButton, NCard, NCheckbox, NEmpty, NInput, NSpace, NSpin, NText } from "naive-ui";
import { useRepositoryPanelSplit } from "../../../features/repository/composables/useRepositoryPanelSplit";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";

export default defineComponent({
  name: "RepositoryChangesPanel",
  components: { NAlert, NButton, NCard, NCheckbox, NEmpty, NInput, NSpace, NSpin, NText },
  setup() {
    return {
      ...useWorkspaceControllerContext(),
      ...useRepositoryPanelSplit(),
    };
  },
});
</script>

<template>
  <template v-if="snapshot">
<n-alert v-if="isUnbornRepository" type="info" :bordered="false" class="unborn-repository-alert">
                  {{ tr("这是一个新仓库。先勾选要提交的文件、填写提交说明，再创建首个提交。", "This is a new repository. Include the files you want, add a commit summary, then create the first commit.") }}
                </n-alert>
                <div class="changes-workspace repository-split-layout" :style="repositoryPanelStyle">
                  <div class="changes-list-pane">
                    <n-card size="small" :title="tr('改动文件', 'Changed files')" class="changes-file-card">
                      <template #header-extra>
                        <n-space size="small">
                          <n-button v-if="untrackedCount" size="tiny" text type="warning" :loading="mutationBusy" @click="confirmCleanUntracked">{{ tr("清理未跟踪…", "Clean untracked…") }}</n-button>
                          <n-button size="tiny" text type="error" :disabled="!snapshot.status.entries.length" :loading="mutationBusy" @click="confirmDiscardChanges(snapshot.status.entries.map((entry) => entry.path))">{{ tr("丢弃全部…", "Discard all…") }}</n-button>
                        </n-space>
                      </template>
                      <div v-if="snapshot.status.entries.length" class="changes-bulk-control">
                        <n-checkbox
                          size="small"
                          :checked="allChangesIncluded"
                          :indeterminate="someChangesIncluded"
                          :disabled="mutationBusy"
                          @update:checked="updateAllChangeInclusion"
                        >{{ tr("全部加入本次提交", "Include all changes") }}</n-checkbox>
                      </div>
                      <div v-if="snapshot.status.entries.length" class="changes-file-list" :aria-label="tr('改动文件列表', 'Changed files list')">
                        <div
                          v-for="entry in snapshot.status.entries"
                          :key="entry.path"
                          class="change-file-row"
                          :class="{ selected: selectedChange?.path === entry.path }"
                          role="button"
                          tabindex="0"
                          @click="loadDiff(entry)"
                          @keydown.enter.prevent="loadDiff(entry)"
                          @keydown.space.prevent="loadDiff(entry)"
                        >
                          <span class="change-file-checkbox" @click.stop>
                            <n-checkbox
                              size="small"
                              :checked="isChangeFullyIncluded(entry)"
                              :indeterminate="isChangePartiallyIncluded(entry)"
                              :disabled="mutationBusy"
                              :aria-label="changeIncludeLabel(entry)"
                              @update:checked="updateChangeInclusion(entry, $event)"
                            />
                          </span>
                          <span class="change-file-copy">
                            <span class="change-file-path" :title="entry.path">{{ entry.path }}</span>
                            <span v-if="changeStatusLabel(entry)" class="change-file-state">{{ changeStatusLabel(entry) }}</span>
                          </span>
                          <span class="change-file-status" :class="`is-${changeStatusTagType(entry)}`" aria-hidden="true" />
                        </div>
                      </div>
                      <n-empty v-if="!snapshot.status.entries.length" :description="tr('工作区干净，没有改动', 'Working tree is clean')" />
                    </n-card>

                    <n-card size="small" :title="tr('创建提交', 'Create commit')" class="commit-card changes-commit-card">
                      <div class="commit-form">
                        <label for="commit-message" class="commit-message-label">{{ tr('提交说明', 'Commit message') }}</label>
                        <n-input
                          v-model:value="commitMessage"
                          type="textarea"
                          :autosize="{ minRows: 3, maxRows: 5 }"
                          :input-props="{ id: 'commit-message' }"
                          :placeholder="tr('第一行写标题；需要时空一行补充正文', 'Write the title on the first line; add a body after a blank line if needed')"
                        />
                        <n-checkbox v-if="canAmendLatestCommit" class="commit-amend-checkbox" :checked="amendLastCommit" @update:checked="setAmendLastCommit">{{ tr("修改最近一次提交（amend）", "Amend the latest commit") }}</n-checkbox>
                      </div>
                      <n-text depth="3" class="commit-selection-summary">{{ commitSelectionSummary }}</n-text>
                      <n-button block type="primary" class="commit-primary-action" :loading="mutationBusy" :disabled="!stagedCount && !amendLastCommit" @click="commitChanges">{{ commitActionLabel }}</n-button>
                    </n-card>
                  </div>

                  <div
                    ref="repositoryPanelResizer"
                    class="repository-column-resizer"
                    role="separator"
                    tabindex="0"
                    aria-orientation="vertical"
                    :aria-label="tr('调整文件列表宽度', 'Resize file list')"
                    :aria-valuemin="240"
                    :aria-valuenow="repositoryPanelAriaValue"
                    :aria-valuemax="repositoryPanelAriaMax"
                    :aria-valuetext="tr('{width} 像素', '{width} pixels', { width: repositoryPanelAriaValue })"
                    :title="tr('拖动调整宽度，双击恢复默认值', 'Drag to resize; double-click to restore the default')"
                    @pointerdown="startRepositoryPanelResize"
                    @keydown="resizeRepositoryPanelWithKeyboard"
                    @dblclick="resetRememberedRepositoryPanelWidth"
                  />

                  <n-card size="small" :title="selectedChange ? selectedChange.path : tr('差异预览', 'Diff preview')" class="diff-card changes-diff-card">
                    <template #header-extra>
                      <n-space v-if="selectedChange" size="small">
                        <n-button size="tiny" tertiary :loading="blameLoading" :disabled="!snapshot.hasHead || (!selectedChange.indexStatus && selectedChange.worktreeStatus === '?')" @click="openFileBlame">{{ tr("查看 Blame", "View blame") }}</n-button>
                        <n-button size="tiny" text type="error" :loading="mutationBusy" @click="discardSelectedChange">{{ tr("丢弃此文件…", "Discard file…") }}</n-button>
                      </n-space>
                    </template>
                    <div class="changes-diff-panel">
                      <n-spin v-if="diffLoading" size="small" class="changes-diff-loading" />
                      <template v-else-if="hasDiffText">
                        <pre class="diff-output"><span v-for="(line, index) in diffLines" :key="index" class="diff-line" :class="diffLineClass(line)">{{ line }}</span></pre>
                        <div v-if="diffHunks.length" class="diff-hunk-actions">
                          <n-space v-for="(hunk, index) in diffHunks" :key="hunk.header + index" justify="space-between" align="center">
                            <n-text code>{{ hunk.header }}</n-text>
                            <n-button size="tiny" tertiary :loading="mutationBusy" @click="applyDiffHunk(hunk)">{{ selectedDiffStaged ? tr("取消暂存此块", "Unstage this hunk") : tr("暂存此块", "Stage this hunk") }}</n-button>
                          </n-space>
                        </div>
                      </template>
                      <n-empty v-else :description="selectedChange ? tr('此文件没有可显示的文本 diff', 'No text diff is available for this file') : tr('选择一个文件以查看 diff', 'Select a file to view its diff')" size="small" />
                    </div>
                  </n-card>
                </div>
  </template>
</template>
