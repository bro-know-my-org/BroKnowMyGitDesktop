<script lang="ts">
import { computed, defineComponent } from "vue";
import { NAlert, NButton, NCard, NCheckbox, NEmpty, NInput, NSpace, NSpin, NText } from "naive-ui";
import RepositoryFileDiff from "../../../features/repository/components/RepositoryFileDiff.vue";
import RepositoryDiffViewer from "../../../features/repository/components/RepositoryDiffViewer.vue";
import { useRepositoryPanelSplit } from "../../../features/repository/composables/useRepositoryPanelSplit";
import { useWorkspaceControllerContext } from "../../../features/workspace/composables/workspaceContext";
import { resolvedInterfaceFontSize } from "../../../shared/fonts";
import { t } from "../../../shared/i18n";
import FixedVirtualList from "../../../shared/virtual-list/FixedVirtualList.vue";

export default defineComponent({
  name: "RepositoryChangesPanel",
  components: { FixedVirtualList, NAlert, NButton, NCard, NCheckbox, NEmpty, NInput, NSpace, NSpin, NText, RepositoryDiffViewer, RepositoryFileDiff },
  setup() {
    const workspace = useWorkspaceControllerContext();
    return {
      ...workspace,
      ...useRepositoryPanelSplit(),
      changeFileRowHeight: computed(() => Math.ceil((25.3 * resolvedInterfaceFontSize.value / 14) + 3)),
      changeFileVirtualKeys: computed(() => workspace.snapshot.value?.status.entries.map((entry) => entry.path) || []),
      t,
    };
  },
});
</script>

<template>
  <template v-if="snapshot">
<n-alert v-if="isUnbornRepository" type="info" :bordered="false" class="unborn-repository-alert">
                  {{ t('repository.changes.newRepositoryHelp') }}
                </n-alert>
                <div class="changes-workspace repository-split-layout" :style="repositoryPanelStyle">
                  <div class="changes-list-pane">
                    <n-card size="small" :title="t('repository.changes.changedFiles')" class="changes-file-card">
                      <template #header-extra>
                        <n-space size="small">
                          <n-button v-if="untrackedCount" size="tiny" text type="warning" :loading="mutationBusy" @click="confirmCleanUntracked">{{ t('repository.changes.cleanUntracked') }}</n-button>
                          <n-button size="tiny" text type="error" :disabled="!snapshot.status.entries.length" :loading="mutationBusy" @click="confirmDiscardChanges(snapshot.status.entries.map((entry) => entry.path))">{{ t('repository.changes.discardAll') }}</n-button>
                        </n-space>
                      </template>
                      <div v-if="snapshot.status.entries.length" class="changes-bulk-control">
                        <n-checkbox
                          size="small"
                          :checked="allChangesIncluded"
                          :indeterminate="someChangesIncluded"
                          :disabled="mutationBusy"
                          @update:checked="updateAllChangeInclusion"
                        >{{ t('repository.changes.includeAllChanges') }}</n-checkbox>
                      </div>
                      <FixedVirtualList
                        v-if="snapshot.status.entries.length"
                        class="changes-file-list"
                        :item-count="snapshot.status.entries.length"
                        :item-height="changeFileRowHeight"
                        :item-keys="changeFileVirtualKeys"
                        :overscan-viewports="3"
                        :reset-key="activeRepositoryPath"
                        :aria-label="t('repository.changes.changedFilesList')"
                      >
                        <template #default="{ index, tabIndex }">
                        <div
                          class="change-file-row"
                          data-virtual-list-focus
                          :class="{ selected: selectedChange?.path === snapshot.status.entries[index].path }"
                          role="button"
                          :tabindex="tabIndex"
                          @click="loadDiff(snapshot.status.entries[index])"
                          @keydown.enter.self.prevent="loadDiff(snapshot.status.entries[index])"
                          @keydown.space.self.prevent="loadDiff(snapshot.status.entries[index])"
                        >
                          <span class="change-file-checkbox" @click.stop>
                            <n-checkbox
                              size="small"
                              :checked="isChangeFullyIncluded(snapshot.status.entries[index])"
                              :indeterminate="isChangePartiallyIncluded(snapshot.status.entries[index])"
                              :disabled="mutationBusy"
                              :aria-label="changeIncludeLabel(snapshot.status.entries[index])"
                              @update:checked="updateChangeInclusion(snapshot.status.entries[index], $event)"
                            />
                          </span>
                          <span class="change-file-copy">
                            <span class="change-file-path" :title="snapshot.status.entries[index].path">{{ snapshot.status.entries[index].path }}</span>
                            <span v-if="changeStatusLabel(snapshot.status.entries[index])" class="change-file-state">{{ changeStatusLabel(snapshot.status.entries[index]) }}</span>
                          </span>
                          <span class="change-file-status" :class="`is-${changeStatusTagType(snapshot.status.entries[index])}`" aria-hidden="true" />
                        </div>
                        </template>
                      </FixedVirtualList>
                      <n-empty v-if="!snapshot.status.entries.length" :description="t('repository.changes.workingTreeClean')" :show-icon="false" />
                    </n-card>

                    <n-card size="small" :title="t('repository.changes.createCommit')" class="commit-card changes-commit-card">
                      <div class="commit-form">
                        <label for="commit-message" class="commit-message-label">{{ t('repository.changes.commitMessage') }}</label>
                        <n-input
                          v-model:value="commitMessage"
                          type="textarea"
                          :autosize="{ minRows: 3, maxRows: 5 }"
                          :input-props="{ id: 'commit-message' }"
                          :placeholder="t('repository.changes.commitMessagePlaceholder')"
                        />
                        <n-checkbox v-if="canAmendLatestCommit" class="commit-amend-checkbox" :checked="amendLastCommit" @update:checked="setAmendLastCommit">{{ t('repository.changes.amendLatestCommit') }}</n-checkbox>
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
                    :aria-label="t('repository.changes.resizeFileList')"
                    :aria-valuemin="240"
                    :aria-valuenow="repositoryPanelAriaValue"
                    :aria-valuemax="repositoryPanelAriaMax"
                    :aria-valuetext="t('repository.changes.widthPixels', { width: repositoryPanelAriaValue })"
                    :title="t('repository.changes.dragResizeDoubleClickRestoreDefault')"
                    @pointerdown="startRepositoryPanelResize"
                    @keydown="resizeRepositoryPanelWithKeyboard"
                    @dblclick="resetRememberedRepositoryPanelWidth"
                  />

                  <n-card size="small" :title="selectedChange ? selectedChange.path : t('repository.changes.diffPreview')" class="diff-card changes-diff-card">
                    <template #header-extra>
                      <n-space v-if="selectedChange" size="small">
                        <n-button v-if="selectedChange.worktreeStatus" size="tiny" :type="selectedDiffStaged ? 'default' : 'primary'" @click="loadDiff(selectedChange, activeRepositoryPath, true, false)">{{ t('repository.resource.worktree') }}</n-button>
                        <n-button v-if="selectedChange.indexStatus" size="tiny" :type="selectedDiffStaged ? 'primary' : 'default'" @click="loadDiff(selectedChange, activeRepositoryPath, true, true)">{{ t('repository.resource.index') }}</n-button>
                        <n-button size="tiny" tertiary :loading="blameLoading" :disabled="!snapshot.hasHead || (!selectedChange.indexStatus && selectedChange.worktreeStatus === '?')" @click="openFileBlame">{{ t('repository.changes.viewBlame') }}</n-button>
                        <n-button size="tiny" text type="error" :loading="mutationBusy" @click="discardSelectedChange">{{ t('repository.changes.discardFile') }}</n-button>
                      </n-space>
                    </template>
                    <div class="changes-diff-panel">
                      <RepositoryFileDiff :repository-path="activeRepositoryPath" :comparison="selectedChange ? { path: selectedChange.path, previousPath: selectedDiffStaged || ['R', 'C'].includes(selectedChange.worktreeStatus) ? selectedChange.originalPath : null, staged: selectedDiffStaged } : null" :diff-text="diffText" :loading="diffLoading">
                      <n-spin v-if="diffLoading" size="small" class="changes-diff-loading" />
                      <template v-else-if="hasDiffText">
                        <RepositoryDiffViewer
                          :lines="diffLines"
                          :line-class="diffLineClass"
                          :file-path="selectedChange?.path || ''"
                          :reset-key="`${selectedChange?.path || ''}\u0000${selectedDiffStaged}`"
                          :label="t('repository.changes.currentFileDiff')"
                        >
                          <template #line-action="{ index }">
                            <n-button
                              v-if="diffHunkAtLine(index)"
                              size="tiny"
                              tertiary
                              :loading="mutationBusy"
                              @click="applyDiffHunkAtLine(index)"
                            >{{ selectedDiffStaged ? t('repository.changes.unstageHunk') : t('repository.changes.stageHunk') }}</n-button>
                          </template>
                        </RepositoryDiffViewer>
                      </template>
                      <n-empty v-else :description="selectedChange ? t('repository.changes.noTextDiffAvailableFile') : t('repository.changes.selectFileViewItsDiff')" size="small" :show-icon="false" />
                      </RepositoryFileDiff>
                    </div>
                  </n-card>
                </div>
  </template>
</template>

<style src="./ChangesPanel.css"></style>
