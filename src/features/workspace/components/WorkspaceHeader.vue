<script lang="ts">
import { defineComponent } from "vue";
import { NButton, NDropdown, NLayoutHeader, NSelect, NSpace, NTag, NText } from "naive-ui";
import { useWorkspaceControllerContext } from "../composables/workspaceContext";

export default defineComponent({
  name: "WorkspaceHeader",
  components: { NButton, NDropdown, NLayoutHeader, NSelect, NSpace, NTag, NText },
  setup() {
    return useWorkspaceControllerContext();
  },
});
</script>

<template>
      <n-layout-header bordered class="topbar">
        <div v-if="section === 'repositories'" class="topbar-content repository-topbar-content">
          <div class="repository-topbar-context">
            <div class="repository-title-row">
              <div class="topbar-title">{{ snapshot?.repository.name || tr("仓库", "Repository") }}</div>
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

