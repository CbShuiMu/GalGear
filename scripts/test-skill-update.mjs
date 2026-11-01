import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';

test('Skill stays local at startup and only the card starts synchronization', async () => {
  const requests=[]; let html='';
  globalThis.GalGearBridge={request(id,command,payload){requests.push({id,command,payload:JSON.parse(payload)});},render(value){html=value;},reset_scroll(){},schedule_toast_expiry(){},schedule_download_notice_expiry(){},local_date(){return '';},local_date_time(){return '';}};
  const ui=await import('../dist/pkg/galgear_ui.js');
  ui.initSync({module:readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm',import.meta.url))});
  const take=command=>{const index=requests.findIndex(r=>r.command===command);assert.notEqual(index,-1,command);return requests.splice(index,1)[0];};
  const reply=(command,value)=>{const request=take(command);ui.resolve(request.id,true,JSON.stringify(value));};
  const settings={library_folders:[],card_size:200,sort_by:'title'};
  reply('bootstrap',{settings,catalog:{games:[],warnings:[]},config_path:'fixture',version:'0.3.2'});
  reply('get_ai_runtime',{available:true,skill:{installed:true,commit:'a'.repeat(40)}});
  reply('get_data_sources',[]);reply('get_fan_status_cache',{status:'idle'});
  reply('scan_library',{games:[],warnings:[]});reply('get_preferences',settings);reply('load_game_icons',{});reply('enrich_library',null);
  ui.dispatch('page','settings');ui.dispatch('settings-section','about');
  assert.match(html,/<h2>Skill<\/h2>/);assert.match(html,/aaaaaaaa/);
  assert.match(html,/https:\/\/github.com\/CbShuiMu\/Galgear-GalgameReverse-Skill/);
  assert(!requests.some(r=>r.command==='check_skill_update'));
  ui.dispatch('check-skill-update','');ui.dispatch('check-skill-update','');
  assert.equal(requests.filter(r=>r.command==='check_skill_update').length,1);
  assert.match(html,/正在检查并更新 Skill/);
  reply('check_skill_update',{available:true,changed:true,commit:'b'.repeat(40)});
  assert.match(html,/bbbbbbbb/);assert.match(html,/Skill 已更新/);
  ui.dispatch('check-skill-update','');
  const failed=take('check_skill_update');ui.resolve(failed.id,false,JSON.stringify({key:'skills.update_failed',args:{detail:'offline fixture'}}));
  assert.match(html,/bbbbbbbb/);assert.match(html,/offline fixture/);
});
