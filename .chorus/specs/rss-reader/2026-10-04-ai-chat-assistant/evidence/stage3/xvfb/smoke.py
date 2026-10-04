import asyncio, hashlib, importlib.util, json, os, re, socket, sqlite3, subprocess, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
repo = Path('/home/tcy/Github/RustRss')
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('probe',repo/'scripts/verify-theme-native-settings.py');mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
out=Path('/tmp/rustrss-stage3-smoke'); (out/'runtime').mkdir(mode=0o700, exist_ok=True); db=out/'db.sqlite'; requests=[]
class Mock(BaseHTTPRequestHandler):
 def do_POST(self):
  request=json.loads(self.rfile.read(int(self.headers['content-length']))); requests.append(request)
  assert self.path=='/api/chat' and request['stream'] is True and len(request['tools'])==10
  self.send_response(200);self.send_header('Content-Type','application/x-ndjson');self.end_headers()
  interrupted=any("断流" in str(message) for message in request["messages"])
  for i in range(10 if interrupted else 60):
   self.wfile.write((json.dumps({'message':{'content':f'流式回答 {i+1}。\n'},'done':False},ensure_ascii=False)+'\n').encode());self.wfile.flush();time.sleep(.15)
  if interrupted:return
  self.wfile.write((json.dumps({'message':{'content':''},'done':True,'done_reason':'stop','prompt_eval_count':11,'eval_count':600})+'\n').encode());self.wfile.flush()
 def log_message(self,*args): pass
mock=ThreadingHTTPServer(('127.0.0.1',0),Mock);threading.Thread(target=mock.serve_forever,daemon=True).start()
base=f'http://127.0.0.1:{mock.server_port}'
with sqlite3.connect(db) as conn:
 conn.execute("PRAGMA foreign_keys=ON");conn.execute("DELETE FROM chat_sessions")
 for key,value in [('ai.provider','ollama'),('ai.model','stage3-mock'),('ai.base_url',base),('ui.locale','zh-CN'),('refresh.on_start','false')]:
  conn.execute('INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES(?,?,0)',(key,value))
async def main():
 display=next(':'+str(n) for n in range(70,95) if not Path(f'/tmp/.X11-unix/X{n}').exists())
 with socket.socket() as s: s.bind(('127.0.0.1',0)); inspector=s.getsockname()[1]
 env=dict(os.environ,HOME=str(out/'home'),XDG_RUNTIME_DIR=str(out/'runtime'),XDG_DATA_HOME=str(out/'data'),RUSTSS_DB=str(db),GDK_GL='disable',DISPLAY=display,WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{inspector}',RUSTSS_LOG_STDOUT='1')
 env.pop('WAYLAND_DISPLAY',None);env.pop('GDK_BACKEND',None)
 xvfb=subprocess.Popen(['Xvfb',display,'-screen','0','1600x1000x24','-nolisten','tcp','-ac'],stdout=(out/'xvfb.log').open('w'),stderr=subprocess.STDOUT)
 app=None; result={'checks':[],'screenshots':{},'limits':['DOM inspector actions on real rebuilt Tauri/WebKitGTK; no native pointer/keyboard or paid BYOK.','Mock Ollama only for desktop smoke; four provider streams covered in Rust mock integration tests.','Android/IME/Wayland/Windows/macOS not exercised.']}
 def check(name,value,detail=None):
  assert value,(name,detail);result['checks'].append({'name':name,'detail':detail})
 try:
  for _ in range(100):
   if Path(f'/tmp/.X11-unix/X{display[1:]}').exists():break
   await asyncio.sleep(.1)
  app=subprocess.Popen([str(repo/'target/debug/rustrss-desktop')],cwd=repo,env=env,stdout=(out/'app.log').open('w'),stderr=subprocess.STDOUT)
  probe=mod.Probe({'root':str(out),'inspector_port':inspector})
  for _ in range(200):
   try: await probe.js('1');break
   except (OSError,IndexError,TimeoutError):
    assert app.poll() is None;await asyncio.sleep(.1)
  await probe.until('!!document.querySelector(\'[data-key="chat:entry"]\')')
  await probe.js('document.querySelector(\'[data-key="chat:entry"]\').click();window.__stage3Events=[];window.__TAURI__.event.listen("chat:chunk",e=>window.__stage3Events.push({...e.payload,time:performance.now()}));true')
  await probe.until('!!document.querySelector("#chat-input")')
  await probe.js('localStorage.clear();document.querySelector("#chat-input").value="流式冒烟";document.querySelector("#chat-input").dispatchEvent(new Event("input"));document.querySelector("#chat-send").click();true')
  await probe.until('!document.querySelector("#generic-confirm-overlay").classList.contains("hidden")')
  await probe.js('document.querySelector("#generic-confirm-ok").click();true')
  await probe.until('document.querySelector(".chat-stream-text")?.textContent.length>60')
  wid=subprocess.check_output(['xdotool','search','--onlyvisible','--name','RustRss'],env=env,text=True).splitlines()[0]
  async def screenshot(name):
   subprocess.run(['xdotool','windowsize',wid,'1239','820'],env=env,check=True);subprocess.run(['xdotool','windowsize',wid,'1240','820'],env=env,check=True)
   subprocess.run(['xdotool','mousemove','1500','900'],env=env,check=True);await asyncio.sleep(.4)
   subprocess.run(['import','-window',wid,str(out/name)],env=env,check=True)
   result['screenshots'][name]=hashlib.sha256((out/name).read_bytes()).hexdigest()
  state=await probe.js('({text:document.querySelector(".chat-stream-text").textContent,status:document.querySelector(".chat-stream-state").textContent,button:document.querySelector("#chat-send").textContent,disabled:document.querySelector("#chat-send").disabled,live:document.querySelector("#chat-messages").getAttribute("aria-live"),label:document.querySelector("#chat-input").getAttribute("aria-label")})')
  check('partial-stream-visible-and-stop-accessible',state['status']=='正在接收回答…' and state['button']=='停止' and not state['disabled'],state)
  check('accessible-live-log-and-labelled-input',state['live']=='polite' and bool(state['label']))
  await screenshot('01-streaming.png')
  await probe.until('document.querySelector("#chat-messages").scrollHeight>document.querySelector("#chat-messages").clientHeight+120')
  await probe.js('window.__stage3Bubble=document.querySelector(".chat-stream");window.__stage3User=document.querySelector(".chat-user");document.querySelector("#chat-messages").scrollTop=0;true')
  await asyncio.sleep(1)
  anchor=await probe.js('({same:__stage3Bubble===document.querySelector(".chat-stream"),userSame:__stage3User===document.querySelector(".chat-user"),top:document.querySelector("#chat-messages").scrollTop,distance:document.querySelector("#chat-messages").scrollHeight-document.querySelector("#chat-messages").clientHeight,length:document.querySelector(".chat-stream-text").textContent.length})')
  check('incremental-node-identity-and-user-upscroll-retained',anchor['same'] and anchor['userSame'] and anchor['top']==0 and anchor['distance']>120 and anchor['length']>len(state['text']),anchor)
  await screenshot('02-streaming-upscroll.png')
  await probe.until('!document.querySelector(".chat-stream") && document.querySelector(".chat-assistant")?.textContent.includes("流式回答 60")')
  events=await probe.js('window.__stage3Events');check('chunk-sequences-monotonic',len(events)>10 and all(a['seq']<b['seq'] for a,b in zip(events,events[1:])),{'count':len(events),'sequence':[e['seq'] for e in events]})
  check('mock-request-used-stream-and-tools',len(requests)==1)
  await screenshot('03-complete.png')
  with sqlite3.connect(db) as conn:
   session_id=conn.execute('SELECT id FROM chat_sessions ORDER BY id DESC LIMIT 1').fetchone()[0]
   terminal=conn.execute('SELECT role,status,input_tokens,output_tokens FROM chat_messages WHERE session_id=? ORDER BY seq DESC LIMIT 1',(session_id,)).fetchone()
   check('real-completion-persisted',terminal==('assistant','done',11,600),terminal)
   body=json.loads(conn.execute('SELECT parts_json FROM chat_message_bodies WHERE message_id=(SELECT id FROM chat_messages WHERE session_id=? ORDER BY seq DESC LIMIT 1)',(session_id,)).fetchone()[0]);check('all-streamed-text-persisted',body==[{'text':''.join(f'流式回答 {i+1}。\n' for i in range(60))}])
   for seq in range(3,123):
    conn.execute('INSERT INTO chat_messages(session_id,seq,role,status,created_at) VALUES(?,?,?,\'done\',0)',(session_id,seq,'user' if seq%2 else 'assistant'))
    mid=conn.execute('SELECT last_insert_rowid()').fetchone()[0];conn.execute('INSERT INTO chat_message_bodies(message_id,parts_json) VALUES(?,?)',(mid,json.dumps([{'text':f'历史消息 {seq}'}],ensure_ascii=False)))
  await probe.js(f'window.RustRssChatBridge.open({session_id});true');await probe.until('document.querySelectorAll("#chat-messages .chat-message").length===50')
  await probe.js('window.__stage3Last=document.querySelector("#chat-messages").lastElementChild;document.querySelector("#chat-messages").scrollTop=0;true')
  await probe.until('document.querySelectorAll("#chat-messages .chat-message").length===100')
  page=await probe.js('({count:document.querySelectorAll("#chat-messages .chat-message").length,top:document.querySelector("#chat-messages").scrollTop,same:__stage3Last===document.querySelector("#chat-messages").lastElementChild})')
  check('real-ipc-prepend-page-and-anchor',page['count']==100 and page['top']>0 and page['same'],page)
  await screenshot('04-pagination.png')
  await probe.js('window.RustRssChatBridge.open();true');await probe.until('!document.querySelector("#chat-input").disabled')
  await probe.js('document.querySelector("#chat-input").value="断流冒烟";document.querySelector("#chat-send").click();true')
  await probe.until('document.querySelector("#chat-error").textContent.includes("EOF")')
  await probe.until('!document.querySelector(".chat-stream") && document.querySelector(".chat-assistant")?.textContent.includes("流式回答 10")')
  failure=await probe.js('({notice:document.querySelector("#chat-error").textContent,partial:document.querySelector(".chat-assistant").textContent,draft:document.querySelector("#chat-input").value})')
  check('disconnect-keeps-partial-error-count-and-draft', '已收到' in failure['notice'] and 'EOF' in failure['partial'] and failure['draft']=='断流冒烟',failure)
  with sqlite3.connect(db) as conn:
   row=conn.execute("SELECT m.status,b.parts_json FROM chat_messages m JOIN chat_message_bodies b ON b.message_id=m.id WHERE role='assistant' ORDER BY m.id DESC LIMIT 1").fetchone()
   check('partial-and-error-persisted-after-disconnect',row[0]=='failed' and len(json.loads(row[1]))==2, {'status':row[0],'blocks':len(json.loads(row[1]))})
  await screenshot('05-disconnected.png')
  check('different-damaged-frames',len(set(result['screenshots'].values()))==5)
  result['binary_sha256']=hashlib.sha256((repo/'target/debug/rustrss-desktop').read_bytes()).hexdigest();check('rebuilt-ui', (repo/'target/debug/rustrss-desktop').stat().st_mtime>(repo/'ui/app.js').stat().st_mtime)
  result['passed']=True
 finally:
  if app:app.terminate();app.wait(timeout=15)
  xvfb.terminate();xvfb.wait(timeout=15);mock.shutdown();(out/'evidence.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
 print(json.dumps(result,ensure_ascii=False,indent=2))
asyncio.run(main())
