<script lang="ts">
import { computed, defineComponent, h, nextTick, ref, watch } from "vue";
import { NButton, NButtonGroup, NDropdown, NLayoutHeader, NPopover, NVirtualList, NSelect, NSpin, NText, type SelectRenderLabel } from "naive-ui";
import { useWorkspaceControllerContext } from "../composables/workspaceContext";
import { api, isAppError } from "../../../shared/api/tauri";
import type { RepositoryRecord } from "../../../shared/types/domain";
import { writeClipboardText } from "../../../shared/clipboard";
import { settingsSection } from "../../../shared/settingsNavigation";
import { filterRepositoryGroups, repositoryRemoteLabel, repositoryStarAriaLabel, type RepositorySidebarGroup } from "../composables/workspaceControllerSupport";
import { useRepositoryPickerPreferences } from "../composables/useRepositoryPickerPreferences";

type PickerRow = { key: string; kind: "group"; group: RepositorySidebarGroup }
  | { key: string; kind: "repository"; repository: RepositoryRecord };

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
  props: { customTitlebar: Boolean },
  components: { NButton, NButtonGroup, NDropdown, NLayoutHeader, NPopover, NVirtualList, NSelect, NSpin, NText },
  setup() {
    const workspace = useWorkspaceControllerContext();
    const repositoryFilter = ref("");
    const pickerPreferences = useRepositoryPickerPreferences(workspace.repositoryGroups);
    const repositoryPickerElement = ref<HTMLElement | null>(null);
    const repositoryVirtualList = ref<InstanceType<typeof NVirtualList> | null>(null);
    const repositoryPickerTrigger = ref<HTMLElement | null>(null);
    const repositorySearchInput = ref<HTMLInputElement | null>(null);
    const repositoryPickerOpen = ref(false);
    const branchPickerOpen = ref(false);
    const repositoryContextMenuOpen = ref(false);
    const repositoryContextMenuX = ref(0);
    const repositoryContextMenuY = ref(0);
    const repositoryContextTarget = ref<RepositoryRecord | null>(null);
    const repositoryContextMenuOptions = computed(() => [
      { label: workspace.t(pickerPreferences.isStarred(repositoryContextTarget.value?.path ?? "")
        ? "workspace.header.unstarRepository" : "workspace.header.starRepository"), key: "toggle-star" },
      { label: workspace.t(pickerPreferences.isHidden(repositoryContextTarget.value?.path ?? "")
        ? "workspace.header.unhideRepository" : "workspace.header.hideRepository"), key: "toggle-hidden" },
      { type: "divider", key: "repository-preferences-divider" },
      {
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
      { label: workspace.t("workspace.header.copyAbsolutePath"), key: "copy-absolute-path" },
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
    const utilityMenuOptions = computed(() => [
      ...(workspace.activeRemote.value && workspace.activeCapabilities.value.repositoryFork && workspace.snapshot.value?.hasHead && workspace.snapshot.value.status.branch
        ? [{
          label: workspace.t('workspace.header.forkAndPushCurrentBranch'), key: "fork-and-push",
          disabled: workspace.mutationBusy.value || Boolean(workspace.networkBusy.value),
        }]
        : []),
      ...(workspace.repositoryWebUrl.value
        ? [{ label: workspace.t('workspace.header.openRepositoryWebsite'), key: "open-website" }]
        : []),
      {
        label: workspace.t('repository.history.refresh'),
        key: "refresh",
        disabled: workspace.mutationBusy.value || Boolean(workspace.networkBusy.value) || workspace.snapshotLoading.value,
        ...(workspace.snapshotLoading.value ? { icon: () => h(NSpin, { size: 14 }) } : {}),
      },
      ...(workspace.activeRemote.value
        ? [{ label: workspace.t('workspace.header.repositorySettings'), key: "repository-settings" }]
        : []),
    ]);
    const branchSwitcherValue = computed(() => {
      const branch = workspace.snapshot.value?.status.branch;
      return workspace.snapshot.value?.branches.some((candidate) => candidate.name === branch) ? `local:${branch}` : null;
    });
    const repositorySwitcherAriaLabel = computed(() => {
      const repository = workspace.snapshot.value?.repository;
      const label = repository?.alias
        ? `${workspace.t("workspace.header.alias")}: ${repository.alias} (${repository.name})`
        : repository?.name || workspace.t("workspace.header.repository");
      return `${workspace.t("workspace.header.switchRepository")}: ${label}`;
    });
    const displayedRepositoryGroups = computed(() =>
      filterRepositoryGroups(pickerPreferences.pickerGroups.value, repositoryFilter.value));
    const isRepositoryGroupCollapsed = (key: string) =>
      !repositoryFilter.value.trim() && pickerPreferences.isCollapsed(key);
    const repositoryPickerRows = computed<PickerRow[]>(() => displayedRepositoryGroups.value.flatMap((group) => [
      { key: `group:${group.key}`, kind: "group", group } as PickerRow,
      ...(isRepositoryGroupCollapsed(group.key) ? [] : group.repositories.map((repository): PickerRow =>
        ({ key: `repository:${repository.path}`, kind: "repository", repository }))),
    ]));

    async function revealRepositoryPickerRow(index: number): Promise<void> {
      await nextTick();
      repositoryVirtualList.value?.scrollTo({ index, debounce: true });
      // Native scroll events update the virtual window after Vue's current tick.
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      await nextTick();
    }

    async function navigateRepositoryPicker(event: KeyboardEvent): Promise<void> {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || !(event.target instanceof HTMLElement)) return;
      const row = event.target.closest<HTMLElement>("[data-picker-index]");
      if (!row) return;
      const index = Number(row.dataset.pickerIndex);
      let next = event.key === "ArrowDown" ? index + 1 : event.key === "ArrowUp" ? index - 1
        : event.key === "Home" ? 0 : event.key === "End" ? repositoryPickerRows.value.length - 1 : null;
      if (next === null) return;
      event.preventDefault();
      const step = event.key === "ArrowUp" || event.key === "End" ? -1 : 1;
      while (repositoryPickerRows.value[next]?.kind === "group" && repositoryFilter.value.trim()) next += step;
      if (next < 0 || next >= repositoryPickerRows.value.length) return;
      await revealRepositoryPickerRow(next);
      if (!repositoryPickerOpen.value) return;
      repositoryPickerElement.value?.querySelector<HTMLButtonElement>(`[data-picker-index="${next}"] button:not(:disabled)`)?.focus();
    }
    function repositoryTitle(repository: RepositoryRecord): string {
      const remote = repositoryRemoteLabel(repository);
      return remote ? `${repository.path}\n${workspace.t("workspace.header.remoteRepository")}: ${[repository.remoteName, repository.remoteHost, remote].filter(Boolean).join(" · ")}` : repository.path;
    }
    async function toggleRepositoryStar(path: string): Promise<void> {
      const restoreFocus = repositoryPickerElement.value?.contains(document.activeElement);
      pickerPreferences.toggleStar(path);
      if (!restoreFocus) return;
      const index = repositoryPickerRows.value.findIndex((row) => row.kind === "repository" && row.repository.path === path);
      if (index >= 0) await revealRepositoryPickerRow(index);
      else {
        await nextTick();
        // Pointer activation can remove the row before native focus settles.
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      }
      if (!repositoryPickerOpen.value) return;
      const target = [...(repositoryPickerElement.value?.querySelectorAll<HTMLButtonElement>(".repository-star-button") ?? [])]
        .find((button) => button.dataset.repositoryPath === path && button.getClientRects().length);
      (target ?? repositorySearchInput.value)?.focus();
    }
    function focusRepositoryPickerControl(): void {
      const target = repositoryPickerOpen.value ? repositorySearchInput.value
        : document.querySelector<HTMLButtonElement>(".repository-switcher");
      target?.focus();
    }
    const branchTracksActiveRemote = computed(() => {
      const upstream = workspace.snapshot.value?.status.upstream;
      const remoteName = workspace.activeRemote.value?.name;
      return Boolean(upstream && remoteName && upstream.startsWith(`${remoteName}/`));
    });
    const pushNeedsAttention = computed(() => {
      const status = workspace.snapshot.value?.status;
      return Boolean(workspace.snapshot.value?.hasHead && status?.branch && (!branchTracksActiveRemote.value || status.ahead > 0));
    });
    const branchStatusLabel = computed(() => {
      const snapshot = workspace.snapshot.value;
      if (!snapshot) return "";
      if (!snapshot.hasHead) return workspace.t("workspace.header.noCommitsYet");
      if (!snapshot.status.branch) return workspace.t("workspace.tables.detachedHead");
      if (!snapshot.status.upstream) return workspace.t("workspace.header.branchOnlyLocal");
      if (workspace.activeRemote.value && !branchTracksActiveRemote.value) return workspace.t("workspace.header.remoteNotTracked");
      return "";
    });
    const pullCount = computed(() => branchTracksActiveRemote.value ? workspace.snapshot.value?.status.behind ?? 0 : 0);
    const pushCount = computed(() => branchTracksActiveRemote.value ? workspace.snapshot.value?.status.ahead ?? 0 : 0);
    const pullButtonTitle = computed(() => workspace.t("workspace.header.pullFromRemote", {
      remote: workspace.activeRemote.value?.name ?? "",
      count: pullCount.value,
    }));
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
      const width = Math.min(420, Math.max(240, 52 + displayUnits * 7.5));
      return { "--branch-select-content-width": `${width}px` };
    });
    const branchSwitcherOptions = computed(() => {
      const repositorySnapshot = workspace.snapshot.value;
      if (!repositorySnapshot) return [];
      const options = [];
      options.push({ label: workspace.t('workspace.header.newBranchAndSwitch'), value: "action:new-branch" });
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

    function openNewBranch(): void {
      if (workspace.mutationBusy.value || workspace.networkBusy.value) return;
      branchPickerOpen.value = false;
      workspace.branchForm.name = "";
      workspace.branchForm.checkout = true;
      workspace.branchForm.stashLocalChanges = false;
      workspace.branchModalShown.value = true;
    }

    function selectUtilityAction(value: string | number): void {
      if (value === "open-website" && workspace.repositoryWebUrl.value) {
        void workspace.openCodeHostUrl(workspace.repositoryWebUrl.value);
      } else if (value === "refresh") {
        if (workspace.mutationBusy.value || workspace.networkBusy.value || workspace.snapshotLoading.value) return;
        void workspace.refreshSnapshot();
      } else if (value === "repository-settings" && workspace.activeRemote.value) {
        workspace.activeTab.value = "branches";
      } else if (value === "fork-and-push" && !workspace.mutationBusy.value && !workspace.networkBusy.value) {
        workspace.openForkModal();
      }
    }

    function selectBranch(value: string | number): void {
      if (typeof value !== "string") return;
      const repositorySnapshot = workspace.snapshot.value;
      if (!repositorySnapshot) return;
      if (value === "action:new-branch") {
        openNewBranch();
        return;
      }
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
      if (value === "toggle-star") {
        await toggleRepositoryStar(repository.path);
        await nextTick();
        focusRepositoryPickerControl();
      } else if (value === "toggle-hidden") {
        pickerPreferences.toggleHidden(repository.path);
        await nextTick();
        focusRepositoryPickerControl();
      } else if (value === "manage-repository-editors") {
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
      } else if (value === "copy-absolute-path") {
        repositoryPickerOpen.value = false;
        try {
          await writeClipboardText(repository.path);
          workspace.message.success(workspace.t("workspace.header.absolutePathCopied"));
        } catch {
          workspace.message.error(workspace.t("workspace.header.couldNotCopyAbsolutePath"));
        }
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
        const shouldRestoreFocus = (repositoryPickerElement.value?.contains(document.activeElement) ?? false)
          || document.activeElement === document.body;
        repositoryContextMenuOpen.value = false;
        repositoryFilter.value = "";
        if (shouldRestoreFocus) {
          void nextTick(focusRepositoryPickerControl);
        }
      }
    });
    return {
      ...workspace,
      ...pickerPreferences,
      toggleStar: toggleRepositoryStar,
      isRepositoryGroupCollapsed,
      repositoryTitle,
      repositoryRemoteLabel,
      repositoryStarAriaLabel,
      repositoryFilter,
      repositoryPickerElement,
      repositoryVirtualList,
      repositoryPickerRows,
      navigateRepositoryPicker,
      repositoryPickerTrigger,
      repositorySearchInput,
      displayedRepositoryGroups,
      repositoryPickerOpen,
      branchPickerOpen,
      repositoryContextMenuOpen,
      repositoryContextMenuX,
      repositoryContextMenuY,
      repositoryContextMenuOptions,
      applicationMenuOptions,
      utilityMenuOptions,
      branchSwitcherOptions,
      branchSwitcherStyle,
      branchSwitcherValue,
      repositorySwitcherAriaLabel,
      branchStatusLabel,
      pushNeedsAttention,
      pullCount,
      pushCount,
      pullButtonTitle,
      pushButtonTitle,
      renderBranchLabel,
      openRepositoryContextMenu,
      openRepositoryContextMenuFromKeyboard,
      selectApplicationMenuItem,
      toggleRepositoryPicker,
      selectUtilityAction,
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
                  <svg v-if="customTitlebar" class="application-brand-mark" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M6 7v10M18 7c0 7-12 3-12 10" /><circle cx="6" cy="5" r="2" /><circle cx="6" cy="19" r="2" /><circle cx="18" cy="5" r="2" /></svg>
                  <template v-else><span v-for="line in 4" :key="line" class="application-menu-line" /></template>
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
                    ref="repositoryPickerTrigger"
                    class="repository-switcher"
                    type="button"
                    :aria-label="repositorySwitcherAriaLabel"
                    :aria-expanded="repositoryPickerOpen"
                    aria-haspopup="dialog"
                    aria-keyshortcuts="Shift+F10"
                    :aria-description="t('workspace.header.rightClickForRepositoryActions')"
                    :title="snapshot ? repositoryTitle(snapshot.repository) : undefined"
                    @click.stop="toggleRepositoryPicker($event)"
                    @contextmenu="snapshot?.repository && openRepositoryContextMenu($event, snapshot.repository)"
                    @keydown="snapshot?.repository && openRepositoryContextMenuFromKeyboard($event, snapshot.repository)"
                  >
                    <span class="repository-switcher-primary">{{ snapshot?.repository.alias || snapshot?.repository.name || t('workspace.header.repository') }}</span>
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
                        :placeholder="t('workspace.header.searchRepositories')"
                      >
                    </div>
                    <div class="repository-picker-command-tools">
                      <span>{{ t('workspace.header.switchRepository') }}</span>
                      <n-select
                        v-model:value="repositoryGrouping"
                        size="tiny"
                        :options="repositoryGroupingOptions"
                        :consistent-menu-width="false"
                        class="repository-grouping-select"
                      />
                    </div>
                  </div>

                  <div class="repository-picker-repositories">
                    <n-spin :show="repositoriesLoading" size="small">
                      <n-virtual-list
                        v-if="displayedRepositoryGroups.length"
                        class="sidebar-repository-list"
                        ref="repositoryVirtualList"
                        :items="repositoryPickerRows"
                        :item-size="28"
                        item-resizable
                        @keydown="navigateRepositoryPicker"
                      >
                        <template #default="{ item, index }">
                        <div :data-picker-index="index">
                          <button
                            v-if="item.kind === 'group'"
                            class="sidebar-group-heading"
                            :class="{ 'sidebar-group-heading-owner': item.group.key.startsWith('owner:') }"
                            type="button"
                            :aria-expanded="!isRepositoryGroupCollapsed(item.group.key)"
                            :disabled="Boolean(repositoryFilter.trim())"
                            @click="toggleCollapsed(item.group.key)"
                          >
                            <span>{{ isRepositoryGroupCollapsed(item.group.key) ? '▸' : '▾' }} {{ item.group.key === 'starred' ? t('workspace.header.starredRepositories') : item.group.label }}</span>
                            <span>{{ item.group.repositories.length }}</span>
                          </button>
                          <div v-else class="sidebar-repository-item">
                              <button
                                class="sidebar-repository"
                                :class="{ active: item.repository.path === activeRepositoryPath }"
                                type="button"
                                :aria-current="item.repository.path === activeRepositoryPath ? 'true' : undefined"
                                aria-keyshortcuts="Shift+F10"
                                :aria-description="t('workspace.header.rightClickForRepositoryActions')"
                                :title="`${repositoryTitle(item.repository)}\n${t('workspace.header.rightClickForRepositoryActions')}`"
                                @click="repositoryPickerOpen = false; selectRepository(item.repository.path)"
                                @contextmenu="openRepositoryContextMenu($event, item.repository)"
                                @keydown="openRepositoryContextMenuFromKeyboard($event, item.repository)"
                              >
                                <span v-if="item.repository.alias" class="visually-hidden">{{ t('workspace.header.alias') }}: </span>
                                <span v-if="item.repository.alias" class="repository-alias-indicator" aria-hidden="true" />
                                <span class="sidebar-repository-copy">
                                  <span class="repository-name" :class="{ 'repository-alias-name': item.repository.alias }">{{ item.repository.alias || item.repository.name }}</span>
                                  <span v-if="item.repository.alias" class="repository-canonical-name">{{ item.repository.name }}</span>
                                  <span v-if="item.repository.remoteRepository && item.repository.remoteRepository !== item.repository.name" class="repository-canonical-name">{{ t('workspace.header.remoteRepository') }}: {{ repositoryRemoteLabel(item.repository) }}</span>
                                </span>
                                <span v-if="isHidden(item.repository.path)" class="repository-hidden-label">{{ t('workspace.header.hiddenRepository') }}</span>
                              </button>
                              <button
                                class="repository-star-button"
                                type="button"
                                :data-repository-path="item.repository.path"
                                :aria-pressed="isStarred(item.repository.path)"
                                :aria-label="repositoryStarAriaLabel(item.repository, t(isStarred(item.repository.path) ? 'workspace.header.unstarRepository' : 'workspace.header.starRepository'))"
                                :title="t(isStarred(item.repository.path) ? 'workspace.header.unstarRepository' : 'workspace.header.starRepository')"
                                @click="toggleStar(item.repository.path)"
                              >{{ isStarred(item.repository.path) ? '★' : '☆' }}</button>
                          </div>
                        </div>
                        </template>
                      </n-virtual-list>
                      <n-text v-else depth="3" class="sidebar-empty">
                        {{ repositoryFilter.trim() ? t('workspace.header.noRepositoriesMatchSearch') : repositoryGroups.length ? t('workspace.header.noVisibleRepositories') : t('workspace.header.noRepositoriesAdded') }}
                      </n-text>
                    </n-spin>
                  </div>

                  <div class="repository-picker-footer">
                    <n-button size="small" text block :aria-pressed="showAllRepositories" @click="showAllRepositories = !showAllRepositories">
                      {{ t(showAllRepositories ? 'workspace.header.showVisibleRepositories' : 'workspace.header.showAllRepositories') }}
                    </n-button>
                    <n-dropdown trigger="click" placement="top-start" to="body" :options="repositoryActionOptions" @select="(value) => { repositoryPickerOpen = false; openRepositoryAction(value); }">
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
              <div v-if="snapshot" class="topbar-branch-group" :style="branchSwitcherStyle">
                <svg class="topbar-action-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="6" cy="5" r="2" /><circle cx="6" cy="19" r="2" /><circle cx="18" cy="5" r="2" /><path d="M6 7v10m12-10c0 7-12 3-12 10" /></svg>
                <n-select
                  v-model:show="branchPickerOpen"
                  :value="branchSwitcherValue"
                  size="small"
                  :options="branchSwitcherOptions"
                  :render-label="renderBranchLabel"
                  :placeholder="currentBranch"
                  :disabled="mutationBusy || Boolean(networkBusy)"
                  :consistent-menu-width="true"
                  :menu-props="{ class: 'branch-select-menu' }"
                  :aria-label="t('workspace.header.switchBranch')"
                  :title="currentBranch"
                  class="branch-select"
                  @update:value="selectBranch"
                >
                  <template #action>
                    <div class="branch-menu-context">
                      <span>{{ currentBranch }}</span>
                      <span v-if="snapshot.status.upstream">{{ t('workspace.tables.upstream') }}: {{ snapshot.status.upstream }}</span>
                      <span v-if="branchStatusLabel">{{ branchStatusLabel }}</span>
                    </div>
                  </template>
                </n-select>
              </div>
              <span v-if="branchStatusLabel" class="branch-status-indicator" :title="branchStatusLabel" :aria-label="branchStatusLabel" role="img">
                <span class="branch-status-label" aria-hidden="true">{{ branchStatusLabel }}</span>
              </span>
              </div>
            </div>
          </div>
          <div class="topbar-actions">
            <n-select
              v-if="snapshot?.remotes.length"
              v-model:value="selectedRemoteName"
              size="small"
              :disabled="mutationBusy || Boolean(networkBusy)"
              :options="snapshot.remotes.map((remote) => ({ label: remote.name, value: remote.name }))"
              :placeholder="t('workspace.header.selectRemote')"
              :aria-label="t('workspace.header.selectRemote')"
              :title="selectedRemoteName || t('workspace.header.selectRemote')"
              :consistent-menu-width="false"
              class="remote-select"
            />
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
                <svg class="topbar-action-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1" /></svg>
                <span class="topbar-action-label">{{ t('repository.branches.fetch') }}</span>
              </n-button>
              <n-button
                v-if="activeRemote"
                size="small"
                :type="pullCount > 0 ? 'primary' : 'default'"
                :loading="networkBusy === 'pull'"
                :disabled="mutationBusy || !canPullFromActiveRemote || Boolean(networkBusy)"
                :aria-label="pullButtonTitle"
                :title="pullButtonTitle"
                @click="runNetworkAction('pull')"
              >
                <svg class="topbar-action-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 3v12m-5-5 5 5 5-5M5 17v4h14v-4" /></svg>
                <span class="topbar-action-label">{{ t('workspace.header.pull') }}</span>
                <span v-if="pullCount > 0" class="topbar-action-count">{{ pullCount }}</span>
              </n-button>
              <n-button-group v-if="activeRemote">
                <n-button
                  size="small"
                  :type="pushNeedsAttention ? 'primary' : 'default'"
                  :loading="networkBusy === 'push'"
                  :disabled="mutationBusy || !snapshot?.hasHead || !snapshot.status.branch || Boolean(networkBusy)"
                  :aria-label="pushButtonTitle"
                  :title="pushButtonTitle"
                  @click="runNetworkAction('push')"
                >
                  <svg class="topbar-action-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 16V4m-5 5 5-5 5 5M5 17v4h14v-4" /></svg>
                  <span class="topbar-action-label">{{ snapshot?.hasHead && snapshot.status.branch && !snapshot.status.upstream ? t('workspace.header.pushFirst') : t('workspace.header.push') }}</span>
                  <span v-if="pushCount > 0" class="topbar-action-count">{{ pushCount }}</span>
                </n-button>
                <n-dropdown
                  trigger="click"
                  placement="bottom-end"
                  :disabled="mutationBusy || !snapshot?.hasHead || !snapshot.status.branch || Boolean(networkBusy)"
                  :options="[{ label: t('workspace.header.pushWithAllTags'), key: 'push-with-tags' }]"
                  @select="runNetworkAction('push', false, undefined, true)"
                >
                  <n-button
                    size="small"
                    :type="pushNeedsAttention ? 'primary' : 'default'"
                    :disabled="mutationBusy || !snapshot?.hasHead || !snapshot.status.branch || Boolean(networkBusy)"
                    :aria-label="t('workspace.header.pushOptions')"
                    :title="t('workspace.header.pushOptions')"
                  >
                    <span aria-hidden="true">▾</span>
                  </n-button>
                </n-dropdown>
              </n-button-group>
            </div>
            <div v-if="snapshot" class="topbar-utility-group">
              <n-dropdown
                trigger="click"
                placement="bottom-end"
                :options="utilityMenuOptions"
                @select="selectUtilityAction"
              >
                <n-button
                  size="small"
                  tertiary
                  :aria-label="t('repository.history.moreActions')"
                  :title="t('repository.history.moreActions')"
                >
                  <span aria-hidden="true">…</span>
                </n-button>
              </n-dropdown>
            </div>
          </div>
        </div>
        <div v-else class="topbar-content">
          <div class="topbar-leading">
            <div class="application-menu">
              <n-dropdown trigger="click" placement="bottom-start" :options="applicationMenuOptions" @select="selectApplicationMenuItem">
                <button class="application-menu-button" type="button" :aria-label="t('workspace.header.openApplicationMenu')" :title="t('workspace.header.openApplicationMenu')">
                  <svg v-if="customTitlebar" class="application-brand-mark" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M6 7v10M18 7c0 7-12 3-12 10" /><circle cx="6" cy="5" r="2" /><circle cx="6" cy="19" r="2" /><circle cx="18" cy="5" r="2" /></svg>
                  <template v-else><span v-for="line in 4" :key="line" class="application-menu-line" /></template>
                </button>
              </n-dropdown>
            </div>
            <div class="topbar-title">{{ t('workspace.header.accountsAndAuthentication') }}</div>
          </div>
        </div>
      </n-layout-header>
</template>

<style src="./WorkspaceHeader.css"></style>
