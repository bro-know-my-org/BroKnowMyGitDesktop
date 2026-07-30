import { computed, h, type Ref } from "vue";
import { NButton, NTag, type DataTableColumns } from "naive-ui";
import { authenticationMethodLabel, authenticationStatusLabel, endpointKindLabel, type t as translate } from "../../../shared/i18n";
import {
  type AccountProfile,
  type BranchInfo,
  type PullRequestSummary,
  type RemoteBranchInfo,
  type RemoteProfile,
  type RepositorySnapshot,
  type StashEntry,
  type SubmoduleInfo,
  type WorktreeInfo,
} from "../../../shared/types/domain";

type Translate = typeof translate;
type MaybePromiseVoid = void | Promise<void>;

interface WorkspaceTableDependencies {
  t: Translate;
  snapshot: Ref<RepositorySnapshot | null>;
  checkoutBranch: (branch: BranchInfo) => MaybePromiseVoid;
  checkoutRemoteBranch: (branch: RemoteBranchInfo) => MaybePromiseVoid;
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
    t,
    snapshot,
    checkoutBranch,
    checkoutRemoteBranch,
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
    title: t("workspace.tables.branch"),
    key: "name",
    render: (branch) =>
      h(
        NButton,
        { text: true, type: branch.isCurrent ? "primary" : "default", onClick: () => void checkoutBranch(branch) },
        { default: () => branch.name },
      ),
  },
  {
    title: t("workspace.tables.upstream"),
    key: "upstream",
    render: (branch) => branch.upstream || "—",
  },
  {
    title: t("workspace.tables.status"),
    key: "isCurrent",
    width: 90,
    render: (branch) => (branch.isCurrent ? h(NTag, { size: "small", type: "primary" }, { default: () => t("workspace.tables.current") }) : ""),
  },
  {
    title: t("workspace.tables.actions"),
    key: "actions",
    width: 195,
    render: (branch) =>
      h("div", { class: "table-action-group" }, [
        ...(!branch.isCurrent
          ? [
              h(
                NButton,
                { size: "tiny", tertiary: true, onClick: () => confirmMergeBranch(branch) },
                { default: () => t("workspace.tables.merge") },
              ),
            ]
          : []),
        h(
          NButton,
          { size: "tiny", tertiary: true, onClick: () => openRenameBranchModal(branch) },
          { default: () => t("workspace.tables.rename") },
        ),
        ...(!branch.isCurrent
          ? [
              h(
                NButton,
                { size: "tiny", tertiary: true, type: "error", onClick: () => confirmDeleteBranch(branch) },
                { default: () => t("workspace.tables.delete") },
              ),
            ]
          : []),
      ]),
  },
]);

function localBranchForRemoteBranch(remoteBranch: RemoteBranchInfo): BranchInfo | undefined {
  return snapshot.value?.branches.find((branch) => branch.name === remoteBranch.name);
}

const remoteBranchColumns = computed<DataTableColumns<RemoteBranchInfo>>(() => [
  {
    title: t("workspace.tables.remoteBranch"),
    key: "name",
    minWidth: 220,
    render: (remoteBranch) => {
      const localBranch = localBranchForRemoteBranch(remoteBranch);
      return h(
        NButton,
        {
          text: true,
          type: localBranch?.isCurrent ? "primary" : "default",
          disabled: localBranch?.isCurrent,
          onClick: () => void checkoutRemoteBranch(remoteBranch),
        },
        { default: () => remoteBranch.name },
      );
    },
  },
  {
    title: t("workspace.tables.remote"),
    key: "remoteName",
    width: 130,
  },
  {
    title: t("workspace.tables.localStatus"),
    key: "localState",
    minWidth: 155,
    render: (remoteBranch) => {
      const localBranch = localBranchForRemoteBranch(remoteBranch);
      if (!localBranch) {
        return h(NTag, { size: "small", type: "warning" }, { default: () => t("workspace.tables.createOnSwitch") });
      }
      if (localBranch.isCurrent) {
        return h(NTag, { size: "small", type: "primary" }, { default: () => t("workspace.tables.currentBranch") });
      }
      return h(
        NTag,
        { size: "small", type: localBranch.upstream === `${remoteBranch.remoteName}/${remoteBranch.name}` ? "success" : "warning" },
        {
          default: () =>
            localBranch.upstream === `${remoteBranch.remoteName}/${remoteBranch.name}`
              ? t("workspace.tables.trackingLocally")
              : t("workspace.tables.sameLocalBranch"),
        },
      );
    },
  },
  {
    title: t("workspace.tables.actions"),
    key: "actions",
    width: 86,
    render: (remoteBranch) => {
      const localBranch = localBranchForRemoteBranch(remoteBranch);
      return h(
        NButton,
        {
          size: "tiny",
          tertiary: true,
          disabled: localBranch?.isCurrent,
          onClick: () => void checkoutRemoteBranch(remoteBranch),
        },
        { default: () => t("workspace.tables.switch") },
      );
    },
  },
]);

const worktreeColumns = computed<DataTableColumns<WorktreeInfo>>(() => [
  {
    title: t("workspace.tables.worktreePath"),
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
    title: t("workspace.tables.branch"),
    key: "branch",
    width: 180,
    render: (worktree) => worktree.branch || t("workspace.tables.detachedHead"),
  },
  {
    title: t("workspace.tables.state"),
    key: "state",
    width: 190,
    render: (worktree) =>
      h("div", { class: "table-action-group" }, [
        ...(worktree.isMain ? [h(NTag, { size: "small", type: "primary" }, { default: () => t("workspace.tables.main") })] : []),
        ...(worktree.isDetached ? [h(NTag, { size: "small", type: "warning" }, { default: () => t("workspace.tables.detached") })] : []),
        ...(worktree.isLocked ? [h(NTag, { size: "small", type: "warning" }, { default: () => t("workspace.tables.locked") })] : []),
        ...(worktree.isPrunable ? [h(NTag, { size: "small", type: "error" }, { default: () => t("workspace.tables.prunable") })] : []),
      ]),
  },
  {
    title: t("workspace.tables.actions"),
    key: "actions",
    width: 145,
    render: (worktree) =>
      h("div", { class: "table-action-group" }, [
        h(
          NButton,
          { size: "tiny", tertiary: true, onClick: () => void selectRepository(worktree.path) },
          { default: () => t("workspace.tables.open") },
        ),
        ...(!worktree.isMain && worktree.path !== snapshot.value?.repository.path
          ? [
              h(
                NButton,
                { size: "tiny", tertiary: true, type: "error", onClick: () => confirmRemoveWorktree(worktree) },
                { default: () => t("workspace.tables.remove") },
              ),
            ]
          : []),
      ]),
  },
]);

const submoduleColumns = computed<DataTableColumns<SubmoduleInfo>>(() => [
  { title: t("workspace.tables.path"), key: "path", minWidth: 260, ellipsis: { tooltip: true } },
  { title: t("workspace.tables.commit"), key: "commit", width: 150, ellipsis: { tooltip: true } },
  {
    title: t("workspace.tables.status"),
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
  { title: t("workspace.tables.remoteRepository"), key: "name", width: 120 },
  {
    title: t("workspace.tables.service"),
    key: "codeHostKind",
    width: 120,
    render: (remote) => h(NTag, { size: "small", bordered: false }, { default: () => endpointKindLabel(remote.codeHostKind) }),
  },
  {
    title: t("workspace.tables.accountBinding"),
    key: "binding",
    minWidth: 180,
    render: (remote) => remoteBindingLabel(remote),
  },
  {
    title: t("workspace.tables.actions"),
    key: "action",
    width: 165,
    render: (remote) =>
      h("div", { class: "table-action-group" }, [
        h(
          NButton,
          { size: "tiny", tertiary: true, onClick: () => openBindingDrawer(remote) },
          { default: () => t("workspace.tables.bindAccount") },
        ),
        h(
          NButton,
          { size: "tiny", tertiary: true, onClick: () => openRemoteEditor(remote) },
          { default: () => t("workspace.tables.edit") },
        ),
      ]),
  },
]);

const accountColumns = computed<DataTableColumns<AccountProfile>>(() => [
  {
    title: t("workspace.tables.account"),
    key: "login",
    minWidth: 180,
    render: (account) => account.displayName || account.login,
  },
  {
    title: t("workspace.tables.service"),
    key: "endpointId",
    width: 130,
    render: (account) => endpointLabel(account.endpointId),
  },
  {
    title: t("workspace.tables.authentication"),
    key: "authMethod",
    minWidth: 150,
    render: (account) => authenticationMethodLabel(account.authMethod),
  },
  {
    title: t("workspace.tables.authenticationStatus"),
    key: "status",
    width: 125,
    render: (account) =>
      h(
        NTag,
        { size: "small", type: authenticationTagType(account.status) },
        { default: () => authenticationStatusLabel(account.status) },
      ),
  },
  {
    title: t("workspace.tables.defaultCandidate"),
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
        { default: () => (account.isDefaultCandidate ? t("workspace.tables.defaultCandidate") : t("workspace.tables.setAsCandidate")) },
      ),
  },
  {
    title: t("workspace.tables.actions"),
    key: "actions",
    width: 80,
    render: (account) =>
      h(
        NButton,
        { size: "tiny", text: true, type: "error", onClick: () => confirmRemoveAccount(account) },
        { default: () => t("workspace.tables.remove") },
      ),
  },
]);

const pullRequestColumns = computed<DataTableColumns<PullRequestSummary>>(() => [
  { title: "#", key: "number", width: 64 },
  { title: t("workspace.tables.title"), key: "title", minWidth: 260 },
  { title: t("workspace.tables.author"), key: "author", width: 120 },
  { title: t("workspace.tables.status"), key: "state", width: 90 },
  { title: t("workspace.tables.updated"), key: "updatedAt", width: 110, render: (item) => item.updatedAt || "—" },
]);

const stashColumns = computed<DataTableColumns<StashEntry>>(() => [
  { title: t("workspace.tables.reference"), key: "reference", width: 105 },
  { title: t("workspace.tables.stashMessage"), key: "message", minWidth: 210, ellipsis: { tooltip: true } },
  {
    title: t("workspace.tables.actions"),
    key: "actions",
    width: 140,
    render: (stash) =>
      h("div", { class: "table-action-group" }, [
        h(
          NButton,
          { size: "tiny", tertiary: true, onClick: () => confirmPopStash(stash) },
          { default: () => t("workspace.tables.pop") },
        ),
        h(
          NButton,
          { size: "tiny", tertiary: true, type: "error", onClick: () => confirmDropStash(stash) },
          { default: () => t("workspace.tables.drop") },
        ),
      ]),
  },
]);

  return {
    branchColumns,
    remoteBranchColumns,
    worktreeColumns,
    submoduleColumns,
    remoteColumns,
    accountColumns,
    pullRequestColumns,
    stashColumns,
  };
}
