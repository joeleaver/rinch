//! A pass that changes no node decoration builds nothing per decorated word:
//! every transaction asks again for every decorated node, so the view compares
//! with what it remembers before it allocates.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, DomTree, NodeHandle, RenderScope};
use rinch_editor_core::decoration::{Decoration, DecorationSet};
use rinch_editor_core::{EditorState, Plugin, PluginKey, Pos};
use rinch_editor_view::create_editor;

/// The system allocator, counting this thread's allocations.
struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    // `try_with`: a thread being torn down still frees and allocates.
    let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
}

// SAFETY: every call goes to `System` unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const BLOCKS: usize = 50;

/// One decoration on every block, with the same class list each time.
struct EveryBlock(String);

impl Plugin for EveryBlock {
    fn key(&self) -> PluginKey {
        PluginKey("test.node-decorations.alloc")
    }
    fn decorations(&self, state: &EditorState) -> DecorationSet {
        let mut pos = 0;
        let mut out = Vec::new();
        for i in 0..state.doc.child_count() {
            let end = pos + state.doc.child(i).node_size();
            out.push(Decoration::node(Pos(pos), Pos(end), self.0.as_str()));
            pos = end;
        }
        DecorationSet::new(out)
    }
}

/// The allocations of one transaction that changes nothing, over `BLOCKS`
/// blocks each decorated with `words` classes.
fn unchanged_pass(words: usize) -> usize {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc.clone(), body);
    let handle = create_editor();
    let class: Vec<String> = (0..words).map(|i| format!("c{i}")).collect();
    assert!(handle.add_plugin(Rc::new(EveryBlock(class.join(" ")))));
    assert!(handle.load_html(&"<p>text</p>".repeat(BLOCKS)));
    let container = handle.mount(&mut scope);
    NodeHandle::new(body, Rc::downgrade(&dyn_doc)).append_child(&container);
    let again = || assert!(handle.update(|state| Some(state.tr())));
    again();
    let before = ALLOCATIONS.with(Cell::get);
    again();
    ALLOCATIONS.with(Cell::get) - before
}

#[test]
fn an_unchanged_pass_allocates_nothing_per_decorated_word() {
    let one = unchanged_pass(1);
    let many = unchanged_pass(40);
    // Building the list before comparing it would cost a word's `String` per
    // word per block: some 2000 more here.
    assert!(
        many < one + BLOCKS,
        "{one} allocations with one class per block, {many} with forty"
    );
}
