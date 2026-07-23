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
      { label: workspace.tr("编辑仓库分组", "Edit repository group"), key: "edit-group" },
    ]);

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
      if (value === "edit-group" && repository) {
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
      openRepositoryContextMenu,
      selectRepositoryContextMenuAction,
    };
  },
});
</script>

<template>
      <n-layout-header bordered class="topbar">
        <div v-if="section === 'repositories'" class="topbar-content repository-topbar-content">
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
                    :aria-label="tr('切换仓库', 'Switch repository')"
                    :aria-expanded="repositoryPickerOpen"
                    :title="tr('切换仓库', 'Switch repository')"
                  >
                    <span class="topbar-title">{{ snapshot?.repository.name || tr("仓库", "Repository") }}</span>
                    <span class="repository-switcher-caret" aria-hidden="true">▾</span>
                  </button>
                </template>

                <aside class="repository-picker" :aria-label="tr('本地仓库', 'Local repositories')">
                  <div class="repository-picker-header">
                    <div class="repository-picker-title-row">
                      <div class="sidebar-heading">{{ tr("本地仓库", "Local repositories") }}</div>
                      <n-select v-model:value="repositoryGrouping" size="tiny" :options="repositoryGroupingOptions" class="repository-grouping-select" />
                    </div>
                    <n-dropdown :options="repositoryActionOptions" @select="(value) => { repositoryPickerOpen = false; openRepositoryAction(value); }">
                      <n-button size="small" type="primary" block class="repository-add-button">
                        <span aria-hidden="true">＋</span>{{ tr("添加仓库", "Add repository") }}
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
                                :title="`${repository.path} · ${tr('右键编辑分组', 'Right-click to edit group')}`"
                                @click="repositoryPickerOpen = false; selectRepository(repository.path)"
                                @contextmenu="openRepositoryContextMenu($event, repository)"
                              >
                                <span class="repository-dot" />
                                <span class="repository-name">{{ repository.name }}</span>
                              </button>
                            </div>
                          </div>
                        </div>
                      </div>
                      <n-text v-else depth="3" class="sidebar-empty">{{ tr("尚未添加仓库", "No repositories added") }}</n-text>
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
                <n-tag v-if="isUnbornRepository" size="small" type="warning" :bordered="false">{{ tr("尚未首次提交", "No commits yet") }}</n-tag>
                <n-tag v-else-if="snapshot.status.upstream" size="small" :bordered="false">{{ snapshot.status.upstream }}</n-tag>
                <n-text v-if="snapshot.status.ahead || snapshot.status.behind" depth="3" class="sync-summary">↑ {{ snapshot.status.ahead }} · ↓ {{ snapshot.status.behind }}</n-text>
              </template>
            </div>
            <n-text v-if="snapshot" depth="3" class="repository-path">{{ snapshot.repository.path }}</n-text>
          </div>
          <n-space align="center" :wrap="false" class="topbar-actions">
            <n-select
              v-if="snapshot?.remotes.length"
              v-model:value="selectedRemoteName"
              size="small"
              :options="snapshot.remotes.map((remote) => ({ label: remote.name, value: remote.name }))"
              :placeholder="tr('选择 remote', 'Select remote')"
              class="remote-select"
            />
            <n-button v-if="snapshot && !snapshot.remotes.length && publishEndpoints.length" size="small" type="primary" @click="openPublishModal">{{ tr("发布到 GitHub", "Publish to GitHub") }}</n-button>
            <n-button v-if="primaryNetworkAction" size="small" type="primary" :loading="networkBusy === primaryNetworkAction" @click="runPrimaryNetworkAction">{{ primaryNetworkActionLabel }}</n-button>
            <n-button v-if="activeRemote" size="small" tertiary @click="activeTab = 'branches'">{{ tr("仓库设置", "Repository settings") }}</n-button>
            <n-select :value="appLocale" size="small" :options="localeOptions" class="language-select" @update:value="updateAppLocale" />
            <n-dropdown :options="colorModeOptions" @select="updateColorMode">
              <n-button size="small" tertiary class="theme-toggle" :aria-label="themeButtonLabel" :title="themeButtonLabel">{{ colorModeIcon }}</n-button>
            </n-dropdown>
            <n-button size="small" :loading="snapshotLoading" @click="refreshSnapshot">{{ tr("刷新", "Refresh") }}</n-button>
          </n-space>
        </div>
        <div v-else class="topbar-content">
          <div>
            <div class="eyebrow">CredentialBroker</div>
            <div class="topbar-title">{{ tr("账号与认证", "Accounts & authentication") }}</div>
          </div>
          <n-space align="center" :wrap="false" class="topbar-actions">
            <n-select :value="appLocale" size="small" :options="localeOptions" class="language-select" @update:value="updateAppLocale" />
            <n-dropdown :options="colorModeOptions" @select="updateColorMode">
              <n-button size="small" tertiary class="theme-toggle" :aria-label="themeButtonLabel" :title="themeButtonLabel">{{ colorModeIcon }}</n-button>
            </n-dropdown>
          </n-space>
        </div>
      </n-layout-header>
</template>
