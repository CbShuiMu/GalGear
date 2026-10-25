# 源码模块边界

`crates/core` 负责可脱离界面使用的业务逻辑；`crates/i18n` 和 `crates/tags` 提供两端共用的国际化与标签规则。UI 不依赖桌面运行时，桌面命令调用 core，再通过既有 JSON 协议与 UI 交互。

## Rust/WASM UI

- `lib.rs` 声明模块并导出 WASM 入口；`bridge.rs` 是 JavaScript 传输与 DOM 边界。
- `models.rs` 放序列化模型；`state.rs` 组合各功能状态，不堆放各功能的临时字段。
- `events.rs` 接收浏览器事件，`actions.rs` 处理导航与跨功能协调，账号、下载、补丁各自处理所属动作。
- `requests.rs` 独占请求编号与待完成请求。界面是否阻塞直接从队列计算；新增请求种类时在 `Pending::blocks_ui` 中声明策略。
- `responses.rs` 处理后端响应；`view.rs` 绘制应用外壳，`widgets.rs` 提供无状态的 HTML 基础函数。
- `library/` 分开维护状态、查询、动作、页面和弹窗。筛选与层级查询只接收需要的目录、配置或功能状态，不依赖整个 `App`。

生产模块显式导入依赖，不从父模块通配导入。可独立计算的辅助函数应接收具体数据；接收整个 `App` 的函数用于交互编排或页面组合。

## Tauri 桌面端

`main.rs` 只调用 `desktop::run`；`desktop.rs` 负责启动、协议、托盘与命令注册。扫描、资料、封面、账号、下载、存档、补丁及启动工具分别由对应模块实现。

`state.rs` 负责共享配置、游戏目录的持久化和跨功能写入互斥。扫描、账号刷新、下载、补丁预览、存档恢复和运行进程的内部状态由各自模块封装；跨模块通过操作接口读取下载结果、使下载计划失效等，不能直接操作对方的锁或临时计划。

存档、补丁与启动流程仍共用 `state::begin_write`，避免功能拆分后失去互斥。新增 Tauri 命令应放在所属业务模块，并在 `desktop.rs` 注册，保持现有命令名和序列化字段兼容。

## 验证

```powershell
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p galgear-ui --target wasm32-unknown-unknown --locked -- -D warnings
```
