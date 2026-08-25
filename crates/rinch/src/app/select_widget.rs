//! Native `<select>` combobox behaviour (issue #121, PR2).
//!
//! rinch-dom renders the *closed* control (the selected label + arrow — PR1);
//! this drives the *interaction*: click to open a popup, navigate/commit with the
//! keyboard, dismiss on Escape or an outside click, and report the chosen value.
//!
//! The split mirrors native `<input>`: rinch-dom paints the box, the app owns the
//! interaction. The popup is **DOM-synthesized** — a backdrop and an option list
//! are appended to `<body>` so they reuse layout, paint, theming, scrolling and
//! hit-testing rather than being hand-painted. They live outside any reactive
//! scope (trailing `<body>` children the reconciler never touches) and are torn
//! down through the [focus arbiter](super::RinchApp::set_focus_target): the popup
//! is open exactly when `focus_target == FocusTarget::Select(_)`.

use super::*;
use rinch_core::dom::NodeId;
use rinch_dom::select::resolve_select_model;

/// Approximate popup row height (used only to estimate popup height for the
/// up/down flip decision before layout runs).
const ROW_HEIGHT_ESTIMATE: f32 = 30.0;
/// Cap on popup height; longer lists scroll.
const MAX_POPUP_HEIGHT: f32 = 260.0;
/// Type-ahead buffer resets after this idle gap.
const TYPEAHEAD_RESET_MS: u128 = 900;

/// The open native-`<select>` popup: the app-created DOM nodes plus keyboard
/// highlight/type-ahead state. Index-aligned vectors mirror the resolved options.
pub(crate) struct OpenSelect {
    /// The `<select>` element the popup belongs to.
    pub select_id: usize,
    /// Full-window click-catcher behind the panel.
    pub backdrop_id: usize,
    /// The option-list panel.
    pub panel_id: usize,
    /// Option row node ids, aligned to the option list.
    pub option_ids: Vec<usize>,
    /// Option submit values, aligned.
    pub values: Vec<String>,
    /// Option display labels, aligned (for type-ahead).
    pub labels: Vec<String>,
    /// Whether each option is disabled, aligned.
    pub disabled: Vec<bool>,
    /// Currently highlighted option index.
    pub highlighted: usize,
    /// Accumulated type-ahead buffer.
    pub typeahead: String,
    /// When the last type-ahead key landed (buffer resets after a gap).
    pub typeahead_at: Option<Instant>,
    /// The value the closed control displayed when the popup opened — the
    /// reference for "did the pick change anything" (issue #226). The `value`
    /// attribute alone can't serve: a value-less `<select>` already displays
    /// its resolved default option, and re-picking it is not a change.
    pub initial_value: String,
}

/// Popup stylesheet, injected once. Uses theme variables with light fallbacks so
/// it looks right with or without the theme feature.
const NATIVE_SELECT_CSS: &str = r#"
.rinch-nsel-backdrop {
    position: fixed;
    left: 0; top: 0; right: 0; bottom: 0;
    z-index: 9998;
}
.rinch-nsel-panel {
    position: fixed;
    z-index: 9999;
    box-sizing: border-box;
    padding: 4px;
    background: var(--rinch-color-body, #ffffff);
    border: 1px solid var(--rinch-color-gray-3, #dee2e6);
    border-radius: 6px;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.15);
    overflow-y: auto;
}
.rinch-nsel-option {
    padding: 6px 10px;
    border-radius: 4px;
    font-size: 14px;
    line-height: 1.4;
    color: var(--rinch-color-text, #212529);
    white-space: nowrap;
    cursor: pointer;
}
.rinch-nsel-option[data-selected]:not([data-highlighted]) {
    background: var(--rinch-color-gray-1, #f1f3f5);
}
.rinch-nsel-option[data-highlighted] {
    background: var(--rinch-primary-color, #228be6);
    color: #ffffff;
}
.rinch-nsel-option[data-disabled] {
    color: var(--rinch-color-gray-5, #adb5bd);
    cursor: default;
}
"#;

impl RinchApp {
    /// Whether a native-select popup is currently open.
    pub(crate) fn is_select_open(&self) -> bool {
        self.open_select.is_some()
    }

    /// Walk up from a hit node to the enclosing `<select>` element, if any.
    pub(super) fn select_ancestor(tree: &rinch_dom::NodeTree, hit_id: usize) -> Option<usize> {
        let mut cur = Some(hit_id);
        while let Some(nid) = cur {
            let node = tree.get(nid)?;
            if node.tag() == Some("select") {
                return Some(nid);
            }
            cur = node.parent;
        }
        None
    }

    /// Absolute on-screen rect `(x, y, w, h)` of a node, summing ancestor offsets
    /// and scroll (the same walk click dispatch uses).
    fn absolute_rect(tree: &rinch_dom::NodeTree, node_id: usize) -> (f32, f32, f32, f32) {
        let Some(node) = tree.get(node_id) else {
            return (0.0, 0.0, 0.0, 0.0);
        };
        let (w, h) = (node.layout.width, node.layout.height);
        let mut x = node.layout.x;
        let mut y = node.layout.y;
        let mut pid = node.parent;
        while let Some(p) = pid {
            let Some(pn) = tree.get(p) else { break };
            x += pn.layout.x - pn.scroll_offset.0 as f32;
            y += pn.layout.y - pn.scroll_offset.1 as f32;
            pid = pn.parent;
        }
        (x, y, w, h)
    }

    /// Open the popup for `select_id`, anchored to the control.
    pub(super) fn open_select_popup(&mut self, select_id: usize, vp_w: f32, vp_h: f32) {
        // Start from a clean slate (also tears down any prior popup / focus).
        self.close_select_popup();

        // Route keyboard to the popup, tearing the previous focus owner down
        // NOW — before the option model and popup geometry are resolved. A
        // blurred input's change commit (issue #226) is user code that may
        // re-render the select or relayout the page; building the popup first
        // would snapshot pre-commit options, coordinates, and (recyclable)
        // node ids (#244 review). This also matches the browser, where
        // mousedown on a select blurs (and commits) the input before the
        // popup opens.
        self.set_focus_target(FocusTarget::Select(select_id));

        let Some(doc) = self.doc.clone() else { return };

        let (rect, model, still_select) = {
            let d = doc.borrow();
            (
                Self::absolute_rect(&d.tree, select_id),
                resolve_select_model(&d.tree, select_id),
                d.tree.get(select_id).and_then(|n| n.tag()) == Some("select"),
            )
        };
        if !still_select || model.options.is_empty() {
            // The commit unmounted/replaced the select (node ids are recycled
            // slab slots), or there is nothing to pop up: don't leave Select
            // focus pointing at a phantom popup.
            self.set_focus_target(FocusTarget::None);
            return;
        }
        let (sx, sy, sw, sh) = rect;

        if !self.select_css_injected {
            doc.borrow_mut().load_css(NATIVE_SELECT_CSS);
            self.select_css_injected = true;
        }

        // Up/down flip: prefer opening below, flip above when the popup wouldn't
        // fit below but would above. Panel geometry is a fixed on-screen rect, so
        // it needs no `--rinch-window-top-inset` (unlike a declarative top: 0).
        let count = model.options.len() as f32;
        let popup_h = (count * ROW_HEIGHT_ESTIMATE + 8.0).min(MAX_POPUP_HEIGHT);
        let space_below = vp_h - (sy + sh);
        let flip_above = space_below < popup_h && sy > space_below;
        let (top, max_h) = if flip_above {
            let mh = popup_h.min(sy - 4.0).max(60.0);
            ((sy - mh).max(0.0), mh)
        } else {
            (sy + sh, popup_h.min((space_below - 4.0).max(60.0)))
        };

        let selected = model.selected_index.unwrap_or(0);

        let mut d = doc.borrow_mut();
        let body = d.body();

        let backdrop = d.create_element("div");
        d.set_attribute(backdrop, "class", "rinch-nsel-backdrop");
        d.append_child(body, backdrop);

        let panel = d.create_element("div");
        d.set_attribute(panel, "class", "rinch-nsel-panel");
        d.set_style(panel, "left", &format!("{sx}px"));
        d.set_style(panel, "top", &format!("{top}px"));
        // Match the control's width. A `width: auto` fixed block fills the
        // viewport, and the closed control is already sized to its widest option
        // (PR1), so the control width is the right popup width. Long labels clip
        // (option is white-space: nowrap), as a browser's popup does.
        d.set_style(panel, "width", &format!("{sw}px"));
        d.set_style(panel, "max-height", &format!("{max_h}px"));

        let mut option_ids = Vec::with_capacity(model.options.len());
        let mut values = Vec::with_capacity(model.options.len());
        let mut labels = Vec::with_capacity(model.options.len());
        let mut disabled = Vec::with_capacity(model.options.len());
        for (i, opt) in model.options.iter().enumerate() {
            let o = d.create_element("div");
            d.set_attribute(o, "class", "rinch-nsel-option");
            d.set_attribute(o, "data-nsel-opt", &i.to_string());
            if i == selected {
                d.set_attribute(o, "data-selected", "");
                d.set_attribute(o, "data-highlighted", "");
            }
            if opt.disabled {
                d.set_attribute(o, "data-disabled", "");
            }
            let t = d.create_text(&opt.label);
            d.append_child(o, t);
            d.append_child(panel, o);
            option_ids.push(o.0);
            values.push(opt.value.clone());
            labels.push(opt.label.clone());
            disabled.push(opt.disabled);
        }
        d.append_child(body, panel);
        drop(d);

        let initial_value = model
            .options
            .get(selected)
            .map(|o| o.value.clone())
            .unwrap_or_default();
        self.open_select = Some(OpenSelect {
            select_id,
            backdrop_id: backdrop.0,
            panel_id: panel.0,
            option_ids,
            values,
            labels,
            disabled,
            highlighted: selected,
            typeahead: String::new(),
            typeahead_at: None,
            initial_value,
        });
        // Select focus was installed at the top, before the popup was built.
        self.scene_dirty = true;
        self.resolve_and_repaint(vp_w, vp_h);
        self.scroll_highlight_into_view(vp_w, vp_h);
    }

    /// Remove the popup DOM nodes (idempotent). Called from the focus arbiter's
    /// teardown; does not touch `focus_target`.
    pub(crate) fn remove_select_popup_nodes(&mut self) {
        let Some(open) = self.open_select.take() else {
            return;
        };
        if let Some(doc) = self.doc.clone() {
            let mut d = doc.borrow_mut();
            d.remove_node(NodeId(open.panel_id));
            d.remove_node(NodeId(open.backdrop_id));
        }
        self.scene_dirty = true;
    }

    /// Close the popup (via the focus arbiter, so teardown removes the nodes).
    pub(super) fn close_select_popup(&mut self) {
        if matches!(self.focus_target, FocusTarget::Select(_)) {
            self.set_focus_target(FocusTarget::None);
        } else {
            // Defensive: nodes without matching focus (shouldn't happen).
            self.remove_select_popup_nodes();
        }
    }

    /// Handle a click while a popup is open. Returns `true` (the click is always
    /// consumed while open — the backdrop is modal).
    pub(super) fn handle_open_select_click(
        &mut self,
        x: f32,
        y: f32,
        vp_w: f32,
        vp_h: f32,
    ) -> bool {
        let Some(open) = self.open_select.as_ref() else {
            return false;
        };
        let panel_id = open.panel_id;
        let Some(doc) = self.doc.clone() else {
            return false;
        };

        enum Hit {
            Option(usize),
            InsidePanel,
            Outside,
        }
        let hit = {
            let d = doc.borrow();
            match hit_test(&d.tree, x, y) {
                Some(hid) => {
                    let mut cur = Some(hid);
                    let mut result = Hit::Outside;
                    while let Some(nid) = cur {
                        let Some(node) = d.tree.get(nid) else { break };
                        if let Some(idx) = node
                            .attributes
                            .get("data-nsel-opt")
                            .and_then(|s| s.parse::<usize>().ok())
                        {
                            result = Hit::Option(idx);
                            break;
                        }
                        if nid == panel_id {
                            result = Hit::InsidePanel;
                            break;
                        }
                        cur = node.parent;
                    }
                    result
                }
                None => Hit::Outside,
            }
        };

        match hit {
            Hit::Option(idx) => {
                if !open.disabled.get(idx).copied().unwrap_or(true) {
                    self.commit_select(idx, vp_w, vp_h);
                }
            }
            Hit::InsidePanel => {} // click on the panel's own padding — keep open
            Hit::Outside => {
                self.close_select_popup();
                self.resolve_and_repaint(vp_w, vp_h);
            }
        }
        true
    }

    /// Commit option `index`: write the value back to the `<select>`, dispatch the
    /// change handler, and close the popup.
    fn commit_select(&mut self, index: usize, vp_w: f32, vp_h: f32) {
        let Some(open) = self.open_select.as_ref() else {
            return;
        };
        let select_id = open.select_id;
        let Some(value) = open.values.get(index).cloned() else {
            return;
        };
        let Some(doc) = self.doc.clone() else {
            return;
        };

        let initial_value = open.initial_value.clone();
        let (input_handler_id, change_handler_id, value_changed) = {
            let mut d = doc.borrow_mut();
            // Did the pick change the value? Compared BEFORE the write, against
            // the `value` attribute when present, else against the option the
            // control displayed when the popup opened — a value-less `<select>`
            // already displays its resolved default, and re-picking it is not
            // a change (HTML fires no change event for it).
            let value_changed = d
                .tree
                .get(select_id)
                .and_then(|n| n.attributes.get("value"))
                .map_or(initial_value != value, |v| v != &value);
            // The resolver reads `value` back, so the closed control repaints with
            // the new label; mark the control dirty so paint re-runs.
            d.set_attribute(NodeId(select_id), "value", &value);
            d.mark_dirty(NodeId(select_id));
            // Handlers may sit on an ancestor: `input`/`change` bubble on the
            // web, so the desktop walk matches the browser's delegation.
            (
                Self::input_attr_handler_up(&d.tree, select_id, "data-oninput"),
                Self::input_attr_handler_up(&d.tree, select_id, "data-onchange"),
                value_changed,
            )
        };

        self.close_select_popup();

        // Report the chosen value to the app's handlers, `Fn(String)` exactly
        // like `<input>`: `oninput` on every commit, then `onchange` — the
        // commit boundary (issue #226) — only when the selection actually
        // changed the value, matching HTML `<select>` semantics. Dispatched
        // with no doc borrow held, since the handlers may mutate the DOM.
        if let Some(hid) = input_handler_id {
            events::dispatch_input_event(events::EventHandlerId(hid), value.clone());
        }
        if value_changed && let Some(hid) = change_handler_id {
            events::dispatch_input_event(events::EventHandlerId(hid), value);
        }
        self.scene_dirty = true;
        self.resolve_and_repaint(vp_w, vp_h);
    }

    /// Route a key to the open popup. Returns `true` if the key was consumed
    /// (every key is consumed while the popup is open, so typing doesn't leak to
    /// the global handlers).
    pub(super) fn handle_select_key(
        &mut self,
        key: KeyCode,
        text: Option<&str>,
        vp_w: f32,
        vp_h: f32,
    ) -> bool {
        if self.open_select.is_none() {
            return false;
        }
        match key {
            KeyCode::Escape => {
                self.close_select_popup();
                self.resolve_and_repaint(vp_w, vp_h);
            }
            KeyCode::Enter | KeyCode::Space => {
                if let Some(open) = self.open_select.as_ref() {
                    let idx = open.highlighted;
                    if !open.disabled.get(idx).copied().unwrap_or(true) {
                        self.commit_select(idx, vp_w, vp_h);
                    }
                }
            }
            KeyCode::ArrowDown => self.step_select_highlight(1, vp_w, vp_h),
            KeyCode::ArrowUp => self.step_select_highlight(-1, vp_w, vp_h),
            KeyCode::Home => self.jump_select_highlight(true, vp_w, vp_h),
            KeyCode::End => self.jump_select_highlight(false, vp_w, vp_h),
            _ => {
                if let Some(t) = text
                    && !t.is_empty()
                    && t.chars().all(|c| !c.is_control())
                {
                    self.select_typeahead(t, vp_w, vp_h);
                }
                // Consume all other keys while open.
            }
        }
        true
    }

    /// Move the highlight by `dir` (+1 down / -1 up) to the next enabled option.
    fn step_select_highlight(&mut self, dir: isize, vp_w: f32, vp_h: f32) {
        let Some(open) = self.open_select.as_ref() else {
            return;
        };
        let n = open.option_ids.len() as isize;
        let mut i = open.highlighted as isize;
        loop {
            let next = i + dir;
            if next < 0 || next >= n {
                return; // no wrap — stay put at the ends
            }
            i = next;
            if !open.disabled.get(i as usize).copied().unwrap_or(false) {
                break;
            }
        }
        self.set_select_highlight(i as usize, vp_w, vp_h);
    }

    /// Highlight the first (`home`) or last enabled option.
    fn jump_select_highlight(&mut self, home: bool, vp_w: f32, vp_h: f32) {
        let Some(open) = self.open_select.as_ref() else {
            return;
        };
        let n = open.option_ids.len();
        let target = if home {
            (0..n).find(|&i| !open.disabled[i])
        } else {
            (0..n).rev().find(|&i| !open.disabled[i])
        };
        if let Some(i) = target {
            self.set_select_highlight(i, vp_w, vp_h);
        }
    }

    /// Type-ahead: extend the buffer and jump to the first enabled option whose
    /// label starts with it (case-insensitive). A single repeated letter cycles
    /// through the options that start with that letter.
    fn select_typeahead(&mut self, ch: &str, vp_w: f32, vp_h: f32) {
        let now = Instant::now();
        let Some(open) = self.open_select.as_mut() else {
            return;
        };
        let expired = open
            .typeahead_at
            .map(|t| now.duration_since(t).as_millis() > TYPEAHEAD_RESET_MS)
            .unwrap_or(true);
        if expired {
            open.typeahead.clear();
        }
        open.typeahead.push_str(&ch.to_lowercase());
        open.typeahead_at = Some(now);

        let query = open.typeahead.clone();
        let start = open.highlighted;
        let n = open.labels.len();

        // If the buffer is a single repeated char, advance to the next match;
        // otherwise match from the current position.
        let single_repeat =
            query.len() >= 2 && query.chars().all(|c| c == query.chars().next().unwrap());
        let (needle, from) = if single_repeat {
            (query[..1].to_string(), (start + 1) % n)
        } else {
            (query.clone(), start)
        };

        let mut found = None;
        for k in 0..n {
            let i = (from + k) % n;
            if open.disabled[i] {
                continue;
            }
            if open.labels[i].to_lowercase().starts_with(&needle) {
                found = Some(i);
                break;
            }
        }
        if let Some(i) = found {
            self.set_select_highlight(i, vp_w, vp_h);
        }
    }

    /// Move the highlight to option `index`, swapping the `data-highlighted`
    /// attribute and scrolling it into view.
    fn set_select_highlight(&mut self, index: usize, vp_w: f32, vp_h: f32) {
        let Some(open) = self.open_select.as_mut() else {
            return;
        };
        if index == open.highlighted || index >= open.option_ids.len() {
            return;
        }
        let prev = open.option_ids[open.highlighted];
        let next = open.option_ids[index];
        open.highlighted = index;

        if let Some(doc) = self.doc.clone() {
            let mut d = doc.borrow_mut();
            d.remove_attribute(NodeId(prev), "data-highlighted");
            d.set_attribute(NodeId(next), "data-highlighted", "");
        }
        self.scene_dirty = true;
        self.resolve_and_repaint(vp_w, vp_h);
        self.scroll_highlight_into_view(vp_w, vp_h);
    }

    /// Scroll the panel so the highlighted option is visible.
    fn scroll_highlight_into_view(&mut self, vp_w: f32, vp_h: f32) {
        let Some(open) = self.open_select.as_ref() else {
            return;
        };
        let (panel_id, opt_id) = (open.panel_id, open.option_ids[open.highlighted]);
        let Some(doc) = self.doc.clone() else {
            return;
        };

        let new_scroll = {
            let d = doc.borrow();
            let (Some(panel), Some(opt)) = (d.tree.get(panel_id), d.tree.get(opt_id)) else {
                return;
            };
            // Option y is relative to the panel's content box; the panel clips to
            // its own height minus padding.
            let pad_top = panel.computed_style.padding_top.to_px();
            let pad_bottom = panel.computed_style.padding_bottom.to_px();
            let visible = panel.layout.height - pad_top - pad_bottom;
            let scroll = panel.scroll_offset.1 as f32;
            let opt_top = opt.layout.y;
            let opt_bottom = opt_top + opt.layout.height;
            if opt_top < scroll {
                Some(opt_top)
            } else if opt_bottom > scroll + visible {
                Some(opt_bottom - visible)
            } else {
                None
            }
        };

        if let Some(s) = new_scroll {
            doc.borrow_mut()
                .set_scroll_top(NodeId(panel_id), s.max(0.0) as f64);
            self.scene_dirty = true;
            self.resolve_and_repaint(vp_w, vp_h);
        }
    }
}
