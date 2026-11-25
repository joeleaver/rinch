//! The runtime's construction-time state, tested without an event loop:
//! bundled and registered fonts, focus, window props and attributes.

use super::*;
use crate::font::AppFont;

const FACE: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

fn component(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

/// The runtime registers a build's bundled faces **at construction** — the
/// only moment early enough, since `RinchRuntime::new` runs before the
/// window and therefore before the first layout pass. A face that arrived
/// later would leave the first frame measured against a fallback, then
/// reflow.
///
/// This is testable only because `new` is a pure struct build: it opens no
/// window and touches no event loop. [`App::run`](crate::App::run) around
/// it is not, which is exactly why the registration lives here rather than
/// further in.
#[test]
fn hiding_the_window_publishes_not_focused_1770() {
    let mut runtime = RinchRuntime::new("Hidden", 640, 480, component, &[]);
    runtime
        .main
        .app
        .handle_event(PlatformEvent::WindowFocus(true), (640, 480), 1.0);
    assert!(crate::window::window_focused(), "positive control");
    runtime.hide_window();
    assert!(!crate::window::window_focused());
}

/// A shell window is not focused until the system's keyboard enter
/// (#2731), so its element's `focus` event and IME wait for it — the
/// answer `window_focused()` already gave. A bare `RinchApp` (an embed
/// host's) still starts focused.
///
/// **Mutant: dropping the `window_focused = false` in `ShellWindow::new`.**
#[test]
fn a_shell_window_starts_unfocused_2731() {
    let mut runtime = RinchRuntime::new("Unfocused", 640, 480, component, &[]);
    assert!(!runtime.main.app.window_focused);
    assert!(!runtime.main.app.ime_state().enabled);
    runtime
        .main
        .app
        .handle_event(PlatformEvent::WindowFocus(true), (640, 480), 1.0);
    assert!(runtime.main.app.window_focused, "positive control");
    assert!(RinchApp::new(component).window_focused);
}

/// Hiding the window takes its keyboard, and the system's `Focused(false)`
/// for a window rinch drops itself never arrives: the focused element hears
/// `blur` at the hide, before `window_focused()` turns false, and `focus`
/// again at the next show's keyboard enter (#2731 review). It logged one
/// `focus` for the whole sequence — every show after the first fired
/// nothing, deskrinch's launcher pattern.
///
/// **Mutant: `hide_window` publishing `focused: false` alone** (as before).
#[test]
fn hiding_and_showing_fires_blur_and_focus_again_2731() {
    use std::cell::RefCell;
    use std::rc::Rc;
    type Shared<T> = Rc<RefCell<T>>;
    let log: Shared<Vec<String>> = Rc::new(RefCell::new(Vec::new()));
    let field: Shared<Option<NodeHandle>> = Rc::new(RefCell::new(None));
    let (l, out) = (log.clone(), field.clone());
    let mut runtime = RinchRuntime::new(
        "Hide and show",
        640,
        480,
        move |scope: &mut RenderScope| {
            let input = scope.create_element("input");
            input.set_attribute("style", "display: block; width: 100px; height: 20px");
            for (attr, what) in [("data-onfocus", "focus"), ("data-onblur", "blur")] {
                let l = l.clone();
                let id = rinch_core::register_handler(Rc::new(move || {
                    // What the app's window state says as the event runs.
                    let w = crate::window::window_focused();
                    l.borrow_mut().push(format!("{what} window={w}"));
                }));
                input.set_attribute(attr, &id.0.to_string());
            }
            input.focus();
            *out.borrow_mut() = Some(input.clone());
            input
        },
        &[],
    );
    let app = &mut runtime.main.app;
    app.mount_component(640.0, 480.0);
    app.handle_event(PlatformEvent::WindowFocus(true), (640, 480), 1.0);
    assert_eq!(*log.borrow(), ["focus window=false"], "positive control");
    log.borrow_mut().clear();

    runtime.hide_window();
    assert_eq!(
        *log.borrow(),
        ["blur window=true"],
        "blur at the hide, before the publish"
    );
    assert!(!crate::window::window_focused());
    log.borrow_mut().clear();

    let app = &mut runtime.main.app;
    app.handle_event(PlatformEvent::AboutToWait, (640, 480), 1.0);
    assert_eq!(*log.borrow(), Vec::<String>::new(), "nothing while hidden");
    app.handle_event(PlatformEvent::WindowFocus(true), (640, 480), 1.0);
    assert_eq!(
        *log.borrow(),
        ["focus window=false"],
        "focus again at the show"
    );
    assert!(field.borrow().is_some());
}

#[test]
fn the_runtime_registers_bundled_fonts_at_construction() {
    let runtime = RinchRuntime::new(
        "t",
        800,
        600,
        component,
        &[AppFont::monospace(FACE), AppFont::serif(FACE)],
    );
    let generics: Vec<_> = runtime
        .main
        .app
        .pending_fonts
        .iter()
        .map(|f| f.font.generics[0])
        .collect();
    assert_eq!(
        generics,
        vec![
            parley::fontique::GenericFamily::Monospace,
            parley::fontique::GenericFamily::Serif
        ],
        "both faces must reach the app, in the order they were declared"
    );
}

/// The shell installs the named video sink factory (rinch-core's hook
/// that rinch-video and rinch-rtc draw through). Counted on this thread,
/// so another test installing it meanwhile cannot make this pass.
#[test]
fn the_runtime_installs_the_video_sink_factory() {
    let before = rinch_core::video_sink::installs_on_this_thread();
    let _runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    assert_eq!(
        rinch_core::video_sink::installs_on_this_thread(),
        before + 1
    );
    assert!(rinch_core::video_sink::has_video_sink_factory());
}

/// `rinch::font::register_font` called before there is a runtime (in
/// `main`, before `App::run`) has no loop to wake: the runtime takes the
/// queued face at construction, after the build's own, so it is in the
/// font context that measures the first frame and kept for later windows.
#[test]
fn a_face_registered_before_the_runtime_reaches_its_first_frame() {
    let _lock = TestQueueLock::hold();
    let _ = crate::font::take_registered_fonts();
    crate::font::register_font(AppFont::monospace(FACE));
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let runtime = RinchRuntime::new("t", 800, 600, component, &[AppFont::serif(FACE)]);
    let generics: Vec<_> = runtime
        .main
        .app
        .pending_fonts
        .iter()
        .map(|f| f.font.generics[0])
        .collect();
    assert_eq!(
        generics,
        vec![
            parley::fontique::GenericFamily::Serif,
            parley::fontique::GenericFamily::Monospace
        ],
        "the build's face first, then the queued one"
    );
    assert_eq!(runtime.fonts.len(), 2, "both kept for windows opened later");
    assert!(crate::font::take_registered_fonts().is_empty());
}

/// While the app runs, `register_font` queues the face and wakes the loop
/// (`RegisterFonts`); the turn that drains it registers the face with
/// every open window's document and keeps it with the app's fonts, which
/// is what `new_other_window` makes a later window with.
#[test]
fn a_face_registered_while_running_reaches_the_windows() {
    let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    let _ = crate::font::take_registered_fonts();

    crate::font::register_font(AppFont::monospace(FACE));
    let queued = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    assert!(
        queued
            .iter()
            .any(|(_, e)| matches!(e, RinchNativeEvent::RegisterFonts)),
        "the loop is woken to register it: {queued:?}"
    );
    assert!(
        runtime.main.app.pending_fonts.is_empty(),
        "nothing is registered until the loop's turn"
    );

    runtime.register_queued_fonts();
    assert_eq!(runtime.main.app.pending_fonts.len(), 1);
    assert_eq!(
        runtime.main.app.pending_fonts[0].font.generics[0],
        parley::fontique::GenericFamily::Monospace
    );
    assert_eq!(runtime.fonts.len(), 1, "kept for windows opened later");

    // A second turn with nothing queued registers nothing again.
    runtime.register_queued_fonts();
    assert_eq!(runtime.main.app.pending_fonts.len(), 1);
    assert_eq!(runtime.fonts.len(), 1);
}

/// And a build that bundles none is not handed a phantom face.
#[test]
fn no_bundled_fonts_registers_nothing() {
    let runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    assert!(runtime.main.app.pending_fonts.is_empty());
}

/// The assembly the real desktop path actually uses. `App::run` resolves
/// the builder and lands in `run_rinch_with_window_props_and_menu`, which
/// creates the event loop and can never be tested — so everything it does
/// *before* that lives in `build_runtime`, and this is what pins it.
///
/// Both halves matter and neither was covered before: the faces must
/// survive the trip (a `&[]` here and `App::fonts` would be a builder
/// method with no effect on any real window), and so must the window props
/// that carry the title, size and `app_id`.
/// #1519: the click-through state the window is created with. The first
/// creation takes `WindowProps::click_through`; a toggle is kept across a
/// hide/show re-creation (which does not re-read the props); a new first
/// creation starts from the props again.
#[test]
fn click_through_starts_at_the_props_and_a_toggle_survives_a_re_show() {
    use super::overlay_window::{AfterCreate, after_create_actions};
    let props = rinch_core::element::WindowProps {
        click_through: true,
        ..Default::default()
    };
    let mut runtime = build_runtime(component, &props, None, &[]);
    let first = runtime.main.main_window_steps(Backend::X11, true);
    assert!(first.click_through, "the prop reaches the first creation");
    assert!(after_create_actions(first, false).contains(&AfterCreate::ClickThrough));

    assert!(!runtime.main.note_click_through(false));
    let reshown = runtime.main.main_window_steps(Backend::X11, false);
    assert!(
        !reshown.click_through,
        "the toggle is kept across hide/show"
    );
    assert!(!after_create_actions(reshown, false).contains(&AfterCreate::ClickThrough));

    assert!(runtime.main.note_click_through(true));
    assert!(
        runtime
            .main
            .main_window_steps(Backend::X11, false)
            .click_through
    );

    // Off in the props: a re-show after a toggle on keeps it on.
    let mut plain = build_runtime(component, &Default::default(), None, &[]);
    assert!(
        !plain
            .main
            .main_window_steps(Backend::Wayland, true)
            .click_through
    );
    plain.main.note_click_through(true);
    assert!(
        plain
            .main
            .main_window_steps(Backend::Wayland, false)
            .click_through
    );
}

/// #1549: the level and workspace settings come from the props on every
/// creation, so a hide/show re-creation (#1546) puts them back; and a
/// raised level is winit's `AlwaysOnTop` at creation on every backend.
#[test]
fn the_level_and_workspaces_survive_a_re_show() {
    use super::overlay_window::{AfterCreate, after_create_actions};
    use rinch_core::element::WindowLevel;
    let props = rinch_core::element::WindowProps {
        level: WindowLevel::ScreenSaver,
        visible_on_all_workspaces: true,
        ..Default::default()
    };
    let mut runtime = build_runtime(component, &props, None, &[]);
    assert!(runtime.main.main_window_always_on_top());
    for first in [true, false] {
        let mac = runtime.main.main_window_steps(Backend::MacOs, first);
        assert!(
            after_create_actions(mac, true).contains(&AfterCreate::MacPlacement {
                level: Some(1000),
                collection: 1 | 256,
            })
        );
        let x11 = runtime.main.main_window_steps(Backend::X11, first);
        assert!(after_create_actions(x11, true).contains(&AfterCreate::ReassertOnTop));
        for backend in [Backend::Windows, Backend::Wayland, Backend::MacOs] {
            let steps = runtime.main.main_window_steps(backend, first);
            let attrs = runtime
                .main
                .main_window_attributes(steps, backend, first, first);
            assert_eq!(
                attrs.window_level,
                winit::window::WindowLevel::AlwaysOnTop,
                "{backend:?}"
            );
        }
    }
    let plain = build_runtime(component, &Default::default(), None, &[]);
    assert!(!plain.main.main_window_always_on_top());
}

/// #2817: a window's activation token goes in its Wayland attributes
/// (winit's `with_activation_token`, activated before the initial
/// commit), and only there: not on X11, not for a non-activating window.
#[test]
fn an_activation_token_goes_in_a_wayland_windows_attributes_2817() {
    let token_in = |runtime: &mut RinchRuntime, backend: Backend| {
        let steps = runtime.main.main_window_steps(backend, true);
        let attrs = runtime
            .main
            .main_window_attributes(steps, backend, true, true);
        format!("{:?}", attrs.platform).contains("\"launch-2817\"")
    };
    let mut runtime = build_runtime(component, &Default::default(), None, &[]);
    assert!(!token_in(&mut runtime, Backend::Wayland), "none given");
    runtime.main.activation_token = Some("launch-2817".into());
    assert!(token_in(&mut runtime, Backend::Wayland));
    assert!(!token_in(&mut runtime, Backend::X11));

    let props = rinch_core::element::WindowProps {
        non_activating: true,
        ..Default::default()
    };
    let mut runtime = build_runtime(component, &props, None, &[]);
    runtime.main.activation_token = Some("launch-2817".into());
    assert!(!token_in(&mut runtime, Backend::Wayland), "non-activating");
}

/// #1519, the default path: an ordinary window takes no hit-test call and
/// no extra step, and a tray re-show is not moved to `WindowProps::x/y`
/// (it never was); only the first creation is placed.
#[test]
fn an_ordinary_window_takes_the_old_path() {
    use super::overlay_window::after_create_actions;
    let props = rinch_core::element::WindowProps {
        x: Some(40),
        y: Some(50),
        ..Default::default()
    };
    let mut runtime = build_runtime(component, &props, None, &[]);
    for backend in [
        Backend::X11,
        Backend::Wayland,
        Backend::Windows,
        Backend::MacOs,
    ] {
        let steps = runtime.main.main_window_steps(backend, true);
        assert!(after_create_actions(steps, false).is_empty(), "{backend:?}");
        assert!(after_create_actions(steps, true).is_empty(), "{backend:?}");
    }
    let steps = runtime.main.main_window_steps(Backend::Wayland, true);
    let first = runtime
        .main
        .main_window_attributes(steps, Backend::Wayland, true, true);
    assert!(first.position.is_some(), "the first creation is placed");
    let reshown = runtime
        .main
        .main_window_attributes(steps, Backend::Wayland, false, false);
    assert!(
        reshown.position.is_none(),
        "a re-show with no recorded geometry is placed by the system"
    );
    assert!(first.active && first.visible && reshown.active && reshown.visible);
}

/// #2821: the main window's layer surface is named by its own namespace,
/// else the window's app id, which without `WindowProps::app_id` is the
/// one derived from the executable.
#[cfg(target_os = "linux")]
#[test]
fn the_main_window_names_its_layer_surface() {
    let props = |namespace: Option<&str>, app_id: Option<&str>| rinch_core::element::WindowProps {
        wayland_layer: Some(rinch_core::element::LayerSurface {
            namespace: namespace.map(str::to_string),
            ..Default::default()
        }),
        app_id: app_id.map(str::to_string),
        ..Default::default()
    };
    let namespace = |props: &rinch_core::element::WindowProps| {
        build_runtime(component, props, None, &[])
            .main
            .layer_spec(true)
            .namespace
    };
    assert_eq!(
        namespace(&props(Some("dock"), Some("org.example.Bar"))),
        "dock"
    );
    assert_eq!(
        namespace(&props(None, Some("org.example.Bar"))),
        "org.example.Bar"
    );
    assert_eq!(namespace(&props(None, None)), resolved_app_id(None));
    // No props at all: neither a namespace nor an app id.
    let mut bare = build_runtime(component, &Default::default(), None, &[]);
    bare.main.app.window_props = None;
    assert_eq!(bare.main.layer_spec(true).namespace, "rinch");
}

#[test]
fn build_runtime_carries_both_the_window_props_and_the_bundled_fonts() {
    let props = rinch_core::element::WindowProps {
        title: "Carried".into(),
        width: 1024,
        height: 768,
        app_id: Some("com.example.carried".into()),
        ..Default::default()
    };

    let runtime = build_runtime(component, &props, None, &[AppFont::monospace(FACE)]);

    assert_eq!(
        runtime.main.app.pending_fonts.len(),
        1,
        "the bundled face must reach the app the window will mount"
    );
    assert_eq!(
        runtime.main.app.pending_fonts[0].font.generics[0],
        parley::fontique::GenericFamily::Monospace,
        "and reach it with the generic it declared"
    );
    assert_eq!((runtime.main.width, runtime.main.height), (1024, 768));
    assert_eq!(runtime.main.title, "Carried");
    let carried = runtime
        .main
        .app
        .window_props
        .as_ref()
        .expect("window props must reach the app");
    assert_eq!(carried.app_id.as_deref(), Some("com.example.carried"));
}
