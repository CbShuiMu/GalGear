// Build the frontend first; exercise request/event ordering through the actual WASM UI.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';

const module = readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm', import.meta.url));
const tick = () => new Promise(resolve => setImmediate(resolve));
let instance = 0;

async function fixture() {
  const game = {id:'fixture',title:'A <script> & very long story title',folder:'F:/Fixture',engine:'unknown',modified:0};
  const settings = {card_size:200,sort_by:'title',library_folders:['F:/Fixture'],game_edits:{fixture:{setup_confirmed:true,ai_summary:false}}};
  const catalog = {games:[game],warnings:[]}, pending = [], timers = [];
  let html = '';
  const ui = await import(`../dist/pkg/galgear_ui.js?ai-progress=${++instance}`);
  globalThis.GalGearBridge = {
    render(value) { html = value; }, reset_scroll() {}, local_date() { return ''; }, local_date_time() { return ''; },
    schedule_toast_expiry(token) { timers.push(token); }, schedule_download_notice_expiry() {},
    request(id, command, payload) {
      const args = JSON.parse(payload);
      if (['set_ai_summary','summarize_game','prepare_ai_route_map','open_ai_route_map','extract_game_media'].includes(command)) {
        if (['set_ai_summary','summarize_game','extract_game_media'].includes(command)) assert.equal(args.requestId,id);
        pending.push({id,command,args});
        return;
      }
      const replies = {
        bootstrap:{settings,catalog,config_path:'fixture',version:'0.5.5'},
        get_preferences:settings,game_runtime:'stopped',scan_library:catalog,get_ai_runtime:{available:true},
        get_data_sources:[],get_fan_status_cache:{status:'idle'},load_game_icons:{},
        detect_saves:{id:game.id,report:{locations:[],limited:false}},
        save_history:{snapshots:[],current_snapshot:null,notice:null},latest_screenshot:null,
        list_game_patches:{folder:'F:/Fixture/patch',patches:[]},
      };
      queueMicrotask(() => ui.resolve(id,true,JSON.stringify(replies[command] ?? null)));
    },
  };
  ui.initSync({module});
  await tick();
  ui.dispatch('game-detail',game.id);
  await tick();
  return {
    ui, pending, timers, settings,
    get html() { return html; },
    get notice() { return html.includes('class="ai-analysis-notices"'); },
    start() { ui.dispatch('toggle-ai-summary',''); return pending.at(-1).id; },
    progress(id,sequence,stages,gameId=game.id,currentStage=null) {
      ui.ai_summary_progress(JSON.stringify({request_id:id,game_id:gameId,progress:{sequence,stages,current_stage:currentStage}}));
    },
    complete(id,success,payload) { ui.resolve(id,success,JSON.stringify(payload)); },
    async ready() {
      const id = this.start();
      settings.game_edits.fixture.ai_summary = true;
      this.complete(id,true,settings);
      const route = pending.at(-1);
      assert.equal(route.command,'prepare_ai_route_map');
      this.complete(route.id,true,null);
      await tick();
    },
  };
}

test('Extract resources follows Edit with or without AI preparation', async () => {
  const app = await fixture();
  assert.match(app.html, /class="icon-button media-tools-toggle" aria-label="提取资源" title="提取资源"/);
  assert.ok(app.html.indexOf('data-action="detail-edit"') < app.html.indexOf('media-tools-anchor'));
  assert.doesNotMatch(app.html, /data-action="summary-run"|data-action="route-map-open"/);
  await app.ready();
  for (const action of ['summary-run','route-map-open','extract-cg','extract-audio','extract-dialogue']) {
    const tag = [...app.html.matchAll(/<button\b[^>]*>/g)].map(match => match[0]).find(tag => tag.includes(`data-action="${action}"`));
    assert.match(tag, /icon-only/);
    assert.match(tag, /aria-label="[^"]+"/);
    assert.match(tag, /title="[^"]+"/);
  }
  assert.ok(app.html.indexOf('data-action="summary-run"') < app.html.indexOf('data-action="route-map-open"'));
  assert.ok(app.html.indexOf('data-action="route-map-open"') < app.html.indexOf('data-action="detail-edit"'));
  assert.ok(app.html.indexOf('data-action="detail-edit"') < app.html.indexOf('media-tools-anchor'));
});

test('all media actions start without preparation and report missing AI configuration', async () => {
  const app = await fixture();
  for (const kind of ['cg','audio','dialogue']) {
    app.ui.dispatch(`extract-${kind}`,'');
    const request = app.pending.at(-1);
    assert.equal(request.command,'extract_game_media');
    assert.equal(request.args.kind,kind);
    assert.equal(app.settings.game_edits.fixture.ai_summary,false);
    app.complete(request.id,false,{key:'summary.ai_missing',args:{}});
    assert.equal(app.notice,false);
    assert.match(app.html,/请先在设置→AI中保存Token并选择可用的对话模型/);
  }
  assert.ok(app.pending.every(request => request.command === 'extract_game_media'));
});

test('media extraction serializes per game, survives navigation and validates the final response', async () => {
  const app = await fixture();
  await app.ready();
  for (const [kind, folder, label] of [['cg','CGs','CG'],['audio','audios','音频'],['dialogue','dialogues','台词']]) {
    app.ui.dispatch(`extract-${kind}`,'');
    const request = app.pending.at(-1), count = app.pending.length;
    assert.equal(request.command,'extract_game_media');
    assert.equal(request.args.kind,kind);
    app.ui.dispatch('extract-cg','');
    app.ui.dispatch('summary-run','');
    assert.equal(app.pending.length,count);
    app.progress(request.id,1,[{stage:'extracting',status:'running'}],'fixture','extracting');
    assert.ok(app.html.includes(`4/5 · 正在后台批量提取 ${label}`) || app.html.includes(`4/5 · 正在后台批量提取${label}`));
    app.ui.ai_summary_progress(JSON.stringify({request_id:request.id,game_id:'fixture',progress:{sequence:2,current_stage:'extracting',stages:[{stage:'extracting',status:'running'}],batch:{completed:1,total:3,files:42}}}));
    assert.match(app.html,/批次 1\/3，已提取 42 个文件/);
    const current = app.html;
    app.progress(request.id,3,[{stage:'save',status:'running'}],'fixture','save');
    assert.equal(app.html,current);
    app.ui.dispatch('close-detail','');
    assert.ok(app.notice);
    app.complete(request.id,true,{game_id:'fixture',kind,output:`F:/Fixture/${folder}/media-1`,files:2,warnings:[]});
    assert.equal(app.notice,false);
    assert.match(app.html,/已提取 2 个文件/);
    app.ui.dispatch('game-detail','fixture');
    await tick();
  }
});

test('media failure, malformed success and partial results never show full success', async () => {
  const app = await fixture();
  await app.ready();
  for (const [success,payload,expected] of [
    [false,{key:'media.io',args:{detail:'fixture disk full'}},/fixture disk full/],
    [true,{game_id:'fixture',kind:'cg',output:'F:/Fixture/CGs',files:0,warnings:[]},/媒体提取失败/],
    [true,{game_id:'other',kind:'cg',output:'F:/Fixture/CGs',files:1,warnings:[]},/媒体提取失败/],
    [true,{game_id:'fixture',kind:'cg',output:'F:/Fixture/CGs',files:1,warnings:['fixture unsupported']},/存在未完成项.*fixture unsupported/],
  ]) {
    app.ui.dispatch('extract-cg','');
    app.complete(app.pending.at(-1).id,success,payload);
    assert.equal(app.notice,false);
    assert.match(app.html,expected);
    assert.doesNotMatch(app.html,/已提取 \d+ 个文件/);
  }
});

test('progress stays in the bottom-right stack, survives navigation and ignores stale events', async () => {
  const app = await fixture(), id = app.start();
  assert.ok(app.notice);
  assert.match(app.html,/notification-stack[\s\S]*ai-analysis-notices/);
  assert.match(app.html,/A &lt;script&gt; &amp; very long story title/);
  const stages = [{stage:'resources',status:'completed'},{stage:'scripts',status:'running'}];
  app.progress(id,2,stages);
  assert.doesNotMatch(app.html,/识别并提取游戏资源|<ol/);
  assert.match(app.html,/stage-toast-current">3\/7 · 正在读取剧情脚本…<\/span>/);
  const current = app.html;
  app.progress(id,1,[]);
  app.progress(id,3,[], 'another-game');
  app.progress(id,3,[{stage:'<img src=x>',status:'running'}]);
  app.ui.ai_summary_progress('broken');
  assert.equal(app.html,current);
  app.ui.dispatch('close-detail','');
  for (const token of app.timers) app.ui.expire_toast(token);
  assert.ok(app.notice);
  app.complete(id,false,{key:'summary.agent_failed',args:{detail:'offline'}});
  assert.equal(app.notice,false);
  assert.match(app.html,/offline/);
  app.progress(id,10,stages);
  assert.equal(app.notice,false);
});

test('automatic route generation uses its own stage number and cannot inherit an active analysis stage', async () => {
  const app = await fixture(), id = app.start();
  app.progress(id,4,[{stage:'resources',status:'running'},{stage:'route',status:'completed'}]);
  app.settings.game_edits.fixture.ai_summary = true;
  app.complete(id,true,app.settings);
  const route = app.pending.at(-1);
  assert.equal(route.command,'prepare_ai_route_map');
  assert.ok(app.notice);
  assert.equal(app.html.match(/class="ai-analysis-notices"/g).length,1);
  assert.doesNotMatch(app.html,/识别并提取游戏资源/);
  assert.match(app.html,/stage-toast-current">1\/1 · 正在生成AI路线图…<\/span>/);
  app.progress(id,20,[{stage:'recap',status:'running'}]);
  assert.doesNotMatch(app.html,/生成前情提要<\/span>/);
  app.complete(route.id,true,null);
  assert.equal(app.notice,false);
  assert.match(app.html,/AI路线图已生成/);
});

test('only the latest active stage is shown, including a return to an earlier stage', async () => {
  const app = await fixture(), id = app.start();
  app.progress(id,3,[{stage:'resources',status:'running'},{stage:'scripts',status:'running'}], 'fixture', 'resources');
  assert.match(app.html, /stage-toast-current">2\/7 · 正在识别并提取游戏资源…<\/span>/);
  app.progress(id,4,[{stage:'resources',status:'completed'},{stage:'scripts',status:'running'}]);
  assert.match(app.html, /stage-toast-current">3\/7 · 正在读取剧情脚本…<\/span>/);
  app.progress(id,5,[{stage:'resources',status:'running'},{stage:'scripts',status:'completed'}]);
  assert.match(app.html, /stage-toast-current">2\/7 · 正在识别并提取游戏资源…<\/span>/);
  app.progress(id,6,[{stage:'resources',status:'completed'},{stage:'scripts',status:'completed'}]);
  assert.match(app.html, /stage-toast-current">正在确认分析结果…/);
  assert.ok(app.notice, 'completed stages do not finish a pending command');
});

test('malformed setup response and retry cannot retain or resurrect an old progress card', async () => {
  const app = await fixture(), old = app.start();
  app.complete(old,true,{});
  assert.equal(app.notice,false);
  const next = app.start();
  assert.ok(next > old);
  app.progress(old,99,[{stage:'route',status:'completed'}]);
  assert.doesNotMatch(app.html,/分析分支与路线/);
  app.progress(next,1,[{stage:'save',status:'running'}]);
  assert.match(app.html,/定位存档对应剧情/);
});

test('closing recap leaves work visible and a later failure is still reported', async () => {
  const app = await fixture();
  await app.ready();
  app.ui.dispatch('summary-run','');
  const request = app.pending.at(-1);
  assert.equal(request.command,'summarize_game');
  app.ui.dispatch('summary-close','');
  assert.ok(app.notice);
  app.complete(request.id,false,{key:'summary.agent_failed',args:{detail:'recap failed'}});
  assert.equal(app.notice,false);
  assert.match(app.html,/recap failed/);
});

test('an invalid final recap is reported as failure even if every stage was completed', async () => {
  const app = await fixture();
  await app.ready();
  app.ui.dispatch('summary-run','');
  const {id} = app.pending.at(-1);
  app.progress(id,10,[{stage:'recap',status:'completed'},{stage:'publishing',status:'completed'}]);
  app.complete(id,true,{game_id:'wrong-game'});
  assert.equal(app.notice,false);
  assert.match(app.html,/class="toast save-ai-toast stage-toast error/);
  assert.doesNotMatch(app.html,/AI 分析已完成/);
});
