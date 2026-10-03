//! The inline-titlebar menu spacer reserves exactly the row it stands in for
//! (issue #529).
//!
//! In `MenuBarLayout::InlineTitlebar` the menu row (hamburger, branded title,
//! top-level labels) is an absolutely positioned layer drawn over the
//! titlebar, and an empty `.rinch-borderlesswindow__menu-spacer` in the
//! titlebar's own flex row holds that space open so the right section and the
//! window controls never slide under it. The spacer's width used to be a
//! guess made before any document existed: `str::len()` (UTF-8 **bytes**) x
//! 8px per label, later (#1308) a per-character "narrow/wide" table. Neither
//! is a text advance, so a Cyrillic or CJK bar over- or under-reserved, a
//! combining mark counted as a whole character, and the branded title — which
//! the row also holds — was not counted at all.
//!
//! These fixtures mount the real `BorderlessWindow` under the real stylesheet
//! with the real inline menu row and ask the one question that matters,
//! independent of which fonts the host has: **does the spacer end where the
//! row ends?** A width measured from the host's fonts would pin the local font
//! set; this relation does not. Every label set here is non-ASCII or carries a
//! title, because ASCII-only labels with no title are the fixed point where
//! the old estimate happened to be close.

use super::*;

use crate::menu::{Menu, MenuItem};
use rinch_components::BorderlessWindow;
use rinch_core::Component;
use rinch_core::reactive::Signal;

const VIEWPORT: (f32, f32) = (1000.0, 400.0);

const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// Mount a `BorderlessWindow` in the inline-titlebar layout with these menu
/// labels and title, the way `app_builder::run_desktop_linux` wires it.
/// `estimate` is the `MenuBarContext::spacer_width` the shell hands in.
fn mount_with(labels: &'static [&'static str], title: &'static str, estimate: u32) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let menus: Vec<(String, Menu)> = labels
            .iter()
            .map(|l| (l.to_string(), Menu::new().item(MenuItem::new("Entry"))))
            .collect();
        let menus = Rc::new(menus);
        let active_menu: Signal<i32> = Signal::new(-1);

        let items_renderer: rinch_core::MenuBarRenderer = {
            let md = menus.clone();
            Rc::new(move |scope| {
                let refs: Vec<(&str, &Menu)> = md.iter().map(|(l, m)| (l.as_str(), m)).collect();
                crate::menu::app_menu_bar::render_menu_items_inline(scope, &refs, active_menu)
            })
        };
        let overlay_renderer: rinch_core::MenuBarRenderer = Rc::new(move |scope| {
            crate::menu::app_menu_bar::render_inline_overlay(scope, active_menu)
        });
        rinch_core::create_context(rinch_core::MenuBarContext {
            renderer: items_renderer.clone(),
            bar_height: 0,
            layout: rinch_core::MenuBarLayout::InlineTitlebar,
            items_renderer: Some(items_renderer),
            overlay_renderer: Some(overlay_renderer),
            spacer_width: estimate,
        });

        // A left section of a known width, standing in for the hamburger.
        let left: rinch_components::SectionRenderer = Rc::new(|scope| {
            let b = scope.create_element("div");
            b.set_attribute("style", "width: 28px; height: 28px; flex-shrink: 0;");
            b
        });

        let content = scope.create_element("div");
        BorderlessWindow {
            title: title.to_string(),
            left_section: Some(left),
            show_minimize: true,
            show_maximize: true,
            show_close: true,
            ..Default::default()
        }
        .render(scope, &[content])
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::css::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    // The first resolve lays the row out; the spacer follows it on the next.
    for _ in 0..3 {
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
    app
}

/// As [`mount_with`], handed the estimate the shell itself computes for these
/// labels — so the fixture fails against the estimate as shipped, not against
/// a straw zero.
fn mount(labels: &'static [&'static str], title: &'static str) -> RinchApp {
    mount_with(
        labels,
        title,
        crate::app_builder::inline_spacer_width(labels),
    )
}

/// The painted box of the one node carrying `class` exactly.
fn painted(app: &RinchApp, class: &str) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let matches: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected one `{class}`, found {}",
        matches.len()
    );
    crate::app::hit_testing::painted_element_box(&d.tree, matches[0])
}

/// The spacer's right edge lands on the row's right edge, rounded up to a
/// whole pixel.
#[track_caller]
fn assert_spacer_reserves_the_row(app: &RinchApp) {
    let (rx, _, rw, _) = painted(app, "rinch-app-menu-bar__inline-row");
    let (sx, _, sw, _) = painted(app, "rinch-borderlesswindow__menu-spacer");
    let row_right = rx + rw;
    let spacer_right = sx + sw;
    assert!(
        rw > 50.0,
        "positive control: the row must have been laid out with its labels (width {rw})"
    );
    assert!(
        spacer_right >= row_right - 0.01 && spacer_right < row_right + 1.0,
        "the spacer must end where the menu row ends: row [{rx}, {row_right}], \
         spacer [{sx}, {spacer_right}] (width {sw})"
    );
}

/// Cyrillic: 2 UTF-8 bytes per character. The byte estimate reserved twice
/// the width.
#[test]
fn a_cyrillic_bar_reserves_its_measured_width() {
    let app = mount(&["Файл", "Правка", "Вид", "Справка"], "");
    assert_spacer_reserves_the_row(&app);
}

/// CJK: 3 UTF-8 bytes per character, and glyphs that are neither one nor two
/// "narrow units" of any fixed size.
#[test]
fn a_cjk_bar_reserves_its_measured_width() {
    let app = mount(&["ファイル", "編集", "表示", "ヘルプ"], "");
    assert_spacer_reserves_the_row(&app);
}

/// Combining marks: several `char`s draw as one grapheme, so any per-char
/// count over-reserves.
#[test]
fn combining_marks_reserve_their_measured_width() {
    let app = mount(
        &[
            "Re\u{0301}gime\u{0301}",
            "e\u{0301}\u{0300}\u{0302}dit",
            "Vie\u{0302}w",
        ],
        "",
    );
    assert_spacer_reserves_the_row(&app);
}

/// Right-to-left labels (Hebrew, Arabic) and an emoji.
#[test]
fn rtl_and_emoji_labels_reserve_their_measured_width() {
    let app = mount(&["קובץ", "עריכה", "ملف", "\u{1F4C1} Files"], "");
    assert_spacer_reserves_the_row(&app);
}

/// The branded title lives in the row too, so the spacer has to hold it open.
/// Every estimate the shell has made ignored it.
#[test]
fn the_branded_title_is_reserved_with_the_labels() {
    let app = mount(&["Файл", "編集"], "Rinch Zoo");
    assert_spacer_reserves_the_row(&app);
}

/// The shell's estimate is only the first frame's answer: a wildly wrong one
/// (here 600px) is replaced by the measured row once the row has a layout.
#[test]
fn the_estimate_is_replaced_by_the_measured_row() {
    let app = mount_with(&["File", "Edit"], "", 600);
    assert_spacer_reserves_the_row(&app);
}
