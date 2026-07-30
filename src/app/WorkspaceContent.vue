<script lang="ts">
import { defineComponent } from "vue";
import { NAlert, NButton, NEmpty, NLayoutContent, NSpace, NSpin } from "naive-ui";
import AccountsPage from "../pages/accounts/index.vue";
import RepositoriesPage from "../pages/repositories/index.vue";
import { useWorkspaceControllerContext } from "../features/workspace/composables/workspaceContext";

export default defineComponent({
  name: "WorkspaceContent",
  components: { NAlert, NButton, NEmpty, NLayoutContent, NSpace, NSpin, AccountsPage, RepositoriesPage },
  setup() {
    return useWorkspaceControllerContext();
  },
});
</script>

<template>
<n-layout-content
  class="content-area"
  :class="{ 'is-fixed-repository-content': section === 'repositories' && (activeTab === 'changes' || activeTab === 'history') && !browserDemo && !lastError }"
>
  <main class="content-inner">
<n-alert v-if="browserDemo" type="info" class="demo-alert" :bordered="false">
            {{ tr("浏览器演示模式：当前展示可交互的本地 mock 数据；启动 Tauri 后会调用 Rust commands。示例 PAT 不会保存或展示。", "Browser demo mode: this uses interactive local mock data. Tauri calls Rust commands. Demo PATs are never saved or shown.") }}
          </n-alert>

          <n-alert v-if="lastError" type="error" closable class="error-alert" @close="clearLastError">
            <template #header>
              {{ lastErrorAction ? tr("最近一次操作失败：{action}", "Most recent operation failed: {action}", { action: lastErrorAction }) : tr("最近一次操作失败", "Most recent operation failed") }}
            </template>
            <strong>{{ lastError.message }}</strong>
            <div class="error-diagnostic-code">{{ tr("错误代码", "Error code") }}: <code>{{ lastError.code }}</code></div>
            <div v-if="lastError.details" class="error-diagnostic-guidance">{{ lastError.details }}</div>
            <template v-if="lastError.code === 'account_selection_required'">{{ tr("请在 remote 绑定中选择明确账号，然后重试。", "Select an explicit account in the remote binding, then retry.") }}</template>
          </n-alert>
    <AccountsPage v-if="section === 'accounts'" />
<n-spin v-else-if="!snapshot && (workspaceInitializing || snapshotLoading)" size="large" class="loading-state" />
<section v-else-if="!snapshot" class="repository-empty-state">
            <n-empty :description="tr('选择或添加一个本地 Git 仓库', 'Select or add a local Git repository')">
              <template #extra>
                <n-space justify="center">
                  <n-button @click="discoverModalShown = true">{{ tr("扫描文件夹", "Scan folder") }}</n-button>
                  <n-button @click="initModalShown = true">{{ tr("新建本地仓库", "New local repository") }}</n-button>
                  <n-button type="primary" @click="cloneModalShown = true">{{ tr("Clone 仓库", "Clone repository") }}</n-button>
                </n-space>
              </template>
            </n-empty>
          </section>
    <RepositoriesPage v-else-if="snapshot" />
  </main>
</n-layout-content>
</template>

<style src="./WorkspaceContent.css"></style>
