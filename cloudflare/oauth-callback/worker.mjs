// NextMoe 中继与 Bangumi 内置应用换码。不持久化用户令牌或记录凭据。
const ORIGIN = "https://galgear.cbshuimu.net";
const BANGUMI_CLIENT_ID = "bgm74746aead5bc3bbb8";
const BANGUMI_REDIRECT = `${ORIGIN}/oauth/bangumi/callback`;
const encoder = new TextEncoder();
const CALLBACKS = new Map([
  ["/oauth/bangumi/callback", "bangumi"],
  ["/oauth/nextmoe/callback", "nextmoe"],
]);

function response(body, status = 200, extraHeaders = {}) {
  return new Response(body === null ? null : JSON.stringify(body), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": "no-store",
      "Referrer-Policy": "no-referrer",
      "Content-Security-Policy": "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
      "X-Content-Type-Options": "nosniff",
      "X-Frame-Options": "DENY",
      "Strict-Transport-Security": "max-age=31536000",
      ...extraHeaders,
    },
  });
}

async function callback(url, provider, env) {
  if (url.search.length > 8192) {
    return response({ error: "invalid_callback" }, 400);
  }
  const params = url.searchParams;
  // 不接受重复字段，防止不同解析器选取不同的 state/code。
  const keys = new Set();
  for (const [key] of params) {
    if (keys.has(key)) return response({ error: "invalid_callback" }, 400);
    keys.add(key);
  }
  const state = params.get("state") ?? "";
  const broker = state.startsWith("gg2.");
  const match = broker ? await sessionParts(state, env)
    : /^gg1\.(bangumi|nextmoe)\.([1-9][0-9]{3,4})\.([A-Za-z0-9_-]{43})$/.exec(state);
  if (!match || match[0] !== state || match[1] !== provider) return response({ error: "invalid_state" }, 400);
  const port = Number(match[2]);
  if (port < 1024 || port > 65535) {
    return response({ error: "invalid_state" }, 400);
  }
  const code = params.get("code");
  const error = params.get("error");
  // 成功与失败只能取其一；控制字符不能进入响应头或本机请求。
  const result = code ?? error;
  if ((code !== null) === (error !== null) || !result || result.length > 2048 || /[\u0000-\u0020\u007f]/u.test(result)) {
    return response({ error: "invalid_callback" }, 400);
  }
  // 目标只由受限端口组成，绝不使用外部传入的 URL、host 或 path。
  const target = new URL(`http://127.0.0.1:${port}/callback`);
  target.searchParams.set("state", state);
  // 内置 Bangumi 的原始 code 不进入环回 URL；临时票据绑定本次 state，拦截方仍需 verifier。
  target.searchParams.set(code !== null ? "code" : "error",
    broker && code !== null ? await sealCode(code, state, env) : result);
  // 302 只表示已转交；只有本机换码并保存成功，才能宣布登录成功。
  return response(null, 302, { Location: target.href });
}

function base64(bytes) {
  return btoa(String.fromCharCode(...new Uint8Array(bytes))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/u, "");
}

function unbase64(value) {
  return Uint8Array.from(atob(value.replaceAll("-", "+").replaceAll("_", "/")), c => c.charCodeAt(0));
}

async function signingKey(env) {
  return crypto.subtle.importKey("raw", encoder.encode(env.OAUTH_STATE_SECRET), { name: "HMAC", hash: "SHA-256" }, false, ["sign", "verify"]);
}

async function ticketKey(env) {
  const key = await crypto.subtle.digest("SHA-256", encoder.encode(`galgear-ticket-v1:${env.OAUTH_STATE_SECRET}`));
  return crypto.subtle.importKey("raw", key, "AES-GCM", false, ["encrypt", "decrypt"]);
}

async function sessionParts(state, env) {
  if (typeof state !== "string" || !env.OAUTH_STATE_SECRET) return null;
  const match = /^gg2\.(bangumi)\.([1-9][0-9]{3,4})\.([A-Za-z0-9_-]{43})\.([0-9]{10})\.([A-Za-z0-9_-]{43})\.([A-Za-z0-9_-]{43})$/.exec(state);
  const now = Math.floor(Date.now() / 1000);
  if (!match || match[0] !== state || Number(match[2]) < 1024 || Number(match[2]) > 65535
    || Number(match[4]) <= now || Number(match[4]) > now + 180) return null;
  const valid = await crypto.subtle.verify("HMAC", await signingKey(env), unbase64(match[6]), encoder.encode(state.slice(0, -44)));
  return valid ? match : null;
}

async function sealCode(code, state, env) {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const encrypted = await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: encoder.encode(state) }, await ticketKey(env), encoder.encode(code));
  return `ggt1.${base64(new Uint8Array([...iv, ...new Uint8Array(encrypted)]))}`;
}

async function unsealCode(ticket, state, env) {
  if (typeof ticket !== "string" || !/^ggt1\.[A-Za-z0-9_-]{38,5500}$/u.test(ticket)) return null;
  try {
    const bytes = unbase64(ticket.slice(5));
    const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: bytes.slice(0, 12), additionalData: encoder.encode(state) }, await ticketKey(env), bytes.slice(12));
    return new TextDecoder().decode(plain);
  } catch { return null; }
}

// 请求与上游响应均有体积上限，不能信任 Content-Length（可能缺失或伪造）。
async function readJson(message, limit = 16384) {
  if (!message.body) throw new Error("missing_body");
  const reader = message.body.getReader();
  const chunks = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > limit) throw new Error("body_too_large");
      chunks.push(value);
    }
  } finally { await reader.cancel(); }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  return JSON.parse(new TextDecoder().decode(bytes));
}

function validText(value, max = 2048) {
  return typeof value === "string" && value.length > 0 && value.length <= max && !/[\u0000-\u0020\u007f]/u.test(value);
}

function randomToken(value) {
  return typeof value === "string" && value.length === 43 && /^[A-Za-z0-9_-]+$/u.test(value);
}

async function brokerRequest(request, path, env) {
  if (!env.OAUTH_STATE_SECRET || !env.BANGUMI_CLIENT_SECRET) return response({ error: "service_unavailable" }, 503);
  if (request.headers.get("Content-Type")?.split(";")[0].trim() !== "application/json") return response({ error: "invalid_request" }, 415);
  let body;
  try { body = await readJson(request); } catch { return response({ error: "invalid_request" }, 400); }
  if (!body || typeof body !== "object" || Array.isArray(body)) return response({ error: "invalid_request" }, 400);
  if (path.endsWith("/session")) {
    if (!Number.isInteger(body.port) || body.port < 1024 || body.port > 65535
      || !randomToken(body.nonce) || !randomToken(body.code_challenge)) return response({ error: "invalid_request" }, 400);
    const state = `gg2.bangumi.${body.port}.${body.nonce}.${Math.floor(Date.now() / 1000) + 180}.${body.code_challenge}`;
    const signature = base64(await crypto.subtle.sign("HMAC", await signingKey(env), encoder.encode(state)));
    return response({ state: `${state}.${signature}` });
  }
  const form = new URLSearchParams({ client_id: BANGUMI_CLIENT_ID, client_secret: env.BANGUMI_CLIENT_SECRET, redirect_uri: BANGUMI_REDIRECT });
  if (body.grant_type === "authorization_code") {
    const parts = await sessionParts(body.state, env);
    if (!parts || !randomToken(body.code_verifier)) return response({ error: "invalid_grant" }, 400);
    const challenge = base64(await crypto.subtle.digest("SHA-256", encoder.encode(body.code_verifier)));
    if (challenge !== parts[5]) return response({ error: "invalid_grant" }, 400);
    const code = await unsealCode(body.code, body.state, env);
    if (!validText(code)) return response({ error: "invalid_grant" }, 400);
    form.set("code", code);
    form.set("state", body.state);
  } else if (body.grant_type === "refresh_token" && validText(body.refresh_token, 4096)) {
    form.set("refresh_token", body.refresh_token);
  } else { return response({ error: "invalid_grant" }, 400); }
  form.set("grant_type", body.grant_type);
  try {
    // Workers 不支持 redirect:error，使用 manual 并拒绝 3xx，防止 Secret 随重定向外发。
    // 固定上游、有限超时、不重试，避免重复消费授权码和轮换刷新令牌。
    const upstream = await fetch("https://bgm.tv/oauth/access_token", {
      method: "POST", body: form, redirect: "manual", signal: AbortSignal.timeout(15000),
      headers: { Accept: "application/json", "User-Agent": "GalGear (https://github.com/CbShuiMu/GalGear)" },
    });
    if (upstream.status >= 300 && upstream.status < 400) {
      await upstream.body?.cancel();
      return response({ error: "upstream_error", reason: "redirect", upstream_status: upstream.status }, 502);
    }
    let value;
    try { value = await readJson(upstream); }
    catch { return response({ error: "upstream_error", reason: "invalid_response", upstream_status: upstream.status }, 502); }
    if (!upstream.ok || value.error) return response({ error: "invalid_grant" }, 400);
    if (!validText(value.access_token, 4096) || !validText(value.refresh_token, 4096)
      || !Number.isSafeInteger(value.expires_in) || value.expires_in <= 0) return response({ error: "upstream_error" }, 502);
    return response({ access_token: value.access_token, refresh_token: value.refresh_token, expires_in: value.expires_in, token_type: "Bearer" });
  } catch (error) {
    // 仅输出固定分类，异常文本可能包含地址或敏感请求上下文。
    return response({ error: "upstream_error", reason: ["TimeoutError", "AbortError"].includes(error?.name) ? "timeout" : "transport" }, 502);
  }
}

export default {
  async fetch(request, env = {}) {
    const url = new URL(request.url);
    if (url.origin !== ORIGIN) return response({ error: "invalid_origin" }, 421);
    const broker = ["/oauth/bangumi/session", "/oauth/bangumi/token"].includes(url.pathname);
    const method = broker ? "POST" : "GET";
    if (request.method !== method) {
      return response({ error: "method_not_allowed" }, 405, { Allow: method });
    }
    // 浏览器不直接调用换码 API；拒绝跨站 Origin，且不开放 CORS。
    if (broker && request.headers.has("Origin")) return response({ error: "invalid_origin" }, 403);
    try {
      if (broker) return await brokerRequest(request, url.pathname, env);
      if (url.pathname === "/healthz") return response({ status: "ok" });
      if (url.pathname === "/") {
        return response({
          service: "galgear-oauth-callback",
          protocol: "gg1",
          bangumi_protocol: "gg2",
          callbacks: Object.fromEntries([...CALLBACKS].map(([path, provider]) => [provider, `${ORIGIN}${path}`])),
        });
      }
      const provider = CALLBACKS.get(url.pathname);
      return provider ? await callback(url, provider, env) : response({ error: "not_found" }, 404);
    } catch { return response({ error: "service_unavailable" }, 503); }
  },
};
