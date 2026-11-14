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

/// A wash on an owner element, shared by every box that names it. One per
/// owner, whatever its colour: a recoloured wash rewrites the shadow, so the
/// owner's own inline value is read only before the first wash is written.
struct Wash {
    el: web_sys::HtmlElement,
    /// The owner's own inline `box-shadow` (value, priority), put back when
    /// the last box lets go.
    saved: (String, String),
    /// The owner's own computed shadow, kept under the wash.
    own: String,
    colour: String,
    users: usize,
}

/// The boxes of one document painted on what they cover.
#[derive(Default)]
pub(crate) struct OverlayHighlights {
    overlays: HashMap<u64, Overlay>,
    ranges: HashMap<(TextKey, String), SharedRange>,
    washes: HashMap<u64, Wash>,
}

impl OverlayHighlights {
    pub(crate) fn set_owner(&mut self, ctx: &Ctx<'_>, overlay: u64, owner: Option<u64>) {
        let entry = self.overlays.entry(overlay).or_insert(Overlay {
            owner: None,
            text: None,
            painted: Painted::Box,
        });
        entry.owner = owner;
        self.repaint(ctx, overlay, false);
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
        // A box that stops naming text (a line reused for a cell wash) is
        // about to be given its new owner: until then it washes nothing, not
        // the block it was a line of.
        if text.is_none() && entry.text.is_some() {
            entry.owner = None;
        }
        entry.text = text;
        self.repaint(ctx, overlay, true);
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
    ///
    /// `refresh`: the text was named again (a redraw), so a shared range the
    /// document moved is built again ([`Self::refresh_text`]); once per range
    /// per task, as every line of a block names the same one.
    fn repaint(&mut self, ctx: &Ctx<'_>, overlay: u64, refresh: bool) {
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
            if let Painted::Text(text, colour) = want
                && refresh
                && first_this_task(&text, &colour)
            {
                self.refresh_text(ctx, &text, &colour);
            }
            return;
        }
        // Let go of the old paint first: a wash recoloured on the same cell
        // must find the cell's own value restored, not save its own wash as
        // the cell's (review of #2673).
        let old = std::mem::replace(
            &mut self.overlays.get_mut(&overlay).expect("present").painted,
            Painted::Box,
        );
        self.release(&old);
        let acquired = match &want {
            Painted::Box => true,
            Painted::Text(text, colour) => self.acquire_text(ctx, text, colour),
            Painted::Wash(owner, colour) => self.acquire_wash(ctx, *owner, colour),
        };
        let want = if acquired { want } else { Painted::Box };
        if let Some(o) = self.overlays.get_mut(&overlay) {
            o.painted = want.clone();
        }
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
            release_colour(colour);
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
        if let Some(wash) = self.washes.get_mut(&owner) {
            wash.users += 1;
            if wash.colour != colour {
                wash.colour = colour.to_owned();
                write_wash(&wash.el, colour, &wash.own);
            }
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
            .map(|own| own.trim().to_owned())
            .filter(|own| !own.is_empty() && own != "none")
            .unwrap_or_default();
        write_wash(&el, colour, &own);
        self.washes.insert(
            owner,
            Wash {
                el,
                saved,
                own,
                colour: colour.to_owned(),
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
                        release_colour(colour);
                    }
                }
            }
            Painted::Wash(owner, _) => {
                if let Some(wash) = self.washes.get_mut(owner) {
                    wash.users -= 1;
                    if wash.users == 0 {
                        let wash = self.washes.remove(owner).expect("present");
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

/// Write the wash of `colour` on `el`, over its own shadow `own`. The spread
/// covers the box however large it is: half its larger side fills it.
fn write_wash(el: &web_sys::HtmlElement, colour: &str, own: &str) {
    let side = el.offset_width().max(el.offset_height()).max(0) + 1;
    let mut shadow = format!("inset 0 0 0 max(100vmax, {side}px) {colour}");
    if !own.is_empty() {
        shadow.push_str(", ");
        shadow.push_str(own);
    }
    let _ = el
        .style()
        .set_property_with_priority("box-shadow", &shadow, "important");
}

/// The box's used `background-color`: the selection's colour, inline or from
/// the app's sheet.
///
/// Read once per task for the boxes of one container (one editor): every
/// box of a selection is called twice per redraw and between the view's own
/// style writes, so a read per call forced a style flush per call (34,000 of
/// them in `review_1449_caret_point`'s sweep, which timed out in Firefox).
/// The boxes of one editor share their rule; an app rule that changes the
/// colour shows from the next task's redraw.
fn background_color(el: &web_sys::HtmlElement) -> String {
    let container = el
        .parent_node()
        .and_then(|p| js_sys::Reflect::get(&p, &JsValue::from_str("__nid")).ok())
        .and_then(|v| v.as_f64());
    let read = || {
        web_sys::window()
            .and_then(|w| w.get_computed_style(el).ok().flatten())
            .and_then(|cs| cs.get_property_value("background-color").ok())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "rgba(26, 115, 232, 0.3)".to_owned())
    };
    let Some(container) = container else {
        return read();
    };
    let key = container as u64;
    if let Some(colour) = with_task_memo(|m| m.colours.get(&key).cloned()) {
        return colour;
    }
    let colour = read();
    with_task_memo(|m| m.colours.insert(key, colour.clone()));
    colour
}

/// What one task has already read or done, cleared at its end (a
/// microtask queued when the first entry goes in).
#[derive(Default)]
struct TaskMemo {
    /// The selection colour per container node id.
    colours: HashMap<u64, String>,
    /// The ranges refreshed.
    refreshed: std::collections::HashSet<(TextKey, String)>,
}

thread_local! {
    static TASK_MEMO: std::cell::RefCell<Option<TaskMemo>> = const { std::cell::RefCell::new(None) };
}

fn with_task_memo<R>(f: impl FnOnce(&mut TaskMemo) -> R) -> R {
    let (r, fresh) = TASK_MEMO.with(|m| {
        let mut m = m.borrow_mut();
        let fresh = m.is_none();
        (f(m.get_or_insert_with(TaskMemo::default)), fresh)
    });
    if fresh && let Some(window) = web_sys::window() {
        let clear = wasm_bindgen::closure::Closure::once_into_js(|| {
            TASK_MEMO.with(|m| m.borrow_mut().take());
        });
        window.queue_microtask(clear.unchecked_ref());
    }
    r
}

/// Whether `text` in `colour` is refreshed for the first time this task.
fn first_this_task(text: &TextKey, colour: &str) -> bool {
    with_task_memo(|m| m.refreshed.insert((text.clone(), colour.to_owned())))
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
    /// The colours with a highlight, by slot, and how many ranges each
    /// holds: slot `i` is painted by the highlight [`highlight_name`]`(i)`.
    /// Unused slots are freed past [`KEPT_COLOURS`]
    /// ([`evict_unused_colours`]). Page-wide, like the registry.
    static COLOURS: std::cell::RefCell<Vec<Option<(String, usize)>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn highlights_supported() -> bool {
    SUPPORTED.with(|s| *s.get_or_init(|| registry().is_some()))
}

fn highlight_name(i: usize) -> String {
    format!("rinch-pm-selection-{i}")
}

/// The attribute marking the `<style>` that holds the `::highlight()` rules.
const SHEET_MARKER: &str = "data-rinch-selection-highlights";

/// The `::highlight()` rules of the colour slots in use.
fn colour_rules(slots: &[Option<(String, usize)>]) -> String {
    slots
        .iter()
        .enumerate()
        .filter_map(|(i, slot)| {
            let (colour, _) = slot.as_ref()?;
            Some(format!(
                "::highlight({}) {{ background-color: {colour}; }}\n",
                highlight_name(i)
            ))
        })
        .collect()
}

/// How many colours keep a highlight while no range uses them. Freeing a
/// colour the moment its last range went unregistered and re-registered it,
/// and rewrote the rules' sheet (a restyle of the page), on every
/// selection drawn after a cleared one.
const KEPT_COLOURS: usize = 4;

/// One range of `colour` fewer.
fn release_colour(colour: &str) {
    COLOURS.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((_, users)) = c.iter_mut().flatten().find(|(k, _)| k == colour) {
            *users = users.saturating_sub(1);
        }
    });
    evict_unused_colours(None);
}

/// Free the slots of colours no range uses while more than
/// [`KEPT_COLOURS`] colours hold one (never `keep`'s): their highlights are
/// unregistered and their rules dropped, so a colour that keeps changing
/// (a `transition` on the boxes) does not grow the page's registry.
fn evict_unused_colours(keep: Option<usize>) {
    let freed = COLOURS.with(|c| {
        let mut c = c.borrow_mut();
        if c.iter().flatten().count() <= KEPT_COLOURS {
            return None;
        }
        let mut freed = Vec::new();
        for (i, slot) in c.iter_mut().enumerate() {
            if Some(i) != keep && slot.as_ref().is_some_and(|(_, users)| *users == 0) {
                *slot = None;
                freed.push(i);
            }
        }
        (!freed.is_empty()).then(|| (freed, colour_rules(&c)))
    });
    let Some((freed, rules)) = freed else {
        return;
    };
    if let Some(registry) = registry() {
        for index in freed {
            let _ = call1(
                &registry,
                "delete",
                &JsValue::from_str(&highlight_name(index)),
            );
        }
    }
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        let _ = write_rules(&doc, &rules);
    }
}

/// The registered `Highlight` that paints `colour`, made (with its
/// `::highlight()` rule) the first time, counted as one more range of it
/// (see [`release_colour`]).
fn highlight_for(doc: &web_sys::Document, colour: &str) -> Option<JsValue> {
    let registry = registry()?;
    let (index, rules) = COLOURS.with(|c| {
        let mut c = c.borrow_mut();
        let found = c
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|(k, _)| k == colour));
        let index = match found {
            Some(i) => i,
            None => match c.iter().position(Option::is_none) {
                Some(i) => {
                    c[i] = Some((colour.to_owned(), 0));
                    i
                }
                None => {
                    c.push(Some((colour.to_owned(), 0)));
                    c.len() - 1
                }
            },
        };
        if let Some((_, users)) = c[index].as_mut() {
            *users += 1;
        }
        (index, colour_rules(&c))
    });
    let highlight = registered_highlight(doc, &registry, index, &rules);
    if highlight.is_none() {
        release_colour(colour);
    } else {
        evict_unused_colours(Some(index));
    }
    highlight
}

/// Write `rules` into the `<style>` that holds them, made when missing.
fn write_rules(doc: &web_sys::Document, rules: &str) -> Option<()> {
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
    if sheet.text_content().as_deref() != Some(rules) {
        sheet.set_text_content(Some(rules));
    }
    Some(())
}

/// The `Highlight` registered for slot `index`, registered (with `rules`
/// written) when it is not yet.
fn registered_highlight(
    doc: &web_sys::Document,
    registry: &JsValue,
    index: usize,
    rules: &str,
) -> Option<JsValue> {
    write_rules(doc, rules)?;
    let name = JsValue::from_str(&highlight_name(index));
    let existing = call1(registry, "get", &name)?;
    if !existing.is_undefined() {
        return Some(existing);
    }
    let window = web_sys::window()?;
    let ctor: js_sys::Function = js_sys::Reflect::get(&window, &JsValue::from_str("Highlight"))
        .ok()?
        .dyn_into()
        .ok()?;
    let highlight = js_sys::Reflect::construct(&ctor, &js_sys::Array::new()).ok()?;
    let set: js_sys::Function = js_sys::Reflect::get(registry, &JsValue::from_str("set"))
        .ok()?
        .dyn_into()
        .ok()?;
    set.call2(registry, &name, &highlight).ok()?;
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
