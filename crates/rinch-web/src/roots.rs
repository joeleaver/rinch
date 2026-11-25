//! The mounted-root registry: which root an event belongs to, the
//! [`RootHandle`] an app unmounts by, and the shared mount core that builds
//! a root and registers it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope, clear_render_scope, set_render_scope};

use crate::page_init::ensure_event_delegation;
use crate::{WebDocument, editor_input, images, web_document};

/// A live root and everything that must stay alive for it to keep reacting.
struct MountedRoot {
    web_doc: Rc<RefCell<WebDocument>>,
    /// Held so the render scope (and the effects that reference it) stay alive.
    /// It also *owns* everything the build created — handlers included — so
    /// dropping it is what reclaims the root (issue #141).
    scope: Rc<RefCell<RenderScope>>,
    root: NodeHandle,
    /// The document's key, kept here so an event's root is found without
    /// borrowing `web_doc` (a listener may run inside a borrow of it).
    doc_key: u64,
    /// The element the root's tree is built in: the island's host, or a
    /// whole-page root's `#rinch-body`. An event whose target is inside it is
    /// this root's ([`root_doc_of`]).
    host: web_sys::Node,
    /// Made by [`mount`](crate::mount) /
    /// [`mount_with_menu_bar`](crate::mount_with_menu_bar): the page is its, so
    /// an event that targets no root (the page's `<body>`, nothing focused) is
    /// its too.
    whole_page: bool,
}

thread_local! {
    /// All currently-mounted roots, keyed by [`RootHandle`] id. Owning the roots
    /// here (rather than leaking them) keeps each alive even if the caller drops
    /// its handle, and lets [`RootHandle::unmount`] tear an individual one down.
    static MOUNTED_ROOTS: RefCell<HashMap<u64, MountedRoot>> = RefCell::new(HashMap::new());
    static NEXT_ROOT_ID: Cell<u64> = const { Cell::new(0) };
    /// The document of the root whose build is running, `0` outside one: an
    /// event during the build whose target is in no mounted root (the root
    /// being built is not registered yet) is that root's (#2811).
    static BUILDING_DOC: Cell<u64> = const { Cell::new(0) };
}

/// The document of the root whose code an event's handlers run as (#2811):
/// the deepest mounted root whose host contains `target`. An event that
/// targets no root's tree is the root being built, during a build (it is not
/// registered until the build ends); else the whole-page root's, if there is
/// one, else the only root's when one is mounted; otherwise no root's
/// (`None`).
pub(crate) fn root_doc_of(target: Option<&web_sys::Node>) -> Option<u64> {
    MOUNTED_ROOTS.with(|m| {
        let m = m.try_borrow().ok()?;
        let mut deepest: Option<&MountedRoot> = None;
        if let Some(target) = target {
            for r in m.values() {
                if r.host.contains(Some(target))
                    && deepest.is_none_or(|d| d.host.contains(Some(&r.host)))
                {
                    deepest = Some(r);
                }
            }
        }
        if deepest.is_none() {
            let building = BUILDING_DOC.with(Cell::get);
            if building != 0 {
                return Some(building);
            }
        }
        deepest
            .or_else(|| m.values().find(|r| r.whole_page))
            .or_else(|| (m.len() == 1).then(|| m.values().next()).flatten())
            .map(|r| r.doc_key)
    })
}

/// Mark the document of the root `event` belongs to as the dispatching one
/// until the guard drops (#2811), so the handlers it runs, and the slots it
/// reads, are that root's: what a root's handler installs in a
/// document-scoped slot (the keyboard and paste interceptors, the selection
/// callbacks) is its document's, and the event is served its root's entry.
/// The desktop marks a window's document around its events the same way.
///
/// An event on the editor's capture textarea (which lives in the page's
/// `<body>`, outside every root) is the focused editor's root's. An event
/// that belongs to no root marks nothing, leaving whatever is current: a
/// listener that runs synchronously inside another root's code (a `focus()`
/// in a handler) stays that root's.
pub(crate) fn mark_event_root(event: &web_sys::Event) -> Option<rinch_core::DispatchDocGuard> {
    use wasm_bindgen::JsCast;
    let target = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Node>().ok());
    let doc = editor_input::capture_target_doc(target.as_ref())
        .or_else(|| root_doc_of(target.as_ref()))?;
    Some(rinch_core::push_dispatching_doc(doc))
}

/// A handle to a mounted root. Drop it freely (the root stays mounted); call
/// [`unmount`](Self::unmount) to remove that root's DOM and handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RootHandle {
    id: u64,
}

impl RootHandle {
    /// The stable numeric id of this root.
    pub fn id(self) -> u64 {
        self.id
    }

    /// Unmount this root: remove its component subtree from the host element and
    /// deregister the event handlers it created at build time. The host element
    /// itself is left in place. A no-op if the root was already unmounted.
    ///
    /// Disposing the scope is also what releases what the build registered
    /// *page-globally* — a menu bar's Escape handler and its keyboard chords —
    /// so an island can be taken out of somebody else's page without leaving
    /// anything of its own behind.
    ///
    /// The disposal runs as the root's document's code
    /// (`rinch_core::push_disposing_doc`), as the root's build, its effects
    /// and the handlers its events run do (#2811, `mark_event_root`). So
    /// what the root's code registered (an app key, an interceptor, a
    /// selection callback), what a cleanup registers as the root goes, and
    /// what is registered with the document's key (dismiss handlers, parked
    /// focus and text-selection requests, focus targets) is the document's.
    /// Then the root's document is swept from the registries keyed by
    /// document ([`rinch_core::forget_doc`]), which takes all of it.
    ///
    /// A document-scoped slot (the keyboard and paste interceptors, the
    /// selection callbacks) is per root, so a clear in this root's cleanup
    /// clears this root's entry, or the page's ownerless fallback if that is
    /// what this root was served, and leaves another root's install alone.
    pub fn unmount(self) {
        let root = MOUNTED_ROOTS.with(|m| m.borrow_mut().remove(&self.id));
        if let Some(r) = root {
            // Dispose the scope *first*, while the DOM it was built against is
            // still standing: disposal runs this root's cleanups and drops its
            // effect closures, and those legitimately touch their own nodes.
            // Tearing the tree down first left them patching a corpse.
            //
            // Disposal is also what deregisters this root's event handlers now,
            // replacing the old `handler_id_watermark` range. The range was both
            // incomplete and unsound: it only covered ids allocated during the
            // synchronous build (missing every handler a later effect run
            // registers), and a synchronous effect flush inside the build lets
            // an *unrelated* root allocate an id inside the recorded window,
            // which unmounting this root would then delete. A scope's own record
            // has neither problem (issue #141).
            //
            // The disposal runs as the document's code (#2797), as a desktop
            // window's does: what its cleanups register is the document's, so
            // the sweep below takes it. Each root has a document of its own.
            let doc_key = r.web_doc.borrow().doc_key();
            {
                let _disposing = rinch_core::push_disposing_doc(doc_key);
                match Rc::try_unwrap(r.scope) {
                    // The expected path: nothing else can reach the scope, so
                    // its cleanups run with no borrow outstanding anywhere.
                    Ok(cell) => cell.into_inner().dispose(),
                    Err(shared) => shared.borrow_mut().dispose_in_place(),
                }
            }
            // Before the borrow: a browser runs its focus listeners inside
            // the removal of a focused node, and a key entry that outlives
            // this root still reads the document from one (issue #1478).
            r.root.release_focus_before_detach();
            r.web_doc.borrow_mut().remove_node(r.root.node_id());
            // The root's document is gone: forget it in every registry keyed
            // by document, what its cleanups registered just now included.
            rinch_core::forget_doc(doc_key);
        }
    }
}

/// Shared mount core: build the tree into `web_doc`'s body, wire theme + event
/// delegation, register the root, and return its handle.
pub(crate) fn mount_tree<F>(
    web_doc: Rc<RefCell<WebDocument>>,
    whole_page: bool,
    build: F,
) -> RootHandle
where
    F: FnOnce(&mut RenderScope) -> NodeHandle,
{
    let doc_as_dom: Rc<RefCell<dyn DomDocument>> = web_doc.clone();
    let body_id = web_doc.borrow().body();
    let doc_key = web_doc.borrow().doc_key();
    let host = web_document::registered_node(body_id.0).expect("the root's body is registered");
    let scope = Rc::new(RefCell::new(RenderScope::new(doc_as_dom, body_id)));

    set_render_scope(scope.clone());

    // What the build reports to (a node view's host, #2075) must listen
    // before it runs, on the page's first mount too.
    editor_input::install_before_build();

    // The root scope owns everything this build creates — signals, memos,
    // effects and event handlers — and `unmount` releases them by disposing it
    // (issue #141). Scoped to the build alone: theme injection, event delegation
    // and root registration below are page-global setup with app lifetime.
    //
    // The build is the root's document's code (#2811), as a desktop window's
    // mount is: what it installs in a document-scoped slot is the document's,
    // and every effect, memo, timer and parked callback it makes records the
    // document and runs as it later, wherever it is flushed.
    let root = {
        let _doc = rinch_core::push_dispatching_doc(doc_key);
        let previous = BUILDING_DOC.with(|b| b.replace(doc_key));
        struct RestoreBuilding(u64);
        impl Drop for RestoreBuilding {
            fn drop(&mut self) {
                let _ = BUILDING_DOC.try_with(|b| b.set(self.0));
            }
        }
        let _building = RestoreBuilding(previous);
        let _owner = scope.borrow().push_owner();
        let mut scope_ref = scope.borrow_mut();
        build(&mut scope_ref)
    };

    web_doc.borrow_mut().append_child(body_id, root.node_id());

    clear_render_scope();

    // Ask the app for the image sources this build gave its `<img>`s, now
    // that no borrow of the document is held (`images.rs`).
    images::resolve_pending();

    // Inject/refresh the page-global theme `<style>` (idempotent across roots).
    if let Some(css) = rinch_core::get_current_theme_css() {
        web_document::update_theme_style_global(&css);
    }

    ensure_event_delegation(&web_doc.borrow());
    editor_input::install(web_doc.borrow().browser_document());

    let id = NEXT_ROOT_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    });
    MOUNTED_ROOTS.with(|m| {
        m.borrow_mut().insert(
            id,
            MountedRoot {
                web_doc,
                scope,
                root,
                doc_key,
                host,
                whole_page,
            },
        );
    });
    RootHandle { id }
}
