//! Typing on a nested wlroots compositor (sway, labwc) as a keyboard would:
//! its virtual keyboard (`zwp_virtual_keyboard_v1`), which also gives a
//! headless seat the keyboard it needs before any window is focused. (The
//! #2501 test's `keys.rs` presses Enter the same way.)

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

/// The letters the test types, with their evdev codes (xkb's keycode is
/// the evdev code plus 8).
const LETTERS: &[(char, u32)] = &[('e', 18), ('o', 24), ('h', 35), ('l', 38)];

fn keymap() -> String {
    let codes: String = LETTERS
        .iter()
        .map(|(c, code)| format!("<K{c}> = {};", code + 8))
        .collect();
    let symbols: String = LETTERS
        .iter()
        .map(|(c, _)| format!("key <K{c}> {{ [ {c} ] }};"))
        .collect();
    format!(
        "xkb_keymap {{
    xkb_keycodes \"rinch\" {{ minimum = 8; maximum = 255; {codes} }};
    xkb_types \"rinch\" {{ include \"complete\" }};
    xkb_compatibility \"rinch\" {{ include \"complete\" }};
    xkb_symbols \"rinch\" {{ {symbols} }};
}};
"
    )
}

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
    ZwpVirtualKeyboardV1
);

/// A virtual keyboard on the compositor whose socket is `socket`.
pub(crate) struct Keyboard {
    connection: Connection,
    queue: wayland_client::EventQueue<State>,
    keyboard: ZwpVirtualKeyboardV1,
    time: u32,
}

impl Keyboard {
    /// Connects to `socket` and makes the keyboard; `runtime` is the
    /// session's runtime directory, for the keymap's file.
    pub(crate) fn new(socket: &Path, runtime: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("connect to the compositor");
        let connection = Connection::from_socket(stream).expect("a Wayland connection");
        let (globals, mut queue) = registry_queue_init::<State>(&connection).expect("the registry");
        let qh = queue.handle();
        let manager: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .expect("zwp_virtual_keyboard_manager_v1 (a wlroots compositor)");
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("a seat");
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        let keymap = keymap();
        let path = runtime.join(format!("rinch-focus-race-keymap-{}", std::process::id()));
        let mut file = std::fs::File::create(&path).expect("the keymap file");
        file.write_all(keymap.as_bytes()).expect("write the keymap");
        file.write_all(&[0]).expect("write the keymap");
        let file = std::fs::File::open(&path).expect("open the keymap");
        let size = u32::try_from(keymap.len() + 1).expect("a small keymap");
        // `XKB_V1`.
        keyboard.keymap(1, file.as_fd(), size);
        queue.roundtrip(&mut State).expect("a roundtrip");
        Self {
            connection,
            queue,
            keyboard,
            time: 0,
        }
    }

    /// Press and release each letter of `text` in one burst: every event is
    /// sent in a single flush, so the compositor delivers them together, as
    /// a program typing ahead (or a fast typist on a loaded machine) does.
    pub(crate) fn type_burst(&mut self, text: &str) {
        for c in text.chars() {
            let code = LETTERS
                .iter()
                .find(|(l, _)| *l == c)
                .unwrap_or_else(|| panic!("no key for {c:?}"))
                .1;
            self.time += 1;
            self.keyboard.key(self.time, code, 1);
            self.time += 1;
            self.keyboard.key(self.time, code, 0);
        }
        self.connection.flush().expect("flush");
        self.queue.roundtrip(&mut State).expect("a roundtrip");
    }
}
