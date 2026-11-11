import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
const module = readFileSync(new URL('../dist/pkg/galgear_ui_bg.wasm',import.meta.url));
let instance = 0;
async function fixture() {
 const settings={library_folders:['F:/Fixture'],card_size:200,sort_by:'title',font_size:16,game_edits:{fixture:{setup_confirmed:true}}};
 const catalog={games:[{id:'fixture',title:'Fixture',folder:'F:/Fixture',executable:'F:/Fixture/game.exe',engine:'unknown',modified:0}],warnings:[]};
 const pending=[];let html='';
 const ui=await import(`../dist/pkg/galgear_ui.js?library-size=${++instance}`);
 globalThis.GalGearBridge={render(value){html=value;},reset_scroll(){},local_date(){return '';},local_date_time(){return '';},schedule_toast_expiry(){},schedule_download_notice_expiry(){},
 request(id,command,payload){
  if(['set_library_card_size','set_display'].includes(command)){pending.push({id,command,payload:JSON.parse(payload)});return;}
  const replies={bootstrap:{settings,catalog,config_path:'fixture',version:'0.4.4'},get_preferences:settings,scan_library:catalog,get_ai_runtime:{available:false},get_data_sources:[],get_fan_status_cache:{status:'idle'},load_game_icons:{}};
  queueMicrotask(()=>ui.resolve(id,true,JSON.stringify(replies[command]??null)));
 }};
 ui.initSync({module});await new Promise(resolve=>setImmediate(resolve));ui.dispatch('page','games');
 return {ui,pending,settings,get html(){return html;},finish(request,success=true,value=settings){ui.resolve(request.id,success,JSON.stringify(success?value:{key:'error.config_write',args:{}}));}};
}
test('old settings default to standard; steps wait for persistence and stop at boundaries',async()=>{
 const app=await fixture();assert.match(app.html,/data-card-size="normal"/);
 app.ui.dispatch('library-size-step','larger');assert.equal(app.pending.length,0);
 for(const size of ['small','tiny']){
  app.ui.dispatch('library-size-step','smaller');app.ui.dispatch('library-size-step','smaller');
  assert.equal(app.pending.length,1);const req=app.pending.shift();assert.deepEqual(req.payload,{size});
  assert.doesNotMatch(app.html,new RegExp(`data-card-size="${size}"`));
  app.settings.library_card_size=size;app.finish(req);assert.match(app.html,new RegExp(`data-card-size="${size}"`));
 }
 app.ui.dispatch('library-size-step','smaller');assert.equal(app.pending.length,0);
 app.ui.dispatch('page','settings');app.ui.dispatch('settings-section','display');
 assert.match(app.html,/data-action="library-size" data-value="tiny" aria-pressed="true"/);
 app.ui.dispatch('library-size-step','larger');assert.equal(app.pending.length,0);
});
test('failed saves retain old size and allow retry from settings',async()=>{
 const app=await fixture();app.ui.dispatch('page','settings');app.ui.dispatch('settings-section','display');
 app.ui.dispatch('library-size','tiny');app.finish(app.pending.shift(),false);
 assert.match(app.html,/data-action="library-size" data-value="normal" aria-pressed="true"/);
 app.ui.dispatch('library-size','tiny');assert.equal(app.pending.length,1);
 app.settings.library_card_size='tiny';app.finish(app.pending.shift());
 app.ui.dispatch('page','games');assert.match(app.html,/data-card-size="tiny"/);
});
test('a display response already in flight cannot undo a saved card size',async()=>{
 const app=await fixture();app.ui.dispatch('library-size','small');const size=app.pending.shift();
 app.ui.dispatch('font-size','18');const font=app.pending.shift();
 app.settings.library_card_size='small';app.finish(size);
 app.finish(font,true,{...app.settings,library_card_size:'normal',font_size:18});
 assert.match(app.html,/data-card-size="small"/);assert.match(app.html,/data-font-size="18"/);
});
