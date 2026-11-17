//! Clicking on a headless wlroots compositor, as a mouse would: wlroots'
//! virtual pointer (`zwlr_virtual_pointer_v1`), which gives a seat with no
//! devices a pointer. The compositor sends the press to the surface under
//! the pointer that takes input there, which is what an input region
//! decides.

use std::os::unix::net::UnixStream;
use std::path::Path;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// The left button's evdev code.
const BTN_LEFT: u32 = 0x110;

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
    ZwlrVirtualPointerManagerV1,
    ZwlrVirtualPointerV1
);

/// A pointer on the compositor whose socket is `socket`, over an output
/// `extent` (logical px) wide and high.
pub(crate) struct Pointer {
    connection: Connection,
    queue: wayland_client::EventQueue<State>,
    pointer: ZwlrVirtualPointerV1,
    extent: (u32, u32),
    time: u32,
}

impl Pointer {
    pub(crate) fn new(socket: &Path, extent: (u32, u32)) -> Self {
        let stream = UnixStream::connect(socket).expect("connect to the compositor");
        let connection = Connection::from_socket(stream).expect("a Wayland connection");
        let (globals, mut queue) = registry_queue_init::<State>(&connection).expect("the registry");
        let qh = queue.handle();
        let manager: ZwlrVirtualPointerManagerV1 = globals
            .bind(&qh, 1..=2, ())
            .expect("zwlr_virtual_pointer_manager_v1 (a wlroots compositor)");
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("a seat");
        let pointer = manager.create_virtual_pointer(Some(&seat), &qh, ());
        queue.roundtrip(&mut State).expect("a roundtrip");
        Self {
            connection,
            queue,
            pointer,
            extent,
            time: 0,
        }
    }

    fn tick(&mut self) -> u32 {
        self.time += 10;
        self.time
    }

    /// Move to `(x, y)` (logical px on the output) and click the left button.
    pub(crate) fn click(&mut self, x: u32, y: u32) {
        let t = self.tick();
        self.pointer
            .motion_absolute(t, x, y, self.extent.0, self.extent.1);
        self.pointer.frame();
        self.flush();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let t = self.tick();
        self.pointer
            .button(t, BTN_LEFT, wl_pointer::ButtonState::Pressed);
        self.pointer.frame();
        let t = self.tick();
        self.pointer
            .button(t, BTN_LEFT, wl_pointer::ButtonState::Released);
        self.pointer.frame();
        self.flush();
    }

    fn flush(&mut self) {
        self.connection.flush().expect("flush");
        self.queue.roundtrip(&mut State).expect("a roundtrip");
    }
}
