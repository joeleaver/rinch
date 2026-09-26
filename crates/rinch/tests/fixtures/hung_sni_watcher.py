# A StatusNotifierWatcher that answers the first RegisterStatusNotifierItem,
# then gives up its name and takes it back, and never answers again: a hung
# watcher. ksni re-registers on the owner change and waits on that call with
# no timeout, where it cannot see a shutdown request (#377, review of #1054).
# Used by tray::tests::live_dropping_a_tray_under_a_hung_watcher_is_bounded;
# needs python3-gi. See that test for the recipe.
#
# With --hang-first it answers no RegisterStatusNotifierItem at all, so the
# very first registration ksni makes, inside TrayIconBuilder::build(), hangs
# (#1057). Used by tray::tests::live_building_a_tray_under_a_hung_watcher_is_bounded.
import sys
from gi.repository import Gio, GLib
XML = """<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
<signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
<signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
<signal name="StatusNotifierHostRegistered"/>
</interface></node>"""
HANG_FIRST = "--hang-first" in sys.argv[1:]
node = Gio.DBusNodeInfo.new_for_xml(XML)
conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
state = {"calls": 0, "owner": None}
pending = []
def method(c, sender, path, iface, name, params, inv):
    if name == "RegisterStatusNotifierItem":
        state["calls"] += 1
        print("register call", state["calls"], flush=True)
        if state["calls"] == 1 and not HANG_FIRST:
            inv.return_value(None)
            GLib.timeout_add(300, cycle)
        else:
            pending.append(inv)  # never answer: a hung watcher
    else:
        inv.return_value(None)
def prop(c, sender, path, iface, name):
    return {"RegisteredStatusNotifierItems": GLib.Variant("as", []),
            "IsStatusNotifierHostRegistered": GLib.Variant("b", True),
            "ProtocolVersion": GLib.Variant("i", 0)}[name]
conn.register_object("/StatusNotifierWatcher", node.interfaces[0], method, prop, None)
def own():
    state["owner"] = Gio.bus_own_name_on_connection(conn, "org.kde.StatusNotifierWatcher", 0, None, None)
def cycle():
    print("cycling name", flush=True)
    Gio.bus_unown_name(state["owner"])
    GLib.timeout_add(200, lambda: (own(), False)[1])
    return False
own()
print("ready", flush=True)
GLib.MainLoop().run()
