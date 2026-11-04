# 0002：发布版内置 OAuth 应用与服务端密钥边界

状态：采用，2026-11-04。

用户要求发布版使用 GalGear 的 NextMoe 与 Bangumi 应用，且已在两个平台注册 HTTPS 回调。NextMoe 是强制 PKCE 的公开客户端；Bangumi 官方授权码及刷新接口要求 App Secret，不能把共享密钥放进可分发的桌面程序。

桌面仅内置公开 Client ID 和固定端点。NextMoe 在本机生成 verifier、验证完整 state 并直接换码。Bangumi 使用现有 Cloudflare Worker 保管 App Secret，通过签名会话把本机端口、nonce、challenge 和期限绑定；回调将 code 封装为绑定完整 state 的 AES-GCM 票据。本机持有 verifier 才能通过服务端换码，环回拦截方不能使用票据。Worker 不存储用户令牌、授权码或会话，不转发任意 URL，不记录请求或响应凭据。

这增加了 Bangumi 登录和刷新对回调服务的依赖；NextMoe 换码与刷新仍直接访问提供方。上游一次性授权码提供票据的单次消费约束；失败不自动重试，拒绝、取消、超时都不覆盖现有凭据。HMAC 与票据使用不同用途的密钥派生，签名密钥只在 Worker Secret 中保存。

凭据持久化记录交换方式和原始 redirect URI。缺少新字段的旧记录继续按自建应用及原环回地址刷新；用户重新登录后切换到内置应用，移除旧本地 Secret。手动令牌和 NextMoe 补丁应用密钥仍沿用系统凭据库，补丁密钥不会用于用户授权。

契约依据：[NextMoe 原生接入](https://developer.nextmoe.dev/docs/native-app)、[Bangumi 授权](https://github.com/bangumi/api/blob/master/docs-raw/How-to-Auth.md)。
