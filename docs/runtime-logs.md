# 运行日志

桌面程序在加载配置、创建窗口之前，在 **GalGear.exe 所在目录的 `logs`** 下创建日志，例如 `galgear_2026-11-06_143025_123.log`。文件名使用启动时的本地时间（毫秒）；同一时间启动多个实例时增加数字后缀，已有文件不会被覆盖。工作目录、便携模式和用户配置目录不影响日志位置。日志保留到用户自行删除，不自动上传。

文件为 UTF-8 JSON Lines，每行一个记录。普通记录包含带时区的时间、启动后毫秒数、日志级别、进程/线程、模块和消息。日志直接写入文件，错误额外调用刷盘，减少崩溃前尾部信息丢失。

## 排查顺序

1. 按发生时间选择对应的日志，先查看 `session_start` 的版本、系统架构、程序位置和运行环境，以及 `desktop_setup` 的 WebView2 版本。
2. 搜索 `command_error`、`javascript_error`、`wasm_panic`、`galgear::panic`、`native_crash` 和后台警告。命令错误保留原有国际化错误码与脱敏后的错误详情，Rust panic 记录位置和强制捕获的堆栈。
3. 用 `request` 关联 `command_start` 与 `command_ok`/`command_error`，查看操作顺序与耗时；游戏操作同时记录合法的游戏哈希 ID。起点在后端接收命令时写入，结果由前端桥接报告，页面或进程提前退出时可能只留下起点，不能单凭缺少结束记录断定命令失败。
4. 结合 `state_loaded`、游戏进程 PID、运行状态变更，以及工具、自动备份、截图、资料更新和账号刷新事件判断上下文。原生后台事件直接记录，不依赖窗口仍然可用。
5. 正常退出具有 `shutdown_start` 和 `session_end`；缺少这些标记说明进程未完整执行退出流程，但可能是断电、强制结束或更新交接，不能单凭标记判定崩溃原因。

## 黑屏和界面无响应

- `webview_diagnostics_ready` 记录 Windows 主窗口的 WebView2 浏览器进程 PID，可用来关联 Windows 事件日志。`webview_process_failed` 由原生侧记录，包括浏览器、渲染或 GPU 进程异常的类型，以及运行时提供的原因码和退出码。监听注册失败时记录 `webview_diagnostics_unavailable`。
- `view_applied` 表示 DOM 已应用。切换页面或弹窗类别时记录一次，例如资料确认关闭会从 `page=Home overlay=Setup` 变为 `page=Home overlay=None`。用同一页面加载内的 `view` 编号关联 `view_frames` 和 `view_frames_delayed`。页面可见且超过 2 秒仍未收到第二次帧回调时才记录延迟，回调随后到达时会记录实际耗时。新视图和页面隐藏会取消之前的帧检查，`frontend_visibility` 提供可见性上下文。
- `view_frames` 只说明浏览器执行了帧回调。即使回调正常，窗口合成或显示仍可能异常。如果 JavaScript 或 IPC 完全停止，前端也可能无法写出延迟记录。排查时结合原生进程事件和录像时间。

Windows 未处理异常回调使用预先打开的独立追加句柄，不使用普通日志锁或堆分配，尽力记录本地时间、异常码、故障地址、主模块基址及线程。它不阻止系统正常处理异常。Rust panic 堆栈在发行版中的符号信息取决于构建符号；需要精确映射原生地址时，应保留对应版本的可执行文件与构建符号。

## 覆盖与限制

- 前端诊断脚本由 Tauri 在页面脚本前注入，捕获未处理异常、Promise 拒绝、资源加载失败和 WASM panic；桥接报告命令结果时不改变原有成功值或错误对象。
- 不记录命令参数、成功响应正文、完整配置、按键、AI 对话、请求头或请求体。界面切换只记录预定义的页面与弹窗类别，不记录页面内容、截图或标题。普通日志只接受项目自身的 INFO/WARN/ERROR，不启用第三方 HTTP 调试日志。URL、常见凭据字段/请求头、Windows 用户目录用户名和邮件地址在写入前脱敏，单条普通消息限长。游戏 ID 用于关联，不记录游戏名称。
- 诊断错误文本仍可能包含游戏/文件路径，分享前请检查业务相关的个人信息。文本脱敏不是任意外部文本中所有个人信息的识别器。
- 安装目录无写权限或磁盘不可用时会明确提示，不偷偷改为其他目录；业务可以继续运行。运行中写入失败只弹出一次提示。
- 系统断电、强制终止、内存耗尽、严重栈损坏、Windows fast-fail、WebView2 子进程原生崩溃可能来不及记录故障本身；本功能不生成内存转储，也不保证仅靠日志就能还原所有故障。更新辅助进程和管理员输入/Magpie 守护进程仍沿用各自退出机制，本日志记录桌面主进程会话。

## 维护与验证

`src-tauri/src/diagnostics.rs` 负责日志文件、panic、命令与后台事件；`diagnostics/privacy.rs` 负责脱敏，`diagnostics/windows.rs` 负责原生异常记录，`diagnostics/webview.rs` 使用 Tauri 依赖树已有的 WebView2 绑定订阅进程故障，`diagnostics/frontend.js` 负责帧检查和前端诊断传输。

```powershell
cargo test -p galgear --locked diagnostics::tests
node --test scripts/test-diagnostics.cjs
cargo test -p galgear --locked native_webview_records_frontend_view_and_renderer_failure -- --ignored
```

崩溃测试派生独立测试进程，并写入临时目录，分别触发 Rust panic 和 Windows 测试异常。WebView2 验证需要 Windows 桌面和运行时，使用临时配置和隐藏窗口，在收到真实前端诊断后触发该测试窗口的渲染进程退出。后续新增后台错误事件时，应在原生诊断订阅中登记；新增前端直调命令应经过 `GalGearDiagnostics.invoke`。
