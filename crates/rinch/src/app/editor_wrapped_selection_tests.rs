//! Extending a selection on a soft-wrapped paragraph, on desktop (reported
//! from Pimble, 2026-10-01: "start on the end of a wrapped line, hold shift,
//! try Home, Ctrl+Left, Up: what gets selected/highlighted isn't right").
//!
//! The behaviour pinned, as in Chrome, Google Docs and Word:
//! - Shift never moves the anchor.
//! - Home / End are the edges of the head's **visual** line, judged with the
//!   caret affinity it has: the end of line N and the start of line N+1 are one
//!   model position, and a caret that arrived from line N's side (End, a press
//!   past its end, a vertical move onto it) belongs to line N.
//! - Up / Down move to the neighbouring visual line at the goal column.
//! - The highlight covers exactly the selected text on each visual line it
//!   touches: from the caret at its start to the caret at its end, a line the
//!   range runs past reaching that line's trailing edge (#1010), and nothing
//!   on a line the range only touches at a wrap point. No caret is drawn over
//!   a range; a collapsed one is drawn on its affinity's line.
//!
//! The model side of every sequence here was already right; the report's
//! symptom is #1010 (each line's highlight was a 1px sliver at the line's
//! start, fixed by #1108), which these fixtures fail on before it. What they
//! found besides is that the overlays were drawn one border width right of and
//! below the glyphs (`overlays_sit_on_the_glyphs_inside_a_bordered_editor`).
//!
//! The face is the bundled Inter at a declared 16px / 24px. Line starts and
//! caret positions are measured from the layout, never hard-coded.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_core::dom::CaretAffinity;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const LINE: f32 = 24.0;

/// Wraps at spaces, which hang at the line ends.
const WORDS: &str = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike \
                     november oscar papa quebec romeo sierra tango";
/// One word broken by `overflow-wrap: anywhere`: a glyph wrap, no hanging space.
const WORD: &str =
    "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghij";

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    text: &'static str,
    /// Char offsets each visual line starts at, plus the text's length.
    starts: Vec<u32>,
}

fn idle(app: &mut RinchApp) {
    for _ in 0..3 {
        let actions = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            if app.has_pending_layout() {
                app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            }
            let _ = app.build_pixels(1.0, VP, false);
        }
    }
}

/// `text` as the first of two paragraphs in an editor styled `editor_style`,
/// focused and settled.
fn page_with(text: &'static str, p_style: &str, editor_style: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(&format!("<p style=\"{p_style}\">{text}</p><p>after</p>")));
    let handle_in = handle.clone();
    let editor_style = editor_style.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute("style", &editor_style);
        root.append_child(&editor);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page {
        app,
        handle,
        text,
        starts: Vec::new(),
    };
    page.handle.focus();
    idle(&mut page.app);
    assert!(matches!(page.app.focus_target, FocusTarget::Editor(_)));
    page.starts = page.measure_line_starts();
    assert!(
        page.starts.len() >= 6,
        "positive control: the paragraph wraps to 5+ lines: {:?}",
        page.starts
    );
    page
}

fn page(text: &'static str, p_style: &str) -> Page {
    page_with(
        text,
        p_style,
        "width: 180px; height: 400px; font-size: 16px; line-height: 24px; \
         font-family: sans-serif; border: none",
    )
}

fn press(app: &mut RinchApp, key: KeyCode, shift: bool, ctrl: bool) {
    let modifiers = Modifiers {
        shift,
        ctrl,
        ..Default::default()
    };
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
        VP,
        1.0,
    );
    idle(app);
}

/// The plain (unshifted) key.
fn key(p: &mut Page, key: KeyCode) {
    press(&mut p.app, key, false, false);
}

/// Shift + the key.
fn shift(p: &mut Page, key: KeyCode) {
    press(&mut p.app, key, true, false);
}

/// Shift + the word modifier + the key.
fn shift_word(p: &mut Page, key: KeyCode) {
    press(&mut p.app, key, true, true);
}

/// A painted box, `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

impl Page {
    fn block(&self) -> usize {
        self.handle.caret_address(Pos(1)).unwrap().0
    }

    fn n(&self) -> u32 {
        self.text.chars().count() as u32
    }

    /// Layout-local caret `(x, y)` at char offset `i` on `affinity`'s side.
    fn local(&self, i: u32, affinity: CaretAffinity) -> (f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_caret_position_with_affinity(self.block() as u64, i as usize, affinity)
            .expect("laid out")
    }

    fn measure_line_starts(&self) -> Vec<u32> {
        let mut starts = vec![0];
        let mut y = self.local(0, CaretAffinity::Downstream).1;
        for i in 1..self.n() {
            let yi = self.local(i, CaretAffinity::Downstream).1;
            if yi > y + 1.0 {
                starts.push(i);
                y = yi;
            }
        }
        starts.push(self.n());
        starts
    }

    /// The visual line holding char offset `i` on `affinity`'s side.
    fn line_of(&self, i: u32, affinity: CaretAffinity) -> usize {
        let lines = self.starts.len() - 1;
        (0..lines)
            .rev()
            .find(|&l| {
                let s = self.starts[l];
                i > s || (i == s && (l == 0 || affinity == CaretAffinity::Downstream))
            })
            .unwrap_or(0)
    }

    fn para_box(&self) -> Rect {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&doc.tree, self.block())
    }

    /// The visual line the caret overlay is drawn on, `None` when hidden.
    fn caret_line(&self) -> Option<usize> {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let id = doc
            .query_selector_all("[data-pm-caret]")
            .into_iter()
            .next()?;
        let cs = &doc.tree.nodes[id.0].computed_style;
        if cs.display == rinch_dom::computed_style::DisplayValue::None
            || cs.visibility != rinch_dom::computed_style::VisibilityValue::Visible
        {
            return None;
        }
        let (_, y, _, h) = painted_element_box(&doc.tree, id.0);
        drop(doc);
        Some(((y + h / 2.0 - self.para_box().1) / LINE).floor() as usize)
    }

    /// The painted highlight rects, paragraph-local, top to bottom.
    fn highlights(&self) -> Vec<Rect> {
        let (bx, by, _, _) = self.para_box();
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let mut v: Vec<Rect> = doc
            .query_selector_all("[data-pm-selection]")
            .into_iter()
            .filter(|id| {
                doc.tree.nodes[id.0].computed_style.display
                    != rinch_dom::computed_style::DisplayValue::None
            })
            .map(|id| painted_element_box(&doc.tree, id.0))
            .filter(|b| b.2 > 0.0 && b.3 > 0.0)
            .map(|(x, y, w, h)| (x - bx, y - by, w, h))
            .collect();
        v.sort_by(|a, b| a.1.total_cmp(&b.1));
        v
    }

    /// What the highlight of chars `[a, b)` should be, paragraph-local: per
    /// visual line it touches, from the downstream caret at its start to the
    /// caret at its end (upstream when that is the line's end).
    fn expected_highlights(&self, a: u32, b: u32) -> Vec<(f32, f32, usize)> {
        let lines = self.starts.len() - 1;
        let mut out = Vec::new();
        for l in 0..lines {
            let (s, e) = (self.starts[l], self.starts[l + 1]);
            let (from, to) = (a.max(s), b.min(e));
            if from >= to {
                continue;
            }
            let x0 = self.local(from, CaretAffinity::Downstream).0;
            let end_affinity = if to == e {
                CaretAffinity::Upstream
            } else {
                CaretAffinity::Downstream
            };
            let x1 = self.local(to, end_affinity).0;
            out.push((x0, x1, l));
        }
        out
    }

    /// `(anchor, head)` as char offsets.
    fn sel(&self) -> (u32, u32) {
        let s = self.handle.selection();
        ((s.anchor().0 - 1) as u32, (s.head().0 - 1) as u32)
    }

    fn caret_at(&mut self, i: u32) {
        self.handle
            .set_selection(Selection::cursor(Pos(i as usize + 1)));
        idle(&mut self.app);
    }

    /// A fresh single press at paragraph-local `(x, y)`.
    fn click(&mut self, x: f32, y: f32) {
        let (bx, by, _, _) = self.para_box();
        let (x, y) = (bx + x, by + y);
        // Not the second click of a double click.
        self.app.click_count = 0;
        self.app.last_click_time = Instant::now() - std::time::Duration::from_secs(10);
        let button = MouseButton::Left;
        self.app
            .handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
        self.app
            .handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
        self.app
            .handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
        idle(&mut self.app);
    }

    /// The selection is `(anchor, head)`, drawn as it should be: the highlight
    /// covers exactly `[min, max)` per visual line, and the caret is hidden
    /// over a range and on the head's line (by its affinity) when collapsed.
    #[track_caller]
    fn assert_sel(&self, step: &str, anchor: u32, head: u32) {
        assert_eq!(self.sel(), (anchor, head), "{step}: (anchor, head)");
        let got = self.highlights();
        let want = self.expected_highlights(anchor.min(head), anchor.max(head));
        assert_eq!(
            got.len(),
            want.len(),
            "{step}: one highlight per visual line the range covers; got {got:?}, \
             want (x0, x1, line) {want:?}"
        );
        for (g, (x0, x1, l)) in got.iter().zip(&want) {
            let line = ((g.1 + g.3 / 2.0) / LINE).floor() as usize;
            assert_eq!(line, *l, "{step}: highlight {g:?} on line {l}");
            assert!(
                (g.0 - x0).abs() <= 0.75 && (g.0 + g.2 - x1).abs() <= 1.0,
                "{step}: line {l}'s highlight spans {}..{}, the selected text {x0}..{x1}",
                g.0,
                g.0 + g.2
            );
        }
        let want_caret = (anchor == head).then(|| self.line_of(head, self.handle.caret_affinity()));
        assert_eq!(self.caret_line(), want_caret, "{step}: the caret's line");
    }
}

/// The char offset Ctrl+Left from `i` reaches: the start of the word before.
fn word_start_before(text: &str, i: u32) -> u32 {
    let chars: Vec<char> = text.chars().collect();
    let mut j = i as usize;
    while j > 0 && chars[j - 1] == ' ' {
        j -= 1;
    }
    while j > 0 && chars[j - 1] != ' ' {
        j -= 1;
    }
    j as u32
}

/// Put the caret at the end of visual line 1 (upstream at the wrap point
/// `starts[2]`) by `how`, and check it is drawn there.
fn caret_at_end_of_line_1(p: &mut Page, how: &str) -> u32 {
    let w = p.starts[2];
    match how {
        "End" => {
            p.caret_at(p.starts[1] + 3);
            key(p, KeyCode::End);
        }
        "press past the end" => {
            // Beside the line's end, in the editor's padding.
            let end = p.local(w, CaretAffinity::Upstream).0;
            p.click(end + 4.0, 1.5 * LINE);
        }
        _ => unreachable!(),
    }
    assert_eq!(p.handle.caret_affinity(), CaretAffinity::Upstream, "{how}");
    p.assert_sel(&format!("{how} on line 1"), w, w);
    w
}

/// Put the caret at the start of visual line 2 (downstream at the wrap point
/// `starts[2]`) by `how`.
fn caret_at_start_of_line_2(p: &mut Page, how: &str) -> u32 {
    let w = p.starts[2];
    match how {
        "Home" => {
            p.caret_at(w + 3);
            key(p, KeyCode::Home);
        }
        "press at the start" => p.click(1.0, 2.5 * LINE),
        _ => unreachable!(),
    }
    p.assert_sel(&format!("{how} on line 2"), w, w);
    w
}

/// Every Shift chord from the end of a wrapped line: the head moves, the
/// anchor stays at the wrap point, and the highlight is the text between.
fn from_the_end_of_a_wrapped_line(p: &mut Page) {
    let s = p.starts.clone();
    for how in ["End", "press past the end"] {
        // Shift+Home selects line 1; again stays; Shift+End comes back to an
        // empty selection drawn at line 1's end.
        let w = caret_at_end_of_line_1(p, how);
        shift(p, KeyCode::Home);
        p.assert_sel("Shift+Home", w, s[1]);
        shift(p, KeyCode::Home);
        p.assert_sel("Shift+Home again", w, s[1]);
        shift(p, KeyCode::End);
        p.assert_sel("then Shift+End", w, w);
        assert_eq!(p.handle.caret_affinity(), CaretAffinity::Upstream);

        // Shift+End at a line's end stays.
        let w = caret_at_end_of_line_1(p, how);
        shift(p, KeyCode::End);
        p.assert_sel("Shift+End at the end", w, w);

        // Shift+Ctrl+Left selects the last word of line 1 (or, in one broken
        // word, back to its start); Shift+Home then takes the rest of the line.
        let w = caret_at_end_of_line_1(p, how);
        let ws = word_start_before(p.text, w);
        shift_word(p, KeyCode::ArrowLeft);
        p.assert_sel("Shift+Ctrl+Left", w, ws);
        shift(p, KeyCode::Home);
        p.assert_sel("then Shift+Home", w, ws.min(s[1]));

        // Shift+Left takes line 1's last char; Shift+Right gives it back, and
        // the next takes line 2's first char, not line 1's end.
        let w = caret_at_end_of_line_1(p, how);
        shift(p, KeyCode::ArrowLeft);
        p.assert_sel("Shift+Left", w, w - 1);
        shift(p, KeyCode::ArrowRight);
        p.assert_sel("then Shift+Right", w, w);
        shift(p, KeyCode::ArrowRight);
        p.assert_sel("then Shift+Right again", w, w + 1);

        // Shift+Up goes to line 0, Shift+Down back, then on to line 2: the
        // goal column is line 1's end, kept through the run.
        let w = caret_at_end_of_line_1(p, how);
        shift(p, KeyCode::ArrowUp);
        let (_, h) = p.sel();
        assert!(h <= s[1], "Shift+Up lands on line 0: {h}");
        p.assert_sel("Shift+Up", w, h);
        shift(p, KeyCode::ArrowDown);
        p.assert_sel("then Shift+Down", w, w);
        shift(p, KeyCode::ArrowDown);
        let (_, h) = p.sel();
        assert!(
            h > s[2] && h <= s[3],
            "Shift+Down from line 1's end lands on line 2: {h} in {s:?}"
        );
        p.assert_sel("then Shift+Down again", w, h);
        // Shift+Home from there: line 2's start, which is the anchor.
        shift(p, KeyCode::Home);
        p.assert_sel("then Shift+Home", w, w);

        // Shift+Up, Shift+End: the end of line 0.
        let w = caret_at_end_of_line_1(p, how);
        shift(p, KeyCode::ArrowUp);
        shift(p, KeyCode::End);
        p.assert_sel("Shift+Up, Shift+End", w, s[1]);
        shift(p, KeyCode::Home);
        p.assert_sel("then Shift+Home", w, 0);
    }
}

/// Every Shift chord from the start of a wrapped (non-first) line.
fn from_the_start_of_a_wrapped_line(p: &mut Page) {
    let s = p.starts.clone();
    for how in ["Home", "press at the start"] {
        // Shift+Home at a line's start stays there (not line 1's start);
        // Shift+End selects line 2.
        let w = caret_at_start_of_line_2(p, how);
        shift(p, KeyCode::Home);
        p.assert_sel("Shift+Home at the start", w, w);
        shift(p, KeyCode::End);
        p.assert_sel("then Shift+End", w, s[3]);
        assert_eq!(p.handle.caret_affinity(), CaretAffinity::Upstream);

        // Shift+Left takes line 1's last char.
        let w = caret_at_start_of_line_2(p, how);
        shift(p, KeyCode::ArrowLeft);
        p.assert_sel("Shift+Left", w, w - 1);
        shift(p, KeyCode::End);
        p.assert_sel("then Shift+End", w, w);

        // Shift+Up: line 1's start. Shift+End from there: line 1's end, which
        // is the anchor.
        let w = caret_at_start_of_line_2(p, how);
        shift(p, KeyCode::ArrowUp);
        p.assert_sel("Shift+Up", w, s[1]);
        shift(p, KeyCode::End);
        p.assert_sel("then Shift+End", w, w);

        // Shift+Down: line 3's start, so the whole of line 2; Shift+Home stays
        // there and Shift+End reaches line 3's end.
        let w = caret_at_start_of_line_2(p, how);
        shift(p, KeyCode::ArrowDown);
        p.assert_sel("Shift+Down", w, s[3]);
        shift(p, KeyCode::Home);
        p.assert_sel("then Shift+Home", w, s[3]);
        shift(p, KeyCode::End);
        p.assert_sel("then Shift+End", w, s[4]);

        // Shift+Ctrl+Right then Shift+Ctrl+Left come back to the anchor.
        let w = caret_at_start_of_line_2(p, how);
        shift_word(p, KeyCode::ArrowRight);
        let (_, h) = p.sel();
        assert!(h > w, "Shift+Ctrl+Right moves forward");
        p.assert_sel("Shift+Ctrl+Right", w, h);
    }
}

#[test]
fn extending_from_the_end_of_a_space_wrapped_line() {
    let mut p = page(WORDS, "");
    from_the_end_of_a_wrapped_line(&mut p);
}

#[test]
fn extending_from_the_start_of_a_space_wrapped_line() {
    let mut p = page(WORDS, "");
    from_the_start_of_a_wrapped_line(&mut p);
}

#[test]
fn extending_from_the_end_of_a_glyph_wrapped_line() {
    let mut p = page(WORD, "overflow-wrap: anywhere");
    from_the_end_of_a_wrapped_line(&mut p);
}

#[test]
fn extending_from_the_start_of_a_glyph_wrapped_line() {
    let mut p = page(WORD, "overflow-wrap: anywhere");
    from_the_start_of_a_wrapped_line(&mut p);
}

/// The editor's default stylesheet: padding and a 1px border, which is what
/// moved the overlays.
#[test]
fn extending_in_the_default_editor_style() {
    let mut p = page_with(
        WORDS,
        "",
        "width: 180px; font-size: 16px; line-height: 24px; font-family: sans-serif",
    );
    from_the_end_of_a_wrapped_line(&mut p);
    from_the_start_of_a_wrapped_line(&mut p);
}

/// The caret and the highlight are drawn over the glyphs inside an editor with
/// a border. They are absolutely-positioned children of the editor, so they
/// anchor to its padding box, inside the border, while the offsets they are
/// placed by are summed border-box origins; the desktop's
/// `content_origin_inset` answered `(0, 0)`, so both were drawn one border
/// width right of and below the text (the web subtracted `clientLeft/Top`).
#[test]
fn overlays_sit_on_the_glyphs_inside_a_bordered_editor() {
    for border in ["none", "1px solid red", "5px solid red"] {
        let mut p = page_with(
            WORDS,
            "",
            &format!(
                "width: 180px; font-size: 16px; line-height: 24px; \
                 font-family: sans-serif; border: {border}"
            ),
        );
        p.caret_at(0);
        let (bx, by, _, _) = p.para_box();
        let caret = {
            let doc = p.app.doc.as_ref().unwrap().borrow();
            let id = doc.query_selector_all("[data-pm-caret]")[0];
            painted_element_box(&doc.tree, id.0)
        };
        assert_eq!(
            (caret.0, caret.1),
            (bx, by),
            "border {border}: the caret at the text's start is drawn at the text's origin"
        );
        let w = p.starts[2];
        p.caret_at(p.starts[1]);
        shift(&mut p, KeyCode::End);
        p.assert_sel(&format!("border {border}: Shift+End"), p.starts[1], w);
        let hl = p.highlights();
        assert!(
            hl[0].0.abs() < 0.5 && (hl[0].1 - LINE).abs() < 0.5,
            "border {border}: line 1's highlight starts at its text, not at {hl:?}"
        );
    }
}
