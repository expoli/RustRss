import subprocess,time,re,hashlib,json,sys
from pathlib import Path
import xml.etree.ElementTree as ET
serial='emulator-5580';pkg='tech.expoli.rustrss';out=Path('/tmp/rustrss-nav-signed');out.mkdir(exist_ok=True)
def run(*a):return subprocess.check_output(['adb','-s',serial,*a],timeout=30)
def tree():
 run('shell','uiautomator','dump','/data/local/tmp/nav-signed.xml');return ET.fromstring(run('shell','cat','/data/local/tmp/nav-signed.xml'))
def tap(rid=None,text=None):
 aliases={'Articles':{'Articles','文章'},'Subscriptions':{'Subscriptions','订阅'},'Saved':{'Saved','收藏'},'Settings':{'Settings','设置'}}.get(text,{text})
 for _ in range(8):
  n=next((n for n in tree().iter('node') if n.get('resource-id')==rid if rid),None) if rid else next((n for n in tree().iter('node') if n.get('text') in aliases),None)
  if n is not None:
   x1,y1,x2,y2=map(int,re.findall(r'\d+',n.get('bounds')));run('shell','input','tap',str((x1+x2)//2),str((y1+y2)//2));time.sleep(.3);return
  time.sleep(.2)
 raise AssertionError((rid,text))
def snapshot(name):
 t=tree();(out/(name+'.xml')).write_bytes(ET.tostring(t));(out/(name+'.png')).write_bytes(run('exec-out','screencap','-p'));return t
def checkNav(name):
 t=snapshot(name);web=next(n for n in t.iter('node') if n.get('class')=='android.webkit.WebView');w=map(int,re.findall(r'\d+',web.get('bounds')));wx1,wy1,wx2,wy2=w
 found={}
 for label,alts in [('Articles',{'Articles','文章'}),('Subscriptions',{'Subscriptions','订阅'}),('Saved',{'Saved','收藏'}),('Settings',{'Settings','设置'})]:
  n=next(n for n in t.iter('node') if n.get('text') in alts and int(re.findall(r'\d+',n.get('bounds'))[1])>wy2-250)
  x1,y1,x2,y2=map(int,re.findall(r'\d+',n.get('bounds')));assert wx1<=x1<x2<=wx2 and wy1<=y1<y2<=wy2;found[label]=[x1,y1,x2,y2]
 return {'webview':[wx1,wy1,wx2,wy2],'labels':found}
def uid():return re.search(r'uid:(\d+)',run('shell','pm','list','packages','-U',pkg).decode()).group(1)
oldUid=uid();path=run('shell','pm','path',pkg).decode().strip().removeprefix('package:');oldHash=hashlib.sha256(run('exec-out','cat',path)).hexdigest();assert oldHash=='b0e9474f7364b3713195e080ed87b99fb0cf29e6e09345e3aef6394076a8b6c1'
tap(text='Subscriptions');assert any(n.get('text','').startswith('Theme fixture') for n in tree().iter('node'));snapshot('published-021-fixture')
run('shell','am','force-stop',pkg);print(run('install','-r','/tmp/rustrss-android-primary-navigation/RustRss_0.2.1_android-navigation-preview.apk').decode(),flush=True);assert uid()==oldUid
run('shell','am','start','-n',pkg+'/.MainActivity');time.sleep(1)
checks={}
for page in ['Articles','Subscriptions','Saved','Settings']:
 tap(text=page);checks[page]=checkNav('preview-'+page.lower())
tap(rid='tab-reading');checks['Reading']=checkNav('preview-reading')
# Actual edge gestures under gestural system navigation.
run('shell','input','swipe','0','1200','500','1200','350');time.sleep(.6)
assert any(n.get('resource-id')=='tab-reading' for n in tree().iter('node'));checks['gestureToHome']=checkNav('preview-gesture-home')
run('shell','input','swipe','0','1200','500','1200','350');time.sleep(.6)
assert not any(n.get('resource-id')=='tab-reading' for n in tree().iter('node'));checks['gestureToSaved']=checkNav('preview-gesture-origin')
tap(text='Subscriptions');assert any(n.get('text','').startswith('Theme fixture') for n in tree().iter('node'));tap(text='Settings')
meta=run('shell','dumpsys','package',pkg).decode();assert 'versionCode=2001' in meta and 'versionName=0.2.1' in meta
result={'published_021_sha256':oldHash,'same_uid':True,'uid':oldUid,'fixture_retained':True,'physical_edge_gestures':True,'checks':checks}
(out/'results.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result),flush=True)
