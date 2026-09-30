// Inspect the real Android accessibility tree with TalkBack enabled on the owned AVD.
// ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/record-reading-experience-reader-talkback.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const serial = process.env.ANDROID_SERIAL;
assert.equal(serial, 'emulator-5582');
const out = resolve(process.argv[2]); mkdirSync(out,{recursive:true});
const adb=(...a)=>execFileSync('/usr/lib/android-sdk/platform-tools/adb',['-s',serial,...a],{timeout:30000,maxBuffer:10*1024*1024});
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const service='com.google.android.marvin.talkback/com.google.android.marvin.talkback.TalkBackService';
const originalServices=adb('shell','settings','get','secure','enabled_accessibility_services').toString().trim();
const originalEnabled=adb('shell','settings','get','secure','accessibility_enabled').toString().trim();
adb('shell','settings','put','secure','enabled_accessibility_services',service);
adb('shell','settings','put','secure','accessibility_enabled','1');
await wait(900);
const status=adb('shell','dumpsys','accessibility').toString();
assert(status.includes(service));
writeFileSync(`${out}/talkback-service.txt`,`before: services=${originalServices} enabled=${originalEnabled}\n${status.match(/Enabled services:\{\{[^\n]+/)?.[0]}\n`);
const target=(await(await fetch(process.env.CDP_URL||'http://127.0.0.1:9229/json')).json()).find(t=>t.type==='page'&&JSON.parse(t.description||'{}').attached);
assert(target);
const ws=new WebSocket(target.webSocketDebuggerUrl); await new Promise((r,e)=>{ws.onopen=r;ws.onerror=e});
let id=0; const pending=new Map();
ws.onmessage=e=>{const m=JSON.parse(e.data),f=pending.get(m.id);if(f){pending.delete(m.id);f(m)}};
async function js(expression){const key=++id;const p=new Promise(r=>pending.set(key,r));ws.send(JSON.stringify({id:key,method:'Runtime.evaluate',params:{expression,returnByValue:true,awaitPromise:true}}));const m=await p;assert(!m.result?.exceptionDetails,JSON.stringify(m.result?.exceptionDetails));return m.result.result.value;}
async function tree(name,required){let xml='';for(let i=0;i<10;i++){adb('shell','uiautomator','dump','/sdcard/t4-talkback.xml');xml=adb('exec-out','cat','/sdcard/t4-talkback.xml').toString();if(required(xml))break;await wait(500)}assert(required(xml),`Missing ${name}`);writeFileSync(`${out}/${name}.xml`,xml);writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'));return xml;}
try{
  await js(`document.querySelector('#entries li[data-id="1"] .title').click()`);
  await wait(600);
  const reader=await tree('talkback-reader',x=>x.includes('更多')&&x.includes('Aa'));
  assert(reader.includes('星标')&&reader.includes('稍后读')&&reader.includes('T4Tag'));
  await js(`document.querySelector('#act-more').click()`);
  await wait(450);
  const menu=await tree('talkback-more',x=>x.includes('管理标签')&&x.includes('AI 摘要'));
  assert(menu.includes('标为')&&menu.includes('管理标签'));
  const dom=await js(`({readerToolbar:document.querySelector('.reader-actions')?.getAttribute('role'),moreLabel:document.querySelector('#act-more')?.textContent,chipRole:document.querySelector('#reader-tags .tag-chip')?.getAttribute('role'),chipHeight:document.querySelector('#reader-tags .tag-chip')?.getBoundingClientRect().height,menuRole:document.querySelector('.ctx-sheet')?.getAttribute('role'),menuItems:[...document.querySelectorAll('.ctx-sheet-list button')].map(n=>({label:n.textContent,role:n.getAttribute('role'),height:n.getBoundingClientRect().height})),backgroundInert:[...document.querySelectorAll('body > *')].filter(n=>n.id!=='ctx-menu').every(n=>n.inert)})`);
  assert(dom.readerToolbar==='toolbar'&&dom.chipRole==='button'&&dom.chipHeight>=48&&dom.menuItems.every(n=>n.height>=48));
  writeFileSync(`${out}/talkback-dom.json`,JSON.stringify(dom,null,2)+'\n');
  console.log(JSON.stringify({readerNamed:true,menuNamed:true,serviceEnabled:true,menuItems:dom.menuItems.length}));
}finally{ws.close();if(originalServices==='null')adb('shell','settings','delete','secure','enabled_accessibility_services');else adb('shell','settings','put','secure','enabled_accessibility_services',originalServices);if(originalEnabled==='null')adb('shell','settings','delete','secure','accessibility_enabled');else adb('shell','settings','put','secure','accessibility_enabled',originalEnabled);}
