//! The root's blocks come first among its host children (`block_order_tests.rs`)
//! whichever overlay the view keeps in the root: the node-selection outline,
//! the drop caret, the IME preedit and the reveal probe each stay after a block
//! appended at the end; a block appended inside a nested container lands in its
//! place there; and the empty-editor placeholder, which leads the root, stays
//! before the first block it overlays.
use super::block_order_tests::{Host, mount, mount_state, order, order_of, para, show_overlays};
use super::*;
use rinch_editor_core::decoration::{Decoration, Widget};
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{NodeSpec, Plugin, PluginKey, Schema, default_plugins};

/// Enter at the end of the last block: split the last textblock at its end.
fn enter_at_the_end(st: &EditorState, view: &mut RinchDomEditorView) -> EditorState {
    let end = st.doc.content_size() - 1;
    let mut tr = st.tr();
    tr.split(end, 1, None).unwrap();
    let next = st.apply(tr);
    view.update_dom(st, &next);
    next
}

/// Gives each of the root's blocks a 200x20 box, stacked from y = 0, so an
/// empty block can place a caret (the mock has no text layout).
fn measure(host: &Host) {
    let blocks = host.doc.borrow().get_children(host.container_id);
    let mut m = host.mock.borrow_mut();
    for (i, b) in blocks.iter().enumerate() {
        m.__set_node_layout(*b, 0.0, 20.0 * i as f32, 200.0, 20.0);
    }
}

#[test]
fn the_node_selection_outline_stays_after_a_new_last_block() {
    let (host, _s, st, mut view) = mount(&["one", "two"]);
    view.position_node_outline(0.0, 0.0, 200.0, 20.0);
    assert_eq!(order(&host), ["one", "two", "data-pm-selected"]);
    enter_at_the_end(&st, &mut view);
    assert_eq!(order(&host), ["one", "two", "", "data-pm-selected"]);
}

#[test]
fn the_drop_caret_stays_after_a_new_last_block() {
    let (host, _s, st, mut view) = mount(&["", ""]);
    measure(&host);
    assert!(view.show_drop_caret(&st.doc, Some((Pos(1), CaretAffinity::Downstream))));
    assert_eq!(order(&host), ["", "", "data-pm-drop-caret"]);
    enter_at_the_end(&st, &mut view);
    assert_eq!(order(&host), ["", "", "", "data-pm-drop-caret"]);
}

#[test]
fn the_preedit_stays_after_a_new_last_block() {
    let (host, _s, st, mut view) = mount(&["one", "two"]);
    view.position_preedit(4.0, 4.0, 16.0, "か", None);
    assert_eq!(order(&host), ["one", "two", "data-pm-preedit"]);
    enter_at_the_end(&st, &mut view);
    assert_eq!(order(&host), ["one", "two", "", "data-pm-preedit"]);
}

#[test]
fn the_reveal_probe_stays_after_a_new_last_block() {
    let (host, _s, st, mut view) = mount(&["", ""]);
    measure(&host);
    assert!(
        view.position_reveal(&st.doc, Pos(3), Pos(3), ScrollAlign::Nearest)
            .is_some(),
        "precondition: the probe is placed"
    );
    assert_eq!(order(&host), ["", "", "data-pm-reveal"]);
    enter_at_the_end(&st, &mut view);
    assert_eq!(order(&host), ["", "", "", "data-pm-reveal"]);
}

/// Enter at the end of a blockquote's last paragraph puts the new paragraph
/// last in the blockquote; a paragraph appended after the blockquote goes in
/// before the root's overlays.
#[test]
fn a_nested_container_keeps_its_blocks_in_order() {
    let s = Rc::new(Schema::starter_kit());
    let quote = s
        .branch(
            "blockquote",
            Fragment::from_children(vec![para(&s, "one"), para(&s, "two")]),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(quote)).unwrap();
    let (host, st, mut view) = mount_state(EditorState::create(s.clone(), doc, default_plugins()));
    show_overlays(&host, &mut view);
    let quote_id = host.doc.borrow().get_children(host.container_id)[0];
    assert_eq!(
        order(&host),
        ["onetwo", "data-pm-caret", "data-pm-selection"]
    );
    // bq(p("one"), p("two")): "two" is 7..10; split there, inside the quote.
    let mut tr = st.tr();
    tr.split(10, 1, None).unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    assert_eq!(order_of(&host, quote_id), ["one", "two", ""]);
    // A paragraph after the quote, at the root's end.
    let end = next.doc.content_size();
    let mut tr = next.tr();
    tr.replace_with(end, end, Fragment::from_node(para(&s, "after")))
        .unwrap();
    let last = next.apply(tr);
    view.update_dom(&next, &last);
    assert_eq!(
        order(&host),
        ["onetwo", "after", "data-pm-caret", "data-pm-selection"]
    );
    assert_eq!(order_of(&host, quote_id), ["one", "two", ""]);
}

// ── The empty-editor placeholder ──────────────────────────────────────────

/// `doc: block*`, so the document can hold no block at all.
fn block_star_schema() -> Rc<Schema> {
    Rc::new(
        Schema::builder()
            .node("doc", NodeSpec::builder("doc").content("block*").build())
            .node(
                "paragraph",
                NodeSpec::builder("paragraph")
                    .content("inline*")
                    .group("block")
                    .build(),
            )
            .node(
                "text",
                NodeSpec::builder("text").group("inline").inline().build(),
            )
            .build(),
    )
}

/// An app's placeholder that shows while the document holds no text, blocks
/// or none.
struct WhileBlank(RefCell<Rc<str>>);
impl Plugin for WhileBlank {
    fn key(&self) -> PluginKey {
        PluginKey("block_order.while_blank")
    }
    fn decorations(&self, s: &EditorState) -> DecorationSet {
        if s.doc.content_size() > 2 {
            return DecorationSet::empty();
        }
        let text = self.0.borrow().clone();
        DecorationSet::new(vec![Decoration::widget(
            Pos(0),
            0,
            Widget::Placeholder(text),
        )])
    }
}

fn mount_blank() -> (
    Host,
    Rc<Schema>,
    EditorState,
    RinchDomEditorView,
    Rc<WhileBlank>,
) {
    let s = block_star_schema();
    let plugin = Rc::new(WhileBlank(RefCell::new(Rc::from("Write…"))));
    let (host, st, view) = mount_state(EditorState::create(
        s.clone(),
        s.branch("doc", Fragment::empty()).unwrap(),
        vec![plugin.clone() as Rc<dyn Plugin>],
    ));
    (host, s, st, view, plugin)
}

/// The first block of an editor with none goes in after the placeholder that
/// leads the root and before the caret, so the placeholder still overlays it.
#[test]
fn the_first_block_goes_after_the_placeholder() {
    let (host, s, st, mut view, _plugin) = mount_blank();
    view.position_caret(4.0, 4.0, 16.0);
    assert_eq!(order(&host), ["data-pm-placeholder", "data-pm-caret"]);
    let mut tr = st.tr();
    tr.replace_with(0, 0, Fragment::from_node(para(&s, "")))
        .unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    assert_eq!(
        order(&host),
        ["data-pm-placeholder", "", "data-pm-caret"],
        "the placeholder still leads the root, the block before the caret"
    );
}

/// A placeholder built while the editor holds no block (its text changed)
/// leads the root too, ahead of the caret already there.
#[test]
fn a_placeholder_rebuilt_with_no_block_leads_the_root() {
    let (host, s, st, mut view, plugin) = mount_blank();
    view.position_caret(4.0, 4.0, 16.0);
    *plugin.0.borrow_mut() = Rc::from("Start here");
    view.refresh_decorations(&st);
    assert_eq!(order(&host), ["data-pm-placeholder", "data-pm-caret"]);
    let mut tr = st.tr();
    tr.replace_with(0, 0, Fragment::from_node(para(&s, "")))
        .unwrap();
    let next = st.apply(tr);
    view.update_dom(&st, &next);
    assert_eq!(order(&host), ["data-pm-placeholder", "", "data-pm-caret"]);
}
