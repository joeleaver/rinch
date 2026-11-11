//! Pressing Enter on a nested compositor, as a keyboard would: wlroots'
//! virtual keyboard (`zwp_virtual_keyboard_v1`, which gives a headless seat
//! a keyboard), or KWin's fake input (`org_kde_kwin_fake_input`, a
//! restricted interface: KWin must run with
//! `KWIN_WAYLAND_NO_PERMISSION_CHECKS=1`). The compositor sends the press to
//! the focused surface as a `wl_keyboard.key` with a serial, which is what
//! an activation token request names.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::Path;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_plasma::fake_input::client::org_kde_kwin_fake_input::OrgKdeKwinFakeInput;

/// Enter's evdev code.
const KEY_ENTER: u32 = 28;

/// The keymap the virtual keyboard brings: Enter, on the standard types and
/// compatibility rules.
const KEYMAP: &str = "xkb_keymap {
    xkb_keycodes \"rinch\" { minimum = 8; maximum = 255; <RTRN> = 36; };
    xkb_types \"rinch\" { include \"complete\" };
    xkb_compatibility \"rinch\" { include \"complete\" };
    xkb_symbols \"rinch\" { key <RTRN> { [ Return ] }; };
};
";

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The proxies here send requests only; their events are not needed.
macro_rules! ignore_events {
    ($($proxy:ty),*) => {$(
        impl Dispatch<$proxy, ()> for State {
            fn event(
                _: &mut Self,
                _: &$proxy,
                _: <$proxy as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(
    wl_seat::WlSeat,
    ZwpVirtualKeyboardManagerV1,
    ZwpVirtualKeyboardV1,
    OrgKdeKwinFakeInput
);

/// A keyboard on the compositor whose socket is `socket`, made before the
/// windows it will press keys in, so the seat has a keyboard when they map.
pub(crate) struct Keyboard {
    connection: Connection,
    queue: wayland_client::EventQueue<State>,
    kind: Kind,
}

enum Kind {
    Virtual(ZwpVirtualKeyboardV1),
    Fake(OrgKdeKwinFakeInput),
}

impl Keyboard {
    /// Connects to `socket` and makes the keyboard; `runtime` is the
    /// session's runtime directory, for the keymap's file.
    pub(crate) fn new(socket: &Path, runtime: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("connect to the compositor");
        let connection = Connection::from_socket(stream).expect("a Wayland connection");
        let (globals, mut queue) = registry_queue_init::<State>(&connection).expect("the registry");
        let qh = queue.handle();
        let kind = if let Ok(manager) =
            globals.bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ())
        {
            let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("a seat");
            let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
            let path = runtime.join("rinch-test-keymap");
            let mut file = std::fs::File::create(&path).expect("the keymap file");
            file.write_all(KEYMAP.as_bytes()).expect("write the keymap");
            file.write_all(&[0]).expect("write the keymap");
            let file = std::fs::File::open(&path).expect("open the keymap");
            let size = u32::try_from(KEYMAP.len() + 1).expect("a small keymap");
            // `XKB_V1`.
            keyboard.keymap(1, file.as_fd(), size);
            Kind::Virtual(keyboard)
        } else {
            let fake: OrgKdeKwinFakeInput = globals.bind(&qh, 4..=5, ()).expect(
                "zwp_virtual_keyboard_manager_v1 or org_kde_kwin_fake_input \
                 (KWin: KWIN_WAYLAND_NO_PERMISSION_CHECKS=1)",
            );
            fake.authenticate("rinch activation token test".into(), "press Enter".into());
            Kind::Fake(fake)
        };
        queue.roundtrip(&mut State).expect("a roundtrip");
        Self {
            connection,
            queue,
            kind,
        }
    }

    /// Press and release Enter.
    pub(crate) fn press_enter(&mut self) {
        match &self.kind {
            Kind::Virtual(keyboard) => {
                keyboard.key(0, KEY_ENTER, 1);
                keyboard.key(1, KEY_ENTER, 0);
            }
            Kind::Fake(fake) => {
                fake.keyboard_key(KEY_ENTER, 1);
                fake.keyboard_key(KEY_ENTER, 0);
            }
        }
        self.connection.flush().expect("flush");
        self.queue.roundtrip(&mut State).expect("a roundtrip");
    }
}
