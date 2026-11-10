# 批评空间与首次语言选择

设置 → 数据源中的批评空间（ErogameScape）卡片紧接 VNDB，使用随应用打包的平台 favicon，无需账号或 Token。资料、标签与封面来源选择器共用来源注册表；批评空间提供游戏标题、品牌、发售日期、类型及站点已有的文本介绍。SQL 返回的介绍若为站外 URL，不自动访问或当作剧情文本。该接口没有统一封面字段，用户可继续从现有的其他来源选择封面。

## 查询接口

- 官方入口：[エロゲーマーのためのSQL](https://erogamescape.dyndns.org/~ap2/ero/toukei_kaiseki/sql_for_erogamer_form.php)。以 `POST application/x-www-form-urlencoded` 发送 `sql` 字段，返回 HTML 表格。
- 官方数据表说明：[テーブル一覧](https://erogamescape.dyndns.org/~ap2/ero/toukei_kaiseki/sql_for_erogamer_tablelist.php)。使用 `gamelist` 与 `brandlist` 的公开字段。
- 协议交叉核对：[Playnite ErogameScape 客户端](https://github.com/minuspiral/playnite-with-erogamescape/blob/main/Services/ErogameScapeApiClient.cs)；其 [v1.0.1 说明](https://github.com/minuspiral/playnite-with-erogamescape/releases/tag/v1.0.1) 记录了站点更新后的非 ASCII SQL 兼容问题。适配器为本项目独立实现，未引入第三方代码或依赖。

`core::data_sources::erogamescape` 只构造固定 SELECT。搜索将用户文本编码为 UTF-8 十六进制并转义 LIKE 通配符，最多返回 30 项；详情仅接受验证过的正整数 ID。站点 `erogamescape.org` 和 `erogamescape.dyndns.org` 的 HTTPS 游戏链接均可识别，重复参数、额外参数、非游戏路径、用户信息和非默认端口被拒绝。响应限制 2 MiB，复用已有连接与请求超时，不自动重试。缺失结果表或格式异常作为来源错误返回，不伪装成空结果。

2026-11-10 实现验证时，本环境访问官网发生 TLS 握手超时；查询协议已通过公开客户端交叉核对并用隔离 HTML 夹具测试，未声称通过真实站点的搜索/详情联调。可在可访问官网的环境运行：

```powershell
cargo run -p galgear-core --bin verify_data_sources --locked -- erogamescape artifacts/erogamescape/fixture-settings.json 千恋万花
```

## 首次启动

`ConfigStore::load` 仅在配置文件不存在时设置 `initial_setup_pending=true`。旧文件缺失字段时默认 false，配置损坏或读取失败仍报错，不重置用户配置。

首次启动的三语弹窗提供简体中文、English、日本語。用户点击语言后通过既有 `set_language` 命令一起保存语言、默认来源和已完成标记：中文使用 `nextmoe`，英语使用 `vndb`，日语使用 `erogamescape`。保存失败保留弹窗以便重试；后续手动切换语言不会改动已选数据源。VNDB 与批评空间的公开查询无需保存凭据即可使用。

UI `startup` 模块统一启动后续任务。完成首次语言保存后才开始版本检查、资料源状态读取和游戏库扫描，避免更新提示或资料确认遮挡语言选择。已有配置直接进入原有启动流程。弹窗复用键盘/手柄焦点边界，Escape 不跳过选择。

验证包括 core 配置迁移与接口解析测试、`scripts/test-startup.mjs` 的实际 WASM 请求/响应测试，以及真实 DOM 的 Logo、布局和焦点检查。使用隔离的便携版目录启动真实 Tauri 桌面程序，确认日语选择后 settings.json 同时保存 ja-JP、erogamescape 和已完成标记；重启不再显示语言选择。
