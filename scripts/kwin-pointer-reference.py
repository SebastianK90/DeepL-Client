"""Independent native-test oracle: read the compositor, not XWayland's stale pointer."""
import json
from pathlib import Path
import tempfile
import uuid
from gi.repository import Gio, GLib


def current_pointer():
    connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    def call(service, path, interface, method, args=None):
        return connection.call_sync(service, path, interface, method, args, None,
                                    Gio.DBusCallFlags.NONE, 1000, None)
    owner = call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                 'org.freedesktop.DBus', 'GetNameOwner',
                 GLib.Variant('(s)', ('org.kde.KWin',))).unpack()[0]
    report = []
    loop = GLib.MainLoop()
    def method(conn, sender, path, interface, name, parameters, invocation):
        if sender != owner:
            invocation.return_dbus_error('dev.local.DeepLClient.Error', 'Unexpected sender')
            return
        report.append(json.loads(parameters.unpack()[0]))
        invocation.return_value(None)
        loop.quit()
    info = Gio.DBusNodeInfo.new_for_xml(
        '<node><interface name="dev.local.DeepLClient.PointerReference">'
        '<method name="Report"><arg type="s" direction="in"/></method></interface></node>')
    nonce = uuid.uuid4().hex
    path = '/dev/local/DeepLClient/PointerReference/p' + nonce
    name = 'deepl-pointer-reference-' + nonce
    registration = connection.register_object(path, info.interfaces[0], method, None, None)
    timer = None
    try:
        with tempfile.TemporaryDirectory(prefix='deepl-pointer-reference-') as directory:
            script = Path(directory) / 'pointer.js'
            script.write_text(
                'const p=workspace.cursorPos;const r={x:p.x,y:p.y,outputs:[]};'
                'for(const s of workspace.screens)r.outputs.push({name:s.name,x:s.geometry.x,'
                'y:s.geometry.y,width:s.geometry.width,height:s.geometry.height});callDBus('
                + ','.join(json.dumps(v) for v in [connection.get_unique_name(), path,
                    'dev.local.DeepLClient.PointerReference', 'Report']) + ',JSON.stringify(r));')
            script_id = call('org.kde.KWin', '/Scripting', 'org.kde.kwin.Scripting',
                             'loadScript', GLib.Variant('(ss)', (str(script), name))).unpack()[0]
            assert script_id >= 0
            call('org.kde.KWin', '/Scripting/Script' + str(script_id), 'org.kde.kwin.Script', 'run')
            timer = GLib.timeout_add(1000, lambda: (loop.quit(), False)[1])
            if not report:
                loop.run()
            assert report, 'No compositor pointer received'
            return report[0]
    finally:
        if timer is not None:
            GLib.source_remove(timer)
        call('org.kde.KWin', '/Scripting', 'org.kde.kwin.Scripting', 'unloadScript',
             GLib.Variant('(s)', (name,)))
        connection.unregister_object(registration)
