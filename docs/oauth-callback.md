# Cloudflare OAuth 回调服务

`galgear.cbshuimu.net` 由无状态 Worker `galgear-oauth-callback` 提供服务。源码和部署配置位于 `cloudflare/oauth-callback/`；没有数据库或第三方运行时依赖。用户令牌仅由桌面端写入 Windows 凭据管理器。

## 内置应用与接口

| 提供方 | 公开 Client ID | 注册的回调地址 |
| --- | --- | --- |
| NextMoe | `1588bff36b26df9d9e56d60f16fb0330` | `https://galgear.cbshuimu.net/oauth/nextmoe/callback` |
| Bangumi | `bgm74746aead5bc3bbb8` | `https://galgear.cbshuimu.net/oauth/bangumi/callback` |

| 接口 | 用途 |
| --- | --- |
| `GET /`、`GET /healthz` | 协议说明、健康检查 |
| `GET /oauth/<provider>/callback` | 接收 `code` 或 `error`，中继至本机 |
| `POST /oauth/bangumi/session` | JSON `{port, nonce, code_challenge}`，返回 `{state}` |
| `POST /oauth/bangumi/token` | JSON 授权码或刷新请求，返回访问令牌、刷新令牌和到期秒数 |

NextMoe 使用 `https://account.nextmoe.com/api/v1/oauth/authorize` 和 `/api/v1/oauth/token`；public client 强制 PKCE S256，没有 Client Secret。仅请求 `catalog:read`，该 scope 无需预先注册；`openid` 始终授予，无需勾选。Bangumi 使用 `https://bgm.tv/oauth/authorize` 与 `https://bgm.tv/oauth/access_token`；共享 App Secret 仅存于 Worker Secret，不放入源代码、普通配置或发行包。

## 本机授权与中继

桌面先绑定 `127.0.0.1:0`，生成相互独立的 32 字节随机 nonce 和 verifier（无填充 Base64URL）。系统浏览器使用上表的 HTTPS redirect URI；换码时也必须使用完全相同的 HTTPS 地址。本机 HTTP 回调只用于接收中继，不能充当上游换码的 redirect URI。

NextMoe 使用 `gg1.nextmoe.<port>.<nonce>` state，携带 `code_challenge=BASE64URL(SHA256(verifier))`、`code_challenge_method=S256` 与 `scope=catalog:read`。Worker 校验字段唯一性、提供方、端口及参数，转交固定 `http://127.0.0.1:<port>/callback`；本机验证完整 state 和路径后，直接向 NextMoe 提交换码请求和 verifier。保留 `gg1.bangumi` 中继兼容已有自建应用，它不提供内置应用换码能力。

内置 Bangumi 使用 `gg2`：

1. 桌面向 session 接口提交端口、nonce 和 SHA-256 challenge；verifier 只留在本机。
2. 服务端返回 `gg2.bangumi.<port>.<nonce>.<expiry>.<challenge>.<signature>`；HMAC-SHA256 签名覆盖前六段，三分钟到期。本机核对返回的端口、nonce 和 challenge 后将完整 state 交给 Bangumi。
3. 回调服务验证签名及期限，用 AES-GCM 将上游 code 封装成 `ggt1.<base64url(iv+ciphertext+tag)>`。完整 state 作为附加认证数据，票据不能挪用至另一会话。仅票据和 state 进入本机回调 URL，不把原始 Bangumi code 交给环回监听器。
4. 桌面向 token 接口提交 `{grant_type:"authorization_code", code:<ticket>, state, code_verifier}`。Worker 验证 state、challenge、票据后，用 Secret 和固定 App ID、HTTPS redirect URI 向 Bangumi 换码。上游授权码一次性、有效期 60 秒；不自动重试。
5. 刷新提交 `{grant_type:"refresh_token", refresh_token}`，服务端使用同一内置应用换取新令牌。请求体和响应均有体积上限；上游超时 15 秒且禁止重定向。只返回允许的令牌字段，错误不反射上游正文或 Secret。

本机只接受当前完整 state；拒绝、取消或三分钟超时关闭监听器，迟到回调不能保存凭据。换码、数据源验证和凭据保存全部成功后才显示成功。令牌刷新与手动保存/清除共用凭据事务，轮换串行持久化；刷新失败明确报错，不降为匿名。

## 兼容与服务边界

凭据新增 `oauth_redirect_uri` 和 `oauth_exchange`。旧记录默认 `direct`，旧 Bangumi 凭据没有 redirect 字段时继续使用 `http://127.0.0.1:38947/callback`；不使用内置 ID 覆盖旧应用刷新配置。重新 OAuth 登录后写入 HTTPS 地址、内置 ID 和交换方式，同时清除本地旧应用 Secret。NextMoe 补丁应用密钥保持独立，不参与 OAuth。

所有响应均设置 `Cache-Control: no-store`、`Referrer-Policy: no-referrer` 和限制性 CSP，不设置 Cookie、不开放 CORS、不加载第三方资源。仅接受固定 HTTPS origin；端口限定 1024–65535，不使用用户传入的目标 URL。服务端 POST 拒绝浏览器 Origin 请求。健康检查不代表两家上游可用。

Worker 不打印请求、令牌、授权码或错误正文。配置关闭自动请求日志和 Logpush，去除可观测性 URL 查询串并关闭追踪；Cloudflare 平台日志仍受账号策略管理，不应另开保留完整回调查询串的日志任务。HTTP 请求返回 421，不重定向带 code 的 HTTP 请求。

## 配置与验证

需要两个 Worker Secret：`BANGUMI_CLIENT_SECRET` 为平台注册的应用密钥；`OAUTH_STATE_SECRET` 为至少 32 字节密码学随机值，用于签名及票据密钥派生。密钥轮换会使三分钟内未完成的 Bangumi 会话失效，不影响已保存的刷新令牌。密钥通过 Cloudflare 连接器的 Secret API 或交互式 `wrangler secret put` 设置，不写入配置文件。重新上传时必须保留已有 Secret bindings。

```powershell
node --test scripts/test-oauth-callback.mjs
cargo test -p galgear-core --locked data_sources::oauth
npx wrangler deploy --config cloudflare/oauth-callback/wrangler.jsonc --dry-run --outdir (Join-Path $PWD 'artifacts/oauth-callback-build')
# 已登录对应账号并配置 Secret 后部署：
npx wrangler deploy --config cloudflare/oauth-callback/wrangler.jsonc
```

Custom Domain 自动维护 DNS 与 TLS，关闭 workers.dev 和预览地址。部署可通过 Cloudflare 连接器上传同一模块与配置，避免只改控制台而遗漏源码。

测试使用隔离的临时本机监听器、虚构 Secret 和上游替身，覆盖两家中继、内置应用换码参数、刷新兼容、取消、错误 state、过期/篡改票据、错误 verifier、请求体上限和错误脱敏。真实账号授权需要用户在系统浏览器中同意，不能以模拟测试代替。

2026-11-04：14 项 Worker 测试、Rust 完整测试、宿主/WASM Clippy、前端构建和 36 项桥接/交互测试通过。已部署内置应用服务并配置两个 Secret；线上用 GalGear 实际 User-Agent 验证健康检查、签名会话、两家本机中继、错误 verifier 拒绝和 Bangumi 无效授权码处理。Cloudflare 会拦截 Python 默认 User-Agent（1010）；客户端原有 User-Agent 可正常访问，未修改站点防护。Workers 不支持 `redirect: "error"`，上游请求使用 `manual` 并显式拒绝 3xx，防止密钥随重定向外发。尚未执行真实账号的同意授权及令牌获取。

## 文档依据

- [Bangumi 官方 OAuth 契约](https://github.com/bangumi/api/blob/master/docs-raw/How-to-Auth.md)
- [NextMoe 原生客户端接入](https://developer.nextmoe.dev/docs/native-app)
- [NextMoe 鉴权与凭据](https://developer.nextmoe.dev/docs/authentication)
- [Cloudflare Workers Custom Domains](https://developers.cloudflare.com/workers/configuration/routing/custom-domains/)
- [Cloudflare Workers 最佳实践](https://developers.cloudflare.com/workers/best-practices/workers-best-practices/)
