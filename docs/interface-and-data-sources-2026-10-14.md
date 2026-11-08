# 2026-10-14 游戏页面、详情与数据源调整

## 本轮行为

- DOM 桥接按节点身份更新内容，连续输入保留输入节点、焦点和选区；详情异步返回保留详情滚动容器与图片。搜索收起为 34px 圆形，展开为两端半圆。
- 游戏工具栏常态与批量模式最小高度均为 66px。1240×820 下实测两种模式高度一致，无横向溢出。
- 删除“游戏资料”标题，来源 logo 与站名位于资料标题行左侧；查看原站、刷新资料、更改资料关联位于同一行右侧。
- 右侧配置及游戏位置移入启动按钮旁的齿轮弹窗，转区与超分为可保存的勾选项，默认均不勾选；新游戏及缺少启动配置的旧游戏使用普通启动，已有明确保存的选择继续生效。删除右侧刷新及折叠区域和其中的内容。
- 启动遵循勾选的转区配置；超分勾选时，检查已配置并运行的 Magpie，再等待游戏窗口后发送已配置快捷键。若设置为启动后退出，在快捷键发送后退出。真实游戏与外部工具组合的缩放效果仍需本机验收。
- 存档卡片内部显示备份历史名称。备份按全局目录中的游戏名分类，目录和显示名称采用本地时间 `YYYY-MM-DD-HH-mm`，同一分钟追加区分编号。内部保留递增序号以兼容历史管理，删除历史或执行保留清理不会重置。旧历史迁移会校验内容并同步更新目录、manifest 与恢复日志。
- 设置中的“账号”改为“数据源”，“关于”分类改为“更新”。
- 设置与资料关联支持 2DFan、VNDB、Bangumi、月幕 Galgame；可设默认来源，搜索候选、读取简介及制作资料、关联、刷新、缓存及允许来源的封面下载沿用现有流程。自动补资料使用默认来源，已有资料刷新沿用其原来源。
- VNDB/Bangumi Token 与月幕私有客户端密钥保存到 Windows 凭据管理器，普通配置、缓存和源码不存放令牌。新数据源设置返回保存状态，页面不会回填已保存的令牌。保存时留空保留当前值，清除按钮显式删除凭据。
- 月幕使用官方公共客户端，也可填写自己的 Client ID/Client Secret；Access Token 按有效期缓存，在 HTTP 401 时更新后重试一次。

## 验证

- 原有 Rust 单元测试通过，新增数据源解析、旧备份迁移及备份编号删除后继续增长的回归检查通过。
- `cargo check --workspace`、`cargo clippy --workspace --all-targets -- -D warnings` 通过。
- 使用实际 WASM 与生产 DOM 桥接的隔离传输夹具检查连续输入、延迟详情响应、备份列表、齿轮勾选、默认数据源、logo 加载及更新分类。夹具不启动真实游戏，不改写真实存档。
- 真实接口验证：VNDB 返回《千恋＊万花》资料；Bangumi 返回《CLANNAD》资料；月幕返回《千恋万花》资料。用户提供的两个令牌已验证并保存到本机凭据管理器。

截图与夹具：`artifacts/design/interface-20261014-qa/`。
独立 EXE：`artifacts/GalGear-20261014.exe`。旧版本保留。

## 接口参考

- [VNDB Kana API](https://api.vndb.org/kana)
- [Bangumi 官方 OpenAPI](https://github.com/bangumi/api/blob/master/open-api/v0.yaml)
- [月幕 Galgame 开放 API](https://www.ymgal.games/developer)

接口结果受来源站点的资料范围、账号权限和请求限制影响。月幕 API 用于非商业项目；本项目在说明中标明数据来源。


## 2026-11-04：NextMoe 补丁与用户授权

- NextMoe 目录默认匿名访问，在标签、资料和图片的数据源胶囊中始终可选；新配置及缺少来源时默认使用 NextMoe，已有明确选择继续生效。设置中的 NextMoe 卡片位于 2DFan 前。所有共享胶囊中 NextMoe 与 2DFan 位置互换。
- 2026-11-07 更新：资料、标签和封面来源胶囊及新游戏导入跟随全局“默认数据源”，各入口手动选择后保留偏好。补丁筛选跟随支持的默认来源；默认来源不提供补丁时显示全部。已关联游戏保留原来源和候选，刷新资料不改变关联。显示名称统一为“鲲Gal”，接口与凭据标识保持 NextMoe。
- 2DFan 查询自动读取用户已保存的 Token/Cookie，按已配置的 CSRF 或 Bearer 模式发送 Token，并同时携带 Cookie；支持只填写其中一项，均未填写时匿名。资料、标签、封面候选和补丁搜索复用账号会话的鉴权规则，读取失败不会静默退回匿名。凭据仍保存在 Windows 凭据库，不进入普通配置、缓存或其他数据源请求；能否读取受限条目由网站实际账号权限决定。
- 设置 → 数据源的 NextMoe 卡片仅保留补丁应用密钥输入框；通过 OAuth 登录取得用户令牌后，右上角显示绿色“已登录”。有已存用户令牌时自动使用令牌，无令牌时匿名访问。旧版 `nmk_` 目录凭据迁移到补丁字段，不再当作用户令牌。保存补丁密钥不会改动已有用户令牌及刷新令牌；密钥留空保留已存值，清除删除该数据源全部凭据。
- 补丁名称先经公开 catalog 解析为作品 ID，再以 `refs=catalog:<id>` 查询 `/v2/moyu/patches`，按补丁 ID 逐页读取资源。NextMoe 结果排在 2DFan 前。一个来源失败保留另一来源结果并显示失败提示，搜索栏不再放置补丁站入口，腾出的宽度交给搜索框；结果行仍保留对应资源的网页入口。“令牌配置”直接跳转设置的数据源页。
- NextMoe 结果提供“下载文件”，使用列表返回的资源 ID 调用资源详情，再调用 `/v2/moyu/resources/{id}/file`，不从展示网页 URL 推测 API ID。仅 `storage=s3` 托管文件可直下；缺少密钥或返回 401/403 时在右下角提示填写具有运营授予的 `moyu:files` 权限的应用密钥。外链或文件不存在时提示使用补丁站入口。只在点击时获取直链，不缓存、不自动重试，也不把应用密钥发送给下载主机。
- 文件传输复用 core 的进度、取消、8 GiB 上限和原子落盘规则，校验实际字节数与接口声明值，保留原压缩包并避免覆盖已有文件；不会宣称已验证 BLAKE3。按游戏搜索的资源下载到该游戏的补丁目录，手动搜索下载到配置的下载目录。返回解压密码时在完成通知中显示，通知保留至手动关闭。后续安装继续走本地补丁导入与预览。
- 下拉菜单使用 CSS 内容固有宽度及视口边界布局，不再以已撑开的 flex 容器宽度反推内容宽度；连续刷新、字体调整和长启动器名称均不会逐次撑宽菜单。回归检查：`node --test scripts/test-choice-layout.cjs`，另有真实浏览器布局检查。
- NextMoe 与 Bangumi 卡片的保存按钮右侧提供 OAuth 登录，使用 GalGear 发布版内置应用，无需填写应用配置。两者均使用 `https://galgear.cbshuimu.net/oauth/<provider>/callback`，中继到动态本机端口。NextMoe 本机 PKCE S256 换码，仅申请 `catalog:read`（无需注册该 scope，`openid` 自动授予）。Bangumi 的 App Secret 仅保存在 Worker Secret，服务端处理换码和刷新；协议与兼容规则见 [OAuth 回调服务](oauth-callback.md)。
- 授权使用系统浏览器，后台环回监听验证路径和随机 state，三分钟超时，可取消，不阻塞页面导航。令牌与刷新令牌保存在 Windows 凭据库，不回填前端。刷新在后台执行，串行轮换并原地保存；失败明确报错，不退回匿名。旧版手动输入的短期令牌过期后，可通过 OAuth 重新登录。
- `getrandom 0.3` 直接复用锁文件已有版本，仅用于操作系统安全随机数；标准库没有密码学随机数接口，现有 SHA-256 和 Base64 继续用于 PKCE，不新增 OAuth 框架依赖。
- VNDB 官方接口只公开手动 Token；月幕只有客户端凭据模式，明确尚未提供用户授权码模式；2DFan 未找到可接入的公开 OAuth 契约，因此不添加误导性的用户登录按钮。

- 详情页封面复用游戏库卡片的 `--poster-shadow`，随明暗主题同步。

契约依据：[NextMoe 鉴权](https://developer.nextmoe.dev/docs/authentication)、[原生客户端](https://developer.nextmoe.dev/docs/native-app)、[moyu 联邦补丁](https://developer.nextmoe.dev/docs/moyu-patches)、[受限文件接口](https://developer.nextmoe.dev/docs/moyu/getResourceFile)、[Bangumi OAuth](https://github.com/bangumi/api/blob/master/docs-raw/How-to-Auth.md)、[VNDB Token](https://api.vndb.org/kana#authentication)、[月幕开放 API](https://www.ymgal.games/developer)。
