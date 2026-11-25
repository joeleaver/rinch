//! [`DomFocus`] for [`WebDocument`]: focus, text-control selection and the
//! work deferred until the borrow is released.

use super::*;
use std::cell::Cell;

use rinch_core::dom::{DomFocus, NodeId};
use wasm_bindgen::JsCast;

impl DomFocus for WebDocument {
    /// The `focus()` runs after the caller's borrow is released
    /// ([`DomFocus::take_after_borrow`]).
    fn focus_element(&mut self, node_id: NodeId) {
        if let Some(n) = self.nodes.get(&node_id.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>()
        {
            self.after_borrow.push(Box::new(move || {
                el.focus().ok();
            }));
        }
    }

    fn take_after_borrow(&mut self) -> Option<Box<dyn FnOnce()>> {
        if self.after_borrow.is_empty() {
            return None;
        }
        let work = std::mem::take(&mut self.after_borrow);
        Some(Box::new(move || {
            for job in work {
                job();
            }
        }))
    }

    /// `moves_in_place` for the two nodes: the question
    /// `append_child` / `insert_before` / `insert_child` ask themselves
    /// (issue #1483).
    fn moves_keeping_focus(&self, parent: NodeId, child: NodeId) -> bool {
        match (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            (Some(p), Some(c)) => moves_in_place(p, c),
            _ => false,
        }
    }

    /// The focused element, when it is inside the part of `node` that is about
    /// to go: its `blur()` (issue #1478).
    ///
    /// No JS call while nothing on the page holds the focus
    /// (`FOCUS_MAY_BE_HELD`), which is the whole of a mount and every
    /// update made from a timer or a network reply. With the focus somewhere:
    /// one (`nodeType`) for a text write to a text node, which can hold no
    /// focused child, and otherwise `activeElement` and `contains`.
    fn release_focus_within(
        &self,
        node: NodeId,
        children_only: bool,
        last_round: bool,
    ) -> Option<Box<dyn FnOnce()>> {
        if !FOCUS_MAY_BE_HELD.with(Cell::get) {
            return None;
        }
        let n = self.nodes.get(&node.0)?;
        if children_only && n.node_type() != web_sys::Node::ELEMENT_NODE {
            return None;
        }
        // A focused editor in the part that goes holds the keyboard through
        // its capture field in `<body>`, which no removal reaches (issue
        // #1126): release the editor, as the browser blurs a focused element
        // it takes out.
        if let Some((id, container)) = crate::editor_input::keyboard_editor(&self.browser_doc) {
            let goes = n.contains(Some(container.unchecked_ref()))
                && !(children_only && n.is_same_node(Some(container.unchecked_ref())));
            return goes.then(|| {
                Box::new(move || crate::editor_input::release_editor(id)) as Box<dyn FnOnce()>
            });
        }
        let active = self.browser_doc.active_element()?;
        if !n.contains(Some(active.unchecked_ref())) {
            return None;
        }
        if children_only && n.is_same_node(Some(active.unchecked_ref())) {
            return None;
        }
        // `<body>` is the browser's spelling of "nothing is focused" (an
        // island mounted into it contains it), and blurring it does nothing.
        if self
            .browser_doc
            .body()
            .is_some_and(|b| b.is_same_node(Some(active.unchecked_ref())))
        {
            return None;
        }
        if !last_round {
            return Some(Box::new(move || blur_now(&active)));
        }
        // A listener has put the focus back inside the part that goes on every
        // round so far. Make that part unfocusable for the length of this last
        // blur: `focus()` into an inert subtree is refused, so when the blur
        // returns nothing in there holds the focus and no listener is running.
        // The attribute is gone again before the mutation.
        let holder = n.dyn_ref::<web_sys::Element>().cloned();
        Some(Box::new(move || {
            let added = holder
                .as_ref()
                .filter(|el| !el.has_attribute("inert"))
                .map(|el| el.set_attribute("inert", "").is_ok())
                .unwrap_or(false);
            blur_now(&active);
            if added && let Some(el) = &holder {
                el.remove_attribute("inert").ok();
            }
        }))
    }

    /// For the focused element: the literal browser write, which is what
    /// `set_attribute` / `remove_attribute` make for these names (issue #1478).
    fn before_attribute_write(
        &self,
        node: NodeId,
        name: &str,
        value: Option<&str>,
    ) -> Option<Box<dyn FnOnce()>> {
        let el = self.nodes.get(&node.0)?.dyn_ref::<web_sys::Element>()?;
        let active = self.browser_doc.active_element()?;
        if !el.is_same_node(Some(active.unchecked_ref())) {
            return None;
        }
        let (el, name, value) = (el.clone(), name.to_owned(), value.map(str::to_owned));
        Some(Box::new(move || {
            match value {
                Some(value) => el.set_attribute(&name, &value).ok(),
                None => el.remove_attribute(&name).ok(),
            };
        }))
    }

    /// `HTMLInputElement`/`HTMLTextAreaElement.setSelectionRange(start, end,
    /// direction)` directly (issue #552) — `start`/`end` are already UTF-16
    /// code units on this backend, a JS string's own unit, so no conversion
    /// is needed the way desktop's `EditableState` (byte offsets) requires.
    /// The browser itself implements "set now, or honour it the first time
    /// this control is focused" for an unfocused target, so there is nothing
    /// here to stash.
    fn set_selection_range(
        &mut self,
        node_id: NodeId,
        start: usize,
        end: usize,
        direction: rinch_core::dom::SelectionDirection,
    ) {
        let Some(n) = self.nodes.get(&node_id.0) else {
            return;
        };
        let start = start as u32;
        let end = end as u32;
        let direction = match direction {
            rinch_core::dom::SelectionDirection::Forward => "forward",
            rinch_core::dom::SelectionDirection::Backward => "backward",
            rinch_core::dom::SelectionDirection::None => "none",
        };
        if let Some(input) = n.dyn_ref::<web_sys::HtmlInputElement>() {
            let _ = input.set_selection_range_with_direction(start, end, direction);
        } else if let Some(textarea) = n.dyn_ref::<web_sys::HtmlTextAreaElement>() {
            let _ = textarea.set_selection_range_with_direction(start, end, direction);
        }
    }

    /// The element's own `selectionStart` / `selectionEnd` /
    /// `selectionDirection` (issue #1678) — UTF-16 code units, a JS string's
    /// own. `None` where the browser answers `null` (an `<input>` type with
    /// no text selection) or for any other element.
    fn text_selection_range(
        &self,
        node_id: NodeId,
    ) -> Option<(usize, usize, rinch_core::dom::SelectionDirection)> {
        let n = self.nodes.get(&node_id.0)?;
        let (start, end, direction) = if let Some(i) = n.dyn_ref::<web_sys::HtmlInputElement>() {
            (
                i.selection_start().ok().flatten()?,
                i.selection_end().ok().flatten()?,
                i.selection_direction().ok().flatten(),
            )
        } else {
            let t = n.dyn_ref::<web_sys::HtmlTextAreaElement>()?;
            (
                t.selection_start().ok().flatten()?,
                t.selection_end().ok().flatten()?,
                t.selection_direction().ok().flatten(),
            )
        };
        let direction = match direction.as_deref() {
            Some("backward") => rinch_core::dom::SelectionDirection::Backward,
            Some("none") => rinch_core::dom::SelectionDirection::None,
            _ => rinch_core::dom::SelectionDirection::Forward,
        };
        let (start, end) = (start as usize, end as usize);
        Some((start.min(end), start.max(end), direction))
    }

    /// Where a caret at a UTF-16 offset of the control's value sits, in
    /// client px (issue #1678). A browser exposes no caret geometry for a
    /// form control, so the value is laid out in a hidden `<div>` appended to
    /// `<body>` for the call (an app's `MutationObserver` on the body sees it
    /// come and go) with the control's computed text styles, direction,
    /// `white-space` and padding, at the width the control lays its text out
    /// in (`clientWidth` less the horizontal padding: a scrollbar's gutter is
    /// not text room), and the caret is a collapsed `Range` at the offset in
    /// it — placed by the browser's own line breaking and bidi rules. That
    /// position is moved by the control's own scroll, unscaled by the
    /// mirror's own painted/layout ratio (a transform or zoom on `<body>` or
    /// `<html>`), scaled by the control's, and placed at its painted box. A
    /// single-line field centres its line in its content box, as Chrome draws
    /// it. `None` for a control that is not rendered.
    fn text_caret_rect(
        &mut self,
        node_id: NodeId,
        offset: Option<usize>,
    ) -> Option<rinch_core::reactive::ElementBounds> {
        const MIRRORED: &[&str] = &[
            "direction",
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
            "font-style",
            "font-variant",
            "font-weight",
            "font-stretch",
            "font-size",
            "font-size-adjust",
            "line-height",
            "font-family",
            "font-kerning",
            "font-feature-settings",
            "font-variation-settings",
            "text-align",
            "text-transform",
            "text-indent",
            "letter-spacing",
            "word-spacing",
            "tab-size",
            "overflow-wrap",
            "word-break",
            "unicode-bidi",
        ];
        if !self.is_rendered(node_id) {
            return None;
        }
        let (start, end, direction) = self.text_selection_range(node_id)?;
        let n = self.nodes.get(&node_id.0)?;
        let el: web_sys::HtmlElement = n.clone().dyn_into().ok()?;
        let (value, single_line, password) =
            if let Some(i) = n.dyn_ref::<web_sys::HtmlInputElement>() {
                (i.value(), true, i.type_().eq_ignore_ascii_case("password"))
            } else {
                (
                    n.dyn_ref::<web_sys::HtmlTextAreaElement>()?.value(),
                    false,
                    false,
                )
            };
        let units: Vec<u16> = value.encode_utf16().collect();
        let head = offset.unwrap_or(
            if direction == rinch_core::dom::SelectionDirection::Backward {
                start
            } else {
                end
            },
        );
        let head = head.min(units.len());
        // The mirror's text and the caret's offset into it. A password field
        // draws a bullet per character, so the offset becomes a char count.
        // A zero-width space after the value gives a caret at its very end
        // (after a trailing newline, in an empty field) a line box to sit in;
        // it is neutral to bidi and takes no width.
        let (text, at) = if password {
            let chars = String::from_utf16_lossy(&units[..head]).chars().count();
            (
                format!("{}\u{200b}", "\u{2022}".repeat(value.chars().count())),
                chars,
            )
        } else {
            (format!("{value}\u{200b}"), head)
        };

        let win = web_sys::window()?;
        let doc = win.document()?;
        let body = doc.body()?;
        let cs = win.get_computed_style(&el).ok()??;
        let px = |p: &str| {
            cs.get_property_value(p)
                .ok()
                .and_then(|v| v.trim_end_matches("px").parse::<f64>().ok())
                .unwrap_or(0.0)
        };
        let (pad_l, pad_r, pad_t, pad_b) = (
            px("padding-left"),
            px("padding-right"),
            px("padding-top"),
            px("padding-bottom"),
        );
        // The border widths as computed, not `clientLeft`/`clientTop`: Gecko
        // answers those for an `<input>` as border plus padding (Firefox 155:
        // `clientLeft` 9 for a 1px border and 8px padding).
        let (border_l, border_t, border_b) = (
            px("border-left-width"),
            px("border-top-width"),
            px("border-bottom-width"),
        );
        // A textarea's text room is its client width (a scrollbar's gutter is
        // not text room); an `<input>` has no scrollbar, and Gecko's
        // `clientWidth` for one already leaves its padding out, so its room
        // comes from the border box.
        let border_r = px("border-right-width");
        let text_width = if single_line {
            el.offset_width() as f64 - border_l - border_r - pad_l - pad_r
        } else {
            el.client_width() as f64 - pad_l - pad_r
        }
        .max(0.0);
        let mirror: web_sys::HtmlElement = doc.create_element("div").ok()?.dyn_into().ok()?;
        let ms = mirror.style();
        for prop in MIRRORED {
            if let Ok(v) = cs.get_property_value(prop) {
                let _ = ms.set_property(prop, &v);
            }
        }
        // A textarea wraps as its own `white-space` says (`wrap="off"` is
        // `pre`); a single-line field never wraps.
        let white_space = if single_line {
            "pre".to_string()
        } else {
            cs.get_property_value("white-space")
                .unwrap_or_else(|_| "pre-wrap".into())
        };
        let width = format!("{text_width}px");
        for (prop, v) in [
            ("position", "absolute"),
            ("visibility", "hidden"),
            ("top", "0"),
            ("left", "0"),
            ("overflow", "hidden"),
            ("box-sizing", "content-box"),
            ("border", "0"),
            ("margin", "0"),
            ("height", "auto"),
            ("transform", "none"),
            ("width", width.as_str()),
            ("white-space", white_space.as_str()),
        ] {
            let _ = ms.set_property(prop, v);
        }
        let node = doc.create_text_node(&text);
        mirror.append_child(&node).ok()?;
        body.append_child(&mirror).ok()?;
        let m = mirror.get_bounding_client_rect();
        // A collapsed range at the offset: the browser places it by the same
        // line-breaking and bidi rules it lays the control's text out with,
        // where a marker's edge is the wrong end of a right-to-left run.
        // Right after a line break the caret starts the next line, but a
        // collapsed range there is the end of the line before in Gecko
        // (Firefox 155; Chrome puts it on the next line). The character at
        // the offset — the zero-width space at the very end — is on that next
        // line in both, so the caret is its start edge.
        let after_break = !password && head > 0 && units[head - 1] == u16::from(b'\n');
        let rtl = cs.get_property_value("direction").is_ok_and(|d| d == "rtl");
        let caret = doc.create_range().ok().and_then(|range| {
            range.set_start(&node, at as u32).ok()?;
            if after_break {
                range.set_end(&node, at as u32 + 1).ok()?;
                let r = range.get_client_rects()?.get(0)?;
                let x = if rtl { r.right() } else { r.left() };
                return web_sys::DomRect::new_with_x_and_y_and_width_and_height(
                    x,
                    r.top(),
                    0.0,
                    r.height(),
                )
                .ok();
            }
            range.collapse_with_to_start(true);
            let rects = range.get_client_rects()?;
            rects
                .get(0)
                .or_else(|| Some(range.get_bounding_client_rect()))
        });
        // The mirror's own painted/layout ratio: a transform or zoom on
        // `<body>` or `<html>` scales the mirror too, and the control's ratio
        // below applies it again.
        let (mw, mh) = (mirror.offset_width() as f64, mirror.offset_height() as f64);
        let mx = if mw > 0.0 { m.width() / mw } else { 1.0 };
        let my = if mh > 0.0 { m.height() / mh } else { 1.0 };
        let _ = body.remove_child(&mirror);
        let caret = caret?;
        // In the control's own (untransformed) padding-box coordinates.
        let local_x = (caret.left() - m.left()) / mx - el.scroll_left() as f64;
        let height = caret.height() / my;
        let local_y = if single_line {
            // From the border box: Gecko's `clientHeight` / `clientLeft` for
            // an `<input>` are its inner editor's, padding excluded.
            let content = el.offset_height() as f64 - border_t - border_b - pad_t - pad_b;
            pad_t + (content - height) / 2.0
        } else {
            (caret.top() - m.top()) / my - el.scroll_top() as f64
        };
        // Onto the painted box: a transform on the control or an ancestor
        // scales its layout box into the rect `getBoundingClientRect` reports.
        let e = el.get_bounding_client_rect();
        let (ow, oh) = (el.offset_width() as f64, el.offset_height() as f64);
        let sx = if ow > 0.0 { e.width() / ow } else { 1.0 };
        let sy = if oh > 0.0 { e.height() / oh } else { 1.0 };
        // A textarea's padding box starts at `clientLeft`/`clientTop`, which
        // count a vertical scrollbar on its left (a right-to-left textarea in
        // Chrome); an `<input>` has no scrollbar, and Gecko's `clientLeft` for
        // one is border plus padding, so its origin is the computed border.
        let (origin_x, origin_y) = if single_line {
            (border_l, border_t)
        } else {
            (el.client_left() as f64, el.client_top() as f64)
        };
        let x = e.left() + (origin_x + local_x) * sx;
        let y = e.top() + (origin_y + local_y) * sy;
        Some(rinch_core::reactive::ElementBounds {
            x: x as f32,
            y: y as f32,
            width: 0.0,
            height: (height * sy) as f32,
        })
    }

    /// Whether `node` is shown, asked of the browser (issue #802; see
    /// [`DomFocus::is_rendered`]).
    ///
    /// The same three questions desktop asks, from `getComputedStyle`, which
    /// resolves style on demand: `isConnected`, no `display: none` on the
    /// element or an ancestor (`display` does not inherit, hence the walk), and
    /// the element's own — inherited — `visibility`. An unknown node, or one
    /// with no element to ask, answers `true`, as the trait's default does.
    fn is_rendered(&self, node: NodeId) -> bool {
        let Some(n) = self.nodes.get(&node.0) else {
            return true;
        };
        if !n.is_connected() {
            return false;
        }
        let el = match n.clone().dyn_into::<web_sys::Element>() {
            Ok(el) => el,
            Err(n) => match n.parent_element() {
                Some(el) => el,
                None => return true,
            },
        };
        let Some(win) = web_sys::window() else {
            return true;
        };
        let prop = |el: &web_sys::Element, name: &str| -> Option<String> {
            win.get_computed_style(el)
                .ok()
                .flatten()
                .and_then(|s| s.get_property_value(name).ok())
        };
        if matches!(
            prop(&el, "visibility").as_deref(),
            Some("hidden") | Some("collapse")
        ) {
            return false;
        }
        let mut cur = Some(el);
        while let Some(e) = cur {
            if prop(&e, "display").as_deref() == Some("none") {
                return false;
            }
            cur = e.parent_element();
        }
        true
    }

    /// `document.activeElement`, mapped back through the `__nid` expando
    /// (issue #695).
    ///
    /// An element rinch did not create carries no `__nid`, so focus sitting
    /// outside the mounted root — the page around an island mount, `<body>`
    /// itself — answers `None`. That is the documented meaning of `None`
    /// (*unknown*, not *nothing*), and it is the honest answer: this document
    /// cannot name that node.
    ///
    /// The focused rich-text editor's capture textarea answers the editor's
    /// container (issue #1126): it is how that editor holds the keyboard, and
    /// it sits in `<body>`, outside everything. So an overlay opened from an
    /// editor remembers the editor, and a key entry whose owner holds the
    /// editor counts the focus as inside it.
    ///
    /// Focus inside an open shadow root (an `HtmlView`'s document, #2357)
    /// answers the focused element there, as desktop does, not the host
    /// `document.activeElement` names.
    fn active_element(&self) -> Option<NodeId> {
        let mut active = crate::editor_input::effective_active_element(&self.browser_doc)?;
        while let Some(inner) = active.shadow_root().and_then(|s| s.active_element()) {
            active = inner;
        }
        get_nid(&active)
    }

    /// `showModal()`'s focusing steps (issue #695), answered on the spot —
    /// unlike desktop, which must wait for a layout, the browser lays out on
    /// demand for `getBoundingClientRect` inside the filter below.
    ///
    /// The candidate set is `trap_focusables`, the **same** set `Tab`
    /// containment cycles, so where an opening dialog puts the keyboard and
    /// where Tab can take it afterwards cannot disagree. `autofocus` is read by
    /// presence, HTML's rule for a boolean attribute, and looked for *within*
    /// that set so an `autofocus` on a hidden or disabled node falls through to
    /// the first real stop.
    ///
    /// The move is **verified** and a refusal steps on, for the reason
    /// `handle_trapped_tab` documents at length: `focus()` is a request the
    /// browser may decline for anything the filter does not know about, and
    /// taking the first candidate on trust would leave an opening dialog with
    /// focus still outside it.
    fn focus_into(&mut self, root: NodeId, policy: rinch_core::dom::FocusIntoPolicy) {
        let Some(root_el) = self
            .nodes
            .get(&root.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let items = crate::event_delegation::trap_focusables(&root_el);
        let autofocus = items.iter().position(|el| el.has_attribute("autofocus"));
        let start = match (autofocus, policy) {
            (Some(i), _) => i,
            (None, rinch_core::dom::FocusIntoPolicy::FirstFocusable) => 0,
            (None, rinch_core::dom::FocusIntoPolicy::AutofocusOnly) => return,
        };
        // Stepping on past a refusal is right for a dialog, which wants *some*
        // stop, and wrong for a popover, which wants that **one** element or
        // none: moving focus to a neighbouring control the author never pointed
        // at is not what `autofocus` asked for, and desktop would not do it.
        let limit = match policy {
            rinch_core::dom::FocusIntoPolicy::FirstFocusable => items.len(),
            rinch_core::dom::FocusIntoPolicy::AutofocusOnly => start + 1,
        };
        // The focus-then-verify loop runs once the caller's borrow is released
        // (`take_after_borrow`): each `focus()` dispatches listeners that touch
        // this document.
        let browser_doc = self.browser_doc.clone();
        let candidates: Vec<web_sys::HtmlElement> =
            items.into_iter().take(limit).skip(start).collect();
        self.after_borrow.push(Box::new(move || {
            for el in candidates {
                let _ = el.focus();
                if browser_doc
                    .active_element()
                    .is_some_and(|a| el.is_same_node(Some(a.unchecked_ref())))
                {
                    return;
                }
            }
        }));
    }

    /// The close half (issue #695): hand the keyboard back to `opener`, or let
    /// it go — with the **browser** as the authority on whether the hand-back
    /// can happen.
    ///
    /// Desktop has to predicate this (is it attached, does it have a box, is it
    /// disabled) because `try_focus_input` answers without asking. Here the same
    /// three refusals — detached, `display: none`, `disabled` — are already the
    /// browser's: `HTMLElement.focus()` on any of them does nothing. So the
    /// shape is focus-then-verify, which is `handle_trapped_tab`'s rule in this
    /// file's sibling module and needs no enumeration to be right.
    ///
    /// The bounding rule is the same on both backends and is *not* the
    /// browser's to give: a claim that has moved **outside** `root` belongs to
    /// the user, and this returns without touching it.
    fn restore_focus(&mut self, opener: Option<NodeId>, root: NodeId) {
        let Some(root_el) = self
            .nodes
            .get(&root.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let opener = opener
            .and_then(|o| self.nodes.get(&o.0))
            .and_then(|n| n.clone().dyn_into::<web_sys::HtmlElement>().ok());
        let browser_doc = self.browser_doc.clone();
        // Decided and done once the caller's borrow is released
        // (`take_after_borrow`): the `focus()`/`blur()` dispatch listeners that
        // touch this document.
        self.after_borrow.push(Box::new(move || {
            restore_focus_now(&browser_doc, &root_el, opener)
        }));
    }
}
