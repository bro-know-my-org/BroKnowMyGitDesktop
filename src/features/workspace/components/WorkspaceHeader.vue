<script lang="ts">
import { computed, defineComponent, h, nextTick, ref, watch } from "vue";
import { NButton, NDropdown, NLayoutHeader, NPopover, NSelect, NSpace, NSpin, NTag, NText, type SelectRenderLabel } from "naive-ui";
import { useWorkspaceControllerContext } from "../composables/workspaceContext";
import type { RepositoryRecord } from "../../../shared/types/domain";

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
  components: { NButton, NDropdown, NLayoutHeader, NPopover, NSelect, NSpace, NSpin, NTag, NText },
  setup() {
    const workspace = useWorkspaceControllerContext();
    const repositoryPickerOpen = ref(false);
    const repositoryContextMenuOpen = ref(false);
    const repositoryContextMenuX = ref(0);
    const repositoryContextMenuY = ref(0);
    const repositoryContextTarget = ref<RepositoryRecord | null>(null);
    const repositoryContextMenuOptions = computed(() => [
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
    const branchSwitcherValue = computed(() => {
      const branch = workspace.snapshot.value?.status.branch;
      return branch ? `local:${branch}` : null;
    });
    const currentBranchLabelParts = computed(() => splitReferenceLabel(workspace.currentBranch.value));
    const upstreamLabelParts = computed(() => splitReferenceLabel(workspace.snapshot.value?.status.upstream ?? ""));
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
      repositoryContextMenuOpen.value = false;
      repositoryContextTarget.value = repository;
      const target = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
      const bounds = target?.getBoundingClientRect();
      repositoryContextMenuX.value = event.clientX || (bounds ? bounds.left + bounds.width / 2 : 0);
      repositoryContextMenuY.value = event.clientY || (bounds ? bounds.top + bounds.height / 2 : 0);
      void nextTick(() => {
        repositoryContextMenuOpen.value = true;
      });
    }

    function selectRepositoryContextMenuAction(value: string | number): void {
      repositoryContextMenuOpen.value = false;
      const repository = repositoryContextTarget.value;
      if (value === "edit-alias" && repository) {
        repositoryPickerOpen.value = false;
        workspace.openRepositoryAliasEditor(repository);
      } else if (value === "edit-group" && repository) {
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
      if (!open) repositoryContextMenuOpen.value = false;
    });
    return {
      ...workspace,
      repositoryPickerOpen,
      repositoryContextMenuOpen,
      repositoryContextMenuX,
      repositoryContextMenuY,
      repositoryContextMenuOptions,
      applicationMenuOptions,
      branchSwitcherOptions,
      branchSwitcherStyle,
      branchSwitcherValue,
      currentBranchLabelParts,
      upstreamLabelParts,
      renderBranchLabel,
      openRepositoryContextMenu,
      selectApplicationMenuItem,
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
                    :aria-label="t('workspace.header.switchRepository')"
                    :aria-expanded="repositoryPickerOpen"
                    :title="t('workspace.header.switchRepository')"
                  >
                    <template v-if="snapshot?.repository.alias">
                      <span class="repository-alias-mark">{{ t('workspace.header.alias') }}</span>
                      <span class="topbar-title repository-alias-name">{{ snapshot.repository.alias }}</span>
                      <span class="topbar-repository-canonical-name">{{ snapshot.repository.name }}</span>
                    </template>
                    <span v-else class="topbar-title">{{ snapshot?.repository.name || t('workspace.header.repository') }}</span>
                    <span class="repository-switcher-caret" aria-hidden="true">▾</span>
                  </button>
                </template>

                <aside class="repository-picker" :aria-label="t('workspace.header.localRepositories')">
                  <div class="repository-picker-header">
                    <div class="repository-picker-title-row">
                      <div class="sidebar-heading">{{ t('workspace.header.localRepositories') }}</div>
                      <n-select v-model:value="repositoryGrouping" size="tiny" :options="repositoryGroupingOptions" class="repository-grouping-select" />
                    </div>
                    <n-dropdown :options="repositoryActionOptions" @select="(value) => { repositoryPickerOpen = false; openRepositoryAction(value); }">
                      <n-button size="small" secondary block class="repository-add-button">
                        <span aria-hidden="true">＋</span>{{ t('workspace.header.addRepository') }}
                      </n-button>
                    </n-dropdown>
                  </div>

                  <div class="repository-picker-repositories">
                    <n-spin :show="repositoriesLoading" size="small">
                      <div v-if="repositories.length" class="sidebar-repository-list">
                        <div v-for="group in repositoryGroups" :key="group.key" class="sidebar-repository-group">
                          <div class="sidebar-group-heading">
                            <span>{{ group.label }}</span>
                            <span>{{ group.repositories.length }}</span>
                          </div>
                          <div class="sidebar-repository-group-list">
                            <div v-for="repository in group.repositories" :key="repository.path" class="sidebar-repository-item">
                              <button
                                class="sidebar-repository"
                                :class="{ active: repository.path === activeRepositoryPath }"
                                type="button"
                                :title="`${repository.path} · ${t('workspace.header.rightClickToSetAnAliasOrEditTheGroup')}`"
                                @click="repositoryPickerOpen = false; selectRepository(repository.path)"
                                @contextmenu="openRepositoryContextMenu($event, repository)"
                              >
                                <template v-if="repository.alias">
                                  <span class="repository-alias-mark">{{ t('workspace.header.alias') }}</span>
                                  <span class="repository-name repository-alias-name">{{ repository.alias }}</span>
                                  <span class="repository-canonical-name">{{ repository.name }}</span>
                                </template>
                                <span v-else class="repository-name">{{ repository.name }}</span>
                              </button>
                            </div>
                          </div>
                        </div>
                      </div>
                      <n-text v-else depth="3" class="sidebar-empty">{{ t('workspace.header.noRepositoriesAdded') }}</n-text>
                    </n-spin>
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
                @select="selectRepositoryContextMenuAction"
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
                <n-tag v-else-if="snapshot.status.upstream" size="small" :bordered="false" class="branch-status-tag" :title="snapshot.status.upstream">
                  <span class="middle-ellipsis-label">
                    <span v-if="upstreamLabelParts.prefix" class="middle-ellipsis-prefix">{{ upstreamLabelParts.prefix }}</span>
                    <span v-if="upstreamLabelParts.prefix" class="middle-ellipsis-separator" aria-hidden="true">/</span>
                    <span class="middle-ellipsis-tail">{{ upstreamLabelParts.tail }}</span>
                  </span>
                </n-tag>
                <n-text v-if="snapshot.status.ahead || snapshot.status.behind" depth="3" class="sync-summary">↑ {{ snapshot.status.ahead }} · ↓ {{ snapshot.status.behind }}</n-text>
              </template>
              </div>
            </div>
          </div>
          <n-space align="center" :wrap="false" class="topbar-actions">
            <div v-if="snapshot?.remotes.length" class="topbar-select-group">
              <n-select
                v-model:value="selectedRemoteName"
                size="small"
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
                :disabled="mutationBusy"
                :consistent-menu-width="false"
                :menu-props="{ class: 'branch-select-menu' }"
                :aria-label="t('workspace.header.switchBranch')"
                :title="snapshot.status.branch || t('workspace.header.switchBranch')"
                :style="branchSwitcherStyle"
                class="branch-select"
                @update:value="selectBranch"
              />
            </div>
            <n-button v-if="snapshot && !snapshot.remotes.length && publishEndpoints.length" size="small" type="primary" @click="openPublishModal">{{ t('workspace.header.publishToGithub') }}</n-button>
            <n-button v-if="activeRemote" size="small" :loading="networkBusy === 'pull'" :disabled="!canPullFromActiveRemote || Boolean(networkBusy && networkBusy !== 'pull')" @click="runNetworkAction('pull')">{{ t('workspace.header.pull') }}</n-button>
            <n-button v-if="activeRemote" size="small" type="primary" :loading="networkBusy === 'push'" :disabled="!snapshot?.hasHead || !snapshot.status.branch || Boolean(networkBusy && networkBusy !== 'push')" @click="runNetworkAction('push')">{{ t('workspace.header.push') }}</n-button>
            <n-button v-if="activeRemote" size="small" tertiary @click="activeTab = 'branches'">{{ t('workspace.header.repositorySettings') }}</n-button>
            <n-button size="small" :loading="snapshotLoading" @click="refreshSnapshot">{{ t('workspace.header.refresh') }}</n-button>
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
