// 先运行 scripts/build-frontend.ps1；使用真实 WASM 与隔离响应，不访问用户配置。
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';

const module = readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm', import.meta.url));
let instance = 0;
async function fixture(overrides = {}, systemDark = false) {
  const settings = {library_folders: [], card_size: 200, sort_by: 'title', theme: 'system',
    color_theme: 'white-pink', font_size: 16, ...overrides};
  const requests = [];
  let html = '';
  globalThis.GalGearBridge = {
    render(value) { html = value; }, reset_scroll() {}, schedule_toast_expiry() {},
    schedule_download_notice_expiry() {}, local_date() { return ''; }, local_date_time() { return ''; },
    request(id, command, payload) { requests.push({id, command, payload: JSON.parse(payload)}); },
  };
  const ui = await import(`../dist/pkg/galgear_ui.js?theme=${++instance}`);
  ui.initSync({module});
  ui.system_theme_changed(systemDark);
  function take(command) {
    const index = requests.findIndex(request => request.command === command);
    assert.notEqual(index, -1, command);
    return requests.splice(index, 1)[0];
  }
  function reply(request, value, success = true) {
    ui.resolve(request.id, success, JSON.stringify(value));
  }
  reply(take('bootstrap'), {settings, catalog: {games: [], warnings: []}, config_path: 'fixture', version: '0.6.6'});
  reply(take('get_data_sources'), []);
  ui.dispatch('page', 'settings');
  ui.dispatch('settings-section', 'display');
  return {ui, settings, requests, take, reply, get html() { return html; }};
}

function assertTheme(app, mode, resolved) {
  assert.match(app.html, new RegExp(`data-theme="${resolved}"`));
  assert.match(app.html, new RegExp(`data-action="set-theme" data-value="${mode}" aria-pressed="true"`));
  assert.match(app.html, /data-color-theme="white-pink"/);
}

for (const dark of [false, true]) {
  test(`system mode restores and follows ${dark ? 'dark' : 'light'} appearance without saving`, async () => {
    const app = await fixture({}, dark);
    const requests = app.requests.length;
    assertTheme(app, 'system', dark ? 'dark' : 'light');
    app.ui.system_theme_changed(!dark);
    assertTheme(app, 'system', dark ? 'light' : 'dark');
    app.ui.system_theme_changed(dark);
    assertTheme(app, 'system', dark ? 'dark' : 'light');
    assert.equal(app.requests.length, requests);
  });

  test(`sidebar switches resolved ${dark ? 'dark' : 'light'} system mode to the opposite manual mode`, async () => {
    const app = await fixture({}, dark);
    app.ui.dispatch('toggle-theme', '');
    const request = app.take('set_theme');
    const theme = dark ? 'light' : 'dark';
    assert.deepEqual(request.payload, {theme});
    app.reply(request, {...app.settings, theme});
    app.ui.system_theme_changed(!dark);
    app.ui.system_theme_changed(dark);
    assertTheme(app, theme, theme);
  });
}

for (const theme of ['light', 'dark']) {
  test(`manual ${theme} mode ignores system changes`, async () => {
    const app = await fixture({theme});
    app.ui.system_theme_changed(true);
    assertTheme(app, theme, theme);
    app.ui.system_theme_changed(false);
    assertTheme(app, theme, theme);
  });
}

test('system selection waits for persistence, allows retry and uses the latest appearance', async () => {
  const app = await fixture({theme: 'light'});
  app.ui.dispatch('set-theme', 'invalid');
  app.ui.dispatch('set-theme', 'system');
  app.ui.dispatch('set-theme', 'system');
  assert.equal(app.requests.filter(request => request.command === 'set_theme').length, 1);
  assertTheme(app, 'light', 'light');
  app.reply(app.take('set_theme'), {key: 'error.config_write', args: {detail: 'fixture failure'}}, false);
  assertTheme(app, 'light', 'light');
  assert.match(app.html, /fixture failure/);
  app.ui.dispatch('set-theme', 'system');
  app.ui.system_theme_changed(true);
  app.reply(app.take('set_theme'), {...app.settings, theme: 'system'});
  assertTheme(app, 'system', 'dark');
});

test('settings requested during a theme save cannot undo system mode', async () => {
  const app = await fixture({theme: 'light'}, true);
  app.ui.dispatch('set-theme', 'system');
  app.ui.dispatch('font-size', '18');
  const display = app.take('set_display');
  app.reply(app.take('set_theme'), {...app.settings, theme: 'system'});
  app.reply(display, {...app.settings, font_size: 18});
  assertTheme(app, 'system', 'dark');
  assert.match(app.html, /data-font-size="18"/);
});

for (const [language, label] of [['zh-CN', '跟随系统'], ['ja-JP', 'システムに合わせる'], ['en-US', 'System']]) {
  test(`system option is localized in ${language}`, async () => {
    const app = await fixture({language});
    assert.match(app.html, new RegExp(`data-value="system"[^>]*>${label}</button>`));
  });
}
