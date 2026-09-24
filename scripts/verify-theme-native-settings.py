"""Native KDE Wayland checks. Requires explicit permission to change global settings.
Run with --allow-desktop-changes INSTANCE_JSON. Restores output scale and color
scheme in finally. Uses the isolated launcher instance, never a user database.
Requires qdbus6, kscreen-doctor, plasma-apply-colorscheme and Python websockets.
No pointer portal or desktop-wide screenshots. DOM clicks are not native input.
"""
import argparse
import asyncio
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import time
import urllib.request
import uuid
import websockets


def run(*args):
    return subprocess.check_output(args, text=True, timeout=15).strip()


class Probe:
    def __init__(self, info):
        self.info = info
        self.root = Path(info['root'])

    def call(self, name, args):
        req = urllib.request.Request(f"http://127.0.0.1:{self.info['mcp_port']}/mcp",
            data=json.dumps({'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':name,'arguments':args}}).encode(),
            headers={'Authorization':'Bearer fixture-write','Content-Type':'application/json','Accept':'application/json, text/event-stream'})
        with urllib.request.urlopen(req, timeout=15) as response:
            reply = json.load(response)['result']
            assert 'structuredContent' in reply, reply
            result = reply['structuredContent']
        assert result['ok'], result
        return result

    async def js(self, expression):
        with urllib.request.urlopen(f"http://127.0.0.1:{self.info['inspector_port']}/",timeout=5) as response:
            path = re.findall(r'(/socket/\d+/\d+/WebPage)',response.read().decode())[0]
        async with websockets.connect(f"ws://127.0.0.1:{self.info['inspector_port']}"+path) as ws:
            while True:
                msg = json.loads(await asyncio.wait_for(ws.recv(),10))
                if msg.get('method') == 'Target.targetCreated':
                    target = msg['params']['targetInfo']['targetId']; break
            request = {'id':1,'method':'Runtime.evaluate','params':{'expression':expression,'returnByValue':True}}
            await ws.send(json.dumps({'id':1,'method':'Target.sendMessageToTarget','params':{'targetId':target,'message':json.dumps(request)}}))
            while True:
                msg = json.loads(await asyncio.wait_for(ws.recv(),10))
                if msg.get('method') != 'Target.dispatchMessageFromTarget': continue
                result = json.loads(msg['params']['message'])
                if result.get('id') != 1: continue
                assert 'error' not in result and not result['result'].get('wasThrown'), result
                return result['result']['result'].get('value')

    async def until(self, expression):
        for _ in range(100):
            if await self.js(expression): return
            await asyncio.sleep(.1)
        raise AssertionError('Timed out: '+expression)

    def config(self):
        with sqlite3.connect(self.root/'fixture.sqlite') as db:
            row=db.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()
            return row[0] if row else None

    def position(self):
        tag = 'RUSTSS_NATIVE_'+uuid.uuid4().hex
        script = self.root/'native-position.js'
        script.write_text("for (const w of workspace.windowList()) if (w.pid === "+str(self.info['pid'])+
            ") { w.frameGeometry={x:60,y:60,width:1240,height:820}; workspace.activeWindow=w; print('"+tag+" '+JSON.stringify({caption:w.caption,frame:w.frameGeometry,output:w.output.name})); }")
        name = tag.lower()
        sid = run('qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.loadScript',str(script),name)
        try:
            run('qdbus6','org.kde.KWin','/Scripting/Script'+sid,'org.kde.kwin.Script.run')
            time.sleep(.3)
            logs=run('journalctl','--user','-n','120','-o','cat')
            return [json.loads(line.split(tag+' ',1)[1]) for line in logs.splitlines() if tag+' ' in line]
        finally:
            run('qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.unloadScript',name)


async def main(options):
    assert options.allow_desktop_changes, 'Explicit desktop-change permission required'
    assert os.environ.get('WAYLAND_DISPLAY'), 'Native Wayland session required'
    info=json.loads(Path(options.instance).read_text()); probe=Probe(info)
    assert Path(f"/proc/{info['pid']}").exists()
    assert str(probe.root/'fixture.sqlite').encode() in Path(f"/proc/{info['pid']}/environ").read_bytes(), 'Not the isolated fixture process'
    subprocess.run(['df','-h','.'],check=True)
    original=json.loads(run('kscreen-doctor','-j'))
    output=next(o for o in original['outputs'] if o['name']==options.output)
    assert output['pos']=={'x':0,'y':0}, 'Probe positions assume target output at origin'
    schemes=run('plasma-apply-colorscheme','--list-schemes')
    scheme=re.search(r'\* (\S+) \(current color scheme\)',schemes).group(1)
    report={'original_output':output,'original_scheme':scheme,'checks':[],'scales':[],'modes':[],
            'limits':['DOM geometry/input only; native pointer and perceived sharpness require human observation',
                      'GTK integer buffer scale is distinct from compositor fractional output scale']}
    def check(condition,name):
        assert condition,name
        report['checks'].append(name)
    async def measure():
        return json.loads(await probe.js("""JSON.stringify({mediaDark:matchMedia('(prefers-color-scheme: dark)').matches,
          theme:document.documentElement.dataset.theme,dpr:devicePixelRatio,viewport:[innerWidth,innerHeight],
          bodyWidth:document.documentElement.scrollWidth,bg:getComputedStyle(document.documentElement).getPropertyValue('--bg').trim(),
          articleSame:!window.__nativeArticle||__nativeArticle===document.querySelector('#reader .article'),
          paragraphOffset:window.__nativeParagraph?__nativeParagraph.getBoundingClientRect().top-document.getElementById('reader').getBoundingClientRect().top:null})"""))
    async def mode(value):
        current=probe.call('get_theme',{})['theme']['config']['revision']
        probe.call('update_theme',{'expected_revision':current,'patch':{'mode':value}})
        await asyncio.sleep(.5)
    try:
        if options.manual_scale:
            await mode('system')
            run('kscreen-doctor',f'output.{options.output}.scale.{options.manual_scale}')
            report['manual_geometry']=probe.position()
            await probe.js("document.getElementById('settings-overlay').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));window.__nativeClicks=[];if(window.__nativeClickListener)document.removeEventListener('click',__nativeClickListener,true);window.__nativeClickListener=e=>{const b=e.target.closest('button');if(!b)return;const r=b.getBoundingClientRect();__nativeClicks.push({id:b.id,trusted:e.isTrusted,x:e.clientX,y:e.clientY,inside:e.clientX>=r.left&&e.clientX<=r.right&&e.clientY>=r.top&&e.clientY<=r.bottom});};document.addEventListener('click',__nativeClickListener,true);true")
            print('MANUAL READY scale='+str(options.manual_scale)+'; click Settings then Reading in isolated RustRss',flush=True)
            deadline=time.monotonic()+180
            while time.monotonic()<deadline:
                clicks=json.loads(await probe.js('JSON.stringify(window.__nativeClicks)'))
                if all(any(c['id']==name and c['trusted'] and c['inside'] for c in clicks) for name in ['btn-settings','tab-reading']):
                    report['native_clicks']=clicks
                    check(True,f'trusted button coordinates at native scale {options.manual_scale}')
                    report['manual_scale']=options.manual_scale
                    report['passed']=True
                    return
                await asyncio.sleep(.5)
            raise TimeoutError('No completed human clicks within 180 seconds; restore desktop')
        await probe.js("document.querySelector('#entries li[data-id]').click();true")
        await probe.until("!!document.querySelector('#reader .article p')")
        await probe.js("window.__nativeArticle=document.querySelector('#reader .article');window.__nativeParagraph=__nativeArticle.querySelectorAll('p')[10];document.getElementById('reader').scrollTop+=__nativeParagraph.getBoundingClientRect().top-document.getElementById('reader').getBoundingClientRect().top;true")
        for scale in [1.25,1.5]:
            run('kscreen-doctor',f'output.{options.output}.scale.{scale}')
            geometry=probe.position(); await asyncio.sleep(1)
            actual=next(o for o in json.loads(run('kscreen-doctor','-j'))['outputs'] if o['name']==options.output)
            check(actual['scale']==scale,f'compositor output scale {scale}')
            check(bool(geometry) and all(w['output']==options.output for w in geometry),f'isolated windows on {options.output} at {scale}')
            value=await measure();check(value['bodyWidth']<=value['viewport'][0],f'main layout fits at {scale}')
            await probe.js("document.getElementById('btn-settings').click();true")
            await probe.until("!!document.querySelector('#appearance-editor input')")
            dialog=json.loads(await probe.js("JSON.stringify(document.querySelector('.settings-dialog').getBoundingClientRect().toJSON())"))
            check(dialog['left']>=0 and dialog['right']<=value['viewport'][0]+1 and dialog['bottom']<=value['viewport'][1]+1,f'settings dialog fits at {scale}')
            await probe.js("document.getElementById('settings-overlay').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));true")
            revision=probe.call('get_theme',{})['theme']['config']['revision']
            p=probe.call('preview_theme',{'base_revision':revision,'patch':{},'scene':'article','mode':'light'})
            probe.position(); await asyncio.sleep(.5)
            p=probe.call('capture_theme_preview',{'preview_id':p['preview_id'],'expected_preview_revision':p['preview_revision']})
            check(p['capture']['display_backend']=='GdkWaylandDisplay',f'native Wayland capture at {scale}')
            image=Path(p['image_path']);check(image.exists() and image.stat().st_size==p['image_bytes'],f'PNG file readable at {scale}')
            report['scales'].append({'scale':scale,'geometry':geometry,'page':value,'dialog':dialog,'capture':p['capture']})
            probe.call('finish_theme_preview',{'preview_id':p['preview_id'],'expected_preview_revision':p['preview_revision'],'action':'cancel'})
        # Real global preference changes, observed by the existing WebView.
        for app_mode, desktop_scheme, expected in [('system','BreezeDark','dark'),('system','BreezeLight','light'),
                ('light','BreezeDark','light'),('dark','BreezeLight','dark')]:
            await mode(app_mode)
            before=probe.config(); before_page=await measure()
            run('plasma-apply-colorscheme',desktop_scheme)
            dark=desktop_scheme=='BreezeDark'
            await probe.until("matchMedia('(prefers-color-scheme: dark)').matches==="+str(dark).lower())
            await probe.until("document.documentElement.dataset.theme==="+json.dumps(expected))
            value=await measure()
            check(probe.config()==before,f'OS preference does not persist config: {app_mode}/{desktop_scheme}')
            check(value['articleSame'] and abs(value['paragraphOffset']-before_page['paragraphOffset'])<2,f'OS preference preserves paragraph: {app_mode}/{desktop_scheme}')
            report['modes'].append({'app_mode':app_mode,'desktop_scheme':desktop_scheme,'page':value})
        report['passed']=True
    except Exception as error:
        report['error']=repr(error)
        raise
    finally:
        errors=[]
        for cmd in [('kscreen-doctor',f"output.{options.output}.scale.{output['scale']}"),('plasma-apply-colorscheme',scheme)]:
            try: run(*cmd)
            except Exception as error: errors.append(repr(error))
        report['restore_errors']=errors
        report['restored_outputs']=json.loads(run('kscreen-doctor','-j'))['outputs']
        report['restored_scheme']=run('plasma-apply-colorscheme','--list-schemes')
        filename=f'native-clicks-{options.manual_scale}.json' if options.manual_scale else 'native-settings-results.json'
        (probe.root/filename).write_text(json.dumps(report,indent=2))
        print(json.dumps({'checks':report['checks'],'passed':report.get('passed',False),'error':report.get('error'),'restore_errors':errors,'output':str(probe.root)}),flush=True)
        assert not errors,errors


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('instance')
    parser.add_argument('--allow-desktop-changes',action='store_true')
    parser.add_argument('--output',default='HDMI-A-1')
    parser.add_argument('--manual-scale',type=float,choices=[1.25,1.5])
    asyncio.run(main(parser.parse_args()))
