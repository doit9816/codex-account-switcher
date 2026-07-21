# Codex Account Switcher Agent Guide

本规范适用于 `codex-account-switcher` 项目。后续改动要优先保持代码按职责分模块组织，不要把不同领域的逻辑继续堆到单个大文件里。

## 核心准则

- 新功能按领域拆分文件：UI 组件、状态逻辑、Tauri command、OAuth、路由、存储、迁移、样式分别放在对应模块中。
- 不要为了赶进度把新增页面、弹窗、业务 helper、接口调用和样式全部塞进 `src/App.tsx`、`src/styles.css` 或 `src-tauri/src/main.rs`。
- 修改既有大文件时，只做必要衔接；如果新增逻辑超过一个小块，优先抽到新模块，再从现有入口引用。
- 保持改动范围聚焦，不做与当前需求无关的大规模重构。
- 用户 token、refresh token、API key、加密 profile、迁移包都视为敏感数据，不能写入日志、通知、测试 fixture 或普通 UI 文案。

## 前端模块

- 页面入口可以保留在 `src/App.tsx`，但新增可复用 UI 应拆到 `src/components/`。
- 账号卡片、账号编辑弹窗、添加账号弹窗、路由面板、设置面板等应逐步拆成独立组件。
- 前端类型优先放到 `src/types/`，不要在多个组件里重复定义 Rust 返回结构。
- Tauri `invoke` 封装优先放到 `src/api/`，组件只调用语义化函数。
- 通用格式化逻辑放到 `src/lib/` 或 `src/utils/`，例如日期、额度、订阅有效期、token 状态。
- 用户可见文案进入统一 i18n 字典；新增 key 要补齐简体中文、繁体中文和英文。
- 纯图标按钮必须提供 `title` 和 `aria-label`，鼠标悬停显示含义。

## 后端模块

- `src-tauri/src/main.rs` 只保留 Tauri 应用启动、command 注册和必要的薄入口。
- OAuth 登录、PKCE、callback、token exchange 放在 `src-tauri/src/oauth.rs` 或继续拆到 `oauth/` 子模块。
- 路由代理、账号选择、健康状态放在 `src-tauri/src/routing.rs` 或 `routing/` 子模块。
- profile 存储、加密、导入导出、迁移、Codex 配置读写应逐步拆到独立模块。
- 新增 Tauri command 时，后端返回结构和前端 TypeScript 类型必须同步更新。
- 持久化结构新增字段必须提供向后兼容默认值，例如 `#[serde(default)]`。
- 不通过 view struct 暴露原始 token、API key、完整 auth JSON 或加密密文。

## 文件组织建议

推荐后续逐步演进为类似结构：

```text
src/
  api/
  components/
  hooks/
  i18n/
  lib/
  types/
  App.tsx
  styles.css

src-tauri/src/
  main.rs
  oauth.rs
  routing.rs
  store.rs
  profiles.rs
  migration.rs
  codex_config.rs
```

不需要一次性重构到位；每次新增功能时，把新增部分放到最合适的模块即可。

## UI 规范

- 这是桌面工具，不是营销页面；界面应紧凑、稳定、面向高频操作。
- 工具按钮优先使用 `lucide-react` 图标。
- 卡片、弹窗、按钮中的文字不能溢出；必要时使用 `min-width: 0`、`text-overflow`、`overflow-wrap`。
- 不要嵌套卡片，不要引入与现有风格割裂的一次性视觉主题。
- 新增样式放在相关 selector 附近；如果样式明显属于某组件，后续可随组件拆出。

## Rust 规范

- 优先使用类型化结构、`serde_json` 和 `toml_edit` 处理数据，不做脆弱的字符串拼接。
- 写入用户 Codex 文件前必须有备份或回滚保护。
- 路径必须校验，禁止导入包写入绝对路径或 `..` 逃逸路径。
- 复用现有 helper，例如 `load_store`、`save_store`、`push_event`、`resolve_codex_home`、`replace_file_with_rollback`。
- 错误信息要方便用户采取行动，但不能泄露 secret。

## 测试与验证

- 前端改动至少运行 `npm run build`。
- Rust 后端改动在 `src-tauri` 目录运行 `cargo test`。
- Rust 格式化运行 `cargo fmt`。
- 提交前运行 `git diff --check`。
- 涉及 UI 时，尽量启动应用检查桌面宽度和窄屏宽度下的关键流程。

## Review Checklist

- 新逻辑是否按领域拆分，避免继续扩大单个大文件。
- TypeScript 类型和 Rust 返回结构是否一致。
- 老 profile/settings 是否仍可读取。
- 新增字段是否有默认值或迁移路径。
- 敏感数据是否没有被日志、通知、导出或 UI 意外暴露。
- 新增文案是否补齐所有语言。
- 按钮、标签、弹窗、卡片在窄宽度下是否不溢出。
