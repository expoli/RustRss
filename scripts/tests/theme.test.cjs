const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const ctx = vm.createContext({});
vm.runInContext(fs.readFileSync('ui/theme.js','utf8'),ctx);
const theme = ctx.RustRssTheme;
const values = () => ({ colors: Object.fromEntries(Object.keys(theme.colorVars).map(k=>[k,'#abcdef'])),
  typography:{ui_family:['Example','sans-serif'],read_family:['serif'],mono_family:['monospace'],ui_size:14,read_size:18,mono_size:13,line_height:1.7},
  list:{density:'comfortable',summary_lines:2,thumbnail:true},reader:{width:680,paragraph_gap:1,layout:'three_column'},chrome:{radius:8,sidebar_width:220,list_width:340}});
const snapshot = () => ({config:{revision:1,mode:'light'},config_hash:'a',light:values(),dark:values()});
function setup() {
 let writes=0,changed;
 const style=new Map();const dataset={};
 const root={style:{setProperty(k,v){writes++;style.set(k,v);}},dataset:new Proxy(dataset,{set(o,k,v){writes++;o[k]=v;return true;}})};
 const media={matches:false,addEventListener(t,f){changed=f;},removeEventListener(){changed=null;}};
 return {root,style,media,change:()=>changed(),writes:()=>writes,renderer:theme.createRenderer(root,{media})};
}
test('every semantic color and layout field maps to a bounded token',()=>{
 const v=values(), t=theme.tokens(v);
 for(const [k,varname] of Object.entries(theme.colorVars)) assert.equal(t[varname],v.colors[k]);
 assert.equal(t['--reader-width'],'680px');assert.equal(t['--summary-lines'],'2');
 assert.equal(t['--font-read-size'],'18px');
});
test('same snapshot has zero writes; color changes do not touch layout or DOM children',()=>{
 const h=setup(),s=snapshot();h.renderer.apply(s);const n=h.writes();
 assert.equal(h.renderer.apply(s).writes,0);assert.equal(h.writes(),n);
 s.light.colors.accent='#112233';assert.equal(h.renderer.apply(s).writes,1);
 assert.equal(h.style.get('--accent'),'#112233');
});
test('system changes use complete mode palette without persisting; stale revisions ignored',()=>{
 const h=setup(),s=snapshot();s.config.mode='system';s.dark.colors.background='#000000';h.renderer.apply(s);
 h.media.matches=true;h.change();assert.equal(h.style.get('--bg'),'#000000');assert.equal(h.root.dataset.theme,'dark');
 const old=snapshot();old.config.revision=0;assert.equal(h.renderer.apply(old).stale,true);
 assert.equal(h.style.get('--bg'),'#000000');
});
test('font names are individual escaped families; generic fallbacks stay generic',()=>{
 assert.equal(theme.fontFamily(['Foo, Bar','X"; color: red','a\\b','serif']), '"Foo, Bar", "X\\"; color: red", "a\\\\b", serif');
});
test('local slider preview resets to the authoritative snapshot without a revision change',()=>{
 const h=setup(),s=snapshot();h.renderer.apply(s,{font_read_size:27});assert.equal(h.style.get('--font-read-size'),'27px');
 h.renderer.apply(s);assert.equal(h.style.get('--font-read-size'),'18px');
});
test('layout change preserves live paragraph offset without replacing nodes',()=>{
 const h=setup();let top=40; const reader={clientHeight:400,scrollTop:100,getBoundingClientRect:()=>({top:0})};
 const node={isConnected:true,getBoundingClientRect:()=>({top:top-reader.scrollTop,bottom:top-reader.scrollTop+200})};
 reader.querySelectorAll=()=>[node];
 const root={...h.root,style:{setProperty(k,v){if(k==='--font-read-size')top=Number.parseInt(v)*10;}}};
 const renderer=theme.createRenderer(root,{reader,media:h.media});const s=snapshot();renderer.apply(s);
 const before=node.getBoundingClientRect().top;s.light.typography.read_size=24;renderer.apply(s);
 assert.equal(node.getBoundingClientRect().top,before);
});
test('late font completion does not undo user scrolling or touch a replaced article',async()=>{
 let resolve;const ready=new Promise(r=>{resolve=r;});const h=setup();let top=40;
 const reader={clientHeight:400,scrollTop:100,getBoundingClientRect:()=>({top:0})};
 const node={isConnected:true,getBoundingClientRect:()=>({top:top-reader.scrollTop,bottom:top-reader.scrollTop+200})};reader.querySelectorAll=()=>[node];
 const root={...h.root,ownerDocument:{fonts:{ready}},style:{setProperty(k,v){if(k==='--font-read-size')top=Number.parseInt(v)*10;}}};
 const renderer=theme.createRenderer(root,{reader,media:h.media});renderer.apply(snapshot());
 reader.scrollTop=900;node.isConnected=false;resolve();await ready;await Promise.resolve();assert.equal(reader.scrollTop,900);
});
test('settings responses cannot roll the current theme backward',()=>{
 const source=fs.readFileSync('ui/app.js','utf8');const start=source.indexOf('function acceptSettings(');const end=source.indexOf('\n}',start)+2;
 const c=vm.createContext({state:{settings:{theme_snapshot:{config:{revision:4}},theme:'dark',font_read_size:24}}});
 vm.runInContext(source.slice(start,end),c);
 c.response={theme_snapshot:{config:{revision:3}},theme:'light',font_read_size:18,locale:'en'};
 vm.runInContext('acceptSettings(response)',c);
 assert.equal(c.state.settings.theme_snapshot.config.revision,4);assert.equal(c.state.settings.font_read_size,24);assert.equal(c.state.settings.locale,'en');
});
