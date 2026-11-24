//! A pointer and keyboard for a nested compositor (#2552): wlroots' virtual
//! devices (`zwlr_virtual_pointer_v1`, `zwp_virtual_keyboard_v1`) on sway
//! and labwc, KWin's `org_kde_kwin_fake_input` on KWin. KWin offers fake
//! input only to an executable an installed desktop file grants it to
//! (`X-KDE-Wayland-Interfaces=org_kde_kwin_fake_input`, `Exec` this
//! binary's absolute path), installed in the session's
//! `$XDG_DATA_HOME/applications` before KWin starts:
//! `scripts/popup-measure.sh` does that.
//!
//! Steps, from the arguments or from stdin one line at a time (the devices
//! stay connected between lines): `move X Y` (absolute, in the output's
//! logical pixels, `POPUP_INPUT_W` x `POPUP_INPUT_H`, 1280 x 720 by
//! default), `press`, `release`, `click` (the left button), `key CODE` (an
//! evdev code, pressed and released) and `sleep MS`.

use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use wayland_client::protocol::wl_pointer::ButtonState;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_plasma::fake_input::client::org_kde_kwin_fake_input::OrgKdeKwinFakeInput;
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

const BTN_LEFT: u32 = 0x110;

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    ptr_mgr: Option<ZwlrVirtualPointerManagerV1>,
    kbd_mgr: Option<ZwpVirtualKeyboardManagerV1>,
    fake: Option<OrgKdeKwinFakeInput>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        s: &mut Self,
        r: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = e
        {
            match interface.as_str() {
                "wl_seat" => s.seat = Some(r.bind(name, 1, qh, ())),
                "zwlr_virtual_pointer_manager_v1" => s.ptr_mgr = Some(r.bind(name, 1, qh, ())),
                "zwp_virtual_keyboard_manager_v1" => s.kbd_mgr = Some(r.bind(name, 1, qh, ())),
                // `keyboard_key` came with version 4.
                "org_kde_kwin_fake_input" if version >= 4 => {
                    s.fake = Some(r.bind(name, 4, qh, ()))
                }
                _ => {}
            }
        }
    }
}
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ZwlrVirtualPointerV1);
delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardV1);
delegate_noop!(State: OrgKdeKwinFakeInput);

const KEYMAP: &str = "xkb_keymap { xkb_keycodes { include \"evdev+aliases(qwerty)\" }; \
xkb_types { include \"complete\" }; xkb_compat { include \"complete\" }; \
xkb_symbols { include \"pc+us+inet(evdev)\" }; };\0";

enum Devices {
    Wlroots {
        pointer: ZwlrVirtualPointerV1,
        keyboard: Option<ZwpVirtualKeyboardV1>,
    },
    Kwin(OrgKdeKwinFakeInput),
}

struct Input {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    devices: Devices,
    start: Instant,
    size: (u32, u32),
}

impl Input {
    fn connect() -> Self {
        let conn = Connection::connect_to_env().expect("no Wayland display");
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());
        let mut state = State::default();
        queue.roundtrip(&mut state).unwrap();
        let devices = if let Some(mgr) = &state.ptr_mgr {
            let seat = state.seat.clone().expect("a seat");
            let pointer = mgr.create_virtual_pointer(Some(&seat), &qh, ());
            let keyboard = state.kbd_mgr.as_ref().map(|m| {
                let k = m.create_virtual_keyboard(&seat, &qh, ());
                let fd = rustix::fs::memfd_create("keymap", rustix::fs::MemfdFlags::CLOEXEC)
                    .expect("memfd");
                let mut f = std::fs::File::from(fd);
                f.write_all(KEYMAP.as_bytes()).unwrap();
                k.keymap(1, f.as_fd(), KEYMAP.len() as u32);
                k
            });
            Devices::Wlroots { pointer, keyboard }
        } else if let Some(fake) = state.fake.clone() {
            fake.authenticate("rinch popup-input".into(), "drive popups in tests".into());
            Devices::Kwin(fake)
        } else {
            panic!(
                "no virtual pointer and no fake input: on KWin, is this binary granted \
                 org_kde_kwin_fake_input by a desktop file?"
            );
        };
        queue.roundtrip(&mut state).unwrap();
        let var = |k: &str, d: u32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
        Self {
            conn,
            queue,
            state,
            devices,
            start: Instant::now(),
            size: (var("POPUP_INPUT_W", 1280), var("POPUP_INPUT_H", 720)),
        }
    }

    fn now(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn flush(&mut self) {
        self.conn.flush().unwrap();
    }

    fn motion(&mut self, x: u32, y: u32) {
        let t = self.now();
        match &self.devices {
            Devices::Wlroots { pointer, .. } => {
                pointer.motion_absolute(t, x, y, self.size.0, self.size.1);
                pointer.frame();
            }
            Devices::Kwin(fake) => fake.pointer_motion_absolute(f64::from(x), f64::from(y)),
        }
    }

    fn button(&mut self, pressed: bool) {
        let t = self.now();
        match &self.devices {
            Devices::Wlroots { pointer, .. } => {
                let state = if pressed {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                };
                pointer.button(t, BTN_LEFT, state);
                pointer.frame();
            }
            Devices::Kwin(fake) => fake.button(BTN_LEFT, u32::from(pressed)),
        }
    }

    fn key(&mut self, code: u32, pressed: bool) {
        let t = self.now();
        match &self.devices {
            Devices::Wlroots { keyboard, .. } => {
                if let Some(k) = keyboard {
                    k.key(t, code, u32::from(pressed));
                }
            }
            Devices::Kwin(fake) => fake.keyboard_key(code, u32::from(pressed)),
        }
    }

    fn run(&mut self, args: &[String]) {
        let num = |i: usize| -> u32 { args[i].parse().expect("a number") };
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "move" => {
                    self.motion(num(i + 1), num(i + 2));
                    i += 3;
                }
                "press" | "release" | "click" => {
                    let states: &[bool] = match args[i].as_str() {
                        "press" => &[true],
                        "release" => &[false],
                        _ => &[true, false],
                    };
                    for &pressed in states {
                        self.button(pressed);
                        self.flush();
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    i += 1;
                }
                "key" => {
                    let code = num(i + 1);
                    self.key(code, true);
                    self.flush();
                    std::thread::sleep(Duration::from_millis(30));
                    self.key(code, false);
                    i += 2;
                }
                "sleep" => {
                    self.flush();
                    self.queue.roundtrip(&mut self.state).unwrap();
                    std::thread::sleep(Duration::from_millis(u64::from(num(i + 1))));
                    i += 2;
                }
                other => panic!("unknown step {other}"),
            }
            self.flush();
        }
        self.queue.roundtrip(&mut self.state).unwrap();
    }
}

fn main() {
    let mut input = Input::connect();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        input.run(&args);
        return;
    }
    use std::io::BufRead;
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let steps: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        input.run(&steps);
        eprintln!("popup-input: {line}");
    }
}
