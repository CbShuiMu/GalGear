// Build the frontend first: exercises actual WASM requests, events and rendered states.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';

test('screenshot settings, preview events and stale responses cross the WASM bridge', async () => {
  const requests = [];
  let html = '';
  globalThis.GalGearBridge = {
    request(id, command, payload) { requests.push({id, command, payload: JSON.parse(payload)}); },
    render(value) { html = value; }, reset_scroll() {}, schedule_toast_expiry() {},
    schedule_download_notice_expiry() {}, local_date() { return ''; }, local_date_time() { return ''; },
  };
  const ui = await import('../dist/pkg/galgear_ui.js');
  ui.initSync({module: readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm', import.meta.url))});
  const settings = {library_folders: [], card_size: 200, sort_by: 'title', game_edits: {one:{setup_confirmed:true},two:{setup_confirmed:true}}};
  const games = ['one', 'two'].map(id => ({id, title: id, folder: `F:/Fixture/${id}`, executable: `F:/Fixture/${id}/game.exe`, engine: 'unknown', modified: 0}));
  function take(command) {
    const index = requests.findIndex(r => r.command === command);
    assert.notEqual(index, -1, `Missing ${command}`);
    return requests.splice(index, 1)[0];
  }
  function reply(command, value) { const request = take(command); ui.resolve(request.id, true, JSON.stringify(value)); return request; }
  reply('bootstrap', {settings, catalog: {games, warnings: []}, config_path: 'fixture', version: '0.3.2'});
  reply('get_ai_runtime', {available:false}); reply('get_data_sources', []);
  reply('get_fan_status_cache', {status:'idle'});
  reply('scan_library', {games, warnings: []}); reply('get_preferences', settings);
  reply('load_game_icons', {}); reply('enrich_library', null);
  ui.dispatch('page', 'settings'); ui.dispatch('settings-section', 'paths');
  assert.match(html, /id="screenshot-hotkey"[^>]+aria-haspopup="dialog"[^>]*>Shift\+S<\/button>/);
  assert.match(html, /id="font-hotkey"[^>]*>Shift\+T<\/button>/);
  const press = () => ui.dispatch('shortcut-pressed', JSON.stringify({code:'F9',ctrl:false,alt:true,shift:false,meta:false}));
  ui.dispatch('shortcut-open', 'screenshot-hotkey');
  assert.match(html, /role="dialog"[^>]+data-shortcut-field="screenshot-hotkey"/);
  press();
  assert.match(html, /<kbd>Alt<\/kbd><span aria-hidden="true">\+<\/span><kbd>F9<\/kbd>/);
  assert.match(html, /id="screenshot-hotkey"[^>]*>Shift\+S<\/button>/);
  ui.dispatch('shortcut-reset', '');
  assert.doesNotMatch(html, /<kbd>/);
  assert.match(html, /data-action="shortcut-confirm"[^>]+disabled/);
  ui.dispatch('shortcut-confirm', '');
  assert.match(html, /class="editor-dialog shortcut-dialog"/);
  press(); ui.dispatch('shortcut-cancel', '');
  assert.doesNotMatch(html, /class="editor-dialog shortcut-dialog"/);
  assert.match(html, /id="screenshot-hotkey"[^>]*>Shift\+S<\/button>/);
  ui.dispatch('shortcut-open', 'screenshot-hotkey'); press(); ui.dispatch('shortcut-confirm', '');
  assert.doesNotMatch(html, /class="editor-dialog shortcut-dialog"/);
  assert.match(html, /id="screenshot-hotkey"[^>]*>Alt\+F9<\/button>/);
  assert(!requests.some(r => r.command === 'configure_tools'));
  ui.dispatch('configure-tools', '');
  const config = take('configure_tools');
  assert.equal(config.payload.screenshotHotkey, 'Alt+F9');
  assert.equal(config.payload.fontHotkey, 'Shift+T');
  assert.doesNotMatch(html, /data-action="screenshot-(toggle|save)"/);
  assert.match(html, /id="screenshot-hotkey"[^>]*>Alt\+F9<\/button><\/div><label class="edit-field" for="backup-retention"/);
  ui.resolve(config.id, false, JSON.stringify({key:'screenshot.hotkey_error',args:{detail:'fixture conflict'}}));
  assert.match(html, /id="screenshot-hotkey"[^>]*>Alt\+F9<\/button>/);
  ui.dispatch('configure-tools', '');
  const saved = take('configure_tools');
  ui.resolve(saved.id, true, JSON.stringify({...settings,screenshot_enabled:true,screenshot_hotkey:'Alt+F9'}));
  assert.match(html, /id="screenshot-hotkey"[^>]*>Alt\+F9<\/button>/);
  reply('get_preferences', {...settings,screenshot_enabled:true,screenshot_hotkey:'Alt+F9'});
  ui.dispatch('game-detail', 'one');
  const stale = take('latest_screenshot');
  ui.dispatch('game-detail', 'two');
  reply('latest_screenshot', null);
  ui.resolve(stale.id, true, JSON.stringify({name:'wrong.png',image:'data:image/png;base64,AA=='}));
  assert.match(html, /screenshot-empty"><p>暂无截图。\n/);
  assert.doesNotMatch(html, /screenshot-empty"><span/); assert.doesNotMatch(html, /wrong.png/);
  ui.screenshot_updated('one'); assert(!requests.some(r => r.command === 'latest_screenshot'));
  ui.screenshot_updated('two');
  const preview = reply('latest_screenshot', {name:'latest.png',image:'data:image/png;base64,AA=='});
  assert.deepEqual(preview.payload, {id:'two'}); assert.match(html, /latest.png/);
  assert(html.indexOf('data-render-key="local-patches"') < html.indexOf('data-render-key="recent-screenshot"'));
  ui.dispatch('screenshot-open', 'two'); assert.deepEqual(take('open_screenshot_folder').payload, {id:'two'});
});
