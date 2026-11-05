# BroKnowMyGitDesktop

面向多账号、多代码托管平台的 Git 桌面客户端。把改动审查、暂存、提交、分支管理和远程同步放在同一个工作区中，并沿用本机的 Git 工具链。

基于 Tauri 2、Vue 3、TypeScript、Naive UI 和 Rust 构建，支持简体中文与 English、系统深浅色主题，以及界面和代码字体设置。

## 下载与安装

前往 [GitHub Releases](https://github.com/bro-know-my-org/BroKnowMyGitDesktop/releases) 下载正式安装包：

| 平台 | 架构 | 安装包 | 说明 |
| --- | --- | --- | --- |
| Windows | x64、ARM64 | NSIS 安装程序 | 当前未进行 Authenticode 代码签名，可能出现 SmartScreen 提示 |
| Linux | x64、ARM64 | DEB、RPM、AppImage | 文件选择依赖宿主系统的 XDG Desktop Portal |
| macOS | Intel x64、Apple Silicon ARM64 | DMG | 仅使用 ad-hoc 签名，尚未进行 Apple 开发者签名与 notarization，首次打开可能被 Gatekeeper 阻止 |

安装包文件名以 Rust 目标三元组开头：`x86_64` 对应 x64，`aarch64` 对应 ARM64；`pc-windows-msvc`、`unknown-linux-gnu`、`apple-darwin` 分别对应 Windows、Linux、macOS。请选择匹配系统和处理器的安装包。

运行前需要安装系统 `git`。GitHub 平台功能还需要已登录的 GitHub CLI（`gh`）；普通 Git 操作不要求安装它。

Linux 需安装 `xdg-desktop-portal` 和匹配桌面的后端，例如 KDE Plasma 的 `xdg-desktop-portal-kde` 或 GNOME 的 `xdg-desktop-portal-gnome`。AppImage 同样依赖这些宿主服务。

## 可以做什么

| 工作流 | 当前能力 |
| --- | --- |
| 管理仓库 | 打开、扫描、初始化和克隆仓库；按手动分组、远端组织 / 用户或最近使用浏览 |
| 审查与提交 | 带行号和语法高亮的 Diff；按文件或代码块暂存 / 取消暂存；提交与 amend |
| 历史与恢复 | 提交历史、Stash、Blame、Reflog，以及带预览和确认的未跟踪文件清理 |
| 分支与同步 | 分支、remote、tag、merge、rebase、cherry-pick、revert、reset，以及 Fetch / Pull / Push |
| 仓库工具 | Worktree、子模块和 Git LFS 状态及部分管理操作；调用外部编辑器处理冲突 |
| GitHub 协作 | 查看仓库信息和 PR、创建 PR、Fork、发布仓库，以及从贡献分支发起 PR 的流程 |

改动页以“选择要提交的内容”为主线：勾选文件决定下一次提交的范围，三态表示部分暂存；文件列表独立滚动，提交表单和当前 Diff 保持可见。丢弃改动、Clean、hard reset 和强制推送等破坏性操作需要确认。

当前没有内置的可视化冲突编辑器、交互式 rebase 或完整的 LFS 管理工作流。完整能力边界与后续工作见 [开发状态与交接说明](docs/development-status.md)。

## 多平台与多账号

Git 操作调用系统 `git`，可连接 GitHub、GitLab、Gitee、自建 Gitea / Forgejo，以及其他 Git 服务。Git 网络访问与平台 API 的支持范围如下：

| 平台 | Git 与认证 | 平台 API |
| --- | --- | --- |
| GitHub | SSH、系统 credential helper、PAT；HTTPS 可绑定 `gh` 账号 | 仓库信息、PR 列表与创建、Fork、发布仓库，依赖匹配的 `gh` 身份 |
| GitLab | SSH、系统 credential helper、PAT；支持导入 / 检测 `glab` 账号 | 暂未开放 |
| Gitee、Gitea、Forgejo | SSH、系统 credential helper、PAT | 暂未开放 |
| 其他 Git 服务 | 添加“通用 Git 服务” Endpoint，使用 SSH、系统 credential helper 或 HTTPS PAT | 不假设平台 API 可用 |

账号选择遵循以下规则：

- HTTPS remote 可以明确绑定某个 PAT 或 GitHub CLI 账号。自动选择存在歧义时，会要求选择账号。
- SSH 身份由本机 SSH 配置与 agent 决定；系统 credential helper 按 remote URL 选择凭据。
- 应用不会自动切换 `gh` / `glab` 的活跃账号，也不会修改全局 Git 或 SSH 配置。
- PAT 仅保存在系统钥匙串，不写入仓库配置、remote URL 或应用元数据。
- 普通 Git 操作沿用系统 Git 的 hook 行为；使用应用管理的 PAT 或绑定 `gh` 账号的网络操作会禁用 hook，保护认证信息。

账号绑定、凭据选择与安全边界详见 [架构说明](docs/architecture.md)。

## 本地开发

### 准备环境

- Node.js `>=20.19.0` 与 pnpm。
- Rust 工具链和系统 `git`。
- 当前平台的 [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)。
- 按需安装 `gh`、`glab` 或 Git LFS。

在仓库根目录执行：

```bash
pnpm install
pnpm tauri dev
```

### 检查与构建

`pnpm check` 包含 CSS 检查、发布逻辑与虚拟列表测试、源码单元测试、国际化检查、TypeScript 类型检查和前端构建：

```bash
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo check --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

构建当前平台的桌面安装包：

```bash
pnpm tauri build
```

本地开发的关于窗口显示 `dev`；手工构建安装包的版本元数据为 `0.0.0`，正式版本由发布流程注入。

<details>
<summary>Linux 运行注意事项</summary>

文件选择由桌面门户后端提供。如果门户仍使用存在缺陷的 GTK 3 文件选择器，需要在系统侧修复其对缺失文件大小属性的处理。

应用默认使用 GIO 的 `base` 网络监控，避免 WARP 等 VPN 批量切换路由时造成 netlink 缓冲区溢出。该模式假定网络可用，实际连接结果仍由请求决定；WebKit 在线状态事件和 GIO 计费网络检测不再准确。网络中断后可能需要重试，此设置不保证连接自动恢复。

应用会保留显式设置的 `GIO_USE_NETWORK_MONITOR`。例如，开发时恢复 netlink 监控：

```bash
GIO_USE_NETWORK_MONITOR=netlink pnpm tauri dev
```

该环境变量可能被子进程继承，但不会修改系统网络配置。

</details>

## 发布

仓库中的 `package.json`、`src-tauri/Cargo.toml` 和 `src-tauri/tauri.conf.json` 保持占位版本 `0.0.0`，正式版本以 `vX.Y.Z` tag 为唯一来源。

tag 必须指向 GitHub 仓库默认分支第一父提交链（`first-parent` 主线）上的 commit，例如合并完成后的主线 HEAD。先确保该提交已推送至默认分支；下面命令假设本地 HEAD 正是该主线提交：

```bash
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions 会在构建前注入真实版本，在 Windows、Linux、macOS 的 x64 与 ARM64 原生运行器上分别测试并打包。六种组合全部成功且 10 个安装包齐全后，生成 `SHA256SUMS.txt` 并公开 GitHub Release。构建产物和文件名均按目标三元组区分，避免不同平台或架构相互覆盖。发布说明包含上一个稳定版本到当前 tag 的提交记录和完整对比链接。

发布流程使用 GitHub 托管运行器，其中 Windows ARM64 和 Linux ARM64 运行器需仓库具备相应的访问权限（本仓库为公开仓库）。Windows 包暂未签名，macOS 包采用无需 Apple 凭据的 ad-hoc 签名；Apple 开发者签名与 notarization 尚未配置。

发布仓库应为 `v*.*.*` 配置禁止更新和删除的 tag Ruleset，防止已发布版本的 tag 被移动。

## 项目文档

- [开发状态与交接说明](docs/development-status.md)：当前可交付范围、交互约定、验证基线和后续优先级。
- [架构说明](docs/architecture.md)：GitCore、账号模型、平台适配器与凭据安全边界。
- [前端目录与职责](docs/frontend-architecture.md)：组件分层、状态编排、样式规则和性能诊断。
