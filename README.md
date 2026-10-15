<div align="center">

<img src="src-tauri/icons/icon.png" width="88" height="88" alt="GalGear 粉色花朵图标" />

# GalGear

**把游戏、资料、存档和补丁，整理进同一个 Galgame 游戏库。**

Windows x64 · Rust / Tauri 2 · WebAssembly · 中文 / 日本語 / English

[开始使用](#开始使用) · [功能](#功能) · [开发与构建](#开发与构建) · [项目文档](#项目文档) · [MIT](LICENSE)

</div>

![GalGear 游戏库：海报网格、搜索、标签筛选及批量操作](assets/readme/library.png)

<p align="center"><sub>实际 Rust/WASM 界面的隔离测试截图；游戏名称、封面和资料为演示数据。</sub></p>

GalGear 是面向本地已安装 EXE 游戏的 Windows 桌面管理器。添加游戏目录后，可以浏览海报、整理收藏夹、补全作品资料、启动游戏，并在详情页管理存档与备份。粉色界面支持亮色与暗色主题。

> [!IMPORTANT]
> 当前版本为 `0.1.0`，仍在开发和验收中。Windows 10、安装器、部分补丁格式及真实外部工具组合尚未完成全部验证，详见[当前限制](#当前限制)。仓库目前提供源码，尚未发布 GitHub Release。

## 功能

| 功能 | 当前支持 |
| --- | --- |
| 游戏库 | 多目录扫描、缓存启动、后台检查、手动添加、搜索、标签筛选、排序、游玩状态与路径失效提示。 |
| 收藏夹 | 嵌套层级、多重归属、拖放整理、批量加入和移除，以及排除条目的恢复。 |
| 游戏资料 | 关联 2DFan、VNDB、Bangumi、月幕 Galgame；缓存简介、制作人员、发行信息和封面，保留手动编辑及主动清空的字段。 |
| 启动配置 | 指定 EXE、参数、工作目录和外部工具，检测游戏运行状态；可接入 Locale Emulator 与 Magpie。 |
| 存档管理 | 检测游戏目录、AppData 和实际“文档”位置；支持手动确认、多位置快照、备注、锁定、历史浏览及恢复预览。 |
| 补丁管理 | 搜索 2DFan 与鲲 Gal 资源；支持可用直链下载、本地补丁预览、覆盖确认、备份与恢复。 |
| 界面偏好 | 中日英三语、亮暗主题、字号和封面缩放、关闭行为与托盘设置。 |

<details>
<summary>查看游戏详情界面</summary>

![GalGear 游戏详情：启动入口、资料来源、标签及存档位置](assets/readme/detail.png)

此图来自隔离测试，展示资料与存档入口的布局。启动按钮旁的齿轮用于配置程序、参数、转区和超分选项。

</details>

## 开始使用

运行环境为 Windows x64，需要 Microsoft Edge WebView2 Runtime。Windows 10 / 11 是目标平台，Windows 10 尚待实机验收。当前请先按[开发与构建](#开发与构建)生成程序。

1. 启动 `artifacts/GalGear.exe`，通过“添加”选择游戏库目录，或手动选择游戏 EXE。
2. 等待扫描条目出现。可以立即搜索、筛选或整理收藏夹；扫描期间支持取消。
3. 打开游戏详情，检查启动程序；在设置的“数据源”中选择默认资料来源，再搜索并关联作品。
4. 确认存档位置后创建第一份备份，再按需使用补丁或恢复功能。

### 资料来源与账号

作品资料来自 **2DFan、VNDB、Bangumi、月幕 Galgame**，补丁搜索来自 **2DFan、鲲 Gal**。资料和图片的范围取决于来源网站，游戏库和已缓存资料可在离线时使用。

- 可切换默认资料来源；已关联作品刷新时沿用其原来源。本地及手动指定封面优先，自动封面不会覆盖已有图片。
- 需要认证的来源在“设置 → 数据源”配置。Token、Cookie 和客户端密钥保存在本机 Windows 凭据管理器中；2DFan 的 `authenticity_token` 单独不足以建立有效会话。
- 2DFan 支持网站提供的直链、免费领取及逐项确认的积分购买；费用或余额无法确认时停止购买。鲲 Gal 当前仅搜索全年龄展示范围，并提供原站链接。
- 月幕 Galgame API 用于非商业项目。接口接入与资料来源说明见[数据源文档](docs/interface-and-data-sources-2026-10-14.md)。

### 存档与补丁

存档检测会区分已确认位置、候选目录和尚未创建的位置。备份支持多个确认目录；恢复前需要预览和确认，并先备份当前状态。游戏运行中或运行状态无法确认时，相关写入操作会受到限制。

本地 **ZIP** 补丁无需额外解压工具，**RAR / 7z** 需要本机 7-Zip。安装前显示文件及覆盖冲突，安装后保留备份；恢复时校验文件，外部修改会阻止恢复，重叠补丁必须逆序撤销。EXE 补丁使用其自身安装流程，退出码不会被当成安装成功的证明。

### 数据保存

| 运行方式 | 数据位置 |
| --- | --- |
| 普通 EXE / 安装版 | `%APPDATA%/com.galgear.desktop` |
| 便携模式 | 在 EXE 旁创建 `portable.flag`，数据保存到同目录的 `data/` |

迁移便携版时一起移动程序和 `data/`。账号凭据仍绑定本机 Windows 凭据管理器，迁移后需要重新配置。移除游戏条目只改变列表，不删除游戏、存档或备份。

## 开发与构建

准备 Windows x64、支持 Rust 2024 edition 的 Rust 工具链、Visual Studio C++ Build Tools / Windows SDK，以及 WebView2 Runtime。以下命令在项目根目录执行。

```powershell
git clone https://github.com/CbShuiMu/GalGear.git
cd GalGear

# 下载与项目版本一致的 wasm-bindgen，并安装 WASM 编译目标
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup.ps1

# 编译前端并运行桌面程序
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/dev.ps1
```

仓库为私有库，克隆需要有访问权限的 GitHub 账号。前端由 Rust 编译为 WASM；日常开发和独立 EXE 构建不需要 Node.js。

**生成独立 EXE：**

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1
```

输出为 `artifacts/GalGear.exe`。构建脚本先生成 `dist/` 前端，再编译桌面程序。

<details>
<summary>构建 NSIS 安装包</summary>

此步骤额外需要 Node.js / npm，用于运行 Tauri CLI。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-frontend.ps1 -Release
npx @tauri-apps/cli@2 build --bundles nsis
```

安装包输出到 `target/release/bundle/nsis/`。生成安装包不等同于安装与运行验收通过。

</details>

**运行检查：**

```powershell
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p galgear-ui --target wasm32-unknown-unknown --locked -- -D warnings
```

### 项目结构

```text
crates/core/    扫描、资料来源、收藏夹、运行检测、存档快照与补丁事务
crates/ui/      Rust/WASM 页面、状态与交互
crates/i18n/    中日英文本资源与结构化消息
src-tauri/     桌面窗口、原生对话框、进程启动与 IPC
web/           样式、站点图标与 DOM / IPC 桥接
scripts/       环境准备、开发及构建脚本
assets/readme/ README 界面截图
docs/          需求、设计、实现记录与验收清单
```

三语文本集中在 `crates/i18n/locales.json`。`dist/`、`target/`、`tools/` 和 `artifacts/` 为本地生成内容，不随源码提交；引擎实验与游戏素材位于本地 `analysis/`，也不随源码上传。

## 当前限制

- 当前只管理已安装的 EXE 游戏；不提供光盘挂载、镜像安装或内置 CG / 回想解锁。
- 下载采用单任务模式；持久化队列、断点续传、下载历史、不同版本关联和作品分组暂缓。
- 存档识别有引擎与搜索范围限制。复杂搬迁冲突、补丁历史目标重新绑定和异常退出后的下载临时文件清理仍需完善。
- RAR 实包、安装器、Windows 10、真实游戏退出备份，以及 Locale Emulator / Magpie 的实际效果尚需进一步验收。
- 更新检查读取 GitHub Releases 的公开 API。当前仓库为私有库且未发布 Release，应用内尚未提供 GitHub 私有仓库认证，暂不能通过该入口检查此仓库更新。

## 项目文档

- [需求基线](docs/requirements.md)：产品范围与已确认需求。
- [游戏页与收藏夹设计](docs/library-page-design.md)：列表、层级及批量操作。
- [界面与数据源实现](docs/interface-and-data-sources-2026-10-14.md)：详情配置、备份编号与四个资料来源。
- [实现与验证记录](docs/implementation-status.md)：历史实现证据，需结合各记录日期阅读。
- [发布验收清单](docs/release-checklist.md)：验收目标及待补场景，历史条目可能落后于后续实现。
- [领域术语](GLOSSARY.md)：项目中的核心概念。
