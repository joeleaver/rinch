//! Read-only selection against Chrome 156 (the review of #2229): what a copy
//! holds (`getSelection().toString()` of the same markup), what is
//! highlighted, where a drag over empty space reaches, and that a move writes
//! only what changed.

use super::hit_testing::painted_element_box;
use super::text_selection_across_tests::{at, drag, highlighted, mount, node, press, release};
use super::*;

const VP: (u32, u32) = (800, 600);
const SEL: &str = "user-select: text; font: 16px/20px sans-serif; width: 600px";

fn select(app: &mut RinchApp, a: (f32, f32), b: (f32, f32)) -> String {
    press(app, a);
    drag(app, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0));
    drag(app, b);
    release(app, b);
    app.text_selection_text().unwrap_or_default()
}

/// The point past the end of `#id`'s last line, inside its box.
fn past_end(app: &RinchApp, id: &str) -> (f32, f32) {
    let n = node(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    let (x, y, w, h) = painted_element_box(&d.tree, n);
    (x + w - 1.0, y + h - 5.0)
}

fn ranges(app: &RinchApp, id: &str) -> Option<String> {
    let n = node(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(n)?
        .attributes
        .get("data-text-sel-ranges")
        .cloned()
}

#[test]
fn paragraphs_copy_with_a_blank_line_between() {
    let mut app = mount(
        r#"<p id="a">first paragraph</p><p id="b">second paragraph</p><div><p id="c">third paragraph</p></div>"#,
        SEL,
    );
    let t = {
        let (a, b) = (at(&app, "a", 0.0), past_end(&app, "c"));
        select(&mut app, a, b)
    };
    assert_eq!(t, "first paragraph\n\nsecond paragraph\n\nthird paragraph");
}

#[test]
fn table_cells_copy_with_tabs_and_rows_with_line_breaks() {
    let mut app = mount(
        r#"<table><tr><td id="c1">left cell</td><td id="c2">right cell</td></tr><tr><td id="c3">row two a</td><td id="c4">row two b</td></tr></table>"#,
        SEL,
    );
    let t = {
        let (a, b) = (at(&app, "c1", 0.0), past_end(&app, "c4"));
        select(&mut app, a, b)
    };
    assert_eq!(t, "left cell\tright cell\nrow two a\trow two b");
}

#[test]
fn lists_line_breaks_and_hidden_text_copy_as_chrome_copies_them() {
    // No bullets; a <br> is a line break; hidden text is left out, the
    // spaces around it kept.
    let mut app = mount(
        r#"<ul><li id="l1">one</li><li id="l2">two</li></ul><p id="br">x<br>y</p><p id="h">a <span style="visibility:hidden">hid</span> b</p><p id="last">end</p>"#,
        SEL,
    );
    let t = {
        let (a, b) = (at(&app, "l1", 0.0), past_end(&app, "last"));
        select(&mut app, a, b)
    };
    assert_eq!(t, "one\ntwo\nx\ny\n\na  b\n\nend");
}

#[test]
fn an_unselectable_span_is_neither_highlighted_nor_copied() {
    let mut app = mount(
        r#"<p id="a">keep <span style="user-select: none">skip</span> keep</p>"#,
        SEL,
    );
    let t = {
        let (a, b) = (at(&app, "a", 0.0), past_end(&app, "a"));
        select(&mut app, a, b)
    };
    assert_eq!(t, "keep  keep");
    assert_eq!(ranges(&app, "a").as_deref(), Some("0-5,9-14"));
}

#[test]
fn an_unselectable_paragraph_between_still_counts_as_a_paragraph_left() {
    let mut app = mount(
        r#"<p id="a">first</p><p id="b" style="user-select: none">skipped</p><p id="c">third</p>"#,
        SEL,
    );
    let t = {
        let (a, b) = (at(&app, "a", 0.0), past_end(&app, "c"));
        select(&mut app, a, b)
    };
    assert_eq!(t, "first\n\n\nthird");
}

#[test]
fn a_user_select_all_element_is_taken_whole() {
    let mut app = mount(
        r#"<p id="a">alpha beta</p><p id="b">x <span id="s" style="user-select: all">ALL UNIT</span> y</p>"#,
        SEL,
    );
    // "x " is about 12px: 30px into #b is on "ALL".
    let in_all = {
        let (x, y) = at(&app, "b", 0.0);
        (x + 30.0, y)
    };
    let t = {
        let a = at(&app, "a", 0.0);
        select(&mut app, a, in_all)
    };
    assert_eq!(t, "alpha beta\n\nx ALL UNIT");
    // A press inside it selects it, too.
    let s = in_all;
    press(&mut app, s);
    release(&mut app, s);
    assert_eq!(app.text_selection_text().as_deref(), Some("ALL UNIT"));
}

#[test]
fn a_drag_into_the_space_beside_a_cell_reaches_that_cell() {
    let mut app = mount(
        r#"<table style="width:400px"><tr><td id="c1" style="width:150px">left cell</td><td id="c2" style="width:150px">right cell</td></tr></table>"#,
        SEL,
    );
    let (x, y, w, _) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, node(&app, "c2"))
    };
    let t = {
        let a = at(&app, "c1", 0.0);
        select(&mut app, a, (x + w + 20.0, y + 10.0))
    };
    assert_eq!(t, "left cell\tright cell");
}

#[test]
fn a_drag_from_a_pre_into_the_margin_below_the_next_paragraph_reaches_it() {
    use rinch_core::Component;
    let mut app = RinchApp::new(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "font: 16px/20px sans-serif; width: 600px");
        let view = rinch_html::HtmlView {
            html: r#"<pre id="pre">code line</pre><p id="p">after the code</p><p id="q">more</p>"#
                .into(),
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&view);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (x, py, _, ph) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, node(&app, "p"))
    };
    // 6px into the 16px margin below #p: nearer #p than #q.
    let t = {
        let a = at(&app, "pre", 0.0);
        select(&mut app, a, (x + 5.0, py + ph + 6.0))
    };
    assert_eq!(t, "code line\nafter the code");
}

#[test]
fn a_drag_below_a_container_reaches_the_next_selectable_text_as_a_hit_does() {
    // Chrome does not clamp a selection to the element it started in, over
    // text or over a gap: both reach the next selectable text.
    let mut app = mount(
        r#"<div id="box" style="user-select: text"><p id="a">inside</p></div><div style="height: 40px"></div><p id="b" style="user-select: text">outside</p>"#,
        "font: 16px/20px sans-serif; width: 600px",
    );
    let (x, y, _, h) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, node(&app, "b"))
    };
    let over_text = {
        let a = at(&app, "a", 0.0);
        select(&mut app, a, (x + 30.0, y + h / 2.0))
    };
    let below = {
        let a = at(&app, "a", 0.0);
        select(&mut app, a, (x + 300.0, y + h + 30.0))
    };
    assert!(over_text.starts_with("inside\n\nout"), "{over_text}");
    assert_eq!(below, "inside\n\noutside");
}

#[test]
fn text_that_shrinks_under_a_selection_is_clamped_at_the_next_layout() {
    let mut app = mount(
        r#"<p id="a">first paragraph</p><p id="b">second paragraph text</p>"#,
        SEL,
    );
    {
        let (a, b) = (at(&app, "a", 0.0), at(&app, "b", 0.9));
        select(&mut app, a, b);
    }
    let b = node(&app, "b");
    let text = {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree.get(b).unwrap().children[0]
    };
    rinch_core::dom::DomDocument::set_text_content(
        &mut *app.doc.as_ref().unwrap().borrow_mut(),
        rinch_core::dom::NodeId(text),
        "é",
    );
    app.resolve_and_repaint(VP.0 as f32 + 1.0, VP.1 as f32);
    let (_, end) = highlighted(&app, "b").expect("still selected");
    assert!(end <= "é".len(), "end {end} past the text");
}

#[test]
fn a_move_writes_only_the_roots_whose_part_changed() {
    let mut html = String::new();
    for i in 0..300 {
        html.push_str(&format!(
            r#"<p id="p{i}" style="margin:0">paragraph number {i} with some text</p>"#
        ));
    }
    let html: &'static str = Box::leak(html.into_boxed_str());
    let vp = (800u32, 6_100u32);
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", SEL);
        root.set_inner_html(html);
        root
    });
    app.mount_component(vp.0 as f32, vp.1 as f32);
    app.resolve_and_repaint(vp.0 as f32, vp.1 as f32);
    let (p0, last) = (at(&app, "p0", 0.0), at(&app, "p299", 0.5));
    let button = MouseButton::Left;
    app.handle_event(
        PlatformEvent::MouseDown {
            x: p0.0,
            y: p0.1,
            button,
        },
        vp,
        1.0,
    );
    app.handle_event(
        PlatformEvent::MouseMove {
            x: last.0,
            y: last.1,
        },
        vp,
        1.0,
    );
    let writes = |app: &RinchApp| {
        app.doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .perf
            .get(rinch_dom::perf::Counter::ReadOnlySelectionWrites)
    };
    let before = writes(&app);
    for i in 0..20 {
        app.handle_event(
            PlatformEvent::MouseMove {
                x: last.0 + i as f32,
                y: last.1,
            },
            vp,
            1.0,
        );
    }
    let per_move = (writes(&app) - before) as f32 / 20.0;
    assert!(
        per_move <= 1.0,
        "{per_move} roots written per move, of 300 selected"
    );
}

// ── Where an inline click lands (the walk starts at the text under it) ──

/// An app whose paragraph holds `html`; each element with an `id` in `ids`
/// gets an `onclick` logging its id.
fn clicks(
    html: &'static str,
    ids: &'static [&'static str],
) -> (RinchApp, Rc<RefCell<Vec<&'static str>>>) {
    let log: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let l = log.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "font: 16px/20px sans-serif; width: 600px");
        root.set_inner_html(html);
        for &id in ids {
            let l = l.clone();
            let h = scope.register_handler(move || l.borrow_mut().push(id));
            if let Some(el) = root.query_selector(&format!("#{id}")) {
                el.set_attribute("data-rid", &h.0.to_string());
            }
        }
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, log)
}

fn click_at(app: &mut RinchApp, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
}

#[test]
fn an_inline_click_reaches_the_innermost_handler_on_the_text() {
    let (mut app, log) = clicks(
        r#"<p id="p"><a id="a" href="https://x.example/"><span id="s">inner</span> link</a> and text</p>"#,
        &["p", "a", "s"],
    );
    let (x, y) = at(&app, "p", 0.0);
    click_at(&mut app, (x + 4.0, y)); // "inner"
    click_at(&mut app, (x + 55.0, y)); // "link"
    click_at(&mut app, (x + 110.0, y)); // "and text"
    assert_eq!(*log.borrow(), vec!["s", "a", "p"]);
}

#[test]
fn a_click_past_the_line_end_or_between_lines_is_the_paragraphs() {
    let (mut app, log) = clicks(
        r#"<p id="p" style="line-height: 40px">ends in <a id="a" href="https://x.example/">a link</a></p>"#,
        &["p", "a"],
    );
    let (x, y, w, _) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, node(&app, "p"))
    };
    click_at(&mut app, (x + w - 5.0, y + 20.0)); // past the line end
    click_at(&mut app, (x + 60.0, y + 2.0)); // above the glyphs, in the line box
    assert_eq!(*log.borrow(), vec!["p", "p"]);
}

#[test]
fn an_inline_block_in_a_paragraph_takes_its_own_click() {
    let (mut app, log) = clicks(
        r#"<p id="p">before <span id="ib" style="display: inline-block; width: 80px">block</span> after</p>"#,
        &["p", "ib"],
    );
    let ib = at(&app, "ib", 0.3);
    click_at(&mut app, ib);
    let (x, y) = at(&app, "p", 0.0);
    click_at(&mut app, (x + 4.0, y));
    assert_eq!(*log.borrow(), vec!["ib", "p"]);
}

#[test]
fn text_that_takes_no_pointer_events_gives_its_click_to_the_paragraph() {
    // Chrome 156: a click on `pointer-events: none` or hidden text inside a
    // paragraph is the paragraph's.
    let (mut app, log) = clicks(
        r#"<p id="p"><span id="pe" style="pointer-events: none">this</span> and <span id="hv" style="visibility: hidden">that</span></p>"#,
        &["p", "pe", "hv"],
    );
    let (x, y) = at(&app, "p", 0.0);
    click_at(&mut app, (x + 6.0, y)); // "this"
    click_at(&mut app, (x + 70.0, y)); // "that"
    assert_eq!(*log.borrow(), vec!["p", "p"]);
}

#[test]
fn a_paragraph_that_is_not_displayed_is_not_a_paragraph_left() {
    let mut app = mount(
        r#"<p id="a">first</p><p style="display: none">gone</p><p id="c">third</p>"#,
        SEL,
    );
    let t = {
        let (a, b) = (at(&app, "a", 0.0), past_end(&app, "c"));
        select(&mut app, a, b)
    };
    assert_eq!(t, "first\n\nthird");
}

#[test]
fn a_visible_child_of_a_hidden_span_is_copied() {
    let mut app = mount(
        r#"<p id="a">a <span style="visibility: hidden">h <span style="visibility: visible">V</span> h</span> b</p>"#,
        SEL,
    );
    let t = {
        let (a, b) = (at(&app, "a", 0.0), past_end(&app, "a"));
        select(&mut app, a, b)
    };
    assert_eq!(t, "a V b");
}
