# BroKnowMyGitDesktop 架构

BroKnowMyGitDesktop 是一个以本地 Git 为核心、以代码托管平台能力为补充的桌面客户端。Git 操作始终通过用户安装的 `git` 可执行文件完成；平台功能通过可替换的 CodeHostAdapter 实现，绝不把 GitHub CLI 当作其他平台的通用依赖。GitHub、GitLab、Gitee 是内建优化，不是可连接的 Git host 边界。

当前交付范围、回归步骤和后续优先级见 [development-status.md](development-status.md)。

## 模块边界

```text
Vue 3 + Naive UI
        │ Tauri IPC（不返回密钥）
        ▼
Commands ── AppState（安全的账号、绑定、Endpoint 与本地仓库元数据）
   ├── GitCore ────────────── system git（参数数组）
   ├── CredentialBroker ───── SSH agent / Git helper / gh / glab / 系统钥匙串
   └── CodeHostAdapter ────── GitHub、GitLab、Gitee、后续 Gitea/Forgejo
                                  │
                                  └── 各平台 API 或官方 CLI
```

### GitCore

- 只以 `std::process::Command` 加参数数组启动系统 `git`，从不拼接 shell 命令。
- 覆盖仓库发现、clone、status、diff、暂存/取消暂存、commit、分支、merge、fetch/pull/push、tag 和 remote 管理。
- 不使用 libgit2，因此保留用户的 SSH、Git LFS、正常 hook 与 credential helper 行为；使用应用管理 PAT 的网络命令会以命令级 `core.hooksPath` 禁用 hook，避免 capability 被仓库代码读取。
- GitCore 不知道平台令牌，也不读写 Git 全局配置。

### CredentialBroker

CredentialBroker 在每次网络 Git 操作前，将 remote URL、RemoteBinding 和 AccountProfile 解析成明确的 CredentialStrategy。它不修改 `git config --global`，也绝不执行 `gh auth switch` 或 `glab auth switch`。

支持的策略：

- `SshAgent`：沿用进程环境中的 SSH agent，但实际身份由 SSH host 配置与 agent 选择。SSH 账号资料仅可显示状态，不能成为 remote 的显式 Profile binding，也不参与 Auto 的账号选择。
- `SystemGitCredentialHelper`：不是 `CredentialStrategy`，也不能保存为“某个账号”的 remote binding；没有明确 PAT 候选时，系统 Git 只按实际 remote URL 调用用户已有 helper。
- `GhCli` / `GlabCli`：检测、导入并重新校验当前 CLI 账号，并用于各自已实现的平台 API（GitHub 的发布也依赖 gh）；不会切换 CLI 活跃账号。由于 CLI token 命令无法安全固定到某个账号，它们绝不作为 Git HTTPS askpass 来源；需要显式账号绑定的 Git 网络操作应使用 PAT，SSH 则保持 Git/SSH 配置的无绑定选择。
- `KeychainPatAskPass`：应用保存的 PAT 只放进系统钥匙串。父 Git 路径为临时 askpass 子进程签发短时、随机、一次性 capability；Git 需要 HTTPS 密码时，helper 先原子消费 capability，再核对 capability 中的 account profile ID 与钥匙串引用一致后读取并直接写给 Git。令牌不经过前端、日志、JSON、SQLite、Git remote 或 Git 配置。

账号与本地仓库元数据存放在应用数据目录的 JSON 中。账号部分只包含账号 ID、平台、登录名、认证方式、状态和钥匙串引用；绝不包含 PAT。仓库部分只保存规范化路径、显示名、手动分组、已解析 remote 的 host/组织路径和最近显式打开时间；它不会写入 Git 配置。本地持久化采用原子写入，避免半写入的账号绑定或仓库分组；Unix 会同步文件和父目录，macOS 不把系统对目录 `fsync` 返回的 `EINVAL` 误报成写入失败。同一仓库的工作树 / index / ref 写入、凭据化网络 Git 操作、remote / binding 修改，以及依赖 remote 身份的 GitHub 平台请求会持有同一个进程内 lease；移除 linked worktree 时会原子取得发起端与目标端两把路径锁，避免在强制删除时让目标工作树继续操作。GitHub 平台能力还要求 `api_url` 精确匹配 `gh --hostname` 对该 Git host 的标准路由，避免把 host-scoped CLI token 发到 Endpoint 配置之外的 API。

### CodeHostAdapter

所有平台能力统一由 `CodeHostAdapter` trait 提供。每个 adapter 声明 `CodeHostCapabilities`，前端只展示已声明并已实现的能力。

| 平台 | MVP 状态 |
| --- | --- |
| GitHub | URL 识别、gh 账号导入/状态、仓库信息、PR 列表与创建、发布仓库（均经匹配的 gh 身份） |
| GitLab | URL 识别、glab 账号导入/状态、账号模型与 capability 占位 |
| Gitee | URL 识别、账号模型、钥匙串 PAT、remote 绑定及按绑定账号 HTTPS Git 认证 |
| Gitea / Forgejo | 账号、PAT、remote 绑定和 Git 网络操作；平台 API capability 暂不展示 |
| 任意 Git 服务 | 可创建“通用 Git 服务” Endpoint；支持显式 HTTPS PAT binding，以及按 URL 自动工作的 SSH 配置、系统 Git credential helper 和私有 Clone；不假设网页或平台 API |

## 领域数据

- `CodeHostKind`：GitHub、GitLab、Gitee、Gitea、Forgejo、通用 Git 服务、未知平台。
- `CodeHostEndpoint`：稳定 ID、平台类型、网页/API 地址、Git host。
- `CodeHostCapabilities`：平台已实现能力的显式开关。
- `AccountProfile`：账号 ID、Endpoint、登录名、显示名、认证方式、钥匙串引用、认证状态和默认候选标记。
- `RemoteProfile`：Git remote 的 fetch/push URL、解析出的 host/path/平台。
- `RemoteBinding`：规范化仓库路径 + remote 名称到 Endpoint 与显式账号或 `Auto` 的映射。
- `RepositoryRecord`：规范化路径与显示名，以及仅供侧栏使用的手动分组、remote host/组织路径、最近显式打开时间。
- `CredentialStrategy`：一次 Git 网络操作实际采用的认证方式；其中只保存账号/钥匙串引用，不保存密钥值。
- `AppError`：带稳定 code 的可读、安全错误；错误文本会避免回传令牌或完整敏感 URL。

## Remote 识别与多账号选择

内建规则自动识别 `github.com`、`gitlab.com`、`gitee.com` 的 HTTPS、`ssh://` 与 SCP 风格 SSH（如 `git@github.com:owner/repo.git`）地址。已保存的自定义 Endpoint 也会按 Git host 自动识别；未配置的 host 不猜测平台，用户可把 remote 明确关联到通用 Git 或某个具体平台 Endpoint。

网络操作的解析顺序如下：

1. 读取远程 URL 并识别或关联 Endpoint。
2. 查找该仓库路径和 remote 名称的 RemoteBinding。
3. 如果 binding 指向具体 AccountProfile，验证它存在、属于该 Endpoint、认证可用且是可固定的 PAT；SSH agent、CLI 与系统 credential helper profile 都不能成为明确绑定。
4. 如果 binding 为 `Auto`，只在唯一可用的 PAT 候选（或唯一有效默认候选）时选择；有多个候选、账号失效、Endpoint 不匹配或无法确认时返回 `account_selection_required`，由界面要求用户选择。没有可固定 PAT 时，HTTPS/SSH remote 回退到系统 Git 按 URL 调用的 credential helper / SSH 配置。
5. 解析出 CredentialStrategy 后才调用 Git。显式 RemoteBinding 永远优先于默认候选账号。

因此“默认账号”只是 Auto 的候选排序信息，不能覆盖某个 remote 的明确绑定，也不能在存在歧义时静默替用户选择账号。

对于不携带账户身份的 `git://` 与本地路径 / `file://` remote，应用不创建 Endpoint 或 CredentialStrategy，而是直接保留系统 Git 的无凭据行为。

Clone 默认同样保留系统 Git 行为，但无论是否选择账号，URL 都会在创建 recovery intent 或启动 Git 前按同一套安全规则验证。用户可为 HTTPS Clone 明确选择 Endpoint 与 PAT；后端会先解析认证策略，在 clone 成功后把该选择保存为新仓库 `origin` 的 remote binding。SSH Clone 保持 SSH 配置的无绑定身份选择。

## 凭据安全约束

- PAT 只可写入系统钥匙串；添加 PAT 的 IPC 是一次性写入，任何查询接口都不会返回它。
- 不把令牌写入日志、持久化元数据、SQLite、repository config、remote URL 或命令行参数。
- Git 认证使用 askpass 时，子进程只能通过父 Git 路径签发的一次性 capability 恢复受限账号配置；密码提示在 capability 原子消费后才可读取钥匙串内容。capability 目录和记录在 Unix 为 owner-only；Windows 会在写入机密前用当前用户 SID 设置受保护的 DACL。应用管理 PAT 的网络 Git 命令还会以命令级 `core.hooksPath` 禁用 hook，避免 repository-controlled hook 继承 capability。token 不作为命令参数或 UI 数据返回。gh/glab 的平台 API 子进程使用有界并发的管道读取：异常后代长期持有 pipe 时，有限 permit 会先抑制重复 worker，再在清理宽限期后释放 slot，避免永久 busy；Unix 以独立 process group 清理，Windows 以带 `KILL_ON_JOB_CLOSE` 的 Job Object 保持树身份，避免子进程退出后的 PID 重用。
- 所有子进程错误均被安全化后再返回；命令本身和环境变量中的密钥不记录。
- 不自动变更用户全局 Git 配置、gh/glab 活跃账号、SSH 配置或 remote URL。

## Tauri command 数据流

页面加载仓库时，前端调用只读 command 获取 status、branches、remotes、绑定和可用能力。暂存、提交、分支、remote、账号和绑定操作分别调用窄粒度 command。fetch/pull/push command 在后端完成 remote→binding→credential strategy 解析；若需要选择账号，返回结构化错误而不是尝试另一个账号。

## 国际化

前端使用应用级 `appLocale` 保存简体中文 / English 选择，并同步 Naive UI 的组件与日期语言。所有页面、弹窗、操作反馈和稳定后端错误码都由前端翻译；后端继续只返回不含密钥的稳定错误 code 与安全的 fallback 文案。

## MVP 边界

本期不包含 Issue 管理、冲突可视化编辑、完整的 SSH 密钥创建器、OAuth 登录流程或 Gitea/Forgejo API。GitHub 已支持仓库信息、PR 列表、创建 PR 和发布仓库，但这些平台操作依赖当前可用且与 remote 明确匹配的 `gh` 身份；GitLab/Gitee 平台 API 入口仍由 capabilities 明确关闭，避免出现无效按钮。

Git LFS 当前提供状态读取、在当前仓库初始化和添加跟踪规则；不包含大文件传输进度、锁定协作或完整的 LFS 管理工作流。任意 Git host 的多账号 HTTPS、SSH、系统 credential helper 和 remote binding 路径是本期完整实现重点。
