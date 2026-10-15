# A fake org.freedesktop.portal.GlobalShortcuts (plus the host Registry), for
# hotkey::backend::portal::tests::the_portal_releases_a_held_binding_across_changes.
# Run it ONLY on a private bus (dbus-launch); it claims the portal's name.
# Argument `refuse`: refuse a BindShortcuts on an already bound session, so the
# backend must move to a new session (default: accept the re-bind).
# Control: org.example.FakeControl.Key(id, down) emits Activated/Deactivated on
# the session a hold started on; a release on a closed session is dropped, as a
# real portal drops it. Needs python3-dbus and python3-gi. Written by #1530's
# review.
import sys, dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
REFUSE_REBIND = len(sys.argv) > 1 and sys.argv[1] == "refuse"
BUS = dbus.SessionBus()
NAME = dbus.service.BusName("org.freedesktop.portal.Desktop", BUS)
PATH = "/org/freedesktop/portal/desktop"
GS = "org.freedesktop.portal.GlobalShortcuts"
log = lambda *a: (print("FAKE", *a, flush=True))

class Request(dbus.service.Object):
    @dbus.service.signal("org.freedesktop.portal.Request", signature="ua{sv}")
    def Response(self, code, results): pass

class Session(dbus.service.Object):
    def __init__(self, path, portal):
        super().__init__(BUS, path); self.path = path; self.portal = portal; self.bound = False
    @dbus.service.method("org.freedesktop.portal.Session", in_signature="", out_signature="")
    def Close(self):
        log("close", self.path); self.portal.closed.add(self.path)

class Portal(dbus.service.Object):
    def __init__(self):
        super().__init__(BUS, PATH)
        self.n = 0; self.sessions = {}; self.closed = set(); self.binds = 0; self.last = None
    def _req(self, sender, token):
        p = "%s/request/%s/%s" % (PATH, sender[1:].replace(".", "_"), token)
        return Request(BUS, p), p
    @dbus.service.method("org.freedesktop.host.portal.Registry", in_signature="sa{sv}", out_signature="")
    def Register(self, app_id, opts):
        log("register", app_id)
    @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="ss", out_signature="v")
    def Get(self, iface, prop):
        return dbus.UInt32(1, variant_level=1)
    @dbus.service.method(GS, in_signature="a{sv}", out_signature="o", sender_keyword="sender")
    def CreateSession(self, opts, sender=None):
        req, p = self._req(sender, opts["handle_token"])
        self.n += 1
        sp = "%s/session/%s/s%d" % (PATH, sender[1:].replace(".", "_"), self.n)
        self.sessions[sp] = Session(sp, self); self.last = sp
        log("create", sp)
        GLib.idle_add(lambda: (req.Response(dbus.UInt32(0), {"session_handle": dbus.String(sp, variant_level=1)}), False)[1])
        return dbus.ObjectPath(p)
    @dbus.service.method(GS, in_signature="oa(sa{sv})sa{sv}", out_signature="o", sender_keyword="sender")
    def BindShortcuts(self, session, shortcuts, parent, opts, sender=None):
        req, p = self._req(sender, opts["handle_token"])
        s = self.sessions[str(session)]
        self.binds += 1
        log("bind", str(session), [str(i) for i, _ in shortcuts], "binds=%d" % self.binds)
        if s.bound and REFUSE_REBIND:
            GLib.idle_add(lambda: (req.Response(dbus.UInt32(2), {}), False)[1]); return dbus.ObjectPath(p)
        s.bound = True
        res = dbus.Array([dbus.Struct((dbus.String(i), dbus.Dictionary({"trigger_description": dbus.String("T-" + str(i), variant_level=1)}, signature="sv"))) for i, _ in shortcuts], signature="(sa{sv})")
        GLib.idle_add(lambda: (req.Response(dbus.UInt32(0), {"shortcuts": dbus.Array(res, signature="(sa{sv})", variant_level=1)}), False)[1])
        return dbus.ObjectPath(p)
    @dbus.service.method(GS, in_signature="osa{sv}", out_signature="")
    def ConfigureShortcuts(self, session, parent, opts): log("configure")
    @dbus.service.signal(GS, signature="osta{sv}")
    def Activated(self, session, sid, ts, opts): pass
    @dbus.service.signal(GS, signature="osta{sv}")
    def Deactivated(self, session, sid, ts, opts): pass
    @dbus.service.signal(GS, signature="oa(sa{sv})")
    def ShortcutsChanged(self, session, shortcuts): pass
    # test control: press/release on the newest open session that the app has
    @dbus.service.method("org.example.FakeControl", in_signature="sb", out_signature="s")
    def Key(self, sid, down):
        live = [s for s in self.sessions if s not in self.closed]
        sp = self.last if self.last not in self.closed else (live[-1] if live else self.last)
        # a real portal ends a hold on the session the hold started on:
        sp = getattr(self, "held_on", {}).get(sid, sp) if not down else sp
        if down:
            self.held_on = getattr(self, "held_on", {}); self.held_on[sid] = sp
            self.Activated(dbus.ObjectPath(sp), sid, dbus.UInt64(0), {})
        else:
            if sp in self.closed: log("release on closed session: dropped"); return "dropped"
            self.Deactivated(dbus.ObjectPath(sp), sid, dbus.UInt64(0), {})
        log("key", sid, down, sp); return sp
    @dbus.service.method("org.example.FakeControl", in_signature="", out_signature="u")
    def Binds(self): return self.binds

Portal(); log("ready", "refuse" if REFUSE_REBIND else "rebind-ok")
GLib.MainLoop().run()
