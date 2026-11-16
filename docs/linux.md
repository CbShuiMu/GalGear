# Linux 开发与试用

Linux x86_64 首版支持从目录扫描、手动添加 Windows `.exe`、匿名资料刮削、封面显示及通过 umu 启动 Proton。继续使用现有 Rust/WASM 界面与 Tauri 命令。

## 构建

Ubuntu 24.04 的开发依赖如下，其他发行版请使用对应的软件包。

```sh
sudo apt install build-essential curl pkg-config libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev librsvg2-dev libssl-dev
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127 --locked
bash scripts/build-linux.sh
./target/release/galgear
```

构建脚本会检查 wasm-bindgen CLI 与 UI crate 的版本是否一致。`tauri.linux.conf.json` 自动覆盖发行目标，移除 Windows 工具资源。安装 cargo-tauri 后，可运行 `cargo tauri build --bundles deb,appimage` 打包。AppImage 需要在所支持的最旧系统上构建，参见 [Tauri Linux 分发说明](https://v2.tauri.app/distribute/appimage/)。

直接运行未打包的 ELF 时需要系统提供 WebKitGTK 4.1、GTK 3 与 Ayatana AppIndicator。Ubuntu 24.04 可安装 `libwebkit2gtk-4.1-0 libayatana-appindicator3-1 librsvg2-2`。这不是已验证的 Steam Deck 安装包。

## 导入、刮削与启动

1. 在游戏库添加目录，或手动选择游戏的 `.exe`。
2. 在导入确认页选择资料源，搜索并确认词条，保存资料和封面。Linux 首版提供匿名访问，凭据保存暂未接入 Linux 系统密钥环，需要授权的数据源功能会返回错误。
3. 按 [umu 安装说明](https://github.com/Open-Wine-Components/umu-launcher#how-do-i-use-it) 安装 `umu-launcher`，确保桌面进程的 `PATH` 中有 `umu-run`。也可以在游戏详情的启动配置中，将“自定义启动器”填写为 `umu-run` 的绝对路径。
4. 可在“Proton 安装目录”填写包含 `proton` 文件的绝对目录。留空时由 umu 下载和维护默认 Proton，umu 也会下载所需的 Steam Runtime。首次启动需要网络和下载空间。
5. 点击启动。游戏参数仍按每行一个参数传递，支持空格与日文路径，全部通过进程参数 API 传递。

每款游戏使用应用配置目录内 `compatdata/<游戏 ID 的 SHA-256>/pfx` 中的独立 Wine prefix，路径与启动日志位于同一游戏目录，`launch.log` 在下次启动时覆盖。普通安装的配置通常位于 `~/.config/com.galgear.desktop/settings.json`。应用日志使用 `$XDG_STATE_HOME/galgear/logs`，未设置时使用 `~/.local/state/galgear/logs`。

umu 提供 Proton 与 Steam Runtime 的组合，GalGear 通过 `WINEPREFIX`、`PROTONPATH` 和参数数组传递启动配置。该方案兼容用户安装的 umu，不将第三方运行库或二进制提交到仓库。

## 当前边界

- Linux 按大小写区分游戏路径与条目身份。Windows 的已有配置仍保留原有默认值。
- 能识别由 GalGear 启动并继承游戏标识的进程。没有可验证的进程时返回“未知”，因此存档恢复等要求明确停止状态的操作仍受保护。其他启动器启动的游戏、Proton prefix 内存档自动发现及完整游玩计时尚待扩展。
- Locale Emulator、Magpie、字体注入、全局截图和 Windows AI 分析宿主尚未移植。Linux 无法开启转区和缩放；已有 Windows 配置可在详情中关闭这些选项后启动，启动端也会校验。
- Linux 暂不使用内置 Windows 更新器，更新通过重新构建或安装新包完成。
- Steam Deck 的游戏模式、内置手柄、休眠恢复及 SD 卡实机行为仍待验证。

## 验证

普通回归使用 `cargo test --workspace --locked`。原生烟雾测试使用隔离的临时游戏库、WebKitGTK profile 和模拟启动器，通过生产 IPC 验证目录导入、配置保存、运行状态及 WASM 渲染。

```sh
cargo test -p galgear --locked linux_native_import_scrape_launch -- --ignored --test-threads=1
# 显式联网查询 VNDB、保存资料并验证封面协议
GALGEAR_LINUX_LIVE=1 cargo test -p galgear --locked linux_native_import_scrape_launch -- --ignored --test-threads=1
```

无显示器的虚拟机可以安装 `xvfb dbus-x11`，在上述命令前加 `xvfb-run -a dbus-run-session --`。应使用普通用户运行原生和 Proton 测试。

本轮 WSL2 图形测试额外设置 `GDK_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1` 使用软件渲染，并提供由测试用户拥有、权限为 `0700` 的 `XDG_RUNTIME_DIR`。这些设置只用于虚拟机测试，不作为应用的默认图形配置。

真实 Proton 测试使用自行编译的 Windows 控制台夹具。它应将收到的参数用换行连接后写入当前目录 `proton-ok.txt` 并正常退出。设置 `GALGEAR_TEST_PE` 为该夹具绝对路径后，运行 `cargo test -p galgear-core --locked real_proton_runs_windows_fixture -- --ignored --test-threads=1`。该测试会触发 umu 的运行库准备，最多等待 15 分钟。它验证进程链和文件结果，游戏画面、视频与性能需要后续游戏及硬件测试。

可额外设置 `GALGEAR_TEST_PROTON` 为已安装的 Proton 目录，以测试指定版本。Windows 上可用以下隔离夹具生成测试 EXE：

```rust
fn main() {
    std::fs::write(
        "proton-ok.txt",
        std::env::args().skip(1).collect::<Vec<_>>().join("\n"),
    ).unwrap();
}
```

### 本轮验证记录

- 环境：独立 WSL2 虚拟机 `GalGear-Linux`，Ubuntu 24.04 x86_64；图形验证使用普通用户、Xvfb 与真实 WebKitGTK。该环境不能替代 SteamOS 或 Steam Deck。
- Windows 与 Linux 的全工作区测试通过；Windows 与 Linux Rust 1.94 的全目标 Clippy、WASM Clippy 通过；前端 69 项 JavaScript 测试通过。
- Linux 本机执行 `build-frontend.sh` 与 `build-linux.sh` 成功，生成包含 WASM 前端的 release 可执行文件；独立启动该文件后确认桌面初始化、页面加载及首次语言选择界面正常。
- 原生界面测试通过：含日文及空格路径的目录导入、VNDB 联网查询与资料持久化、本地封面协议、启动状态、重新加载后的 WASM 首页与封面解码。
- 真实启动测试通过：umu-launcher 1.4.4、UMU-Proton 10.0-4、Steam Runtime sniper 3.0.20261110.268881；Windows EXE 在独立 prefix 中成功运行，参数与输出文件内容吻合。下载超时后，使用本机代理取得并按官方摘要校验运行库；测试指定已安装的 Proton。
- `cargo fmt --all -- --check` 仍报告仓库既有格式差异，未批量格式化无关代码。Linux 最初使用 Rust 1.100 时还报告既有代码的新 Clippy 规则，随后使用与 Windows 一致的 1.94 验证通过。
- 尚未验证：deb/AppImage 安装包、真实游戏的视频和图形兼容、性能、Steam Deck 游戏模式、手柄与休眠恢复。
