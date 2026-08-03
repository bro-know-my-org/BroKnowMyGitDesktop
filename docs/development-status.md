# 开发状态与交接说明

最后更新：2026-08-03

这份文档记录当前可交付范围、核心交互约定、已完成验证和下一步工作。它面向继续开发者；不要仅凭 UI 文案推断 Git 行为，应同时检查对应的 Rust command 和 `GitCore`。

## 当前可用范围

### 本地 Git 工作流

- 仓库发现、初始化、Clone、仓库快照与状态刷新；Clone 与网络 Git 操作均在后台 worker 执行，不能占用 Tauri IPC 执行器。
- 本地仓库侧栏可在手动分组、远端组织 / 用户、最近使用三种视图间切换。手动分组可逐仓库编辑并持久化；远端归属优先使用 `origin`，最近使用只在用户实际打开仓库时更新。
- 工作区状态、带行号和语法高亮的文件 Diff、按文件暂存/取消暂存、全部加入/移出提交、按 hunk 暂存/取消暂存、丢弃改动（有确认）。Diff 的增删背景与每行高度绑定，不能因字体缩放或语法 token 造成相邻行颜色重叠。
- 创建提交与 amend；无首个提交（unborn）仓库可正确取消暂存，不依赖 `HEAD`。
- 历史与 Stash 操作、分支（包括已获取的远程跟踪分支；可创建本地跟踪分支并切换）、merge / rebase / cherry-pick / revert / reset、tag、worktree、子模块和 Git LFS 状态入口。切换分支前如有本地改动，会提供暂存后切换、直接切换或取消。
- 单文件 Blame、Reflog 恢复记录，以及预览后强确认的未跟踪文件清理（不会删除已忽略文件）。
- 冲突文件可交给应用本地配置的外部编辑器处理；用户返回后刷新、暂存并继续或中止原 Git 操作。
- remote 管理，以及按 ahead/behind 状态收敛为 Fetch / Pull / Push 的一个主按钮；force-with-lease 保留在次级确认入口。使用已验证 URL 固定凭据目标时，Pull 仍必须读取当前分支的实际上游和仓库 local/worktree scope 的 fetch refspec，保留自定义远程跟踪命名空间、force 标记与 negative exclusion，不得回退到 URL 的默认分支或继承 system/global 的同名 refspec。

### 外观与平台集成

- 界面支持跟随系统、浅色和深色主题，自定义主题色会持久化并同步 Tauri 原生窗口主题。
- 默认读取当前平台的系统界面字体、等宽字体、字号与字重：Linux 兼容 GNOME/GSettings、KDE 和 fontconfig，Windows 使用原生 UI 字体并提供系统等宽回退，macOS 使用 Core Text 系统字体。
- 设置页只列出本机已安装字体，可分别覆盖界面/代码字体、字号和字重，并控制编程连字。Diff 行高从代码字号派生；恢复系统字体会重新使用平台值；关闭连字时必须同时关闭 `font-feature-settings` 和 `font-variant-ligatures`。
- 所有字体设置通过共享 CSS token 作用于 Naive UI、Diff、SHA、分支名等代码区域；禁止组件另写固定字体族绕过设置。

### 认证与代码托管

- GitHub、GitLab、Gitee、Gitea、Forgejo 和通用 Git Endpoint 的账号/remote binding 模型。
- 显式绑定的系统钥匙串 PAT，以及按 remote URL 自动工作的 SSH 配置和系统 Git credential helper；SSH agent 账号资料只用于状态显示，不能把实际 SSH identity 固定到 Profile。PAT 不会写入前端状态、Git 配置、remote URL 或应用元数据。
- GitHub 通过当前匹配的 `gh` 身份支持仓库信息、PR 列表、创建 PR 和发布仓库；发布前会自动读取个人账号与组织所有者，并标记当前身份能否在组织中创建仓库。所有者读取失败时发布按钮保持禁用，最终创建权限仍由 GitHub 的组织策略、仓库可见性和 SSO 授权判定。发布会通过 `gh auth token --user` 将所选账号固定到本次内存态操作，查询、创建和初次推送不受并发 `gh auth switch` 影响；令牌不进入参数、日志、IPC 或持久化，并通过禁用本次推送的 hooks、fsmonitor 和不受信任 Git 网络配置，避免仓库进程读取或重定向认证信息。这些 `gh` 网络命令在后台 worker 执行，有 45 秒超时，并以有限 pipe-operation permit 抑制异常后代长期持有输出管道时的重复读取线程。进程树终止后仍未关闭的 pipe 会在清理宽限期后释放 permit，不能永久把后续 GitHub 操作锁成 busy；Unix 用独立 process group，Windows 用 Job Object 保持进程树身份，避免直接子进程退出后以 PID 误杀复用进程。PR 和组织列表分页均共享各自命令的 45 秒总时限；超过上限会明确报错，绝不静默截断。
- “账号与认证”路由先读取持久化的认证状态缓存，再在后台刷新 CLI、钥匙串和 SSH agent 状态；后台检测不持有应用状态锁，60 秒内避免重复触发，页面提供手动刷新入口。网络 Git 操作仍会在执行前即时校验账号，缓存不参与授权放行。

完整的信任边界和账号选择规则见 [architecture.md](architecture.md)。

## 改动页的交互约定

这是后续 UI 改动最需要保持的部分。设计参考 GitHub Desktop 的工作流思想，而非其代码或视觉实现。

1. 默认落在 **Changes**；主导航顺序是 Changes、History、Branches & remotes、Details。
2. “是否纳入下一次提交”是文件列表的主动作。勾选代表完整纳入，未勾选代表不纳入，三态代表文件有一部分已经暂存。
3. 点击处于三态的文件/总勾选会先完成“全部纳入”；再次取消勾选才会全部移出。这与普通三态 checkbox 的预期一致。
4. 文件列表只做选择与审查入口。提交表单固定在左栏底部，文件列表独立滚动；右侧始终显示当前选择的 diff。高密度文件列表、历史列表和 diff 必须在各自面板内滚动，不能因内容过多撑破窗口。
5. History 使用两行提交列表：第一行 SHA + 标题，第二行作者 + 时间；标题和元数据必须截断，不能回退为在窄栏中挤四列表格。
6. `Discard`、Hard reset、force push 等破坏性操作不应在行内主路径里堆按钮，必须经确认流程。
7. 提交按钮必须说明结果（文件数和分支）；未选择文件时应明确提示“选择要提交的文件”，不能显示机械的 `0 file(s)`。
8. 外观首次跟随系统深浅色；顶栏外观菜单支持跟随系统、浅色和深色，并保存用户选择，同时同步 Tauri 原生窗口主题。设置页支持持久化自定义主题色，默认 `#4B0082`，颜色选择器的 Alpha 通道在保留最低可见度的前提下控制选中背景、边框和焦点光晕强度；主操作色会按深浅模式和实际合成背景自动生成满足对比度要求的变体。新增自定义颜色时优先使用 `src/styles/app.css` 的 `--app-*` 令牌，避免只在浅色模式可读。
9. Diff 按行做 Git 语义着色并显示新旧行号：文件头、hunk、元信息、增加行和删除行均需可区分，代码 token 由 `highlight.js` 生成。渲染必须保持受控转义，不能把仓库文本作为任意 HTML 注入；行背景、行号栏和内容使用同一行高，字体缩放后也不能上下重叠。
10. Diff 加载态和内容必须互斥渲染。不要让已取得的补丁继续嵌套在会降低内容透明度的 loading 遮罩中；请求进行时显示 spinner，完成后直接显示文本或明确的空状态，避免出现“Git 已返回补丁但面板看起来空白”。
11. 仓库分组是应用本地元数据：不改 Git 配置、不触发网络。扫描和普通列表刷新只能登记 / 读取仓库，不能更新“最近使用”；只有 `get_repository_snapshot` 的显式打开路径可以更新最近时间。切到组织/用户视图时才会批量读取本地 remote 配置以补全归属，`origin` 的 `host + namespace` 优先，缺失时才回退到其他可识别 remote。
12. 桌面端 Modal 直接显示和关闭，不使用缩放、位移、淡入淡出或遮罩过渡。WebView 在弹窗首次挂载、布局和文本绘制期间容易超过单帧预算；新增或升级 Naive UI 弹窗时必须确认 `WorkspaceDialogs.css` 的统一覆盖仍然生效。
13. 按钮进入 loading 状态时不能改变自身宽度或推动相邻操作。Spinner 应覆盖在按钮中央，原标签保留布局占位；弹窗内部已有加载反馈时，不要在打开弹窗的入口按钮上重复显示 Spinner。按钮 loading 覆盖必须绑定 `.n-button--loading` 等稳定状态类，不能用 `:has()` 追踪 Transition 动态增删的内部 Spinner 节点；WebKitGTK 可能不会及时使这类关系选择器失效，导致加载结束后文字仍长期隐藏。
14. 仓库 watcher 与只读 Git 命令不能互相触发。后台 `git status` 必须设置 `GIT_OPTIONAL_LOCKS=0`，避免 Git 创建临时 `index.lock` 后被 watcher 识别为外部修改，形成“刷新 → 锁文件事件 → 再刷新”的循环。新增自动刷新来源时应通过性能报告确认空闲窗口不会持续调用 `refresh_repository_snapshot`。
15. 可能启动 Git 子进程或扫描工作区的只读 Tauri command 也必须通过 `spawn_blocking` 执行，不能占用 Tauri IPC 执行器。默认“改动”页只加载当前 Diff；工作树、子模块和 LFS 仅在首次进入“分支与远程”时并行读取，不能把非首屏面板的冷启动成本加入仓库打开关键路径。
16. 仓库 watcher 必须按 Git ignore 规则生成工作树监听计划，逐个目录使用非递归监听；不能把仓库根目录直接交给 `RecursiveMode::Recursive`，否则 `node_modules`、`target` 等被 Git 忽略的依赖和构建树仍会被底层 watcher 全量注册。Git 元数据目录单独递归监听；已跟踪但后来被 ignore 的路径必须补回监听计划。新目录、`.gitignore`、`.git/info/exclude` 或相关 Git 配置变化会使计划失效，协调刷新后必须自动重建。安装和销毁都通过 blocking worker 执行，安装完成事件继续触发一次协调刷新，以覆盖计划生成窗口内发生的外部变化。
17. Git 子进程等待不能对所有命令固定休眠 25ms 后才检查退出。仓库 snapshot 会串行执行多条极短 Git 查询，固定轮询间隔会把每条查询的人为等待累加到启动路径。统一执行器从 1ms 开始指数退避并封顶 25ms：短命令及时完成，长命令在约 31ms 后恢复原有低频轮询。调整轮询策略时必须同时覆盖普通输出和有上限的预览读取路径，且不能削弱超时、输出上限或进程树清理。
18. 字体设置必须全平台退化安全。系统探测失败或保存的字体已卸载时回退到 CSS 通用字体栈；界面和代码字号、字重分别存储；连字只影响显示，不改变 Diff、SHA 或分支名文本。新增代码文本区域必须使用 `--app-monospace-font-family`、代码字号/字重和连字 token。

相关前端入口：

- `src/pages/repositories/components/ChangesPanel.vue`：Changes 页面与文件选择交互。
- `src/features/repository/components/RepositoryDiffViewer.vue`：Diff 行模型、行号与语法高亮展示。
- `src/features/workspace/composables/useRepositoryChanges.ts`：暂存、取消暂存和 Diff 刷新编排。
- `src/shared/theme/index.ts`、`src/shared/fonts/index.ts`：系统外观、字体探测结果和持久化偏好。
- `src-tauri/src/git_core/changes.rs` 与 `src-tauri/src/git_core/changes/`：状态、Diff、暂存、丢弃和 Clean。
- `src-tauri/src/git_core/network.rs`：Fetch / Pull / Push 的 URL 固定、上游和 refspec 处理。
- `src-tauri/src/commands/system.rs`：Linux、Windows、macOS 系统字体探测与已安装字体枚举。

### 关键状态约束

- Git porcelain v2 的 `.` 表示该侧无改动，必须规范化为空字符串；不可把它当成已暂存状态。
- 未跟踪文件只有 worktree 状态 `?`，不应显示为已选择提交。
- 普通 `git diff` 不会输出未跟踪文件。单文件未跟踪文本预览要在后端用 `git diff --no-index -- /dev/null <path>` 生成新增补丁，并把 exit code `1` 视为预期的“文件不同”；执行此前必须用工作树相对路径校验，不能让 `--no-index` 读取仓库外文件。
- 刷新 snapshot、暂存、取消暂存、应用 hunk 或提交后，需要用最新 snapshot 中同一路径的条目刷新 diff；不要拿旧 `GitStatusEntry` 重新请求 diff。
- 无 `HEAD` 时，`git restore --staged` 不可用。后端对这类仓库使用 `git rm --cached --ignore-unmatch` 仅移出 index，保留工作区文件。
- 提交历史的机器格式使用 `git log --format=format:...`，不可换成 `tformat:`；所有七个字段均以 NUL 分隔，不能用提交标题可包含的控制字符作为记录分隔符。注意 `format:` 仍会在相邻记录间插入一个 LF，`parse_commits` 必须只从第二条及之后的 SHA 前移除该 LF。
- 所有 UI 预览 diff 都必须带 `--no-ext-diff`，不能因为仓库或用户的 `diff.external` 配置而执行外部程序。新增/修改 remote URL 与所有 Clone 路径都必须先验证 URL；仅允许 HTTPS、HTTP、SSH、`git+ssh`、`git` 和 `file` 这几类显式协议，且拒绝凭据、查询或可注入参数。账号认证只允许 HTTPS/SSH，不能经明文 HTTP 发送凭据。Windows drive / UNC 路径只在 Windows 识别为本地 remote，其他平台不能把 `c:group/repo.git` 错判为本地路径。
- 对已解析并验证的 URL 执行 Fetch / Pull 时，必须先捕获 named remote 的有效配置，再把明确 refspec 传给 URL 形式的 Git 命令。读取 fetch mapping 时仅信任 `local` 与 `worktree` scope；Pull 使用 Git 解析出的 `%(upstream:remoteref)` 和 `%(upstream)`，并验证 source→tracking destination 的实际映射。negative refspec 命中当前上游时应拒绝拉取，不能静默绕过。
- Git 子进程启动前会移除继承来的 askpass、SSH helper 和 `GIT_CONFIG_*` 认证注入变量，再按当前已解析的账号显式设置安全环境。每次 askpass 都必须由父 Git 路径签发短时、随机、一次性的 capability；Unix capability 目录与记录为 owner-only，Windows 会在写入机密前将 owner 和受保护 DACL 收敛到当前用户 SID。helper 只能从该记录恢复配置，且 capability 内的 account profile ID 必须与钥匙串引用相同。用户名提示只校验、密码提示在读取钥匙串 PAT 前原子消费并再次核对账号引用。密码提示可带 Git 已回填的用户名，但它必须精确等于所选账号。PAT askpass 必须从 Git 提示中解析出与 Endpoint 相同的规范化 HTTPS authority（保留任何显式端口，包括 `:443`/`:22`，并支持 IPv6 方括号），不能只比较 hostname。携带 PAT capability 的网络 Git 命令必须以命令级 `core.hooksPath` 禁用 hook；不得让 repository-controlled hook 继承并调用 askpass。`gh/glab` 的活跃账号和 SSH agent 都无法安全固定到一次 Git 身份选择，因此它们不能保存为 Git remote 的 Profile binding；CLI 账号只用于平台 API，SSH 继续由 Git/SSH 配置选择。系统 credential helper 也不能伪装成指定账号，它只在没有明确 PAT 候选时按 URL 回退。SSH agent 状态探测必须在短超时后终止子进程，不能让账号刷新或网络命令无限等待。gh/glab 的 stdout 与 stderr必须并发、有上限地排空；异常 pipe 读取先受有限 permit 限制，若进程树终止后仍未关闭则在清理宽限期后释放 slot，不能永久耗尽 CLI 能力；清理通过 Unix process group 或 Windows Job Object 处理后代，不能根据已退出直接子进程的 PID 误杀复用进程。
- 一个 remote binding 同时授权 fetch 和 push；保存、修改 remote URL、clone 恢复、pending publish 恢复和浏览器演示都必须验证两条有效 URL 属于同一个 Endpoint，否则清理 / 拒绝该 binding。`file://host/...` 仍是本地 remote，绝不能因其语法 authority 关联 Endpoint。对同一仓库，所有工作树 / index / ref 写入、凭据化 fetch / pull / push / 子模块更新 / 发布、remote 或 binding 修改，以及解析 remote 后调用 GitHub 平台 API 都必须共用 operation lease，不能在旧 URL、临时移除的 binding 或并发工作区变更之间执行操作；删除 linked worktree 时必须原子占用发起端和目标端的路径锁。新建 remote 前必须先清理同名但无 Git remote 的遗留 binding，不能因 remote 名复用让旧账号选择重新生效。重命名 remote 时先持久化同一授权下的新旧双 binding，再执行 Git 重命名并清理旧名；这样在崩溃边界两种 remote 名都不会无意回退到其他账号。pending clone 除双 URL 外还必须验证原 Clone URL 的规范化 authority（保留显式端口）和预期 `owner/repository`；同一目标目录的 Clone 必须持有 destination lease，避免并发调用覆盖或清理彼此的 recovery intent。pending publish 除双 URL 外还必须验证发起发布时的 `HEAD` 和预期 GitHub `owner/repository`；`gh repo create --push` 即使最终返回失败也可能已经建立 origin，因此必须保留 intent 并在下次调用优先 reconciliation。GitHub 平台能力只能用于 `api_url` 精确匹配 `gh --hostname` 标准路由的 Endpoint；带端口或不匹配的 Endpoint 仍可使用普通 Git 凭据，但不开放 gh 导入、发布、仓库信息或 PR API。缺失身份、丢失 Endpoint 或账号的 intent 必须清理，不能在路径复用后恢复账号绑定或永久阻塞下一次发布。
- `AppError` 的 code、message 和 details 均只接受编译期固定的安全文本，字段对模块外不可变；系统、CLI、HTTP 和解析错误的原始文本不得跨 IPC。损坏的 `profiles.json` 会先备份为同目录的 `profiles.invalid.<uuid>.json`，应用以空配置继续启动；若 schema 版本高于当前版本，必须保留原文件并阻止启动，不能自动恢复为空配置。Unix 上新建的 profile store 必须为 owner-only (`0600`)；写入会同步文件和父目录，但 macOS 对目录 `fsync` 返回的 `EINVAL` 是平台限制，不能把已完成的原子替换误报为失败。账号移除先持久化元数据，再删除钥匙串 PAT，删除失败会记录待重试的无密钥引用，并清理关联的 pending publish / clone 恢复记录。若元数据已跨过原子替换边界但最终目录同步告警，仍必须在本次执行钥匙串删除，再把持久化告警返回给调用方。
- 顶层 `NLayout` 必须占据固定 WebView 视口，只有 `.content-area` 的内部滚动容器可以滚动；不要在固定 Header 下的 `NLayoutContent` 重新设置 `height: 100%`，否则会多出一段等于 Header 高度的顶层滚动。
- 大量改动文件时，`.changes-workspace`、`.changes-list-pane`、`.changes-file-card` 和卡片内容必须把可用高度逐层约束下来：桌面端文件卡占据左栏的可伸缩行，只有 `.changes-file-list` 自身滚动；窄窗口改为单列且将文件卡限制在视口高度的一部分。不要让文件列表以内容高度撑开卡片、挤掉提交区或溢出 WebView。
- `RepositoryRecord` 的 `group`、`host`、`namespace` 与 `lastOpenedAt` 是向后兼容的持久化字段。注册仓库时必须保留这些字段；存储上限为 200，避免手动分组的仓库被旧的“最近 40 个”策略静默淘汰。
- `list_accounts` 只能返回缓存的 `AccountProfile.status`，不能在路由加载时调用 CLI、SSH agent 或钥匙串。耗时检测必须经 `refresh_account_statuses` 的后台 worker，并在结果回来后按账号 ID 合并；刷新期间若账号被删除，不能将其重新写入。

- 外部冲突编辑器只使用应用元数据中保存的可执行文件和独立参数；冲突文件会先验证仍位于仓库内，再作为独立进程参数传入。不得通过 Git 或仓库本地 mergetool 配置拼接或执行 shell 命令。编辑器启动后不等待其退出，用户完成处理后必须刷新状态、暂存文件，再继续操作。
- 未跟踪文件清理始终先使用 Git dry-run 预览；执行阶段只使用正常 clean 语义删除未跟踪且非忽略的文件/目录，必须有明确的不可撤销确认。

### 桌面调试入口

- 开发服务器固定使用高位端口：Vite 为 `51420`，远程主机场景下的 HMR 为 `51421`。不要恢复默认的 `1420`；修改端口时必须同时更新 `vite.config.ts`、`src-tauri/tauri.conf.json` 的 `devUrl`，以及 CSP 中对应的 HTTP / WebSocket 地址。生产 `csp` 只放行本地开发地址；`devCsp` 单独以 `ws:` 放行 `TAURI_DEV_HOST` 的 HMR，不能把这个开发期宽松规则复制到生产 CSP。
- `pnpm tauri dev` 的 Tauri Debug 窗口支持 `F12` 和 `Ctrl+Shift+I` 开关 Web Inspector；发布构建不启用开发者工具。
- `src-tauri/tauri.conf.json` 显式声明 `devtools: true`，但 Tauri 仍只会在 Debug 构建（或显式开启 Rust `devtools` feature）中提供该能力。
- Vite 的 HMR 只能更新前端。改动 `src-tauri/`（例如 `GitCore` 的 diff 逻辑）后，必须让 Tauri Debug 进程重新编译并重启；刷新页面、继续使用旧的 `target/debug` 窗口都不会替换 Rust 后端。排查 diff 时先确认文件状态也来自当前版本：未跟踪文件应显示“未跟踪”，而不是旧版的泛化“已修改”。

## 已完成验证

2026-08-03 的自动化验证基线：

- `pnpm check`：通过（Stylelint、i18n 静态错误码覆盖、Vue TypeScript 检查与 Vite 生产构建）；当前仅有 Vite 对主 chunk 超过 500 kB 的体积警告。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`：206/206 通过。
- `cargo check --manifest-path src-tauri/Cargo.toml`：通过。
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：通过。
- `git diff --check`：通过。
- Windows `x86_64-pc-windows-msvc`：生产的凭据与进程树模块已在 `-D warnings` 下完成 API 编译检查，覆盖 Job Object 与 capability ACL 代码。本 Linux 主机不能直接完成 Tauri 应用级交叉检查，因为 Tauri 的 Windows 资源步骤需要 `llvm-rc`；完整 Windows 包必须在具备 Windows SDK 的 Windows CI / 主机上验证。
- 自动化覆盖还包括：固定 URL Pull 的自定义 tracking namespace 与推送状态、local/worktree fetch mapping、system/global scope 隔离、negative exclusion、Linux 的 KDE/Pango 字体偏好解析，以及 Diff、历史、凭据、进程树、持久化和 operation lease 的安全约束。Windows 与 macOS 字体探测仍需各平台构建和真机回归。
- 本项目运行中的 Vite 开发服务监听 `127.0.0.1:51420`，HTTP 健康检查返回 `200`；配置中没有残留 `1420`。
- 使用隔离应用数据启动新编译的 Tauri Debug 窗口，打开本项目的无首个提交仓库并选中 `package.json`：右侧实际渲染了完整的新增文件 diff（24 行内容、文件头和 hunk），不是仅验证后端字符串。
- 在 `BroKnowMySparkAnalyzer` 的真实已暂存 `package.json` 改动上验证：`git diff --cached`、Tauri command 和桌面窗口三者均得到同一补丁；窗口可见文件头、hunk 和红绿删除行。修复前端 loading 遮罩后，补丁不再被透明化为看似空白的面板。
- 在本项目含 56 个未跟踪文件的状态下，验证 Changes 左栏只显示可视文件并在面板内滚动，提交卡仍保持可达；以 `1024×680` 最小窗口尺寸验证窄布局时文件卡不会撑出窗口。
- 在本项目这个无首个提交仓库中，通过 Tauri 窗口验证：
  - 未跟踪文件 → 单文件暂存 → 单文件取消暂存；
  - 部分选择状态 → 全部加入提交 → 全部移出提交；
  - 暂存后右侧显示 staged diff，取消暂存后状态和提交按钮同步更新。
- 新增 GitCore 真实仓库覆盖：Blame、Reflog、未跟踪文件 clean 的 dry-run 与执行，以及真实 merge 冲突下由配置的无 Shell 外部编辑器接收冲突文件路径。

测试没有创建提交。结束测试前应运行 `git diff --cached --name-only`，确保输出为空；不要用 `reset --hard` 清理测试状态。

## 推荐回归流程

```bash
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo check --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
git diff --check
```

Windows 打包或应用级交叉检查还需要 Windows SDK 提供的 `llvm-rc`；不要把 Linux 主机缺少该工具误判为 Rust 业务代码编译失败。涉及 Windows-only FFI 时，至少应在 Windows CI 运行完整 Tauri 构建。

需要实际窗口回归时，使用隔离的应用数据目录，避免读取或修改日常使用的账号与仓库列表：

```bash
XDG_DATA_HOME=/tmp/bkm-e2e-data pnpm tauri dev
```

手工检查至少覆盖以下路径：

1. 已有 `HEAD` 的仓库：修改文件、暂存、取消暂存、提交前确认 diff 来源正确。
2. 无 `HEAD` 的新仓库：添加文件、暂存、取消暂存，确认文件仍在工作区。
3. 有部分暂存的文件：确认三态勾选先补全暂存，再可取消全部暂存。
4. 有 remote 的仓库：主按钮分别在落后、领先、同步时呈现 Pull、Push、Fetch；在“分支与远程”中获取远程分支后，确认 `remote/branch` 可创建对应本地跟踪分支，并在有本地改动时正确显示暂存 / 直接切换 / 取消；force push 仍需二次确认。
5. 破坏性操作：取消确认后不得改变 index 或工作区。
6. 仓库侧栏：分别切换手动分组、远端组织 / 用户、最近使用；编辑分组、留空取消分组，重启后确认手动分组仍在。确认列表刷新不改变最近使用顺序；首次打开有 remote 的仓库后，组织归属与最近使用均正确更新。
7. 冲突工作流：配置外部编辑器（例如 `code` 与单独一行的 `--wait`），制造 merge / rebase / cherry-pick 冲突；确认应用仅打开冲突文件，回到应用刷新、暂存后可继续或中止。
8. Clean：创建未跟踪文件和目录，先检查预览和确认文案，再确认只删除未跟踪、非忽略文件；取消确认时文件必须保留。
9. 外观设置：分别在 Linux、Windows、macOS 检查“跟随系统”字体；覆盖界面/代码字体、字号、字重和连字后重启，确认设置保持，关闭连字后 `==`、`=>`、`!=` 不再合字，并确认 Diff 行高随代码字号变化且增删行背景不重叠。
10. 自定义 fetch mapping：在 linked worktree 或启用 `extensions.worktreeConfig` 的仓库中配置非默认 tracking namespace 和 negative refspec，确认 Pull 更新正确 tracking ref，ahead/behind 与 pushed 状态同步，且不会读取 global/system 同名 mapping。

## 后续优先级

### P0：自动化和回归可靠性

- 为 Tauri UI 建立自动化回归（至少覆盖无 `HEAD` 的暂存/取消暂存、部分暂存和 diff 刷新）；当前这部分仍以手工真机验证为主。
- 为 Git 状态解析补充更多带空格、重命名、冲突、子模块和二进制文件的 fixture。
- 把文件级操作、批量操作和 hunk 操作的快照/diff 同步规则提取为可测试的前端单元逻辑，避免未来 UI 调整重新引入旧 diff。

### P1：工作流完整度

- 改善二进制、图片和大 diff 的预览与性能策略；当前没有文本 diff 时只显示明确占位。
- 为提交过程增加 hook 输出/失败定位的可视化回归。
- 在保持安全边界的前提下，逐步增加 Git 配置管理、交互式 rebase、sparse-checkout、bisect、签名提交/标签和补丁邮件流等专业工作流。
- 扩展子模块与 LFS 的完整生命周期，而不把网络认证或仓库本地 hook/配置的执行权交给前端。

### P2：平台能力

- 按 `CodeHostCapabilities` 逐项扩展 GitLab、Gitee、Gitea、Forgejo 的平台 API；未实现前不要显示可点击入口。
- PR/发布流程补充端到端测试，并明确 `gh` 登录、组织权限和网络失败时的恢复提示。

## 修改前检查清单

- 是否仍然只经 Tauri command 调用 Git，而没有把 shell 字符串拼到前端？
- 是否保留 `--` 作为路径参数分界，且没有把 token 放入日志、URL、命令参数或前端状态？
- 是否同时覆盖有 `HEAD` 与无 `HEAD` 的仓库？
- 状态变更后，已选文件的 diff 是否来自最新 snapshot？
- 是否给破坏性操作保留确认、可读后果和安全默认值？
- 是否更新本文件中的验证结果、能力边界或待办？
