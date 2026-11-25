//! Reading the main window's document: nodes, text, attributes, values,
//! computed styles, bounds and focus.

use super::*;

impl TestApp {
    // ── Reading the document ────────────────────────────────────────────

    pub(super) fn doc(&self) -> &Rc<RefCell<RinchDocument>> {
        self.app.doc().expect("a mounted app has a document")
    }

    fn doc_weak(&self) -> Weak<RefCell<dyn DomDocument>> {
        read::weak(self.doc())
    }

    /// A handle to `target`, for the [`NodeHandle`] API (attributes,
    /// children, scroll offsets, `live_value`, …).
    pub fn handle(&self, target: impl Target) -> NodeHandle {
        NodeHandle::new(target.node_id(self), self.doc_weak())
    }

    /// The first element matching the CSS `selector`, in document order.
    pub fn query(&self, selector: &str) -> Option<NodeHandle> {
        self.query_all(selector).into_iter().next()
    }

    /// Every element matching the CSS `selector`, in document order.
    pub fn query_all(&self, selector: &str) -> Vec<NodeHandle> {
        read::query_all(self.doc(), selector)
    }

    /// The text in `target`'s subtree (`textContent`).
    pub fn text(&self, target: impl Target) -> String {
        read::text(self.doc(), target.node_id(self))
    }

    /// `target`'s attribute `name`.
    pub fn attribute(&self, target: impl Target, name: &str) -> Option<String> {
        read::attribute(self.doc(), target.node_id(self), name)
    }

    /// The value of a form control (`<input>`, `<textarea>`, `<select>`) as the
    /// user left it.
    pub fn value(&self, target: impl Target) -> Option<String> {
        let id = target.node_id(self).0;
        let d = self.doc().borrow();
        rinch_dom::form_control::control_value(&d.tree.nodes, id).map(|v| v.into_owned())
    }

    /// One of `target`'s computed style properties, in the form the MCP
    /// `get_computed_styles` tool reports it: `name` is a field of rinch's
    /// computed style (`"display"`, `"background_color"`; a CSS spelling
    /// such as `"background-color"` is accepted), and a value that is not a
    /// plain string comes back as its JSON (`"12.0"`, `"[0,0,0,255]"`).
    ///
    /// **Unstable:** the names and the value spellings are rinch's internal
    /// computed style, and change when it does. Prefer [`Self::bounds`] or an
    /// attribute where they answer the question.
    pub fn computed_style(&self, target: impl Target, name: &str) -> Option<String> {
        let id = target.node_id(self).0;
        let d = self.doc().borrow();
        let style = serde_json::to_value(&d.tree.get(id)?.computed_style).ok()?;
        match style.get(name.replace('-', "_"))? {
            serde_json::Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        }
    }

    /// The box `target` is painted in (its border box), in logical pixels
    /// from the window's top-left — `getBoundingClientRect()`, the box a
    /// click lands in.
    pub fn bounds(&self, target: impl Target) -> ElementBounds {
        read::bounds(self.doc(), target.node_id(self))
    }

    /// The centre of [`Self::bounds`].
    pub fn center(&self, target: impl Target) -> (f32, f32) {
        let b = self.bounds(target);
        (b.x + b.width / 2.0, b.y + b.height / 2.0)
    }

    /// The element with keyboard focus, if any.
    pub fn focused(&self) -> Option<NodeHandle> {
        let id = self.doc().borrow().active_element()?;
        Some(NodeHandle::new(id, self.doc_weak()))
    }
}
