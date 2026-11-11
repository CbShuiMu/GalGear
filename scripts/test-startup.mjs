// Run after scripts/build-frontend.ps1; uses the real WASM UI with isolated responses.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';

const bytes = readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm', import.meta.url));
let instance = 0;
async function fixture(settings) {
  const requests = []; let html = '';
  globalThis.GalGearBridge = {
    request(id, command, payload) { requests.push({id, command, payload: JSON.parse(payload)}); },
    render(value) { html = value; }, reset_scroll() {}, schedule_toast_expiry() {},
    schedule_download_notice_expiry() {}, local_date() { return ''; }, local_date_time() { return ''; },
  };
  const ui = await import(`../dist/pkg/galgear_ui.js?startup=${++instance}`);
  ui.initSync({module: bytes});
  const reply = (command, value, success = true) => {
    const index = requests.findIndex(r => r.command === command);
    assert.notEqual(index, -1, command);
    const request = requests.splice(index, 1)[0];
    ui.resolve(request.id, success, JSON.stringify(value));
    return request.payload;
  };
  reply('bootstrap', {settings, catalog: {games: [], warnings: []}, config_path: 'fixture', version: '0.4.2'});
  return {ui, requests, reply, get html() { return html; }};
}

for (const [language, source, erogamescapeLabel] of [
  ['zh-CN', 'nextmoe', '批评空间'],
  ['en-US', 'vndb', 'ErogameScape'],
  ['ja-JP', 'erogamescape', '批評空間'],
]) {
  test(`first launch saves ${language} before starting background tasks`, async () => {
    const settings = {library_folders: [], card_size: 200, sort_by: 'title', initial_setup_pending: true};
    const f = await fixture(settings);
    assert.match(f.html, /startup-dialog/);
    assert.match(f.html, /简体中文/); assert.match(f.html, /English/); assert.match(f.html, /日本語/);
    assert.equal(f.requests.length, 0);
    f.ui.dispatch('check-update', ''); f.ui.dispatch('editor-close', '');
    assert.equal(f.requests.length, 0);
    f.ui.dispatch('startup-language', language); f.ui.dispatch('startup-language', language);
    assert.equal(f.requests.length, 1);
    assert.deepEqual(f.reply('set_language', {key:'error.config_write', args:{detail:'fixture failure'}}, false), {language});
    assert.match(f.html, /startup-dialog/);
    assert.match(f.html, /role="alert"[^>]*>[^<]*fixture failure/);
    assert.equal(f.requests.length, 0);
    f.ui.dispatch('startup-language', language);
    f.reply('set_language', {...settings, language, metadata_provider: source, initial_setup_pending: false});
    assert.doesNotMatch(f.html, /startup-dialog/);
    for (const command of ['check_update', 'check_skill_version', 'scan_library']) {
      assert.equal(f.requests.filter(r => r.command === command).length, 1);
    }
    f.reply('get_data_sources', []);
    f.ui.dispatch('page', 'settings'); f.ui.dispatch('settings-section', 'account');
    const options = [...f.html.matchAll(/data-action="source-default" data-value="([^"]+)"[^>]*><img[^>]*\/>([^<]+)<\/button>/g)]
      .map(([, id, label]) => [id, label]);
    assert.deepEqual(options, [
      ['nextmoe', '鲲Gal'], ['2dfan', '2DFan'], ['bangumi', 'Bangumi'],
      ['vndb', 'VNDB'], ['erogamescape', erogamescapeLabel], ['ymgal', '月幕Gal'],
    ]);
    assert.match(f.html, /source-erogamescape/);
    assert(f.html.indexOf('data-render-key="source-bangumi"') < f.html.indexOf('data-render-key="source-vndb"'));
    assert(f.html.indexOf('data-render-key="source-vndb"') < f.html.indexOf('data-render-key="source-erogamescape"'));
    assert.match(f.html, /assets\/erogamescape\.png/);
    assert.doesNotMatch(f.html, /source-erogamescape-token/);
    if (language === 'zh-CN') assert.doesNotMatch(f.html, /ErogameScape|月幕\s*Galgame/);
  });
}

test('existing settings skip language onboarding', async () => {
  const f = await fixture({library_folders: [], card_size: 200, sort_by: 'title', language:'en-US', metadata_provider:'bangumi'});
  assert.doesNotMatch(f.html, /startup-dialog/);
  assert.equal(f.requests.filter(r => r.command === 'check_update').length, 1);
  assert(!f.requests.some(r => r.command === 'set_language'));
});
