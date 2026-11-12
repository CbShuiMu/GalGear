// Build the frontend first; exercise rematching through the actual WASM boundary.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
const module = readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm', import.meta.url));
const tick = () => new Promise(resolve => setImmediate(resolve));
let instance = 0;
async function fixture() {
  const game = {id:'fixture',title:'Old title',folder:'F:/Fixture',executable:'F:/Fixture/game.exe',engine:'unknown',modified:0,metadata:{url:'https://vndb.org/v1',brand:'Old brand'}};
  const settings = {card_size:200,sort_by:'title',library_folders:['F:/Fixture'],game_edits:{fixture:{setup_confirmed:true,metadata_provider:'vndb'}}};
  const catalog = {games:[game],warnings:[]};
  const pending = [];
  let html = '';
  const ui = await import(`../dist/pkg/galgear_ui.js?rematch=${++instance}`);
  globalThis.GalGearBridge = {
    render(value) { html=value; },reset_scroll() {},local_date() {return '';},local_date_time() {return '';},schedule_toast_expiry() {},schedule_download_notice_expiry() {},
    request(id,command,payload) {
      if (['search_metadata','rematch_game_metadata'].includes(command)) {pending.push({id,command,payload:JSON.parse(payload)});return;}
      const replies = {bootstrap:{settings,catalog,config_path:'fixture',version:'0.4.6'},get_preferences:settings,scan_library:catalog,game_runtime:'stopped',get_ai_runtime:{available:false},get_data_sources:[{source:'vndb',configured:true}],get_fan_status_cache:{status:'idle'},load_game_icons:{},detect_saves:{id:'fixture',report:{locations:[],limited:false}},save_history:{snapshots:[],current_snapshot:null,notice:null},latest_screenshot:null,list_game_patches:{folder:'F:/Fixture/patch',patches:[]}};
      queueMicrotask(()=>ui.resolve(id,true,JSON.stringify(replies[command]??null)));
    },
  };
  ui.initSync({module});await tick();
  ui.dispatch('game-detail','fixture');await tick();
  ui.dispatch('detail-edit','');
  return {ui,game,pending,get html(){return html;}};
}

test('detail edit offers a tag input without a website tag restore button',async()=>{
  const app=await fixture();
  assert.match(app.html,/data-input="draft-tags"/);
  assert.doesNotMatch(app.html,/data-action="detail-tags-reset"/);
});

test('rematch button precedes cancel, hides tag source and preserves edits on cancel',async()=>{
  const app=await fixture();
  assert.doesNotMatch(app.html,/data-action="tags-association"/);
  assert.ok(app.html.indexOf('data-action="detail-rematch"')<app.html.indexOf('data-action="detail-cancel"'));
  app.ui.input_changed('draft-title','Unsaved title');
  app.ui.dispatch('detail-rematch','');
  assert.match(app.html,/<h2 id="setup-title">重新匹配<\/h2>/);
  assert.match(app.html,/data-input="setup-query:fixture" value="Unsaved title"/);
  app.ui.dispatch('setup-defer-all','');
  assert.doesNotMatch(app.html,/class="setup-overlay"/);
  assert.match(app.html,/data-input="draft-title" value="Unsaved title"/);
});

test('search selection and confirmation replace only after successful backend save',async()=>{
  const app=await fixture();
  app.ui.dispatch('detail-rematch','');
  app.ui.input_changed('setup-query:fixture','New title');
  app.ui.dispatch('setup-search','fixture');
  let request=app.pending.shift();
  assert.equal(request.command,'search_metadata');
  app.ui.resolve(request.id,true,JSON.stringify([{title:'New title',url:'https://vndb.org/v2'}]));
  app.ui.dispatch('setup-candidate',JSON.stringify(['fixture','https://vndb.org/v2']));
  app.ui.dispatch('setup-save','');
  assert.equal(app.pending.length,0,'must confirm the selected entry');
  app.ui.dispatch('setup-confirm-launcher','fixture');
  app.ui.dispatch('setup-save','');
  request=app.pending.shift();
  assert.equal(request.command,'rematch_game_metadata');
  assert.equal(request.payload.url,'https://vndb.org/v2');
  app.ui.resolve(request.id,false,JSON.stringify({key:'error.network'}));
  assert.match(app.html,/class="setup-overlay"/);
  assert.match(app.html,/data-action="detail-save"/);
  app.ui.dispatch('setup-save','');
  request=app.pending.shift();
  app.ui.resolve(request.id,true,JSON.stringify({game:{...app.game,title:'New title',metadata:{url:'https://vndb.org/v2',brand:'New brand'},tags:['New tag']},edit:{setup_confirmed:true,metadata_provider:'vndb'}}));
  assert.doesNotMatch(app.html,/class="setup-overlay"|data-action="detail-save"|data-action="detail-rematch"/);
  assert.match(app.html,/New title/);
  assert.match(app.html,/New brand/);
  assert.match(app.html,/New tag/);
});
