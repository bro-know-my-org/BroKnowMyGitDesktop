# BroKnowMyGitDesktop

轻量的多账号 Git 与代码托管桌面客户端，基于 Tauri 2、Vue 3、TypeScript、Naive UI 和 Rust 构建。内建 GitHub、GitLab、Gitee 优化，同时可为任意 Git host 添加“通用 Git 服务” Endpoint。支持简体中文与 English，并在 Linux、Windows、macOS 上使用各平台的系统能力。

Git 操作使用系统 `git`，保留 SSH、LFS、正常系统 Git hook 与既有 credential helper 行为。任意 HTTPS Git host 可显式绑定系统钥匙串 PAT；SSH 身份始终由 Git/SSH 配置选择，不能假称固定到某个账号资料。系统 Git credential helper 也始终按 remote URL 自动选择，不伪装成某个已选账号。为防止 repository hook 窃取 askpass capability，使用应用管理的 PAT 的网络操作会禁用 hook。`git://` 与本地 `file://` remote 沿用无凭据的系统 Git 行为。GitHub、GitLab、Gitee 通过独立的 CodeHostAdapter 模型接入；PAT 仅保存在系统钥匙串中，不写入仓库配置或应用元数据。

界面默认跟随系统深浅色与系统字体。设置中可分别选择已安装的界面字体和等宽代码字体，调整字号、字重和编程连字，也可恢复平台系统值；Diff 行高会随代码字号同步计算，并可选择是否自动换行显示过长的行。

## 当前状态

核心的本地 Git 工作流已可用：查看改动与带行号、语法高亮的 Diff，按文件或代码块暂存/取消暂存，创建提交，浏览 History 与 Stash，以及管理分支、remote、tag、worktree、子模块和 Git LFS。Fetch / Pull / Push 会在后端解析 remote、上游和凭据；固定凭据 URL 的 Pull 仍遵循仓库 local/worktree scope 的 fetch refspec、自定义远程跟踪命名空间和排除规则。

改动页遵循“先审查、再选择、后提交”的交互：文件勾选框表示是否纳入下一次提交，部分暂存使用三态勾选；文件列表独立滚动，提交表单固定可见，右侧始终保留当前文件的 Diff。支持 Blame、Reflog、未跟踪文件 Clean 预览和外部冲突编辑器；丢弃、Clean 与 force push 等破坏性操作均需要确认。仓库侧栏支持手动分组、按远端组织/用户和按最近使用切换，元数据不会改写 Git 配置。

功能边界、当前验证基线、回归步骤和后续待办见 [开发状态与交接说明](docs/development-status.md)；模块和安全边界见 [架构说明](docs/architecture.md)，前端目录职责见 [前端架构](docs/frontend-architecture.md)。

## 开发

```bash
pnpm install
pnpm tauri dev
```

```bash
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo check --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

## 发布

仓库中的 `package.json`、`src-tauri/Cargo.toml` 和
`src-tauri/tauri.conf.json` 固定使用占位版本 `0.0.0`。正式版本以 `vX.Y.Z` tag
为唯一来源。tag 指向的 commit 必须已经进入 GitHub 仓库的默认分支，GitHub
Actions 才会在各平台构建前临时写入真实版本：

```bash
git tag v0.2.0
git push origin HEAD v0.2.0
```

本地开发的关于窗口显示为 `dev`；未经过发布 workflow 手工生成的安装包元数据仍为
`0.0.0`。GitHub Actions 会构建 Windows NSIS 安装包以及 Linux DEB、RPM、
AppImage，生成 `SHA256SUMS.txt`，然后公开 GitHub Release。Release 页面会列出
上一个稳定版本 tag 到当前 tag 之间的 commit，并提供完整版本对比链接。Windows
安装包当前未进行 Authenticode 代码签名，安装时可能出现 SmartScreen 提示。

当前不发布 macOS 安装包；源码仍保留 macOS 平台实现，但没有 Apple 签名和
notarization 前不作为正式二进制分发平台。GitHub 仓库上线前还应为 `v*.*.*` 配置
禁止更新和删除的 tag Ruleset，避免已发布版本的 tag 被移动。
