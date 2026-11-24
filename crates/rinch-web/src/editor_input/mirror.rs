//! The capture mirror: the hidden textarea holds the caret's textblock, and
//! an edit is recovered by diffing the field against what was written.

use super::capture::capture_target;
use super::context_menu::menu_cycle_id;
use super::*;

// ── The capture mirror ────────────────────────────────────────────────────────
//
// The capture textarea used to be held empty, which cost us every edit a keyboard
// expresses as a *replacement* of text it believes is already there. Tapping an
// autocorrect suggestion, for instance, sets a composing region over the word behind
// the caret and then commits the correction; with an empty field that region covers
// nothing, so "word" + a tap on "world" committed as a bare insert and produced
// "wordworld".
//
// So the textarea mirrors the caret's textblock and tracks the model selection. The
// keyboard's ranges then land on real characters, and the edit is recovered by diffing
// the textarea against what we wrote — which handles insert, replace, and delete
// uniformly, whatever inputType (or composition) the keyboard chose to express it with.
// This is the same mirror-and-diff CodeMirror uses for exactly these keyboards.

/// The capture textarea's contents as we last wrote them: one textblock's text and the
/// DOM id of the block it came from.
///
/// The text holds one character per model position of the block ([`mirror_text`]): a
/// hard break is a `\n` and any other inline leaf (an image) a U+FFFC. So a char offset
/// into it IS the model offset into the block, and an edit a keyboard makes right after
/// a leaf maps to the position after it (#1025) — a leaf with no character of its own
/// would make the offset before it and the one after it the same number.
#[derive(Clone)]
pub(super) struct Mirror {
    textblock_nid: u64,
    text: String,
    /// The selection we wrote, in UTF-16 code units — what the textarea counts in.
    /// Kept so a redundant re-sync can be skipped.
    sel: (u32, u32),
}

/// The mirror's text for `textblock`: its text, with one character for each inline
/// leaf (a `[data-pm-type]` element) — `\n` for a hard break, U+FFFC otherwise — so it
/// has exactly one character per model position (see [`Mirror`]).
fn mirror_text(textblock: &web_sys::Element) -> String {
    fn walk(node: &web_sys::Node, out: &mut String) {
        let kids = node.child_nodes();
        for i in 0..kids.length() {
            let Some(child) = kids.item(i) else { continue };
            if child.node_type() == web_sys::Node::TEXT_NODE {
                out.push_str(&child.text_content().unwrap_or_default());
            } else if let Some(el) = child.dyn_ref::<web_sys::Element>()
                && el.has_attribute("data-pm-type")
            {
                out.push(if el.tag_name().eq_ignore_ascii_case("br") {
                    '\n'
                } else {
                    '\u{FFFC}'
                });
            } else {
                walk(&child, out);
            }
        }
    }
    let mut out = String::new();
    walk(textblock, &mut out);
    out
}

/// The number of UTF-16 code units — the unit `selectionStart`/`selectionEnd` count in
/// — in the first `chars` characters of `text` (all of it for `chars` past the end).
pub(super) fn utf16_of_chars(text: &str, chars: usize) -> u32 {
    text.chars().take(chars).map(|c| c.len_utf16() as u32).sum()
}

/// The number of characters of `text` in its first `units` UTF-16 code units — the
/// inverse of [`utf16_of_chars`]. A unit count inside a surrogate pair counts the
/// whole character, so a stale selection never splits one.
pub(super) fn chars_of_utf16(text: &str, units: u32) -> usize {
    let mut seen = 0u32;
    let mut n = 0;
    for c in text.chars() {
        if seen >= units {
            break;
        }
        seen += c.len_utf16() as u32;
        n += 1;
    }
    n
}

/// The minimal replacement turning `base` into `now`: `(from, to, inserted)` where
/// `from..to` is a **char** range in `base`. `None` when they are equal.
///
/// Trimming the common prefix and suffix keeps the edit as small as the change really
/// was: a suggestion that rewrites "word" to "world" comes back as "insert `l` at 3",
/// not "replace the whole word", so marks either side survive and undo stays granular.
pub(super) fn text_diff(base: &str, now: &str) -> Option<(usize, usize, String)> {
    if base == now {
        return None;
    }
    let b: Vec<char> = base.chars().collect();
    let n: Vec<char> = now.chars().collect();
    let mut prefix = 0;
    while prefix < b.len() && prefix < n.len() && b[prefix] == n[prefix] {
        prefix += 1;
    }
    // The suffix may not reach back into the prefix from either side.
    let mut suffix = 0;
    while suffix < b.len() - prefix
        && suffix < n.len() - prefix
        && b[b.len() - 1 - suffix] == n[n.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let inserted: String = n[prefix..n.len() - suffix].iter().collect();
    Some((prefix, b.len() - suffix, inserted))
}

/// Empty the capture textarea and drop the mirror **together**.
///
/// The two must never disagree. A field emptied behind a live mirror is read by the next
/// [`reconcile_mirror`] as "the user deleted the whole block", and that reconcile is
/// applied to the document — so every path that clears the field clears the mirror with
/// it, and vice versa.
pub(super) fn clear_mirror(ta: &web_sys::HtmlTextAreaElement) {
    MIRROR.with(|m| *m.borrow_mut() = None);
    if !ta.value().is_empty() {
        ta.set_value("");
    }
}

/// Rewrite the capture textarea to mirror the caret's textblock, and remember what we
/// wrote. A no-op mid-composition — the IME owns the field until it commits — and
/// while a context-menu cycle holds the field (it carries the Select-All sentinels
/// then, and is re-mirrored when the cycle ends).
pub(super) fn sync_mirror(handle: &EditorHandle) {
    if COMPOSING.with(|c| c.get()) || menu_cycle_id().is_some() {
        return;
    }
    let Some(ta) = capture_target() else {
        return;
    };
    let selection = handle.selection();
    let Some((textblock_nid, head_off)) = handle.textblock_offset(selection.head()) else {
        // No text caret (a node selection, or an unmounted editor): nothing to mirror.
        clear_mirror(&ta);
        return;
    };
    let Some(block) =
        node_by_nid(textblock_nid).and_then(|n| n.dyn_into::<web_sys::Element>().ok())
    else {
        // The caret's block isn't in the host node map (mid-unmount). Leaving the old
        // mirror standing would let the next `input` diff live text against a field
        // that no longer holds it, so drop both rather than half of the pair.
        clear_mirror(&ta);
        return;
    };
    let text = mirror_text(&block);
    let head = utf16_of_chars(&text, head_off);
    // Mirror the selection too, but only when it lies in this same block — a
    // cross-block selection has no honest representation in one block's text.
    let anchor = match handle.textblock_offset(selection.anchor()) {
        Some((nid, off)) if nid == textblock_nid => utf16_of_chars(&text, off),
        _ => head,
    };
    let sel = (anchor.min(head), anchor.max(head));

    let unchanged = MIRROR.with(|m| {
        m.borrow()
            .as_ref()
            .is_some_and(|p| p.textblock_nid == textblock_nid && p.text == text && p.sel == sel)
    });
    if unchanged && ta.value() == text {
        return;
    }
    ta.set_value(&text);
    let _ = ta.set_selection_range(sel.0, sel.1);
    MIRROR.with(|m| {
        *m.borrow_mut() = Some(Mirror {
            textblock_nid,
            text,
            sel,
        })
    });
}

/// Apply whatever the browser did to the capture textarea to the model, by diffing it
/// against the mirror. Returns whether an edit was applied.
///
/// This is the recovery path for every edit we let the browser express in the textarea
/// rather than intercepting: composition commits, autocorrect replacements, and
/// anything else a keyboard writes without an `inputType` we recognise.
pub(super) fn reconcile_mirror(handle: &EditorHandle) -> bool {
    let Some(ta) = capture_target() else {
        return false;
    };
    let Some(mirror) = MIRROR.with(|m| m.borrow().clone()) else {
        return false;
    };
    // The mirror is only refreshed from `refresh_caret`, so a document change that
    // arrives by another route — `load_html`, a collab delta, a toolbar command with no
    // pointer event behind it — leaves it describing text the block no longer holds.
    // Splicing a diff at those offsets would corrupt the document, so fail closed and
    // rebuild the mirror from the live block instead.
    if node_by_nid(mirror.textblock_nid)
        .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        .map(|el| mirror_text(&el))
        .as_deref()
        != Some(mirror.text.as_str())
    {
        sync_mirror(handle);
        return false;
    }
    let Some((from_char, to_char, inserted)) = text_diff(&mirror.text, &ta.value()) else {
        return false;
    };
    // The mirror has one char per model position, so a char offset is the model
    // offset into the block.
    let (Some(from), Some(to)) = (
        handle.pos_in_textblock(mirror.textblock_nid, from_char),
        handle.pos_in_textblock(mirror.textblock_nid, to_char),
    ) else {
        return false;
    };
    // Remember where the user actually was: moving the selection is how the edit is
    // targeted, but an edit that then declines to apply must not leave the caret parked
    // on the diff range it never touched.
    let before = handle.selection();
    handle.set_selection(Selection::text(from, to));
    let applied = if inserted.is_empty() {
        handle.command("deleteSelection")
    } else {
        // `insert_text` replaces a non-empty selection, so this covers both a pure
        // insertion and a replacement in one call.
        handle.insert_text(&inserted)
    };
    if applied {
        adopt_field_caret(handle, &ta, mirror.textblock_nid);
    } else {
        handle.set_selection(before);
    }
    applied
}

/// Move the model caret to wherever the browser left the textarea's caret.
///
/// The minimal diff ends where the *characters* stopped differing, which is not where
/// the keyboard means the caret to be: correcting "here" to "there" is one inserted
/// `t`, so the edit ends at offset 13 while the user expects the caret after "there".
/// The field already holds the right answer — the browser placed it — so take it from
/// there rather than inferring it.
fn adopt_field_caret(handle: &EditorHandle, ta: &web_sys::HtmlTextAreaElement, textblock_nid: u64) {
    let text = ta.value();
    let start = ta.selection_start().ok().flatten().unwrap_or(0);
    let end = ta.selection_end().ok().flatten().unwrap_or(start);
    let (Some(from), Some(to)) = (
        handle.pos_in_textblock(textblock_nid, chars_of_utf16(&text, start)),
        handle.pos_in_textblock(textblock_nid, chars_of_utf16(&text, end)),
    ) else {
        return;
    };
    handle.set_selection(Selection::text(from, to));
}
