// Build the frontend first. Exercise the real local-date adapter and Rust/WASM rules.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
import vm from 'node:vm';

const bridge = readFileSync(new URL('../web/bridge.js', import.meta.url), 'utf8');
const calendar = bridge.slice(bridge.indexOf('  local_date_time(seconds)'), bridge.indexOf('  schedule_toast_expiry('));
const moduleBytes = readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm', import.meta.url));
let instance = 0;

test('home and game details share local dates and the library switches duration units at one hour', async () => {
  const previousZone = process.env.TZ;
  try {
    for (const scenario of [
      {zone:'Asia/Shanghai', now:'2026-11-08T00:15:00+08:00', played:'2026-11-07T23:55:00+08:00', date:'2026-11-07'},
      {zone:'America/New_York', now:'2026-11-02T00:15:00-05:00', played:'2026-11-01T00:05:00-04:00', date:'2026-11-01'},
    ]) {
      process.env.TZ = scenario.zone;
      class FixedDate extends Date { static now() { return Date.parse(scenario.now); } }
      const dates = vm.runInNewContext(`({${calendar}})`, {Date:FixedDate});
      const seconds = Date.parse(scenario.played) / 1000;
      assert.equal(dates.local_date(seconds), scenario.date);
      assert.equal(dates.local_date(Number.MAX_VALUE), '');
      const game = {id:'fixture', title:'Fixture', folder:'F:/Fixture', engine:'unknown', modified:0,
        last_played:seconds, playtime_seconds:3599, metadata:{brand:'Key', release_date:'2022-09-30'}};
      const settings = {library_folders:['F:/Fixture'], card_size:200, sort_by:'title',
        game_edits:{fixture:{setup_confirmed:true}}};
      const catalog = {games:[game], warnings:[]};
      let html = '';
      const ui = await import(`../dist/pkg/galgear_ui.js?playtime-test=${++instance}`);
      globalThis.GalGearBridge = {
        ...dates,
        render(value) { html = value; }, reset_scroll() {},
        schedule_toast_expiry() {}, schedule_download_notice_expiry() {},
        request(id, command) {
          const replies = {
            bootstrap:{settings,catalog,config_path:'fixture',version:'0.4.0'},
            get_preferences:settings, scan_library:catalog, get_ai_runtime:{available:false},
            get_data_sources:[], get_fan_status_cache:{status:'idle'}, load_game_icons:{},
            detect_saves:{id:game.id,report:{locations:[],limited:false}},
            save_history:{snapshots:[],current_snapshot:null,notice:null}, game_runtime:'stopped',
            list_game_patches:{folder:'F:/Fixture/patch',patches:[]},
          };
          queueMicrotask(() => ui.resolve(id, true, JSON.stringify(replies[command] ?? null)));
        },
      };
      ui.initSync({module:moduleBytes});
      await new Promise(resolve => setImmediate(resolve));
      assert.match(html, /<span class="home-playtime">最近游玩 昨天<\/span>/, scenario.zone);
      ui.dispatch('page', 'games');
      assert.match(html, /class="library-playtime"><strong>59分<\/strong>/);
      ui.dispatch('game-detail', game.id);
      await new Promise(resolve => setImmediate(resolve));
      const facts = html.match(/<dl class="hero-facts">([\s\S]*?)<\/dl>/)?.[1];
      assert.ok(facts, scenario.zone);
      assert.deepEqual([...facts.matchAll(/<dt>(.*?)<\/dt>/g)].map(match => match[1]),
        ['品牌','发售日期','游玩时间','上次游玩']);
      assert.match(facts, /<dt>游玩时间<\/dt><dd>59 分钟<\/dd>/);
      assert.match(facts, /<dt>上次游玩<\/dt><dd>昨天<\/dd>/, scenario.zone);
      for (const [playtime_seconds, hours] of [[3600, '1'], [3960, '1.1']]) {
        ui.playtime_updated(JSON.stringify({[game.id]:{playtime_seconds, last_played:seconds}}));
        assert.ok(html.includes(`class="library-playtime"><strong>${hours}h</strong>`));
        assert.ok(html.includes(`<dt>游玩时间</dt><dd>${hours} 小时</dd>`));
      }
    }
  } finally {
    if (previousZone === undefined) delete process.env.TZ;
    else process.env.TZ = previousZone;
  }
});
