<script lang="ts">
import { defineComponent } from "vue";
import { NLayoutSider } from "naive-ui";
import { useWorkspaceControllerContext } from "../composables/workspaceContext";

export default defineComponent({
  name: "RepositorySidebar",
  components: { NLayoutSider },
  setup() {
    return useWorkspaceControllerContext();
  },
});
</script>

<template>
  <n-layout-sider :width="64" class="application-sidebar">
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
  </n-layout-sider>
</template>
