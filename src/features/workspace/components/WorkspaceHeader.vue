<script lang="ts">
import { computed, defineComponent, nextTick, ref, watch } from "vue";
import { NButton, NDropdown, NLayoutHeader, NPopover, NSelect, NSpace, NSpin, NTag, NText } from "naive-ui";
import { useWorkspaceControllerContext } from "../composables/workspaceContext";
import type { RepositoryRecord } from "../../../shared/types/domain";

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

    function selectApplicationMenuItem(value: string | number): void {
      if (value === "repositories" || value === "accounts") {
        workspace.updateSection(value);
      } else if (value === "settings") {
        workspace.settingsModalShown.value = true;
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
      openRepositoryContextMenu,
      selectApplicationMenuItem,
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
                <n-tag size="small" type="primary">{{ currentBranch }}</n-tag>
                <n-tag v-if="isUnbornRepository" size="small" type="warning" :bordered="false">{{ t('workspace.header.noCommitsYet') }}</n-tag>
                <n-tag v-else-if="snapshot.status.upstream" size="small" :bordered="false">{{ snapshot.status.upstream }}</n-tag>
                <n-text v-if="snapshot.status.ahead || snapshot.status.behind" depth="3" class="sync-summary">↑ {{ snapshot.status.ahead }} · ↓ {{ snapshot.status.behind }}</n-text>
              </template>
              </div>
            </div>
          </div>
          <n-space align="center" :wrap="false" class="topbar-actions">
            <n-select
              v-if="snapshot?.remotes.length"
              v-model:value="selectedRemoteName"
              size="small"
              :options="snapshot.remotes.map((remote) => ({ label: remote.name, value: remote.name }))"
              :placeholder="t('workspace.header.selectRemote')"
              class="remote-select"
            />
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
