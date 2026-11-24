//! [`DomContent`] for [`WebDocument`]: text content, attributes, inline
//! styles, style isolation and highlight overlays.

use super::*;

use rinch_core::dom::{DomContent, NodeId};
use wasm_bindgen::JsCast;

impl DomContent for WebDocument {
    /// Attaches an open shadow root (#2357): the page's style sheets do not
    /// match what is in it, while it inherits from the element, which is
    /// what desktop's `isolate_styles` mirrors. Children added from here on
    /// go into it (`children_of`).
    fn isolate_styles(&mut self, node: NodeId) {
        let Some(el) = self
            .nodes
            .get(&node.0)
            .and_then(|n| n.dyn_ref::<web_sys::Element>())
        else {
            return;
        };
        if el.shadow_root().is_some() {
            return;
        }
        if el
            .attach_shadow(&web_sys::ShadowRootInit::new(web_sys::ShadowRootMode::Open))
            .is_ok()
        {
            el.set_attribute("rinch-style-isolated", "").ok();
        }
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            // A <textarea>'s child text is its *default value*, and nothing
            // more (issue #1206): the browser shows it until the textarea's
            // dirty value flag is set — by a user edit or a `value` write — and
            // from then on a child change reaches `defaultValue` only. That is
            // HTML's rule and desktop's (#1186), so it is left to the browser.
            // This used to write `.value` from the children on every change
            // (#100), which made `textarea { {|| draft.get()} }` a controlled
            // field on the web alone; a controlled textarea is spelled
            // `value_fn` or a reactive `value:`, which `set_attribute("value")`
            // mirrors onto the property.
            n.set_text_content(Some(text));
        }
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            // Fold the name ASCII-lowercase before deciding which of the few
            // special-cased attributes below this is — matching the browser's
            // own fold of an HTML attribute name (`Element.setAttribute` lowers
            // it for an element in the HTML namespace, per the DOM spec), which
            // is why `el.set_attribute` below is still given the ORIGINAL
            // `name`: the browser folds that write itself for HTML elements,
            // and an SVG element's attribute name is case-SIGNIFICANT (`viewBox`)
            // and must reach it unfolded. Without this, `set_attribute("CHECKED",
            // "")` set the content attribute (the browser folded that) but fell
            // through every match arm here by missing them literally, so the
            // live `.checked`/`.selected`/`.muted` property was never mirrored
            // (issue #758, the web-only gap #738 closed on desktop).
            let folded = name.to_ascii_lowercase();
            let matched = folded.as_str();
            if let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                match matched {
                    // `checked`, `selected` and `muted` are HTML **boolean**
                    // attributes: presence is the whole value. `set_attribute`
                    // is the literal primitive on both backends, so the string
                    // is written as given and the control follows the
                    // attribute's *presence*, whatever that string says (issue
                    // #622). That is the browser's own rule for raw markup —
                    // `<input checked="false">` is checked, measured in
                    // `tests/boolean_attributes.rs` — and desktop's, whose
                    // `:checked` and `<option selected>` readers look at presence
                    // alone.
                    //
                    // This arm used to mirror `attr_is_truthy` onto presence,
                    // because `rsx!` stringified a `bool` closure straight into
                    // here and nothing else would ever have turned the attribute
                    // off. #551 moved that mapping into the writer —
                    // `NodeHandle::write_attribute` removes a boolean attribute
                    // whose value is falsey, on both backends — so a reactive
                    // `checked: {|| flag.get()}` still turns off, and what was
                    // left here was a `set_attribute` that meant two different
                    // things on the two backends for exactly two names.
                    //
                    // Reach for `write_attribute` when you are rendering a value;
                    // reach for `remove_attribute` when you mean off.
                    "checked" | "selected" | "muted" => {
                        el.set_attribute(name, value).ok();
                        // Presence, not the string: see `sync_presence_property`
                        // for why the live property is written at all.
                        sync_presence_property(n, matched, true);
                        return;
                    }
                    // `indeterminate` is a property-only flag with no HTML content
                    // attribute; don't materialize a bogus one (the property is
                    // synced below).
                    "indeterminate" => {}
                    // `value` goes through `write_value_attribute`: the live
                    // property is written FIRST (with the focused control's
                    // selection carried through, issue #238), then the content
                    // attribute. On a pristine control the attribute still
                    // mirrors into `.value` — and that mirror resets the caret
                    // — so writing it first would both move the caret and make
                    // the property write below a no-op. During an IME
                    // composition the whole write is deferred.
                    "value" => {
                        write_value_attribute(&el, value);
                        return;
                    }
                    // An `<img>` source the app answers for (`images.rs`) is
                    // shown from the URL the app resolves it to; the source
                    // itself is kept beside it and is what `get_attribute`
                    // answers.
                    "src" if el.tag_name().eq_ignore_ascii_case("img") => {
                        crate::images::set_img_src(&el, name, value);
                        return;
                    }
                    _ => {
                        el.set_attribute(name, value).ok();
                    }
                }
            }
            // A handful of attributes are reflected *asymmetrically* onto a live
            // DOM property on form controls (`value`, `checked`, `selected`,
            // `indeterminate`, `muted`). Writing the attribute alone only sets
            // the control's *default*: the moment the user edits the control it
            // goes "dirty" and the browser stops mirroring the attribute into the
            // property. A reactive `value:`/`checked:` binding would then silently
            // stop updating what is displayed. Mirror the property too so
            // signal-driven updates keep working after the control has been typed
            // into / toggled (issue #100).
            //
            // `checked`, `selected` and `muted` returned above, having mirrored
            // their own presence; `value` returned into `write_value_attribute`.
            // What reaches here is `indeterminate` and everything with no
            // reflected property at all.
            sync_reflected_property(n, matched, value);
        }
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        if let Some(n) = self.nodes.get(&node.0) {
            // See the matching comment in `set_attribute`: fold only for the
            // match decisions below, never for the literal write.
            let folded = name.to_ascii_lowercase();
            let matched = folded.as_str();
            if let Ok(el) = n.clone().dyn_into::<web_sys::Element>() {
                // Removing `value` is a write of `""`: property first (for the
                // same pristine-control mirroring reason as in `set_attribute`)
                // and deferred during an IME composition, so both spellings of
                // "clear this field" obey one policy (issue #238).
                if matched == "value" {
                    remove_value_attribute(&el);
                    return;
                }
                if matched == "src" {
                    crate::images::remove_img_src(&el);
                }
                el.remove_attribute(name).ok();
            }
            // Keep the reflected property in sync when the attribute is removed,
            // otherwise a dirtied control keeps showing the stale property (#100).
            match matched {
                // Absence is the off state for the presence trio, and the only
                // one — a dirtied control would otherwise keep showing the stale
                // property (#100, #622, #754).
                "checked" | "selected" | "muted" => sync_presence_property(n, matched, false),
                // `indeterminate` has no content attribute to be absent, so
                // "remove it" is simply "off".
                "indeterminate" => sync_reflected_property(n, matched, "false"),
                _ => {}
            }
        }
    }

    fn attribute_list(&self, node: NodeId) -> Vec<(String, String)> {
        let Some(el) = self
            .nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return Vec::new();
        };
        el.get_attribute_names()
            .iter()
            .filter_map(|name| name.as_string())
            .filter_map(|name| {
                let value = self.get_attribute(node, &name)?;
                Some((name, value))
            })
            .collect()
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        let n = self.nodes.get(&node.0)?;
        let el: web_sys::Element = n.clone().dyn_into().ok()?;
        // An app-resolved `<img>` answers the source it was given, not the
        // object URL the browser is loading for it.
        if name.eq_ignore_ascii_case("src")
            && let Some(logical) = crate::images::logical_src(&el)
        {
            return Some(logical);
        }
        el.get_attribute(name)
    }

    /// The `.value` **property** of an `<input>`, `<textarea>` or `<select>`
    /// (issue #238) — the text the user sees — never the content attribute,
    /// which typing leaves at whatever was last written programmatically. Any
    /// other element answers from its `value` attribute, as the trait default
    /// does. A `file` input's `.value` is the browser's fake path
    /// (`C:\fakepath\…`), which is what the browser itself reports.
    fn live_value(&self, node: NodeId) -> Option<String> {
        let n = self.nodes.get(&node.0)?;
        if let Some(input) = n.dyn_ref::<web_sys::HtmlInputElement>() {
            Some(input.value())
        } else if let Some(textarea) = n.dyn_ref::<web_sys::HtmlTextAreaElement>() {
            Some(textarea.value())
        } else if let Some(select) = n.dyn_ref::<web_sys::HtmlSelectElement>() {
            Some(select.value())
        } else {
            n.dyn_ref::<web_sys::Element>()?.get_attribute("value")
        }
    }

    /// Straight to the browser's own CSSOM, which is why nothing here restates
    /// the property-name rules desktop needs (#711): `setProperty` lowercases a
    /// non-custom name and compares custom ones exactly, and no `rinch-web` code
    /// compares a property name itself. The `style:` prop's merge is a
    /// different path and *is* shared — [`StyleProp`] reads and writes the
    /// whole `style` attribute through `rinch-core`'s parser on both backends,
    /// so it takes the fold from there.
    ///
    /// [`StyleProp`]: rinch_core::dom::StyleProp
    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>()
        {
            el.style().set_property(property, value).ok();
        }
    }

    /// The DOM `textContent`: a text node's text, an element's descendants'.
    /// Below an `isolate_styles` host it includes the shadow root's text
    /// (after the host's light children's), as desktop's does (#2357);
    /// the browser's own `textContent` stops at a shadow root.
    fn text_content(&self, node: NodeId) -> Option<String> {
        let n = self.nodes.get(&node.0)?;
        let holds_host = n.dyn_ref::<web_sys::Element>().is_some_and(|el| {
            el.shadow_root().is_some()
                || el
                    .query_selector("[rinch-style-isolated]")
                    .ok()
                    .flatten()
                    .is_some()
        });
        if !holds_host {
            return n.text_content();
        }
        let mut text = String::new();
        composed_text(n, &mut text);
        Some(text)
    }

    fn set_highlight_owner(&mut self, overlay: NodeId, owner: Option<NodeId>) {
        let ctx = crate::highlight::Ctx {
            doc: &self.browser_doc,
            nodes: &self.nodes,
        };
        self.highlights
            .set_owner(&ctx, overlay.0, owner.map(|o| o.0));
    }

    fn set_highlight_text(
        &mut self,
        overlay: NodeId,
        text: Option<rinch_core::dom::HighlightText<'_>>,
    ) -> bool {
        let ctx = crate::highlight::Ctx {
            doc: &self.browser_doc,
            nodes: &self.nodes,
        };
        self.highlights.set_text(&ctx, overlay.0, text)
    }
}
