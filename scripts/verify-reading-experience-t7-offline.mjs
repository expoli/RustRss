// Final APK native cached-feed failure/retry on an isolated loopback fixture.
// ANDROID_SERIAL=emulator-5584 node scripts/verify-reading-experience-t7-offline.mjs OUT
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {createServer} from 'node:http';
import {mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import {resolve} from 'node:path';

const serial=process.env.ANDROID_SERIAL;assert.equal(serial,'emulator-5584');
const out=resolve(process.argv[2]);mkdirSync(out,{recursive:true});
const adb=(...args)=>execFileSync('/usr/lib/android-sdk/platform-tools/adb',['-s',serial,...args],{timeout:30000,maxBuffer:20*1024*1024});
const say=(...args)=>adb(...args).toString().trim(),wait=ms=>new Promise(ok=>setTimeout(ok,ms));
assert.match(say('emu','avd','name'),/RustRssT5/);
const hash=file=>createHash('sha256').update(readFileSync(file)).digest('hex');
const apkSha256=hash('src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk');
const shot=name=>writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'));
const body=`<?xml version="1.0"?><rss version="2.0"><channel><title>T7 Offline Fixture</title><link>http://10.0.2.2/</link><description>isolated</description><item><guid>cached-1</guid><title>Cached Offline Article</title><link>http://10.0.2.2/cached-1</link><description>Readable while transport is down.</description></item></channel></rss>`;
const server=createServer((_req,res)=>{res.writeHead(200,{'content-type':'application/rss+xml; charset=utf-8'});res.end(body)});
await new Promise(ok=>server.listen(0,'0.0.0.0',ok));const port=server.address().port;
const url=`http://10.0.2.2:${port}/feed.xml`;
async function attach(){
  adb('shell','am','force-stop','tech.expoli.rustrss');adb('shell','am','start','-n','tech.expoli.rustrss/.MainActivity');
  let pid='';for(let i=0;i<50;i++){try{pid=say('shell','pidof','tech.expoli.rustrss').split(' ')[0]}catch{}if(pid)break;await wait(200)}assert(pid);
  adb('forward','tcp:9237',`localabstract:webview_devtools_remote_${pid}`);
  let target;for(let i=0;i<50;i++){try{target=(await(await fetch('http://127.0.0.1:9237/json')).json()).find(t=>t.type==='page'&&JSON.parse(t.description||'{}').attached);if(target)break}catch{}await wait(200)}assert(target);
  const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((ok,fail)=>{ws.onopen=ok;ws.onerror=fail});
  let id=0;const pending=new Map();ws.onmessage=e=>{const m=JSON.parse(e.data),p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(m.error):p.ok(m.result)};
  const js=expr=>new Promise((ok,fail)=>{const n=++id,timer=setTimeout(()=>{pending.delete(n);fail(Error('CDP timeout'))},15000);pending.set(n,{ok:r=>r.exceptionDetails?fail(Error(JSON.stringify(r.exceptionDetails))):ok(r.result.value),fail,timer});ws.send(JSON.stringify({id:n,method:'Runtime.evaluate',params:{expression:expr,returnByValue:true,awaitPromise:true}}))});
  const until=async expr=>{for(let i=0;i<100;i++){try{if(await js(expr))return}catch{}await wait(100)}throw Error(`Timed out ${expr}`)};
  const core=(cmd,args={})=>js(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
  return {ws,js,until,core,pid};
}
let p;
try{
  p=await attach();await p.until(`document.body.dataset.mpage==='articles'`);
  assert.deepEqual(await p.core('list_feeds'),[],'Only run after disposable empty DB fixture');
  const feedId=await p.core('add_feed',{url});assert(Number.isInteger(feedId));
  const initialRefresh=await p.core('refresh_feed',{feedId,concurrency:1});
  const initial=await p.core('get_entry',{id:1});assert.equal(initial.title,'Cached Offline Article');
  p.ws.close();p=null;
  p=await attach();await p.until(`!!document.querySelector('#entries li[data-id="1"]')`);
  const cached=await p.js(`({listCount:document.querySelectorAll('#entries li[data-id]').length,title:document.querySelector('#entries li[data-id="1"] .title')?.textContent,viewport:visualViewport.width,scrollWidth:document.documentElement.scrollWidth})`);
  shot('online-cached-list');
  await new Promise(ok=>server.close(ok));
  const failure=await p.core('refresh_feed',{feedId,concurrency:1});
  const failedFeed=(await p.core('list_feeds')).find(f=>f.id===feedId);
  await p.js(`document.querySelector('#entries li[data-id="1"] .title').click();true`);
  await p.until(`document.body.dataset.mpage==='reader'`);
  const offline=await p.js(`({page:document.body.dataset.mpage,title:document.querySelector('#reader h1')?.textContent,body:document.querySelector('#reader .article')?.textContent,viewport:visualViewport.width,scrollWidth:document.documentElement.scrollWidth})`);
  shot('offline-cached-reader');
  assert.equal(offline.title,'Cached Offline Article');assert(offline.body.includes('Readable while transport is down.'));assert(offline.scrollWidth<=offline.viewport+1);
  await new Promise(ok=>server.listen(port,'0.0.0.0',ok));
  const retry=await p.core('refresh_feed',{feedId,concurrency:1});
  const recoveredFeed=(await p.core('list_feeds')).find(f=>f.id===feedId);
  const recovered=await p.core('get_entry',{id:1});
  shot('retry-cached-reader');
  assert.equal(recovered.title,'Cached Offline Article');
  const result={serial,avd:'RustRssT5',apkSha256,url,feedId,initialRefresh,cached,failure,failedFeed:{lastStatus:failedFeed.last_status,lastError:failedFeed.last_error},offline,retry,recoveredFeed:{lastStatus:recoveredFeed.last_status,lastError:recoveredFeed.last_error},recoveredTitle:recovered.title,passed:true};
  writeFileSync(`${out}/offline-results.json`,JSON.stringify(result,null,2)+'\n');
  console.log(JSON.stringify({apkSha256,feedId,failure,failedStatus:failedFeed.last_status,retry,recoveredStatus:recoveredFeed.last_status}));
}finally{p?.ws.close();try{await new Promise(ok=>server.close(ok))}catch{}}
