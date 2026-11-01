# Native regression fixture; requires the debug build, KDE/XWayland, xdotool and PyGObject/GTK.
from pathlib import Path
import os,subprocess,time,json,signal,importlib.util
# PyGObject can initialize GTK during import; choose X11 before importing it.
os.environ["GDK_BACKEND"]="x11"
import gi
gi.require_version("Gdk", "3.0")
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, Gtk
spec=importlib.util.spec_from_file_location("pointer_reference",Path(__file__).with_name("kwin-pointer-reference.py"));reference=importlib.util.module_from_spec(spec);spec.loader.exec_module(reference)
Gtk.init([])
root=Path.cwd();base=root/'.tools/quick-resize/placement-native';base.mkdir(exist_ok=True)
def xdo(*args):return subprocess.check_output(['xdotool',*map(str,args)],text=True,stderr=subprocess.DEVNULL).strip()
def fields(text):return{k:int(v) for k,v in (line.split('=',1) for line in text.splitlines()) if v.lstrip('-').isdigit()}
results=[]
for policy in ['centered','mouse_cursor','remember_last']:
 directory=base/policy;directory.mkdir(exist_ok=True)
 (directory/'reopen-quick.request').unlink(missing_ok=True)
 (directory/'config.toml').write_text('provider = "web"\nhistory_enabled = false\nquick_position_policy = "'+policy+'"\n[quick_position]\nx = 160\ny = 120\n')
 snapshot=reference.current_pointer()
 source=next(o for o in snapshot['outputs'] if o['x']<=snapshot['x']<o['x']+o['width'] and o['y']<=snapshot['y']<o['y']+o['height'])
 display=Gdk.Display.get_default();monitors=[display.get_monitor(i) for i in range(display.get_n_monitors())]
 monitor=next((m for m in monitors if m.get_model()==source['name']),monitors[0] if len(monitors)==1 else None)
 assert monitor is not None, 'Unmatched native output'
 rect=monitor.get_geometry();scale=monitor.get_scale_factor();work=monitor.get_workarea()
 cursor={'X':round(rect.x*scale+(snapshot['x']-source['x'])*rect.width*scale/source['width']), 'Y':round(rect.y*scale+(snapshot['y']-source['y'])*rect.height*scale/source['height'])}
 area=[work.x*scale,work.y*scale,work.width*scale,work.height*scale]
 with (directory/'process.log').open('w') as log:
  process=subprocess.Popen([str(root/'target/debug/deepl-client')],env={**os.environ,'DEEPL_CLIENT_TEST_DIR':str(directory),'DEEPL_CLIENT_UI_TEST':'1','DEEPL_CLIENT_PLACEMENT_TEST':'1'},stdout=log,stderr=log,start_new_session=True)
  try:
   time.sleep(3)
   assert process.poll() is None
   ids=xdo('search','--pid',process.pid).splitlines();names={xdo('getwindowname',xid):xid for xid in ids}
   quick=names['Quick Assist'];geometry=fields(xdo('getwindowgeometry','--shell',quick))
   if policy=='centered':expected=(area[0]+(area[2]-geometry['WIDTH'])//2,area[1]+(area[3]-geometry['HEIGHT'])//2)
   elif policy=='remember_last':expected=(160,120)
   else:expected=(min(max(cursor['X'],area[0]),area[0]+area[2]-geometry['WIDTH']),min(max(cursor['Y']+12*scale,area[1]),area[1]+area[3]-geometry['HEIGHT']))
   result={'policy':policy,'cursor_before_show':cursor,'geometry':geometry,'expected':expected,'error':[geometry['X']-expected[0],geometry['Y']-expected[1]]};results.append(result)
   print(json.dumps(result),flush=True)
   if policy=='centered':
    xdo('windowmove',quick,160,120);time.sleep(.3)
    (directory/'reopen-quick.request').write_text('1');time.sleep(.6)
    reopened=fields(xdo('getwindowgeometry','--shell',quick));print('reopened centered',reopened,flush=True)
    assert abs(reopened['X']-expected[0])<=2 and abs(reopened['Y']-expected[1])<=2, reopened
    xdo('windowsize',quick,900,600);xdo('windowmove',quick,160,120);time.sleep(.4)
    (directory/'reopen-quick.request').write_text('2');time.sleep(.6)
    resized=fields(xdo('getwindowgeometry','--shell',quick));print('resized and reopened centered',resized,flush=True)
    assert abs(resized['X']-(area[0]+(area[2]-resized['WIDTH'])//2))<=2
    assert abs(resized['Y']-(area[1]+(area[3]-resized['HEIGHT'])//2))<=2
  finally:
   os.killpg(process.pid,signal.SIGTERM);process.wait(timeout=5)
(base/'result.json').write_text(json.dumps(results,indent=2)+'\n')
assert all(abs(value)<=2 for r in results for value in r['error']),results
print('Native Quick Assist placement passed: centered reopen, resized center, pointer alignment and remembered position.',flush=True)
