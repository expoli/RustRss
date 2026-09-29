import {readFileSync,writeFileSync} from 'node:fs';import assert from 'node:assert/strict';
const info=JSON.parse(readFileSync(process.argv[2]));
const html=await(await fetch(`http://127.0.0.1:${info.port}/`)).text();const path=html.match(/\/socket\/\d+\/\d+\/WebPage/)[0];
const ws=new WebSocket(`ws://127.0.0.1:${info.port}${path}`);const pending=new Map();let seq=0;
let target;await new Promise((r,j)=>{ws.onerror=j;ws.onmessage=e=>{const m=JSON.parse(e.data);if(m.method==='Target.targetCreated'){target=m.params.targetInfo.targetId;r()}}});
ws.onmessage=e=>{const m=JSON.parse(e.data);if(m.method!=='Target.dispatchMessageFromTarget')return;const p=JSON.parse(m.params.message),h=pending.get(p.id);if(h){pending.delete(p.id);p.error?h.reject(p.error):h.resolve(p.result)}};
const call=(method,params)=>{const id=++seq;return new Promise((resolve,reject)=>{pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method:'Target.sendMessageToTarget',params:{targetId:target,message:JSON.stringify({id,method,params})}}))})};
const js=async expression=>{const r=await call('Runtime.evaluate',{expression,returnByValue:true});assert(!r.wasThrown,JSON.stringify(r));return r.result.value};const wait=ms=>new Promise(r=>setTimeout(r,ms));
try{
 assert.equal(await js('I18N.selfTest().ok'),true);
 await js("if(!document.querySelector('#add-row').classList.contains('hidden'))document.querySelector('#btn-add').click();true");
 await js("document.querySelector('#btn-settings').click();true");await wait(400);
 const check=JSON.parse(await js(`JSON.stringify({mobileHeader:getComputedStyle(document.querySelector('.m-settings-header')).display,nav:getComputedStyle(document.querySelector('.settings-nav')).flexDirection,body:getComputedStyle(document.querySelector('.settings-body')).display,layout:!!document.querySelector('#reading-editor [data-theme-field="reader.layout"]'),advancedAi:document.querySelector('.m-setting-advanced').open,width:innerWidth,dialog:document.querySelector('.settings-dialog').getBoundingClientRect().toJSON()})`));
 assert.equal(check.mobileHeader,'none');assert.equal(check.nav,'column');assert.notEqual(check.body,'none');assert.equal(check.layout,true);assert.equal(check.advancedAi,true);
 await js("document.querySelector('#tab-reading').click();true");await wait(200);
 assert.equal(await js("document.querySelector('#pane-reading').classList.contains('hidden')"),false);
 const ratio=await js('devicePixelRatio');const rect=Object.fromEntries(Object.entries(check.dialog).map(([k,v])=>[k,v*ratio]));const snap=await call('Page.snapshotRect',{x:rect.x,y:rect.y,width:rect.width,height:rect.height,coordinateSystem:'Viewport'});
 writeFileSync(info.root+'/desktop-settings.png',Buffer.from(snap.dataURL.split(',')[1],'base64'));
 await js("document.querySelector('#settings-close').click();document.querySelector('#btn-add').click();true");await wait(150);
 assert.equal(await js("document.querySelector('#add-row').classList.contains('hidden')"),false);
 assert.equal(await js("document.querySelector('#add-row').nextElementSibling.className"),'sidebar-foot');
 writeFileSync(info.root+'/results.json',JSON.stringify(check,null,2));console.log(JSON.stringify({root:info.root,desktopSettings:true,desktopFeedEntry:true,keysets:true}));
}finally{ws.close()}
