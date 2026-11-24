//! Building, mounting and dispatching: the handle's constructors, the one
//! mutation path (`update`/`dispatch`), plugins and commands.

use super::*;

impl EditorHandle {
    /// Build a handle over a fresh [`EditorState`] **without** a host projection —
    /// the deferred-mount path. The view is attached later by [`Self::mount`] (or
    /// [`Self::attach`]) when the host container exists. Used by
    /// [`create_editor`](crate::create_editor).
    pub(crate) fn unmounted(
        schema: Rc<Schema>,
        doc: Node,
        plugins: Vec<Rc<dyn Plugin>>,
    ) -> EditorHandle {
        // Plugins' `init_state` runs here; untracked, as under the core (#943).
        let state = untracked_handler(|| EditorState::create(schema.clone(), doc, plugins.clone()));
        EditorHandle {
            inner: Rc::new(RefCell::new(EditorCore {
                state,
                view: None,
                schema,
                plugins,
                on_change: None,
                on_link_click: None,
                on_link_hover: None,
                on_image_hover: None,
                on_key: None,
                on_image_input: None,
                on_selection_change: None,
                selection_owed: false,
                on_caret_moved: None,
                caret_moved_owed: false,
                anchors: Rc::new(RefCell::new(AnchorMap::default())),
                read_only: false,
                placeholder: None,
                tab_behavior: TabBehavior::default(),
                scroll: ScrollGate::default(),
                reveal: None,
                reveal_misses: 0,
                caret_hint: None,
                #[cfg(feature = "collaboration")]
                collab: None,
                node_views: Rc::default(),
                margins: Margins::default(),
                on_margin_layout: None,
                spell: Default::default(),
                spell_seen: 0,
            })),
        }
    }

    /// Build a handle and project it into `container` in one step (the eager path,
    /// used by tests and by [`Self::mount`]). `doc_ref` is a weak handle to the
    /// host document the view patches. Does **not** register the editor with the
    /// runtime — that is [`Self::mount`]'s job. `doc` is loaded as
    /// [`Self::load_doc`] loads one: re-homed onto `schema` first (#440 — `doc`
    /// and `schema` are two separate caller-supplied arguments, so a `doc` built
    /// on a different `Rc<Schema>` is the same latent mismatch `load_doc` had),
    /// then a `colspan` past 1000 and a `rowspan` past 65534 are capped (#1214,
    /// #1244). A `doc` that cannot be
    /// re-homed (an unknown node or mark type by name) is used as given rather
    /// than refused — this constructor has no failure return, and every current
    /// caller builds `doc` on `schema` already, so that path is a same-schema,
    /// effectively-free `rebind`.
    pub fn new(
        container: NodeHandle,
        doc_ref: Weak<RefCell<dyn DomDocument>>,
        schema: Rc<Schema>,
        doc: Node,
        plugins: Vec<Rc<dyn Plugin>>,
    ) -> EditorHandle {
        let doc = doc.rebind(&schema).unwrap_or(doc);
        // A load: spans past Chrome's limits are capped (#1214, #1244).
        let doc = rinch_editor_core::tables::cap_spans(&doc);
        // Plugins' `init_state` and `decorations` run here; untracked, as under
        // the core (#943).
        let node_views: Rc<node_views::Registry> = Rc::default();
        let (state, view) = untracked_handler(|| {
            let state = EditorState::create(schema.clone(), doc, plugins.clone());
            let view =
                RinchDomEditorView::with_node_views(container, doc_ref, &state, node_views.clone());
            (state, view)
        });
        EditorHandle {
            inner: Rc::new(RefCell::new(EditorCore {
                state,
                view: Some(view),
                schema,
                plugins,
                on_change: None,
                on_link_click: None,
                on_link_hover: None,
                on_image_hover: None,
                on_key: None,
                on_image_input: None,
                on_selection_change: None,
                selection_owed: false,
                on_caret_moved: None,
                caret_moved_owed: false,
                anchors: Rc::new(RefCell::new(AnchorMap::default())),
                read_only: false,
                placeholder: None,
                tab_behavior: TabBehavior::default(),
                scroll: ScrollGate::default(),
                reveal: None,
                reveal_misses: 0,
                caret_hint: None,
                #[cfg(feature = "collaboration")]
                collab: None,
                node_views,
                margins: Margins::default(),
                on_margin_layout: None,
                spell: Default::default(),
                spell_seen: 0,
            })),
        }
    }

    /// Project this (unmounted) handle into `container`, building the view from the
    /// current state — so any content loaded before mount renders immediately. The
    /// caller owns `container` (an empty host element); `doc_ref` is a weak handle
    /// to the host document. Re-attaching a handle that is already mounted replaces
    /// its view (the old projection is abandoned).
    pub(crate) fn attach(&self, container: NodeHandle, doc_ref: Weak<RefCell<dyn DomDocument>>) {
        // A view this one replaces takes its node views with it: disposed with no
        // borrow held (a view's cleanup is app code).
        let (old, host_cx) = {
            let mut core = self.core_mut();
            let host_cx = Self::host_cx_of(&core);
            (
                core.view
                    .as_mut()
                    .map(RinchDomEditorView::dispose_node_views),
                host_cx,
            )
        };
        if let Some(old) = old {
            node_views::dispose_all(old, host_cx.as_ref());
        }
        let mut core = self.core_mut();
        let view = RinchDomEditorView::with_node_views(
            container,
            doc_ref,
            &core.state,
            core.node_views.clone(),
        );
        // The switch lives on the handle, so one set before mount (or across a
        // re-mount) is on the new container from its first frame.
        view.set_read_only(core.read_only);
        core.view = Some(view);
    }

    /// Mount this handle into `scope`: create its host container (`data-pm-editor`,
    /// deliberately **not** `contenteditable` so the legacy CE engine doesn't also
    /// activate), project the current state into it, and register the editor so the
    /// runtime drives its caret and routes input. Returns the container to place in
    /// the tree. The [`Editor`](crate::Editor) component calls this on render.
    ///
    /// The registration is released when `scope` is disposed (a conditional
    /// hide, a tab switch): the runtime stops driving this mount and forgets a
    /// link hover it held, without calling back into the disposed scope. The
    /// handle itself may live on and be mounted again.
    pub fn mount(&self, scope: &mut RenderScope) -> NodeHandle {
        let container = scope.create_element("div");
        container.set_attribute("data-pm-editor", "true");
        self.attach(container.clone(), scope.doc_weak());
        // Register scoped to this document — container ids collide across
        // documents on one thread (issue #134).
        let doc_key = scope
            .doc_weak()
            .upgrade()
            .map(|d| d.borrow().doc_key())
            .unwrap_or(0);
        let container_id = container.node_id().0;
        let token = crate::registry::register_editor_token(doc_key, container_id, self.clone());
        // Margin anchors set before this mount (or kept across a re-mount)
        // report once it lands: the web runs a pass only when one is owed.
        if self.inner.try_borrow().is_ok_and(|c| c.margins_wanted()) {
            crate::registry::owe_overlay_pass(doc_key);
        }
        let weak = self.weak();
        scope.on_cleanup(move || {
            // Its own registration only: the container may have been freed
            // and its id re-issued to another editor by now (#1498).
            crate::registry::unregister_editor_token(doc_key, container_id, token);
            // Every node view goes with the editor (its scope, then its root).
            if let Some(handle) = weak.upgrade() {
                handle.dispose_node_views();
                handle.release_margins();
            }
        });
        container
    }

    /// Apply the transaction built by `build` (given the current state), then
    /// re-project the host. `build` returns `None` to dispatch nothing. Returns
    /// whether a transaction was applied — `false`, too, when the editor is
    /// [read-only](Self::set_read_only) and the transaction would have changed the
    /// document. The single dispatch path — `command`, keyboard insert, paste, and
    /// IME commit all funnel through here.
    ///
    /// **Scrolling.** A transaction that sets the selection explicitly
    /// ([`Transaction::set_selection`], which `insert_text` and
    /// `replace_selection_with` do too) brings the caret into view on the next
    /// caret pass, as typing does. One that only edits the document and lets the
    /// selection be *mapped* — an app inserting text somewhere else, the shape a
    /// collaborator's edit takes — does not: a browser does not scroll a
    /// contenteditable to its caret because the DOM changed elsewhere, and a user
    /// who scrolled away would be pulled back.
    ///
    /// **Tracking.** `build` is the caller's own code and runs tracked: an app
    /// effect that reads a signal in it depends on that signal. The plugin code
    /// the dispatch then runs (`Plugin::apply`, `decorations`, …) does not
    /// subscribe the caller (#943).
    pub fn update(&self, build: impl FnOnce(&EditorState) -> Option<Transaction>) -> bool {
        self.dispatch_inner(build, false, true)
    }

    /// [`Self::update`], with the scroll decision made by the caller: `input`
    /// says the transaction is the user's own input at the selection (a keystroke,
    /// a paste, an IME edit), which brings the caret into view whether or not the
    /// transaction set the selection explicitly — an input rule's rewrite or an
    /// IME's surrounding-text delete maps it instead.
    ///
    /// `build` here is the handle's own (input rules, a plugin's paste claim)
    /// and runs untracked; see [`CoreGuard`].
    ///
    /// [`CoreGuard`]: super::guards::CoreGuard
    pub(super) fn dispatch(
        &self,
        build: impl FnOnce(&EditorState) -> Option<Transaction>,
        input: bool,
    ) -> bool {
        self.dispatch_inner(build, input, false)
    }

    /// [`Self::dispatch`]; `caller_build` runs `build` with tracking resumed,
    /// for [`Self::update`], whose `build` is the caller's code.
    fn dispatch_inner(
        &self,
        build: impl FnOnce(&EditorState) -> Option<Transaction>,
        input: bool,
        caller_build: bool,
    ) -> bool {
        let mut core = self.core_mut();
        if caller_build {
            // Resume tracking for the caller's closure, and only for it.
            core.untracked = None;
        }
        let built = build(&core.state);
        if caller_build {
            core.untracked = Some(rinch_core::reactive::suspend_tracking());
        }
        let Some(mut tr) = built else {
            return false;
        };
        // The history's typing-merge window reads the time (#940).
        crate::clock::stamp(&mut tr);
        let prev = core.state.clone();
        // The mapping has to be taken before `apply` consumes the transaction.
        let mapping = tr.mapping().clone();
        let kept = kept_nodes(&tr);
        let scroll = if input || tr.selection_set() {
            Scroll::IfChanged
        } else {
            Scroll::No
        };
        let next = core.state.apply(tr);
        let Some(doc_changed) = core.commit(prev, next, Some(&mapping), scroll, kept) else {
            return false;
        };
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        self.spell_after_dispatch(doc_changed, input);
        true
    }

    /// Read the editor state.
    pub(crate) fn with_state<R>(&self, f: impl FnOnce(&EditorState) -> R) -> R {
        f(&self.core().state)
    }

    /// A weak handle.
    pub(crate) fn downgrade(&self) -> WeakHandle {
        WeakHandle(Rc::downgrade(&self.inner))
    }

    /// The host container, once mounted.
    pub(crate) fn container_node(&self) -> Option<NodeHandle> {
        self.core().view.as_ref().map(|v| v.container().clone())
    }

    /// The host element of the textblock at `pos`.
    pub(crate) fn block_host(&self, pos: usize) -> Option<NodeHandle> {
        let core = self.core();
        let view = core.view.as_ref()?;
        let (id, _) = view.caret_address(&core.state.doc, Pos(pos + 1))?;
        Some(view.host_node(id))
    }

    /// Run `f` on the spellchecker's driver.
    pub(crate) fn spell_driver<R>(
        &self,
        f: impl FnOnce(&mut crate::spellcheck::Driver) -> R,
    ) -> Option<R> {
        let mut core = self.inner.try_borrow_mut().ok()?;
        Some(f(&mut core.spell))
    }

    /// Dispatch a transaction of the handle's own (not the user's input, not
    /// the app's `build`).
    pub(crate) fn update_internal(
        &self,
        build: impl FnOnce(&EditorState) -> Option<Transaction>,
    ) -> bool {
        self.dispatch_inner(build, false, false)
    }

    /// Add `plugin` to this editor, rebuilding its state over the **current**
    /// document and selection.
    ///
    /// The plugin list is otherwise fixed at construction
    /// ([`create_editor`](crate::create_editor) installs
    /// [`default_plugins`](rinch_editor_core::default_plugins)), which leaves an app
    /// no way to contribute one of its own — a spellchecker's decorations, say. This
    /// is that seam.
    ///
    /// Rebuilding the state **discards every plugin's folded state**, undo history
    /// included, so this is a construction-time call: add the plugin to a freshly
    /// created handle, before any content is loaded or typed. Adding a key that is
    /// already installed is a no-op, so calling it twice is harmless.
    ///
    /// Adding a plugin is **not an edit**: the document is the same node before
    /// and after, so a [read-only](Self::set_read_only) editor accepts it — a
    /// collaborating one included — nothing is recorded onto a collaboration
    /// session, and [`on_change`](Self::on_change) does not fire. The selection
    /// and any stored marks are carried over.
    ///
    /// Returns whether the plugin was added. It is all-or-nothing: `false` leaves
    /// the plugin list untouched, so a later call can still add it.
    pub fn add_plugin(&self, plugin: Rc<dyn Plugin>) -> bool {
        self.install_plugin(plugin, false, |plugins| plugins.len())
    }

    /// [`add_plugin`](Self::add_plugin), keeping every installed plugin's
    /// folded state (undo history included) when `keep` (the spellchecker
    /// added once the app's dictionaries arrive, #2862), at the index `at`
    /// picks from the installed list (clamped to its length): the end for
    /// `add_plugin`, the [`default_plugins`](rinch_editor_core::default_plugins)
    /// place for [`set_markdown_shortcuts`](Self::set_markdown_shortcuts)
    /// (#2210).
    pub(crate) fn install_plugin(
        &self,
        plugin: Rc<dyn Plugin>,
        keep: bool,
        at: impl FnOnce(&[Rc<dyn Plugin>]) -> usize,
    ) -> bool {
        let mut core = self.core_mut();
        if core.plugins.iter().any(|p| p.key() == plugin.key()) {
            return false;
        }
        let mut plugins = core.plugins.clone();
        let index = at(&plugins).min(plugins.len());
        plugins.insert(index, plugin);
        let prev = core.state.clone();
        let next = if keep {
            prev.reconfigure(plugins.clone())
        } else {
            let mut next =
                EditorState::create(core.schema.clone(), prev.doc.clone(), plugins.clone());
            next.selection = prev.selection.clone();
            next.stored_marks = prev.stored_marks.clone();
            next
        };
        // The identity mapping says what this is: a step forward over the same
        // document, not a load. `commit` treats a `None` mapping as a load, which
        // a read-only editor with a collaboration session attached refuses (a
        // load there is a write to the shared document) — and this used to pass
        // `None` after pushing the plugin, so that refusal left the key taken
        // and every retry refused too (review of #836).
        // Not an edit, so it never scrolls the caret into view.
        if core
            .commit(prev, next, Some(&Mapping::new()), Scroll::No, Vec::new())
            .is_none()
        {
            return false;
        }
        core.plugins = plugins;
        true
    }

    /// Take the plugin installed under `key` out of this editor (#2210) — the
    /// counterpart of [`add_plugin`](Self::add_plugin), and how an app drops one
    /// of the [`default_plugins`](rinch_editor_core::default_plugins): the
    /// markdown input rules ([`set_markdown_shortcuts`](Self::set_markdown_shortcuts)
    /// is the shorthand for those), or the history, to put a
    /// `HistoryPlugin::with(delay, depth)` of its own in its place, or none at
    /// all when an outer undo stack owns it.
    ///
    /// The same construction-time call as `add_plugin`, with the same terms:
    /// the state is rebuilt, which discards every plugin's folded state (undo
    /// history included); it is not an edit (a read-only editor accepts it,
    /// [`on_change`](Self::on_change) does not fire); the selection and stored
    /// marks are carried over. Returns whether a plugin was removed: `false`
    /// when none is installed under `key`.
    pub fn remove_plugin(&self, key: PluginKey) -> bool {
        let mut core = self.core_mut();
        if !core.plugins.iter().any(|p| p.key() == key) {
            return false;
        }
        let plugins: Vec<Rc<dyn Plugin>> = core
            .plugins
            .iter()
            .filter(|p| p.key() != key)
            .cloned()
            .collect();
        let prev = core.state.clone();
        let mut next = EditorState::create(core.schema.clone(), prev.doc.clone(), plugins.clone());
        next.selection = prev.selection.clone();
        next.stored_marks = prev.stored_marks.clone();
        // An identity step, as in `add_plugin`: not a load, never a scroll.
        if core
            .commit(prev, next, Some(&Mapping::new()), Scroll::No, Vec::new())
            .is_none()
        {
            return false;
        }
        core.plugins = plugins;
        // The placeholder slot names the plugin `set_placeholder` installed;
        // once it is gone, a later `set_placeholder` installs a new one rather
        // than changing the text of a plugin no state holds (#2210).
        if core.placeholder.as_ref().is_some_and(|p| p.key() == key) {
            core.placeholder = None;
        }
        true
    }

    /// Turn the markdown shortcuts on or off (#2210): the input rules that
    /// rewrite `- `, `1. `, `# `, `> `, `*x*`, `` `x` ``, `==x==` and `---` as
    /// they are typed ([`MarkdownInputRulesPlugin`](rinch_editor_core::MarkdownInputRulesPlugin)).
    /// Off in an editor from [`create_editor`](crate::create_editor), as in a
    /// Chrome `contenteditable`, which inserts what is typed literally; `true`
    /// turns them on. (A handle built by [`EditorHandle::new`] with
    /// [`default_plugins`](rinch_editor_core::default_plugins) has them on.)
    /// The `Editor { markdown_shortcuts: … }` prop calls this at mount.
    ///
    /// Installs or removes the plugin, so it is construction-time as
    /// [`add_plugin`](Self::add_plugin) is (the undo history restarts). It goes
    /// where `default_plugins` has it, right after the `BaseCommandsPlugin`
    /// (first when that is not installed), so its input rules run before those
    /// of plugins the app added. A call that changes nothing changes nothing.
    /// Returns whether they are on afterwards.
    pub fn set_markdown_shortcuts(&self, on: bool) -> bool {
        use rinch_editor_core::{BaseCommandsPlugin, MarkdownInputRulesPlugin};
        let key = MarkdownInputRulesPlugin.key();
        let installed = self.core().plugins.iter().any(|p| p.key() == key);
        match (on, installed) {
            (true, false) => {
                self.install_plugin(Rc::new(MarkdownInputRulesPlugin), false, |plugins| {
                    let base = BaseCommandsPlugin.key();
                    plugins
                        .iter()
                        .position(|p| p.key() == base)
                        .map_or(0, |i| i + 1)
                })
            }
            (false, true) => !self.remove_plugin(key),
            _ => installed,
        }
    }

    /// Whether the markdown shortcuts are on — see
    /// [`set_markdown_shortcuts`](Self::set_markdown_shortcuts).
    pub fn markdown_shortcuts(&self) -> bool {
        let key = rinch_editor_core::MarkdownInputRulesPlugin.key();
        self.core().plugins.iter().any(|p| p.key() == key)
    }

    /// Run the named command (applying + re-projecting if it applies). Returns
    /// whether it applied — a [read-only](Self::set_read_only) editor refuses every
    /// command that would change the document. The toolbar/keymap entry point.
    pub fn command(&self, name: &str) -> bool {
        let mut core = self.core_mut();
        let Some(cmd) = core.state.command(name) else {
            return false;
        };
        // `run_command_mapped`, keeping the attribute proof of the transaction
        // (an undo of a node view's `set_attrs` keeps the view mounted).
        let mut result: Option<(EditorState, Mapping, Vec<Kept>)> = None;
        let applied = {
            let state = &core.state;
            let mut dispatch = |mut tr: Transaction| {
                // The history's typing-merge window reads the time (#940).
                crate::clock::stamp(&mut tr);
                let mapping = tr.mapping().clone();
                let kept = kept_nodes(&tr);
                result = Some((state.apply(tr), mapping, kept));
            };
            let dispatch: rinch_editor_core::Dispatch = &mut dispatch;
            cmd(state, Some(dispatch))
        };
        let Some((next, mapping, kept)) = result.filter(|_| applied) else {
            return false;
        };
        let prev = core.state.clone();
        let Some(doc_changed) = core.commit(prev, next, Some(&mapping), Scroll::IfChanged, kept)
        else {
            return false;
        };
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        true
    }
}
