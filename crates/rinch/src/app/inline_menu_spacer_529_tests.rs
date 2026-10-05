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

use super::perf_expect;
use super::*;

use crate::menu::{Menu, MenuItem};
use rinch_components::BorderlessWindow;
use rinch_core::Component;
use rinch_core::reactive::Signal;

const VIEWPORT: (f32, f32) = (perf_expect::SIZE.0 as f32, perf_expect::SIZE.1 as f32);

const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// Mount a `BorderlessWindow` in the inline-titlebar layout with these menu
/// labels and title, the way `app_builder::run_desktop_linux` wires it.
/// `estimate` is the `MenuBarContext::spacer_width` the shell hands in.
fn mount_with(labels: &'static [&'static str], title: &'static str, estimate: u32) -> RinchApp {
    let mut app = build(labels, title, estimate, 0);
    drive(&mut app);
    app
}

/// Drive `app` the way the desktop loop does — a redraw, then an
/// `AboutToWait` that asks for the next one only if something is owed — so
/// the spacer has to reach its width through the real frame scheduling, not
/// through a resolve the test forced. Answers how many frames were painted.
fn drive(app: &mut RinchApp) -> usize {
    let mut frames = 0;
    for _ in 0..6 {
        perf_expect::paint(app);
        frames += 1;
        if !perf_expect::about_to_wait(app) {
            break;
        }
    }
    frames
}

/// The mounted window with its stylesheets loaded and **no frame painted
/// yet**. `right_width` above zero adds a `.probe-right` box of that width as
/// the titlebar's right section.
fn build(
    labels: &'static [&'static str],
    title: &'static str,
    estimate: u32,
    right_width: u32,
) -> RinchApp {
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
        let right: Option<rinch_components::SectionRenderer> = (right_width > 0).then(|| {
            let r: rinch_components::SectionRenderer = Rc::new(move |scope| {
                let b = scope.create_element("div");
                b.set_attribute("class", "probe-right");
                b.set_attribute(
                    "style",
                    &format!("width: {right_width}px; height: 20px; flex-shrink: 0;"),
                );
                b
            });
            r
        });

        BorderlessWindow {
            title: title.to_string(),
            left_section: Some(left),
            right_section: right,
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

/// The one node carrying `class` exactly.
fn node_of(app: &RinchApp, class: &str) -> usize {
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
    matches[0]
}

/// The painted box of the one node carrying `class` exactly.
fn painted(app: &RinchApp, class: &str) -> (f32, f32, f32, f32) {
    let id = node_of(app, class);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    crate::app::hit_testing::painted_element_box(&d.tree, id)
}

/// Write one inline declaration on the node carrying `class`, as an app's
/// own `style:` would.
fn restyle(app: &RinchApp, class: &str, property: &str, value: &str) {
    let id = node_of(app, class);
    let doc = app.doc.as_ref().unwrap();
    doc.borrow_mut()
        .set_style(rinch_core::dom::NodeId(id), property, value);
}

/// The inline `style` attribute of the node carrying `class`.
fn inline_style(app: &RinchApp, class: &str) -> String {
    let id = node_of(app, class);
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[id]
        .attributes
        .get("style")
        .cloned()
        .unwrap_or_default()
}

/// Settle after a change and demand the loop goes idle.
#[track_caller]
fn settle_and_idle(app: &mut RinchApp) {
    drive(app);
    let (redraws, _) = perf_expect::idle_turns(app, 5);
    assert_eq!(redraws, 0, "the loop must go idle once the spacer settled");
}

/// The spacer's right edge lands on the row's right edge (within the layout's
/// own pixel rounding).
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

/// Following the row costs one extra layout, then nothing: once the spacer
/// matches the row the loop goes idle. A spacer whose width fed back into the
/// row it measures would ask for a frame on every turn.
#[test]
fn the_spacer_settles_and_the_loop_goes_idle() {
    let mut app = mount(&["ファイル", "Правка"], "Rinch Zoo");
    assert_spacer_reserves_the_row(&app);
    let (redraws, _) = perf_expect::idle_turns(&mut app, 5);
    assert_eq!(
        redraws, 0,
        "a settled inline menu bar must not keep asking for frames"
    );
}

// ---- #1375: the layout frame, the row's offset, and the first frame ----

const ROW: &str = "rinch-app-menu-bar__inline-row";
const LAYER: &str = "rinch-app-menu-bar__inline-layer";
const SPACER: &str = "rinch-borderlesswindow__menu-spacer";
const TITLEBAR: &str = "rinch-borderlesswindow__titlebar";
const WINDOW: &str = "rinch-borderlesswindow";

/// A window drawn under a transform reserves the same **layout** width. The
/// spacer's width is a CSS length, so it has to be worked out in the frame
/// CSS lays out in; subtracting painted (scaled) boxes and writing the result
/// back as a width reserved half the row under `scale(0.5)` (row right edge
/// 329, spacer right edge 267 — review of #1373).
#[test]
fn a_scaled_window_reserves_the_whole_row() {
    let mut app = mount(&["Файл", "Правка"], "Rinch Zoo");
    let unscaled = painted(&app, SPACER).2;
    restyle(&app, WINDOW, "transform", "scale(0.5)");
    settle_and_idle(&mut app);
    let (_, _, rw, _) = painted(&app, ROW);
    let (_, _, sw, _) = painted(&app, SPACER);
    assert!(
        (sw - unscaled / 2.0).abs() < 0.51,
        "positive control: the spacer is painted at half size ({sw} of {unscaled})"
    );
    assert!(rw > 25.0, "positive control: the row is laid out ({rw})");
    assert_spacer_reserves_the_row(&app);
}

/// Not only a uniform shrink: a magnified, off-centre window too.
#[test]
fn a_magnified_window_reserves_the_whole_row() {
    let mut app = mount(&["ファイル", "編集"], "Zoo");
    restyle(&app, WINDOW, "transform-origin", "10px 0px");
    restyle(&app, WINDOW, "transform", "translate(13px, 5px) scale(1.5)");
    settle_and_idle(&mut app);
    assert_spacer_reserves_the_row(&app);
}

/// The row need not start at the window's left edge: an app that moves the
/// row itself (`left: 30px`) moves the edge the spacer has to reach. Every
/// other fixture has the row at x = 0, where dropping its x changes nothing.
#[test]
fn a_row_offset_inside_its_layer_is_reserved() {
    let mut app = mount(&["Файл", "編集"], "");
    let before = painted(&app, SPACER).2;
    restyle(&app, ROW, "left", "30px");
    settle_and_idle(&mut app);
    assert_eq!(
        painted(&app, ROW).0,
        30.0,
        "positive control: the row moved"
    );
    assert_spacer_reserves_the_row(&app);
    let after = painted(&app, SPACER).2;
    assert!(
        (after - before - 30.0).abs() < 1.0,
        "the spacer grows by the row's offset: {before} -> {after}"
    );
}

/// Nor need the layer that holds the row: the layer's own offset counts.
#[test]
fn a_layer_offset_inside_the_window_is_reserved() {
    let mut app = mount(&["Файл", "編集"], "");
    restyle(&app, LAYER, "left", "17px");
    settle_and_idle(&mut app);
    assert_eq!(
        painted(&app, ROW).0,
        17.0,
        "positive control: the row moved"
    );
    assert_spacer_reserves_the_row(&app);
}

/// A window with its own left padding starts the titlebar — and so the
/// spacer — further right, while the row's layer stays at the padding edge.
/// The spacer then needs less than the row's width.
#[test]
fn a_padded_window_reserves_only_what_is_left_of_the_row() {
    let mut app = mount(&["Файл", "編集"], "Rinch Zoo");
    restyle(&app, WINDOW, "padding-left", "40px");
    settle_and_idle(&mut app);
    let (rx, _, rw, _) = painted(&app, ROW);
    let (sx, _, sw, _) = painted(&app, SPACER);
    assert!(sx >= 40.0, "positive control: the titlebar moved ({sx})");
    assert!(
        sw < rx + rw - 39.0,
        "the spacer is shorter than the row by the padding: row {rw}, spacer {sw}"
    );
    assert_spacer_reserves_the_row(&app);
}

/// A spacer that starts right of the row's end reserves nothing — and says
/// so as `0px`, never as a negative width. Stylo drops a negative `width`
/// and falls back to the stylesheet's zero, so the box alone cannot tell the
/// two apart; a browser's `style.setProperty` ignores one and keeps the
/// previous value, which would leave the estimate standing.
#[test]
fn a_spacer_past_the_rows_end_is_zero_not_negative() {
    let mut app = mount(&["File"], "");
    restyle(&app, WINDOW, "padding-left", "400px");
    settle_and_idle(&mut app);
    let (rx, _, rw, _) = painted(&app, ROW);
    let (sx, _, sw, _) = painted(&app, SPACER);
    assert!(
        rw > 50.0 && sx > rx + rw,
        "positive control: the spacer starts past the row (row ends {}, spacer at {sx})",
        rx + rw
    );
    assert_eq!(sw, 0.0);
    let style = inline_style(&app, SPACER);
    assert!(
        style.contains("width: 0px"),
        "the spacer's inline width must be `0px`, got `{style}`"
    );
}

/// The first painted frame reserves the shell's estimate — the row has no
/// box when that frame is laid out — and exactly one more frame replaces it.
/// Sampled with an estimate (173) that is neither zero nor near the row.
#[test]
fn the_first_frame_reserves_the_estimate_and_the_second_the_row() {
    let mut app = build(&["Файл", "Правка"], "Rinch Zoo", 173, 0);
    assert!(
        inline_style(&app, SPACER).contains("width: 173px"),
        "before any layout the spacer carries the estimate: `{}`",
        inline_style(&app, SPACER)
    );
    perf_expect::paint(&mut app);
    assert_eq!(
        painted(&app, SPACER).2,
        173.0,
        "the first frame is laid out with the estimate"
    );
    let row = painted(&app, ROW).2;
    assert!(
        (row - 173.0).abs() > 20.0,
        "positive control: the estimate is not the row's width ({row})"
    );
    assert!(
        perf_expect::about_to_wait(&mut app),
        "the measured row owes one more frame"
    );
    perf_expect::paint(&mut app);
    assert_spacer_reserves_the_row(&app);
    let (redraws, _) = perf_expect::idle_turns(&mut app, 5);
    assert_eq!(redraws, 0, "one extra frame, then idle");
}

/// Where the estimate shows: a titlebar too narrow for what it holds
/// overflows, so the spacer's width pushes the right section. Between the
/// first frame and the settled one the right section moves by the estimate's
/// error and no further, and the loop still settles and idles.
#[test]
fn a_narrow_titlebar_jumps_by_the_estimate_error_and_no_more() {
    let labels: &'static [&'static str] = &["Файл", "Правка", "Вид", "Справка"];
    let estimate = crate::app_builder::inline_spacer_width(labels);
    let mut app = build(labels, "", estimate, 40);
    // The helpers lay out at one viewport size, so the window is narrowed
    // itself, before its first frame.
    restyle(&app, WINDOW, "width", "300px");

    perf_expect::paint(&mut app);
    let first_spacer = painted(&app, SPACER);
    let first_right = painted(&app, "probe-right").0;
    assert_eq!(first_spacer.2, estimate as f32);

    let frames = drive(&mut app);
    assert!(frames <= 2, "settles in one extra frame, took {frames}");
    let spacer = painted(&app, SPACER);
    let right = painted(&app, "probe-right").0;
    assert_spacer_reserves_the_row(&app);

    let (tx, _, tw, _) = painted(&app, TITLEBAR);
    assert!(
        right + 40.0 > tx + tw,
        "positive control: the titlebar overflows, so the right section is \
         pushed by the spacer (right section ends {}, titlebar ends {})",
        right + 40.0,
        tx + tw
    );
    let error = (first_spacer.2 - spacer.2).abs();
    let jump = (first_right - right).abs();
    assert!(
        error > 4.0,
        "positive control: the estimate is off by a visible amount ({error})"
    );
    assert!(
        (jump - error).abs() < 0.01,
        "the right section jumps by the estimate's error ({error}), not {jump}"
    );

    let (redraws, _) = perf_expect::idle_turns(&mut app, 5);
    assert_eq!(redraws, 0, "a settled narrow titlebar asks for no frames");
}
