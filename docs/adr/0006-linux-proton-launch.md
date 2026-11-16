# ADR 0006：Linux 通过 umu 启动 Proton

状态：已采纳。范围：Linux x86_64 的游戏导入、匿名资料刮削与 Windows 游戏启动。

## 背景

GalGear 不依赖 Steam 商店游戏记录，也不应自行维护 Proton 与 Steam Runtime 的组合。Windows 的自定义启动器、转区与缩放配置不能直接用于 Linux。Linux 文件系统通常区分大小写，路径身份与本地资源协议也需要保留平台语义。

## 决策

- 使用用户安装的 [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)。默认调用 `umu-run`，沿用每游戏自定义启动器字段覆盖可执行路径。新增可选 `proton_path` 指向包含 `proton` 的目录；留空由 umu 管理默认版本及运行库。
- `core::runtime::proton` 仅构建命令与准备隔离的 prefix、日志；Tauri `tools` 继续拥有启动互斥、子进程生命周期及失败事件。参数通过 `Command` 传递，不拼接 shell 命令。
- `WINEPREFIX` 固定在应用数据目录的 `compatdata/<游戏 ID 的 SHA-256>/pfx`，不改写现有 Steam prefix。拒绝指向符号链接的游戏 prefix 和日志，避免重定向写入。
- 通过继承的游戏环境标识观察 `/proc` 中的启动树，只报告有证据的运行中状态。缺少证据时保持未知，存档写入继续遵守原有运行状态检查。外部启动识别与 prefix 内存档发现属于后续范围。
- UI 读取后端提供的平台字段，显示 Proton 配置并禁用 Windows 专用启动工具。配置文件不能覆盖平台字段。DOM 传输层通过 Tauri 的 `convertFileSrc` 转换本地封面、图标和背景协议。
- Linux 匿名刮削不依赖 Windows 凭据管理器；凭据保存仍明确返回不支持，待独立接入 Linux 系统密钥环。Linux 构建不包含 Windows 工具，也不应用 Windows 更新资产。

## 影响与验证

首次运行可能由 umu 下载较大的运行库，失败详情写入每游戏 `launch.log`。用户需安装 umu；GalGear 不分发 Proton 二进制。已有 Windows 配置字段保留默认值，新增字段对旧配置可选。

验证分为命令与路径边界测试、真实 WebKitGTK 的生产 IPC 测试，以及真实 Windows EXE 的 Proton 执行测试。Ubuntu WSL2 能验证这些链路，Steam Deck 游戏模式、手柄、图形与休眠行为仍需实机覆盖。操作与验证记录见 [Linux 开发与试用](../linux.md)。
