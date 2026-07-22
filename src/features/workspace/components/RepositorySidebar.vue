<script lang="ts">
import { defineComponent } from "vue";
import { NButton, NDropdown, NLayoutSider, NSelect, NSpin, NText } from "naive-ui";
import { useWorkspaceControllerContext } from "../composables/workspaceContext";

export default defineComponent({
  name: "RepositorySidebar",
  components: { NButton, NDropdown, NLayoutSider, NSelect, NSpin, NText },
  setup() {
    return useWorkspaceControllerContext();
  },
});
</script>

<template>
    <n-layout-sider bordered :width="294" class="application-sidebar">
      <div class="sidebar-shell">
        <aside class="app-rail" :aria-label="tr('应用导航', 'Application navigation')">
          <button class="rail-brand" type="button" :title="tr('BroKnowMyGit 首页', 'BroKnowMyGit home')" @click="updateSection('repositories')">
            <span class="brand-mark">BK</span>
          </button>

          <div class="rail-navigation">
            <button
              class="rail-navigation-button"
              :class="{ active: section === 'repositories' }"
              type="button"
              :aria-label="tr('仓库', 'Repositories')"
              :title="tr('仓库', 'Repositories')"
              @click="updateSection('repositories')"
            >
              <span class="rail-icon" aria-hidden="true">⌘</span>
              <span class="rail-label">{{ tr("仓库", "Repos") }}</span>
            </button>
            <button
              class="rail-navigation-button"
              :class="{ active: section === 'accounts' }"
              type="button"
              :aria-label="tr('账号与认证', 'Accounts & authentication')"
              :title="tr('账号与认证', 'Accounts & authentication')"
              @click="updateSection('accounts')"
            >
              <span class="rail-icon" aria-hidden="true">◎</span>
              <span class="rail-label">{{ tr("账号", "Accounts") }}</span>
            </button>
          </div>
        </aside>

        <aside class="repository-sidebar" :aria-label="tr('本地仓库', 'Local repositories')">
          <div class="repository-sidebar-header">
            <div class="repository-sidebar-title-row">
              <div class="sidebar-heading">{{ tr("本地仓库", "Local repositories") }}</div>
              <n-select v-model:value="repositoryGrouping" size="tiny" :options="repositoryGroupingOptions" class="repository-grouping-select" />
            </div>
            <n-dropdown :options="repositoryActionOptions" @select="openRepositoryAction">
              <n-button size="small" type="primary" block class="repository-add-button">
                <span aria-hidden="true">＋</span>{{ tr("添加仓库", "Add repository") }}
              </n-button>
            </n-dropdown>
          </div>

          <div class="sidebar-repositories">
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
                        :title="repository.path"
                        @click="selectRepository(repository.path)"
                      >
                        <span class="repository-dot" />
                        <span class="repository-name">{{ repository.name }}</span>
                      </button>
                      <button
                        class="repository-group-edit"
                        type="button"
                        :aria-label="tr('编辑仓库分组', 'Edit repository group')"
                        :title="tr('编辑仓库分组', 'Edit repository group')"
                        @click.stop="openRepositoryGroupEditor(repository)"
                      >
                        ✎
                      </button>
                    </div>
                  </div>
                </div>
              </div>
              <n-text v-else depth="3" class="sidebar-empty">{{ tr("尚未添加仓库", "No repositories added") }}</n-text>
            </n-spin>
          </div>
        </aside>
      </div>
    </n-layout-sider>
</template>

