<script lang="ts">
import { computed, defineComponent, h, nextTick, ref, watch } from "vue";
import { NButton, NDropdown, NLayoutHeader, NPopover, NScrollbar, NSelect, NSpace, NSpin, NTag, NText, type SelectRenderLabel } from "naive-ui";
import { useWorkspaceControllerContext } from "../composables/workspaceContext";
import { api, isAppError } from "../../../shared/api/tauri";
import type { RepositoryRecord } from "../../../shared/types/domain";
import { settingsSection } from "../../../shared/settingsNavigation";
import { filterRepositoryGroups } from "../composables/workspaceControllerSupport";

function splitReferenceLabel(label: string): { prefix: string; tail: string } {
  const separatorIndex = label.lastIndexOf("/");
  if (separatorIndex <= 0 || separatorIndex === label.length - 1) return { prefix: "", tail: label };
  return {
    prefix: label.slice(0, separatorIndex),
    tail: label.slice(separatorIndex + 1),
  };
}

function renderMiddleEllipsisLabel(label: string) {
  const { prefix, tail } = splitReferenceLabel(label);
  return h("span", { class: "middle-ellipsis-label", title: label }, [
    prefix ? h("span", { class: "middle-ellipsis-prefix" }, prefix) : null,
    prefix ? h("span", { class: "middle-ellipsis-separator", "aria-hidden": "true" }, "/") : null,
    h("span", { class: "middle-ellipsis-tail" }, tail),
  ]);
}

export default defineComponent({
  name: "WorkspaceHeader",
  components: { NButton, NDropdown, NLayoutHeader, NPopover, NScrollbar, NSelect, NSpace, NSpin, NTag, NText },
  setup() {
    const workspace = useWorkspaceControllerContext();
    const repositoryFilter = ref("");
    const repositoryPickerElement = ref<HTMLElement | null>(null);
    const repositoryPickerTrigger = ref<HTMLElement | null>(null);
    const repositorySearchInput = ref<HTMLInputElement | null>(null);
    const repositoryPickerOpen = ref(false);
    const repositoryContextMenuOpen = ref(false);
    const repositoryContextMenuX = ref(0);
    const repositoryContextMenuY = ref(0);
    const repositoryContextTarget = ref<RepositoryRecord | null>(null);
    const repositoryContextMenuOptions = computed(() => [{
      label: workspace.t('workspace.header.openInIde'),
      key: "repository-editors",
      children: [
        ...workspace.repositoryEditors.value.map((editor) => ({
          label: editor.name,
          key: `open-repository-editor:${editor.name}`,
        })),
        ...(workspace.repositoryEditors.value.length ? [{ type: "divider", key: "repository-editors-divider" }] : []),
        { label: workspace.t('workspace.header.manageIdes'), key: "manage-repository-editors" },
      ],
    },
      { label: workspace.t('workspace.header.openInSystemDefaultTerminal'), key: "open-system-terminal" },
      { label: workspace.t('workspace.header.openInPreferredTerminal'), key: "open-preferred-terminal" },
      { label: workspace.t('workspace.header.openInFileManager'), key: "open-file-manager" },
      { label: workspace.t('workspace.header.openRepositoryWebsite'), key: "open-remote" },
      { type: "divider", key: "open-divider" },
      { label: workspace.t('workspace.header.setRepositoryAlias'), key: "edit-alias" },
      { label: workspace.t('workspace.header.editRepositoryGroup'), key: "edit-group" },
    ]);
    const applicationMenuOptions = computed(() => [
      {
        label: `${workspace.section.value === "repositories" ? "✓ " : ""}${workspace.t('workspace.header.repositories')}`,
        key: "repositories",
      },
      {
        label: `${workspace.section.value === "accounts" ? "✓ " : ""}${workspace.t('workspace.header.accountsAndAuthentication')}`,
        key: "accounts",
      },
      { type: "divider", key: "settings-divider" },
      { label: workspace.t('workspace.header.settings'), key: "settings" },
    ]);
    const contributionMenuOptions = computed(() => [
      { label: workspace.t('workspace.header.newBranchAndSwitch'), key: "new-branch" },
      ...(workspace.activeRemote.value && workspace.activeCapabilities.value.repositoryFork && workspace.snapshot.value?.hasHead && workspace.snapshot.value.status.branch
        ? [{ label: workspace.t('workspace.header.forkAndPushCurrentBranch'), key: "fork-and-push" }]
        : []),
    ]);
    const branchSwitcherValue = computed(() => {
      const branch = workspace.snapshot.value?.status.branch;
      return branch ? `local:${branch}` : null;
    });
    const repositorySwitcherAriaLabel = computed(() => {
      const repository = workspace.snapshot.value?.repository;
      const label = repository?.alias
        ? `${workspace.t("workspace.header.alias")}: ${repository.alias} (${repository.name})`
        : repository?.name || workspace.t("workspace.header.repository");
      return `${workspace.t("workspace.header.switchRepository")}: ${label}`;
    });
    const displayedRepositoryGroups = computed(() =>
      filterRepositoryGroups(workspace.repositoryGroups.value, repositoryFilter.value));
    const currentBranchLabelParts = computed(() => splitReferenceLabel(workspace.currentBranch.value));
    const upstreamLabelParts = computed(() => splitReferenceLabel(workspace.snapshot.value?.status.upstream ?? ""));
    const branchTracksActiveRemote = computed(() => {
      const upstream = workspace.snapshot.value?.status.upstream;
      const remoteName = workspace.activeRemote.value?.name;
      return Boolean(upstream && remoteName && upstream.startsWith(`${remoteName}/`));
    });
    const pushNeedsAttention = computed(() => {
      const status = workspace.snapshot.value?.status;
      return Boolean(status?.branch && (!branchTracksActiveRemote.value || status.ahead > 0));
    });
    const pushButtonLabel = computed(() => {
      const status = workspace.snapshot.value?.status;
      if (status?.branch && !branchTracksActiveRemote.value) {
        return status.upstream
          ? workspace.t("workspace.header.pushToSelectedRemote")
          : workspace.t("workspace.header.pushFirst");
      }
      if (status && branchTracksActiveRemote.value && status.ahead > 0) {
        return workspace.t("workspace.header.pushPending", { count: status.ahead });
      }
      return workspace.t("workspace.header.push");
    });
    const pushButtonTitle = computed(() => {
      const status = workspace.snapshot.value?.status;
      const remote = workspace.activeRemote.value?.name || workspace.t("workspace.header.selectRemote");
      if (status?.branch && !branchTracksActiveRemote.value) {
        return status.upstream
          ? workspace.t("workspace.header.pushAdditionalToRemote", { remote, upstream: status.upstream })
          : workspace.t("workspace.header.pushFirstToRemote", { remote });
      }
      if (status && branchTracksActiveRemote.value && status.ahead > 0) {
        return workspace.t("workspace.header.pushPendingToRemote", { count: status.ahead, remote });
      }
      return workspace.t("workspace.header.pushToRemote", { remote });
    });
    const renderBranchLabel: SelectRenderLabel = (option, selected) => {
      const label = typeof option.label === "string" ? option.label : String(option.label ?? "");
      return selected ? renderMiddleEllipsisLabel(label) : label;
    };
    const branchSwitcherStyle = computed(() => {
      const branch = workspace.snapshot.value?.status.branch ?? "";
      const displayUnits = [...branch].reduce((total, character) =>
        total + (character.codePointAt(0)! > 0xff ? 2 : 1), 0);
      const width = Math.min(320, Math.max(148, 52 + displayUnits * 7.5));
      return { "--branch-select-content-width": `${width}px` };
    });
    const branchSwitcherOptions = computed(() => {
      const repositorySnapshot = workspace.snapshot.value;
      if (!repositorySnapshot) return [];
      const options = [];
      if (repositorySnapshot.branches.length) {
        options.push({
          type: "group",
          label: workspace.t('workspace.header.localBranches'),
          key: "local-branches",
          children: repositorySnapshot.branches.map((branch) => ({
            label: branch.name,
            value: `local:${branch.name}`,
          })),
        });
      }
      const remoteName = workspace.selectedRemoteName.value;
      const remoteBranches = repositorySnapshot.remoteBranches.filter((branch) => branch.remoteName === remoteName);
      if (remoteName && remoteBranches.length) {
        options.push({
          type: "group",
          label: workspace.t('workspace.header.remoteBranches', { remote: remoteName }),
          key: `remote-branches:${remoteName}`,
          children: remoteBranches.map((branch) => ({
            label: branch.name,
            value: `remote:${branch.name}`,
          })),
        });
      }
      return options;
    });

    function selectApplicationMenuItem(value: string | number): void {
      if (value === "repositories" || value === "accounts") {
        workspace.updateSection(value);
      } else if (value === "settings") {
        workspace.settingsModalShown.value = true;
      }
    }

    function toggleRepositoryPicker(event: MouseEvent): void {
      if (!repositoryPickerOpen.value && event.currentTarget instanceof HTMLElement) {
        repositoryPickerTrigger.value = event.currentTarget;
      }
      repositoryPickerOpen.value = !repositoryPickerOpen.value;
    }

    function selectContributionAction(value: string | number): void {
      if (value === "new-branch") {
        workspace.branchForm.name = "";
        workspace.branchForm.checkout = true;
        workspace.branchForm.stashLocalChanges = false;
        workspace.branchModalShown.value = true;
      } else if (value === "fork-and-push") {
        workspace.openForkModal();
      }
    }

    function selectBranch(value: string | number): void {
      if (typeof value !== "string") return;
      const repositorySnapshot = workspace.snapshot.value;
      if (!repositorySnapshot) return;
      if (value.startsWith("local:")) {
        const name = value.slice("local:".length);
        const branch = repositorySnapshot.branches.find((candidate) => candidate.name === name);
        if (branch && !branch.isCurrent) workspace.checkoutBranch(branch);
        return;
      }
      if (value.startsWith("remote:")) {
        const name = value.slice("remote:".length);
        const remoteBranch = repositorySnapshot.remoteBranches.find((candidate) =>
          candidate.remoteName === workspace.selectedRemoteName.value && candidate.name === name);
        if (remoteBranch) workspace.checkoutRemoteBranch(remoteBranch);
      }
    }

    function openRepositoryContextMenu(event: MouseEvent, repository: RepositoryRecord): void {
      event.preventDefault();
      showRepositoryContextMenu(event.currentTarget, event.clientX, event.clientY, repository);
    }

    function openRepositoryContextMenuFromKeyboard(event: KeyboardEvent, repository: RepositoryRecord): void {
      if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
      event.preventDefault();
      showRepositoryContextMenu(event.currentTarget, null, null, repository);
    }

    function showRepositoryContextMenu(
      eventTarget: EventTarget | null,
      clientX: number | null,
      clientY: number | null,
      repository: RepositoryRecord,
    ): void {
      repositoryContextMenuOpen.value = false;
      repositoryContextTarget.value = repository;
      const target = eventTarget instanceof HTMLElement ? eventTarget : null;
      const bounds = target?.getBoundingClientRect();
      repositoryContextMenuX.value = clientX ?? (bounds ? bounds.left + bounds.width / 2 : 0);
      repositoryContextMenuY.value = clientY ?? (bounds ? bounds.top + bounds.height / 2 : 0);
      void nextTick(() => {
        repositoryContextMenuOpen.value = true;
      });
    }

    async function openRemoteRepository(repository: RepositoryRecord): Promise<void> {
      let url: string | null;
      try {
        url = await api.getRepositoryRemoteWebUrl(repository.path);
      } catch (error) {
        workspace.reportError(error, workspace.t("workspace.controller.couldNotLoadGitServiceRepositoryInformation"));
        return;
      }
      if (!url) {
        workspace.message.warning(workspace.t("workspace.header.remoteRepositoryWebsiteUnavailable"));
        return;
      }
      await workspace.openCodeHostUrl(url);
    }

    async function selectRepositoryContextMenuAction(value: string | number): Promise<void> {
      repositoryContextMenuOpen.value = false;
      const repository = repositoryContextTarget.value;
      if (!repository) return;
      if (value === "manage-repository-editors") {
        repositoryPickerOpen.value = false;
        workspace.openRepositoryEditorsModal();
      } else if (typeof value === "string" && value.startsWith("open-repository-editor:")) {
        repositoryPickerOpen.value = false;
        const editorName = value.slice("open-repository-editor:".length);
        const editor = workspace.repositoryEditors.value.find((candidate) => candidate.name === editorName);
        if (!editor) return;
        try {
          await api.openRepositoryInEditor(repository.path, editor.name);
        } catch (error) {
          if (isAppError(error) && error.code === "repository_editor_not_found") {
            if (await workspace.loadRepositoryEditors()) {
              workspace.message.warning(workspace.t("workspace.controller.repositoryEditorListChanged"));
            }
            return;
          }
          workspace.reportError(error, workspace.t("workspace.controller.couldNotOpenRepositoryInIde"));
        }
      } else if (value === "open-system-terminal") {
        repositoryPickerOpen.value = false;
        try {
          await api.openRepositoryInTerminal(repository.path);
        } catch (error) {
          workspace.reportError(error, workspace.t("workspace.controller.couldNotOpenRepositoryInSystemTerminal"));
        }
      } else if (value === "open-preferred-terminal") {
        repositoryPickerOpen.value = false;
        try {
          await api.openRepositoryInPreferredTerminal(repository.path);
        } catch (error) {
          if (isAppError(error) && error.code === "preferred_terminal_not_configured") {
            settingsSection.value = "general";
            workspace.settingsModalShown.value = true;
            workspace.message.warning(workspace.t("workspace.controller.configureAPreferredTerminalFirst"));
            return;
          }
          workspace.reportError(error, workspace.t("workspace.controller.couldNotOpenRepositoryInPreferredTerminal"));
        }
      } else if (value === "open-file-manager") {
        repositoryPickerOpen.value = false;
        try {
          await api.openRepositoryInFileManager(repository.path);
        } catch (error) {
          workspace.reportError(error, workspace.t("workspace.controller.couldNotOpenRepositoryInFileManager"));
        }
      } else if (value === "open-remote") {
        repositoryPickerOpen.value = false;
        await openRemoteRepository(repository);
      } else if (value === "edit-alias") {
        repositoryPickerOpen.value = false;
        workspace.openRepositoryAliasEditor(repository);
      } else if (value === "edit-group") {
        repositoryPickerOpen.value = false;
        workspace.openRepositoryGroupEditor(repository);
      }
    }

    watch(workspace.section, (section) => {
      if (section !== "repositories") {
        repositoryPickerOpen.value = false;
        repositoryContextMenuOpen.value = false;
      }
    });
    watch(repositoryPickerOpen, (open) => {
      if (open) {
        void nextTick(() => repositorySearchInput.value?.focus());
      } else {
        const shouldRestoreFocus = repositoryPickerElement.value?.contains(document.activeElement) ?? false;
        repositoryContextMenuOpen.value = false;
        repositoryFilter.value = "";
        if (shouldRestoreFocus) {
          void nextTick(() => repositoryPickerTrigger.value?.focus());
        }
      }
    });
    return {
      ...workspace,
      repositoryFilter,
      repositoryPickerElement,
      repositorySearchInput,
      displayedRepositoryGroups,
      repositoryPickerOpen,
      repositoryContextMenuOpen,
      repositoryContextMenuX,
      repositoryContextMenuY,
      repositoryContextMenuOptions,
      applicationMenuOptions,
      contributionMenuOptions,
      branchSwitcherOptions,
      branchSwitcherStyle,
      branchSwitcherValue,
      repositorySwitcherAriaLabel,
      currentBranchLabelParts,
      upstreamLabelParts,
      branchTracksActiveRemote,
      pushNeedsAttention,
      pushButtonLabel,
      pushButtonTitle,
      renderBranchLabel,
      openRepositoryContextMenu,
      openRepositoryContextMenuFromKeyboard,
      selectApplicationMenuItem,
      toggleRepositoryPicker,
      selectContributionAction,
      selectBranch,
      selectRepositoryContextMenuAction,
    };
  },
});
</script>

<template>
      <n-layout-header bordered class="topbar">
        <div v-if="section === 'repositories'" class="topbar-content repository-topbar-content">
          <div class="topbar-leading">
            <div class="application-menu">
              <n-dropdown trigger="click" placement="bottom-start" :options="applicationMenuOptions" @select="selectApplicationMenuItem">
                <button class="application-menu-button" type="button" :aria-label="t('workspace.header.openApplicationMenu')" :title="t('workspace.header.openApplicationMenu')">
                  <span v-for="line in 4" :key="line" class="application-menu-line" />
                </button>
              </n-dropdown>
            </div>
            <div class="repository-topbar-context">
              <div class="repository-title-row">
              <n-popover
                v-model:show="repositoryPickerOpen"
                trigger="click"
                placement="bottom-start"
                :show-arrow="false"
                :overlap="false"
                class="repository-picker-popover"
              >
                <template #trigger>
                  <button
                    class="repository-switcher"
                    type="button"
                    :aria-label="repositorySwitcherAriaLabel"
                    :aria-expanded="repositoryPickerOpen"
                    aria-haspopup="dialog"
                    aria-keyshortcuts="Shift+F10"
                    :aria-description="t('workspace.header.rightClickForRepositoryActions')"
                    :title="snapshot?.repository.path"
                    @click.stop="toggleRepositoryPicker($event)"
                    @contextmenu="snapshot?.repository && openRepositoryContextMenu($event, snapshot.repository)"
                    @keydown="snapshot?.repository && openRepositoryContextMenuFromKeyboard($event, snapshot.repository)"
                  >
                    <span class="repository-switcher-primary">{{ snapshot?.repository.alias || snapshot?.repository.name || t('workspace.header.repository') }}</span>
                    <span v-if="snapshot?.repository.alias" class="repository-switcher-secondary">{{ snapshot.repository.name }}</span>
                    <span class="repository-switcher-caret" aria-hidden="true">▾</span>
                  </button>
                </template>

                <aside
                  ref="repositoryPickerElement"
                  class="repository-picker repository-picker-searchable"
                  :aria-label="t('workspace.header.localRepositories')"
                  role="dialog"
                >
                  <div class="repository-picker-command-header">
                    <div class="repository-picker-command-row">
                      <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5" /><path d="m15.5 15.5 4 4" /></svg>
                      <input
                        ref="repositorySearchInput"
                        v-model="repositoryFilter"
                        class="repository-picker-search"
                        type="search"
                        :aria-label="t('workspace.header.localRepositories')"
                        :placeholder="`${t('workspace.header.localRepositories')}…`"
                      >
                    </div>
                    <div class="repository-picker-command-tools">
                      <span>{{ t('workspace.header.switchRepository') }}</span>
                      <n-select v-model:value="repositoryGrouping" size="tiny" :options="repositoryGroupingOptions" class="repository-grouping-select" />
                    </div>
                  </div>

                  <div class="repository-picker-repositories">
                    <n-spin :show="repositoriesLoading" size="small">
                      <n-scrollbar
                        v-if="displayedRepositoryGroups.length"
                        class="sidebar-repository-list"
                        content-class="sidebar-repository-list-content"
                      >
                        <div v-for="group in displayedRepositoryGroups" :key="group.key" class="sidebar-repository-group">
                          <div
                            class="sidebar-group-heading"
                            :class="{ 'sidebar-group-heading-owner': group.key.startsWith('owner:') }"
                          >
                            <span>{{ group.label }}</span>
                            <span>{{ group.repositories.length }}</span>
                          </div>
                          <div class="sidebar-repository-group-list">
                            <div v-for="repository in group.repositories" :key="repository.path" class="sidebar-repository-item">
                              <button
                                class="sidebar-repository"
                                :class="{ active: repository.path === activeRepositoryPath }"
                                type="button"
                                :aria-current="repository.path === activeRepositoryPath ? 'true' : undefined"
                                aria-keyshortcuts="Shift+F10"
                                :aria-description="t('workspace.header.rightClickForRepositoryActions')"
                                :title="`${repository.path} · ${t('workspace.header.rightClickForRepositoryActions')}`"
                                @click="repositoryPickerOpen = false; selectRepository(repository.path)"
                                @contextmenu="openRepositoryContextMenu($event, repository)"
                                @keydown="openRepositoryContextMenuFromKeyboard($event, repository)"
                              >
                                <template v-if="repository.alias">
                                  <span class="visually-hidden">{{ t('workspace.header.alias') }}: </span>
                                  <span class="repository-alias-indicator" aria-hidden="true" />
                                  <span class="sidebar-repository-copy">
                                    <span class="repository-name repository-alias-name">{{ repository.alias }}</span>
                                    <span class="repository-canonical-name">{{ repository.name }}</span>
                                  </span>
                                </template>
                                <span v-else class="repository-name">{{ repository.name }}</span>
                              </button>
                            </div>
                          </div>
                        </div>
                      </n-scrollbar>
                      <n-text v-else depth="3" class="sidebar-empty">
                        {{ repositoryFilter.trim() ? t('workspace.header.noRepositoriesMatchSearch') : t('workspace.header.noRepositoriesAdded') }}
                      </n-text>
                    </n-spin>
                  </div>

                  <div class="repository-picker-footer">
                    <n-dropdown :options="repositoryActionOptions" @select="(value) => { repositoryPickerOpen = false; openRepositoryAction(value); }">
                      <n-button size="small" block class="repository-add-button">
                        <svg class="repository-add-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
                        {{ t('workspace.header.addRepository') }}
                      </n-button>
                    </n-dropdown>
                  </div>
                </aside>
              </n-popover>
              <n-dropdown
                v-model:show="repositoryContextMenuOpen"
                trigger="manual"
                placement="bottom-start"
                :x="repositoryContextMenuX"
                :y="repositoryContextMenuY"
                :options="repositoryContextMenuOptions"
                @clickoutside="repositoryContextMenuOpen = false"
                @select="(value) => void selectRepositoryContextMenuAction(value)"
              />
              <template v-if="snapshot">
                <n-tag v-if="!snapshot.branches.length" size="small" type="primary" class="branch-status-tag" :title="currentBranch">
                  <span class="middle-ellipsis-label">
                    <span v-if="currentBranchLabelParts.prefix" class="middle-ellipsis-prefix">{{ currentBranchLabelParts.prefix }}</span>
                    <span v-if="currentBranchLabelParts.prefix" class="middle-ellipsis-separator" aria-hidden="true">/</span>
                    <span class="middle-ellipsis-tail">{{ currentBranchLabelParts.tail }}</span>
                  </span>
                </n-tag>
                <n-tag v-if="isUnbornRepository" size="small" type="warning" :bordered="false">{{ t('workspace.header.noCommitsYet') }}</n-tag>
                <n-tag v-else-if="snapshot.status.upstream && branchTracksActiveRemote" size="small" :bordered="false" class="branch-status-tag" :title="snapshot.status.upstream">
                  <span class="middle-ellipsis-label">
                    <span v-if="upstreamLabelParts.prefix" class="middle-ellipsis-prefix">{{ upstreamLabelParts.prefix }}</span>
                    <span v-if="upstreamLabelParts.prefix" class="middle-ellipsis-separator" aria-hidden="true">/</span>
                    <span class="middle-ellipsis-tail">{{ upstreamLabelParts.tail }}</span>
                  </span>
                </n-tag>
                <n-tag
                  v-if="snapshot.status.branch && !branchTracksActiveRemote"
                  size="small"
                  type="warning"
                  :bordered="false"
                  class="branch-status-tag"
                  :title="snapshot.status.upstream ? pushButtonTitle : t('workspace.header.branchOnlyLocalTitle')"
                >
                  {{ snapshot.status.upstream ? t('workspace.header.remoteNotTracked') : t('workspace.header.branchOnlyLocal') }}
                </n-tag>
                <n-text
                  v-if="branchTracksActiveRemote && (snapshot.status.ahead || snapshot.status.behind)"
                  depth="3"
                  class="sync-summary"
                  :class="{ 'sync-summary-behind': snapshot.status.behind > 0 }"
                  :title="snapshot.status.upstream"
                  :aria-label="t('workspace.header.syncSummary', {
                    ahead: snapshot.status.ahead,
                    behind: snapshot.status.behind,
                  })"
                  aria-live="polite"
                >
                  <span aria-hidden="true">↑ {{ snapshot.status.ahead }} · ↓ {{ snapshot.status.behind }}</span>
                </n-text>
              </template>
              </div>
            </div>
          </div>
          <n-space align="center" :size="8" :wrap="false" class="topbar-actions">
            <div v-if="snapshot?.remotes.length || snapshot?.branches.length" class="topbar-reference-group">
              <div v-if="snapshot?.remotes.length" class="topbar-select-group">
                <n-select
                  v-model:value="selectedRemoteName"
                  size="small"
                  :disabled="mutationBusy || Boolean(networkBusy)"
                  :options="snapshot.remotes.map((remote) => ({ label: remote.name, value: remote.name }))"
                  :placeholder="t('workspace.header.selectRemote')"
                  :aria-label="t('workspace.header.selectRemote')"
                  :title="t('workspace.header.selectRemote')"
                  class="remote-select"
                />
              </div>
              <span v-if="snapshot?.remotes.length && snapshot.branches.length" class="topbar-reference-separator" aria-hidden="true">/</span>
              <div v-if="snapshot?.branches.length" class="topbar-select-group">
                <n-select
                  :value="branchSwitcherValue"
                  size="small"
                  :options="branchSwitcherOptions"
                  :render-label="renderBranchLabel"
                  :placeholder="t('workspace.header.selectBranch')"
                  :disabled="mutationBusy || Boolean(networkBusy)"
                  :consistent-menu-width="false"
                  :menu-props="{ class: 'branch-select-menu' }"
                  :aria-label="t('workspace.header.switchBranch')"
                  :title="snapshot.status.branch || t('workspace.header.switchBranch')"
                  :style="branchSwitcherStyle"
                  class="branch-select"
                  @update:value="selectBranch"
                />
              </div>
            </div>
            <div v-if="(snapshot && !snapshot.remotes.length && publishEndpoints.length) || activeRemote" class="topbar-action-group">
              <n-button
                v-if="snapshot && !snapshot.remotes.length && publishEndpoints.length"
                size="small"
                type="primary"
                :disabled="mutationBusy || Boolean(networkBusy)"
                @click="openPublishModal"
              >
                {{ t('workspace.header.publishToGithub') }}
              </n-button>
              <n-button
                v-if="activeRemote"
                size="small"
                :loading="networkBusy === 'fetch'"
                :disabled="mutationBusy || Boolean(networkBusy)"
                :aria-label="t('repository.branches.fetchRemote', { remote: activeRemote.name })"
                :title="t('repository.branches.fetchRemote', { remote: activeRemote.name })"
                @click="runNetworkAction('fetch')"
              >
                {{ t('repository.branches.fetch') }}
              </n-button>
              <n-button v-if="activeRemote" size="small" :loading="networkBusy === 'pull'" :disabled="mutationBusy || !canPullFromActiveRemote || Boolean(networkBusy)" @click="runNetworkAction('pull')">{{ t('workspace.header.pull') }}</n-button>
              <n-button
                v-if="activeRemote"
                size="small"
                :type="pushNeedsAttention ? 'warning' : 'primary'"
                :loading="networkBusy === 'push'"
                :disabled="mutationBusy || !snapshot?.hasHead || !snapshot.status.branch || Boolean(networkBusy)"
                :aria-label="pushButtonTitle"
                :title="pushButtonTitle"
                @click="runNetworkAction('push')"
              >
                {{ pushButtonLabel }}
              </n-button>
            </div>
            <div v-if="snapshot" class="topbar-utility-group">
              <n-button
                v-if="repositoryWebUrl"
                size="small"
                tertiary
                :title="t('workspace.header.openRepositoryWebsite')"
                @click="openCodeHostUrl(repositoryWebUrl)"
              >
                {{ t('workspace.header.openRepositoryWebsite') }}
              </n-button>
              <n-dropdown
                trigger="click"
                placement="bottom-end"
                :options="contributionMenuOptions"
                @select="selectContributionAction"
              >
                <n-button size="small" tertiary :disabled="mutationBusy || Boolean(networkBusy)">
                  {{ t('workspace.header.contribute') }}
                </n-button>
              </n-dropdown>
              <n-button
                size="small"
                tertiary
                :loading="snapshotLoading"
                :disabled="mutationBusy || Boolean(networkBusy)"
                :aria-label="t('repository.history.refresh')"
                :title="t('repository.history.refresh')"
                @click="refreshSnapshot"
              >
                {{ t('repository.history.refresh') }}
              </n-button>
              <n-button v-if="activeRemote" size="small" tertiary @click="activeTab = 'branches'">
                {{ t('workspace.header.repositorySettings') }}
              </n-button>
            </div>
          </n-space>
        </div>
        <div v-else class="topbar-content">
          <div class="topbar-leading">
            <div class="application-menu">
              <n-dropdown trigger="click" placement="bottom-start" :options="applicationMenuOptions" @select="selectApplicationMenuItem">
                <button class="application-menu-button" type="button" :aria-label="t('workspace.header.openApplicationMenu')" :title="t('workspace.header.openApplicationMenu')">
                  <span v-for="line in 4" :key="line" class="application-menu-line" />
                </button>
              </n-dropdown>
            </div>
            <div class="topbar-title">{{ t('workspace.header.accountsAndAuthentication') }}</div>
          </div>
        </div>
      </n-layout-header>
</template>

<style src="./WorkspaceHeader.css"></style>
