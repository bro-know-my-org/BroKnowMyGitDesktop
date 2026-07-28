<script lang="ts">
import { defineComponent, onBeforeUnmount, ref } from "vue";
import { NAlert, NButton, NCard, NCheckbox, NColorPicker, NDescriptions, NDescriptionsItem, NDivider, NDrawer, NDrawerContent, NForm, NFormItem, NInput, NModal, NSelect, NSpace, NSpin, NTabPane, NTabs, NText } from "naive-ui";
import packageMetadata from "../../../../package.json";
import { useWorkspaceControllerContext } from "../composables/workspaceContext";

export default defineComponent({
  name: "WorkspaceDialogs",
  components: { NAlert, NButton, NCard, NCheckbox, NColorPicker, NDescriptions, NDescriptionsItem, NDivider, NDrawer, NDrawerContent, NForm, NFormItem, NInput, NModal, NSelect, NSpace, NSpin, NTabPane, NTabs, NText },
  setup() {
    const narrowSettingsViewport = typeof window === "undefined" || typeof window.matchMedia !== "function"
      ? null
      : window.matchMedia("(max-width: 600px)");
    const settingsTabsPlacement = ref<"left" | "top">(narrowSettingsViewport?.matches ? "top" : "left");
    function updateSettingsTabsPlacement(event: MediaQueryListEvent): void {
      settingsTabsPlacement.value = event.matches ? "top" : "left";
    }
    narrowSettingsViewport?.addEventListener("change", updateSettingsTabsPlacement);
    onBeforeUnmount(() => narrowSettingsViewport?.removeEventListener("change", updateSettingsTabsPlacement));
    return {
      ...useWorkspaceControllerContext(),
      appVersion: packageMetadata.version,
      settingsSection: ref<"general" | "about">("general"),
      settingsTabsPlacement,
    };
  },
});
</script>

<template>
  <n-modal v-model:show="settingsModalShown" preset="card" :title="tr('设置', 'Settings')" class="settings-dialog-card">
    <n-tabs v-model:value="settingsSection" type="line" :placement="settingsTabsPlacement" class="settings-tabs">
      <n-tab-pane name="general" :tab="tr('通用', 'General')">
        <section class="settings-section">
          <div class="settings-section-heading">
            <strong>{{ tr("外观与语言", "Appearance & language") }}</strong>
            <n-text depth="3">{{ tr("这些选项会自动保存在本机。", "These options are saved automatically on this device.") }}</n-text>
          </div>
          <n-form label-placement="top" class="settings-form">
            <n-form-item :label="tr('界面语言', 'Language')">
              <n-select :value="appLocale" :options="localeOptions" @update:value="updateAppLocale" />
            </n-form-item>
            <n-form-item :label="tr('外观主题', 'Appearance')">
              <n-select
                :value="colorMode"
                :options="colorModeOptions.map((option) => ({ label: option.label, value: option.key }))"
                @update:value="updateColorMode"
              />
            </n-form-item>
            <n-form-item :label="tr('主题色', 'Theme color')">
              <n-space vertical class="theme-color-setting">
                <n-color-picker
                  :value="themeColor"
                  :modes="['hex']"
                  show-alpha
                  :swatches="['#4B0082FF', '#6D28D9FF', '#2563EBFF', '#C026D3FF', '#DC2626FF', '#EA580CFF']"
                  @update:value="updateThemeColor"
                />
                <div class="theme-color-description">
                  <n-text depth="3" class="theme-color-hint">
                    {{ tr("透明度会影响选中背景、边框和焦点光晕的强度。", "Alpha controls the strength of selected backgrounds, borders, and focus rings.") }}
                  </n-text>
                  <n-button size="small" quaternary @click="resetThemeColor">{{ tr("恢复默认", "Reset") }}</n-button>
                </div>
              </n-space>
            </n-form-item>
          </n-form>
        </section>
      </n-tab-pane>
      <n-tab-pane name="about" :tab="tr('关于', 'About')">
        <section class="settings-about">
          <div class="about-brand-mark" aria-hidden="true">BK</div>
          <div class="settings-about-heading">
            <strong>BroKnowMyGitDesktop</strong>
            <n-text depth="3">{{ tr("版本 {version}", "Version {version}", { version: appVersion }) }}</n-text>
          </div>
          <p>{{ tr("轻量的多账号 Git 与代码托管桌面客户端。以本地 Git 为核心，并提供 GitHub、GitLab、Gitee 与通用 Git 服务集成。", "A lightweight multi-account desktop client for Git and code hosting. It keeps local Git at the center, with integrations for GitHub, GitLab, Gitee, and generic Git services.") }}</p>
          <div class="settings-about-details">
            <span>{{ tr("技术栈", "Built with") }}</span>
            <strong>Tauri 2 · Vue 3 · TypeScript · Rust</strong>
          </div>
        </section>
      </n-tab-pane>
    </n-tabs>
    <template #action>
      <n-space justify="end">
        <n-button type="primary" @click="settingsModalShown = false">{{ tr("完成", "Done") }}</n-button>
      </n-space>
    </template>
  </n-modal>

<n-modal v-model:show="repositoryGroupModalShown" preset="card" :title="tr('编辑仓库分组', 'Edit repository group')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="tr('分组名称', 'Group name')">
        <n-input v-model:value="repositoryGroupForm.name" maxlength="80" show-count :placeholder="tr('例如：工作、个人项目、客户 A', 'For example: Work, Personal, Client A')" />
      </n-form-item>
    </n-form>
    <n-text depth="3">{{ tr("留空并保存即可移到“未分组”。分组仅保存在本应用，不会修改 Git 仓库。", "Leave this blank and save to move it to Ungrouped. Groups are stored by this app and never change the Git repository.") }}</n-text>
    <template #action><n-space justify="end"><n-button @click="repositoryGroupModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="saveRepositoryGroup">{{ tr("保存分组", "Save group") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="externalEditorModalShown" preset="card" :title="tr('外部冲突编辑器', 'External conflict editor')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ tr("发生冲突时，应用会把所有冲突文件作为独立参数交给此编辑器。不会执行 Git 或仓库配置中的 shell 命令。", "When conflicts occur, the app passes every conflicted file to this editor as a separate argument. It never runs a shell command from Git or repository configuration.") }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="tr('可执行文件（绝对路径）', 'Executable (absolute path)')">
        <n-input v-model:value="externalEditorForm.executable" :placeholder="tr('例如：/usr/bin/code 或 /Applications/Visual Studio Code.app/Contents/Resources/app/bin/code', 'For example: /usr/bin/code or /Applications/Visual Studio Code.app/Contents/Resources/app/bin/code')" />
      </n-form-item>
      <n-form-item :label="tr('附加参数（每行一个，可选）', 'Additional arguments (one per line, optional)')">
        <n-input v-model:value="externalEditorForm.arguments" type="textarea" :autosize="{ minRows: 2, maxRows: 6 }" placeholder="--wait" />
      </n-form-item>
    </n-form>
    <n-text depth="3">{{ tr("请填写已安装编辑器的绝对可执行文件路径。VS Code 可搭配参数 --wait；保存后，在冲突提示中点击“在外部编辑器中解决”。", "Enter the installed editor's absolute executable path. VS Code can use argument --wait; after saving, click Resolve in external editor from the conflict alert.") }}</n-text>
    <template #action>
      <n-space justify="space-between">
        <n-button v-if="externalEditor" type="error" tertiary :loading="mutationBusy" @click="confirmClearExternalEditor">{{ tr("清除配置", "Clear configuration") }}</n-button>
        <span v-else />
        <n-space>
          <n-button @click="externalEditorModalShown = false">{{ tr("取消", "Cancel") }}</n-button>
          <n-button type="primary" :loading="mutationBusy" @click="saveExternalEditor">{{ tr("保存编辑器", "Save editor") }}</n-button>
        </n-space>
      </n-space>
    </template>
  </n-modal>

  <n-modal v-model:show="blameModalShown" preset="card" :title="tr('文件 Blame', 'File blame')" class="dialog-card">
    <n-text depth="3">{{ selectedChange?.path || '—' }}</n-text>
    <n-spin :show="blameLoading">
      <pre v-if="blameText" class="diff-output dialog-output">{{ blameText }}</pre>
      <n-text v-else-if="!blameLoading" depth="3">{{ tr("此文件没有可显示的 Blame 记录。", "This file has no blame records to display.") }}</n-text>
    </n-spin>
    <template #action><n-space justify="end"><n-button @click="blameModalShown = false">{{ tr("关闭", "Close") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="reflogModalShown" preset="card" :title="tr('Reflog（恢复记录）', 'Reflog (recovery history)')" class="dialog-card">
    <n-alert type="info" :bordered="false" class="token-alert">{{ tr("Reflog 记录本地引用移动，可用于定位误删、重置或变基前的提交；恢复操作仍需在历史页谨慎执行。", "Reflog records local reference movements and helps locate commits before resets, rebases, or accidental changes. Use recovery actions from History carefully.") }}</n-alert>
    <n-spin :show="reflogLoading">
      <pre v-if="reflogText" class="diff-output dialog-output">{{ reflogText }}</pre>
      <n-text v-else-if="!reflogLoading" depth="3">{{ tr("当前仓库没有可显示的 Reflog 记录。", "This repository has no reflog records to display.") }}</n-text>
    </n-spin>
    <template #action><n-space justify="end"><n-button @click="reflogModalShown = false">{{ tr("关闭", "Close") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="cloneModalShown" preset="card" :title="tr('Clone 仓库', 'Clone repository')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="tr('远程地址', 'Remote URL')"><n-input v-model:value="cloneForm.url" :placeholder="tr('https://gitee.com/org/repository.git 或 git@host:org/repository.git', 'https://git.example.com/org/repository.git or git@host:org/repository.git')" /></n-form-item>
      <n-form-item :label="tr('目标目录', 'Destination folder')"><n-input v-model:value="cloneForm.destinationPath" placeholder="/Users/you/Projects/repository" /></n-form-item>
      <n-form-item :label="tr('CodeHost Endpoint（私有仓库可选）', 'Git service endpoint (optional for private repositories)')"><n-select v-model:value="cloneForm.endpointId" clearable :options="endpoints.map((endpoint) => ({ label: endpoint.displayName, value: endpoint.id }))" :placeholder="tr('留空则使用系统 Git 的默认行为', 'Leave empty to use normal system Git behavior')" /></n-form-item>
      <n-form-item :label="tr('Clone 使用的账号', 'Account used for clone')"><n-select v-model:value="cloneForm.accountId" :options="cloneAccountOptions" /></n-form-item>
    </n-form>
      <n-alert type="info" :bordered="false">{{ tr("不指定账号时，Clone 使用系统 Git；SSH、LFS、hooks 与现有 credential helper 保持兼容。选择 PAT 后会安全地使用该账号并在成功后保存 origin 绑定；选择 SSH agent 时由系统 SSH 配置选择实际密钥。", "Without an app account, clone uses system Git and keeps SSH, LFS, hooks, and your existing credential helper compatible. Selecting a PAT uses it safely and saves the origin binding after success; an SSH-agent selection leaves key choice to your system SSH configuration.") }}</n-alert>
    <template #action><n-space justify="end"><n-button @click="cloneModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="cloneRepository">{{ tr("Clone", "Clone") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="initModalShown" preset="card" :title="tr('新建本地仓库', 'New local repository')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ tr("将在指定目录初始化一个空 Git 仓库；不会创建或修改远程仓库。", "This initializes an empty Git repository at the selected folder; it does not create or modify a remote repository.") }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="tr('新仓库目录', 'New repository folder')"><n-input v-model:value="initForm.destinationPath" placeholder="/Users/you/Projects/my-repository" /></n-form-item>
      <n-form-item :label="tr('初始分支（可选）', 'Initial branch (optional)')"><n-input v-model:value="initForm.initialBranch" placeholder="main" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="initModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="initRepository">{{ tr("创建仓库", "Create repository") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="discoverModalShown" preset="card" :title="tr('发现本地仓库', 'Discover local repositories')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top"><n-form-item :label="tr('扫描根目录', 'Scan root folder')"><n-input v-model:value="discoverForm.rootPath" placeholder="/Users/you/Projects" /></n-form-item></n-form>
    <n-text depth="3">{{ tr("只读取本地目录，扫描结果由 GitCore 返回。", "Only local directories are read; GitCore returns the scan results.") }}</n-text>
    <template #action><n-space justify="end"><n-button @click="discoverModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="discoverRepositories">{{ tr("开始扫描", "Start scan") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="accountModalShown" preset="card" :title="tr('添加认证账号', 'Add authentication account')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert v-if="accountUsesPat" type="warning" :bordered="false" class="token-alert">{{ tr("PAT 只会通过本次 IPC 写入系统钥匙串。请勿在登录名或显示名中放入令牌。", "The PAT is written to the system keychain only through this IPC request. Do not put a token in the login or display name.") }}</n-alert>
    <n-alert v-else type="info" :bordered="false" class="token-alert">{{ tr("SSH agent 保留现有系统配置，并可作为明确的 remote 账号绑定。系统 Git credential helper 按 remote URL 自动工作，无需创建账号资料。", "The SSH agent keeps its system configuration and can be bound explicitly to a remote. The system Git credential helper works automatically by remote URL and needs no account profile.") }}</n-alert>
    <n-form label-placement="top">
      <n-form-item :label="tr('Git 服务 Endpoint', 'Git service endpoint')"><n-select v-model:value="accountForm.endpointId" :options="endpoints.map((endpoint) => ({ label: endpoint.displayName, value: endpoint.id }))" /></n-form-item>
      <n-form-item :label="tr('认证方式', 'Authentication method')"><n-select v-model:value="accountForm.authMethod" :options="accountMethodOptions" /></n-form-item>
      <n-form-item :label="tr('登录名', 'Login')"><n-input v-model:value="accountForm.login" :placeholder="tr('例如 your-login', 'for example, your-login')" /></n-form-item>
      <n-form-item :label="tr('显示名称（可选）', 'Display name (optional)')"><n-input v-model:value="accountForm.displayName" :placeholder="tr('例如 工作账号', 'for example, Work account')" /></n-form-item>
      <n-form-item v-if="accountUsesPat" :label="tr('个人访问令牌', 'Personal access token')"><n-input v-model:value="accountForm.token" type="password" show-password-on="click" :placeholder="tr('仅一次性写入', 'Written once only')" /></n-form-item>
    </n-form>
    <n-alert v-if="accountUsesPat && !accountFormCapabilities.patKeychain" type="error" :bordered="false">{{ tr("所选 Endpoint 尚未声明 PAT 钥匙串能力，不能保存。", "The selected endpoint does not declare PAT keychain support, so it cannot be saved.") }}</n-alert>
    <template #action><n-space justify="end"><n-button @click="accountModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" :disabled="accountUsesPat && !accountFormCapabilities.patKeychain" @click="saveAccount">{{ accountUsesPat ? tr('保存到系统钥匙串', 'Save to system keychain') : tr('保存账号资料', 'Save account profile') }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="customEndpointModalShown" preset="card" :title="tr('添加 Git 服务 Endpoint', 'Add Git service endpoint')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">{{ tr("通用 Git 服务不要求网页或 API；可将 SSH 或 HTTPS PAT 作为明确 remote 绑定。系统 credential helper 则按 remote URL 自动工作。", "A generic Git service needs no web site or API. SSH or an HTTPS PAT can be explicitly bound to a remote; the system credential helper works automatically by remote URL.") }}</n-alert>
    <n-form label-placement="top">
      <n-form-item :label="tr('稳定 ID / Git host', 'Stable ID / Git host')"><n-input v-model:value="endpointForm.id" placeholder="git.example.com" /></n-form-item>
      <n-form-item :label="tr('平台类型', 'Service type')"><n-select v-model:value="endpointForm.kind" :options="[{ label: tr('通用 Git 服务（无平台 API）', 'Generic Git service (no platform API)'), value: 'genericGit' }, { label: 'GitHub', value: 'gitHub' }, { label: 'GitLab', value: 'gitLab' }, { label: 'Gitee', value: 'gitee' }, { label: 'Gitea', value: 'gitea' }, { label: 'Forgejo', value: 'forgejo' }]" /></n-form-item>
      <n-form-item :label="tr('显示名称', 'Display name')"><n-input v-model:value="endpointForm.displayName" :placeholder="tr('公司 Git 服务', 'Company Git service')" /></n-form-item>
      <n-form-item :label="tr('网页地址（通用 Git 可选）', 'Web URL (optional for generic Git)')"><n-input v-model:value="endpointForm.webUrl" placeholder="https://git.example.com" /></n-form-item>
      <n-form-item v-if="endpointForm.kind !== 'genericGit'" :label="tr('API 地址', 'API URL')"><n-input v-model:value="endpointForm.apiUrl" placeholder="https://gitlab.example.com/api/v4" /></n-form-item>
      <n-form-item :label="tr('Git host（可含端口）', 'Git host (port optional)')"><n-input v-model:value="endpointForm.gitHost" placeholder="git.example.com:8443" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="customEndpointModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="saveCustomEndpoint">{{ tr("保存 Endpoint", "Save endpoint") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="branchModalShown" preset="card" :title="tr('新建分支', 'New branch')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top"><n-form-item :label="tr('分支名称', 'Branch name')"><n-input v-model:value="branchForm.name" placeholder="feature/description" /></n-form-item><n-form-item><n-checkbox v-model:checked="branchForm.checkout">{{ tr("创建后立即切换到此分支", "Switch to this branch after creation") }}</n-checkbox></n-form-item></n-form>
    <template #action><n-space justify="end"><n-button @click="branchModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="createBranch">{{ tr("创建分支", "Create branch") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="renameBranchModalShown" preset="card" :title="tr('重命名分支', 'Rename branch')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="tr('当前名称', 'Current name')"><n-input :value="renameBranchForm.oldName" readonly /></n-form-item>
      <n-form-item :label="tr('新名称', 'New name')"><n-input v-model:value="renameBranchForm.newName" placeholder="feature/description" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="renameBranchModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="renameBranch">{{ tr("重命名", "Rename") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="stashModalShown" preset="card" :title="tr('保存改动到暂存栈', 'Stash changes')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ tr("暂存栈会从工作区移走当前改动，之后可在“历史与暂存栈”中恢复。", "Stash removes current changes from the working tree. You can restore them later from History & stash.") }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="tr('说明（可选）', 'Message (optional)')"><n-input v-model:value="stashForm.message" :placeholder="tr('例如：切换任务前的工作进度', 'For example: work in progress before switching tasks')" /></n-form-item>
      <n-form-item><n-checkbox v-model:checked="stashForm.includeUntracked">{{ tr("同时保存未跟踪文件", "Include untracked files") }}</n-checkbox></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="stashModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="stashChanges">{{ tr("保存改动", "Stash changes") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="rebaseModalShown" preset="card" :title="tr('变基当前分支', 'Rebase current branch')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="warning" :bordered="false" class="token-alert">
      {{ tr("变基会改写当前分支的提交历史。若该分支已推送，请在确认团队协作约定后使用 force-with-lease 推送。", "Rebase rewrites the current branch history. If it was pushed already, confirm your team workflow before force-with-lease pushing it.") }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="tr('变基目标分支', 'Base branch')"><n-select v-model:value="rebaseForm.base" :options="rebaseBaseOptions" :placeholder="tr('选择本地分支', 'Select a local branch')" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="rebaseModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="startRebase">{{ tr("开始变基", "Start rebase") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="cherryPickModalShown" preset="card" :title="tr('拣选提交', 'Cherry-pick commit')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ tr("输入另一个分支或远程跟踪分支上的提交 SHA。若发生冲突，应用会保留状态以便你解决后继续或中止。", "Enter a commit SHA from another local or remote-tracking branch. If it conflicts, the app preserves the state so you can resolve, continue, or abort.") }}
    </n-alert>
    <n-form label-placement="top"><n-form-item :label="tr('提交 SHA 或引用', 'Commit SHA or revision')"><n-input v-model:value="cherryPickForm.revision" placeholder="a1b2c3d" /></n-form-item></n-form>
    <template #action><n-space justify="end"><n-button @click="cherryPickModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="cherryPickCommits">{{ tr("拣选提交", "Cherry-pick") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="worktreeModalShown" preset="card" :title="tr('新建工作树', 'New worktree')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">
      {{ tr("工作树让你同时在不同目录检出同一仓库的不同分支。每个分支一次只能在一个工作树中检出。", "Worktrees let you check out different branches of the same repository in separate folders. A branch can be checked out in only one worktree at a time.") }}
    </n-alert>
    <n-form label-placement="top">
      <n-form-item :label="tr('新工作树目录', 'New worktree folder')"><n-input v-model:value="worktreeForm.path" placeholder="/Users/you/Projects/repository-feature" /></n-form-item>
      <n-form-item :label="tr('检出方式', 'Checkout mode')"><n-select v-model:value="worktreeForm.mode" :options="[{ label: tr('创建新分支', 'Create a new branch'), value: 'new' }, { label: tr('检出现有分支', 'Check out an existing branch'), value: 'existing' }]" /></n-form-item>
      <n-form-item v-if="worktreeForm.mode === 'new'" :label="tr('新分支名称', 'New branch name')"><n-input v-model:value="worktreeForm.branch" placeholder="feature/description" /></n-form-item>
      <n-form-item v-else :label="tr('现有分支', 'Existing branch')"><n-select v-model:value="worktreeForm.branch" :options="worktreeExistingBranchOptions" :placeholder="tr('选择未被检出的分支', 'Select a branch not checked out elsewhere')" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="worktreeModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="addWorktree">{{ tr("创建工作树", "Create worktree") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="lfsTrackModalShown" preset="card" :title="tr('添加 Git LFS 跟踪规则', 'Add Git LFS tracking rule')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="info" :bordered="false" class="token-alert">{{ tr("规则会写入 .gitattributes；提交前请在改动面板审查它。", "The rule is written to .gitattributes. Review it in the Changes panel before committing.") }}</n-alert>
    <n-form label-placement="top"><n-form-item :label="tr('文件模式', 'File pattern')"><n-input v-model:value="lfsTrackForm.pattern" placeholder="*.psd" /></n-form-item></n-form>
    <template #action><n-space justify="end"><n-button @click="lfsTrackModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="trackLfsPattern">{{ tr("添加规则", "Add rule") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="pullRequestModalShown" preset="card" :title="tr('创建 Pull Request', 'Create pull request')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="warning" :bordered="false" class="token-alert">{{ tr("此操作会通过当前匹配的 gh 账号在 GitHub 上创建 Pull Request。请先确认当前分支已推送。", "This creates a pull request on GitHub through the currently matched gh account. Confirm that the current branch has been pushed first.") }}</n-alert>
    <n-form label-placement="top">
      <n-form-item :label="tr('源分支', 'Head branch')"><n-input :value="snapshot?.status.branch || ''" readonly /></n-form-item>
      <n-form-item :label="tr('目标分支', 'Base branch')">
        <div class="pull-request-base-field">
          <n-input v-model:value="pullRequestForm.base" placeholder="main" />
          <n-space v-if="pullRequestBaseOptions.length" size="small" wrap class="pull-request-base-suggestions">
            <n-button v-for="option in pullRequestBaseOptions" :key="option.value" size="tiny" tertiary @click="pullRequestForm.base = option.value">{{ option.label }}</n-button>
          </n-space>
        </div>
      </n-form-item>
      <n-form-item :label="tr('标题', 'Title')"><n-input v-model:value="pullRequestForm.title" :placeholder="tr('简洁说明这次改动', 'Briefly describe this change')" /></n-form-item>
      <n-form-item :label="tr('说明（可选）', 'Description (optional)')"><n-input v-model:value="pullRequestForm.body" type="textarea" :autosize="{ minRows: 4, maxRows: 10 }" /></n-form-item>
      <n-form-item><n-checkbox v-model:checked="pullRequestForm.draft">{{ tr("创建为草稿 Pull Request", "Create as a draft pull request") }}</n-checkbox></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="pullRequestModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="createPullRequest">{{ tr("创建 Pull Request", "Create pull request") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="publishModalShown" preset="card" :title="tr('发布到 GitHub', 'Publish to GitHub')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="warning" :bordered="false" class="token-alert">{{ tr("此操作会通过所选 gh CLI 身份在 GitHub 创建仓库、添加 origin，并推送当前分支。不会切换全局 gh 活跃账号。", "This creates a GitHub repository through the selected gh CLI identity, adds origin, and pushes the current branch. It never switches the global active gh account.") }}</n-alert>
    <n-form label-placement="top">
      <n-form-item :label="tr('GitHub Endpoint', 'GitHub endpoint')"><n-select v-model:value="publishForm.endpointId" :options="publishEndpoints.map((endpoint) => ({ label: endpoint.displayName, value: endpoint.id }))" /></n-form-item>
      <n-form-item :label="tr('发布所用 gh 账号', 'gh account used to publish')"><n-select v-model:value="publishForm.accountId" :options="publishAccountOptions" :placeholder="tr('选择已验证的 gh 账号', 'Select a verified gh account')" /></n-form-item>
      <n-form-item :label="tr('仓库名称', 'Repository name')"><n-input v-model:value="publishForm.name" placeholder="desktop-client 或 owner/desktop-client" /></n-form-item>
      <n-form-item><n-checkbox v-model:checked="publishForm.isPrivate">{{ tr("创建私有仓库", "Create a private repository") }}</n-checkbox></n-form-item>
    </n-form>
    <n-alert v-if="!publishAccountOptions.length" type="error" :bordered="false">{{ tr("没有可用的 gh CLI 账号。请先在“账号与认证”中导入已登录的 gh 身份。", "No usable gh CLI account is available. Import a signed-in gh identity from Accounts & authentication first.") }}</n-alert>
    <template #action><n-space justify="end"><n-button @click="publishModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" :disabled="!publishAccountOptions.length" @click="publishGitHubRepository">{{ tr("创建并发布", "Create and publish") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="squashModalShown" preset="card" :title="tr('压缩最近提交', 'Squash recent commits')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-alert type="warning" :bordered="false" class="token-alert">{{ tr("此操作会重写当前分支历史，且要求工作区干净。若该分支已经推送，请在团队确认后使用 force-with-lease 推送。若 Git hooks 拒绝新提交，压缩后的改动会保留在暂存区供你处理。", "This rewrites current branch history and requires a clean worktree. If the branch was pushed already, confirm with your team before force-with-lease pushing. If Git hooks reject the new commit, the squashed changes remain staged for you to handle.") }}</n-alert>
    <n-form label-placement="top">
      <n-form-item :label="tr('压缩最近多少个提交', 'Number of recent commits to squash')"><n-input v-model:value="squashForm.count" inputmode="numeric" placeholder="2" /></n-form-item>
      <n-form-item :label="tr('新的提交说明', 'New commit message')"><n-input v-model:value="squashForm.message" type="textarea" :autosize="{ minRows: 2, maxRows: 5 }" /></n-form-item>
    </n-form>
    <template #action><n-space justify="end"><n-button @click="squashModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="squashRecentCommits">{{ tr("压缩提交", "Squash commits") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="tagModalShown" preset="card" :title="tr('新建标签', 'New tag')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="tr('标签名称', 'Tag name')"><n-input v-model:value="tagForm.name" placeholder="v1.0.0" /></n-form-item>
      <n-form-item :label="tr('目标引用', 'Target reference')"><n-select v-model:value="tagForm.target" :options="tagTargetOptions" /></n-form-item>
      <n-form-item :label="tr('注释（可选）', 'Annotation (optional)')"><n-input v-model:value="tagForm.message" type="textarea" :autosize="{ minRows: 2, maxRows: 4 }" :placeholder="tr('留空则创建轻量标签', 'Leave empty to create a lightweight tag')" /></n-form-item>
    </n-form>
    <n-alert type="info" :bordered="false">{{ tr("标签只在本地创建；推送请使用常规 Push 或 Git 命令。", "Tags are created locally only. Push them with your usual Push workflow or Git command.") }}</n-alert>
    <template #action><n-space justify="end"><n-button @click="tagModalShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="createTag">{{ tr("创建标签", "Create tag") }}</n-button></n-space></template>
  </n-modal>

  <n-modal v-model:show="remoteModalShown" preset="card" :title="editedRemoteName ? tr('编辑远程仓库', 'Edit remote') : tr('添加远程仓库', 'Add remote')" class="dialog-card" :mask-closable="!mutationBusy">
    <n-form label-placement="top">
      <n-form-item :label="tr('Remote 名称', 'Remote name')"><n-input v-model:value="remoteForm.name" placeholder="origin" /></n-form-item>
      <n-form-item :label="tr('Fetch 地址', 'Fetch URL')"><n-input v-model:value="remoteForm.fetchUrl" placeholder="https://git.example.com/org/repository.git" /></n-form-item>
      <n-form-item :label="tr('Push 地址（可选）', 'Push URL (optional)')"><n-input v-model:value="remoteForm.pushUrl" :placeholder="tr('留空则保持现有 push 地址', 'Leave empty to keep the existing push URL')" /></n-form-item>
    </n-form>
    <n-alert type="info" :bordered="false">{{ tr("远程地址中不能包含令牌或密码。账号认证请通过 remote 账号绑定处理。", "Remote URLs cannot contain tokens or passwords. Use a remote account binding for authentication.") }}</n-alert>
    <template #action>
      <n-space justify="space-between">
        <n-button v-if="editedRemoteName" type="error" tertiary :loading="mutationBusy" @click="confirmRemoveRemote">{{ tr("移除 remote", "Remove remote") }}</n-button>
        <span v-else />
        <n-space>
          <n-button @click="remoteModalShown = false">{{ tr("取消", "Cancel") }}</n-button>
          <n-button type="primary" :loading="mutationBusy" @click="saveRemote">{{ tr("保存 remote", "Save remote") }}</n-button>
        </n-space>
      </n-space>
    </template>
  </n-modal>

  <n-drawer v-model:show="bindingDrawerShown" :width="440" placement="right">
    <n-drawer-content :title="tr('Remote 账号绑定', 'Remote account binding')" closable>
      <template v-if="bindingRemote">
        <n-descriptions :column="1" size="small" label-placement="left">
          <n-descriptions-item :label="tr('远程仓库', 'Remote')">{{ bindingRemote.name }}</n-descriptions-item>
          <n-descriptions-item :label="tr('地址', 'URL')"><span class="drawer-url">{{ bindingRemote.fetchUrl }}</span></n-descriptions-item>
          <n-descriptions-item :label="tr('已识别平台', 'Recognized service')">{{ endpointKindLabel(bindingRemote.codeHostKind, appLocale) }}</n-descriptions-item>
        </n-descriptions>
          <n-divider />
          <n-form label-placement="top">
            <n-form-item :label="tr('Git 服务 Endpoint', 'Git service endpoint')"><n-select v-model:value="bindingForm.endpointId" :options="bindingEndpointOptions" /></n-form-item>
            <n-form-item :label="tr('此 remote 使用的账号', 'Account used by this remote')"><n-select v-model:value="bindingForm.accountId" :options="bindingAccountOptions" /></n-form-item>
          </n-form>
          <n-button text type="primary" @click="openCustomEndpointModal">{{ tr("添加 Git 服务 Endpoint", "Add Git service endpoint") }}</n-button>
        <n-alert type="info" :bordered="false">
          {{ tr("Auto 仅在后端可唯一确定有效账号时才会继续操作；多个候选或认证失效时一定会要求选择，不会静默使用错误账号。", "Auto continues only when the backend can determine exactly one valid account. Multiple candidates or expired authentication always require a choice; it never silently uses the wrong account.") }}
        </n-alert>
        <n-card v-if="bindingForm.accountId !== 'auto'" size="small" class="binding-note">
          {{ tr("选中的账号将覆盖该 Endpoint 的默认候选，仅作用于此仓库的 ", "The selected account overrides this endpoint's default candidate only for this repository's ") }}<strong>{{ bindingRemote.name }}</strong> {{ tr(" remote。", " remote.") }}
        </n-card>
      </template>
      <template #footer><n-space justify="end"><n-button @click="bindingDrawerShown = false">{{ tr("取消", "Cancel") }}</n-button><n-button type="primary" :loading="mutationBusy" @click="saveBinding">{{ tr("保存绑定", "Save binding") }}</n-button></n-space></template>
    </n-drawer-content>
  </n-drawer>
</template>
