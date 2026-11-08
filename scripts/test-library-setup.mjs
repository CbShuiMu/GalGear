// Run after scripts/build-frontend.ps1; exercises the real Rust/WASM UI boundary.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';

const moduleBytes = readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm', import.meta.url));
let instance = 0;

async function emptyHome() {
  const requests = [];
  let html = '';
  globalThis.GalGearBridge = {
    request(id, command, payload) { requests.push({id, command, payload: JSON.parse(payload)}); },
    render(value) { html = value; },
    schedule_toast_expiry() {},
    schedule_download_notice_expiry() {},
    reset_scroll() {},
    local_date() { return ''; },
    local_date_time() { return ''; },
  };
  const ui = await import(`../dist/pkg/galgear_ui.js?setup-test=${++instance}`);
  ui.initSync({module: moduleBytes});
  const settings = {library_folders: [], card_size: 200, sort_by: 'title', game_edits: {}};
  const empty = {games: [], warnings: []};
  const bootstrap = () => ({settings, catalog: empty, config_path: 'fixture', version: '0.3.0'});
  const reply = (command, value) => {
    const index = requests.findIndex(request => request.command === command);
    assert.notEqual(index, -1, `Expected request: ${command}`);
    const [request] = requests.splice(index, 1);
    ui.resolve(request.id, true, JSON.stringify(value));
    return request.payload;
  };
  const finishScan = games => {
    reply('scan_library', {games, warnings: []});
    reply('get_preferences', settings);
    reply('load_game_icons', {});
    reply('enrich_library', null);
  };
  reply('bootstrap', bootstrap());
  reply('get_ai_runtime', {available: false});
  reply('get_data_sources', []);
  reply('get_fan_status_cache', {status: 'idle'});
  finishScan([]);
  assert.match(html, /class="home-empty"/);
  assert.doesNotMatch(html, /class="setup-overlay"/);
  return {
    ui, settings, reply, finishScan, requests,
    get html() { return html; },
    addFolder() {
      ui.dispatch('choose-folder', '');
      reply('choose_library_folder', 'F:/Fixture');
      settings.library_folders = ['F:/Fixture'];
      assert.deepEqual(reply('add_library_folder', bootstrap()), {path: 'F:/Fixture'});
    },
  };
}

function game(id = 'new') {
  return {id, title: id, folder: `F:/Fixture/${id}`, executable: `F:/Fixture/${id}/game.exe`, engine: 'unknown', modified: 0};
}

test('first library added from empty home opens setup even without progress events', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.settings.game_edits.new = {setup_pending: true};
  app.finishScan([game()]);
  assert.match(app.html, /id="setup-title">确认游戏资料与启动器/);
  assert.match(app.html, /data-setup-game="new"/);
  assert.doesNotMatch(app.html, /id="home-game-new"/);
});

test('readded library displays every restored game awaiting confirmation', async () => {
  const app = await emptyHome();
  const games = Array.from({length: 10}, (_, index) => game(`restored-${index}`));
  for (const game of games) app.settings.game_edits[game.id] = {setup_pending: true, setup_confirmed: false};
  app.addFolder();
  app.ui.scan_progress(JSON.stringify({games, folders_scanned: 10}));
  app.finishScan(games);
  assert.equal((app.html.match(/data-setup-game=/g) || []).length, 10);
  assert.match(app.html, /id="setup-title">确认游戏资料与启动器/);
});

test('adding a folder again reopens games deferred on the empty home', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.finishScan([game()]);
  app.ui.dispatch('setup-defer-all', '');
  assert.match(app.html, /class="home-empty"/);
  assert.doesNotMatch(app.html, /class="setup-overlay"/);
  app.addFolder();
  app.finishScan([game()]);
  assert.match(app.html, /data-setup-game="new"/);
});

test('cancelling the folder chooser keeps deferred games deferred', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.finishScan([game()]);
  app.ui.dispatch('setup-defer-all', '');
  app.ui.dispatch('choose-folder', '');
  app.reply('choose_library_folder', null);
  assert.doesNotMatch(app.html, /class="setup-overlay"/);
});

test('scanning an empty directory does not open an empty setup dialog', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.finishScan([]);
  assert.match(app.html, /class="home-empty"/);
  assert.doesNotMatch(app.html, /class="setup-overlay"/);
});


test('offline game remains accessible with a permanent path warning and disabled configuration', async () => {
  const app = await emptyHome();
  app.settings.game_edits.offline = {setup_confirmed: true};
  app.ui.library_devices_changed();
  app.finishScan([{...game('offline'), available: false}]);
  app.ui.dispatch('page', 'games');
  assert.match(app.html, /class="path-warning"/);
  app.ui.dispatch('game-detail', 'offline');
  assert.match(app.html, /id="detail-title">offline/);
  assert.match(app.html, /data-action="game-settings"[^>]*disabled/);
  assert.match(app.html, /data-action="launch"[^>]*disabled/);
  app.ui.dispatch('game-settings', '');
  assert.doesNotMatch(app.html, /class="editor-dialog game-settings-dialog"/);
  app.ui.dispatch('page', 'games');
  app.ui.dispatch('card-delete', 'offline');
  assert.match(app.html, /永久删除/);
  assert.match(app.html, /重新导入/);
});

test('device changes coalesce and rescan after an in-flight scan', async () => {
  const app = await emptyHome();
  app.ui.library_devices_changed();
  app.ui.library_devices_changed();
  app.ui.library_devices_changed();
  assert.equal(app.requests.filter(r => r.command === 'scan_library').length, 1);
  app.finishScan([]);
  assert.equal(app.requests.filter(r => r.command === 'scan_library').length, 1);
  app.finishScan([]);
  assert.equal(app.requests.filter(r => r.command === 'scan_library').length, 0);
  assert.doesNotMatch(app.html, /扫描完成/);
});

test('disconnecting a drive disables an already open configuration editor', async () => {
  const app = await emptyHome();
  app.settings.game_edits.game = {setup_confirmed: true};
  app.ui.library_devices_changed();
  app.finishScan([{...game('game'), available: true}]);
  app.ui.dispatch('game-detail', 'game');
  app.ui.dispatch('game-settings', '');
  app.ui.dispatch('detail-edit', '');
  assert.match(app.html, /class="configuration-fields"/);
  assert.doesNotMatch(app.html, /class="configuration-fields" disabled/);
  app.ui.library_devices_changed();
  app.finishScan([{...game('game'), available: false}]);
  assert.match(app.html, /class="configuration-fields" disabled/);
  assert.match(app.html, /data-action="detail-save"[^>]*disabled/);
});
