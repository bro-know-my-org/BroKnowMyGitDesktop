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
- `features/workspace/composables/` 管理页面级状态、表格列和业务操作；展示无关的格式化逻辑放在 `features/repository/utils/`。
- `shared/api/tauri.ts` 是前端访问 Rust command 的唯一入口，禁止组件直接调用 Tauri IPC。

## 样式规则

- 设计变量和跨组件布局规则放在 `styles/app.css`。
- 新增局部视觉规则优先放在所属组件的 `<style scoped>`；只有多个同级页面共享时才提升到页面或全局样式。
- 不为单行或一次性样式单独创建 CSS 文件。
