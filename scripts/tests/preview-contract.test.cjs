const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const html=fs.readFileSync('ui/index.html','utf8');
const preview=fs.readFileSync('ui/preview.js','utf8');
test('MCP preview only references IDs in the current shared markup',()=>{
 const ids=new Set([...html.matchAll(/\bid="([^"]+)"/g)].map(m=>m[1]));
 for(const match of preview.matchAll(/\bel\('([^']+)'\)/g)) assert.ok(ids.has(match[1]),'missing preview element: '+match[1]);
});
test('fixed preview protocol serves every script referenced by shared markup',()=>{
 const adapter=fs.readFileSync('src-tauri/src/theme_preview.rs','utf8');
 for(const match of html.matchAll(/<script src="([^"]+)"/g)) {
  const path=match[1]==='app.js'?'preview.js':match[1];
  assert.ok(adapter.includes('"/'+path+'"'),'missing fixed protocol resource: '+path);
 }
});
test('preview inline colors use shared renderer variables',()=>{
 const renderer=fs.readFileSync('ui/theme.js','utf8');
 const defined=new Set([...renderer.matchAll(/['"](--[\w-]+)['"]/g)].map(m=>m[1]));
 for(const match of preview.matchAll(/var\((--[\w-]+)\)/g)) assert.ok(defined.has(match[1]),'unknown preview CSS variable: '+match[1]);
});
