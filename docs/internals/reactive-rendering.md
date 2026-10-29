# Fine-grained reactive rendering: node lifetime, styles across detach, animations

Read this page when your task touches: remove/discard, scope ownership, cache scopes, execution order, transitions/animations on detach and hide, transform interpolation.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

## Fine-Grained Reactive Rendering

Rinch uses fine-grained reactive rendering for surgical DOM updates. Instead of regenerating HTML on every signal change, reactive expressions become Effects that update specific DOM nodes.

### Architecture

```
Signal.set() → Effect runs → NodeHandle.set_text() → Minimal re-layout
```

**Key principle:** `app()` runs once to build the DOM. Effects handle all reactive updates surgically.

### Key Components

| Component | Location | Purpose |
|-----------|----------|---------|
| `NodeHandle` | `rinch-core/src/dom/` | Stable reference to a DOM node for surgical updates |
| `RenderScope` | `rinch-core/src/dom/` | Context for building DOM trees with effect tracking |
| `DomDocument` | `rinch-core/src/dom/` | Trait abstracting DOM mutation operations |
| `RinchDocument` | `rinch-dom/src/lib.rs` | DOM implementation using Taffy + Parley + Vello |
| `rsx!` | `rinch-macros/src/lib.rs` | Macro generating DOM construction code |

### Taking a node out: `remove()` vs `discard()`

**`NodeHandle::remove()` is a detach, on every backend** (issue #719). The node
and its whole subtree keep their identity: append the handle again and the
subtree comes back exactly as it was, and you may read, style or restructure it
while it is out. `replace_with()` leaves the node it displaced in the same
state. That post-condition is what a reactive branch re-showing a **captured**
handle rests on — `rsx!`'s `if cond { {panel} }` desugars to `show_dom` with a
branch closure returning that same `NodeHandle` every toggle, and `match` arms
and a memoised `for` row are the same shape.

**`NodeHandle::discard()` says the caller is finished with the subtree for
good.** Treat a discarded handle as dead — build a fresh node rather than
re-attaching one — because a backend is then free to drop its bookkeeping and
make every operation on it a silent no-op. `rinch-web` and the test
`MockDomDocument` both do, so re-attaching a discarded node fails `cargo test`
as well as a browser.

**The contract is one-sided on purpose.** A `discard()` is *at least* a
`remove()` and may be much more; you may not rely on it being less. On
`rinch-dom` today it is exactly a `remove()` — a discarded node still
re-inserts, still keeps its subtree and still takes writes — and that is #723,
not a promise. Writing code that depends on desktop's inertness is the #719
mistake one verb along: right on desktop, dead on web, with nothing in the app's
own tests to say so. The mock is what closes that.

**A discarded id is never re-issued** on either backend, so a stale discard
handle names nothing rather than somebody else: `rinch-web`'s counter is a
monotonic `fetch_add` with no free list, and `rinch-dom` frees nothing on this
route. That is a claim about **`discard` alone** — `rinch-dom` does free slab
keys through `set_inner_html` and pseudo-element pruning on restyle, and
`slab::Slab` recycles them (measured: `NodeId(4)` handed to a second node), so
#304's recycled-slot hazard is live on desktop today, independently of this API.

### Who picks the verb: **scope ownership**, not a judgement call

The four marker-based reactive helpers — `show_dom`, `match_dom`,
`reactive_component_dom`, and `for_each_dom_typed` / `virtual_list` rows —
cannot know whether a subtree will be wanted again, so they do not guess. Each
runs its user closure inside its own `RenderScope`, and **a node that scope
minted is the helper's to discard; a node the closure was handed is the
caller's, and is only detached**. That is the rule #141 PR4 gave signals and
effects (`RenderScope::created`), applied to nodes.

It falls out of that, with no special cases:

| shape | verb | why |
|---|---|---|
| `if open { p { "hi" } }` — fresh markup | `discard` | the branch built it; nothing can show it again |
| `if open { {panel} }` — a captured handle (the #654 shape) | `remove` | the closure was handed it; the next show puts it back |
| a `render_fn`, branch closure or `for` view that **memoises a subtree built outside it** | `remove` | same reason: the closure was handed the node, so it is the caller's |
| a `for` view that builds a row once **through `s.cache_scope()`** and caches it with that scope (#733) | `remove` | the row's scope did not mint it; the cache scope has no parent |
| a `for` view that builds a row once **through the row's own scope** and caches it | `discard` — the row is lost on web; counted, and a `tracing` warning emitted, only when it is the closure's root node | the row's scope minted it |
| a nested `for`'s rows inside a discarded branch | reclaimed | the discard is recursive, and nothing outside minted them either |
| `if open { div { {panel} } }` — a captured handle *inside* branch-built markup | `remove` for `panel`, `discard` for the `div` | the walk below takes `panel` out before the recursive discard reaches it (#732) |

**A root is asked `created`; everything under a root the helper discards is
asked whether the branch's render built it (#732).** "Built it" is scope
ancestry, not `created` (a `for` row, a nested branch, a component re-render or
a late patch is minted by a scope of its own) and not id order (code that merely
runs during the render is not the render's). Every `RenderScope` has a
`ScopeId` and a parent fixed at construction (`RenderScope::with_parent(doc,
node, Some(scope.id()))`); every node it mints is recorded against its id; a
node is owned when the chain from its minting scope reaches the branch's scope.
The four helpers, `virtual_list`'s rows and spacers and rinch-components'
`late_children` patches (`List::icon`, `Stepper`, `RadioGroup`) all pass the
scope they were called from, captured once, so content built later by an effect
or an observer still chains back. **`RenderScope::new` has no parent**, and is
not defaulted from whatever is rendering: nodes a parentless scope mints inside a
branch are nobody's, and a hide only detaches them — a leak, never a loss. Any
code that builds nodes on another scope's behalf must use `with_parent`; content
meant to outlive a branch (a cache) must not name the branch's scope, or the
hide discards it under the cache. **A node with no record at all** — minted by
raw backend access: the editor view's blocks, `parse_html` — belongs to whatever
owned node it sits under and is discarded with it. That is how every nested node
was treated before #732, and it includes a raw-minted handle the app captured
and nests in branch markup: it is retired with the wrapper, exactly as on main
(`raw_minted_nodes_732.rs` pins both).

The walk is `render_scope::sweep_for_discard`: one pass over the subtree (ids
from `get_children`, document and table borrowed once) that drops the records of
what is about to be discarded and collects the first non-owned node on each
branch without entering it; the helper detaches those, then discards with
`NodeHandle::discard_swept`. `show_dom`/`match_dom`/`reactive_component_dom`
sweep before disposing the old scope; a parked `for`/`virtual_list` row sweeps
at release, by its scope's id. The tables are per document (`Ancestry`), held
by that document's `RenderScope`s and dropped with the last one, slot included.
A scope's entry outlives the scope while a node it minted is recorded or a child
entry names it, so a throwaway scope's nodes (a late patch, a spacer) still
chain to its parent. A record goes when its node is discarded or replaced by
`NodeHandle::set_inner_html`; a node that is only detached keeps it.
**`NodeHandle::set_text` over an element's children still only detaches them,
and collects later** (#1487). The children the element's own render built —
minted by the scope that minted the element, or one descended from it — are
filed under that scope (`Ancestry::orphans`; under its nearest live ancestor if
it is gone, and not at all if none is), and when that scope drops, after its
cleanups, each one still detached, not retired and with an unchanged minting
record is discarded (`render_scope::discard_filed_orphans`; a captured handle
inside is taken out first, with no child observer told,
`late_child::without_notifications`). Orphans are filed per node (`filed`,
with a per-scope set), so a host toggling text over one kept child holds one
entry, and any `NodeHandle` insertion verb (`append_child`, `insert_before`,
`insert_after`, `replace_with`) unfiles the node it puts back
(`render_scope::unfile`, one `Cell` read while nothing is filed): from then on
it is an ordinary node, kept at the scope's end even if `remove()`d again. The
record check matters only for a node put back through the backend directly,
whose id `rinch-dom` may free and re-issue. So every handle re-inserts while its
scope lives — a static component's handed children share its caller's scope,
and a component may keep a handle to its own child — and a branch's orphans go
with the branch. Before, every orphan stayed in the node and minting tables for
the life of the document: +300 / +300 per 100 show/hide cycles of
`div > (span > text, span)` set over with text, in Chrome 155 and on the mock.
A child the render was handed, or with no record, is only detached. A long-lived
scope's orphans live as long as it does. A minting record carries a text bit
(`Minted`), so a write to a scope-minted text node — every reactive `{|| text}`
— asks the backend nothing. `child.remove()` with the handle dropped is #1508.
Pins: `reinsertion_tests::text_over_children_1487`,
`rinch-macros/tests/text_over_handed_children_1487.rs`,
`rinch-dom/tests/text_over_children_1487_tests.rs`,
`rinch-web/tests/text_over_children_1487.rs`. Cost: one
hash insert per minted node, and the sweep per hide — and every plain
`NodeHandle::discard()` sweeps its subtree too (`sweep_for_discard(self, None,
..)`), including the editor's `ViewDesc` discards; on web each `get_children`
there is a JS call. Measured: +9.8% instructions on a 1000-row hide (`shell::branch_hide` in
`rinch-bench`). Pins: `reinsertion_tests` (the `*_nested_*`, `*_later_*`,
`virtual_list_*`, `a_late_child_*`, `depth_three_*` and table-growth fixtures),
`render_scope::ancestry_tests`, `rinch-core/tests/raw_minted_nodes_732.rs`,
`rinch-editor-view/tests/editor_in_branch_732.rs`,
`rinch-components/tests/list_in_branch_732.rs`,
`rinch-web/tests/{reinsertion,branch_reclaim_732,editor_in_branch_732}.rs`,
`rinch/tests/embed_drop_minting_732.rs`.

**A cache filled from inside the closure builds through a cache scope (#733).**
A `view` closure that builds *lazily through the row's own scope* and caches
afterwards owns its row by this rule, so the first removal discards it — and
disposes every effect, signal and handler it made, on every backend (the
reactive half is #141's rule, not the node rule: counted on the mock, an effect
built through the row scope did not run again for a write made after its row
had left and come back). A
branch closure or a `render_fn` can build its cached subtree outside itself and
capture it; a `for` view is only ever handed the row's scope. So the view asks
for a scope that is not the row's:

```rust
move |id: u32, s: &mut RenderScope| {
    if let Some((row, _scope)) = cache.borrow().get(&id) {
        return row.clone();
    }
    let mut keep = s.cache_scope();                  // parentless, same document
    let row = keep.build(|__scope| rsx! { article { {id.to_string()} } });
    cache.borrow_mut().insert(id, (row.clone(), keep));   // the scope stays with the node
    row
}
```

In `rsx!` that closure is the `for` body, where `__scope` is the row's scope:
`for id in ids.get() { {cached_tab(__scope, &cache, id)} }`
(`rinch-macros/tests/rsx_for_cache_scope_733.rs`).

`RenderScope::cache_scope(&self) -> RenderScope` is `RenderScope::new` on the
same document: no ancestry parent, so no hide or row removal discards what it
mints, including a cached node nested in row-built markup (the #732 walk takes
it out first). `RenderScope::build(&mut self, f) -> R` runs `f` with the scope
as the **ambient owner**; forgetting it (building through the cache scope's
`&mut` directly) is silent: the node comes back, but a `Signal::new`, memo or
`onclick` made while building belonged to the row that happened to be rendering
and was freed with it, so its writes are no-ops and its bindings never update.

**The scope and the node are two values the caller releases in step, and
nothing ties them** (one owning value is #1454): drop the scope and keep the
node → an inert row; discard the node and keep the scope → its effects keep
running against a retired node; **drop both without discarding — which is what
dropping the cache does, when the component that owns it unmounts — → the
effects stop and every cached node stays in the backend for the life of the
page.** A parentless scope's nodes are only detached by the hide around them
(#732's rule) and `RenderScope`'s `Drop` discards nothing. Measured on the mock
and in Chrome 153: 50 mounts of a list of two cached rows leave **+200** nodes,
and **0** when the owner drains the cache in `on_cleanup`:

```rust
let evict = cache.clone();
__scope.on_cleanup(move || {
    for (_, (row, keep)) in evict.borrow_mut().drain() {
        keep.dispose();
        row.discard();
    }
});
```

Evicting one entry is the same pair. A key that never returns holds its row
until then. The `Changed` arm hands the same cached node back for a changed
item under one key, so a cached row is not rebuilt when its item changes.

**A closure whose ROOT node is retired is counted, and a `tracing` warning is
emitted for it** — the old shape, still lost: `dom::warn_if_retired`, called on
what every `for` (initial, insert and changed arms), `virtual_list`, `if`,
`match` and component re-render closure **returns**, bumps a per-thread counter
(`dom::__retired_view_returns() -> (returns, nodes warned)`) and emits one
`tracing::warn!` per `(doc_key, node)`, for the first 32 nodes on a thread and
none after. Three limits, each measured:

- **Nobody sees the warning by default where it can fire.** The desktop shell
  is the only place rinch installs a `tracing` subscriber, and there
  `is_retired` is always `false`; `rinch-web` installs none (#1455) and a
  `cargo test` run prints none. A test asserts on the counter.
- **Only the returned node is asked.** A retired node nested in markup the
  closure builds fresh (`for t in tabs { div { {cached} } }`) loses the row the
  same way — the wrapper comes back empty — and is neither counted nor logged
  (#1456).
- It asks `DomDocument::is_retired` (default `false`), which `rinch-web` and
  `MockDomDocument` answer from their node tables; **`rinch-dom` cannot tell**,
  because its `discard` reclaims nothing (#723) and the row still comes back
  there — so desktop alone gives no sign that the same code loses the row in a
  browser, and `branch_helper_transition_tests`'s lazy `for` fixture still
  passes on it.

Pins: `reinsertion_tests::lazy_memo_733` and `cache_scope_733_tests` (mock),
`rinch-web/tests/for_memo_733.rs` and `for_memo_733_shapes.rs` (Chrome 153),
`rinch-macros/tests/rsx_for_cache_scope_733.rs` (the guide's recipe, compiled):
the row comes back, its effect runs while it is out, 200 toggles grow the node
count by 0, an eviction returns the count to where it was, and the two leak
directions above.

Every other release site says the verb outright, because it knows: the editor's
`ViewDesc` diff (popped children, kind-changed blocks, placeholder, selection
rects), `virtual_list`'s drained spacers, `Stepper`'s replaced default glyph,
and DevTools' rebuilt panels all `discard`.

**"Takes its whole subtree with it" is true of everything *in* a subtree, and a
node in none is never reached.** An `rsx!` component site mints a scratch
`<template>`, builds the site's children into it, and hands them to
`Component::render`, which re-parents the ones it adopts; the container is then
dead and attached to nothing, so no walk from a branch's content root can find
it. That is `rinch_core::dom::release_scratch_container`, called at the three
codegen sites and at `element.rs`'s `Element::Component` arm — one orphan per
component render otherwise, measured at **+99 over 198 toggles** of
`if open { Card {} }` in Chrome and on the mock alike. Anything still under the
container was not adopted and leaves with it, by the same ownership rule one
level down.

Two ordering facts that fall out and are easy to get wrong: the verb is chosen
**before** the branch scope is disposed, so a cleanup that re-parents a
scope-built node during disposal cannot rescue it — and it is *applied*
**after**, so a cleanup still reads its own node live and mounted. That holds for
all four helpers; a `for` or `virtual_list` row got it with #356, whose rows used
to be discarded mid-pass with the scope disposed at the end, which on web left
the row's cleanup reading `None` off a retired node. A row cannot be disposed
where the reconcile takes it out (disposal runs user code under the list's
`RefMut`s, #141), so it is **parked** — node, verb, scope — and torn down after
the borrows are released (`for_loop::release_parked`), skipping a node a live row
holds again. And the scratch container is
released **after** `Component::render`, so the children it adopted have been
re-parented out by then.

**Getting it wrong is silent either way**: `remove` where `discard` was meant
costs memory on web, `discard` where `remove` was meant costs the subtree. The
suite pins **both** directions for every helper — a re-show fixture and a
node-count-over-200-toggles growth fixture, on the host through the mock and in
real Chrome through `NODE_REGISTRY`. A round of PR #728 had only the re-show
half for `show_dom`/`match_dom` and leaked one subtree per toggle in a browser
with the board green.

This divergence is what #719 was. `rinch-web`'s `remove_node` used to prune both
maps, so on that backend alone the first hide retired the id and every later show
inserted nothing — reproduced in Chrome 150, silent, no error anywhere.
`crates/rinch-core/src/dom/mock.rs` is the host-runnable oracle for both verbs
(it retires a discard and keeps a remove), which is why a `remove`/`discard`
mistake now fails `cargo test` rather than only a browser.

### Usage

Components use `#[component]` and return a `NodeHandle`:

```rust
use rinch::prelude::*;

#[component]
fn counter() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        div {
            // Closure syntax {|| ...} creates a reactive Effect
            p { "Count: " {|| count.get().to_string()} }

            // Reactive styles also use closures
            div {
                style: {|| format!("width: {}px", count.get() * 10)},
                "Progress bar"
            }

            button { onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}
```

The closure syntax `{|| expr}` tells the macro to create an Effect that:
1. Runs the closure and renders the initial value
2. Tracks which signals are read inside the closure
3. Re-runs and updates only that DOM node when those signals change

Without the closure, expressions like `{count.get()}` are captured once at initial render and never update.

### How It Works

1. **Initial Render**: Component runs once, creating DOM nodes via `RenderScope`
2. **Effect Setup**: Dynamic expressions (`{|| expr}`) become Effects with NodeHandles
3. **Signal Changes**: Effects run and surgically update their target nodes
4. **Batched Updates**: Multiple updates are collected for efficient re-layout

**Execution order is a contract** (#154): effects observing the same signal run in **registration order** (the order their `Effect`/`Memo` was created), and the pending queue drains FIFO — so an effect registered *after* an `rsx!` tree sees the post-patch DOM in the same flush ("run me last"), and a signal written from inside an effect queues its observers *behind* the current flush rather than preempting it. **The one exception is the running effect itself** (#343): a write flushed while its own body is on the stack — any write made with no batch open, and every flush runs with the batch closed — skips that effect rather than queueing it, so a cleanup of a scope the body disposes cannot re-run the disposing effect (that is what keeps `for_each_dom`'s reconcile from re-running for a row's cleanup write). A body running inside an **open** batch (`Effect::new`/`Effect::run` in a handler) queues its writes, and does re-run for them. Deliberate, not a bug: queued, a body that disposes a fresh scope each run would never settle. Pinned by `reactive::effect::self_wake_tests`. Enforced by `BTreeSet<ObserverId>` subscriber sets (ids are monotonic and never reused, so ascending id *is* registration order) plus `pop_front` in `flush_effects`. Don't swap either for a `HashSet`/LIFO. See `docs/src/guide/reactivity.md#execution-order`.

**A node outside the document is not styled** (#651, #668). `set_attribute` /
`remove_attribute` / `set_style` record the node in `tree.style_roots`, and
`resolve_styles` **drops** an entry whose node is not connected to the document
before cascading anything — "what is this element's style?" is a question CSS
answers only for elements in a document. It used to answer anyway, against no
parent at all, which is a cascade in which every inherited property lands on its
*initial* value and no descendant selector can match:

- **Unmount.** A removed subtree recascaded to `font-family: serif` / `color:
  black`, and `build_ifc_layouts` — which collects its roots from the whole node
  slab rather than by walking the document (#628) — reshaped its text in serif
  from there. That is where #654's serif came from.
- **Mount.** A component classes a child before splicing it in, so anything that
  resolves in that window cascaded the child parentless *and* set its
  `has_been_styled` flag. Its first real resolution then read as a **change** on
  an already-styled node, which is exactly what `transition` waits for: every
  mount of a component sized by a modifier class on its wrapper (`Checkbox`,
  `Switch`, `Select` all are) animated to its own size. A browser never animates
  there, because an element enters the document already carrying its final style.

Two consequences worth knowing. **A detached node's `computed_style` is now
whatever it last resolved to *in* the document** — a never-attached node's is the
default `ComputedStyle` — rather than an invented parentless cascade. Every
reader keeps working. Paint and hit testing never see one, since both walk from
`tree.body_id`, and so do `query_selector` and an unscoped `dom_tree`; but
`dom_tree` takes a `root_id` and hands it straight to the serializer, so
**`dom_tree(root_id: <a detached id>)` does reach one** — and what it reports is
that last in-document style, where before this change it reported the parentless
cascade's `serif`. And connectivity is asked at **resolve** time, not where the
entry was pushed, so a node classed while detached and spliced in before the next
layout is still styled by that same entry. Entering the document is what styles a
node, through `recompute_node_styles_recursive`, which every insertion route ends
in (`append_child`, `insert_before`, `insert_child`, `replace_node`) — except a
styled node moved within its own parent, which never left and keeps its style
(#914, see **Style invalidation**).

**A subtree that leaves the document loses its before-change style** (#699) —
the same rule as above, read from the other end. A node styled while it was
*connected*, then detached, used to keep `has_been_styled` **and** the
`computed_style` it had in the document; so if an ancestor's class changed while
it was out, its re-insertion resolved to a different value, the cascade read
old ≠ new on an already-styled node, and the box **animated in from a style the
user never saw**. `RinchDocument::detach_subtree_styles` now clears the flag and
cancels any running transition **and animation** for the **whole removed
subtree**. Cancelling the transition is not optional — `tick_transitions` walks
`tree.active_transitions`, not the document, so one left behind would go on
writing interpolated values straight through the re-insertion. Cancelling the
*animation* is a separate repair riding along: an animation has no declared
duration to expire, and the desktop shell keeps asking for frames while
`tree.active_animations` holds a running (not paused) animation, so a removed
`Loader` kept an app rendering forever.

**Five places in `dom_impl/dom_document_impl.rs` write `parent = None`; four
call the helper.** `remove_node` (every reactive removal funnels through
`NodeHandle::remove`), `remove_child`, `replace_node`'s displaced `old`, and —
least obviously — **`set_text_content` on an element with children**, which
orphans every one of them without freeing the slab, so a handle the app still
holds stays alive and styled. That fourth one was missed on the first pass, when
this paragraph said "the three routes"; it is the count to re-check against
`grep -n '\.parent = None'` if a sixth ever appears, because an unhooked one is
silent. The fifth is `set_inner_html`, safe by **destruction** rather than
reset — it calls `NodeTree::remove_subtree`, which frees the entries and drops
both animation maps with them.

Three things it deliberately does not do.

- **It does not clear `computed_style` or `text_layout`.** A detached node still
  reads back as it last did in the document (above), and the re-insertion's own
  staleness gates compare against that same style to decide whether to re-shape
  the text (#654, #661, #678). The flag is what the transition reads; the value
  is what everything else reads.
- **`display: none` is not a detach**, and does not need to be (**#703**). §3
  asks whether an element is being *rendered*, not whether it is in the
  document, so the cascade answers that half itself: before starting a
  transition it checks the node's display **before** the change as well as
  after, then walks its ancestors — `display` does not inherit, so a box under a
  hidden wrapper computes `display: block` and its own style says nothing. A
  change made while an element is hidden lands outright, so it is already at its
  new value when shown; an element that stops being rendered has its
  transitions cancelled, and so does everything under it. `visibility: hidden`
  is **rendered** and still transitions.
  **The rule this puts on the component library**: *an overlay that animates
  must stay rendered — animate `opacity`, `visibility` or `transform`, never
  toggle `display`* (**#751**). A `display` flip cannot transition on **either**
  backend, because a browser refuses it for the same reason, which is what
  `@starting-style` and `transition-behavior: allow-discrete` exist for.
  `Drawer` was the one component that had it backwards: its root toggled
  `display: none` while its panel transitioned `transform`, so the slide-in ran
  on desktop only until #703 and had never run on `rinch-web` at all. Its closed
  state is `visibility: hidden` now, like `Popover`'s — hidden, still rendered,
  and — once its close has run — out of paint, hit testing and the Tab order
  on both backends (for the 300ms slide-out the root is held `visible` and
  still paints, takes clicks and is tabbable, as in a browser; #759).
  **Out of paint now covers the content, not only the box** (#829): desktop used to gate only a hidden
  box's own background, border and shadow, and went on drawing the text its
  IFC laid out — so a closing `Drawer` or `Popover` left its title and body on
  screen, with no panel behind them, for the length of the slide or fade.
  `paint::text::TextMask` now gives each IFC text range the visibility of its
  text node's parent element and drops hidden glyphs (and their share of the
  underline, line-through and `text-shadow`, and of the wavy underline
  `paint_wavy_decorations` draws, cluster by cluster) glyph by glyph, since a
  `display: contents` wrapper that differs only in `visibility` shares its
  neighbours' Parley glyph run; inline backgrounds, SVG shapes, scrollbar
  thumbs, a `RenderSurface`'s frame, the `filter` overlay and the read-only
  selection highlight are gated too. Because visibility is read at paint time,
  a box-less inline's restyle dirties its IFC root's rect
  (`compute_dirty_region`), or the incremental frame would keep the old text.
  A hidden span on a line `text-overflow: ellipsis` cuts is hidden too: the
  cut keeps the lines and their text ranges (#853, #1100). Known exceptions, each filed: a hidden `data-viewport`'s layer is still composited (#854; paint stopped clearing its box in #1866), a
  ligature across a visibility boundary follows the wrong end (#852). A
  `visibility: visible` descendant is drawn, as in CSS — but desktop hit
  testing still skips the whole hidden subtree, so it is drawn and not
  clickable (**#843**).
  `rinch/src/app/overlay_animation_audit_tests.rs` holds one fixture per overlay
  and fails the moment a transitioned property starts changing on a reveal pass.
  **The close animates too** (#413, #759): `TransitionProperty::Visibility`
  interpolates by css-values-4's rule (every progress strictly between 0 and 1
  is `visible` when an end is — `transition::interpolate_visibility`), and the
  closed state of `Drawer`/`Popover` carries `transition: visibility 0s linear
  <duration>`, so the root stays visible through the slide or fade and hides at
  its end. It is the **one** transitioned property whose animated value reaches
  descendants: rinch's children take their style from Stylo, which knows
  nothing of rinch transitions, so `transition::propagate_inherited_visibility`
  hands the held value down to every descendant whose `visibility` is
  inherited (read from its Stylo rule chain, each rule at its own importance;
  one that declares its own is not reached), after every cascade pass and
  whenever a tick moves the value. The cascade also gives an inheriting node
  that declares its own `visibility` transition (`Checkbox`/`Radio`'s
  `transition: all`) the *animated* value of the ancestor it inherits from as
  its after-change `visibility` (`transition::animated_inherited_visibility`;
  only such nodes pay the rule-chain read — doing it for every descendant cost
  `drawer_toggle` +0.8% for nothing), so it starts nothing on the close pass;
  the hand-down starts that transition when the held value flips, which is
  Chrome's order. And a running `visibility` transition whose current value
  already equals a new after-change value that is not its end is cancelled
  (§3 item 4.1 — `diff_animatable` sees no change there, so a reopen during a
  checkbox's own hide, or of a two-way `transition: visibility` root, used to
  run on to `hidden`). The cancel is made in two places: in the cascade, for a
  node whose own after-change value arrives by restyle, and in the hand-down,
  for a descendant whose inherited value comes back through the walk — a
  checkbox under a two-way root reopened after the root hid: on the reopen pass
  the checkbox inherits the root's animated value at p = 0, which is still
  `hidden`, so its hide is retargeted rather than cancelled, and `visible` only
  reaches it on the next tick, by the hand-down. A
  transition on `color` or `font-size` still stops at its own node. Reopening
  mid-close works because §3 item 3 is implemented now (#693,
  `transition::cancel_unmatched_transitions`): a running transition whose
  property no longer matches `transition-property` is cancelled on the next
  cascade of its node. The #912 pause applies from the close, so a spinner
  slides out frozen — what a browser does with the same CSS.
- **A `@keyframes` animation on a hidden element does not run either**
  (**#747**), and that one is not only a paint question: the desktop frame clock
  schedules another frame whenever `tree.active_animations` holds a running
  (not paused) animation (`app/event_dispatch.rs`), so a `Loader` in a
  `display: none` panel kept an app rendering at full rate with nothing on
  screen moving. css-animations-1 §3 is
  stricter than the transition rule it sits beside — an element that is not
  being rendered has no animation *effect* at all, and is shown again with a
  **new** animation from t=0 rather than the one it had. So the entries are
  dropped rather than parked (parking them would leave the frame clock running,
  which is the whole complaint), and three things happen in the cascade:
  nothing **starts** on a node that is not rendered, everything under a subtree
  that **stops** being rendered is dropped, and everything under one that
  **starts** being rendered again is started afresh. The last is not
  symmetry for its own sake — a node shown by an *ancestor* need not be
  re-cascaded at all, because `set_style` invalidates the one node it was
  written to, so without the walk a panel un-hidden that way would come back
  with its spinner permanently still. None of the three reads
  `transitions_enabled` (**#762**, below): the walk ran behind that flag when
  #747 landed, so a panel shown by an inline write on a cascade before the first
  layout completed kept its spinner still until something re-cascaded it.
- **`visibility: hidden` runs an animation, and that costs a frame clock.** It
  is the same answer the transition rule gives — such a box is rendered — and it
  is what a browser does, measured in Chrome. But an animation has no duration to
  expire, so the cost is not a one-off: after #751 made the closed `Drawer`
  `visibility: hidden`, a `Loader` inside a **closed** drawer kept animating and
  kept the app rendering until #912 paused it (below). Measured before that, on
  the software backend at 804x600, closed
  drawer, 20 idle frames: **20/20 asked for a redraw at 2.53ms per tick+paint**,
  where the `display: none` spelling asked for 0. That is accepted, not
  overlooked — the two rules cannot disagree without desktop diverging from the
  web — and the cure belongs to the component: `animation-play-state: paused`
  on a closed overlay's subtree. **That cure works since #763.** A paused
  animation keeps its `ActiveAnimation` entry (its frozen sample is still
  written into `computed_style` on every cascade), but `tick_animations`
  neither counts it nor marks its node dirty, and the `AboutToWait` guard asks
  `NodeTree::has_running_animations()` rather than whether the map is empty —
  so it schedules no frame, on desktop or through the Android loop. Resuming
  continues from the frozen time. A paused *typography* animation is measured
  **by each cascade that writes its sample, and never per tick** — and that is
  not free, because a cascade of such a node now always re-measures and re-runs
  Taffy, whether or not the sample moved. Measured, release, 500 rows: a
  one-row colour-only hover goes **0.143 → 0.679ms** when that row carries a
  paused `font-size` animation (one extra Taffy compute per hover), and a
  whole-document colour-only restyle **16.1 → 45.1ms** with all 500 rows
  animated. It is bounded by "nodes carrying a text-measure animation", which
  is rare, and `main` paid a compute *every frame* for the same node. The
  narrowing is available and **not done**: compare the node's old
  `computed_style` with the post-animation style instead of asking only whether
  an animation has a `font-size` stop. A finished
  `forwards`/`both` animation is the same shape (**#782**): the tick that
  finishes it writes the fill and dirties the node once
  (`ActiveAnimation::settled`), and after that it is kept, re-applied and
  not counted. That includes a zero-duration **infinite** one
  (`0s infinite forwards`): its active duration is `0 × ∞ = 0`, so it is
  finished at its delay's end, as in Chrome 155 (#786). A finished animation
  **without** a forwards fill (`none`/`backwards`) is kept too (**#783**), as a
  settled marker with no sample, and for a different reason: it is what stops
  the next cascade — an unrelated class, a pause, a theme toggle — from finding
  no entry of that name and starting the still-declared animation again from
  t=0, which Chrome does not do. Its finishing tick cannot restore the cascaded
  values (`computed_style` holds only the last sample), so it pushes the node
  onto `NodeTree::ended_animations` and `RinchDocument::tick_animations` calls
  `mark_restyle` on it — on the **originator** for a generated box, which
  `mark_restyle` refuses and whose cascade styles the box (#1023). That cascade
  drops the marker's properties from its transition diff, so the way back is
  not a transition start (the before-change style is the one updated to the
  current time), and clears the list. `start_animations` keeps `settled` only
  while `has_ended` holds (`is_complete`, but a paused animation is asked at its
  frozen elapsed time, so pausing and resuming a finished one costs no frame),
  so more iterations run it again; a new `animation-name`, taking the animation
  away, or `display: none` and back (#747, which drops the marker), starts a new
  one. **The marker lives in `NodeTree::finished_animations`, not in
  `active_animations`** (review of #2000): a list of rows with a one-shot
  entrance animation keeps one per row for the rows' lives, and in
  `active_animations` every frame of an unrelated spinner cloned all of them
  (2000 rows, debug: 0.003 → ~5 ms per tick). No tick walks the second map;
  `start_animations` looks a name up in both and puts a settled entry without a
  fill back there. Being out of `active_animations` also keeps it out of the
  cascade's `animated_text_measure`, so a finished `font-size` one-shot does not
  make every later restyle of its node re-measure. **Known gap:** the transition
  diff drops the marker's properties whole, so a *real* change of the same
  property in the same frame as the end (a class moving `width` on the
  finishing tick) starts no transition and jumps; the previous cascaded value
  it would start from is not kept anywhere.
  `tests/finished_animation_no_fill_783_tests.rs` is the pin, and
  `perf_counter_baselines::transform_animation_tick_beside_finished_one_shots`
  pins the frame. A caller of `animation::tick_animations` alone (several
  tests do) gets the marker but owns the restyle. **The component library declares the pause itself** (#912):
  `.rinch-drawer__root--hidden *`, a closed `.rinch-popover__dropdown`'s
  subtree and an unhovered `.rinch-hover-card__dropdown`'s subtree all carry
  `animation-play-state: paused !important` — every descendant rather than a
  list of known spinners, and `!important` because the `animation` shorthand
  resets the play state and `.rinch-loader__oval` ties with the `*` rule on
  specificity. The popover and hover-card selectors use a complex `:not()` that
  is the exact complement of the rule making the dropdown visible (Stylo and
  Chrome both match it). A closed drawer holding a `Loader` now asks for no
  frames and resumes where it paused on open — pinned by
  `perf_regression_tests::a_loader_in_a_closed_drawer_idles` and
  `paused_animation_frames_tests::a_loader_in_a_closed_drawer_idles_and_resumes_where_it_paused`.
  A browser stops the spinner during a popover's or hover card's 150ms
  fade-out, since the pause lands when the dropdown starts to close. **No
  `*::before` / `*::after`**, so a pseudo-element spinner under a closed
  overlay still runs — on rinch-web, and since #1004 on desktop too, where a
  generated box now carries its pseudo cascade and its `@keyframes` animation
  starts (it did not before, #925), keeping the frame clock running under a
  closed drawer. The box is kept across a cascade of its originator (#1023), so
  such an animation keeps its clock as in Chrome: it restarts when its
  `animation-name` changes, when the pseudo stops being rendered (`content:
  none`, `display: none`) and comes back, or when its originator leaves the
  document and is inserted again (#699), and
  a `transition` declared on a `::before`/`::after` runs. The pause selectors were left
  out because rinch-dom matches pseudo-element rules with no bloom filter
  (#935), so they cost +10% of style instructions on every page loading the
  component CSS (+71% under a closed drawer); now that they would pause
  something on desktop, that trade is worth revisiting (no component in the
  library's stylesheets declares an animation on a pseudo-element today, by a
  grep of `crates/rinch-components/src/styles`). `rinch-bench`'s `drawer_toggle`
  bench, which loads the theme and component CSS, is there to catch that
  class of cost.
  `LoadingOverlay` is the other `visibility: hidden` overlay and declares no
  pause: it takes no children and its own loader has no animated child (#924);
  fixing that needs the pause too.
- **A move is not a detach.** `append_child`, `insert_before` and `insert_child`
  unlink a node from its old parent with the same lines `remove_child` uses, but
  it is back in the document before the call returns — so a row that was
  mid-transition when a keyed `for` reordered the list goes on transitioning
  (`insert_after` is how a reorder moves rows). **Unless the destination is
  itself detached** (**#702**): a mounted node moved into a parent not connected
  to `tree.root_id` has left the document while keeping a parent, so it fails
  the `parent = None` test — and connectivity, not the parent field, is the
  question (#696). `detach_subtree_styles_if_moved_out` answers it at **four**
  sites: the three move verbs plus `replace_node`, which splices its incoming
  `new` into `old`'s parent. Two guards decide: the destination must differ from
  the old parent (a move within one container cannot change connectivity), and
  the child must already have a parent (a node created moments ago is not a
  move). So a **keyed reorder never walks** — counted on 500 rows, it enters the
  helper 499 times and walks 0 — and a reparenting move walks once each.
  **An `rsx!` component site does walk**, which is the non-obvious part and is
  not free of the reset either: the macro builds a site's children into a
  detached `<template>` and `Component::render` then adopts them into a root
  that is *also* still detached (#719), so each adoption is a move into a
  detached parent. Counted on 500 sites of 20 nodes: 500 entries, 500 walks, 500
  resets over 10,000 nodes. The reset is semantically a no-op there (a fresh node
  is already unstyled with empty transition maps) and the cost does not show
  above noise, but "building a tree pays nothing" is true only of nodes appended
  straight into their final parent.
  One narrow behaviour change comes with it: a **mounted** node round-tripped
  out through a detached parent and back in the same pass loses its running
  transitions and restarts its animations, where a browser — whose style recalc
  is batched to the end of the task — never observes the intermediate state. The
  component *re-render* path does not reach it (`reactive_component_dom` removes
  the old output first, so #699 has already reset it); handing a component a
  handle that is mounted elsewhere and still connected does.

**The reactive helpers reach all of that, and `NodeHandle::clear_animations` is
gone** (**#704**). It used to be called before `remove()` by `show_dom`,
`match_dom`, `for_each_dom_typed`'s `Remove` arm and `reclaim_displaced`, and
the component re-render effect, and it stamped a literal inline
`transition: none; animation: none` over the whole subtree and **never took it
off again** — so a branch hidden once could never transition again, for any
reason, which is what a reactive `if` returning a captured `NodeHandle` (the
#654 shape) does on its first hide. It also masked #699 completely: through one
of those helpers, removing that whole fix changed nothing, which is why its
fixtures drive the `DomDocument` API directly.

Deleting the method was safe because every one of the five call sites was
`clear_animations(); remove();`, and `NodeHandle::remove` is
`DomDocument::remove_node` — the first of the five routes listed above. The cheapest
evidence was already in the tree: `for_each_dom_typed`'s **`Changed` arm** never
called it, and had no defect. On `rinch-web` the browser cancels a removed
element's transitions itself and treats a re-insertion as a first style, so the
inline write there was redundant at best. (A CSS *animation* does restart on
re-insertion in a browser — that is spec behaviour, not something the stamp was
guarding.) **Do not add a `set_style` of any kind to a removal path**: the node
survives the removal, so anything written there is permanent.
`crates/rinch-dom/tests/branch_helper_transition_tests.rs` is the pin: seven
fixtures over four of the five sites, three of them on `show_dom`.
`reclaim_displaced` is the fifth and has none — a
`debug_assert!(clobbered.is_none())` sits one line above the call, so a debug
test panics on the assertion before it can get there.

It was not free either, which is why a deprecated no-op shim would have been the
wrong shape too: `set_style` re-merges the node's whole inline `style` string,
re-parses it into a Stylo declaration block and invalidates the node's inline
style, and `clear_animations` called it **twice per node** over the subtree.
Measured on a 500-row list unmounted one row at a time, best of 200 alternated
rounds in one release binary: 340us without the stamp, 5541us with it —
**16x**, about 3.5us per node against the ~11ns per node #699's reset costs
(`crates/rinch-dom/tests/detach_reset_bench.rs`, both harnesses `#[ignore]`d).

**Nothing transitions on load, and everything animates on load** (**#762**).
`NodeTree::transitions_enabled` is set at the *end* of the first
`resolve_layout`, so a freshly mounted tree cannot transition into existence —
and `recompute_all_styles_full` forces it off again for its own re-cascade, so a
theme change applies instantly instead of every element sliding from the old
palette to the new one. Both are rules about **transitions**. A `@keyframes`
animation has no before-change style to be wrong about: it plays its own
timeline, and a browser runs one on the very first frame the element exists. The
two questions used to be asked with one `if`, and the cost was two bugs — an
animation present in the first frame never started at all (an embedded
`RinchContext` at a fixed size, where no later event re-cascades the tree, kept
a dead spinner for good), and a theme change stopped **every** animation in the
document permanently, so toggling dark mode killed every `Loader`, `Skeleton`
and `Progress` stripe in the app. The animation half of
`apply_stylo_styles_to_taffy` now reads no flag at all, and
`recompute_all_styles_full` no longer clears `tree.active_animations`: the
re-cascade matches each running animation by name and **keeps its clock**
(`start_time_ms`, `paused_elapsed_ms`, `play_state`). Measured in Chrome, replacing
a `<style>` element's text under a running animation leaves `currentTime`
untouched — so preserving is right and restarting would be the smaller wrong
answer. **The clock is all a kept animation keeps, on every cascade of its
node** (#766, #780, #781; until then only `recompute_all_styles_full` refreshed,
behind a `NodeTree::refreshing_animations` flag that is gone). On a class change,
a hover, a `<style>` append (a sheet carrying `@keyframes` takes
`flush_stylist`'s whole-document re-cascade, #884), a `@media` flip or a theme,
`start_animations`
takes the timing (duration, delay, direction, fill, count, easing) from the
spec, looks the `@keyframes` rule up again — **dropping the animation if no rule
of that name exists any more**, as Chrome does — and re-extracts the stops when
`ActiveAnimation::stop_inputs` no longer holds: the rule's declaration blocks
(held as `Arc`s, so a redefined, removed or newly winning rule compares unequal
and a freed address cannot be reused under it), and **only the base-style
inputs the authored stops read** at extraction. The extractor resolves through a
`keyframes::StopBasis` that records each read (`StopDeps`): an `em` reads the
element's `font-size`, a `rem` the root font size, a `currentcolor` the
element's `color`, and `color: currentcolor` the parent's `color`. A
`to { transform: rotate(…) }` spin reads none, so a hover that moves `color`
re-extracts nothing; a `from { background-color: currentcolor }` reads the
colour and is re-extracted by the same hover. (The "root font size" is
`tree.root_id`'s, the document node's, which is always the initial 16px, so a
`rem` stop ignores `html { font-size }` and that input never moves: #2058.)
Otherwise it keeps the stops and re-reads only their **implicit** values
(`keyframes::refresh_implicit_values`): a 0%/100% stop holds the element's base
value for each property the rule animates and that stop does not author (Web
Animations' neutral keyframe) — no other property, so a `to`-only `transform`
spin never writes `color` — and that value follows the base style. Which
properties those are is settled once at extraction (`KeyframeStop::implicit`),
and a cascade reads just those from the base style
(`keyframes::base_style_value`). Until #767 (PR #2050) lands, that function
carries opacity, background colour and position, colour and transform only, so
an implicit stop for any other animated property is left out rather than
filled from the base, and the authored value holds throughout
(`to { width: 110px }` gives 110px at 30% where Chrome gives 40px from a 10px
base). A paused
animation keeps its frozen elapsed *time* across a re-timing, as Chrome keeps
`currentTime`. Repeated names match by position from the end (`k 10s, k 20s`
stays two animations); `ActiveAnimation::position` orders entries split between
the running and finished maps. The cost on a cascade of an animated node is,
per animation, a keyframes lookup, a pointer compare per keyframe plus the
inputs its stops read, and the re-read of its implicit values; an extraction
only when an input it read moved (500 spinning rows under a colour class:
`rinch-bench`'s `animated_list_color`). A node with no animation never
reaches `start_animations`. A viewport change re-cascades the animated node only
if Stylo reports a media-query result changed, which is the case where a
`@media`-held `@keyframes` can have moved. One divergence the theme path reached, now
fixed (**#783**): **a finished one-shot animation replayed on a theme toggle**
— its entry was dropped when it completed, so the full restyle minted it again
from t=0. Chrome does not. The entry is now kept as a settled marker (below).
#747's restart walk — the one that gives a subtree shown by an inline `display`
write its animations back — reads no flag either; it shipped behind this one, so
a panel shown by such a write on a cascade before the first layout completed
kept a still spinner until something unrelated re-cascaded it. The full restyle
reaches the walk as well: the walk runs on a shown panel's cascade before its
descendants' and mints their entries from their pre-restyle styles, so an
`em`-sized spinner under a theme that also changed the font-size starts on the
old basis. Each descendant's own cascade runs after the walk on that pass (and
on a class write that restyles it), and the refresh there re-extracts the stops.
`crates/rinch-dom/tests/animation_start_gating_tests.rs`,
`full_restyle_animation_refresh_tests.rs` and
`animation_restyle_refresh_tests.rs` are the pins, with the Chrome
measurements and mutant-by-fixture tables in their module docs.

**A `transform` transitions and animates by function list, not matrix entry by
entry** (#414). `TransformValue` keeps the list it was composed from
(`functions`; paint, hit testing and layout still read only the composed
`matrix` + percentage coefficients), and `transition::transform::interpolate_lists`
pairs functions by type, pads a shorter list (and `none`) with identity
functions, and decomposes the unmatched remainder the way Chromium does
(Gram–Schmidt shear, shorter-way angle, linear translation). A rotation keeps
its size mid-turn, `rotate(0deg)` → `rotate(360deg)` is a change that spins,
and `none` → `rotate(0deg)` starts nothing. `@keyframes` stops use the same
path; `crates/rinch-dom/tests/transform_interpolation_tests.rs` pins Chrome
153's numbers. Guide: `docs/src/architecture/rendering-pipeline.md`.

**Every easing function eases** (#1564): `steps()`, `step-start`/`step-end`
and `linear(<stops>)` reach `transition::types::TimingFunction` as themselves
(they used to become `linear`), in transitions, `@keyframes` and a keyframe's
own `animation-timing-function`, through one converter. A step reads CSS
Easing 2's before flag (`apply_with_before_flag`: set in the before phase going
forwards and the after phase going backwards; a transition's delay is its easing
at 0 with the flag, not always `from`), and an iteration's start and end
are eased at interval distance 0 and 1 (`ActiveAnimation::interval`), not
snapped to the keyframe. `TimingFunction` is not `Copy` (a `linear()` holds an
`Arc`). The `jump-*` positions do not parse in Stylo's servo build, so such a
declaration is dropped (#1581). Pins: `crates/rinch-dom/tests/easing_steps_linear_1564_tests.rs`
(Chrome 155).

**The 3D transform functions are drawn flat** (#405; they used to compute to the
identity, `translate3d` aside). A list holding any of them is composed as 4×4
matrices (`transition::transform::Mat4`) and **only the result** is flattened —
applied to `z = 0`, divided by `w`, `z` dropped, which is what Chrome draws with
no `perspective`/`preserve-3d` ancestor — so `rotateX(60deg)` halves the height
and `perspective(100px) translateZ(10px)` magnifies by 100/90; a plane at or
behind the viewer (`w <= 0`) flattens to the zero matrix and is neither drawn
nor hit, as in Chrome, and so does a **singular** 4×4 (#1051: `scaleZ(0)`,
`scale3d(1, 1, 0)`, a `matrix3d` with a zero or repeated column), even though
its flattening can be invertible — `scaleZ(0)` flattens to the identity. The
cut is Chrome 153's, measured: a determinant that is not a normal `f32`
(`scaleZ(2.5e-38)` drawn, `scaleZ(1e-38)` not); only
`compose_about_origin_z` applies it, never `compose`, which also feeds
interpolation. A list of planar
functions still composes as 2D affines, unchanged. Every `rotate*` interpolates
as one primitive (common axis → angle lerp; otherwise a quaternion slerp that,
like Chrome's, takes the shorter arc), `perspective(d)` as `1/d`, and
`matrix3d()` pairs with `matrix3d()` only. A mismatched remainder (and a
`matrix3d()` pair) whose 4×4 is not a 2D matrix is decomposed in 3D before
anything is flattened — css-transforms-2's decomposition in Chromium's form, a
2D end still by the 2D one, the quaternion slerped the shorter way — and kept
as a `matrix3d()`, z included (#989, `transform::interpolate_mat4`); such lists
"hold still" only when their 4×4s are equal, so `scaleY(0.5)` →
`rotateX(60deg)` transitions. Not Chrome's: the `perspective` and
`transform-style` properties do nothing; a projective result (`perspective(200px)
rotateY(30deg)`, a trapezoid) loses its `m14`/`m24`. `transform-origin`'s z is
applied before the flattening (`transition::compose_about_origin_z`: the list is
composed as `M · T(0, 0, -z)`, the x and y still at paint), so `rotateY(45deg)`
about `50% 50% 100px` swings 56px left as in Chrome; and `backface-visibility:
hidden` on a box whose **own** transform turns its back to the viewer (Chrome's
`IsBackFaceVisible` on the 4×4 — `scaleX(-1)` is a mirror, not a turn;
`TransformValue::back_facing`) composes it to the zero matrix, so it and its
subtree are neither drawn nor hit — except a `position: fixed` descendant,
which rinch does not contain in a transformed ancestor and still draws
(#1372) (#997).
`crates/rinch-dom/tests/transform_3d_tests.rs` pins Chrome 153.
