use super::*;

/// Window props that share **no** field value with `WindowProps::default()`
/// among the fields these tests read, so a mutant that ignores the argument
/// and returns the default cannot pass by coincidence.
fn distinctive_props() -> WindowProps {
    WindowProps {
        title: "From props".into(),
        width: 1024,
        height: 768,
        ..Default::default()
    }
}

#[test]
fn window_props_survive_when_no_override_is_set() {
    let resolved = resolve_window_props(distinctive_props(), None, None);
    assert_eq!(resolved.title, "From props");
    assert_eq!((resolved.width, resolved.height), (1024, 768));
}

#[test]
fn title_override_beats_the_window_props_title() {
    // Both titles are non-empty and different, so the assertion
    // distinguishes "override applied" from "override dropped" — a fixture
    // where they matched could not.
    let resolved = resolve_window_props(distinctive_props(), Some("From title()".into()), None);
    assert_eq!(resolved.title, "From title()");
    // The override touches nothing else.
    assert_eq!((resolved.width, resolved.height), (1024, 768));
}

#[test]
fn size_override_beats_the_window_props_size() {
    let resolved = resolve_window_props(distinctive_props(), None, Some((640, 480)));
    assert_eq!((resolved.width, resolved.height), (640, 480));
    assert_eq!(resolved.title, "From props");
}

/// A component that is never called — every test here stops at
/// [`App::into_startup`], which runs no user code.
fn component(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

/// #1769: what a web mount reads. An explicit `title()` wins over a
/// window-props title; a window-props title that is not the default one is
/// kept; with neither, `None`, so the page keeps its own `<title>`.
#[test]
fn web_parts_title_follows_title_then_a_named_window_props_title() {
    let explicit = App::new(component)
        .window_props(distinctive_props())
        .title("From title()")
        .into_web_parts();
    assert_eq!(explicit.title.as_deref(), Some("From title()"));

    let from_props = App::new(component)
        .window_props(distinctive_props())
        .into_web_parts();
    assert_eq!(from_props.title.as_deref(), Some("From props"));

    let unnamed = App::new(component)
        .window_props(WindowProps::default())
        .into_web_parts();
    assert_eq!(unnamed.title, None);
    assert_eq!(App::new(component).into_web_parts().title, None);
}

/// #1769: the theme and the menu bar reach the web mount; no theme is the
/// default theme, as on the desktop.
#[test]
fn web_parts_carry_the_theme_and_the_menus() {
    let theme = ThemeProviderProps {
        primary_color: Some("grape".into()),
        ..Default::default()
    };
    let parts = App::new(component)
        .theme(theme)
        .menu(vec![
            ("File", crate::menu::Menu::new()),
            ("Edit", crate::menu::Menu::new()),
        ])
        .into_web_parts();
    assert_eq!(parts.theme.primary_color.as_deref(), Some("grape"));
    let labels: Vec<&str> = parts
        .menus
        .as_ref()
        .expect("the menus reach the web mount")
        .iter()
        .map(|(label, _)| label.as_str())
        .collect();
    assert_eq!(labels, ["File", "Edit"]);

    let bare = App::new(component).into_web_parts();
    assert!(bare.menus.is_none());
    assert_eq!(
        bare.theme.primary_color,
        ThemeProviderProps::default().primary_color
    );
}

/// The GPU choice is one field, so the last of
/// [`App::gpu_config`] / [`App::external_gpu`] set on a builder is the one
/// that reaches `run`.
///
/// Only the `gpu_config` half is exercised: `ExternalGpu` needs a real
/// instance, adapter and device, which a unit test has no way to build.
/// What this pins is the part that would break silently — moving
/// `set_gpu_init` out of `run()` and into the builder methods would invert
/// the semantics to first-one-wins, and nothing else in the suite touches
/// `gpu` at all.
#[cfg(feature = "gpu-renderer")]
#[test]
fn the_last_gpu_choice_set_is_the_one_that_reaches_run() {
    use crate::shell::desktop::{GpuInit, RinchGpuConfig};

    let first = RinchGpuConfig {
        required_features: crate::wgpu::Features::empty(),
        required_limits: crate::wgpu::Limits {
            max_texture_dimension_2d: 4096,
            ..Default::default()
        },
        ..Default::default()
    };
    let second = RinchGpuConfig {
        required_features: crate::wgpu::Features::empty(),
        required_limits: crate::wgpu::Limits {
            max_texture_dimension_2d: 8192,
            ..Default::default()
        },
        ..Default::default()
    };
    assert_ne!(
        first.required_limits.max_texture_dimension_2d,
        second.required_limits.max_texture_dimension_2d,
        "the two configs must be distinguishable or this proves nothing"
    );

    let startup = App::new(component)
        .gpu_config(first)
        .gpu_config(second)
        .into_startup();

    match startup.gpu {
        Some(GpuInit::Config(cfg)) => {
            assert_eq!(cfg.required_limits.max_texture_dimension_2d, 8192)
        }
        _ => panic!("gpu_config must record a Config"),
    }
}

/// `activation_policy` reaches the startup, and leaving it out records
/// nothing (issue #1528): `None` is what keeps winit's own default and a
/// bundle's `LSUIElement`, so a default of `Some(Regular)` would be a
/// behaviour change for every bundled accessory app.
#[test]
fn activation_policy_is_recorded_only_when_set() {
    use crate::ActivationPolicy;
    assert_eq!(App::new(component).into_startup().activation_policy, None);
    for policy in [
        ActivationPolicy::Regular,
        ActivationPolicy::Accessory,
        ActivationPolicy::Prohibited,
    ] {
        let startup = App::new(component)
            .activation_policy(ActivationPolicy::Regular)
            .activation_policy(policy)
            .into_startup();
        assert_eq!(
            startup.activation_policy,
            Some(policy),
            "the last call wins"
        );
    }
}

/// `title_tooltips` reaches the startup (issue #1646): on unless turned off.
#[test]
fn debug_interface_reaches_the_startup_off_by_default() {
    assert!(!App::new(component).into_startup().debug_interface);
    assert!(
        App::new(component)
            .debug_interface(true)
            .into_startup()
            .debug_interface
    );
}

/// `devtools` reaches the startup (issue #2689): on unless turned off.
#[test]
fn devtools_is_on_unless_turned_off() {
    assert!(App::new(component).into_startup().devtools);
    assert!(!App::new(component).devtools(false).into_startup().devtools);
    assert!(
        App::new(component)
            .devtools(false)
            .devtools(true)
            .into_startup()
            .devtools,
        "the last call wins"
    );
}

/// `popup_renderers_kept` sets how many closed popups' renderers the
/// shell keeps (#2552): three unless set.
#[test]
fn popup_renderers_kept_is_three_unless_set() {
    assert_eq!(crate::popup::renderers_kept(), 3);
    let _ = App::new(component).popup_renderers_kept(0);
    assert_eq!(crate::popup::renderers_kept(), 0);
    let _ = App::new(component).popup_renderers_kept(crate::popup::DEFAULT_RENDERERS_KEPT);
}

/// `font_warmup` reaches the startup (#2552): after the first frame
/// unless set.
#[test]
fn font_warmup_is_after_the_first_frame_unless_set() {
    use crate::font::FontWarmup;
    assert_eq!(
        App::new(component).into_startup().font_warmup,
        FontWarmup::AfterFirstFrame
    );
    for when in [FontWarmup::AtStart, FontWarmup::Off] {
        assert_eq!(
            App::new(component)
                .font_warmup(when)
                .into_startup()
                .font_warmup,
            when
        );
    }
}

#[test]
fn title_tooltips_is_on_unless_turned_off() {
    assert!(App::new(component).into_startup().title_tooltips);
    assert!(
        !App::new(component)
            .title_tooltips(false)
            .into_startup()
            .title_tooltips
    );
}

/// `font_defaults` reaches the startup (#2255): Chrome's unless set, and
/// a second call replaces the first.
#[test]
fn font_defaults_reach_the_startup() {
    use crate::font::FontDefaults;
    assert_eq!(App::new(component).into_startup().font_defaults, None);
    let mine = FontDefaults::default()
        .with_standard("Inter, sans-serif")
        .with_serif(None)
        .with_sans_serif(None);
    assert_eq!(
        App::new(component)
            .font_defaults(FontDefaults::default())
            .font_defaults(mine.clone())
            .into_startup()
            .font_defaults,
        Some(mine)
    );
}

/// `smooth_wheel_scroll` reaches the startup (issue #2287): on unless
/// turned off.
#[test]
fn smooth_wheel_scroll_is_on_unless_turned_off() {
    assert!(App::new(component).into_startup().smooth_wheel_scroll);
    assert!(
        !App::new(component)
            .smooth_wheel_scroll(false)
            .into_startup()
            .smooth_wheel_scroll
    );
}

/// **The canonical chain.** Every doc example, and almost every migrated
/// call site, sets `title` AND `size` together — and until this test, no
/// fixture passed both overrides at once, so a `resolve_window_props` that
/// handled either alone but dropped both together was invisible.
#[test]
fn title_and_size_are_both_applied_when_both_are_set() {
    let startup = App::new(component)
        .title("From title()")
        .size(640, 480)
        .window_props(distinctive_props())
        .into_startup();
    assert_eq!(startup.props.title, "From title()");
    assert_eq!((startup.props.width, startup.props.height), (640, 480));
}

/// Order does not matter: `.window_props()` after `.title()`/`.size()` must
/// not silently swallow them. Replace-semantics here would open the window
/// named "Rinch Window" with no diagnostic.
#[test]
fn window_props_does_not_swallow_an_earlier_title_or_size() {
    let after = App::new(component)
        .title("Mine")
        .size(640, 480)
        .window_props(distinctive_props())
        .into_startup();
    let before = App::new(component)
        .window_props(distinctive_props())
        .title("Mine")
        .size(640, 480)
        .into_startup();
    assert_eq!(after.props.title, "Mine");
    assert_eq!(before.props.title, "Mine");
    assert_eq!(
        (after.props.width, after.props.height),
        (before.props.width, before.props.height)
    );
}

/// Each builder method must actually record its argument. Without this,
/// every one of them could be a no-op body and the suite stayed green.
#[test]
fn every_builder_method_records_its_argument() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        ..Default::default()
    };
    let startup = App::new(component)
        .title("Recorded")
        .size(1280, 720)
        .theme(theme)
        .fonts(&[crate::font::AppFont::monospace(FIXTURE_FACE)])
        .menu(vec![("File", crate::menu::Menu::new())])
        .into_startup();

    assert_eq!(startup.props.title, "Recorded");
    assert_eq!((startup.props.width, startup.props.height), (1280, 720));
    assert_eq!(startup.theme.primary_color.as_deref(), Some("cyan"));
    assert_eq!(
        startup.fonts.len(),
        1,
        "fonts() must record the face it was given"
    );
    assert_eq!(
        startup.fonts[0].generics,
        &[
            crate::font::GenericFamily::Monospace,
            crate::font::GenericFamily::UiMonospace
        ],
        "and record the generics it declared, not just the bytes"
    );
    let labels: Vec<&str> = startup
        .menus
        .as_ref()
        .expect("menu() was called")
        .iter()
        .map(|(label, _)| label.as_str())
        .collect();
    assert_eq!(labels, vec!["File"]);
}

/// A face for the builder tests. Its bytes are never rendered here — only
/// carried — so any real font file does.
const FIXTURE_FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// `fonts()` accumulates rather than replacing, and does so **in call
/// order** — which is the order they are registered in, and therefore the
/// order two faces claiming one generic resolve in. Replacing would
/// silently drop the first call; a set would lose the order.
///
/// The two entries are given *different* generics so a swap is visible: two
/// `AppFont::new` faces would be indistinguishable in the resolved list and
/// the ordering half of this test would hold for a broken implementation.
#[test]
fn fonts_accumulate_across_calls_in_call_order() {
    use crate::font::{AppFont, GenericFamily};

    let startup = App::new(component)
        .fonts(&[AppFont::serif(FIXTURE_FACE)])
        .fonts(&[
            AppFont::monospace(FIXTURE_FACE),
            AppFont::sans_serif(FIXTURE_FACE),
        ])
        .into_startup();

    let firsts: Vec<GenericFamily> = startup.fonts.iter().map(|f| f.generics[0]).collect();
    assert_eq!(
        firsts,
        vec![
            GenericFamily::Serif,
            GenericFamily::Monospace,
            GenericFamily::SansSerif
        ],
        "a second fonts() call must append to the first, in order"
    );
}

/// An app that bundles nothing carries an empty list, not a surprise.
#[test]
fn omitting_fonts_bundles_none() {
    assert!(App::new(component).into_startup().fonts.is_empty());
}

/// `App::windowless` reaches the startup with no main window and runs
/// until `exit()` (#2416), and `exit_when` still changes that.
#[test]
fn windowless_reaches_the_startup_and_runs_until_exit() {
    let startup = App::windowless(component).into_startup();
    assert!(startup.windowless);
    assert_eq!(startup.exit_when, crate::ExitWhen::Explicitly);
    let startup = App::windowless(component)
        .exit_when(crate::ExitWhen::LastWindowCloses)
        .into_startup();
    assert_eq!(startup.exit_when, crate::ExitWhen::LastWindowCloses);
    let startup = App::new(component).into_startup();
    assert!(!startup.windowless);
    assert_eq!(startup.exit_when, crate::ExitWhen::MainWindowCloses);
}

/// `prefer_x11` reaches the startup, off unless asked (issue #1519).
#[test]
fn prefer_x11_reaches_the_startup() {
    assert!(!App::new(component).into_startup().prefer_x11);
    assert!(
        App::new(component)
            .prefer_x11(true)
            .into_startup()
            .prefer_x11
    );
}

/// With no `.theme()`, the DEFAULT theme is installed rather than nothing —
/// that is what makes `rinch-components` visible out of the box, and a
/// regression would leave every component in every app unstyled.
#[test]
fn omitting_the_theme_still_installs_the_default() {
    let startup = App::new(component).into_startup();
    assert_eq!(
        startup.theme.primary_color,
        ThemeProviderProps::default().primary_color
    );
    assert!(startup.menus.is_none());
}

/// The fields the builder does **not** resolve must survive untouched. This
/// covers `app_id`, on which the Wayland identity now depends, and
/// `on_close_requested`, which a minimize-to-tray app cannot work without.
#[test]
fn window_props_fields_the_builder_does_not_resolve_are_preserved() {
    let props = WindowProps {
        transparent: true,
        always_on_top: true,
        menu_in_titlebar: true,
        x: Some(17),
        y: Some(23),
        app_id: Some("com.example.notes".into()),
        ..distinctive_props()
    };
    let startup = App::new(component)
        .title("Anything")
        .size(1, 2)
        .window_props(props)
        .into_startup();

    assert!(startup.props.transparent);
    assert!(startup.props.always_on_top);
    assert!(startup.props.menu_in_titlebar);
    assert_eq!((startup.props.x, startup.props.y), (Some(17), Some(23)));
    assert_eq!(startup.props.app_id.as_deref(), Some("com.example.notes"));
}

#[test]
fn borderless_resizable_arms_the_default_resize_inset() {
    let resolved = resolve_window_props(
        WindowProps {
            borderless: true,
            resizable: true,
            resize_inset: None,
            ..Default::default()
        },
        None,
        None,
    );
    assert_eq!(resolved.resize_inset, Some(8.0));
}

#[test]
fn an_explicit_resize_inset_is_never_overwritten() {
    // 12.0, not 8.0: a mutant that assigns unconditionally would be
    // invisible against a fixture that already held the default.
    let resolved = resolve_window_props(
        WindowProps {
            borderless: true,
            resizable: true,
            resize_inset: Some(12.0),
            ..Default::default()
        },
        None,
        None,
    );
    assert_eq!(resolved.resize_inset, Some(12.0));
}

#[test]
fn a_decorated_window_gets_no_resize_inset() {
    let resolved = resolve_window_props(
        WindowProps {
            borderless: false,
            resizable: true,
            resize_inset: None,
            ..Default::default()
        },
        None,
        None,
    );
    assert_eq!(resolved.resize_inset, None);
}

#[test]
fn a_fixed_size_borderless_window_gets_no_resize_inset() {
    let resolved = resolve_window_props(
        WindowProps {
            borderless: true,
            resizable: false,
            resize_inset: None,
            ..Default::default()
        },
        None,
        None,
    );
    assert_eq!(resolved.resize_inset, None);
}
