import { computed, h, type Ref } from "vue";
import { NButton, NTag, type DataTableColumns } from "naive-ui";
import type { AppLocale, tr as translate } from "../../../shared/i18n";
import {
  authenticationMethodLabel,
  authenticationStatusLabel,
  endpointKindLabel,
  type AccountProfile,
  type BranchInfo,
  type PullRequestSummary,
  type RemoteProfile,
  type RepositorySnapshot,
  type StashEntry,
  type SubmoduleInfo,
  type WorktreeInfo,
} from "../../../shared/types/domain";

type Translate = typeof translate;
type MaybePromiseVoid = void | Promise<void>;

interface WorkspaceTableDependencies {
  tr: Translate;
  appLocale: Ref<AppLocale>;
  snapshot: Ref<RepositorySnapshot | null>;
  checkoutBranch: (branch: BranchInfo) => MaybePromiseVoid;
  confirmMergeBranch: (branch: BranchInfo) => void;
  openRenameBranchModal: (branch: BranchInfo) => void;
  confirmDeleteBranch: (branch: BranchInfo) => void;
  selectRepository: (path: string) => MaybePromiseVoid;
  confirmRemoveWorktree: (worktree: WorktreeInfo) => void;
  submoduleStatusLabel: (status: SubmoduleInfo["status"]) => string;
  submoduleStatusTagType: (status: SubmoduleInfo["status"]) => "success" | "warning" | "error";
  remoteBindingLabel: (remote: RemoteProfile) => string;
  openBindingDrawer: (remote: RemoteProfile) => void;
  openRemoteEditor: (remote?: RemoteProfile) => void;
  endpointLabel: (endpointId: string) => string;
  authenticationTagType: (status: AccountProfile["status"]) => "default" | "success" | "warning" | "error";
  setDefaultCandidate: (account: AccountProfile) => MaybePromiseVoid;
  confirmRemoveAccount: (account: AccountProfile) => void;
  confirmPopStash: (stash: StashEntry) => void;
  confirmDropStash: (stash: StashEntry) => void;
}

export function useWorkspaceTables(dependencies: WorkspaceTableDependencies) {
  const {
    tr,
    appLocale,
    snapshot,
    checkoutBranch,
    confirmMergeBranch,
    openRenameBranchModal,
    confirmDeleteBranch,
    selectRepository,
    confirmRemoveWorktree,
    submoduleStatusLabel,
    submoduleStatusTagType,
    remoteBindingLabel,
    openBindingDrawer,
    openRemoteEditor,
    endpointLabel,
    authenticationTagType,
    setDefaultCandidate,
    confirmRemoveAccount,
    confirmPopStash,
    confirmDropStash,
  } = dependencies;

const branchColumns = computed<DataTableColumns<BranchInfo>>(() => [
  {
    title: tr("分支", "Branch"),
    key: "name",
    render: (branch) =>
      h(
        NButton,
        { text: true, type: branch.isCurrent ? "primary" : "default", onClick: () => void checkoutBranch(branch) },
        { default: () => branch.name },
      ),
  },
  {
    title: tr("跟踪分支", "Upstream"),
    key: "upstream",
    render: (branch) => branch.upstream || "—",
  },
  {
    title: tr("状态", "Status"),
    key: "isCurrent",
    width: 90,
    render: (branch) => (branch.isCurrent ? h(NTag, { size: "small", type: "success" }, { default: () => tr("当前", "Current") }) : ""),
  },
  {
    title: tr("操作", "Actions"),
    key: "actions",
    width: 195,
    render: (branch) =>
      h("div", { class: "table-action-group" }, [
        ...(!branch.isCurrent
          ? [
              h(
                NButton,
                { size: "tiny", tertiary: true, onClick: () => confirmMergeBranch(branch) },
                { default: () => tr("合并", "Merge") },
              ),
            ]
          : []),
        h(
          NButton,
          { size: "tiny", tertiary: true, onClick: () => openRenameBranchModal(branch) },
          { default: () => tr("重命名", "Rename") },
        ),
        ...(!branch.isCurrent
          ? [
              h(
                NButton,
                { size: "tiny", tertiary: true, type: "error", onClick: () => confirmDeleteBranch(branch) },
                { default: () => tr("删除", "Delete") },
              ),
            ]
          : []),
      ]),
  },
]);

const worktreeColumns = computed<DataTableColumns<WorktreeInfo>>(() => [
  {
    title: tr("工作树路径", "Worktree path"),
    key: "path",
    minWidth: 280,
    ellipsis: { tooltip: true },
    render: (worktree) =>
      h(
        NButton,
        { text: true, onClick: () => void selectRepository(worktree.path) },
        { default: () => worktree.path },
      ),
  },
  {
    title: tr("分支", "Branch"),
    key: "branch",
    width: 180,
    render: (worktree) => worktree.branch || tr("分离 HEAD", "Detached HEAD"),
  },
  {
    title: tr("状态", "State"),
    key: "state",
    width: 190,
    render: (worktree) =>
      h("div", { class: "table-action-group" }, [
        ...(worktree.isMain ? [h(NTag, { size: "small", type: "success" }, { default: () => tr("主工作树", "Main") })] : []),
        ...(worktree.isDetached ? [h(NTag, { size: "small", type: "warning" }, { default: () => tr("分离", "Detached") })] : []),
        ...(worktree.isLocked ? [h(NTag, { size: "small", type: "warning" }, { default: () => tr("已锁定", "Locked") })] : []),
        ...(worktree.isPrunable ? [h(NTag, { size: "small", type: "error" }, { default: () => tr("可清理", "Prunable") })] : []),
      ]),
  },
  {
    title: tr("操作", "Actions"),
    key: "actions",
    width: 145,
    render: (worktree) =>
      h("div", { class: "table-action-group" }, [
        h(
          NButton,
          { size: "tiny", tertiary: true, onClick: () => void selectRepository(worktree.path) },
          { default: () => tr("打开", "Open") },
        ),
        ...(!worktree.isMain && worktree.path !== snapshot.value?.repository.path
          ? [
              h(
                NButton,
                { size: "tiny", tertiary: true, type: "error", onClick: () => confirmRemoveWorktree(worktree) },
                { default: () => tr("移除", "Remove") },
              ),
            ]
          : []),
      ]),
  },
]);

const submoduleColumns = computed<DataTableColumns<SubmoduleInfo>>(() => [
  { title: tr("路径", "Path"), key: "path", minWidth: 260, ellipsis: { tooltip: true } },
  { title: tr("提交", "Commit"), key: "commit", width: 150, ellipsis: { tooltip: true } },
  {
    title: tr("状态", "Status"),
    key: "status",
    width: 130,
    render: (submodule) =>
      h(
        NTag,
        { size: "small", type: submoduleStatusTagType(submodule.status) },
        { default: () => submoduleStatusLabel(submodule.status) },
      ),
  },
]);

const remoteColumns = computed<DataTableColumns<RemoteProfile>>(() => [
  { title: tr("远程仓库", "Remote"), key: "name", width: 120 },
  {
    title: tr("平台", "Service"),
    key: "codeHostKind",
    width: 120,
    render: (remote) => h(NTag, { size: "small", bordered: false }, { default: () => endpointKindLabel(remote.codeHostKind, appLocale.value) }),
  },
  {
    title: tr("认证绑定", "Account binding"),
    key: "binding",
    minWidth: 180,
    render: (remote) => remoteBindingLabel(remote),
  },
  {
    title: tr("操作", "Actions"),
    key: "action",
    width: 165,
    render: (remote) =>
      h("div", { class: "table-action-group" }, [
        h(
          NButton,
          { size: "tiny", tertiary: true, onClick: () => openBindingDrawer(remote) },
          { default: () => tr("绑定账号", "Bind account") },
        ),
        h(
          NButton,
          { size: "tiny", tertiary: true, onClick: () => openRemoteEditor(remote) },
          { default: () => tr("编辑", "Edit") },
        ),
      ]),
  },
]);

const accountColumns = computed<DataTableColumns<AccountProfile>>(() => [
  {
    title: tr("账号", "Account"),
    key: "login",
    minWidth: 180,
    render: (account) => account.displayName || account.login,
  },
  {
    title: tr("平台", "Service"),
    key: "endpointId",
    width: 130,
    render: (account) => endpointLabel(account.endpointId),
  },
  {
    title: tr("认证方式", "Authentication"),
    key: "authMethod",
    minWidth: 150,
    render: (account) => authenticationMethodLabel(account.authMethod, appLocale.value),
  },
  {
    title: tr("认证状态", "Authentication status"),
    key: "status",
    width: 125,
    render: (account) =>
      h(
        NTag,
        { size: "small", type: authenticationTagType(account.status) },
        { default: () => authenticationStatusLabel(account.status, appLocale.value) },
      ),
  },
  {
    title: tr("默认候选", "Default candidate"),
    key: "isDefaultCandidate",
    width: 120,
    render: (account) =>
      h(
        NButton,
        {
          size: "tiny",
          tertiary: !account.isDefaultCandidate,
          type: account.isDefaultCandidate ? "primary" : "default",
          onClick: () => void setDefaultCandidate(account),
        },
        { default: () => (account.isDefaultCandidate ? tr("默认候选", "Default candidate") : tr("设为候选", "Set as candidate")) },
      ),
  },
  {
    title: tr("操作", "Actions"),
    key: "actions",
    width: 80,
    render: (account) =>
      h(
        NButton,
        { size: "tiny", text: true, type: "error", onClick: () => confirmRemoveAccount(account) },
        { default: () => tr("移除", "Remove") },
      ),
  },
]);

const pullRequestColumns = computed<DataTableColumns<PullRequestSummary>>(() => [
  { title: "#", key: "number", width: 64 },
  { title: tr("标题", "Title"), key: "title", minWidth: 260 },
  { title: tr("作者", "Author"), key: "author", width: 120 },
  { title: tr("状态", "Status"), key: "state", width: 90 },
  { title: tr("更新", "Updated"), key: "updatedAt", width: 110, render: (item) => item.updatedAt || "—" },
]);

const stashColumns = computed<DataTableColumns<StashEntry>>(() => [
  { title: tr("引用", "Reference"), key: "reference", width: 105 },
  { title: tr("说明", "Message"), key: "message", minWidth: 210, ellipsis: { tooltip: true } },
  {
    title: tr("操作", "Actions"),
    key: "actions",
    width: 140,
    render: (stash) =>
      h("div", { class: "table-action-group" }, [
        h(
          NButton,
          { size: "tiny", tertiary: true, onClick: () => confirmPopStash(stash) },
          { default: () => tr("恢复", "Pop") },
        ),
        h(
          NButton,
          { size: "tiny", tertiary: true, type: "error", onClick: () => confirmDropStash(stash) },
          { default: () => tr("删除", "Drop") },
        ),
      ]),
  },
]);

  return {
    branchColumns,
    worktreeColumns,
    submoduleColumns,
    remoteColumns,
    accountColumns,
    pullRequestColumns,
    stashColumns,
  };
}
