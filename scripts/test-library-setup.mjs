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
      reply('choose_library_folders', ['F:/Fixture']);
      settings.library_folders = ['F:/Fixture'];
      assert.deepEqual(reply('add_library_folders', {settings, added: 1, unchanged: 0, failed: []}), {paths: ['F:/Fixture']});
    },
  };
}

function game(id = 'new') {
  return {id, title: id, folder: `F:/Fixture/${id}`, executable: `F:/Fixture/${id}/game.exe`, engine: 'unknown', modified: 0};
}

for (const count of [600, 10_000]) {
  test(`${count} folders use one batch request and one scan, with repeated clicks suppressed`, async () => {
    const app = await emptyHome();
    const paths = Array.from({length: count}, (_, i) => `F:/Fixture/Library ${i}`);
    app.ui.dispatch('choose-folder', '');
    app.ui.dispatch('choose-folder', '');
    assert.equal(app.requests.filter(r => r.command === 'choose_library_folders').length, 1);
    app.reply('choose_library_folders', paths);
    app.ui.dispatch('choose-folder', '');
    assert.equal(app.requests.filter(r => r.command === 'choose_library_folders').length, 0);
    assert.equal(app.requests.filter(r => r.command === 'add_library_folders').length, 1);
    app.settings.library_folders = paths;
    assert.deepEqual(app.reply('add_library_folders', {settings: app.settings, added: count, unchanged: 0, failed: []}), {paths});
    assert.equal(app.requests.filter(r => r.command === 'scan_library').length, 1);
    assert.ok(app.html.includes(`已添加 ${count} 个`));
    app.finishScan([]);
  });
}

test('cancelled or empty selection makes no batch request', async () => {
  const app = await emptyHome();
  for (const selection of [null, []]) {
    app.ui.dispatch('choose-folder', '');
    app.reply('choose_library_folders', selection);
    assert.equal(app.requests.filter(r => r.command === 'add_library_folders').length, 0);
  }
});

test('adding during a scan waits and performs a single follow-up scan', async () => {
  const app = await emptyHome();
  app.ui.dispatch('refresh', '');
  app.addFolder();
  assert.equal(app.requests.filter(r => r.command === 'scan_library').length, 1);
  app.reply('scan_library', {games: [], warnings: []});
  assert.equal(app.requests.filter(r => r.command === 'scan_library').length, 1);
  app.finishScan([]);
  assert.equal(app.requests.filter(r => r.command === 'scan_library').length, 0);
});

test('duplicate-only result skips scanning and save failure permits retry', async () => {
  const app = await emptyHome();
  app.ui.dispatch('choose-folder', '');
  app.reply('choose_library_folders', ['F:/Existing']);
  app.reply('add_library_folders', {settings: app.settings, added: 0, unchanged: 1, failed: []});
  assert.equal(app.requests.filter(r => r.command === 'scan_library').length, 0);
  app.ui.dispatch('choose-folder', '');
  app.reply('choose_library_folders', ['F:/New']);
  const index = app.requests.findIndex(r => r.command === 'add_library_folders');
  const [request] = app.requests.splice(index, 1);
  app.ui.resolve(request.id, false, JSON.stringify({key: 'error.config_write', args: {detail: 'fixture failure'}}));
  assert.equal(app.requests.filter(r => r.command === 'scan_library').length, 0);
  assert.match(app.html, /fixture failure/);
  app.ui.dispatch('choose-folder', '');
  assert.equal(app.requests.filter(r => r.command === 'choose_library_folders').length, 1);
});

for (const count of [600, 10_000]) {
  test(`${count} failures render 40 escaped paths per page and preserve successful additions`, async () => {
    const app = await emptyHome();
    const failed = Array.from({length: count}, (_, i) => ({path: `F:/<bad-${i}>`, error: {key: 'error.folder_select'}}));
    app.ui.dispatch('choose-folder', '');
    app.reply('choose_library_folders', ['F:/Good', ...failed.map(f => f.path)]);
    app.settings.library_folders = ['F:/Good'];
    app.reply('add_library_folders', {settings: app.settings, added: 1, unchanged: 0, failed});
    assert.equal((app.html.match(/<li><strong>/g) ?? []).length, 40);
    assert.match(app.html, /&lt;bad-0&gt;/);
    assert.doesNotMatch(app.html, /<bad-0>/);
    app.settings.game_edits.new = {setup_pending: true};
    app.finishScan([game()]);
    assert.doesNotMatch(app.html, /class="setup-overlay"/);
    app.ui.dispatch('folder-failures-page', String(Math.ceil(count / 40) - 1));
    assert.ok(app.html.includes(`&lt;bad-${count - 1}&gt;`));
    assert.doesNotMatch(app.html, /&lt;bad-0&gt;/);
    app.ui.dispatch('folder-failures-page', '999999999');
    assert.ok(app.html.includes(`&lt;bad-${count - 1}&gt;`));
    assert.equal((app.html.match(/<li><strong>/g) ?? []).length, 40);
    app.ui.dispatch('library-close', '');
    assert.match(app.html, /class="setup-overlay"/);
  });
}

test('600 automatic searches keep three slots, advance after failures and retry only unresolved games', async () => {
  const app = await emptyHome();
  const games = Array.from({length: 600}, (_, i) => game(`scrape-${i}`));
  app.addFolder();
  games.forEach(game => { app.settings.game_edits[game.id] = {setup_pending: true}; });
  app.finishScan(games);
  app.ui.dispatch('setup-auto', '');
  assert.match(app.html, /id="setup-scrape-toast" class="toast save-ai-toast"/);
  assert.match(app.html, /正在刮削 0\/600/);
  let failed = 0;
  for (let completed = 1; completed <= 600; completed++) {
    const active = app.requests.filter(request => request.command === 'search_game_metadata');
    assert.equal(active.length, Math.min(3, 601 - completed));
    // 逆序完成模拟不同数据源和网络延迟，确保队列不依赖响应顺序。
    const index = app.requests.findLastIndex(request => request.command === 'search_game_metadata');
    const [request] = app.requests.splice(index, 1);
    const mode = completed % 10;
    if (mode === 0) {
      failed++;
      app.ui.resolve(request.id, false, JSON.stringify({key: 'setup.search_timeout'}));
    } else if (mode === 5 || mode === 7) {
      failed++;
      app.ui.resolve(request.id, true, mode === 5 ? JSON.stringify({query: 'Missing', candidates: [], matched_url: null}) : '{broken');
    } else {
      app.ui.resolve(request.id, true, JSON.stringify({query: request.payload.id, candidates: [{title: request.payload.id, url: `https://vndb.org/v${completed}`}], matched_url: `https://vndb.org/v${completed}`}));
    }
    if (completed < 600) assert.match(app.html, new RegExp(`正在刮削 ${completed}/600`));
  }
  assert.equal(failed, 180);
  assert.doesNotMatch(app.html, /id="setup-scrape-toast"/);
  assert.match(app.html, /刮削完成 600\/600 · 未匹配或失败 180/);
  assert.equal(app.requests.filter(request => request.command === 'search_game_metadata').length, 0);
  app.ui.dispatch('setup-auto', '');
  assert.match(app.html, /正在刮削 0\/180/);
  app.ui.dispatch('setup-auto-stop', '');
  assert.doesNotMatch(app.html, /id="setup-scrape-toast"/);
});

test('stop and restart wait for old requests and ignore their late results', async () => {
  const app = await emptyHome();
  const games = Array.from({length: 6}, (_, i) => game(`restart-${i}`));
  app.addFolder();
  games.forEach(game => { app.settings.game_edits[game.id] = {setup_pending: true}; });
  app.finishScan(games);
  app.ui.dispatch('setup-auto', '');
  const oldRequests = app.requests.filter(request => request.command === 'search_game_metadata');
  assert.equal(oldRequests.length, 3);
  app.ui.dispatch('setup-auto-stop', '');
  app.ui.dispatch('setup-auto', '');
  assert.equal(app.requests.filter(request => request.command === 'search_game_metadata').length, 3);
  for (const old of oldRequests) {
    app.requests.splice(app.requests.indexOf(old), 1);
    app.ui.resolve(old.id, true, '[{"title":"stale","url":"https://vndb.org/v999"}]');
    assert.match(app.html, /正在刮削 0\/6/);
    assert.doesNotMatch(app.html, /stale|v999/);
    assert.equal(app.requests.filter(request => request.command === 'search_game_metadata').length, 3);
  }
  for (let i = 0; i < 6; i++) app.reply('search_game_metadata', {query: 'Fresh', candidates: [{title: 'Fresh', url: 'https://vndb.org/v1'}], matched_url: 'https://vndb.org/v1'});
  assert.match(app.html, /刮削完成 6\/6/);
  assert.doesNotMatch(app.html, /id="setup-scrape-toast"/);
});

test('batch confirmation accepts a single-game edit response', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.settings.game_edits.new = {setup_pending: true};
  app.finishScan([game()]);
  app.ui.dispatch('setup-auto', '');
  app.reply('search_game_metadata', {query: 'Matched', candidates: [{title: 'Matched', url: 'https://vndb.org/v1'}], matched_url: 'https://vndb.org/v1'});
  app.ui.dispatch('setup-save', '');
  app.reply('complete_game_setup', {
    game: game(), edit: {setup_confirmed: true, setup_pending: false, launcher_pending: true},
  });
  assert.doesNotMatch(app.html, /id="setup-title"/);
  assert.match(app.html, /id="home-game-new"/);
});

test('unmatched automatic candidates remain available and require manual confirmation before saving', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.settings.game_edits.new = {setup_pending: true};
  app.finishScan([game()]);
  app.ui.dispatch('setup-auto', '');
  assert.deepEqual(app.reply('search_game_metadata', {
    query: '8  援女交際 ～亜梨紗の情事～',
    candidates: [{title: '性治疗师', url: 'https://developer.nextmoe.dev/explore/work/226643?nsfw=1'}],
    matched_url: null,
  }), {id: 'new', source: 'nextmoe'});
  assert.match(app.html, /未匹配或失败 1/);
  assert.match(app.html, /value="8  援女交際 ～亜梨紗の情事～"/);
  app.ui.dispatch('setup-save', '');
  assert.equal(app.requests.some(request => request.command === 'complete_game_setup'), false);
  app.ui.dispatch('choice-toggle', 'setup-candidate:new');
  assert.match(app.html, /性治疗师/);
  app.ui.dispatch('setup-candidate', JSON.stringify(['new', 'https://developer.nextmoe.dev/explore/work/226643?nsfw=1']));
  app.ui.dispatch('setup-save', '');
  assert.equal(app.requests.some(request => request.command === 'complete_game_setup'), false);
  app.ui.dispatch('setup-confirm-launcher', 'new');
  app.ui.dispatch('setup-save', '');
  assert.equal(app.requests.filter(request => request.command === 'complete_game_setup').length, 1);
});

test('candidate dropdown renders separate escaped title, brand and release rows', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.settings.game_edits.new = {setup_pending: true};
  app.finishScan([game()]);
  app.ui.dispatch('setup-search', 'new');
  app.reply('search_metadata', [{title: 'Game <Title>', brand: 'Brand & Studio', release_date: '2003-02-28', url: 'https://vndb.org/v1'}]);
  app.ui.dispatch('choice-toggle', 'setup-candidate:new');
  assert.match(app.html, /<span class="choice-copy"><span>Game &lt;Title&gt;<\/span><small>Brand &amp; Studio<\/small><small>2003-02-28<\/small><\/span>/);
});

test('empty search replaces the disabled candidate picker with a notice and clears it on retry', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.settings.game_edits.new = {setup_pending: true};
  app.finishScan([game()]);
  app.ui.dispatch('setup-search', 'new');
  app.reply('search_metadata', []);
  assert.match(app.html, /class="setup-notice" role="status"/);
  assert.match(app.html, /没有找到对应词条，请修改名称或切换数据源后重试。/);
  assert.doesNotMatch(app.html, /data-value="setup-candidate:new"/);
  assert.match(app.html, /data-action="setup-confirm-launcher"[^>]*disabled/);
  app.ui.dispatch('setup-search', 'new');
  assert.doesNotMatch(app.html, /class="setup-notice"/);
  app.reply('search_metadata', [{title: 'Recovered', url: 'https://vndb.org/v1'}]);
  assert.match(app.html, /data-value="setup-candidate:new"/);
  app.ui.dispatch('setup-candidate', JSON.stringify(['new', 'https://vndb.org/v1']));
  app.ui.dispatch('setup-confirm-launcher', 'new');
  assert.match(app.html, /data-action="setup-confirm-launcher"[^>]*aria-pressed="true"/);
  assert.doesNotMatch(app.html, /class="setup-notice"/);
});

test('search failure shows an escaped alert without a misleading empty-result notice', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.settings.game_edits.new = {setup_pending: true};
  app.finishScan([game()]);
  app.ui.dispatch('setup-search', 'new');
  const index = app.requests.findIndex(request => request.command === 'search_metadata');
  const [request] = app.requests.splice(index, 1);
  app.ui.resolve(request.id, false, JSON.stringify({key: 'error.read_data', args: {detail: '<offline> & retry'}}));
  assert.match(app.html, /class="setup-notice" role="alert"/);
  assert.match(app.html, /&lt;offline&gt; &amp; retry/);
  assert.doesNotMatch(app.html, /<offline>|没有找到对应词条/);
  app.ui.dispatch('setup-search', 'new');
  assert.doesNotMatch(app.html, /class="setup-notice"/);
  app.reply('search_metadata', []);
  assert.match(app.html, /class="setup-notice" role="status"/);
  assert.doesNotMatch(app.html, /class="setup-notice" role="alert"/);
});

test('custom order is the last sort option', async () => {
  const app = await emptyHome();
  app.ui.dispatch('page', 'games');
  app.ui.dispatch('choice-toggle', 'sort');
  const options = [...app.html.matchAll(/class="choice-option[^>]*data-value="([^\"]+)"/g)].map(match => match[1]);
  assert.equal(options.length, 5);
  assert.equal(options.at(-1), '[&quot;sort&quot;,&quot;custom&quot;]');
});

test('first launch saves the sole executable without a picker and uses its fresh runtime status', async () => {
  const app = await emptyHome();
  const executable = 'F:/Fixture/new/only.exe';
  app.settings.game_edits.new = {setup_confirmed: true, launcher_pending: true};
  app.ui.library_devices_changed();
  app.finishScan([{...game(), executable: null}]);
  app.ui.runtime_updated(JSON.stringify({new: 'unknown'}));

  app.ui.dispatch('launch', 'new');
  assert.doesNotMatch(app.html, /id="launcher-title"/);
  app.reply('list_game_executables', [{path: executable, name: 'only.exe', icon: null}]);
  assert.doesNotMatch(app.html, /id="launcher-title"/);
  assert.equal(app.requests.some(request => request.command === 'launch_game'), false);

  app.settings.game_edits.new.launcher_pending = false;
  assert.deepEqual(app.reply('confirm_game_executable', {
    game: {...game(), executable}, settings: app.settings, runtime: 'stopped',
  }), {id: 'new', executable});
  assert.doesNotMatch(app.html, /launcher-title/);
  assert.deepEqual(app.reply('launch_game', null), {id: 'new', mode: 'normal', confirmedUnknown: false});

  app.ui.dispatch('launch', 'new');
  assert.equal(app.requests.some(request => request.command === 'list_game_executables'), false);
  assert.equal(app.reply('launch_game', null).id, 'new');
});

test('first launch waits for executable confirmation and failed saves remain retryable', async () => {
  const app = await emptyHome();
  app.settings.game_edits.new = {setup_confirmed: true, launcher_pending: true};
  app.ui.library_devices_changed();
  app.finishScan([{...game(), executable: null}]);
  assert.doesNotMatch(app.html, /class="[^\"]*game-launch-button[^\"]*"[^>]*disabled/);
  app.ui.runtime_updated(JSON.stringify({new: 'stopped'}));
  app.ui.dispatch('launch-normal', 'new');
  app.reply('list_game_executables', [
    {path: 'F:/Fixture/new/game.exe', name: 'game.exe', icon: null},
    {path: 'F:/Fixture/new/patch.exe', name: '汉化启动器.exe', icon: null},
  ]);
  assert.match(app.html, /class="launcher-list"/);
  assert.match(app.html, /汉化启动器.exe/);
  app.ui.dispatch('launcher-select', 'F:/Fixture/new/patch.exe');
  app.ui.dispatch('launcher-save', '');
  const index = app.requests.findIndex(request => request.command === 'confirm_game_executable');
  assert.notEqual(index, -1);
  const [failed] = app.requests.splice(index, 1);
  app.ui.resolve(failed.id, false, JSON.stringify({key: 'error.operation', args: {}}));
  assert.match(app.html, /class="launcher-list"/);
  assert.equal(app.requests.some(request => request.command === 'launch_game'), false);
  app.ui.dispatch('launcher-save', '');
  app.settings.game_edits.new.launcher_pending = false;
  app.reply('confirm_game_executable', {game: {...game(), executable: 'F:/Fixture/new/patch.exe'}, settings: app.settings});
  assert.doesNotMatch(app.html, /class="launcher-list"/);
  assert.equal(app.requests.some(request => request.command === 'launch_game'), true);
});

test('large setup renders a bounded page and scan progress accepts counters without game payloads', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.ui.scan_progress(JSON.stringify({folders_scanned: 13, completed: 12, total: 600}));
  assert.match(app.html, /12\/600/);
  assert.match(app.html, /id="library-import-toast" class="toast save-ai-toast"/);
  assert.doesNotMatch(app.html, /<progress|class="import-progress"/);
  app.ui.scan_progress(JSON.stringify({folders_scanned: 14, completed: 13, total: 600}));
  assert.match(app.html, /13\/600/);
  assert.doesNotMatch(app.html, /12\/600/);
  const games = Array.from({length: 600}, (_, index) => game(`bulk-${index}`));
  games.forEach(game => { app.settings.game_edits[game.id] = {setup_pending: true}; });
  app.finishScan(games);
  assert.doesNotMatch(app.html, /id="library-import-toast"/);
  assert.equal((app.html.match(/data-setup-game=/g) || []).length, 40);
  assert.match(app.html, /data-setup-game="bulk-0"/);
  app.ui.dispatch('setup-page', '14');
  assert.match(app.html, /data-setup-game="bulk-599"/);
  assert.equal((app.html.match(/data-setup-game=/g) || []).length, 40);
  assert.doesNotMatch(app.html, /data-action="setup-browse"/);
});

test('first library added from empty home opens setup even without progress events', async () => {
  const app = await emptyHome();
  app.addFolder();
  app.settings.game_edits.new = {setup_pending: true};
  app.finishScan([game()]);
  assert.match(app.html, /id="setup-title">确认游戏资料/);
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
  assert.match(app.html, /id="setup-title">确认游戏资料/);
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
  app.reply('choose_library_folders', null);
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
