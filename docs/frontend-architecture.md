# 前端目录与职责

前端按“应用壳、页面、功能域、共享能力”分层。页面只组装界面，状态与命令编排留在 composable，平台调用集中在共享 API 层。

```text
src/
├─ app/                         # 应用壳与页面级装配
├─ pages/
│  ├─ accounts/                 # 账号页
│  └─ repositories/             # 仓库页及各 Tab 面板
├─ features/
│  ├─ repository/               # 仓库展示工具
│  └─ workspace/                # 工作区状态、表格与交互组件
├─ shared/
│  ├─ api/                      # Tauri IPC 适配
│  ├─ i18n/
│  ├─ theme/
│  └─ types/
└─ styles/                      # 全局设计 token 与跨组件布局样式
```

## 组件边界

- `src/App.vue` 只创建工作区控制器并提供上下文。
- `app/AppShell.vue` 只拼装侧栏、顶部栏、内容区和弹窗层。
- `pages/*/index.vue` 是页面编排层；仓库页的改动、历史、分支、详情各自位于 `components/`。
- `features/workspace/composables/useWorkspaceController.ts` 只负责装配公开工作区接口、生命周期和跨域刷新。
- 工作区业务按职责拆到 `useRepositoryCatalog`、`useRepositoryChanges`、`useRepositoryHistory`、`useRepositoryOperations`、`useRepositoryAuxiliary`、`useRepositoryPlatform` 和 `useWorkspaceAccounts`；模块通过窄依赖回调协作，不建立全局单例。
- `shared/api/tauri.ts` 保持稳定的 IPC facade；运行时请求、内建 Endpoint、浏览器 Demo fixture 与 diff 解析分别位于同目录的小模块中。
- 展示无关的格式化逻辑放在 `features/repository/utils/`。
- `shared/api/tauri.ts` 是前端访问 Rust command 的唯一入口，禁止组件直接调用 Tauri IPC。

## 样式规则

- 设计变量和跨组件布局规则放在 `styles/app.css`。
- 新增局部视觉规则优先放在所属组件的 `<style scoped>`；只有多个同级页面共享时才提升到页面或全局样式。
- 不为单行或一次性样式单独创建 CSS 文件。

## 启动性能诊断

- 开发环境默认记录启动阶段和统一 Tauri API 请求耗时，不记录请求参数、仓库路径、返回值或认证信息。
- 启动完成后在 WebView 开发者工具控制台查看折叠的 `[BKM performance]` 表格；执行 `window.__BKM_PERFORMANCE__.report()` 可以再次输出当前汇总，`window.__BKM_PERFORMANCE__.snapshot()` 可取得便于自动分析的结构化数据。
- Release 构建默认关闭。需要测试 Release 性能时使用 `VITE_BKM_PERF=true pnpm tauri build`；也可以在开发者工具中执行 `localStorage.setItem("bkm:performance", "true")` 后重载，执行 `localStorage.removeItem("bkm:performance")` 可关闭。
- `Bootstrap to main module execution` 包含入口模块图加载、解析和求值时间；`Vue root mount` 反映同步挂载成本；`Bootstrap to interactive frame` 覆盖前端资源启动、工作区任务及其后的两帧绘制。控制台先按发生顺序展示每次命令调用，再按总耗时汇总调用次数、平均值、最大值和错误数。
- 报告的 `context` 会记录 WebView 的可见和焦点状态。WebKit 会限制后台或未激活窗口的动画帧频率；此时双帧指标可能平白增加约 500ms，比较报告前必须先确认 `visibilityState === "visible"` 且 `focused === true`。
- 性能打标用于定位热点，不能替代 WebKit Inspector 的 Timeline。并行命令的总耗时会重叠，判断启动关键路径时应结合各阶段墙钟时间，不要直接相加命令总耗时。
