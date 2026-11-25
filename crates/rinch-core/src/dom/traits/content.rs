//! The [`DomContent`] supertrait of [`DomDocument`](super::DomDocument):
//! what a node holds — its text, its attributes, its inline styles and style
//! isolation, and highlight overlays.

use super::types::HighlightText;
use crate::dom::NodeId;

/// What a node holds: text content, attributes, inline styles and highlight
/// overlays.
///
/// One of the five supertraits [`DomDocument`](super::DomDocument) is made
/// of; see that trait for how they fit together. Where a node sits is
/// [`DomTree`](super::DomTree).
///
/// # Required Operations
///
/// Implementors must provide:
/// - Attribute mutation: [`set_attribute`](DomContent::set_attribute), [`remove_attribute`](DomContent::remove_attribute)
/// - Text mutation: [`set_text_content`](DomContent::set_text_content)
pub trait DomContent {
    /// Set the text content of a node.
    fn set_text_content(&mut self, node: NodeId, text: &str);

    /// Set an attribute on an element.
    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str);

    /// Remove an attribute from an element.
    fn remove_attribute(&mut self, node: NodeId, name: &str);

    /// Get an attribute value.
    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String>;

    /// Every attribute of `node`, as (name, value) pairs in no promised order
    /// (#2154: a component's copy of an element). Empty for a text node, a
    /// node that is gone, and a host that cannot list them (the default).
    fn attribute_list(&self, _node: NodeId) -> Vec<(String, String)> {
        Vec::new()
    }

    /// The text a form control **holds right now** — what the user sees and
    /// what the next keystroke edits — as opposed to its `value` *content
    /// attribute* (issue #238).
    ///
    /// The two are the same thing on desktop and different things in a
    /// browser, and a component that reads the attribute to ask "what does the
    /// field say?" gets the wrong answer on the web the moment the user types:
    ///
    /// - **Desktop** (`RinchDocument`) keeps them equal by construction. The
    ///   runtime mirrors the edited text into the `value` attribute *before* it
    ///   dispatches `oninput`, and adopts a programmatic write to the focused
    ///   field back into the text engine (#287). So the default — the `value`
    ///   attribute — is the live text. Desktop overrides it for one case: a
    ///   `<textarea>` with no `value` attribute answers its text children,
    ///   its default value, as the browser's property does (#1159).
    /// - **Web** (`rinch-web`) answers from the element's `.value` **property**
    ///   for `<input>`, `<textarea>` and `<select>`. There the attribute holds
    ///   only what was last written programmatically: typing moves the property
    ///   and leaves the attribute behind. Any other element answers from the
    ///   attribute, as the default does.
    ///
    /// **What it is for:** a controlled component's `value_fn` effect asks it
    /// whether the field already shows the value it is about to write, and
    /// writes nothing if so. That echo — keystroke → `oninput` → signal →
    /// effect → the same text written back — is the common case, and skipping
    /// it keeps the write off the focused field altogether. It also lets a
    /// component tell "the author's text still denotes this value" without
    /// keeping its own record of every keystroke, which is what `ColorPicker`
    /// and `ColorInput` did until this existed (GH #231, #235).
    ///
    /// `None` means the node carries no value this backend can name — a
    /// desktop control with no `value` attribute yet, or a node id that no
    /// longer exists. A web form control always answers `Some`, `""` when
    /// empty, because the property always has a value. A caller comparing
    /// against the text it is about to write should therefore treat `None` as
    /// "differs" and write, which is what the empty-string case costs.
    fn live_value(&self, node: NodeId) -> Option<String> {
        self.get_attribute(node, "value")
    }

    /// Set a CSS style property on an element.
    ///
    /// CSSOM's `setProperty`: afterwards the element's inline style gives
    /// `property` this value however the attribute declared it before. A
    /// backend that keeps the attribute as a string of declarations must not
    /// leave the write in front of a later declaration that covers it — a
    /// shorthand, or a logical twin — or that later declaration wins the
    /// cascade and the attribute names a value the element is not laid out
    /// with (#470).
    fn set_style(&mut self, node: NodeId, property: &str, value: &str);

    /// Set multiple CSS style properties on an element in a single operation.
    /// More efficient than calling `set_style` multiple times because it only
    /// parses the style string once.
    fn set_styles(&mut self, node: NodeId, properties: &[(&str, &str)]) {
        // Default implementation falls back to calling set_style for each property.
        for &(property, value) in properties {
            self.set_style(node, property, value);
        }
    }

    /// Paint `overlay` as a **highlight of `owner`'s content** (#2666): after
    /// `owner`'s background, border and inline backgrounds, and before its
    /// text and in-flow children, as a browser paints a text selection.
    /// `None` paints it in its own place in the stacking order again.
    ///
    /// `overlay` is a positioned box drawn in a stacking context that `owner`
    /// is painted in (the rich-text editor's selection wash: a `z-index: -1`
    /// child of the editor, owned by the textblock or table cell it covers).
    /// Its box, transform and style are its own; only *when* it is drawn
    /// changes, so an opaque background on `owner` (a code block, a table
    /// header, an inline `code` or `mark`) no longer covers it, and its text
    /// is still drawn over it. Where `owner` is not painted after it (a block
    /// the editor keeps collapsed, a `paint_subtree` snapshot without it), it
    /// is painted at the end of its stacking context's sequence. Painted
    /// there, it is under that stacking context's own clip only: a clip
    /// between the stacking context and the overlay (an `overflow` ancestor
    /// that is not a stacking context) does not apply to it. The editor's
    /// washes are children of the stacking context, so nothing is lost.
    ///
    /// The owner id is checked against its generation at every paint: an
    /// owner freed since names nothing, and the overlay is painted in its own
    /// place until it is given another.
    ///
    /// Default: nothing, so the overlay keeps its own place. rinch-dom
    /// answers it; the browser has no such order.
    fn set_highlight_owner(&mut self, overlay: NodeId, owner: Option<NodeId>) {
        let _ = (overlay, owner);
    }

    /// Name the **text** the highlight box `overlay` covers (#2667), for a host
    /// that paints a text highlight itself rather than by the box's paint
    /// order. `None` says the box covers no text (a table cell's wash) or no
    /// longer highlights anything.
    ///
    /// The rich-text editor calls it for every selection box, before
    /// [`set_highlight_owner`](Self::set_highlight_owner), with the range of
    /// the textblock the box is a line of (every line of one block names the
    /// same range).
    ///
    /// Answers whether the host paints the text itself. Such a highlight is
    /// held as DOM ranges, which a document change can rebuild, so the editor
    /// then names the text again after every document change.
    ///
    /// Default: nothing, `false`; rinch-dom paints the box with its owner
    /// instead. The browser backend (rinch-web), which cannot reorder a box's
    /// paint, paints the range as a CSS Custom Highlight (`::highlight()`,
    /// over the text's backgrounds and under its glyphs, as a selection) in
    /// the box's computed `background-color`, and hides the box.
    fn set_highlight_text(&mut self, overlay: NodeId, text: Option<HighlightText<'_>>) -> bool {
        let _ = (overlay, text);
        false
    }

    /// Stop the document's style sheets matching `node`'s descendants
    /// (#2357), as a shadow root stops them in a browser: below `node` only
    /// the user-agent sheet, presentational attributes (`width`, `bgcolor`,
    /// …) and `style` attributes apply, while inherited properties (`color`,
    /// `font-*`, `line-height`, …) still inherit from `node`. `node` itself
    /// is styled as before. One-way. On the web it attaches a shadow root,
    /// and children added afterwards go into it; children `node` already
    /// has stay in the light tree, where they are not rendered (a `<style>`
    /// among them still applies, to the document), so give it its rendered
    /// children afterwards.
    ///
    /// The default does nothing, for a backend without style sheets.
    fn isolate_styles(&mut self, _node: NodeId) {}

    /// Get the text content of a node.
    ///
    /// For text nodes, returns the text. For elements, returns concatenated descendant text.
    fn text_content(&self, _node: NodeId) -> Option<String> {
        None
    }
}
