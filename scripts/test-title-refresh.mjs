// Build the frontend first; verify title-sync notices through the actual WASM boundary.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';

const module = readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm', import.meta.url));
let instance = 0;
const tick = () => new Promise(resolve => setImmediate(resolve));

async function fixture() {
  const game = {id:'fixture', title:'Old title', folder:'F:/Fixture', executable:'F:/Fixture/game.exe', engine:'unknown', modified:0};
  const settings = {card_size:200,sort_by:'title',library_folders:['F:/Fixture'], game_edits:{fixture:{setup_confirmed:true}}};
  const catalog = {games:[game], warnings:[]};
  const pending = [], timers = [], commands = [];
  let html = '';
  const ui = await import(`../dist/pkg/galgear_ui.js?title-refresh=${++instance}`);
  globalThis.GalGearBridge = {
    render(value) { html = value; }, reset_scroll() {}, local_date() { return ''; }, local_date_time() { return ''; },
    schedule_toast_expiry(token) { timers.push(token); }, schedule_download_notice_expiry() {},
    request(id, command, payload) {
      commands.push(command);
      const args = JSON.parse(payload);
      if (command === 'refresh_title_metadata') {
        assert.equal(args.requestId, id);
        pending.push({id, args});
        return;
      }
      if (command === 'save_game_details') game.title = args.draft.title;
      const replies = {
        bootstrap:{settings,catalog,config_path:'fixture',version:'0.4.4'},
        get_preferences:settings, game_runtime:"stopped", scan_library:catalog, get_ai_runtime:{available:false},
        get_data_sources:[], get_fan_status_cache:{status:'idle'}, load_game_icons:{},
        save_game_details:game, detect_saves:{id:game.id,report:{locations:[],limited:false}},
        save_history:{snapshots:[],current_snapshot:null,notice:null}, latest_screenshot:null,
        list_game_patches:{folder:'F:/Fixture/patch',patches:[]},
      };
      const response = JSON.stringify(replies[command] ?? null);
      queueMicrotask(() => ui.resolve(id, true, response));
    },
  };
  ui.initSync({module});
  await tick();
  ui.dispatch('game-detail', game.id);
  await tick();
  return {
    ui, pending, timers,
    get html() { return html; },
    get notice() { return html.includes('id="title-refresh-toast"'); },
    async rename(title) {
      ui.dispatch('detail-edit', '');
      ui.input_changed('draft-title', title);
      ui.dispatch('detail-save', '');
      await tick();
      assert.ok(pending.length, JSON.stringify({commands, toasts: html.match(/<div class="toast[\s\S]*?<\/div>/g), editing: html.includes('draft-title')}));
      return pending.at(-1).id;
    },
    async finish(id, success = true) {
      ui.resolve(id, success, JSON.stringify(success ? {...game,tags:['New tag'],cover:'http://galgear-cover.localhost/new-cover'} : {key:'error.network',args:{}}));
      await tick();
    },
  };
}

test('notice starts only when a new unique work is found, persists past toast timers, and closes on completion', async () => {
  const app = await fixture();
  const id = await app.rename('New title');
  assert.equal(app.notice, false);
  const timerCount = app.timers.length;
  app.ui.title_refresh_started(id);
  assert.equal(app.notice, true);
  assert.match(app.html, /正在同步封面，标签与资料/);
  assert.equal(app.timers.length, timerCount, 'sync notice must not install an expiry timer');
  for (const token of app.timers) app.ui.expire_toast(token);
  assert.equal(app.notice, true);
  app.ui.title_refresh_started(id);
  assert.equal(app.html.match(/id="title-refresh-toast"/g).length, 1);
  app.ui.dispatch('close-detail', '');
  assert.equal(app.notice, true, 'navigation must not close a running sync notice');
  await app.finish(id);
  assert.equal(app.notice, false);
  app.ui.dispatch('game-detail', 'fixture');
  await tick();
  assert.match(app.html, /http:\/\/galgear-cover.localhost\/new-cover/);
  app.ui.title_refresh_started(id);
  assert.equal(app.notice, false, 'late start event must not reopen a completed notice');
});

test('search with no replacement never shows a syncing notice', async () => {
  const app = await fixture();
  const id = await app.rename('No match');
  app.ui.resolve(id, true, 'null');
  assert.equal(app.notice, false);
});

test('failure closes the progress notice and retains the failure message', async () => {
  const app = await fixture();
  const id = await app.rename('New title');
  app.ui.title_refresh_started(id);
  await app.finish(id, false);
  assert.equal(app.notice, false);
  assert.match(app.html, /未能自动更新/);
});

test('a superseded request cannot close the current sync or reopen its own notice', async () => {
  const app = await fixture();
  const first = await app.rename('New title');
  app.ui.title_refresh_started(first);
  const second = await app.rename('Newest title');
  assert.equal(app.notice, false);
  app.ui.title_refresh_started(first);
  assert.equal(app.notice, false);
  app.ui.title_refresh_started(second);
  assert.equal(app.notice, true);
  await app.finish(first);
  assert.equal(app.notice, true);
  await app.finish(second);
  assert.equal(app.notice, false);
});
