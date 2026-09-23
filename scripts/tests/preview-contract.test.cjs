const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');

function javascriptFiles(directory) {
 return fs.readdirSync(directory,{withFileTypes:true}).flatMap(entry=>{
  const name=path.join(directory,entry.name);
  return entry.isDirectory()?javascriptFiles(name):entry.name.endsWith('.js')?[name]:[];
 });
}
const sources=Object.fromEntries(javascriptFiles('ui').map(file=>[file.split(path.sep).join('/'),fs.readFileSync(file,'utf8')]));
const html=fs.readFileSync('ui/index.html','utf8');
const adapter=fs.readFileSync('src-tauri/src/theme_preview.rs','utf8');
const preview=sources['ui/preview.js'];

// Static contract, not a JS reachability proof. Runtime HTML templates and literal
// node.id/setAttribute creations count alongside index.html. In particular, act-*,
// ai-panel*, ctx-menu and reader-tags are NOT an unchecked prefix allowlist.
// Cover the repo's literal lookup, ternary, ID-array, binding and dropdown idioms.
// Computed IDs (e.g. 'tab-' + pane, host.id + '-fonts') cannot be enumerated here;
// keep their runtime tests. Comments/strings are not a full JavaScript AST.
function createdIds(markup, files) {
 const ids=new Set();
 for(const source of [markup,...Object.values(files)]) {
  for(const m of source.matchAll(/(?:^|[\s<])id\s*=\s*(['"])([\w:-]+)\1/g)) ids.add(m[2]);
  for(const m of source.matchAll(/\.id\s*=(?!=)\s*(['"`])([\w:-]+)\1\s*[;,]/g)) ids.add(m[2]);
  for(const m of source.matchAll(/\.setAttribute\(\s*['"]id['"]\s*,\s*(['"`])([\w:-]+)\1\s*\)/g)) ids.add(m[2]);
 }
 return ids;
}
function referencedIds(source) {
 const ids=new Set();
 const addStrings=text=>{for(const m of text.matchAll(/(['"`])([\w:-]+)\1/g)) ids.add(m[2]);};
 for(const m of source.matchAll(/\b(?:el|getElementById|bind|mountThemeEditor)\(\s*(['"`])([\w:-]+)\1\s*[,)]/g)) ids.add(m[2]);
 for(const m of source.matchAll(/\b(?:el|getElementById)\([^\n()?]*\?\s*(['"`])([\w:-]+)\1\s*:\s*(['"`])([\w:-]+)\3\s*\)/g)) {ids.add(m[2]);ids.add(m[4]);}
 for(const m of source.matchAll(/for\s*\(\s*(?:const|let)\s+id\s+of\s*\[([^\]]*)\]/g)) addStrings(m[1]);
 // Dropdown descriptors reference existing nodes; `id: '...'` is not a creator.
 for(const m of source.matchAll(/\bid\s*:\s*(['"`])([\w:-]+)\1|\.id\s*===?\s*(['"`])([\w:-]+)\3/g)) ids.add(m[2]||m[4]);
 for(const m of source.matchAll(/\b(?:querySelector|querySelectorAll|closest)\(\s*(['"])([^'"\n]*)\1\s*\)/g)) {
  for(const id of m[2].matchAll(/#([\w-]+)/g)) ids.add(id[1]);
 }
 return ids;
}
function assertIdContract(markup, files) {
 const created=createdIds(markup,files), missing=[];
 for(const [file,source] of Object.entries(files)) {
  for(const id of referencedIds(source)) if(!created.has(id)) missing.push(`${file}: ${id}`);
 }
 assert.deepEqual(missing,[],'ID references without markup/runtime creation: '+missing.join(', '));
}
// Attribute order, quote style, boolean attributes and line breaks are immaterial.
// These local static tags don't contain HTML entities or a quoted `>`.
function resourcePaths(markup) {
 const resources=[];
 for(const tag of markup.matchAll(/<(script|link)\b([^>]*)>/gi)) {
  const attrs={};
  for(const m of tag[2].matchAll(/([\w:-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/g)) attrs[m[1].toLowerCase()]=m[2]??m[3]??m[4];
  const source=tag[1].toLowerCase()==='script'?attrs.src:
   (attrs.rel||'').toLowerCase().split(/\s+/).includes('stylesheet')?attrs.href:undefined;
  if(source) resources.push('/'+source.replace(/^\.\//,''));
 }
 return resources;
}
function assertResourceContract(markup, protocol) {
 // Apply the adapter's actual literal entrypoint replacement, not an assumed
 // app.js alias (changing its quoting must not silently bypass the route check).
 for(const m of protocol.matchAll(/\.replace\(\s*("(?:\\.|[^"\\])*")\s*,\s*("(?:\\.|[^"\\])*")\s*\)/g)) {
  markup=markup.replaceAll(JSON.parse(m[1]),JSON.parse(m[2]));
 }
 // Match fixed match arms, rather than a resource name alone in a comment.
 const routes=new Set([...protocol.matchAll(/"([^"\n]+)"\s*=>\s*\(/g)].map(m=>m[1]));
 for(const resource of resourcePaths(markup)) assert.ok(routes.has(resource),'missing fixed protocol resource: '+resource);
}

test('all UI JavaScript literal ID references have markup or runtime creators',()=>assertIdContract(html,sources));
test('runtime IDs are accepted only with a creation path',()=>{
 const markup="<div id='fixed'></div>";
 const code="el('fixed'); el('act-star'); el('ai-panel'); el('ctx-menu'); el('reader-tags');";
 const creators="node.id = 'ctx-menu'; node.setAttribute('id', 'reader-tags'); const template = `<button id=\"act-star\"></button><aside id='ai-panel'></aside>`;";
 assertIdContract(markup,{'app.js':code,'components.js':creators});
 assert.throws(()=>assertIdContract(markup,{'app.js':code}),/act-star.*ai-panel.*ctx-menu.*reader-tags/);
 assert.throws(()=>assertIdContract("<div data-id='not-an-id'></div>",{'app.js':"el('not-an-id');"}),/not-an-id/);
});
test('negative control: deleted IDs in every UI file are rejected',()=>{
 assertIdContract(html,sources); // Establish a clean baseline before mutating it.
 for(const file of Object.keys(sources)) {
  assert.throws(()=>assertIdContract(html,{...sources,[file]:sources[file]+"\nel('set-font-size');"}),/set-font-size/);
 }
 // Every dead-reference shape from T4, including indirect labels and bindings.
 for(const code of ["el('set-font-size');", "el('set-font-line');",
  "el(field === 'font_read_size' ? 'set-font-size-value' : 'set-font-line-value');",
  "for (const id of ['set-font-ui', 'set-font-read', 'set-font-mono']) el(id);",
  "bind('set-font-size', 'font_read_size', FONT_SIZE);", "bind('set-font-line', 'font_read_line', FONT_LINE);",
  "if (d.id === 'set-theme-preset') {}", "const d = {id: 'set-theme-preset'};",
  "document.getElementById('deleted-arbitrary-id');", "document.querySelector('#deleted-arbitrary-id');"]) {
  assert.throws(()=>assertIdContract(html,{'synthetic.js':code}),/ID references without/);
 }
});
test('fixed preview protocol serves scripts and stylesheets in shared markup',()=>assertResourceContract(html,adapter));
test('resource scanning tolerates attribute order, quote style and extra attributes',()=>{
 const markup=`<script defer type='module' src="app.js"></script>
  <script\n src='theme.js' defer type="text/javascript"></script>
  <link href = 'style.css' media='screen' rel = 'stylesheet'>
  <link rel=stylesheet href=extra.css><link rel='icon' href='icon.png'>`;
 assert.deepEqual(resourcePaths(markup),['/app.js','/theme.js','/style.css','/extra.css']);
 assertResourceContract(markup,adapter+'\n"/extra.css" => ("text/css", bytes),');
});
test('negative control: omitted script and stylesheet resources are rejected',()=>{
 for(const [resource,tag] of [['missing.js',"<script defer src='missing.js' type='module'></script>"],['missing.css',"<link href='missing.css' rel='stylesheet'>"]]) {
  assert.throws(()=>assertResourceContract(html+tag,adapter),new RegExp('missing fixed protocol resource: /'+resource.replace('.','\\.')));
 }
 assert.throws(()=>assertResourceContract(html.replace('src="app.js"',"src='app.js'"),adapter),/missing fixed protocol resource: \/app\.js/);
 for(const resource of ['theme-settings.js','style.css']) {
  const bad=adapter.replace('"/'+resource+'" =>','"/removed" =>');
  assert.notEqual(bad,adapter);
  assert.throws(()=>assertResourceContract(html,bad),/missing fixed protocol resource/);
 }
});
test('preview inline colors use shared renderer variables',()=>{
 const defined=new Set([...sources['ui/theme.js'].matchAll(/['"](--[\w-]+)['"]/g)].map(m=>m[1]));
 for(const match of preview.matchAll(/var\((--[\w-]+)\)/g)) assert.ok(defined.has(match[1]),'unknown preview CSS variable: '+match[1]);
});
