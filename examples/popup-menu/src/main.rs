//! Popup windows from a panel (#2552): a 40 px bar (a `wlr-layer-shell`
//! surface, keyboard none) whose "Menu" button opens a tray-style menu with
//! a submenu (and a submenu on that), and whose "?" shows a tooltip while
//! the pointer is over it. Each is a popup window
//! ([`open_popup`]): it leaves the bar's surface, is
//! sized by its content, and opens away from the bar's edge (above a bottom
//! bar, below a top one), sliding along the bar to stay on the output.
//!
//! - `BAR_EDGE=top` puts the bar along the top edge (default: the bottom).
//! - `POPUP_KEYBOARD=1` gives the menu the keyboard through the bar
//!   (`keyboard_via_parent`, for sway and labwc).
//! - `POPUP_TIP_FLIP=1` asks the tooltip for the side of the bar that faces
//!   the screen's edge, so the system flips it.
//! - `POPUP_FONT_WARMUP=0` turns the font warm-up off (`start`: at start)
//!   ([`App::font_warmup`]); `POPUP_RENDERERS_KEPT=N` keeps N closed popups'
//!   GPU renderers ([`App::popup_renderers_kept`]): for measuring them.
//! - `POPUP_SCRIPT=1` prints what happens (`POPUP …` lines: each popup's
//!   placement and why it went), shows the tooltip at once, closes the bar
//!   after `POPUP_SCRIPT_MS` (8000) ms and exits: what the live checks read.
//!
//! Where popups are not supported (X11, Windows, macOS for now) the bar
//! says so and the button does nothing.

use std::rc::Rc;

use rinch::prelude::*;
use rinch::{
    Adjust, Dismissal, LayerAnchor, LayerAxes, LayerExclusiveZone, LayerKeyboard, LayerSurface,
    Placement, PopupKind, WindowProps,
};

const BAR: u32 = 40;

fn top_edge() -> bool {
    std::env::var("BAR_EDGE").as_deref() == Ok("top")
}

fn scripted() -> bool {
    std::env::var("POPUP_SCRIPT").is_ok()
}

/// A menu from the bar opens away from its edge and may slide along it; a
/// menu taller than the space left is made shorter and scrolls.
fn bar_placement() -> Placement {
    if top_edge() {
        Placement::BOTTOM_START
    } else {
        Placement::TOP_START
    }
}

/// Report `handle`'s placement each time the system places it, under
/// `POPUP_SCRIPT`.
fn report(name: &'static str, handle: WindowHandle) {
    if !scripted() {
        return;
    }
    rinch::Effect::new(move || {
        if let Some(p) = handle.placement() {
            println!(
                "POPUP placed {name} id={} x={} y={} w={} h={} flipped_x={} flipped_y={}",
                handle.id(),
                p.x,
                p.y,
                p.width,
                p.height,
                p.flipped_x,
                p.flipped_y
            );
        }
    });
}

fn told(name: &'static str) -> Option<Rc<dyn Fn(Dismissal)>> {
    Some(Rc::new(move |why| {
        eprintln!("POPUP dismissed {name} {why:?}")
    }))
}

/// One menu: `items` rows; a row named with a trailing "▸" opens the next
/// level of submenu (up to `depth`).
fn menu_content(level: u32, depth: u32) -> impl FnOnce(&mut RenderScope) -> NodeHandle + 'static {
    move |__scope: &mut RenderScope| {
        let sub_row: Signal<Option<NodeHandle>> = Signal::new(None);
        let sub: Signal<Option<WindowHandle>> = Signal::new(None);
        let open_sub = move || {
            if scripted() {
                eprintln!("POPUP hover More at level {level}");
            }
            if level >= depth || sub.get().is_some_and(|s| s.is_open()) {
                return;
            }
            let Some(anchor) = sub_row.get() else { return };
            let name = ["menu", "submenu", "sub-submenu"][level.min(2) as usize];
            let opened = open_popup(
                PopupProps {
                    anchor: PopupAnchor::Node(anchor),
                    placement: Placement::RIGHT_START,
                    // The submenu's first row level with its opener.
                    offset: (0.0, -6.0),
                    on_dismissed: told(name),
                    ..PopupProps::default()
                },
                menu_content(level + 1, depth),
            );
            match opened {
                Ok(h) => {
                    report(name, h);
                    sub.set(Some(h));
                }
                Err(e) => println!("POPUP refused {name}: {e}"),
            }
        };
        let activate = move |what: &'static str| {
            move || {
                println!("POPUP activated {what}");
                if let Some(me) = WindowHandle::current() {
                    me.close_popup_chain();
                }
            }
        };
        let row = "padding: 6px 14px; border-radius: 4px; cursor: default;";
        rsx! {
            div {
                style: "min-width: 180px; padding: 6px; background: #262a36; color: #e6e8ef; \
                        font-family: sans-serif; font-size: 13px; border: 1px solid #3a4052; \
                        border-radius: 8px;",
                div { style: row, onclick: activate("Open"), "Open" }
                div { style: row, onclick: activate("Settings"), "✓ Settings" }
                div { style: "height: 1px; margin: 4px 6px; background: #3a4052;" }
                div { style: "padding: 6px 14px; color: #6b7185;", "Disabled item" }
                if level < depth {
                    div {
                        ref: sub_row,
                        style: row,
                        onmouseenter: open_sub,
                        onclick: open_sub,
                        "More ▸"
                    }
                }
                div { style: row, onclick: activate("Quit"), "Quit" }
            }
        }
    }
}

#[component]
fn bar() -> NodeHandle {
    let button: Signal<Option<NodeHandle>> = Signal::new(None);
    let help: Signal<Option<NodeHandle>> = Signal::new(None);
    let menu: Signal<Option<WindowHandle>> = Signal::new(None);
    let tip: Signal<Option<WindowHandle>> = Signal::new(None);
    let open_menu = move || {
        if scripted() {
            eprintln!("POPUP click on the menu button");
        }
        // Open: the press on the button closes it (a press outside the menu,
        // `PressOutside { on_anchor: true }`), at the end of that press, so
        // this handler still finds it open and leaves it to close.
        if menu.get().is_some_and(|m| m.is_open()) {
            return;
        }
        let Some(anchor) = button.get() else { return };
        let opened = open_popup(
            PopupProps {
                // The button's columns across the bar's whole thickness, so
                // the menu opens past the bar's edge and never covers it.
                anchor: PopupAnchor::NodeAcross(anchor),
                placement: bar_placement(),
                adjust: Adjust::BAR_MENU,
                // A 4 px gap from the bar.
                offset: (0.0, if top_edge() { 4.0 } else { -4.0 }),
                keyboard_via_parent: std::env::var("POPUP_KEYBOARD").is_ok(),
                on_dismissed: told("menu"),
                ..PopupProps::default()
            },
            menu_content(1, 3),
        );
        match opened {
            Ok(h) => {
                report("menu", h);
                menu.set(Some(h));
            }
            Err(e) => println!("POPUP refused menu: {e}"),
        }
    };

    let show_tip = move || {
        if tip.get().is_some_and(|t| t.is_open()) {
            return;
        }
        let Some(anchor) = help.get() else { return };
        let opened = open_popup(
            PopupProps {
                anchor: PopupAnchor::Node(anchor),
                // `POPUP_TIP_FLIP=1` asks for the side the bar is against, so
                // the system flips it (what the live check of a flip reads).
                placement: if top_edge() != std::env::var("POPUP_TIP_FLIP").is_ok() {
                    Placement::BOTTOM_CENTER
                } else {
                    Placement::TOP_CENTER
                },
                kind: PopupKind::Tooltip,
                offset: (0.0, if top_edge() { 6.0 } else { -6.0 }),
                on_dismissed: told("tooltip"),
                ..PopupProps::default()
            },
            |__scope: &mut RenderScope| {
                rsx! {
                    div {
                        style: "padding: 4px 8px; background: #fff8c5; color: #222; \
                                font-family: sans-serif; font-size: 12px; \
                                border: 1px solid #d4c26a; border-radius: 4px;",
                        "A popup tooltip: it can leave the bar"
                    }
                }
            },
        );
        match opened {
            Ok(h) => {
                report("tooltip", h);
                tip.set(Some(h));
            }
            Err(e) => println!("POPUP refused tooltip: {e}"),
        }
    };
    let hide_tip = move || {
        if let Some(t) = tip.get() {
            t.close();
        }
    };

    if scripted() {
        // The tooltip needs no press: shown once the bar is laid out.
        set_timeout(500, show_tip);
        let ms: u32 = std::env::var("POPUP_SCRIPT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8000);
        set_timeout(ms, move || {
            println!("POPUP closing the bar");
            if let Some(main) = WindowHandle::main() {
                main.close();
            }
        });
    }

    rsx! {
        div {
            style: "display: flex; align-items: center; gap: 8px; height: 100%; padding: 0 8px; \
                    box-sizing: border-box; background: #1d1f27; color: #e6e8ef; \
                    font-family: sans-serif; font-size: 13px;",
            button {
                ref: button,
                class: "menu-button",
                style: "padding: 4px 12px; border: none; border-radius: 6px; \
                        background: #3b5bdb; color: white;",
                onclick: open_menu,
                "Menu"
            }
            span {
                ref: help,
                class: "help",
                style: "padding: 4px 8px; border-radius: 10px; background: #2c3040;",
                onmouseenter: show_tip,
                onmouseleave: hide_tip,
                "?"
            }
            span {
                style: "flex: 1; text-align: right; color: #a4a9b8;",
                {if popups_supported() { "popups: on" } else { "popups: not supported here" }}
            }
        }
    }
}

fn main() {
    let edge = if top_edge() {
        LayerAnchor::TOP
    } else {
        LayerAnchor::BOTTOM
    };
    App::new(bar)
        .window_props(WindowProps {
            title: "Popup menu".into(),
            width: 800,
            height: BAR,
            borderless: true,
            app_id: Some("rinch-popup-menu".into()),
            wayland_layer: Some(LayerSurface {
                anchor: edge | LayerAnchor::LEFT | LayerAnchor::RIGHT,
                exclusive_zone: LayerExclusiveZone::Reserve(BAR),
                compositor_size: LayerAxes::WIDTH,
                keyboard: LayerKeyboard::None,
                ..Default::default()
            }),
            ..Default::default()
        })
        .font_warmup(match std::env::var("POPUP_FONT_WARMUP").as_deref() {
            Ok("0") => rinch::font::FontWarmup::Off,
            Ok("start") => rinch::font::FontWarmup::AtStart,
            _ => rinch::font::FontWarmup::AfterFirstFrame,
        })
        .popup_renderers_kept(
            std::env::var("POPUP_RENDERERS_KEPT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(rinch::popup::DEFAULT_RENDERERS_KEPT),
        )
        .run();
}
