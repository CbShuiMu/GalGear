import assert from "node:assert/strict";
import { createHash, createHmac, randomBytes } from "node:crypto";
import { createServer } from "node:http";
import { once } from "node:events";
import test from "node:test";
import worker from "../cloudflare/oauth-callback/worker.mjs";

const origin = "https://galgear.cbshuimu.net";
const nonce = randomBytes(32).toString("base64url");
const state = (provider, port = 38947) => `gg1.${provider}.${port}.${nonce}`;
const call = (provider, query, method = "GET") => worker.fetch(new Request(
  `${origin}/oauth/${provider}/callback?${new URLSearchParams(query)}`, { method },
));

for (const provider of ["bangumi", "nextmoe"]) {
  test(`${provider}: preserve code and state in a fixed loopback destination`, async () => {
    const expected = state(provider);
    const result = await call(provider, { state: expected, code: "a+b/c=&你好", redirect_uri: "https://evil.example" });
    assert.equal(result.status, 302);
    const location = new URL(result.headers.get("Location"));
    assert.equal(location.origin, "http://127.0.0.1:38947");
    assert.equal(location.pathname, "/callback");
    assert.deepEqual([...location.searchParams], [["state", expected], ["code", "a+b/c=&你好"]]);
  });

  test(`${provider}: forward denial without reporting success or reflecting error descriptions`, async () => {
    const result = await call(provider, { state: state(provider), error: "access_denied", error_description: "<script>secret</script>" });
    assert.equal(result.status, 302);
    const location = new URL(result.headers.get("Location"));
    assert.equal(location.searchParams.get("error"), "access_denied");
    assert.equal(location.searchParams.has("error_description"), false);
    assert.equal(location.searchParams.has("code"), false);
  });
}

test("reject invalid states and provider mixups without a redirect", async () => {
  for (const value of [
    "", nonce, state("nextmoe"), state("bangumi", 80), state("bangumi", 1023),
    state("bangumi", 65536), state("bangumi", "038947"), state("bangumi", "38947@evil.example"),
    state("bangumi").slice(0, -1), `${state("bangumi")}\r\n`, `${state("bangumi")}\n`, `gg2.bangumi.38947.${nonce}`,
  ]) {
    const result = await call("bangumi", { state: value, code: "test-code" });
    assert.equal(result.status, 400, value);
    assert.equal(result.headers.has("Location"), false);
  }
});

test("accept nonprivileged port boundaries", async () => {
  for (const port of [1024, 65535]) {
    const result = await call("nextmoe", { state: state("nextmoe", port), code: "test" });
    assert.equal(new URL(result.headers.get("Location")).port, String(port));
  }
});

test("reject missing, conflicting, repeated and oversized callback parameters", async () => {
  const validState = state("bangumi");
  for (const params of [
    { state: validState }, { state: validState, code: "" },
    { state: validState, error: "" }, { state: validState, code: "a", error: "access_denied" },
    { state: validState, code: "a\r\nLocation:https://evil.example" },
    { state: validState, code: "a\u0000b" }, { state: validState, code: "x".repeat(2049) },
    { state: validState, code: "a", other: "x".repeat(8192) },
    [["state", validState], ["state", validState], ["code", "a"]],
    [["state", validState], ["code", "a"], ["code", "b"]],
    [["state", validState], ["error", "access_denied"], ["error", "server_error"]],
  ]) {
    const result = await call("bangumi", params);
    assert.equal(result.status, 400);
    assert.equal(result.headers.has("Location"), false);
  }
});

test("apply privacy headers to success, errors, metadata and health", async () => {
  for await (const result of [
    call("bangumi", { state: state("bangumi"), code: "test" }),
    call("nextmoe", {}), call("unknown", {}),
    worker.fetch(new Request(`${origin}/`)), worker.fetch(new Request(`${origin}/healthz`)),
  ]) {
    assert.equal(result.headers.get("Cache-Control"), "no-store");
    assert.equal(result.headers.get("Referrer-Policy"), "no-referrer");
    assert.equal(result.headers.get("X-Content-Type-Options"), "nosniff");
    assert.equal(result.headers.has("Set-Cookie"), false);
    assert.equal(result.headers.has("Access-Control-Allow-Origin"), false);
  }
});

test("reject other origins, HTTP, unknown paths and unsupported methods", async () => {
  for (const url of ["http://galgear.cbshuimu.net/", "https://evil.example/", "https://galgear.cbshuimu.net:8443/"]) {
    assert.equal((await worker.fetch(new Request(url))).status, 421);
  }
  assert.equal((await call("unknown", {})).status, 404);
  for (const method of ["POST", "PUT", "HEAD", "OPTIONS"]) {
    const result = await call("bangumi", {}, method);
    assert.equal(result.status, 405);
    assert.equal(result.headers.get("Allow"), "GET");
  }
});

test("complete the relay against an isolated ephemeral loopback listener", async () => {
  let server;
  const received = new Promise(resolve => {
    server = createServer((request, response) => {
      resolve(new URL(request.url, "http://127.0.0.1"));
      response.end("test callback received");
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  try {
    const expected = state("nextmoe", server.address().port);
    const result = await call("nextmoe", { state: expected, code: "test-only" });
    const reply = await fetch(result.headers.get("Location"));
    assert.equal(await reply.text(), "test callback received");
    const url = await received;
    assert.equal(url.searchParams.get("state"), expected);
    assert.equal(url.searchParams.get("code"), "test-only");
  } finally {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
});

// 仅使用虚构 Secret、令牌和 fetch 替身，不消耗真实授权码。
const env = { OAUTH_STATE_SECRET: "fixture-state-secret", BANGUMI_CLIENT_SECRET: "fixture-app-secret" };
const post = (path, body, options = {}) => worker.fetch(new Request(`${origin}/oauth/bangumi/${path}`, {
  method: "POST", headers: { "Content-Type": "application/json", ...options.headers }, body: JSON.stringify(body),
}), options.env ?? env);
async function session(port = 38947) {
  const verifier = randomBytes(32).toString("base64url");
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  const result = await post("session", { port, nonce, code_challenge: challenge });
  assert.equal(result.status, 200);
  return { state: (await result.json()).state, verifier };
}
const brokerCallback = (state, code = "fixture-code") => worker.fetch(new Request(
  `${origin}/oauth/bangumi/callback?${new URLSearchParams({ state, code })}`,
), env);

test("built-in Bangumi binds encrypted callback tickets to state and verifier", async t => {
  const current = await session();
  const callback = await brokerCallback(current.state);
  assert.equal(callback.status, 302);
  const location = new URL(callback.headers.get("Location"));
  assert.equal(location.origin, "http://127.0.0.1:38947");
  assert.equal(location.searchParams.get("state"), current.state);
  const ticket = location.searchParams.get("code");
  assert.match(ticket, /^ggt1\./u);
  assert.equal(location.href.includes("fixture-code"), false);
  let calls = 0;
  t.mock.method(globalThis, "fetch", async (url, options) => {
    calls++;
    assert.equal(url, "https://bgm.tv/oauth/access_token");
    assert.equal(options.redirect, "manual");
    assert.equal(options.body.get("code"), "fixture-code");
    assert.equal(options.body.get("client_id"), "bgm74746aead5bc3bbb8");
    assert.equal(options.body.get("client_secret"), env.BANGUMI_CLIENT_SECRET);
    assert.equal(options.body.get("redirect_uri"), `${origin}/oauth/bangumi/callback`);
    assert.equal(options.body.get("state"), current.state);
    return Response.json({ access_token: "fixture-access", refresh_token: "fixture-refresh", expires_in: 3600, private_extra: "omitted" });
  });
  const grant = { grant_type: "authorization_code", state: current.state, code_verifier: current.verifier, code: ticket };
  const other = await session();
  for (const invalid of [
    { ...grant, code_verifier: other.verifier }, { ...grant, code: "raw-code" },
    { ...grant, code: `${ticket.slice(0, -3)}abc` },
    { ...grant, state: other.state, code_verifier: other.verifier },
    { ...grant, state: current.state.replace("38947", "38948") },
  ]) assert.equal((await post("token", invalid)).status, 400);
  assert.equal(calls, 0);
  const result = await post("token", grant);
  assert.equal(result.status, 200);
  assert.equal(result.headers.get("Cache-Control"), "no-store");
  assert.deepEqual(await result.json(), { access_token: "fixture-access", refresh_token: "fixture-refresh", expires_in: 3600, token_type: "Bearer" });
  assert.equal(calls, 1);
});

test("signed session rejects expiration, provider mixups and forged fields", async () => {
  const current = await session();
  const expired = current.state.split(".").slice(0, -1);
  expired[4] = String(Math.floor(Date.now() / 1000) - 1);
  const expiredState = `${expired.join(".")}.${createHmac("sha256", env.OAUTH_STATE_SECRET).update(expired.join(".")).digest("base64url")}`;
  for (const value of [expiredState, current.state.replace("bangumi", "nextmoe"), current.state.replace("38947", "38948")]) {
    assert.equal((await brokerCallback(value)).status, 400);
  }
  const denied = await worker.fetch(new Request(`${origin}/oauth/bangumi/callback?${new URLSearchParams({ state: current.state, error: "access_denied" })}`), env);
  assert.equal(new URL(denied.headers.get("Location")).searchParams.get("error"), "access_denied");
});

test("broker refreshes without exposing app secrets or upstream failures", async t => {
  let calls = 0;
  const upstream = t.mock.method(globalThis, "fetch", async (url, options) => {
    calls++;
    assert.equal(url, "https://bgm.tv/oauth/access_token");
    assert.equal(options.body.get("grant_type"), "refresh_token");
    assert.equal(options.body.get("refresh_token"), "fixture-refresh");
    assert.equal(options.body.get("client_secret"), env.BANGUMI_CLIENT_SECRET);
    return Response.json({ access_token: "rotated-access", refresh_token: "rotated-refresh", expires_in: 3600 });
  });
  const grant = { grant_type: "refresh_token", refresh_token: "fixture-refresh" };
  assert.equal((await post("token", grant)).status, 200);
  upstream.mock.mockImplementation(async () => Response.json({ error: "invalid_grant", detail: env.BANGUMI_CLIENT_SECRET }, { status: 400 }));
  const error = await post("token", grant);
  assert.equal(error.status, 400);
  assert.deepEqual(await error.json(), { error: "invalid_grant" });
  upstream.mock.mockImplementation(async () => { throw new Error(env.BANGUMI_CLIENT_SECRET); });
  assert.deepEqual(await (await post("token", grant)).json(), { error: "upstream_error", reason: "transport" });
  upstream.mock.mockImplementation(async () => new Response("x".repeat(16385)));
  assert.equal((await post("token", grant)).status, 502);
  upstream.mock.mockImplementation(async () => new Response(null, { status: 302, headers: { Location: "https://evil.example" } }));
  assert.deepEqual(await (await post("token", grant)).json(), { error: "upstream_error", reason: "redirect", upstream_status: 302 });
  assert.equal(calls, 1);
});

test("broker validates input size, methods, origin, and missing bindings", async () => {
  assert.equal((await post("session", {})).status, 400);
  assert.equal((await post("session", { port: 80, nonce, code_challenge: nonce })).status, 400);
  assert.equal((await post("session", { port: 38947, nonce: `${nonce}\n`, code_challenge: nonce })).status, 400);
  assert.equal((await post("token", {})).status, 400);
  assert.equal((await post("token", { padding: "x".repeat(16384) })).status, 400);
  assert.equal((await post("token", {}, { headers: { Origin: "https://evil.example" } })).status, 403);
  assert.equal((await post("token", {}, { headers: { "Content-Type": "text/plain" } })).status, 415);
  assert.equal((await post("token", {}, { env: {} })).status, 503);
  assert.equal((await worker.fetch(new Request(`${origin}/oauth/bangumi/token`), env)).status, 405);
});
