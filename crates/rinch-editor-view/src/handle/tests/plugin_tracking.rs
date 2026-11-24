use super::*;
use rinch_core::reactive::{Effect, Signal};
use rinch_editor_core::PluginKey;
use rinch_editor_core::decoration::DecorationSet;
use std::any::Any;
use std::cell::Cell;

/// Where the test plugin reads its signal.
#[derive(Clone, Copy, PartialEq)]
enum Site {
    Init,
    Apply,
    Decorations,
    Paste,
    Command,
}

/// A plugin that calls `reads` from exactly one [`Site`].
struct Reads {
    site: Site,
    reads: Rc<dyn Fn()>,
}

impl Reads {
    fn at(&self, site: Site) {
        if self.site == site {
            (self.reads)();
        }
    }
}

impl Plugin for Reads {
    fn key(&self) -> PluginKey {
        PluginKey("test.reads-943")
    }
    fn commands(&self) -> Vec<(&'static str, rinch_editor_core::Command)> {
        let reads = self.reads.clone();
        let on = self.site == Site::Command;
        vec![(
            "test.reads943",
            Rc::new(move |_state: &EditorState, _dispatch| {
                if on {
                    reads();
                }
                true
            }),
        )]
    }
    /// So a read-only editor's `can_run` asks it too.
    fn selection_commands(&self) -> Vec<&'static str> {
        vec!["test.reads943"]
    }
    fn init_state(&self, _doc: &Node) -> Option<Rc<dyn Any>> {
        self.at(Site::Init);
        None
    }
    fn apply(
        &self,
        _tr: &Transaction,
        _old: &EditorState,
        _new_doc: &Node,
        _prev: Option<&dyn Any>,
    ) -> Option<Rc<dyn Any>> {
        self.at(Site::Apply);
        None
    }
    fn decorations(&self, _state: &EditorState) -> DecorationSet {
        self.at(Site::Decorations);
        DecorationSet::empty()
    }
    fn handle_paste(&self, _state: &EditorState, _paste: &PasteContent) -> Option<Transaction> {
        self.at(Site::Paste);
        None
    }
}

fn two_paragraphs() -> EditorHandle {
    let s = schema();
    mount(doc_node(&s, vec![para(&s, "hello"), para(&s, "world")])).handle
}

/// A caret that moves on every call.
fn alternating(a: usize, b: usize) -> impl Fn() -> Selection {
    let flip = Cell::new(false);
    move || {
        flip.set(!flip.get());
        Selection::cursor(Pos(if flip.get() { a } else { b }))
    }
}

/// Install a [`Reads`] plugin reading `store` at `site` on `h`, then
/// `drive` the editor from an app effect created inside another
/// effect's run (two frames deep on the observer stack, as #931's
/// hook fixtures are). Writing `store` must re-run neither effect.
fn assert_plugin_untracked(h: &EditorHandle, site: Site, drive: impl Fn() + 'static) {
    let (own, store) = (Signal::new(0u32), Signal::new(0u32));
    let fired = Rc::new(Cell::new(0u32));
    let reads: Rc<dyn Fn()> = Rc::new({
        let fired = fired.clone();
        move || {
            fired.set(fired.get() + 1);
            let _ = store.get();
        }
    });
    assert!(h.add_plugin(Rc::new(Reads { site, reads })));
    let drive = Rc::new(drive);
    let (outer, inner) = (Rc::new(Cell::new(0u32)), Rc::new(Cell::new(0u32)));
    let slot: Rc<RefCell<Option<Effect>>> = Rc::default();
    let before = fired.get();
    let _outer = Effect::new({
        let (outer, inner, slot) = (outer.clone(), inner.clone(), slot.clone());
        move || {
            outer.set(outer.get() + 1);
            let (inner, drive) = (inner.clone(), drive.clone());
            slot.replace(Some(Effect::new(move || {
                let _ = own.get();
                inner.set(inner.get() + 1);
                drive();
            })));
        }
    });
    let after_first = fired.get();
    assert!(
        after_first > before,
        "control: the plugin ran inside the effect"
    );
    store.set(1);
    assert_eq!(
        (outer.get(), inner.get()),
        (1, 1),
        "a signal only the plugin read re-ran the app effect that drove the editor"
    );
    own.set(1);
    assert_eq!(inner.get(), 2, "positive control: the app effect is live");
    assert!(
        fired.get() > after_first,
        "and its re-run ran the plugin again"
    );
}

/// The issue's repro: a mounted editor asks `decorations()` on every
/// transaction (the view's decoration diff).
#[test]
fn decorations_asked_from_an_effect_subscribe_nobody() {
    let h = two_paragraphs();
    let next = alternating(3, 4);
    let driven = h.clone();
    assert_plugin_untracked(&h, Site::Decorations, move || driven.set_selection(next()));
}

#[test]
fn apply_run_from_an_effect_subscribes_nobody() {
    let h = two_paragraphs();
    let next = alternating(3, 4);
    let driven = h.clone();
    assert_plugin_untracked(&h, Site::Apply, move || driven.set_selection(next()));
}

/// `insert_text` runs the handle's own transaction builder (input
/// rules included), which is internal and untracked too.
#[test]
fn apply_run_by_insert_text_from_an_effect_subscribes_nobody() {
    let h = two_paragraphs();
    let driven = h.clone();
    assert_plugin_untracked(&h, Site::Apply, move || {
        driven.insert_text("x");
    });
}

#[test]
fn handle_paste_run_from_an_effect_subscribes_nobody() {
    let h = two_paragraphs();
    let driven = h.clone();
    assert_plugin_untracked(&h, Site::Paste, move || {
        driven.paste(&PasteContent::text("y"));
    });
}

#[test]
fn a_plugin_command_run_from_an_effect_subscribes_nobody() {
    let h = two_paragraphs();
    let driven = h.clone();
    assert_plugin_untracked(&h, Site::Command, move || {
        driven.command("test.reads943");
    });
}

/// `can_run` is a query (the immutable borrow), and still runs the
/// command's applicability branch: a toolbar's `{|| h.can_run(..)}`
/// must not subscribe to what the command reads.
#[test]
fn a_plugin_command_queried_from_an_effect_subscribes_nobody() {
    let h = two_paragraphs();
    let driven = h.clone();
    assert_plugin_untracked(&h, Site::Command, move || {
        driven.can_run("test.reads943");
    });
}

/// The same on a read-only editor (#834), where a declared selection
/// command is asked the same way.
#[test]
fn a_plugin_command_queried_read_only_from_an_effect_subscribes_nobody() {
    let h = two_paragraphs();
    h.set_read_only(true);
    let driven = h.clone();
    assert_plugin_untracked(&h, Site::Command, move || {
        assert!(driven.can_run("test.reads943"));
    });
}

/// `load_html` rebuilds the state, running `init_state`.
#[test]
fn init_state_run_by_a_load_from_an_effect_subscribes_nobody() {
    let h = two_paragraphs();
    let driven = h.clone();
    assert_plugin_untracked(&h, Site::Init, move || {
        driven.load_html("<p>again</p>");
    });
}

/// `add_plugin` rebuilds the state too, and a handle built inside an
/// effect (a component re-rendered by one) initialises its plugins and
/// projects its decorations there.
#[test]
fn add_plugin_and_construction_from_an_effect_subscribe_nobody() {
    for site in [Site::Init, Site::Decorations] {
        let store = Signal::new(0u32);
        let reads: Rc<dyn Fn()> = Rc::new(move || {
            let _ = store.get();
        });
        let runs = Rc::new(Cell::new(0u32));
        let _added = Effect::new({
            let (runs, reads) = (runs.clone(), reads.clone());
            move || {
                runs.set(runs.get() + 1);
                let h = two_paragraphs();
                h.add_plugin(Rc::new(Reads {
                    site,
                    reads: reads.clone(),
                }));
            }
        });
        let built = Rc::new(Cell::new(0u32));
        let _built = Effect::new({
            let (built, reads) = (built.clone(), reads.clone());
            move || {
                built.set(built.get() + 1);
                let s = schema();
                let mut plugins = default_plugins();
                plugins.push(Rc::new(Reads {
                    site,
                    reads: reads.clone(),
                }));
                let mock = Rc::new(RefCell::new(MockDomDocument::new()));
                let doc: Rc<RefCell<dyn DomDocument>> = mock;
                let id = doc.borrow_mut().create_element("div");
                let container = NodeHandle::new(id, Rc::downgrade(&doc));
                let _h = EditorHandle::new(
                    container,
                    Rc::downgrade(&doc),
                    Rc::new(s.clone()),
                    doc_node(&s, vec![para(&s, "x")]),
                    plugins,
                );
            }
        });
        // `create_editor`'s deferred-mount path.
        let unmounted = Rc::new(Cell::new(0u32));
        let _unmounted = Effect::new({
            let (unmounted, reads) = (unmounted.clone(), reads.clone());
            move || {
                unmounted.set(unmounted.get() + 1);
                let s = schema();
                let mut plugins = default_plugins();
                plugins.push(Rc::new(Reads {
                    site,
                    reads: reads.clone(),
                }));
                let _h = EditorHandle::unmounted(
                    Rc::new(s.clone()),
                    doc_node(&s, vec![para(&s, "x")]),
                    plugins,
                );
            }
        });
        store.set(1);
        assert_eq!(runs.get(), 1, "add_plugin subscribed its effect");
        assert_eq!(built.get(), 1, "EditorHandle::new subscribed its effect");
        assert_eq!(
            unmounted.get(),
            1,
            "EditorHandle::unmounted subscribed its effect"
        );
    }
}

/// A peer's delta rebuilds the state and re-projects the view, running
/// the plugins, under `collab_receive`.
#[cfg(feature = "collaboration")]
#[test]
fn collab_receive_from_an_effect_subscribes_nobody() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    let wire: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let snapshot = host
        .start_collaboration_host({
            let wire = wire.clone();
            move |d| wire.borrow_mut().push(d)
        })
        .unwrap();
    guest.start_collaboration_guest(&snapshot, |_| {}).unwrap();
    let store = Signal::new(0u32);
    let fired = Rc::new(Cell::new(0u32));
    let reads: Rc<dyn Fn()> = Rc::new({
        let fired = fired.clone();
        move || {
            fired.set(fired.get() + 1);
            let _ = store.get();
        }
    });
    // `Apply` covers the rebuild's transaction; decorations are asked
    // by the view's diff in the same call.
    assert!(guest.add_plugin(Rc::new(Reads {
        site: Site::Apply,
        reads,
    })));
    assert!(host.insert_text("!"));
    let deltas: Vec<Vec<u8>> = wire.borrow_mut().drain(..).collect();
    assert!(!deltas.is_empty(), "control: the host broadcast");
    let runs = Rc::new(Cell::new(0u32));
    let _receives = Effect::new({
        let (runs, guest) = (runs.clone(), guest.clone());
        move || {
            runs.set(runs.get() + 1);
            for d in &deltas {
                guest.collab_receive(d);
            }
        }
    });
    assert!(fired.get() > 0, "control: the rebuild ran the plugin");
    store.set(1);
    assert_eq!(runs.get(), 1, "collab_receive subscribed its effect");
}

/// Issue #948: the collaboration `outbound` sink is app code (a
/// transport that may read a `connected` signal) called from
/// `commit`, inside whatever effect made the local edit.
#[cfg(feature = "collaboration")]
#[test]
fn the_outbound_sink_run_from_an_effect_subscribes_nobody() {
    let (own, connected) = (Signal::new(0u32), Signal::new(true));
    let sent = Rc::new(Cell::new(0u32));
    let host = two_paragraphs();
    host.start_collaboration_host({
        let sent = sent.clone();
        move |_delta| {
            if connected.get() {
                sent.set(sent.get() + 1);
            }
        }
    })
    .unwrap();
    let (outer, inner) = (Rc::new(Cell::new(0u32)), Rc::new(Cell::new(0u32)));
    let slot: Rc<RefCell<Option<Effect>>> = Rc::default();
    let _outer = Effect::new({
        let (outer, inner, slot, host) = (outer.clone(), inner.clone(), slot.clone(), host.clone());
        move || {
            outer.set(outer.get() + 1);
            let (inner, host) = (inner.clone(), host.clone());
            slot.replace(Some(Effect::new(move || {
                let _ = own.get();
                inner.set(inner.get() + 1);
                host.insert_text("x");
            })));
        }
    });
    assert_eq!(sent.get(), 1, "control: the edit reached the sink");
    connected.set(false);
    assert_eq!(
        (outer.get(), inner.get()),
        (1, 1),
        "the sink's read subscribed the effect that made the edit"
    );
    connected.set(true);
    own.set(1);
    assert_eq!(inner.get(), 2, "positive control: the app effect is live");
    assert_eq!(sent.get(), 2, "and its edit was sent");
}

/// Positive control: the caller's own `update` build closure is the
/// caller's code, and what it reads **does** subscribe the effect —
/// while a plugin read in the same dispatch still does not.
#[test]
fn the_update_build_closure_stays_tracked() {
    let h = two_paragraphs();
    let (mine, store) = (Signal::new(0u32), Signal::new(0u32));
    assert!(h.add_plugin(Rc::new(Reads {
        site: Site::Apply,
        reads: Rc::new(move || {
            let _ = store.get();
        }),
    })));
    let runs = Rc::new(Cell::new(0u32));
    let next = alternating(3, 4);
    let _e = Effect::new({
        let (runs, h) = (runs.clone(), h.clone());
        move || {
            runs.set(runs.get() + 1);
            let sel = next();
            h.update(|state| {
                let _ = mine.get();
                let mut tr = state.tr();
                tr.set_selection(sel);
                Some(tr)
            });
        }
    });
    store.set(1);
    assert_eq!(runs.get(), 1, "the plugin's read subscribed the effect");
    mine.set(1);
    assert_eq!(runs.get(), 2, "the build closure's own read must track");
}
