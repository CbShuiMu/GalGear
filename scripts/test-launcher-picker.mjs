// Build the frontend first; exercise configuration changes through the real WASM UI.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';

const module = readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm', import.meta.url));
let instance = 0;

async function fixture() {
  const game = {id:'fixture', title:'Fixture', folder:'F:/Fixture', executable:'F:/Fixture/game.exe', engine:'unknown', modified:0};
  const settings = {library_folders:['F:/Fixture'], card_size:200, sort_by:'title', game_edits:{fixture:{setup_confirmed:true, arguments:['--test'], launcher:'F:/Tools/wrapper.exe'}}};
  const catalog = {games:[game], warnings:[]};
  const requests = [];
  const pending = [];
  let html = '';
  const ui = await import(`../dist/pkg/galgear_ui.js?launcher-test=${++instance}`);
  const tick = () => new Promise(resolve => setImmediate(resolve));
  globalThis.GalGearBridge = {
    render(value) { html = value; }, reset_scroll() {}, local_date() { return ''; }, local_date_time() { return ''; },
    schedule_toast_expiry() {}, schedule_download_notice_expiry() {},
    request(id, command, payload) {
      const request = {id, command, payload:JSON.parse(payload)};
      requests.push(request);
      if (['list_game_executables', 'confirm_game_executable'].includes(command)) {
        pending.push(request);
        return;
      }
      const replies = {
        bootstrap:{settings,catalog,config_path:'fixture',version:'0.4.1'},
        get_preferences:settings, scan_library:catalog, get_ai_runtime:{available:false},
        get_data_sources:[], get_fan_status_cache:{status:'idle'}, load_game_icons:{},
        detect_saves:{id:game.id,report:{locations:[],limited:false}},
        save_history:{snapshots:[],current_snapshot:null,notice:null}, latest_screenshot:null,
        list_game_patches:{folder:'F:/Fixture/patch',patches:[]},
      };
      queueMicrotask(() => ui.resolve(id, true, JSON.stringify(replies[command] ?? null)));
    },
  };
  ui.initSync({module});
  await tick();
  ui.dispatch('game-detail', game.id);
  await tick();
  ui.dispatch('game-settings', '');
  return {
    ui, game, settings, requests, pending,
    get html() { return html; },
    reply(command, value, success = true) {
      assert.equal(pending.length, 1);
      const request = pending.shift();
      assert.equal(request.command, command);
      ui.resolve(request.id, success, JSON.stringify(value));
      return request.payload;
    },
  };
}

const candidates = ['game.exe', 'patch.exe'].map(name => ({path:`F:/Fixture/${name}`, name, icon:null}));

test('configuration path reuses the picker, preselects the current EXE and saves without launching', async () => {
  const app = await fixture();
  assert.match(app.html, /<button[^>]*data-action="launcher-edit"[^>]*title="F:\/Fixture\/game.exe"[^>]*>game.exe<\/button>/);
  app.ui.dispatch('launcher-edit', app.game.id);
  app.ui.dispatch('launcher-edit', app.game.id);
  assert.match(app.html, /正在查找启动程序/);
  assert.deepEqual(app.reply('list_game_executables', candidates), {id:app.game.id});
  assert.match(app.html, /data-value="F:\/Fixture\/game.exe" aria-pressed="true"/);
  app.ui.dispatch('launcher-select', candidates[1].path);
  assert.match(app.html, /class="configuration-executable"[^>]*>game.exe<\/button>/);
  app.ui.dispatch('launcher-save', '');
  assert.match(app.html, /class="configuration-executable"[^>]*>game.exe<\/button>/);
  const settings = {...app.settings, game_edits:{fixture:{...app.settings.game_edits.fixture, executable:candidates[1].path}}};
  assert.deepEqual(app.reply('confirm_game_executable', {game:{...app.game, executable:candidates[1].path}, settings, runtime:'stopped'}), {id:app.game.id, executable:candidates[1].path});
  assert.doesNotMatch(app.html, /class="launcher-list"/);
  assert.match(app.html, /class="configuration-executable"[^>]*>patch.exe<\/button>/);
  assert.match(app.html, /--test/);
  assert.match(app.html, /F:\/Tools\/wrapper.exe/);
  assert.equal(app.requests.some(request => request.command === 'launch_game'), false);
});

test('one candidate still requires confirmation and cancelling keeps the current path', async () => {
  const app = await fixture();
  app.ui.dispatch('launcher-edit', app.game.id);
  app.reply('list_game_executables', [candidates[1]]);
  assert.match(app.html, /class="launcher-list"/);
  assert.equal(app.pending.length, 0);
  app.ui.dispatch('launcher-close', '');
  assert.doesNotMatch(app.html, /class="launcher-list"/);
  assert.match(app.html, /class="configuration-executable"[^>]*>game.exe<\/button>/);
  assert.equal(app.requests.some(request => request.command === 'confirm_game_executable'), false);
});

test('save failures preserve the old path and allow retrying the selected EXE', async () => {
  const app = await fixture();
  app.ui.dispatch('launcher-edit', app.game.id);
  app.reply('list_game_executables', candidates);
  app.ui.dispatch('launcher-select', candidates[1].path);
  app.ui.dispatch('launcher-save', '');
  app.reply('confirm_game_executable', {key:'error.operation', args:{}}, false);
  assert.match(app.html, /role="alert"/);
  assert.match(app.html, /data-value="F:\/Fixture\/patch.exe" aria-pressed="true"/);
  assert.match(app.html, /class="configuration-executable"[^>]*>game.exe<\/button>/);
  app.ui.dispatch('launcher-save', '');
  assert.deepEqual(app.pending[0].payload, {id:app.game.id, executable:candidates[1].path});
  assert.equal(app.requests.some(request => request.command === 'launch_game'), false);
});

test('empty, malformed or failed candidate lists can be closed and cannot save the old path', async () => {
  for (const [success, payload] of [[true, []], [true, null], [false, {key:'error.operation',args:{}}]]) {
    const app = await fixture();
    app.ui.dispatch('launcher-edit', app.game.id);
    app.reply('list_game_executables', payload, success);
    if (!success || payload === null) assert.match(app.html, /role="alert"/);
    assert.match(app.html, /data-action="launcher-save"[^>]*disabled/);
    app.ui.dispatch('launcher-save', '');
    assert.equal(app.pending.length, 0);
    app.ui.dispatch('launcher-close', '');
    assert.doesNotMatch(app.html, /class="launcher-list"/);
    assert.match(app.html, /class="configuration-executable"[^>]*>game.exe<\/button>/);
  }
});
