import type {
  AccountProfile,
  CommitSummary,
  GitStatusEntry,
  LfsStatus,
  PullRequestSummary,
  RepositoryRecord,
  RepositorySnapshot,
  SubmoduleInfo,
  WorktreeInfo,
} from "../types/domain";

export const demoMarkers = {
  repositoryAlias: "__bkm_demo_repository_alias__",
  repositoryGroup: "__bkm_demo_repository_group__",
  commitMultiAccount: "__bkm_demo_commit_multi_account__",
  commitInitial: "__bkm_demo_commit_initial__",
  updatedNow: "__bkm_demo_updated_now__",
  accountPersonal: "__bkm_demo_account_personal__",
  accountGitHubCli: "__bkm_demo_account_github_cli__",
  accountWork: "__bkm_demo_account_work__",
} as const;


const sampleRepository: RepositoryRecord = {
  path: "/Users/you/Projects/broknowmygitdesktop",
  name: "BroKnowMyGitDesktop",
  alias: demoMarkers.repositoryAlias,
  group: demoMarkers.repositoryGroup,
  host: "gitee.com",
  namespace: "broknowmyorg",
  remoteName: "origin",
  remoteRepository: "broknowmygitdesktop",
  remoteHost: "gitee.com",
  remoteNamespace: "broknowmyorg",
  lastOpenedAt: Date.now() - 1000 * 60 * 18,
};

const sampleEntries: GitStatusEntry[] = [
  {
    path: "src/App.vue",
    originalPath: null,
    indexStatus: "M",
    worktreeStatus: "M",
  },
  {
    path: "src-tauri/src/domain.rs",
    originalPath: null,
    indexStatus: "",
    worktreeStatus: "M",
  },
  {
    path: "docs/architecture.md",
    originalPath: null,
    indexStatus: "A",
    worktreeStatus: "",
  },
];

export const mockSnapshot: RepositorySnapshot = {
  repository: sampleRepository,
  hasHead: true,
  status: {
    headCommit: "9d7a4f0c2ea846105d1200f9cf6a4eb68d748301",
    branch: "main",
    upstream: "origin/main",
    upstreamCommit: "55b91f2b0e1a6540dcdb43f1f7db2e3a22d7db10",
    ahead: 1,
    behind: 0,
    entries: sampleEntries,
  },
  branches: [
    { name: "main", isCurrent: true, upstream: "origin/main" },
    { name: "feature/multi-account", isCurrent: false, upstream: "origin/feature/multi-account" },
  ],
  remoteBranches: [
    { remoteName: "origin", name: "feature/multi-account" },
    { remoteName: "origin", name: "main" },
    { remoteName: "upstream", name: "main" },
  ],
  remotes: [
    {
      name: "origin",
      fetchUrl: "git@gitee.com:broknowmyorg/broknowmygitdesktop.git",
      pushUrl: "git@gitee.com:broknowmyorg/broknowmygitdesktop.git",
      parsed: {
        transport: "ssh",
        host: "gitee.com",
        port: null,
        namespace: "broknowmyorg",
        repository: "broknowmygitdesktop",
      },
      endpointId: "gitee.com",
      codeHostKind: "gitee",
    },
    {
      name: "upstream",
      fetchUrl: "https://github.com/broknowmyorg/broknowmygitdesktop.git",
      pushUrl: "https://github.com/broknowmyorg/broknowmygitdesktop.git",
      parsed: {
        transport: "https",
        host: "github.com",
        port: null,
        namespace: "broknowmyorg",
        repository: "broknowmygitdesktop",
      },
      endpointId: "github.com",
      codeHostKind: "gitHub",
    },
  ],
  tags: [{ name: "v0.1.0", target: "main" }],
  bindings: [
    {
      repositoryPath: sampleRepository.path,
      remoteName: "origin",
      endpointId: "gitee.com",
      accountSelection: { mode: "profile", accountProfileId: "gitee-personal" },
    },
    {
      repositoryPath: sampleRepository.path,
      remoteName: "upstream",
      endpointId: "github.com",
      accountSelection: { mode: "auto" },
    },
  ],
  operation: null,
};


export const initialBrowserCommits: CommitSummary[] = [
  {
    sha: "9d7a4f0c2ea846105d1200f9cf6a4eb68d748301",
    shortSha: "9d7a4f0",
    parents: ["55b91f2b0e1a6540dcdb43f1f7db2e3a22d7db10"],
    tags: ["v0.1.0"],
    tagsTruncated: false,
    pushed: false,
    authorName: "BroKnowMy",
    authorEmail: "broknowmy@example.test",
    authoredAt: "2026-07-19T10:20:30+08:00",
    summary: demoMarkers.commitMultiAccount,
  },
  {
    sha: "55b91f2b0e1a6540dcdb43f1f7db2e3a22d7db10",
    shortSha: "55b91f2",
    parents: [],
    tags: [],
    tagsTruncated: false,
    pushed: true,
    authorName: "BroKnowMy",
    authorEmail: "broknowmy@example.test",
    authoredAt: "2026-07-18T09:00:00+08:00",
    summary: demoMarkers.commitInitial,
  },
];

export const initialBrowserWorktrees: WorktreeInfo[] = [
  {
    path: sampleRepository.path,
    head: initialBrowserCommits[0].sha,
    branch: "main",
    isMain: true,
    isDetached: false,
    isLocked: false,
    isPrunable: false,
  },
];
export const initialBrowserSubmodules: SubmoduleInfo[] = [];
export const initialBrowserLfsStatus: LfsStatus = {
  available: true,
  initialized: false,
  version: "git-lfs/3.6.1 (browser demo)",
  files: [],
};

export const initialBrowserPullRequests: PullRequestSummary[] = [
  {
    number: 12,
    title: demoMarkers.commitMultiAccount,
    author: "broknowmy",
    state: "OPEN",
    webUrl: "https://github.com/broknowmyorg/broknowmygitdesktop/pull/12",
    updatedAt: demoMarkers.updatedNow,
  },
];
export const initialBrowserAccounts: AccountProfile[] = [
  {
    id: "gitee-personal",
    endpointId: "gitee.com",
    login: "broknowmy",
    displayName: demoMarkers.accountPersonal,
    authMethod: "personalAccessToken",
    secretRef: { service: "io.github.broknowmyorg.broknowmygitdesktop", account: "gitee-personal" },
    status: "valid",
    isDefaultCandidate: true,
  },
  {
    id: "github-cli",
    endpointId: "github.com",
    login: "broknowmy",
    displayName: demoMarkers.accountGitHubCli,
    authMethod: "ghCli",
    secretRef: null,
    status: "available",
    isDefaultCandidate: true,
  },
  {
    id: "gitlab-cli",
    endpointId: "gitlab.com",
    login: "broknowmy-work",
    displayName: demoMarkers.accountWork,
    authMethod: "glabCli",
    secretRef: null,
    status: "needsReauthentication",
    isDefaultCandidate: false,
  },
];

