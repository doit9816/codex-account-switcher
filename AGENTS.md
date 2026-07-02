# Codex Switch 编码规范

本规范适用于 `codex-account-switcher` 项目。

当前项目有意保持非模块化结构：大部分前端行为集中在 `src/App.tsx`，样式集中在
`src/styles.css`，Tauri 后端主要集中在 `src-tauri/src/main.rs`，OAuth 相关后端逻辑位于
`src-tauri/src/oauth.rs`。除非当前需求明确需要拆分，否则不要引入大范围模块化重构。

## 项目结构

- 前端技术栈：React 18、TypeScript、Vite、Tauri v2 API、`lucide-react` 图标。
- 后端技术栈：Rust、Tauri v2 commands、serde JSON/TOML 处理、本地账号数据加密、
  Codex 配置迁移、OAuth 登录支持。
- 用户可见的应用名称、概念和文案应始终围绕 Codex 账号/profile 切换。
- 除非功能明确只面向某个系统，否则应保持 Windows、macOS、Linux 跨平台可用。

## 通用规则

- 优先做小而聚焦的改动，并匹配当前文件组织方式。
- 保持用户数据语义不变：没有明确备份路径或现有回滚机制时，不要删除或覆盖 Codex 文件。
- 账号 token、API key、refresh token、加密 profile 数据、迁移包都视为敏感数据。
  不要记录到日志、显示在通知里，也不要在测试 fixture 中使用真实值。
- TypeScript 与 Rust 之间的 wire payload 统一使用 `camelCase`。
- Rust 结构体字段保持惯用 `snake_case`，需要序列化给前端或持久化 JSON 时使用
  `#[serde(rename_all = "camelCase")]`。
- 避免修改持久化文件格式；如果必须修改，需要提供向后兼容默认值或迁移路径。
- 简单逻辑不要新增依赖，优先使用当前技术栈清晰实现。

## 非模块化文件策略

因为项目当前不是模块化结构，相关改动优先保留在现有文件中：

- `src/App.tsx`：React 类型、i18n 文案、状态、command 调用、渲染辅助函数和 UI。
- `src/styles.css`：所有应用样式和响应式行为。
- `src-tauri/src/main.rs`：Tauri commands、profile 存储、Codex 文件操作、迁移、
  额度探测、token 刷新和后端测试。
- `src-tauri/src/oauth.rs`：OAuth session、callback、PKCE、token exchange 等专属逻辑。

只有在以下情况才新增模块或文件：

- 新代码是类似 `oauth.rs` 的明确独立子系统。
- 继续塞进当前文件会明显降低可读性和可维护性。
- 测试或领域逻辑需要靠近新的子系统。

如果新增文件，应保持对外接口小，并通过现有入口串接，避免把管线逻辑扩散到很多文件。

## 前端规范

- 使用函数式 React 组件和 hooks；除非状态已经在 `App` 中共享，否则保持局部状态。
- 新增格式化日期、用量、额度标签、通知、账号状态等逻辑前，先复用现有 helper。
- 新增 Tauri command 调用时，通过 `invoke` 调用，并显式声明 TypeScript 返回类型。
- TypeScript 类型必须与 Rust command 返回结构保持同步。
- 按钮和工具操作优先使用 `lucide-react` 中已有的合适图标。
- 当按钮没有可见文字或含义不够明确时，应提供有用的 `title` 或 `aria-label`。
- 控件保持紧凑、偏工具型。这是桌面工具，不是营销页面。
- 避免嵌套卡片、大型装饰性 hero 布局。
- 响应式布局应延续现有 media query、grid、flex 模式。
- 不使用随 viewport 缩放的字体尺寸；使用和当前 CSS 一致的明确字号。
- 按钮、标签、卡片、弹窗中的文字不能溢出。必要时使用 `min-width: 0`、
  `overflow-wrap`、`text-overflow` 等方式处理。

## I18n 与文案

- 所有用户可见字符串都应进入现有 `messages` 字典。
- 当前 `messages` 中已有的每种语言都要补齐对应 key。
- 文案 key 保持语义化和稳定，不要用字面文本作为 key。
- 除了兼容已有数据的映射逻辑外，不要在业务 helper 中混入特定语言文案。
- 如果修复乱码，应尽量保持简体中文、繁体中文和英文的语义一致。

## 样式规范

- 延续现有浅色、实用型桌面风格：中性色面、克制边框、紧凑间距、清晰操作状态。
- 圆角保持适中。现有 panel 和控件通常为 6-10px，除非匹配现有组件，否则不要引入过大的圆角卡片。
- 优先复用现有 class 和变体，例如 `icon-button`、`mini-button`、`panel`、
  `status-pill`、`probe-box`、`limit-chip`。
- 新增 CSS 放在相关 selector 附近。不要为了局部改动重排整个样式文件。
- 只有在能消除多处有意义重复值时，才新增 CSS 变量。
- 避免某个功能突然拥有一套和整体应用不一致的一次性视觉主题。

## Rust 后端规范

- 优先使用 `serde_json`、`toml_edit` 和类型化结构做解析与序列化，避免临时字符串拼接。
- Codex config/auth 文件写入应尽量使用原子写入或可回滚写入。
- 替换用户控制的 Codex 文件前，必须创建或保留备份。
- 迁移包内路径必须通过安全相对路径逻辑校验，禁止绝对路径或 `..` 逃逸目标 Codex home。
- 加密和 key 处理集中复用现有 helper。
- 优先复用现有 helper，例如 `load_store`、`save_store`、`push_event`、
  `resolve_codex_home`、`backup_auth_file`、`replace_file_with_rollback`
  以及 bundle 校验相关 helper。
- 编辑 Rust 时，优先使用 `format!("{value}")` 这种 inline capture 风格。
- 当可读性更好时，折叠嵌套 `if`。
- 优先使用穷尽式 `match` 或明确错误处理，避免宽泛兜底逻辑。
- 新增内部 helper 时，避免让调用点出现含义不清的布尔参数，例如 `foo(true)`；
  必要时使用小 enum 或命名更清晰的 helper。
- 不要为了只调用一次的简单逻辑新增 helper；除非它隔离的是复杂逻辑或安全敏感逻辑。

## Tauri Command 规范

- command 名称保持稳定；前端调用依赖这些名称。
- 结构化数据应返回类型化 struct，不要返回松散 JSON。
- 使用 `Result<T, String>`，错误信息应方便用户采取行动。
- 不要通过 view struct 暴露加密 secret、原始 token、API key 或完整 auth JSON。
- 后端返回结构变更影响前端模型时，必须在同一次改动中更新 Rust response struct 和对应 TypeScript 类型。
- 只为有意义的异步状态变化 emit event，payload 保持小而明确。

## 测试

- Rust 后端逻辑优先在现有 `#[cfg(test)]` 模块中新增或更新测试；除非引入了新的子系统文件。
- 优先测试纯 helper：路径校验、序列化、迁移语义、额度解析、token 刷新、OAuth callback/token exchange。
- 测试中只使用假 secret 和假 JWT，绝不包含真实 token 或账号数据。
- 纯前端改动至少运行 TypeScript build 检查。只有相关区域已经引入测试框架时才补前端测试。
- 不要为静态值写测试，除非它保护的是外部协议兼容性。

## 验证

代码改动后，运行最小但有效的检查：

- 前端和类型检查：`npm run build`
- Tauri/Rust 单元测试：在 `src-tauri` 目录运行 `cargo test`
- Rust 格式化：在 `src-tauri` 目录运行 `cargo fmt`

如果改动涉及 UI，还应启动应用，并在桌面宽度和窄屏响应式宽度下目测受影响流程。

## Review Checklist

完成改动前确认：

- 敏感数据没有被记录、意外展示或意外导出。
- 已有存储的 profiles 和 settings 仍可读取。
- Codex `auth.json` 和 `config.toml` 写入有备份或回滚保护。
- TypeScript 与 Rust payload 结构仍保持一致。
- 新增用户可见文案已经补齐所有语言字典。
- 按钮、弹窗、卡片、标签在窄宽度下不会溢出。
- 改动范围聚焦在当前需求，没有顺手开启无关模块化重构。
