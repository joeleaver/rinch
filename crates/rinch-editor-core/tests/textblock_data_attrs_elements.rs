//! No edit ever puts one element's `data-*` attributes on another: heading
//! levels are different elements. Every attribute here names the element it was
//! put on, and every textblock any edit leaves must carry only its own
//! element's. The rules are in `schema::data_attrs`.

mod data_attr_common;

use data_attr_common::*;
use rinch_editor_core::commands::{delete_char_backward, delete_char_forward, split_block};
use rinch_editor_core::serialize::{clipboard_slice, slice_to_html};
use rinch_editor_core::{EditorState, Fragment, Node, Pos, Slice};

const TAGS: [&str; 4] = ["p", "h1", "h2", "h3"];

/// A textblock of element `tag` holding `text`, naming its element when
/// `marked`.
fn block(tag: &str, text: &str, marked: bool) -> String {
    if marked {
        format!(r#"<{tag} data-el="{tag}" data-rinch-clip="">{text}</{tag}>"#)
    } else {
        format!("<{tag}>{text}</{tag}>")
    }
}

/// Every textblock of `node` carries no `data-el` but its own element's.
fn check(node: &Node, what: &dyn Fn() -> String) {
    if node.is_textblock() {
        if let Some(from) = node.attrs().get_str("data-el") {
            let tag = match node.type_name() {
                "heading" => format!("h{}", node.attrs().get_int("level").unwrap_or(1)),
                "paragraph" => "p".into(),
                other => other.into(),
            };
            assert_eq!(from, tag, "{}", what());
        }
        return;
    }
    for child in node.content().children() {
        check(child, what);
    }
}

/// Documents of every pair of elements, marked or not, empty or not, at the top
/// level, in a list and in table cells.
fn documents() -> Vec<String> {
    let mut one = Vec::new();
    for tag in TAGS {
        for marked in [true, false] {
            for text in ["", "ab"] {
                one.push(block(tag, text, marked));
            }
        }
    }
    let mut docs = one.clone();
    for a in &one {
        for b in &one {
            docs.push(format!("{a}{b}"));
        }
    }
    for a in ["p", "h2"] {
        for b in TAGS {
            let (a, b) = (block(a, "ab", true), block(b, "cd", true));
            docs.push(format!("<ul><li>{a}</li><li>{b}</li></ul>"));
            docs.push(format!("{a}<ul><li>{b}</li></ul>"));
            docs.push(format!("<ul><li>{a}</li></ul>{b}"));
            docs.push(format!("<blockquote>{a}</blockquote>{b}"));
            docs.push(format!(
                "<table><tbody><tr><td>{a}</td><td>{b}</td></tr></tbody></table>"
            ));
            docs.push(format!(
                "<table><tbody><tr><td>{a}{b}</td></tr></tbody></table>"
            ));
        }
    }
    docs
}

/// Pasted content: whole blocks as copy-out writes them, open copies cut from
/// inside blocks, and the same blocks as closed slices.
fn clips(s: &EditorState) -> Vec<(String, Slice)> {
    let mut html: Vec<String> = Vec::new();
    for a in TAGS {
        for marked in [true, false] {
            for text in ["", "X"] {
                html.push(block(a, text, marked));
            }
        }
        for b in TAGS {
            let two = format!("{}{}", block(a, "12", true), block(b, "34", true));
            // Open copies of part of two blocks, and of all of one.
            let source = state(&two, 1).doc;
            for (from, to) in [(1, 7), (2, 6), (3, 6), (1, 3), (2, 3)] {
                html.push(slice_to_html(&clipboard_slice(&source, from, to).unwrap()));
            }
            html.push(format!("<ul><li>{}</li></ul>", block(a, "X", true)));
            html.push(two);
            html.push(format!(
                "{}{}{}",
                block(a, "1", true),
                block(b, "2", false),
                block(a, "3", true)
            ));
        }
    }
    let mut out = Vec::new();
    for clip in html {
        let slice =
            rinch_editor_core::serialize::slice_from_pasted_html(s.schema(), &clip).unwrap();
        if slice.open_start > 0 {
            let closed =
                Slice::from_fragment(Fragment::from_children(slice.content.children().to_vec()));
            out.push((format!("closed {clip}"), closed));
        }
        out.push((clip, slice));
    }
    out
}

/// Every position in a textblock of `doc`.
fn carets(doc: &Node) -> Vec<usize> {
    (0..=doc.content_size())
        .filter(|p| {
            doc.resolve(Pos(*p))
                .is_ok_and(|r| r.parent().is_textblock())
        })
        .collect()
}

#[test]
fn no_edit_puts_one_elements_attributes_on_another() {
    let clips = clips(&state("<p></p>", 1));
    let mut edits = 0usize;
    for doc in documents() {
        let start = state(&doc, 1);
        // The loaded document holds the invariant.
        check(&start.doc, &|| format!("load {doc}"));
        let carets = carets(&start.doc);
        for (i, &from) in carets.iter().enumerate() {
            for &to in &carets[i..] {
                let s = select(start.clone(), from, to);
                let mut after = |what: &str, tr: rinch_editor_core::Transaction| {
                    edits += 1;
                    let next = s.apply(tr);
                    check(&next.doc, &|| {
                        format!("{what} over {from}..{to} of {doc}: {}", html(&next))
                    });
                };
                for (clip, slice) in &clips {
                    let mut tr = s.tr();
                    if tr.replace_selection(slice.clone()).is_ok() {
                        after(&format!("paste {clip}"), tr);
                    }
                }
                for text in ["x", "x\ny", "x\ny\nz", "\n", "x\n", "\nx"] {
                    let slice =
                        rinch_editor_core::serialize::slice_from_text(s.schema(), text).unwrap();
                    let mut tr = s.tr();
                    if tr.replace_selection_with_lines(slice).is_ok() {
                        after(&format!("lines {text:?}"), tr);
                    }
                }
                let mut tr = s.tr();
                if tr.insert_text("t").is_ok() {
                    after("typing", tr);
                }
                let mut tr = s.tr();
                if tr.delete_selection().is_ok() {
                    after("delete", tr);
                }
                for (name, command) in [
                    ("Enter", split_block()),
                    ("Backspace", delete_char_backward()),
                    ("Delete", delete_char_forward()),
                ] {
                    let mut out = None;
                    command(&s, Some(&mut |tr| out = Some(tr)));
                    if let Some(tr) = out {
                        after(name, tr);
                    }
                }
            }
        }
    }
    assert!(edits > 100_000, "{edits} edits");
}

#[test]
fn a_marked_paragraph_pasted_on_a_heading_gives_it_nothing() {
    let p = block("p", "X", true);
    let out = html(&paste(&state("<h2></h2>", 1), &p));
    assert_eq!(out, "<h2>X</h2>");
    let out = html(&paste(&state("<h2>ab</h2>", 1), &p));
    assert!(!out.contains("<h2 data-el"), "{out}");
    let h3 = block("h3", "X", true);
    let out = html(&paste(&state("<h2></h2>", 1), &h3));
    assert!(!out.contains(r#"<h2 data-el"#), "{out}");
}
