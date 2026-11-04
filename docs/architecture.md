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

详情页顶部品牌与制作资料共用已应用手动覆盖的 `Game.metadata.brand`。旧配置中的 `summary_brand` 仅为兼容保留，不再用于显示或在关联资料时生成，避免切换资料源后仍显示旧品牌。故事阅读模式保留行宽限制，编辑模式的输入区随资料卡片宽度伸缩。

## Tauri 桌面端

`main.rs` 只调用 `desktop::run`；`desktop.rs` 负责启动、协议、托盘与命令注册。扫描、资料、封面、账号、下载、存档、补丁及启动工具分别由对应模块实现。

`state.rs` 负责共享配置、游戏目录的持久化和跨功能写入互斥。扫描、账号刷新、下载、补丁预览、存档恢复和运行进程的内部状态由各自模块封装；跨模块通过操作接口读取下载结果、使下载计划失效等，不能直接操作对方的锁或临时计划。

存档、补丁与启动流程仍共用 `state::begin_write`，避免功能拆分后失去互斥。新增 Tauri 命令应放在所属业务模块，并在 `desktop.rs` 注册，保持现有命令名和序列化字段兼容。

游戏、存档和补丁目录统一通过 `folders` 打开；各命令保留所属业务的路径校验。Windows 端将目录解析为 Shell 项目再打开，避免 `explorer.exe` 命令行解析路径后跳转到默认目录。

`core::downloads` 封装文件流接收、进度、取消、体积检查及不覆盖已有文件的落盘；`fan_account` 和 `data_sources::moyu` 分别负责各自的授权与文件获取规则。桌面 `downloads` 共用任务互斥和进度事件，UI 的 NextMoe 下载请求不触发无关的 2DFan 账号刷新。NextMoe 的下载 ID 与展示用网页 URL 分开传输，签名下载 URL 不进入持久化配置或前端。

`core::fan_http` 为 2DFan 资料查询与账号会话提供共用的 Token/Cookie、超时、响应大小及同源重定向规则。桌面数据源入口从既有 `GalGear/2DFan/…` 凭据条目读取会话，`data_sources::Credentials.fan` 只在内存中传递且不参与序列化；搜索、资料、标签、封面候选和补丁列表使用该会话，没有凭据时匿名访问。CDN 图片和文件下载不携带站点凭据。

## OAuth HTTPS 回调

`core::data_sources::oauth` 负责内置应用授权、PKCE、本机监听和令牌轮换；桌面命令负责打开系统浏览器、取消与结果保存。`cloudflare/oauth-callback` 是独立部署的无状态 Worker，绑定 `galgear.cbshuimu.net`：NextMoe 仅中继授权码，Bangumi 则保管发布版 App Secret，签发短期会话及加密换码票据，并调用上游换码/刷新。用户令牌只在桌面凭据库持久化，不保存在 Worker。协议与旧令牌兼容见 [OAuth 回调服务](oauth-callback.md)，取舍见 [ADR 0002](adr/0002-builtin-oauth-applications.md)。

## 验证

```powershell
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p galgear-ui --target wasm32-unknown-unknown --locked -- -D warnings
```

## 游戏截图

`core::screenshots` 维护快捷键校验、游戏内 `screenshots` 路径边界、PNG 原子写入和最近截图读取；桌面端 `screenshots` 的独立线程管理前台游戏识别、Win32 热键、客户区采集和剪贴板。UI 通过 `configure_tools` 统一保存卡片中的工具、保留份数和截图快捷键，通过 `latest_screenshot`、`open_screenshot_folder` 读取预览和打开目录；`screenshot-updated` 只携带游戏 ID，预览响应用游戏 ID 和请求代数隔离迟到结果。

设置位于“路径与备份 → 存档备份与内置工具”，新配置默认启用 `Shift+S`，快捷键与保留份数并排，通过卡片右上角保存。只在库中游戏位于前台时注册，离开游戏后释放；快捷键冲突或设置写入失败保留旧配置。游戏识别使用前台进程的真实可执行路径，支持游戏目录内的子启动器。GalGear 退出后快捷键失效。

截图以 UTC 时间命名，PNG 保留客户区实际物理像素，不做缩放或有损压缩；详情卡片的显示预览可以缩小。保存成功后才通知刷新，剪贴板同时提供原始 PNG 和 bottom-up 24 位 CF_DIB（包含 DWORD 行对齐，Windows 可合成 CF_BITMAP / CF_DIBV5），复制失败独立提示并保留已保存图片。截图目录拒绝符号链接和重解析点，读取预览不创建目录；“打开目录”会按需创建。

当前采集为前台客户区的屏幕像素复制，排除标准标题栏和边框；游戏需完整位于屏幕内。Magpie 通过公开的 `Magpie.SrcHWND` 属性（旧版回退到 owner）与游戏关联；全屏截取整个缩放输出客户区，窗口模式优先截取 `Magpie_Renderer` 子窗口。保留缩放后的实际物理像素，不对原游戏截图二次插值。采集前后均校验前台及输出关联。参见 [Magpie ScalingWindow](https://github.com/Blinue/Magpie/blob/v0.13.0/src/Magpie.Core/ScalingWindow.cpp) 和 [Windows 剪贴板格式](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats)。独占全屏、受保护画面、自绘标题栏和第三方覆盖层可能影响结果，建议窗口或无边框模式。不会回退为整张桌面截图。工作线程的短提示不抢游戏焦点，使用应用明暗/配色与游戏所在屏幕的 DPI。热键同时接收 WM_HOTKEY 与后台 Raw Input，补充仅依赖系统热键消息的触发方式；只在已识别的前台游戏中注册原始键盘接收，失焦恢复原注册，不记录按键内容，长按及两种通知去重。

原生隔离验证：`cargo test -p galgear --locked native_client_pixels_clipboard_and_hotkey_conflict -- --ignored --test-threads=1`（需要交互式桌面，会创建测试窗口并将测试图像写入剪贴板）。设置 `GALGEAR_TEST_MAGPIE=1` 时，测试等待手动对测试窗口启动 Magpie 全屏缩放，并校验截图尺寸等于显示器物理分辨率及中心像素。普通测试不启动真实游戏、不操作真实存档。

管理员游戏可能让普通权限进程收不到键盘消息。`screenshots::windows::elevated` 在识别到这种前台窗口后，以系统授权启动同一可执行文件的 `--galgear-screenshot-input` 辅助模式；每次 GalGear 会话只启动一次，取消后可保存快捷键重试。辅助进程验证父进程可执行文件和窗口归属，只接收快捷键并回传目标 HWND 与配置代数，不接收文件路径、不执行截图或写入。主进程再次核对前台游戏后完成原尺寸 PNG、剪贴板和主题提示。失焦或修改设置撤销旧代数，退出销毁消息窗口，辅助进程随之结束；UAC 等待不阻塞设置和消息处理。键盘接收仍遵循 [Windows Raw Input](https://learn.microsoft.com/en-us/windows/win32/inputdev/using-raw-input)，不记录按键文本，也不向游戏注入代码。

## 更新检查与 Skill 手动安装

`core::ai::skills::status` 只读本地安装与 commit；`latest_commit` 只查询远端 main 的 SHA，与 `sync` 共用查询及校验逻辑，不写目录、不下载资源包；`sync` 负责固定仓库 main 的检查、按 commit 下载和原子替换，并返回是否更新。桌面 `skill_package` 不再有启动任务或 OnceCell；只有 `check_skill_update` 命令能同步，`get_ai_runtime` 仅查询本地状态，`check_skill_version` 在后台线程只读查询远端版本，无需独占资源租约。AI 任务持有资源读取租约，更新持有排他租约，防止替换仍被使用的目录。

UI `skill_update` 在设置 → 更新中复用 GalGear 卡片布局，显示 Skill、本地 commit、固定仓库和“更新”按钮。每次启动完成初始化后，UI 并发后台检查 GalGear 和 Skill 版本，不阻塞本地功能、不自动安装、不弹出确认；失败信息显示在更新卡片中。检测到新版本时，在版本标记左侧显示同样式的“待更新”，设置左侧“更新”入口显示圆形数量提示，GalGear 和 Skill 各计一项，最多为 2。关闭 GalGear 更新弹窗保留通知，Skill 安装成功后清除对应通知；迟到的后台检查不能恢复旧通知。更新不阻塞本地游戏库，重复点击合并为一次请求；成功后刷新版本和 AI 可用状态，迟到的启动查询不能覆盖更新结果。失败保留本地版本，可重试。
