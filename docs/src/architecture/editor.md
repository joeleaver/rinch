# Rich-Text Editor Architecture

This page is the **structural** view of the editor: which crate owns what, how data
flows between them, which invariants each boundary exists to protect, and why the
boundaries are where they are.

It deliberately does not duplicate the concepts or the API:

- [Rich-Text Editor](../guide/editor.md) — the model, positions, schema, steps,
  transactions, commands, history, plugins. Read that first if you want to
  *understand* the editor.
- [Rich-text editing](../guide/contenteditable.md) — `create_editor()`, the
  `Editor {}` component, `EditorHandle`, the command catalogue.
- `docs/design/editor-rearchitecture.md` (in the repo, not in this book) — the design
  record: the milestone plan and the numbered decisions (§6, A3, A7, A22 …) that the
  source comments cite.

## Crate map

Three editor crates plus two platform-wiring crates, split along one axis: **how much
a crate is allowed to know about a renderer.**

| Crate | Knows about | Role |
|---|---|---|
| `rinch-editor-core` | nothing | The pure model: document tree, positions, schema, steps, state, commands, plugins, serialization. Defines the `EditorView` seam. |
| `rinch-editor-view` | `rinch-core`'s `DomDocument` trait | The projection: `RinchDomEditorView`, `EditorHandle`, the `Editor {}` component, the mounted-editor registry, the default stylesheet, the caret-blink clock. |
| `rinch-editor-collab` | a CRDT engine (`yrs`) | Optional, off by default: projects the model onto a CRDT and rebuilds remote changes back into `Step`s. |
| `rinch` | `rinch-dom`, winit, AT-SPI | Desktop wiring: input translation, the layout-coupled caret pass, block virtualization, accessibility, the cross-thread collab inbound. |
| `rinch-web` | `web_sys` | Browser wiring: the same jobs against the real browser DOM. |

`rinch-editor-core`'s manifest states the ban list as a contract — no `rinch-dom`,
`winit`, `web_sys`, `parley`, `taffy`, `vello`, and no CRDT engine — and it compiles
to `wasm32` unchanged. That is not tidiness for its own sake: it is what makes the
model testable without a renderer, shareable between desktop and browser, and
projectable onto a CRDT without either side learning about the other.

```text
┌──────────────────── rinch-editor-core (pure, wasm-clean) ─────────────────────┐
│  model/  Node · Mark · Fragment · Slice     pos/       Pos · ResolvedPos      │
│  schema/ NodeSpec · MarkSpec · ContentMatch transform/ Step · Mapping         │
│  state/  EditorState · Transaction          commands/ plugins/ serialize/     │
│  view.rs  trait EditorView   ◄── the only outward seam                       │
└───────────────────────────────┬───────────────────────────────────────────────┘
                                │ implements EditorView
┌───────────────────────────────▼───────────────────────────────────────────────┐
│  rinch-editor-view — RinchDomEditorView over ANY rinch-core DomDocument       │
│  EditorHandle (owns state + view) · Editor {} component · registry · styles   │
└──────┬───────────────────────────────┬────────────────────────────┬───────────┘
       │ desktop host                  │ browser host               │ optional
┌──────▼─────────────────────┐  ┌──────▼──────────────────┐  ┌──────▼───────────┐
│ rinch — rinch-dom renderer │  │ rinch-web — web_sys DOM │  │ rinch-editor-    │
│ input glue · caret pass    │  │ input glue · caret pass │  │ collab (yrs)     │
│ a11y · block virtualization│  │                         │  │ `collaboration`  │
└────────────────────────────┘  └─────────────────────────┘  └──────────────────┘
```

## The invariant everything else serves

**The model is the only source of truth, and the host tree is derived from it.**

`EditorState { doc, selection, stored_marks, plugin_state }` is a value.
`state.apply(tr)` is pure and returns a *new* state. The host tree — rinch-dom nodes
on desktop, real DOM elements in the browser — is a projection of that state and is
**never read back for content**. There is no `contenteditable` attribute anywhere;
the editor container is marked `data-pm-editor` precisely so nothing mistakes it for
a browser-managed editing surface.

Everything downstream is a consequence:

- **One mutation path.** Every edit becomes a `Transaction` of invertible `Step`s,
  applied by `EditorState::apply`, then projected. In `rinch-editor-view` that
  funnel is literally one function, `EditorCore::commit`: store the new state,
  call the view's phase-1 update, and (if collaborating) record the change onto the
  CRDT. `EditorHandle::update` and `EditorHandle::command` are the only two entry
  points that reach it, and typing, paste, IME commit and toolbar clicks all route
  through one of those two.
- **Commands read state, never the DOM.** Toolbar enablement (`can_run`), mark state
  (`is_mark_active`), block type (`current_block_type`) are all state queries. A
  command that consulted the host would be reading its own output.
- **"The DOM and the model disagree" is structurally impossible** — the failure class
  that motivated the rewrite. The previous architecture had several mutation paths
  writing to different sources of truth (the host tree, an Automerge document, a
  second cursor model) with no reconciliation between them; the design record's audit
  names that as the root cause.

## `rinch-editor-core` — the pure layer

The guide covers what these are; the architectural points are *why each is a single
thing rather than several*:

- **One position space.** `Pos` is a single depth-aware integer space (one unit per
  node boundary, one per Unicode scalar of text). Byte offsets exist only transiently
  at platform seams — Parley layout, IME — and never in the model. A second position
  representation would need a conversion at every boundary, and every conversion is a
  place for the two to drift.
- **The schema is enforced at the step boundary, not by convention.** Content
  expressions compile to a `ContentMatch` NFA. Every replace reconstructs its parent
  through one gate that checks the resulting child sequence against that parent's
  content expression *and* each child's marks against the allowed set; a violation is a
  `StepError`, the step is never accumulated into the transaction, and the edit is a
  no-op. Invalid structure is therefore unrepresentable in a committed state, so no
  downstream consumer — view, serializer, CRDT projection — needs a defensive path
  for it.
- **A deliberately minimal `Step` set.** `ReplaceStep`, `ReplaceAroundStep`,
  `AddMarkStep`, `RemoveMarkStep`, `SetNodeAttrStep`, `SetDocAttrStep`. Steps are
  invertible (undo), mappable (redo, decoration tracking, collaboration rebase) and
  composable. Tables and collaboration add **no** new step kinds — that is the test
  of whether the set is actually primitive.
- **Persistent tree, so identity is cheap.** `Node` is `Rc`-shared and every edit
  produces a new tree sharing unchanged subtrees. `Node::same_ref` (an `Rc::ptr_eq`)
  therefore answers "did this subtree change?" in constant time — which is what makes
  history cheap to keep and the view diff cheap to run.
- **Nothing is special-cased.** The command catalogue + keymap
  (`BaseCommandsPlugin`), the markdown input rules (`MarkdownInputRulesPlugin`) and
  undo/redo (`HistoryPlugin`) are all plugins; `default_plugins()` just lists them.
  History has no privileged hook in `apply` — it folds its state forward like any
  other plugin. Features that cannot be added as plugins are a design smell, not an
  excuse to reach into the core.

## The view seam

`rinch-editor-core::view` defines the whole outward contract in two methods:

```rust
pub trait EditorView {
    fn update_dom(&mut self, prev: &EditorState, next: &EditorState) -> Vec<ViewRequest>;
    fn update_caret(&mut self, next: &EditorState) -> Vec<ViewRequest>;
}
```

Three structural decisions live in that shape.

**It is two phases because layout sits between them.** `update_dom` runs *before*
layout and mutates the host. `update_caret` runs *after* layout, once the host has
fresh geometry, and computes caret/selection rectangles and the IME candidate box.
Caret geometry computed in the call that mutates the host would read stale layout —
so the seam straddles the host's *mutate → layout → measure* pipeline rather than
pretending it isn't there.

**Requests cross back as data.** The view owns no window; only the runtime does. So
"scroll the selection into view" is a returned `ViewRequest` the runtime fulfils, not
a call the view makes.

**Input translation is deliberately absent from the trait.** Turning a key, pointer
or IME event into a command is irreducibly platform-specific — desktop reads Parley
geometry to map a click to a `Pos`; the browser has its own event model — so each
runtime owns that glue. The trait covers only the state→host direction, which is the
part that *can* be shared.

## `rinch-editor-view` — one projection, two hosts

`RinchDomEditorView` implements `EditorView` against
`rinch_core::dom::DomDocument`, the same trait the reactive layer uses. It holds
only a `Weak` reference to the document and never names a concrete renderer — which
is why the desktop (`rinch-dom`) and browser (`web_sys`) editors are the *same* view
code, and why its own tests project onto a mock document with no renderer present at
all.

**The `ViewDesc` tree** mirrors the document: one descriptor per model node, holding
its host node, its mark-wrapper chain, and its children. Diffing walks descriptors
against the new document, with `Node::same_ref` as a fast skip — an unchanged subtree
costs one pointer comparison. The diff is **positional, not keyed**: text edits are
local, so position is a good proxy for identity, and a keyed/LIS pass would buy
nothing until reorder churn matters. A node whose *kind* changed (tag or mark set)
is rebuilt and swapped rather than patched in place, because a changed mark set
alters the wrapper chain around it. The root's host element is created by the caller
(the `Editor` component) and is never replaced — only its children reconcile.

**Decorations diff independently of the document.** A transaction that changes only
decorations — an IME preedit, a placeholder appearing — must still produce a visible
update, so `update_dom` runs the document diff and a separate decoration sync. Folding
them together would make preedit invisible whenever the document happened not to
change.

**`EditorHandle` owns both the state and its projection**, behind an `Rc<RefCell<…>>`
so it is cheap to clone into toolbar closures and the runtime alike. It works
*unmounted*: `create_editor()` builds a handle over a fresh document with no host at
all, and `load_html` / `command` / `set_selection` operate on the owned state, which
the view renders when the `Editor {}` component mounts it. This is what lets a handle
be created in a component body and handed to buttons *and* to `Editor {}` in the same
`rsx!` block.

**The registry is keyed by `(doc_key, container_id)`, not by container id alone.**
Container ids are per-document slab indices, so two documents on one thread — two
desktop windows, or two embedded `RinchContext`s — collide at the same id (issue
#134). Keyboard focus is *not* tracked here: the platform runtime is the single focus
authority, and it resolves its focused container id to a handle through the registry.
The `Editor` component registers on mount and unregisters via `scope.on_cleanup`, so
a hidden tab's editor stops being driven while its handle (and state) may live on.

## Platform wiring

The wiring crates hold exactly what cannot cross the `DomDocument` seam.

**Desktop (`crates/rinch/src/editor/`)** re-exports `rinch-editor-view` wholesale and
adds three things that reach past the seam into the `rinch-dom` renderer or a native
platform API:

| Path | Why it can't be in the shared view |
|---|---|
| `virtualization.rs`, `virtual_window.rs` | Block virtualization gives off-screen blocks a fixed estimated height so Taffy skips their Parley measurement. It manipulates the concrete Taffy layout tree. |
| `a11y.rs` | The accessibility *derivation* is portable (`rinch_editor_core::a11y` → an `accesskit::TreeUpdate`); only the adapter is not. Linux uses `accesskit_unix` over AT-SPI; other desktops get a no-op bridge. |
| `post_remote_delta` | The `Send`-safe collaboration inbound: marshals a delta from a network thread onto the main thread, then integrates it and wakes the runtime. |

Input translation and the caret pass live in the runtime proper:
`app/event_dispatch.rs` turns platform key/pointer/IME events into `KeyBinding`s and
handle calls, `app/focus.rs` holds the focus arbiter, and `shell/rinch_runtime.rs`
drives the post-layout caret pass and the blink clock.

**Browser (`crates/rinch-web/`)** does the same jobs against the browser DOM in
`editor_input.rs`, and re-exports `Editor` / `EditorHandle` / `create_editor` so web
app code is *identical* to desktop app code. `rinch-web` depends on
`rinch-editor-view` unconditionally; on desktop the editor arrives with the `desktop`
feature.

## Serialization

`rinch-editor-core::serialize` is the durability boundary, and its contract is
**totality**: a node or mark either round-trips faithfully or the boundary returns an
error. Nothing is dropped silently and nothing is rendered to a fallback tag.

| Module | Shape | Notes |
|---|---|---|
| `doc_json` (feature `serde`) | `DocNode` | The durable save/load shape. `Node::to_doc()` ⇄ `Schema::node_from_doc()`. Unknown types and missing required attrs are hard errors. |
| `html` | HTML string | Schema-driven tags (`node_dom_tag` / `mark_dom_tag`), whitelist parse. The *same* table the view uses to pick host tags and that `Editor { content: … }` / `load_html` parse with. |
| `markdown` (feature `markdown`) | markdown | Via `pulldown-cmark`. |
| `text` | plain text | The lossy `text/plain` clipboard fallback. |

That the view and the serializers share one tag table is the point: copy-out,
paste-in, and on-screen rendering cannot disagree about what a `heading` is.

## Collaboration — an optional projection

`rinch-editor-collab` projects the model onto a **yrs** (Yjs) CRDT and rebuilds
remote CRDT changes back into `Step`s. Architecturally it is a *peer* of the view: a
second projection of the same model, sitting behind the same one-way mutation path.
The model does not know it exists.

- **Off by default.** Gated behind the `collaboration` feature, so default builds —
  desktop *and* web — link zero CRDT code. This is the only crate in the workspace
  that links a CRDT engine.
- **One invariant:** `model ≡ project(model)`. Every local step is projected onto the
  CRDT; every remote change is rebuilt into the model. Convergence then follows from
  the CRDT's own convergence rather than from hand-written merge logic.
- **Fail loud, never silently drop.** Content outside the staged scope returns
  `CollabError::Unsupported` and leaves the CRDT untouched — projection is
  all-or-nothing. A silent drop would reintroduce exactly the divergence class the
  rewrite eliminated.
- **wasm-compatible with no shims**, so the desktop and browser editors share one
  adapter instead of bridging to a separate JS CRDT.

For the API, the staged scope, the transport contract and the engine-choice rationale
see the Collaboration section of CLAUDE.md, the
[Rich-text editing](../guide/contenteditable.md) guide, and
`docs/design/yrs-migration.md`.

## Not the rich-text editor: `rinch-editable`

`rinch-editable` is a **separate, unrelated** engine for single-line `<input>` and
`<textarea>` widgets: `StringDocument`, `EditCommand`, `EditableState`,
`InputHandler`, and its own `Position`/`Selection`/`UndoStack` types. It shares no
code and no types with `rinch-editor-core` — the identical names live in different
crates. The quickest tell: `rinch-editable`'s `Selection` is a **struct** of two
`Position`s, while `rinch-editor-core`'s is an **enum** of `Text`/`Node`/`Cell`.

The split is intentional: a text field does not need a schema, steps, or a CRDT, and
the rich editor should not carry a second document model for the sake of one.

## Related

- [Rich-Text Editor](../guide/editor.md) — the model and its concepts.
- [Rich-text editing](../guide/contenteditable.md) — the component and command API.
- [Architecture Overview](./overview.md) — where these crates sit in the whole system.
- [RenderScope API](./render-scope.md) — the `DomDocument` / `NodeHandle` seam the
  view projects onto.
- [Fine-Grained Reactivity](./fine-grained.md) — how the surrounding UI updates.
