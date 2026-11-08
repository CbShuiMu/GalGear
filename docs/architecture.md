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

## 运行诊断

桌面 `diagnostics` 在主程序启动时创建安装目录下 `logs/galgear_启动时间.log`，统一记录启动与退出、命令关联、后台错误、前端异常和崩溃。日志边界负责脱敏，core 不依赖桌面日志实现。覆盖范围、故障排查和隔离验证见 [运行日志](runtime-logs.md)。

## 验证

```powershell
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p galgear-ui --target wasm32-unknown-unknown --locked -- -D warnings
```

## 游戏截图

`core::screenshots` 维护快捷键校验、游戏内 `screenshots` 路径边界、PNG 原子写入和最近截图读取；桌面端 `screenshots` 的独立线程管理前台游戏识别、Win32 热键、客户区采集和剪贴板；后台键盘接收及管理员输入转发由 `game_input` 统一管理。UI 通过 `configure_tools` 统一保存卡片中的工具、保留份数和截图快捷键，通过 `latest_screenshot`、`open_screenshot_folder` 读取预览和打开目录；`screenshot-updated` 只携带游戏 ID，预览响应用游戏 ID 和请求代数隔离迟到结果。

设置位于“路径与备份 → 存档备份与内置工具”，新配置默认启用 `Shift+S`，快捷键与保留份数并排，通过卡片右上角保存。只在库中游戏位于前台时注册，离开游戏后释放；快捷键冲突或设置写入失败保留旧配置。游戏识别使用前台进程的真实可执行路径，支持游戏目录内的子启动器。GalGear 退出后快捷键失效。

截图以 UTC 时间命名，PNG 保留客户区实际物理像素，不做缩放或有损压缩；详情卡片的显示预览可以缩小。保存成功后才通知刷新，剪贴板同时提供原始 PNG 和 bottom-up 24 位 CF_DIB（包含 DWORD 行对齐，Windows 可合成 CF_BITMAP / CF_DIBV5），复制失败独立提示并保留已保存图片。截图目录拒绝符号链接和重解析点，读取预览不创建目录；“打开目录”会按需创建。

当前采集为前台客户区的屏幕像素复制，排除标准标题栏和边框；游戏需完整位于屏幕内。Magpie 通过公开的 `Magpie.SrcHWND` 属性（旧版回退到 owner）与游戏关联；全屏截取整个缩放输出客户区，窗口模式优先截取 `Magpie_Renderer` 子窗口。保留缩放后的实际物理像素，不对原游戏截图二次插值。采集前后均校验前台及输出关联。参见 [Magpie ScalingWindow](https://github.com/Blinue/Magpie/blob/v0.13.0/src/Magpie.Core/ScalingWindow.cpp) 和 [Windows 剪贴板格式](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats)。独占全屏、受保护画面、自绘标题栏和第三方覆盖层可能影响结果，建议窗口或无边框模式。不会回退为整张桌面截图。工作线程的短提示显示在游戏客户区右下角（Magpie 启用时跟随缩放输出客户区），保留内边距，小窗口内收窄提示；目标隐藏或关闭时不回退到屏幕角落。提示不抢游戏焦点，使用应用明暗/配色与输出窗口的 DPI。热键同时接收 WM_HOTKEY 与后台 Raw Input，补充仅依赖系统热键消息的触发方式；只在已识别的前台游戏中注册原始键盘接收，失焦恢复原注册，不记录按键内容，长按及两种通知去重。

原生隔离验证：`cargo test -p galgear --locked native_client_pixels_clipboard_and_hotkey_conflict -- --ignored --test-threads=1`（需要交互式桌面，会创建测试窗口并将测试图像写入剪贴板）。设置 `GALGEAR_TEST_MAGPIE=1` 时，测试等待手动对测试窗口启动 Magpie 全屏缩放，并校验截图尺寸等于显示器物理分辨率及中心像素。提示定位隔离验证：`cargo test -p galgear --locked native_notice_stays_inside_game_client_without_activation -- --ignored --test-threads=1`，创建不同位置和大小的原生测试窗口，检查客户区内右下角定位、不抢焦点及隐藏目标处理，不写剪贴板。普通测试不启动真实游戏、不操作真实存档。

管理员游戏可能让普通权限进程收不到键盘消息。`game_input::elevated` 在识别到这种前台窗口后，以系统授权启动同一可执行文件的 `--galgear-game-input` 辅助模式；每次 GalGear 会话只启动一次，取消后可保存快捷键重试。辅助进程验证父进程可执行文件和窗口归属，只接收截图与字体面板两组快捷键并回传对应槽位、目标 HWND 与配置代数，不接收文件路径、不执行截图或写入。主进程再次核对前台游戏后完成原尺寸 PNG、剪贴板和主题提示。失焦或修改设置撤销旧代数，退出销毁消息窗口，辅助进程随之结束；UAC 等待不阻塞设置和消息处理。键盘接收仍遵循 [Windows Raw Input](https://learn.microsoft.com/en-us/windows/win32/inputdev/using-raw-input)，不记录按键文本，也不向游戏注入代码。

## 更新检查与 Skill 手动安装

`core::ai::skills::status` 只读本地安装与 commit；`latest_commit` 只查询远端 main 的 SHA，与 `sync` 共用查询及校验逻辑，不写目录、不下载资源包；`sync` 负责固定仓库 main 的检查、按 commit 下载和原子替换，并返回是否更新。桌面 `skill_package` 不再有启动任务或 OnceCell；只有 `check_skill_update` 命令能同步，`get_ai_runtime` 仅查询本地状态，`check_skill_version` 在后台线程只读查询远端版本，无需独占资源租约。AI 任务持有资源读取租约，更新持有排他租约，防止替换仍被使用的目录。

UI `skill_update` 在设置 → 更新中复用 GalGear 卡片布局，显示 Skill、本地 commit、固定仓库和“更新”按钮。每次启动完成初始化后，UI 并发后台检查 GalGear 和 Skill 版本，不阻塞本地功能、不自动安装、不弹出确认；失败信息显示在更新卡片中。检测到新版本时，在版本标记左侧显示同样式的“待更新”，设置左侧“更新”入口显示圆形数量提示，GalGear 和 Skill 各计一项，最多为 2。关闭 GalGear 更新弹窗保留通知，Skill 安装成功后清除对应通知；迟到的后台检查不能恢复旧通知。更新不阻塞本地游戏库，重复点击合并为一次请求；成功后刷新版本和 AI 可用状态，迟到的启动查询不能覆盖更新结果。失败保留本地版本，可重试。

## 快捷键录制

`ui::shortcut_picker` 管理按键录制、重置、取消和确认后的设置草稿；DOM 桥接仅转发物理键代码及修饰键并维护弹窗焦点。三项快捷键通过 `configure_tools` 统一校验并保存，由 `runtime::canonical_hotkey` 规范化后检查重复组合；Magpie 配置仍沿用其位掩码适配入口。字体面板工作线程跟随已保存的 `font_hotkey` 更新系统热键与 Raw Input，旧配置默认 Shift+T。

## 游戏字体面板

`game_window` 和 `game_input` 统一游戏窗口识别与后台快捷键。Rust 原生 `font_tools` 面板负责游戏内调节和本地文字预览，core 维护参数及版本化 IPC。C++ 使用 SimpleFontHook 兼容构建，运行时兼容代码集中在 `native/font-hook/compat`，负责转区字体准备、运行中安装及引擎刷新。上游原 UI 隐藏，更新自动重建兼容组件。协议、更新与实时刷新限制见 [游戏内字体面板](font-tools.md)。

## 别名检索与数据源偏好

扫描结束（包括每次启动的增量扫描）后，后台为 `Game.aliases` 为空的游戏查询 NextMoe 多语言名称和别名。已有 NextMoe 关联按作品 ID 查询，其余只接受名称完全匹配的唯一游戏，避免混入续作。别名写入既有原子目录缓存，后续扫描及资料编辑保留；旧缓存默认空列表。失败或未匹配项保留为空，下次启动重试，同一运行期间不重复请求。游戏库文本搜索仅匹配标题和别名；标签通过独立的标签筛选匹配，不参与文本搜索。别名查询不改变资料来源。

`Settings.source_preferences` 保存资料、标签、封面、补丁入口的手动来源选择；没有覆盖时跟随 `metadata_provider`。旧配置默认空映射，已有逐游戏资料关联仍保留。补丁仅支持鲲Gal/2DFan，默认来源不支持补丁时显示全部。鲲Gal仅为显示名称，API 标识与凭据仍使用 `nextmoe`。

## 汉化启动器与游戏运行会话

`core::runtime::Monitor` 属于应用实例，由桌面 `tools::RuntimeState` 持有并显式共享给计时、启动检查、存档备份/恢复、补丁写入和封面处理。启动后立即从 `Child` 句柄记录 PID 与创建时间，启动器即使瞬间退出也不会丢失交接线索。`runtime/processes.rs` 采集一次进程表和窗口表；`runtime/monitor.rs` 在锁外完成系统采集，锁内仅更新会话，较旧的并发快照不能覆盖新状态。桌面观察周期为 1 秒，耗时采集运行在后台任务中。

识别后继进程综合父子关系、创建时间、可见窗口、前台窗口及启动目录，不要求与配置的 EXE 同名或使用 `.exe` 后缀。对于中间启动器过快退出而未被采集的情况，可接续启动之后在同目录创建的窗口进程；不接纳启动前已存在的工具。进程身份同时核对创建时间和退出时间，避免 PID 重用以及保留句柄的已退出对象被当成运行进程。每轮独立查询一次前台窗口，兼容带 owner 的游戏对话框。

`runtime/monitor/confidence.rs` 以时间常数 2 秒的 EMA 积累证据，可见窗口、前台窗口和启动关系分别贡献 0.35、0.40、0.25；仅靠同目录识别时使用一半关系权重。配置的游戏映像本身按强关联处理，覆盖 Steam 等中间层未保留父子关系的情况。目标仍存活时持续验证候选：分数至少 0.6、领先当前目标至少 0.3、领先其他候选至少 0.25，且优势连续保持 5 秒才换绑，每次会话最多执行 3 次此类换绑。候选的创建时间和首次观察时间不得早于目标，已经替换掉的进程不再被选回。游戏最小化或切换焦点不会直接停止计时，换绑本身保持同一个游戏的计时检查点和自动备份会话。

目标退出后的唯一关联窗口可立即接续；无父子关联的目录候选和多个窗口并存时，需要至少 3 秒的观察、0.6 的分数和 0.25 的领先差距。没有明确领先者时保留 `Unknown`。启动器退出后仅剩一个有映像路径的关联进程，连续观察 10 秒后可确认，覆盖无窗口加载过程。超过 3 秒的采样中断会清除置信度和待确认的连续优势，休眠或采集失败期间不补算证据。

目标退出后保留 3 秒交接观察期；检查失败、权限不足或尚未确认交接时返回 `Unknown`，不累计不确定时间，也不触发结束自动备份。确认没有后继进程才返回 `Stopped`。会话证据只保存在内存中；应用重新启动后仍能按原有的精确路径/同目录同名 PE 识别运行进程，但无法还原已退出启动器的历史父子链。手工从外部启动、且配置 EXE 早已退出的异名临时进程仍可能无法识别。

设计参考：[LunaBox：如何较为正确地判断一个 galgame 真正的进程](https://www.kungal.com/topic/4648)。本实现保留 GalGear 的 `Running / Stopped / Unknown` 协议，未引入额外依赖。Windows 隔离进程测试会让测试启动器先退出，再观察更名为 `.tmp`、带窗口的实际子进程；规则测试覆盖 PID 重用、驻留启动器、最小化、后继交接和计时/会话结束。


## 离线游戏库与全局存档备份

游戏目录失效时只更新 `Game.available`，保留条目、资料、收藏和游玩记录。UI 在库卡片和首页卡片右上角显示红色圆形感叹号，允许进入详情编辑资料，禁用启动与游戏配置。提交启动配置时桌面端重新检查路径，覆盖编辑过程中磁盘离线的情况。

桌面 `library_devices` 每两秒在后台读取 Windows 逻辑磁盘位图与已配置游戏库的可访问状态，变化时发出 `library-devices-changed`。UI 复用非阻塞 `BackgroundScan`，连续变化在当前扫描结束后合并补扫，完成时静默更新。扫描器按原目录缓存沿用手动导入或重新定位后的 ID。

条目删除统一永久清除资料、配置、收藏归属和游玩记录，以 `excluded_paths` 保存最小扫描排除路径。显式添加游戏或移除后重新添加库目录可重新导入，旧版排除档案仍可通过原入口查看与恢复。磁盘游戏、存档与备份由独立操作管理。

`core::snapshots::storage` 负责全局目录中的 `games.json` 身份索引与时间命名。`backup_folder` 为空时使用应用数据目录的 `save-backups`，`backup_legacy_folders` 记录待迁移来源，旧配置默认空。核心迁移先校验并发布完整副本，重试复用已发布内容，然后清理源目录；改名同时更新 manifest 和恢复日志。目录和索引路径拒绝符号链接及 Windows 重解析点。桌面备份、恢复和目录设置共用 `state::begin_write`，沿用现有运行状态检查、恢复预览和回滚。
