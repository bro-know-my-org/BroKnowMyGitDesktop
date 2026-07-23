# BroKnowMyGitDesktop

轻量的多账号 Git 与代码托管桌面客户端，基于 Tauri 2、Vue 3、TypeScript、Naive UI 和 Rust 构建。内建 GitHub、GitLab、Gitee 优化，同时可为任意 Git host 添加“通用 Git 服务” Endpoint。

Git 操作使用系统 `git`，保留 SSH、LFS、正常系统 Git hook 与既有 credential helper 行为。任意 HTTPS Git host 可显式绑定系统钥匙串 PAT；SSH 身份始终由 Git/SSH 配置选择，不能假称固定到某个账号资料。系统 Git credential helper 也始终按 remote URL 自动选择，不伪装成某个已选账号。为防止 repository hook 窃取 askpass capability，使用应用管理的 PAT 的网络操作会禁用 hook。`git://` 与本地 `file://` remote 沿用无凭据的系统 Git 行为。GitHub、GitLab、Gitee 通过独立的 CodeHostAdapter 模型接入；PAT 仅保存在系统钥匙串中，不写入仓库配置或应用元数据。

界面提供简体中文和 English 切换，并会记住本机选择。

## 当前状态（2026-07-23）

核心的本地 Git 工作流已可用并已做过真机回归：查看改动与 diff、按文件或按代码块暂存、取消暂存、创建提交、分支/remote 管理，以及 fetch / pull / push。分支页会展示已获取的远程跟踪分支；点击可切换到已有同名本地分支，或创建对应的本地跟踪分支。工作区有未提交改动时，会明确提供“暂存后切换 / 直接切换 / 取消”选择。新建但尚未首次提交的仓库也支持暂存和取消暂存。本地仓库侧栏支持手动分组、按远端组织/用户和按最近使用切换；分组只保存在应用元数据中，不会改写 Git 配置。

改动页遵循“先审查、再选择、后提交”的交互：文件勾选框表示是否纳入下一次提交，部分暂存使用三态勾选；文件列表独立滚动，提交表单固定可见，右侧始终保留当前文件的 diff。支持单文件 Blame、Reflog 恢复记录，以及预览后清理未跟踪（非忽略）文件。发生冲突时，可配置外部编辑器打开所有冲突文件，完成后回到应用刷新、暂存并继续或中止。危险的丢弃、Clean 与 force push 都需要确认。

当前验证结果：`pnpm build` 通过、Rust 测试 105/105 通过、`cargo clippy -- -D warnings` 无告警，且已完成 Windows `x86_64-pc-windows-msvc` 的凭据 / 进程树 API 编译检查；完整 Tauri Windows 包仍须在带 Windows SDK（`llvm-rc`）的 Windows CI 或主机上构建。新增真实仓库测试覆盖 Blame、Reflog、Clean 和外部编辑器冲突路径；已在本项目的无首个提交仓库中验证单文件与批量“暂存 → 取消暂存”，测试未创建提交且暂存区已清空。

功能边界、维护入口、回归步骤和后续待办见 [docs/development-status.md](docs/development-status.md)。

## 开发

```bash
pnpm install
pnpm tauri dev
```

```bash
pnpm build
cd src-tauri && cargo test
```

详细模块边界、多账号选择和凭据安全规则见 [docs/architecture.md](docs/architecture.md)。
