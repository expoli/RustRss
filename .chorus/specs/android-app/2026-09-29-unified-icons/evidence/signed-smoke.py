import pathlib,subprocess,time,xml.etree.ElementTree as ET,re,json,tempfile,sys,shutil
adb='/usr/lib/android-sdk/platform-tools/adb';serial='emulator-5580';pkg='tech.expoli.rustrss'
old='/tmp/rustrss-android-opml-fix/RustRss_0.2.0_android-opml-fix-preview.apk';new='/tmp/rustrss-android-unified-icons/RustRss_0.2.0_android-unified-icons-preview.apk'
out=pathlib.Path(tempfile.mkdtemp(prefix='rustrss-icons-signed-'))
def run(*a):return subprocess.check_output([adb,'-s',serial,*a],timeout=30)
def tree():
 for _ in range(5):
  try:dumped=run('shell','uiautomator','dump','/data/local/tmp/opml-signed.xml').decode()
  except subprocess.CalledProcessError as e:
   if e.returncode!=137:raise
   print('Retry native UI dump after process termination',flush=True);time.sleep(.5);continue
  if 'dumped to:' in dumped:return ET.fromstring(run('shell','cat','/data/local/tmp/opml-signed.xml'))
  time.sleep(.2)
 raise AssertionError('No native UI root')
def touch(n):
 x1,y1,x2,y2=map(int,re.findall(r'\d+',n.get('bounds')));assert x2>x1 and y2>y1
 run('shell','input','tap',str((x1+x2)//2),str((y1+y2)//2));time.sleep(.4)
def tap(rid=None,text=None,desc=None):
 labels={'Settings':{'Settings','设置'},'Subscriptions':{'Subscriptions','订阅'}}.get(text,{text})
 for _ in range(5):
  nodes=list(tree().iter('node')); n=next((n for n in nodes if (n.get('resource-id')==rid if rid else n.get('content-desc')==desc if desc else n.get('text') in labels)),None)
  if n is not None:touch(n);return
  time.sleep(.2)
 raise AssertionError((rid,text,desc))
def screenshot(name):(out/(name+'.png')).write_bytes(run('exec-out','screencap','-p'))
def uid():return re.search(r'uid:(\d+)',run('shell','pm','list','packages','-U',pkg).decode()).group(1)
def launch():run('shell','am','start','-n',pkg+'/.MainActivity');time.sleep(2)
def importFile(name,before=False):
 tap(text='Settings');tap(rid='tab-data');tap(rid='act-import-opml')
 for _ in range(8):
  nodes=list(tree().iter('node'))
  download=next((n for n in nodes if n.get('resource-id')=='android:id/title' and n.get('text') in {'Downloads','下载'}),None)
  if download is not None:touch(download);break
  menu=next((n for n in nodes if n.get('content-desc') in {'Show roots','显示根目录'}),None)
  if menu is not None:touch(menu)
 else:raise AssertionError('Downloads root unavailable')
 nodes=list(tree().iter('node'));listview=next((n for n in nodes if n.get('content-desc') in {'List view','列表视图'}),None)
 if listview is not None:touch(listview)
 nodes=list(tree().iter('node'));n=next(n for n in nodes if n.get('text')==name);assert n.get('enabled')=='true'
 if before:
  generic=next(n for n in nodes if n.get('text')=='RustRss-generic.opml');assert generic.get('enabled')=='false';screenshot('signed-prior-opml-disabled')
 else:screenshot('signed-fixed-opml-selectable')
 touch(n)
 for _ in range(15):
  current=run('shell','dumpsys','activity','activities').decode()
  top=next(l for l in current.splitlines() if 'topResumedActivity' in l)
  if pkg in top:break
  time.sleep(.2)
 else:raise AssertionError('Selection did not return to RustRss')
 tap(text='Subscriptions')
def hasFeed(title):
 for _ in range(10):
  if any(n.get('text')==title or n.get('text','').startswith(title+' ') for n in tree().iter('node')):return True
  time.sleep(.2)
 return False

def launcher(name):
 run('shell','input','keyevent','KEYCODE_HOME');time.sleep(.5)
 run('shell','input','swipe','540','2200','540','600','400');time.sleep(.6)
 for _ in range(8):
  nodes=list(tree().iter('node'))
  n=next((n for n in nodes if n.get('text')=='RustRss'),None)
  if n is not None:
   screenshot(name)
   (out/(name+'.xml')).write_bytes(ET.tostring(tree(),encoding='utf-8'))
   touch(n);time.sleep(1)
   assert pkg in run('shell','dumpsys','activity','activities').decode()
   return
  run('shell','input','swipe','540','1800','540','700','350');time.sleep(.4)
 raise AssertionError('RustRss launcher entry unavailable')
print(run('uninstall',pkg).decode().strip(),flush=True)
print(run('install',old).decode().strip(),flush=True);launch();oldUid=uid()
importFile('RustRss-xml.xml');assert hasFeed('Picker xml');screenshot('prior-signed-subscription')
launcher('prior-launcher')
run('shell','am','force-stop',pkg);print(run('install','-r',new).decode().strip(),flush=True)
newUid=uid();assert oldUid==newUid
launcher('unified-launcher')
tap(text='Subscriptions');assert hasFeed('Picker xml');screenshot('retained-subscription')
importFile('RustRss-generic.opml');assert hasFeed('Picker generic') and hasFeed('Picker xml');screenshot('retained-opml-import')
tap(text='Settings');assert any(n.get('resource-id')=='tab-reading' for n in tree().iter('node'));screenshot('retained-settings-home')
result={'serial':serial,'same_uid':oldUid==newUid,'prior_xml_feed_retained':True,'opml_fix_retained':True,'phone_settings_retained':True,'launched_from_new_launcher':True}
(out/'results.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'out':str(out),'result':result}),flush=True)
