//! Loading documents: `load_doc`, `load_html`, caps and schema adoption.

use super::*;

/// #1214: the guest join's load (`cap: false`) installs a wide `colspan` as
/// it is; a load proper caps it. (The join itself is pinned in
/// `tests/colspan_cap_collab_guest.rs` once tables are in the collab scope,
/// #1233.)
#[test]
fn only_a_capped_load_caps_a_colspan() {
    let s = Schema::starter_kit();
    let p = s.branch("paragraph", Fragment::empty()).unwrap();
    let h = mount(s.branch("doc", Fragment::from_node(p.clone())).unwrap());
    let cell = s
        .create_node(
            "table_cell",
            rinch_editor_core::Attrs::from_iter([(
                "colspan",
                rinch_editor_core::AttrValue::Int(5000),
            )]),
            Fragment::from_node(p.clone()),
        )
        .unwrap();
    let row = s
        .create_node("table_row", Default::default(), Fragment::from_node(cell))
        .unwrap();
    let table = s
        .create_node("table", Default::default(), Fragment::from_node(row))
        .unwrap();
    let wide = s
        .branch("doc", Fragment::from_children(vec![table, p]))
        .unwrap();
    let colspan = |h: &EditorHandle| {
        h.doc()
            .child(0)
            .child(0)
            .child(0)
            .attrs()
            .get_int("colspan")
    };
    assert!(h.handle.load_doc_inner(wide.clone(), false));
    assert_eq!(colspan(&h.handle), Some(5000));
    assert!(h.handle.load_doc_inner(wide, true));
    assert_eq!(colspan(&h.handle), Some(1000));
}

/// Issue #217/#440 where a user actually meets it. `create_editor` mints a
/// **new** `Rc<Schema>` per handle and `NodeType`/`MarkType` equality is
/// `Rc::ptr_eq`, so the documented `doc()` → `load_doc()` pair used to hand
/// one editor a document whose marks belong to another editor's schema.
///
/// On unfixed `main` the next `toggleBold` over that text answered `true` and
/// left the run carrying **two** `bold` marks — the document's real one plus
/// a freshly added foreign twin — until #427's `Transform::add_mark` guard
/// turned that into a loud refusal instead (`is_mark_active` still answered
/// `false` over bold text, so the command failed rather than corrupting).
///
/// #440 is the actual fix: `load_doc` re-homes the incoming document onto the
/// receiving editor's own schema (`Node::rebind`), matching node and mark
/// types by name, so the adopted mark is a real mark B's schema recognizes.
/// `is_mark_active` now answers `true`, and `toggleBold` *removes* the mark
/// (the correct toggle) rather than being refused or duplicating it.
#[test]
fn a_document_adopted_from_another_handle_is_recognized_not_duplicated() {
    let s = Schema::starter_kit();
    let a = mount(doc_node(&s, vec![para(&s, "hello")]));
    a.handle.set_selection(Selection::text(Pos(1), Pos(6)));
    assert!(a.handle.command("toggleBold"), "A bolds its own text");
    assert_eq!(a.handle.doc().child(0).child(0).marks().len(), 1);

    let b = mount(doc_node(&s, vec![para(&s, "x")]));
    b.handle.load_doc(a.handle.doc());
    b.handle.set_selection(Selection::text(Pos(1), Pos(6)));
    assert!(
        b.handle.is_mark_active("bold"),
        "the adopted mark is re-homed onto B's schema, so B recognizes it as bold"
    );
    assert!(
        b.handle.command("toggleBold"),
        "a re-homed mark is a real mark B's schema can toggle (not refused)"
    );
    assert_eq!(
        b.handle.doc().child(0).child(0).marks().len(),
        0,
        "toggleBold over already-bold adopted text removes the mark, it does not duplicate it"
    );
}

/// #440: `EditorHandle::new` has the identical latent defect `load_doc` had —
/// `schema` and `doc` are two separate caller-supplied arguments, and
/// `EditorState::create` stores whatever pair it is given with no check they
/// match. Builds `doc` with a bold mark on a schema the constructor is never
/// given, and hands `EditorHandle::new` a *different* `Rc<Schema>` directly
/// (not through the `mount` test helper's own fresh schema, so this is an
/// explicit, intentional mismatch rather than every other `mount`-based
/// fixture's incidental one). The resulting handle must recognize the
/// adopted mark as its own schema's bold, exactly as a `load_doc` adoption
/// does.
#[test]
fn editor_handle_new_rebinds_a_doc_built_on_a_different_schema() {
    // A "donor" editor on its own schema, bolded — same shape as the
    // load_doc fixture's A, just to produce a foreign-schema `Node`
    // without reaching into the Transform/Mark construction API directly.
    let donor_schema = Schema::starter_kit();
    let donor = mount(doc_node(&donor_schema, vec![para(&donor_schema, "hello")]));
    donor.handle.set_selection(Selection::text(Pos(1), Pos(6)));
    assert!(donor.handle.command("toggleBold"));
    let foreign_doc = donor.handle.doc();
    assert_eq!(
        foreign_doc.child(0).child(0).marks().len(),
        1,
        "the fixture itself is bold, on the donor's schema"
    );

    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let dd: Rc<RefCell<dyn DomDocument>> = mock;
    let container_id = dd.borrow_mut().create_element("div");
    let container = NodeHandle::new(container_id, Rc::downgrade(&dd));
    let receiving_schema = Rc::new(Schema::starter_kit());
    let handle = EditorHandle::new(
        container,
        Rc::downgrade(&dd),
        receiving_schema,
        foreign_doc,
        default_plugins(),
    );
    std::mem::forget(dd);

    handle.set_selection(Selection::text(Pos(1), Pos(6)));
    assert!(
        handle.is_mark_active("bold"),
        "EditorHandle::new re-homed the foreign doc's mark onto its own schema"
    );
    assert!(
        handle.command("toggleBold"),
        "a re-homed mark is a real mark this schema can toggle"
    );
    assert_eq!(
        handle.doc().child(0).child(0).marks().len(),
        0,
        "toggleBold removes it rather than being refused or duplicating it"
    );
}

#[test]
fn load_doc_replaces_content() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "old")]));
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("old")
    );

    h.handle
        .load_doc(doc_node(&s, vec![para(&s, "fresh"), para(&s, "lines")]));
    let blocks = children(&h, h.container_id);
    assert_eq!(blocks.len(), 2);
    assert_eq!(text(&h, blocks[0]).as_deref(), Some("fresh"));
    assert_eq!(text(&h, blocks[1]).as_deref(), Some("lines"));
}

#[test]
fn load_html_parses_and_loads() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "x")]));
    assert!(
        h.handle
            .load_html("<h1>Title</h1><p>Body <strong>bold</strong></p>")
    );
    let blocks = children(&h, h.container_id);
    assert_eq!(tag(&h, blocks[0]).as_deref(), Some("h1"));
    assert_eq!(text(&h, blocks[0]).as_deref(), Some("Title"));
    assert_eq!(tag(&h, blocks[1]).as_deref(), Some("p"));
    // The bold run is wrapped.
    let p_children = children(&h, blocks[1]);
    let has_strong = p_children
        .iter()
        .any(|&c| tag(&h, c).as_deref() == Some("strong"));
    assert!(has_strong, "bold inline survived the load");
}

#[test]
fn load_html_empty_keeps_one_empty_paragraph() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "old")]));
    // Empty HTML must NOT leave a block-less doc (no textblock to render or
    // place a caret in) — it loads a single empty paragraph instead.
    assert!(h.handle.load_html(""));
    let doc = h.handle.doc();
    assert_eq!(doc.child_count(), 1, "one block, not zero");
    assert_eq!(doc.child(0).type_name(), "paragraph");
    assert_eq!(doc.child(0).child_count(), 0, "the paragraph is empty");

    // The host re-projected to exactly one (empty) <p>.
    let blocks = children(&h, h.container_id);
    assert_eq!(blocks.len(), 1);
    assert_eq!(tag(&h, blocks[0]).as_deref(), Some("p"));

    // And the cursor can be placed in it (a block-less doc would have no valid
    // position here).
    h.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(h.handle.insert_text("x"));
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("x")
    );
}

#[test]
fn load_doc_blockless_is_repaired() {
    // A directly-constructed block-less doc (Schema::branch does not fill
    // required content) is repaired to one empty paragraph by load_doc.
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "old")]));
    let blockless = s.branch("doc", Fragment::empty()).unwrap();
    assert_eq!(blockless.child_count(), 0);
    h.handle.load_doc(blockless);
    assert_eq!(h.handle.doc().child_count(), 1);
    assert_eq!(h.handle.doc().child(0).type_name(), "paragraph");
}
