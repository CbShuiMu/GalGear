// Build the frontend first; verify shortcuts against the actual Rust/WASM request boundary.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';

const module = readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm', import.meta.url));
let instance = 0;
async function fixture({locations, aiAvailable = true} = {}) {
  const game = {id:'fixture', title:'Fixture', folder:'F:/Fixture', executable:'F:/Fixture/game.exe', engine:'unknown', modified:0};
  const settings = {library_folders:['F:/Fixture'], card_size:200, sort_by:'title', game_edits:{fixture:{setup_confirmed:true}}};
  const catalog = {games:[game], warnings:[]};
  const pending = [];
  const openRequests = [];
  let html = '';
  const ui = await import(`../dist/pkg/galgear_ui.js?detail-tools=${++instance}`);
  const tick = () => new Promise(resolve => setImmediate(resolve));
  globalThis.GalGearBridge = {
    render(value) { html = value; }, reset_scroll() {}, local_date() { return ''; }, local_date_time() { return ''; },
    schedule_toast_expiry() {}, schedule_download_notice_expiry() {},
    request(id, command, payload) {
      if (command === 'open_save_folder') openRequests.push(JSON.parse(payload));
      if (['set_game_tools', 'set_ai_summary'].includes(command)) {
        pending.push({id, command, payload:JSON.parse(payload)});
        return;
      }
      const replies = {
        bootstrap:{settings,catalog,config_path:'fixture',version:'0.4.1'},
        get_preferences:settings, scan_library:catalog, get_ai_runtime:{available:aiAvailable},
        get_data_sources:[], get_fan_status_cache:{status:'idle'}, load_game_icons:{},
        detect_saves:{id:game.id,report:{locations:locations ?? [{path:'F:/Fixture/save',exists:true,confidence:'confirmed',source:'manual'}],limited:false}},
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
  return {
    ui, settings, pending, openRequests,
    get html() { return html; },
    buttons(action) { return [...html.matchAll(/<button\b[^>]*>/g)].map(match => match[0]).filter(tag => tag.includes(`data-action="${action}"`)); },
    async finish(success = true) {
      assert.equal(pending.length, 1);
      const request = pending.shift();
      if (success) {
        const edit = settings.game_edits.fixture;
        if (request.command === 'set_ai_summary') edit.ai_summary = request.payload.enabled;
        else Object.assign(edit, {launch_mode:request.payload.launchMode, upscale:request.payload.upscale, auto_backup:request.payload.autoBackup});
      }
      ui.resolve(request.id, success, JSON.stringify(success ? settings : {key:'error.desktop',args:{}}));
      await tick();
      return request;
    },
  };
}

test('all four shortcuts save through existing commands and stay synchronized in both directions', async () => {
  const app = await fixture();
  for (const action of ['toggle-locale','toggle-upscale','toggle-auto-backup','toggle-ai-summary']) {
    app.ui.dispatch(action, '');
    assert.match(app.buttons(action)[0], /aria-pressed="false"/);
    const request = await app.finish();
    assert.equal(request.command, action === 'toggle-ai-summary' ? 'set_ai_summary' : 'set_game_tools');
    assert.match(app.buttons(action)[0], /aria-pressed="true"/);
    app.ui.dispatch('game-settings', '');
    assert.match(app.buttons(action)[1], /aria-checked="true"/);
    app.ui.dispatch(action, '');
    await app.finish();
    assert.match(app.buttons(action)[0], /aria-pressed="false"/);
    assert.match(app.buttons(action)[1], /aria-checked="false"/);
    app.ui.dispatch('editor-close', '');
  }
});

test('a rejected save keeps both controls off and preserves other tool preferences', async () => {
  const app = await fixture();
  app.ui.dispatch('toggle-upscale', '');
  await app.finish();
  app.ui.dispatch('toggle-locale', '');
  assert.equal(app.pending[0].payload.upscale, true);
  await app.finish(false);
  app.ui.dispatch('game-settings', '');
  assert.match(app.buttons('toggle-locale')[0], /aria-pressed="false"/);
  assert.match(app.buttons('toggle-locale')[1], /aria-checked="false"/);
  assert.match(app.buttons('toggle-upscale')[0], /aria-pressed="true"/);
});

test('missing save locations and AI tools disable the same options in both views', async () => {
  const app = await fixture({locations:[], aiAvailable:false});
  app.ui.dispatch('game-settings', '');
  for (const action of ['toggle-auto-backup','toggle-ai-summary']) {
    assert.equal(app.buttons(action).length, 2);
    assert.ok(app.buttons(action).every(tag => tag.includes('disabled')));
  }
  assert.match(app.buttons('open-save')[0], /disabled/);
});

test('one save directory button follows manual selection and requests all confirmed folders', async () => {
  const app = await fixture({locations:[
    {path:'F:/Fixture/save',exists:true,confidence:'confirmed',source:'manual'},
    {path:'F:/Fixture/extra',exists:true,confidence:'confirmed',source:'manual'},
    {path:'F:/Fixture/candidate',exists:true,confidence:'candidate',source:'game'},
  ]});
  const card = app.html.split('class="detail-card save-card"')[1].split('</section>')[0];
  const header = card.split('class="save-entry"')[0];
  const [openButton, ...otherOpenButtons] = app.buttons('open-save');
  assert.equal(otherOpenButtons.length, 0);
  assert.ok(header.indexOf('data-action="open-save"') > header.indexOf('data-action="choose-save"'));
  assert.match(openButton, /icon-only/);
  assert.match(openButton, /data-value=""/);
  assert.doesNotMatch(openButton, /\bdisabled\b/);
  assert.match(card, /F:\/Fixture\/save/);
  assert.match(card, /F:\/Fixture\/extra/);
  assert.doesNotMatch(card, /F:\/Fixture\/candidate/);
  assert.match(app.buttons('screenshot-open')[0], /icon-only/);
  assert.doesNotMatch(card.split('class="save-entry"').slice(1).join(''), /data-action="open-save"/);
  app.ui.dispatch('open-save', '');
  assert.deepEqual(app.openRequests, [{id:'fixture',path:''}]);
});
