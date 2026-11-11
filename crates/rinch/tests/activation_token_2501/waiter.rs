//! The program a token is for, already running: a plain Wayland toplevel
//! (an empty 64x64 buffer) that asks the compositor to activate it with a
//! token when told, and notes whether its window has the keyboard.
//!
//! An already-mapped window is the case where a token decides anything on
//! the compositors tested: KWin and sway both focus a *new* window when it
//! maps, token or not, while an activation of a window that is already
//! there is granted only for a token the compositor trusts.

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const SIZE: i32 = 64;

/// The waiter's window, on a thread of its own.
pub(crate) struct Waiter {
    connection: Connection,
    surface: wl_surface::WlSurface,
    activation: xdg_activation_v1::XdgActivationV1,
    qh: QueueHandle<State>,
    focused: Arc<AtomicBool>,
    mapped: Arc<AtomicBool>,
}

struct State {
    surface: wl_surface::WlSurface,
    buffer: wl_buffer::WlBuffer,
    activation: xdg_activation_v1::XdgActivationV1,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    focused: Arc<AtomicBool>,
    mapped: Arc<AtomicBool>,
}

impl Waiter {
    /// Maps the waiter's window on the compositor at `socket`; `runtime` holds
    /// its buffer's file.
    pub(crate) fn start(socket: &Path, runtime: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("connect to the compositor");
        let connection = Connection::from_socket(stream).expect("a Wayland connection");
        let (globals, mut queue) = registry_queue_init::<State>(&connection).expect("registry");
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor =
            globals.bind(&qh, 4..=6, ()).expect("wl_compositor");
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
        let wm_base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
        let activation: xdg_activation_v1::XdgActivationV1 =
            globals.bind(&qh, 1..=1, ()).expect("xdg_activation_v1");
        let _seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("wl_seat");

        let path = runtime.join("rinch-test-waiter-buffer");
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("the buffer's file");
        let bytes = SIZE * SIZE * 4;
        file.set_len(u64::try_from(bytes).expect("a small buffer"))
            .expect("size the buffer");
        let pool = shm.create_pool(file.as_fd(), bytes, &qh, ());
        let buffer = pool.create_buffer(0, SIZE, SIZE, SIZE * 4, wl_shm::Format::Argb8888, &qh, ());

        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg_surface.get_toplevel(&qh, ());
        toplevel.set_app_id("waiter".into());
        toplevel.set_title("waiter".into());
        surface.commit();

        let focused = Arc::new(AtomicBool::new(false));
        let mapped = Arc::new(AtomicBool::new(false));
        let mut state = State {
            surface: surface.clone(),
            buffer,
            activation: activation.clone(),
            keyboard: None,
            focused: Arc::clone(&focused),
            mapped: Arc::clone(&mapped),
        };
        queue.roundtrip(&mut state).expect("a roundtrip");
        std::thread::spawn(move || {
            // Kept alive with the queue: dropping them would end the window.
            let _alive = (xdg_surface, toplevel, pool);
            while queue.blocking_dispatch(&mut state).is_ok() {}
        });
        Self {
            connection,
            surface,
            activation,
            qh,
            focused,
            mapped,
        }
    }

    /// Whether the window has been shown.
    pub(crate) fn mapped(&self) -> bool {
        self.mapped.load(Ordering::SeqCst)
    }

    /// Whether the window has the keyboard.
    pub(crate) fn focused(&self) -> bool {
        self.focused.load(Ordering::SeqCst)
    }

    /// Ask to be activated with `token`, as a program given one does.
    pub(crate) fn activate_with(&self, token: &str) {
        self.activation.activate(token.into(), &self.surface);
        self.connection.flush().expect("flush");
    }

    /// Ask to be activated with a token of its own, as a program given none
    /// can: requested for its own surface, which does not have the focus,
    /// with no input serial. The activation follows the compositor's answer.
    pub(crate) fn activate_with_own_token(&self) {
        let token = self.activation.get_activation_token(&self.qh, ());
        token.set_surface(&self.surface);
        token.commit();
        self.connection.flush().expect("flush");
    }
}

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

/// Proxies whose events the waiter does not need.
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
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    xdg_toplevel::XdgToplevel,
    xdg_activation_v1::XdgActivationV1
);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.surface.attach(Some(&state.buffer), 0, 0);
            state.surface.damage_buffer(0, 0, SIZE, SIZE);
            state.surface.commit();
            state.mapped.store(true, Ordering::SeqCst);
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => state.focused.store(true, Ordering::SeqCst),
            wl_keyboard::Event::Leave { .. } => state.focused.store(false, Ordering::SeqCst),
            _ => {}
        }
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for State {
    fn event(
        state: &mut Self,
        token: &xdg_activation_token_v1::XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token: text } = event {
            state.activation.activate(text, &state.surface);
            token.destroy();
        }
    }
}
