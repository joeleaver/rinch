use super::*;
use crate::Schema;

fn sk() -> Rc<Schema> {
    Rc::new(Schema::starter_kit())
}
fn para(s: &Schema, t: &str) -> Node {
    s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
        .unwrap()
}
fn doc(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}
fn all_text(node: &Node) -> String {
    if let Some(t) = node.text() {
        return t.to_string();
    }
    node.content().iter().map(all_text).collect()
}

#[test]
fn insert_text_advances_cursor_and_carries_no_marks_by_default() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(2)), None);
    tr.insert_text("X").unwrap();
    assert_eq!(all_text(tr.doc()), "aXb");
    // cursor was at 2 (between a,b); after inserting one char → 3
    assert_eq!(tr.selection(), Selection::cursor(Pos(3)));
    assert!(tr.doc_changed());
}

#[test]
fn insert_text_consumes_stored_marks() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let bold = Mark::simple(s.mark_type("bold").unwrap().clone());
    let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(2)), Some(vec![bold]));
    tr.insert_text("X").unwrap();
    // the inserted "X" carries bold; stored marks were consumed
    let p = tr.doc().child(0);
    let bolded_x = p
        .content()
        .iter()
        .any(|n| n.text() == Some("X") && n.marks().iter().any(|m| m.type_name() == "bold"));
    assert!(bolded_x, "inserted text should carry the stored bold mark");
    assert!(
        tr.stored_marks().is_none(),
        "stored marks consumed by the insert"
    );
}

/// `"see "` + `mark("here")` + `" now"`, typed into at `pos`: the runs of the
/// result as `(text, mark names)`.
fn type_into_marked(mark: &str, pos: usize) -> Vec<(String, Vec<String>)> {
    let s = sk();
    let mt = s.mark_type(mark).unwrap().clone();
    let m = if mark == "link" {
        Mark::new(
            mt,
            crate::Attrs::from_iter([("href", crate::AttrValue::from("https://a.example"))]),
        )
    } else {
        Mark::simple(mt)
    };
    let p = s
        .branch(
            "paragraph",
            Fragment::from_children(vec![
                s.text("see ").unwrap(),
                s.text_with_marks("here", vec![m]).unwrap(),
                s.text(" now").unwrap(),
            ]),
        )
        .unwrap();
    let d = doc(&s, vec![p]);
    let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(pos)), None);
    tr.insert_text("X").unwrap();
    tr.doc()
        .child(0)
        .content()
        .iter()
        .map(|n| {
            let names = n.marks().iter().map(|m| m.type_name().to_string());
            (n.text().unwrap().to_string(), names.collect())
        })
        .collect()
}

fn runs(r: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    r.iter()
        .map(|(t, m)| (t.to_string(), m.iter().map(|m| m.to_string()).collect()))
        .collect()
}

#[test]
fn typing_right_after_a_link_is_not_linked() {
    assert_eq!(
        type_into_marked("link", 9),
        runs(&[("see ", &[]), ("here", &["link"]), ("X now", &[])])
    );
}

#[test]
fn typing_inside_a_link_is_linked() {
    assert_eq!(
        type_into_marked("link", 7),
        runs(&[("see ", &[]), ("heXre", &["link"]), (" now", &[])])
    );
}

#[test]
fn typing_at_a_link_start_is_not_linked() {
    // As for every mark: at a boundary the text before decides.
    assert_eq!(
        type_into_marked("link", 5),
        runs(&[("see X", &[]), ("here", &["link"]), (" now", &[])])
    );
}

#[test]
fn typing_right_after_bold_is_bold() {
    assert_eq!(
        type_into_marked("bold", 9),
        runs(&[("see ", &[]), ("hereX", &["bold"]), (" now", &[])])
    );
}

#[test]
fn selection_maps_forward_through_a_leading_insert() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    // cursor at the end of "ab" (pos 3)
    let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(3)), None);
    // insert "YY" at the start of the text (pos 1) via a position gesture
    let yy = s.text("YY").unwrap();
    tr.replace_with(1, 1, Fragment::from_node(yy)).unwrap();
    assert_eq!(all_text(tr.doc()), "YYab");
    // the cursor at 3 shifts by +2 → 5
    assert_eq!(tr.selection(), Selection::cursor(Pos(5)));
}

#[test]
fn set_selection_clears_stored_marks() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let bold = Mark::simple(s.mark_type("bold").unwrap().clone());
    let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(2)), None);
    tr.set_stored_marks(Some(vec![bold]));
    assert!(tr.stored_marks().is_some());
    tr.set_selection(Selection::cursor(Pos(1)));
    assert!(tr.stored_marks().is_none());
}

#[test]
fn delete_selection_collapses_cursor() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "abcd")]);
    let mut tr = Transaction::new(s.clone(), d, Selection::text(Pos(2), Pos(4)), None);
    tr.delete_selection().unwrap();
    assert_eq!(all_text(tr.doc()), "ad");
    assert_eq!(tr.selection(), Selection::cursor(Pos(2)));
}

#[test]
fn meta_and_add_to_history() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "x")]);
    let mut tr = Transaction::new(s.clone(), d, Selection::cursor(Pos(1)), None);
    assert!(tr.add_to_history());
    tr.set_add_to_history(false);
    assert!(!tr.add_to_history());
    tr.set_meta("origin", "remote".to_string());
    assert_eq!(
        tr.get_meta("origin")
            .and_then(|a| a.downcast_ref::<String>()),
        Some(&"remote".to_string())
    );
}
