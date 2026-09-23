"""Native Wayland main-reader contracts via WebKit's loopback inspector.
Start only an isolated desktop with WEBKIT_INSPECTOR_HTTP_SERVER and supply its
instance.json. DOM automation opens a fixture article; it does NOT prove a native
pointer click. Local native cancel is a separate portal/input check.
Requires distro Python's websockets package. No screenshot of the user's desktop.
"""
import argparse
import asyncio
import base64
import json
from pathlib import Path
import re
import sqlite3
import time
import urllib.request
import websockets

parser=argparse.ArgumentParser()
parser.add_argument('instance')
options=parser.parse_args()
info=json.loads(Path(options.instance).read_text())
root=Path(info['root'])

def http(name,args):
    payload={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':name,'arguments':args}}
    req=urllib.request.Request(f"http://127.0.0.1:{info['mcp_port']}/mcp",data=json.dumps(payload).encode(),headers={'Authorization':'Bearer fixture-write','Content-Type':'application/json','Accept':'application/json, text/event-stream'})
    with urllib.request.urlopen(req,timeout=15) as response:return json.load(response)['result']

def content(result):
    value=result['structuredContent'];assert value['ok'],value;return value

async def main():
    with urllib.request.urlopen(f"http://127.0.0.1:{info['inspector_port']}/",timeout=5) as r:html=r.read().decode()
    path=re.search(r"(/socket/\d+/\d+/WebPage)",html).group(1)
    report={'input_method':'WebKit inspector DOM automation; native pointer cancellation reported separately','checks':[]}
    async with websockets.connect(f"ws://127.0.0.1:{info['inspector_port']}"+path,max_size=4*1024**2) as ws:
        while True:
            event=json.loads(await ws.recv())
            if event.get('method')=='Target.targetCreated' and event['params']['targetInfo']['type']=='page':
                target=event['params']['targetInfo']['targetId'];break
        counter=0
        async def evaluate(expression):
            nonlocal counter
            counter+=1;mid=counter
            await ws.send(json.dumps({'id':mid,'method':'Target.sendMessageToTarget','params':{'targetId':target,'message':json.dumps({'id':mid,'method':'Runtime.evaluate','params':{'expression':expression,'returnByValue':True}})}}))
            while True:
                msg=json.loads(await asyncio.wait_for(ws.recv(),10))
                if msg.get('method')!='Target.dispatchMessageFromTarget':continue
                answer=json.loads(msg['params']['message'])
                if answer.get('id')!=mid:continue
                assert 'error' not in answer and not answer['result'].get('wasThrown'),answer
                return answer['result']['result'].get('value')
        async def until(expression):
            for _ in range(100):
                if await evaluate(expression):return
                await asyncio.sleep(.05)
            raise AssertionError(expression)
        def check(ok,name):
            assert ok,name
            report['checks'].append(name)
        await evaluate("window.__t7Previous=document.querySelector('#reader .article');document.querySelector('#entries li[data-id]').click();true")
        await until("!!document.querySelector('#reader .article p') && document.querySelector('#reader .article')!==window.__t7Previous")
        await evaluate("""(()=>{const r=document.getElementById('reader'), a=r.querySelector('.article');
          window.__t7={reader:r,article:a,row:document.querySelector('#entries li.active'),paragraph:a.querySelectorAll('p')[15],clicks:[]};
          r.scrollTop+=__t7.paragraph.getBoundingClientRect().top-r.getBoundingClientRect().top;
          __t7.before=__t7.paragraph.getBoundingClientRect().top-r.getBoundingClientRect().top;
          return true;})()""")
        measure="""JSON.stringify({offset:__t7.paragraph.getBoundingClientRect().top-__t7.reader.getBoundingClientRect().top,
          before:__t7.before,sameArticle:__t7.article===document.querySelector('#reader .article'),
          sameRow:__t7.row===document.querySelector('#entries li.active'),sameParagraph:__t7.paragraph.isConnected,
          scrollTop:__t7.reader.scrollTop,font:getComputedStyle(__t7.article).fontSize,dpr:devicePixelRatio})"""
        async def capture_with_retry(arguments):
            result=await asyncio.to_thread(http,'preview_theme',arguments)
            for _ in range(2):
                v=result['structuredContent']
                if v.get('ok'): return result
                if v.get('error_code')!='render_timeout' or not v.get('preview_id'): break
                report.setdefault('render_timeout_retries',[]).append(v)
                result=await asyncio.to_thread(http,'capture_theme_preview',{'preview_id':v['preview_id'],'expected_preview_revision':v['preview_revision'],'scene':arguments['scene'],'mode':arguments['mode']})
            return result
        report['before']=json.loads(await evaluate(measure))
        current=content(http('get_theme',{}))
        # The current revision comes from the actual core, including restart runs.
        revision=current['theme']['config']['revision']
        read_size=22 if current['theme']['light']['typography']['read_size']==24 else 24
        width=620 if current['theme']['light']['reader']['width']==540 else 540
        report['requested']={'read_size':read_size,'width':width}
        result=await capture_with_retry({'base_revision':revision,'patch':{'mode':'light','overrides':{'typography':{'read_size':read_size},'reader':{'width':width}}},'scene':'article','mode':'light'})
        p=content(result)
        check(p['capture']['display_backend']=='GdkWaylandDisplay','native GdkWaylandDisplay capture (not Xwayland)')
        image=next(c for c in result['content'] if c['type']=='image')
        (root/'native-preview.png').write_bytes(base64.b64decode(image['data']))
        report['capture']=p['capture']
        preview_state=json.loads(await evaluate(measure))
        check(preview_state==report['before'],'temporary preview preserves main reader position and nodes')
        save=content(http('finish_theme_preview',{'preview_id':p['preview_id'],'expected_preview_revision':p['preview_revision'],'action':'save'}))
        await until(f"getComputedStyle(__t7.article).fontSize==='{read_size}px'")
        await asyncio.sleep(.2)
        after=json.loads(await evaluate(measure));report['after_save']=after
        check(after['sameArticle'] and after['sameRow'] and after['sameParagraph'],'save preserves real article/row/paragraph identities')
        check(abs(after['offset']-report['before']['offset'])<2,'save preserves paragraph offset within 2 CSS px')
        report['saved_revision']=save['saved_revision']
        with sqlite3.connect(root/'fixture.sqlite') as db:
            stored=json.loads(db.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()[0])['current']
        check(stored['revision']==save['saved_revision'] and stored['overrides']['reader']['width']==width,'saved configuration confirmed directly in isolated SQLite')
        # Leave a fresh candidate for the independently authorized real click.
        p=content(await capture_with_retry({'base_revision':stored['revision'],'patch':{'mode':'dark'},'scene':'settings','mode':'dark'}))
        report['local_cancel_candidate']={'preview_id':p['preview_id'],'expected_preview_revision':p['preview_revision'],'base_revision':stored['revision'],'capture':p['capture']}
        report['before_cancel']=json.loads(await evaluate(measure))
        (root/'wayland-results.json').write_text(json.dumps(report,indent=2))
        print(json.dumps({'root':str(root),'checks':report['checks'],'before':report['before'],'after':after,'capture_backend':p['capture']['display_backend']}),flush=True)
asyncio.run(main())
