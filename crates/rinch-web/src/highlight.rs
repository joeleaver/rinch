//! The rich-text editor's selection, painted as a browser paints a selection
//! (#2667).
//!
//! The editor draws its selection as `[data-pm-selection]` boxes, `z-index: -1`
//! in its container's stacking context. A browser paints such a layer before
//! every in-flow background of that context, so a code block, a table header,
//! an inline `code` or `mark` with an opaque background hid the selection
//! whole. rinch-dom paints each box with the block it covers
//! (`DomDocument::set_highlight_owner`, #2666); the browser has no such order,
//! so this document paints the selection on what it covers instead, and hides
//! the box:
//!
//! - **a line of text** ([`DomDocument::set_highlight_text`]) as a CSS Custom
//!   Highlight: the range is added to a `Highlight` registered in
//!   `CSS.highlights`, styled by a `::highlight()` rule with the box's computed
//!   `background-color` and nothing else, so it is painted over the text's
//!   backgrounds (the block's, an inline `code`'s or `mark`'s) and under its
//!   glyphs, in their own colour. Where the browser has no `CSS.highlights`
//!   the box stays as it was.
//! - **a table cell's wash** (an owner and no text) as an inset `box-shadow`
//!   on the cell in the same colour, painted over the cell's background and
//!   under its content, as desktop paints the wash. Any shadow the cell had
//!   is kept under it and put back afterwards.
//!
//! The colour is read from the box, so an app restyles the selection with one
//! rule on both backends: `[data-pm-editor] [data-pm-selection] {
//! background-color: … !important }`.
//!
//! [`DomDocument::set_highlight_text`]: rinch_core::dom::DomDocument::set_highlight_text

use std::collections::HashMap;

use rinch_core::dom::HighlightText;
use wasm_bindgen::{JsCast, JsValue};

/// What a highlight call needs from the document.
pub(crate) struct Ctx<'a> {
    pub(crate) doc: &'a web_sys::Document,
    pub(crate) nodes: &'a HashMap<u64, web_sys::Node>,
}

/// The text a box highlights, as [`HighlightText`] names it, owned.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TextKey {
    block: u64,
    start: usize,
    end: usize,
    boxes: Vec<u64>,
}

/// How a box is painted now.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Painted {
    /// As itself.
    Box,
    /// As the range of [`TextKey`] in the highlight of this colour.
    Text(TextKey, String),
    /// As a wash of this colour over the owner element.
    Wash(u64, String),
}

#[derive(Debug)]
struct Overlay {
    owner: Option<u64>,
    text: Option<TextKey>,
    painted: Painted,
}

/// One range in the page's highlight registry, shared by every box (line)
/// that names it.
struct SharedRange {
    range: web_sys::Range,
    highlight: JsValue,
    users: usize,
}

/// A wash on an owner element, shared by every box that names it.
struct Wash {
    el: web_sys::HtmlElement,
    /// The owner's own inline `box-shadow` (value, priority), put back when
    /// the last box lets go.
    saved: (String, String),
    users: usize,
}

/// The boxes of one document painted on what they cover.
#[derive(Default)]
pub(crate) struct OverlayHighlights {
    overlays: HashMap<u64, Overlay>,
    ranges: HashMap<(TextKey, String), SharedRange>,
    washes: HashMap<(u64, String), Wash>,
}

impl OverlayHighlights {
    pub(crate) fn set_owner(&mut self, ctx: &Ctx<'_>, overlay: u64, owner: Option<u64>) {
        let entry = self.overlays.entry(overlay).or_insert(Overlay {
            owner: None,
            text: None,
            painted: Painted::Box,
        });
        entry.owner = owner;
        self.repaint(ctx, overlay);
    }

    /// Whether this document paints text highlights itself.
    pub(crate) fn set_text(
        &mut self,
        ctx: &Ctx<'_>,
        overlay: u64,
        text: Option<HighlightText<'_>>,
    ) -> bool {
        let text = text.map(|t| TextKey {
            block: t.block.0,
            start: t.start,
            end: t.end,
            boxes: t.boxes.to_vec(),
        });
        if text.is_none() && !self.overlays.contains_key(&overlay) {
            return highlights_supported();
        }
        let entry = self.overlays.entry(overlay).or_insert(Overlay {
            owner: None,
            text: None,
            painted: Painted::Box,
        });
        entry.text = text;
        self.repaint(ctx, overlay);
        highlights_supported()
    }

    /// Let go of every box `nodes` no longer holds (a discarded box).
    pub(crate) fn forget_retired(&mut self, nodes: &HashMap<u64, web_sys::Node>) {
        if self.overlays.is_empty() {
            return;
        }
        let gone: Vec<u64> = self
            .overlays
            .keys()
            .copied()
            .filter(|id| !nodes.contains_key(id))
            .collect();
        for id in gone {
            if let Some(o) = self.overlays.remove(&id) {
                self.release(&o.painted);
            }
        }
    }

    /// Let go of every box (the document is dropped).
    pub(crate) fn clear(&mut self) {
        for (_, o) in std::mem::take(&mut self.overlays) {
            self.release(&o.painted);
        }
    }

    /// Paint `overlay` the way its owner and text now ask.
    fn repaint(&mut self, ctx: &Ctx<'_>, overlay: u64) {
        let Some(o) = self.overlays.get(&overlay) else {
            return;
        };
        let Some(el) = ctx
            .nodes
            .get(&overlay)
            .and_then(|n| n.clone().dyn_into::<web_sys::HtmlElement>().ok())
        else {
            return;
        };
        let want = match (&o.text, o.owner) {
            (Some(text), _) if highlights_supported() => {
                Painted::Text(text.clone(), background_color(&el))
            }
            // Without `CSS.highlights` a line of text stays a box.
            (Some(_), _) => Painted::Box,
            (None, Some(owner)) => Painted::Wash(owner, background_color(&el)),
            (None, None) => Painted::Box,
        };
        if want == o.painted {
            if let Painted::Text(text, colour) = want {
                self.refresh_text(ctx, &text, &colour);
            }
            return;
        }
        let acquired = match &want {
            Painted::Box => true,
            Painted::Text(text, colour) => self.acquire_text(ctx, text, colour),
            Painted::Wash(owner, colour) => self.acquire_wash(ctx, *owner, colour),
        };
        let want = if acquired { want } else { Painted::Box };
        let Some(o) = self.overlays.get_mut(&overlay) else {
            return;
        };
        let old = std::mem::replace(&mut o.painted, want.clone());
        self.release(&old);
        let style = el.style();
        if want == Painted::Box {
            let _ = style.remove_property("visibility");
        } else {
            // Its colour is painted on what it covers; the box keeps its
            // geometry (a scroll anchor, the app's own reads of it).
            let _ = style.set_property("visibility", "hidden");
        }
    }

    fn acquire_text(&mut self, ctx: &Ctx<'_>, text: &TextKey, colour: &str) -> bool {
        let key = (text.clone(), colour.to_owned());
        if let Some(shared) = self.ranges.get_mut(&key) {
            shared.users += 1;
            return true;
        }
        let Some(block) = ctx.nodes.get(&text.block) else {
            return false;
        };
        let Some(range) = crate::web_document::selection_range(
            ctx.doc,
            ctx.nodes,
            block,
            text.start,
            text.end,
            &text.boxes,
        ) else {
            return false;
        };
        let Some(highlight) = highlight_for(ctx.doc, colour) else {
            return false;
        };
        if call1(&highlight, "add", &range).is_none() {
            return false;
        }
        self.ranges.insert(
            key,
            SharedRange {
                range,
                highlight,
                users: 1,
            },
        );
        true
    }

    /// Build the range of `text` again where the document moved it: a
    /// rebuilt run (a mark toggled over the selection) leaves a live range
    /// collapsed in the node that replaced it.
    fn refresh_text(&mut self, ctx: &Ctx<'_>, text: &TextKey, colour: &str) {
        let Some(shared) = self.ranges.get_mut(&(text.clone(), colour.to_owned())) else {
            return;
        };
        let Some(block) = ctx.nodes.get(&text.block) else {
            return;
        };
        let Some(fresh) = crate::web_document::selection_range(
            ctx.doc,
            ctx.nodes,
            block,
            text.start,
            text.end,
            &text.boxes,
        ) else {
            return;
        };
        let old = &shared.range;
        let same = |a: Result<web_sys::Node, JsValue>, b: Result<web_sys::Node, JsValue>| matches!((a, b), (Ok(a), Ok(b)) if a.is_same_node(Some(&b)));
        if same(old.start_container(), fresh.start_container())
            && same(old.end_container(), fresh.end_container())
            && old.start_offset().ok() == fresh.start_offset().ok()
            && old.end_offset().ok() == fresh.end_offset().ok()
        {
            return;
        }
        let _ = call1(&shared.highlight, "delete", &shared.range);
        if call1(&shared.highlight, "add", &fresh).is_some() {
            shared.range = fresh;
        }
    }

    fn acquire_wash(&mut self, ctx: &Ctx<'_>, owner: u64, colour: &str) -> bool {
        let key = (owner, colour.to_owned());
        if let Some(wash) = self.washes.get_mut(&key) {
            wash.users += 1;
            return true;
        }
        let Some(el) = ctx
            .nodes
            .get(&owner)
            .and_then(|n| n.clone().dyn_into::<web_sys::HtmlElement>().ok())
        else {
            return false;
        };
        let style = el.style();
        let saved = (
            style.get_property_value("box-shadow").unwrap_or_default(),
            style.get_property_priority("box-shadow"),
        );
        // The cell's own shadow (from a sheet or inline) stays, under the wash.
        let own = web_sys::window()
            .and_then(|w| w.get_computed_style(&el).ok().flatten())
            .and_then(|cs| cs.get_property_value("box-shadow").ok())
            .unwrap_or_default();
        let mut shadow = format!("inset 0 0 0 100vmax {colour}");
        let own = own.trim();
        if !own.is_empty() && own != "none" {
            shadow.push_str(", ");
            shadow.push_str(own);
        }
        let _ = style.set_property_with_priority("box-shadow", &shadow, "important");
        self.washes.insert(
            key,
            Wash {
                el,
                saved,
                users: 1,
            },
        );
        true
    }

    fn release(&mut self, painted: &Painted) {
        match painted {
            Painted::Box => {}
            Painted::Text(text, colour) => {
                let key = (text.clone(), colour.clone());
                if let Some(shared) = self.ranges.get_mut(&key) {
                    shared.users -= 1;
                    if shared.users == 0 {
                        let shared = self.ranges.remove(&key).expect("present");
                        let _ = call1(&shared.highlight, "delete", &shared.range);
                    }
                }
            }
            Painted::Wash(owner, colour) => {
                let key = (*owner, colour.clone());
                if let Some(wash) = self.washes.get_mut(&key) {
                    wash.users -= 1;
                    if wash.users == 0 {
                        let wash = self.washes.remove(&key).expect("present");
                        let style = wash.el.style();
                        let (value, priority) = &wash.saved;
                        if value.is_empty() {
                            let _ = style.remove_property("box-shadow");
                        } else {
                            let _ = style.set_property_with_priority("box-shadow", value, priority);
                        }
                    }
                }
            }
        }
    }
}

/// The box's used `background-color`: the selection's colour, inline or from
/// the app's sheet.
fn background_color(el: &web_sys::HtmlElement) -> String {
    web_sys::window()
        .and_then(|w| w.get_computed_style(el).ok().flatten())
        .and_then(|cs| cs.get_property_value("background-color").ok())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "rgba(26, 115, 232, 0.3)".to_owned())
}

/// `CSS.highlights`, when the browser has it (Chrome 105, Firefox 140,
/// Safari 17.2).
fn registry() -> Option<JsValue> {
    let window = web_sys::window()?;
    let css = js_sys::Reflect::get(&window, &JsValue::from_str("CSS")).ok()?;
    if css.is_undefined() || css.is_null() {
        return None;
    }
    let registry = js_sys::Reflect::get(&css, &JsValue::from_str("highlights")).ok()?;
    let highlight = js_sys::Reflect::get(&window, &JsValue::from_str("Highlight")).ok()?;
    (registry.is_object() && highlight.is_function()).then_some(registry)
}

thread_local! {
    /// Whether the page has `CSS.highlights` (it cannot change).
    static SUPPORTED: std::cell::OnceCell<bool> = const { std::cell::OnceCell::new() };
    /// The colours given a highlight so far, by index: colour `i` is painted
    /// by the highlight [`highlight_name`]`(i)`. Page-wide, like the registry.
    static COLOURS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn highlights_supported() -> bool {
    SUPPORTED.with(|s| *s.get_or_init(|| registry().is_some()))
}

fn highlight_name(i: usize) -> String {
    format!("rinch-pm-selection-{i}")
}

/// The attribute marking the `<style>` that holds the `::highlight()` rules.
const SHEET_MARKER: &str = "data-rinch-selection-highlights";

/// The registered `Highlight` that paints `colour`, made (with its
/// `::highlight()` rule) the first time.
fn highlight_for(doc: &web_sys::Document, colour: &str) -> Option<JsValue> {
    let registry = registry()?;
    let (index, rules) = COLOURS.with(|c| {
        let mut c = c.borrow_mut();
        let index = match c.iter().position(|k| k == colour) {
            Some(i) => i,
            None => {
                c.push(colour.to_owned());
                c.len() - 1
            }
        };
        let rules: String = c
            .iter()
            .enumerate()
            .map(|(i, colour)| {
                format!(
                    "::highlight({}) {{ background-color: {colour}; }}\n",
                    highlight_name(i)
                )
            })
            .collect();
        (index, rules)
    });
    // The rules are (re)written whenever the sheet is missing or behind: an
    // app may replace `<head>`'s content.
    let sheet = match doc
        .query_selector(&format!("style[{SHEET_MARKER}]"))
        .ok()
        .flatten()
    {
        Some(sheet) => sheet,
        None => {
            let sheet = doc.create_element("style").ok()?;
            sheet.set_attribute(SHEET_MARKER, "").ok()?;
            let parent: web_sys::Node = match doc.head() {
                Some(head) => head.into(),
                None => doc.document_element()?.into(),
            };
            parent.append_child(&sheet).ok()?;
            sheet
        }
    };
    if sheet.text_content().as_deref() != Some(rules.as_str()) {
        sheet.set_text_content(Some(&rules));
    }
    let name = JsValue::from_str(&highlight_name(index));
    let existing = call1(&registry, "get", &name)?;
    if !existing.is_undefined() {
        return Some(existing);
    }
    let window = web_sys::window()?;
    let ctor: js_sys::Function = js_sys::Reflect::get(&window, &JsValue::from_str("Highlight"))
        .ok()?
        .dyn_into()
        .ok()?;
    let highlight = js_sys::Reflect::construct(&ctor, &js_sys::Array::new()).ok()?;
    let set: js_sys::Function = js_sys::Reflect::get(&registry, &JsValue::from_str("set"))
        .ok()?
        .dyn_into()
        .ok()?;
    set.call2(&registry, &name, &highlight).ok()?;
    Some(highlight)
}

/// `target.method(arg)`.
fn call1(target: &JsValue, method: &str, arg: &JsValue) -> Option<JsValue> {
    let f: js_sys::Function = js_sys::Reflect::get(target, &JsValue::from_str(method))
        .ok()?
        .dyn_into()
        .ok()?;
    f.call1(target, arg).ok()
}
