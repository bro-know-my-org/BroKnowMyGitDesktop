<script lang="ts">
import { defineComponent } from "vue";
import { NButton, NCard, NDataTable, NEmpty, NGrid, NGridItem, NSpace, NText } from "naive-ui";
import { useWorkspaceControllerContext } from "../../features/workspace/composables/workspaceContext";
import { t } from "../../shared/i18n";

export default defineComponent({
  name: "AccountsPage",
  components: { NButton, NCard, NDataTable, NEmpty, NGrid, NGridItem, NSpace, NText },
  setup() {
    return { ...useWorkspaceControllerContext(), t };
  },
});
</script>

<template>
<section class="accounts-page">
            <div class="page-heading">
              <div>
                <div class="eyebrow">{{ t("accounts.heading.securityEyebrow") }}</div>
                <h1>{{ t("accounts.heading.title") }}</h1>
                <p>{{ t("accounts.heading.description") }}</p>
              </div>
              <n-space>
                <n-button
                  v-for="endpoint in endpoints.filter((item) => capabilityForEndpoint(item.id).accountImport)"
                  :key="endpoint.id"
                  secondary
                  :loading="mutationBusy"
                  @click="importCliAccounts(endpoint.id)"
                >
                  {{ t("accounts.action.detectCli") }} {{ endpoint.kind === 'gitHub' ? 'gh' : 'glab' }}
                </n-button>
                <n-button secondary @click="openCustomEndpointModal">{{ t("accounts.action.addEndpoint") }}</n-button>
                <n-button type="primary" @click="accountModalShown = true">{{ t("accounts.action.addAuthenticationAccount") }}</n-button>
              </n-space>
            </div>

            <n-card size="small" class="security-card">
              <n-grid :cols="3" x-gap="18" y-gap="12" responsive="screen" item-responsive>
                <n-grid-item span="3 m:1"><strong>{{ t("accounts.security.keychain.title") }}</strong><br /><n-text depth="3">{{ t("accounts.security.keychain.description") }}</n-text></n-grid-item>
                <n-grid-item span="3 m:1"><strong>{{ t("accounts.security.cliIdentity.title") }}</strong><br /><n-text depth="3">{{ t("accounts.security.cliIdentity.description") }}</n-text></n-grid-item>
                <n-grid-item span="3 m:1"><strong>{{ t("accounts.security.gitConfig.title") }}</strong><br /><n-text depth="3">{{ t("accounts.security.gitConfig.description") }}</n-text></n-grid-item>
              </n-grid>
            </n-card>

            <n-card size="small" :title="t('accounts.saved.title')" :segmented="{ content: true }">
              <template #header-extra>
                <n-space size="small" align="center">
                  <n-text v-if="accountStatusRefreshing" depth="3" class="account-status-refresh-note">{{ t("accounts.saved.statusChecking") }}</n-text>
                  <n-button size="tiny" tertiary :loading="accountStatusRefreshing" @click="refreshAccountStatuses(true)">{{ t("accounts.action.refreshAuthentication") }}</n-button>
                </n-space>
              </template>
              <n-data-table :columns="accountColumns" :data="accounts" :loading="accountsLoading" :bordered="false" />
              <n-empty v-if="!accountsLoading && !accounts.length" :description="t('accounts.saved.empty')" :show-icon="false">
                <template #extra><n-button type="primary" @click="accountModalShown = true">{{ t("accounts.action.addAccount") }}</n-button></template>
              </n-empty>
            </n-card>
          </section>
</template>

<style src="./index.css"></style>
