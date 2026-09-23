const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const context=vm.createContext({structuredClone});
vm.runInContext(fs.readFileSync('ui/theme-settings.js','utf8'),context);
const {fields,put,parseField,draftSession}=context.RustRssThemeSettings;
const plain=x=>JSON.parse(JSON.stringify(x));
test('appearance and Aa use one reading field definition',()=>{
 const reading=fields.filter(f=>f[1]==='reading');
 assert.equal(reading.length,8);
 assert.equal(new Set(fields.map(f=>f[0])).size,fields.length);
 assert.deepEqual(plain(parseField(reading[0],' Noto Serif , serif ')),['Noto Serif','serif']);
});
test('sparse patches preserve unrelated fields and distinguish inherit from clear all',()=>{
 const p={};put(p,'light_preset','paper');put(p,'overrides.reader.width',720);
 assert.deepEqual(plain(p),{light_preset:'paper',overrides:{reader:{width:720}}});
 put(p,'overrides.reader.width',null);assert.equal(p.overrides.reader.width,null);
 put(p,'overrides',null);put(p,'overrides.reader.width',640);
 assert.equal(p.overrides.reader.width,640);
});
test('draft preview never persists or changes the base revision',async()=>{
 const base={config:{revision:5}};let request;
 const s=draftSession(base,async(rev,patch)=>{request={rev,patch};return {config:{revision:rev}};});
 s.set('overrides.reader.width',720);await s.preview();
 assert.equal(base.config.revision,5);assert.equal(request.rev,5);assert.equal(s.dirty,true);
 const copy=s.patch;copy.overrides.reader.width=500;assert.equal(s.patch.overrides.reader.width,720);
 s.reset({config:{revision:6}});assert.equal(s.dirty,false);assert.equal(s.base.config.revision,6);
});
test('late validation is discarded after another edit or cancel',async()=>{
 let finish;const s=draftSession({config:{revision:0}},()=>new Promise(r=>finish=r));
 const first=s.preview();s.set('mode','dark');finish({old:true});assert.equal(await first,null);
 const next=s.preview();s.reset({config:{revision:1}});finish({old:true});assert.equal(await next,null);
});
test('seven categories retain old non-theme action IDs exactly once',()=>{
 const html=fs.readFileSync('ui/index.html','utf8');
 const tabs=[...html.matchAll(/role="tab" id="tab-([^"]+)"/g)].map(m=>m[1]);
 assert.deepEqual(tabs,['appearance','reading','subscriptions','ai','mcp','data','general']);
 const ids=[...html.matchAll(/\bid="([^"]+)"/g)].map(m=>m[1]);assert.equal(new Set(ids).size,ids.length);
 for(const id of ['act-import-opml','act-export-opml','act-backup-db','act-restore-db','act-open-logs','btn-rsshub-save','set-language','set-refresh-interval','set-mark-read','btn-list-bulk'])assert.equal(ids.filter(x=>x===id).length,1,id);
});
test('reset-all then edit preserves removal of every other existing override',()=>{
 const s=draftSession({config:{revision:1,overrides:{typography:{ui_size:18,read_size:24},reader:{width:640}}}},()=>{});
 s.set('overrides',null);s.set('overrides.typography.ui_size',14);
 assert.deepEqual(plain(s.patch),{overrides:{typography:{ui_size:14,read_size:null},reader:{width:null}}});
});
test('superseded validation errors do not replace a newer successful preview',async()=>{
 let reject;const s=draftSession({config:{revision:1}},()=>new Promise((_,r)=>reject=r));
 const old=s.preview();s.reset({config:{revision:2}});reject(Error('stale'));assert.equal(await old,null);
});
test('language switching cannot silently discard a theme draft',()=>{
 const source=fs.readFileSync('ui/app.js','utf8');
 const block=source.slice(source.indexOf("id: 'set-language'"),source.indexOf("id: 'set-close-action'"));
 assert.match(block,/if \(themeEditors\.some\(editor => editor\.dirty\)\) throw new Error/);
 assert.ok(block.indexOf('editor.dirty')<block.indexOf("invoke('set_ui_locale'"));
});
