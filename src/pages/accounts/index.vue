<script lang="ts">
import { defineComponent } from "vue";
import { NButton, NCard, NDataTable, NEmpty, NGrid, NGridItem, NSpace, NText } from "naive-ui";
import { useWorkspaceControllerContext } from "../../features/workspace/composables/workspaceContext";

export default defineComponent({
  name: "AccountsPage",
  components: { NButton, NCard, NDataTable, NEmpty, NGrid, NGridItem, NSpace, NText },
  setup() {
    return useWorkspaceControllerContext();
  },
});
</script>

<template>
<section class="accounts-page">
            <div class="page-heading">
              <div>
                <div class="eyebrow">{{ tr("不保存令牌明文", "No plaintext tokens stored") }}</div>
                <h1>{{ tr("账号与认证", "Accounts & authentication") }}</h1>
                <p>{{ tr("默认账号只用于 Auto 的候选排序；每个 remote 的明确绑定始终优先。", "A default account only ranks Auto candidates; an explicit remote binding always wins.") }}</p>
              </div>
              <n-space>
                <n-button
                  v-for="endpoint in endpoints.filter((item) => capabilityForEndpoint(item.id).accountImport)"
                  :key="endpoint.id"
                  secondary
                  :loading="mutationBusy"
                  @click="importCliAccounts(endpoint.id)"
                >
                  {{ tr("检测", "Detect") }} {{ endpoint.kind === 'gitHub' ? 'gh' : 'glab' }}
                </n-button>
                <n-button secondary @click="openCustomEndpointModal">{{ tr("添加 Git 服务 Endpoint", "Add Git service endpoint") }}</n-button>
                <n-button type="primary" @click="accountModalShown = true">{{ tr("添加认证账号", "Add authentication account") }}</n-button>
              </n-space>
            </div>

            <n-card size="small" class="security-card">
              <n-grid :cols="3" x-gap="18" y-gap="12" responsive="screen" item-responsive>
                <n-grid-item span="3 m:1"><strong>{{ tr("令牌只进入系统钥匙串", "Tokens only enter the system keychain") }}</strong><br /><n-text depth="3">{{ tr("前端只做一次性写入，不会读取、记录或显示 PAT。", "The UI only writes a PAT once; it never reads, logs, or displays it.") }}</n-text></n-grid-item>
                <n-grid-item span="3 m:1"><strong>{{ tr("不切换 CLI 活跃账号", "No CLI active-account switching") }}</strong><br /><n-text depth="3">{{ tr("gh / glab 仅使用当前匹配的 CLI 身份，不调用 auth switch。", "gh / glab only use the currently matching CLI identity and never call auth switch.") }}</n-text></n-grid-item>
                <n-grid-item span="3 m:1"><strong>{{ tr("不改 Git 全局配置", "No global Git configuration changes") }}</strong><br /><n-text depth="3">{{ tr("网络操作由后端按 remote binding 解析认证策略。", "The backend resolves the authentication strategy per remote binding.") }}</n-text></n-grid-item>
              </n-grid>
            </n-card>

            <n-card size="small" :title="tr('保存的账号资料', 'Saved account profiles')" :segmented="{ content: true }">
              <template #header-extra>
                <n-space size="small" align="center">
                  <n-text v-if="accountStatusRefreshing" depth="3" class="account-status-refresh-note">{{ tr("正在后台检测认证状态", "Checking authentication in background") }}</n-text>
                  <n-button size="tiny" tertiary :loading="accountStatusRefreshing" @click="refreshAccountStatuses(true)">{{ tr("刷新认证状态", "Refresh authentication") }}</n-button>
                </n-space>
              </template>
              <n-data-table :columns="accountColumns" :data="accounts" :loading="accountsLoading" :bordered="false" />
              <n-empty v-if="!accountsLoading && !accounts.length" :description="tr('还没有账号资料', 'No account profiles yet')">
                <template #extra><n-button type="primary" @click="accountModalShown = true">{{ tr("添加账号", "Add account") }}</n-button></template>
              </n-empty>
            </n-card>
          </section>
</template>

