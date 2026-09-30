// T4 follow-up: existing thumbnail preference and 400-row second-page return state.
// Reset the isolated 400-row fixture and use the installed APK on emulator-5582.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const serial=process.env.ANDROID_SERIAL;assert.equal(serial,'emulator-5582');
const out=resolve(process.argv[2]);mkdirSync(out,{recursive:true});
const adb=(...args)=>execFileSync('/usr/lib/android-sdk/platform-tools/adb',['-s',serial,...args],{timeout:30000});
const target=(await(await fetch(process.env.CDP_URL||'http://127.0.0.1:9229/json')).json()).find(t=>t.type==='page'&&JSON.parse(t.description||'{}').attached);assert(target);
const socket=new WebSocket(target.webSocketDebuggerUrl);await new Promise((r,e)=>{socket.onopen=r;socket.onerror=e});
let seq=0;const pending=new Map();socket.onmessage=e=>{const m=JSON.parse(e.data),slot=pending.get(m.id);if(!slot)return;clearTimeout(slot.timer);pending.delete(m.id);m.error?slot.fail(m.error):slot.done(m.result)};
const call=(method,params={})=>new Promise((done,fail)=>{const id=++seq,timer=setTimeout(()=>{pending.delete(id);fail(new Error(`CDP ${method} timeout`))},15000);pending.set(id,{done,fail,timer});socket.send(JSON.stringify({id,method,params}))});
async function js(expression){const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});assert(!r.exceptionDetails,JSON.stringify(r.exceptionDetails));return r.result.value;}
const wait=ms=>new Promise(r=>setTimeout(r,ms));
async function until(expression){for(let i=0;i<120;i++){try{if(await js(expression))return}catch{}await wait(100)}throw new Error(`Timed out: ${expression}`)}
async function native(selector){const p=await js(`(() => {const n=document.querySelector(${JSON.stringify(selector)});if(!n)return null;n.scrollIntoView({block:'nearest'});const r=n.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;return {x,y,dpr:devicePixelRatio,visible:r.width>0&&r.height>0&&y>=0&&y<visualViewport.height&&document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n}})()`);assert(p?.visible,`Not tappable ${selector}: ${JSON.stringify(p)}`);adb('shell','input','tap',String(Math.round(p.x*p.dpr)),String(Math.round(JSON.parse(target.description).screenY+p.y*p.dpr)));await wait(300)}
function back(){adb('shell','input','keyevent','KEYCODE_BACK')}
function shot(name){writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'))}
const checks=[];const record=(name,data)=>checks.push({name,...data});
const settings=()=>js(`window.__TAURI__.core.invoke('get_ui_settings')`);
const imageCount=()=>Number(readFileSync('/tmp/rustrss-t4-image-count.txt','utf8'));
async function all(){await native('#m-seg-articles [data-mview="all"]');await until(`document.querySelector('#views li.active')?.dataset.kind==='all' && !!document.querySelector('#entries li[data-id="1"]')`)}
async function setThumbnail(value){const current=await settings();await js(`window.__TAURI__.core.invoke('update_ui_theme',{expectedRevision:${current.theme_snapshot.config.revision},patch:{overrides:{list:{thumbnail:${value}}}}})`);await call('Page.reload',{ignoreCache:true});await until(`!!document.querySelector('#m-seg-articles [data-mview="all"]')`);await all();}
function thumbnail(){return js(`(() => {const n=document.querySelector('#entries li[data-id="1"] img.entry-thumbnail');return {nodePresent:!!n,complete:n?.complete,naturalWidth:n?.naturalWidth,display:n?getComputedStyle(n).display:null,preference:document.documentElement.dataset.thumbnails}})()`)}
try{
 await until(`!!document.querySelector('#m-seg-articles [data-mview="all"]')`);
 await js(`window.__TAURI__.core.invoke('set_list_sort',{sort:'newest'})`);
 await call('Page.reload',{ignoreCache:true});
 await until(`!!document.querySelector('#m-seg-articles [data-mview="all"]')`);
 await all();
 await setThumbnail(true);
 await until(`document.querySelector('#entries li[data-id="1"] img.entry-thumbnail')?.naturalWidth===64`);
 const onBefore=await thumbnail();
 assert(onBefore.nodePresent&&onBefore.display!=='none'&&onBefore.preference==='true');
 shot('10-thumbnail-on');
 await setThumbnail(false);
 const off=await thumbnail();
 const offStored=(await settings()).theme_snapshot.light.list.thumbnail;
 assert(off.nodePresent&&off.display==='none'&&off.preference==='false'&&offStored===false);
 shot('11-thumbnail-off');
 await native('#entries li[data-id="1"] .title');
 await until(`document.body.dataset.mpage==='reader' && !!document.querySelector('.reader-image-fallback')`);
 const offlineWhenOff=await js(`({fallback:document.querySelector('.reader-image-fallback')?.textContent,articleImages:document.querySelectorAll('.article img').length})`);
 assert.equal(offlineWhenOff.fallback,'Fixture illustration');
 back();await until(`document.body.dataset.mpage==='articles'`);
 record('global-thumbnail-off-reader-offline-fallback',{before:onBefore,off,offStored,offlineWhenOff,imageRequests:imageCount()});
 await setThumbnail(true);
 await until(`document.querySelector('#entries li[data-id="1"] img.entry-thumbnail')?.naturalWidth===64`);
 const onAfter=await thumbnail();
 const onStored=(await settings()).theme_snapshot.light.list.thumbnail;
 assert(onAfter.nodePresent&&onAfter.display!=='none'&&onAfter.preference==='true'&&onStored===true);
 shot('12-thumbnail-restored');
 await native('#entries li[data-id="1"] .title');
 await until(`document.body.dataset.mpage==='reader' && !!document.querySelector('.reader-image-fallback')`);
 const offlineWhenOn=await js(`document.querySelector('.reader-image-fallback')?.textContent`);
 assert.equal(offlineWhenOn,offlineWhenOff.fallback);
 back();await until(`document.body.dataset.mpage==='articles'`);
 record('global-thumbnail-on-reader-offline-fallback',{onAfter,onStored,offlineWhenOn,imageRequests:imageCount(),policy:'list thumbnail visibility; article external-image loading unchanged'});

 await native('#btn-list-sort');
 await until(`!!document.querySelector('#ctx-menu button')`);
 const sortFound=await js(`(() => {const b=[...document.querySelectorAll('#ctx-menu button')].find(n=>n.textContent.includes('最早在前'));if(!b)return false;b.dataset.t4Target='true';return true})()`);
 assert(sortFound);
 await native('#ctx-menu button[data-t4-target="true"]');
 await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.list_sort==='oldest')`);
 await until(`document.querySelector('#list-count')?.textContent.includes('/400')`);
 const firstPage=await js(`({count:document.querySelector('#list-count').textContent,loaded:document.querySelectorAll('#entries li[data-id]').length,sort:document.querySelector('#btn-list-sort').getAttribute('aria-label')})`);
 assert(firstPage.loaded===200);
 await js(`document.querySelector('#entries').scrollTop=document.querySelector('#entries').scrollHeight`);
 await until(`document.querySelectorAll('#entries li[data-id]').length>200`);
 const pageTwo=await js(`({count:document.querySelector('#list-count').textContent,loaded:document.querySelectorAll('#entries li[data-id]').length})`);
 assert(pageTwo.loaded>200);
 const targetId=await js(`[...document.querySelectorAll('#entries li[data-id]')].slice(200).find(n=>Number(n.dataset.id)>200)?.dataset.id`);
 assert(Number(targetId)>200);
 await js(`document.querySelector('#entries li[data-id="${targetId}"]').scrollIntoView({block:'center'})`);
 const capture=()=>js(`(() => {const l=document.querySelector('#entries'),top=l.getBoundingClientRect().top,row=[...l.querySelectorAll('li[data-id]')].find(n=>n.getBoundingClientRect().bottom>top);return {view:document.querySelector('#views li.active')?.dataset.kind,sort:document.querySelector('#btn-list-sort').getAttribute('aria-label'),storedSort:null,count:document.querySelector('#list-count').textContent,loaded:l.querySelectorAll('li[data-id]').length,scrollTop:l.scrollTop,topRow:row?.dataset.id,topOffset:row?.getBoundingClientRect().top-top,selected:l.querySelector('li.active')?.dataset.id,query:document.querySelector('#search').value}})()`);
 const before=await capture();before.storedSort=(await settings()).list_sort;
 assert(before.storedSort==='oldest'&&before.loaded>200&&before.scrollTop>0);
 shot('13-page-two-list');
 await native(`#entries li[data-id="${targetId}"] .title`);
 await until(`document.body.dataset.mpage==='reader'`);
 const opened=await js(`({selected:document.querySelector('#entries li.active')?.dataset.id,title:document.querySelector('.reader-head h1')?.textContent})`);
 assert.equal(opened.selected,targetId);
 shot('14-page-two-reader');
 back();await until(`document.body.dataset.mpage==='articles'`);
 const after=await capture();after.storedSort=(await settings()).list_sort;
 assert.equal(after.view,before.view);assert.equal(after.storedSort,before.storedSort);assert.equal(after.count,before.count);assert.equal(after.loaded,before.loaded);assert.equal(after.query,before.query);assert.equal(after.selected,targetId);assert.equal(after.topRow,before.topRow);assert(Math.abs(after.scrollTop-before.scrollTop)<3);assert(Math.abs(after.topOffset-before.topOffset)<3);
 record('page-two-sort-scroll-reader-back',{firstPage,pageTwo,targetId,before,opened,after});
 shot('15-page-two-back');
 writeFileSync(`${out}/followup-results.json`,JSON.stringify({serial,fixtureSha256:'bcee169937f14b9db3dcb11bc95d0bb6f77dbe3f8712bbff8235c90324ebd38a',checks},null,2)+'\n');
 console.log(JSON.stringify({checks:checks.map(x=>x.name)}));
}finally{socket.close()}
