# Components: icons and container defaults

Read this page when your task touches: icons, List/Stepper/RadioGroup defaults, child insertion/removal observers, DropdownMenu/Popover placement, field labels and ids.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

## Icon System

There is **one** icon system: the **`TablerIcon` enum** in `rinch-tabler-icons`, which is also what every component's icon prop takes (`Option<TablerIcon>`). There is no `Icon` enum in the workspace — an older curated `rinch-core` `Icon` enum was removed, so treat any doc comment or snippet still showing `Icon::CheckCircle` as stale.

`TablerIcon` is **not** in the `rinch` prelude — the facade doesn't depend on `rinch-tabler-icons`. Any crate that names an icon variant (including one just passing `icon:` to a component) must depend on it directly.

**Add to Cargo.toml:**
```toml
rinch-tabler-icons = { workspace = true }
```

The crate generates the enum at build time from **vendored JSON committed under `crates/rinch-tabler-icons/data/`**, pinned to `@tabler/icons@3.36.1`, so it builds with no network. It only downloads from unpkg if a vendored file is missing or invalid.

**Usage:**
```rust
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

#[component]
fn my_component() -> NodeHandle {
    rsx! {
        div {
            // Render an icon
            {render_tabler_icon(__scope, TablerIcon::Home, TablerIconStyle::Outline)}

            // Filled variant
            {render_tabler_icon(__scope, TablerIcon::Heart, TablerIconStyle::Filled)}
        }
    }
}
```

The `tabler_icon!` macro is a shorter equivalent when the icon is a literal variant name: `tabler_icon!(__scope, Home)` (Outline is the default style) or `tabler_icon!(__scope, Home, Filled)`.

For size, stroke width, or a CSS class, use `render_tabler_icon_with_options(__scope, icon, TablerIconOptions { style, class, size, stroke_width })` — all fields but `style` are `Option`, so `..Default::default()` covers the rest.

**Features:**
- **4,985 icons** (`ICON_COUNT`; iterate `ALL_ICONS`), every one available in Outline
- **Filled is a subset** — about 1,000 icons have real filled artwork. Asking for `TablerIconStyle::Filled` on any other icon silently falls back to its outline paths, so a glyph that still looks like an outline is expected, not a bug
- **Type-safe** - Use enum variants instead of strings
- **Scales with font size** - Icons carry a `0 0 24 24` viewBox and are sized `1em` unless you pass an explicit `size`, so they track the parent's `font-size`
- **Themeable** - Outline icons use `stroke: currentColor` (default `stroke-width: 2`), filled icons `fill: currentColor`
- **Offline build** - Generated from the vendored JSON in `data/`; no network needed

**Using with ActionIcon:**
```rust
// Pass the rendered icon as a child
ActionIcon {
    variant: "subtle",
    onclick: || do_something(),
    {render_tabler_icon(__scope, TablerIcon::Menu2, TablerIconStyle::Outline)}
}
```

**Sample Categories:**
- Navigation: `Home`, `ArrowLeft`, `ArrowRight`, `ChevronUp`, `ChevronDown`, `Menu2`
- Actions: `Plus`, `Minus`, `X`, `Check`, `Edit`, `Trash`, `Search`, `Settings`
- Status: `AlertCircle`, `AlertTriangle`, `CircleCheck`, `InfoCircle`, `CircleX`
- Communication: `Mail`, `Phone`, `Message`, `Bell`, `Send`, `Share`
- Media: `Photo`, `Video`, `Music`, `Microphone`, `Camera`, `PlayerPlay`
- Files: `File`, `Folder`, `Download`, `Upload`, `Copy`, `ClipboardCopy`

### Components with Icon Support

Every one of these props is `Option<TablerIcon>`. The `rsx!` macro adds the `Some(...)` for you, so write `icon: TablerIcon::Check`, never `icon: Some(TablerIcon::Check)`.

| Component | Icon props | Source |
|--------|-----------|--------|
| `ActionIcon` | `icon` | `action_icon.rs:122` |
| `Alert` | `icon` | `alert.rs:161` |
| `Notification` | `icon` | `notification.rs:113` |
| `AccordionControl` | `icon` | `accordion.rs:251` |
| `Blockquote` | `icon` | `blockquote.rs:25` |
| `List`, `ListItem` | `icon` | `list.rs:109`, `list.rs:233` |
| `Stepper` | `completed_icon`, `progress_icon` | `stepper.rs:143`, `:150` |
| `StepperStep` | `icon`, `completed_icon`, `progress_icon` | `stepper.rs:544`, `:546`, `:548` |
| `NavLink` | `left_section`, `right_section` | `navlink.rs:100`, `:102` |
| `DropdownMenuItem` | `left_section`, `right_section` | `dropdown_menu.rs:483`, `:485` |
| `Tab` | `left_section`, `right_section` | `tabs.rs:397`, `:399` |

The `Tree` component takes its icons through data rather than a prop: `TreeNodeData::icon` (`tree.rs:56`), set with the `with_icon(TablerIcon)` builder.

**`List::icon` and the two `Stepper` icons are container *defaults*, and they
work by patching the rendered children** (#707). A parent component renders
*after* its children — the rsx macro builds the children into a `<template>` and
hands the finished nodes to `Component::render` — so nothing a container knows
can reach an item as a prop. `List` therefore finds each `.rinch-list__item`
that carries no icon of its own and rebuilds it into the layout `ListItem` would
have built; `Stepper` replaces the glyph in each step icon box that its
`StepperStep` marked `data-icon-fallback`. **The item's own icon always wins**,
and `Stepper::progress_icon` stands in for the *`progress_icon`* a step did not
set, which therefore outranks that step's plain `icon` exactly as its own
`progress_icon` would have. All three were declared and read by nothing until
#707; the doc example on `Stepper` (`completed_icon: TablerIcon::CircleCheck`)
is one of the things that now does what it says.

**`Stepper::active` travels the same way** (#709): one pass gives each step the
index of its position and the state that position has against `active` — before
it completed, at it in progress, after it inactive. A step that named a `state`
or a `step` of its own keeps it, and `data-state` is what carries that ask,
because `state: String` renders an explicit `"inactive"` and an unset field into
the same class. A closure prop (`active: {|| sig.get()}`) re-renders the whole
component, children included, so the derivation simply runs again. The delicate
half is the icon: a step draws its icon *before* its parent exists, and the icon
for a state it was moved into is a `TablerIcon` in its props rather than anything
in its DOM. So a step with no `state` of its own renders those icons and leaves
them in its icon box as **hidden alternates**
(`.rinch-stepper__step-icon-alt`, `display: none` inline *and* from the sheet);
the parent promotes the one it needs. Promotion re-parents
**before** clearing the box, and a glyph that is gone for good leaves by
**`discard`** (#719) because `rinch-web` compiles this crate. Those
two facts are one fact: a `discard` retires the whole subtree, so discarding a
wrapper that still held the promoted glyph would retire the glyph with it. The
selection family **acts now** (issue #737): `Stepper::on_step_click` fires with
a step's 0-based position when it is clicked or activated (Enter/Space) on a
step the stepper considers reachable — a strictly **completed** step (`position
< active`) by default (Mantine's `shouldAllowSelect`: `state === 'stepCompleted'
|| allowNextStepsSelect`, and the active step's own state is `'stepProgress'`,
not `'stepCompleted'` — so `active` itself is *not* reachable by default).
The active step and every step past it need `allow_next_steps_select` or the
step's own `allow_step_click`/`allow_step_select`, and `StepperStep::disabled`
always wins over every grant. The wiring is done by the same `settle_steps`
pass that derives state and index, in `settle_step_clickability`: a reachable
step gets `tabindex="0"`, `role="button"` and a `data-rid` — the same shape
`Tree`'s own rows use for generic keyboard activation, no
`register_focus_target` needed — and loses all three, **and the clickable
class**, the moment a sibling insertion or removal shifts it past `active`
with nothing else granting it (a step's own ask is the one thing that keeps
the class regardless of position). The handler is registered once per step
**per stepper render**, lazily (#1428: it closes over that render's
`on_step_click`, so the step's record names who registered each id; a step
moved to another stepper is wired to that one and stops calling the first, and
one moved into a stepper with no callback loses its `data-rid`, `tabindex`
and `role`, and the clickable class unless the step asked for it itself or
that stepper's `allow_next_steps_select` grants it past `active`; an entry
whose handler is gone is pruned at the next registration, and a handler a
late pass registers is owned by the stepper's render, not by the code that
made the insertion, so both go when the stepper unmounts; a wired step moved
out of **every** stepper keeps its wiring, #1453), and reads the step's
*current* position back from the DOM at **click**
time rather than closing over the position it saw when it registered (the
#714 pattern: a late insertion or removal can renumber a step after its
handler is wired, issues #716/#745). With no `on_step_click` set, nothing is
wired and every behaviour `allow_next_steps_select`/`allow_step_click`/
`allow_step_select` had before #737 — the class, and only the class — is
unchanged.

**A container default reaches a child that arrives after the container rendered**
(#716), **and a container that counts positions is re-derived when one leaves**
(#745). The parent patch is still there and still runs first — a parent
component renders *after* its children, so nothing a container knows can travel
as a prop — but the container also registers
`rinch_core::dom::on_child_inserted` against its own root, and the four
`NodeHandle` verbs that put a node into a tree (`append_child`, `insert_before`,
`insert_after`, `replace_with`) tell **every** registered ancestor, nearest
first, synchronously. So a `for` reconcile, a `show_dom` branch and a hand-rolled
`append_child` all land with the default in place before the frame is laid out,
with no deferred queue and no drain site in any host — which is what the
alternative would have needed, since `queue_main_callback` takes a `Send`
closure (a `NodeHandle` is `!Send`) and `rinch-web` drains it nowhere.

`rinch_core::dom::on_child_removed` is the other half, fired by the four verbs
that take a node **out** of a tree (`remove_child`, `remove`, `discard`, and
`replace_with` for the node it displaces) plus the implicit detach an insertion
verb performs when handed a node that already has a parent — which is the only
thing that tells a container a child *moved away*. No batch moves a node:
`UpdateBatch` / `DomUpdate` (both in the prelude) carry **property writes only**
(`SetText`, `SetAttribute`, `RemoveAttribute`, `SetStyle`) since #756, which
removed their `AppendChild` / `InsertBefore` / `RemoveChild` / `ReplaceNode`
variants — `apply` takes a `&mut dyn DomDocument` and could not notify, so a
node moved through a batch reached no observer. `apply` is still the literal
backend call per arm: `SetAttribute` is not `write_attribute` (no boolean
rule), and no pending effect is flushed first. **A write that replaces a child
list fires the removal half too** (#1440): `NodeHandle::set_text` on an
**element** (it orphans every child — the ones the element's own render built
are discarded when its scope goes, #1487 below; told only when the list
changed — an
element whose only child is a text node loses none on any backend (on
`rinch-dom` "only" among its own children: its generated `::before` /
`::after` boxes do not count, and stay, #1630): `rinch-dom`
writes that node in place, where it used to orphan it and mint another per
write, and the mock and a browser list no child for it) and
`NodeHandle::set_inner_html` (it frees them; it also drops the
observers registered on the freed nodes, whose ids `rinch-dom` re-issues to the
markup it parses). Each fires once, with the node whose children went, and
neither fires the insertion half for what arrives. While no removal observer is
registered on the thread `set_text` pays one `Cell` read; with one, a text node
(every reactive `{|| text}`) pays a kind check (`DomDocument::is_text_node`,
default `false`; one JS getter on the web) and no child-list read, an element
one read, two when it had children. `MockDomDocument` orphans / retires the
children as the backends do. Not notified: a batched `DomUpdate::SetText` over
an element's children — `apply` holds the document mutably borrowed, and an
observer edits the tree through `NodeHandle`s that borrow it again — so write
over an element's children with `NodeHandle::set_text`. Pins:
`late_child::tests::*_written_over_an_elements_children_*`,
`rinch-dom/tests/child_list_replaced_1440_tests.rs`,
`rinch-components/tests/stepper_text_over_children_1440.rs`. The two halves are separate
registrations: `Stepper` takes both, `List` and `RadioGroup` only the first,
since neither of their defaults can be changed by a row going away.

**The removal half is handed the node the subtree LEFT — its former parent —
not the node that went**, and that asymmetry is forced rather than chosen. A
removed node is detached, so it has no ancestor chain for an observer's boundary
test to walk, and after a `discard` the backend may have retired it (`remove`
and `discard` are one verb to a `for` reconcile). Every removal verb therefore
reads that parent *before* it mutates the document; a `Cell` counted separately
from the insertion one keeps an app whose containers only care about arrivals
from paying that read. A reorder **inside** one parent fires the insertion half
only: the child set is unchanged, and firing both would make an idempotent
container re-derive twice per moved row.

The pieces that follow from it:
- **The boundary is the observer's own question.** Every registered ancestor is
  told, because two containers of *different* kinds can nest; each declines a
  node whose chain up to it crosses one of its own items, which is its
  render-time walk's rule read upwards. A `List` inside another list's item owns
  its own rows.
- **A callback's own edits do not call it back.** Dispatch is suppressed for the
  duration of a callback; without that, a container that patches the subtree it
  watches recurses without bound (measured: stack overflow).
- **An observer is released by the ambient scope's `on_cleanup`**, the #147
  discipline, and by `NodeHandle::discard` on the container — a discarded id may
  be reissued. The cleanup releases **its own registration**, by a token, not
  whatever stands under the node's id when it runs (#1490): `rinch-dom` hands a
  freed id to the next node it mints, and a cleanup that forgot by
  `(doc, id)` dropped the observers of the container now holding the id (a
  second `on_child_*` call on one node was dropped the same way by the first
  call's scope). `register_focus_target` releases by token too. The editor
  registry (`unregister_editor`, and its blink clock) still releases by
  `(doc_key, container id)` (#1498).
- **A child moved between containers re-resolves**, because a container marks
  what it supplied (`data-list-icon`) and never touches what the child asked for
  itself.
- **`Stepper` re-runs its pass over the whole list, not just the newcomer**: an
  insertion renumbers the steps after it and can restate them. The pass walks
  every step and **derives only the ones the change moved** (#748): a step
  carrying this render's `data-stepper-settled` (a per-render id,
  `Derivation::settler`) and still standing at its `data-step-position` is
  passed over, since everything the pass gives a step follows from the
  stepper's props, the step's position and what the step asked for when it
  rendered. So a step's own asks (`disabled`, `data-state`, `data-step`) are
  read when it is derived, and a hand edit of one on a settled step is not seen
  until the step moves. That pass is idempotent by
  construction — `data-step-derived` says who wrote an index, `data-icon-has`
  records the step's icon *props* rather than what it drew, `data-icon-live`
  names the content key showing — and a glyph the step's props supplied is
  **parked** hidden rather than discarded when it stops being drawn, since a
  later insertion — or a keyed `for` **reorder**, which repositions a live node
  with `insert_before` and can move it *backwards* — can want it back. Every
  alternate is kept while the stepper owns the step's state; one that named its
  own `state` keeps none. A **removal** runs the same pass for the same
  reason, through `on_child_removed` (issue #745): a step that goes moves every
  step behind it backwards, which renumbers it and can restate it.
- **A `StepperCompleted` is not a position, and neither is anything inside it**
  (issue #741). The step walk skips that block: it is what a stepper shows
  *instead of* its steps once they are all done. It used to walk straight into
  it, re-deriving a nested stepper's steps at the outer stepper's indices.
  **A skip, not a stop** — Mantine writes `Stepper.Completed` last by convention
  but does not require it and never stops counting at it, so the count runs
  straight across the block and a step after it is a position like any other.
  Reading it as terminal makes a stepper whose block is written *first* derive
  nothing at all: no step numbered, every step inactive, two steps both drawing
  the number `1`.
- **Cost:** one `Cell` read per insertion (and one per removal) while **nothing
  on the thread** is registered for that half — the counts are thread-local, not
  per document, so one live container anywhere makes every insertion in every
  document on that thread pay an ancestor walk. That walk is **0.12 µs** per insertion at depth 8 (0.16 µs at
  depth 1, 0.56 µs at depth 32 — roughly 0.013 µs per level), measured on 5000
  appends, software build, best of 40. A registered **removal** observer costs
  about the same per removal (+0.13 µs at depth 8, against a 0.54 µs
  `MockDomDocument` detach) and adds **+0.02 µs to every insertion** on the
  thread, for the parent read an insertion verb makes to find out whether it is
  moving a node out of somewhere. `Stepper` is the one container whose own patch
  walks every item per change. A step already settled where it stands costs
  three attribute reads and no write (#748), so what a change costs beyond that
  is the steps it moved: one for an append, none for a removal from the end,
  every step behind it otherwise (three writes each when only renumbered).
  Counted on `MockDomDocument`, appending to a 40-step stepper went from 1005
  attribute reads and 126 attribute writes to 142 and 4, and growing one from 0
  to 40 steps from 2650 writes to 238. Growing or shrinking one step at a time
  is still quadratic in the three reads, and a `for` row edited in place still
  renumbers the steps behind it twice (an insertion, then a removal).
  `crates/rinch-components/tests/stepper_cost_748.rs` pins the per-step
  figures as differences between a 10-step and a 40-step stepper (three reads
  per settled step, three writes per renumbered one, writes that do not grow
  with the size) and the append and growth totals quoted here; the other
  absolute counts are not pinned. `stepper::differential_tests` and
  `stepper::extended_differential_tests` compare the pass with one that derives
  every step, tree for tree, over random histories (insert, remove, move,
  replace; nested steppers, stash and re-insert, moved wrappers and completed
  blocks, re-render with new props).

`RadioGroup::size` and the `Stepper` props are the same shape.

Paths are relative to `crates/rinch-components/src/`. `ActionIcon`'s `icon` prop is a convenience that renders the icon for you as Outline, sized from the component's own `size` prop. It is **mutually exclusive with children** — `loading` wins, then `icon`, and children render only if neither is set — so pass a rendered icon as a child (not via `icon:`) when you need a filled or custom-sized glyph.

## Dropdown placement (#1764)

`DropdownMenu` and `Popover` keep their dropdown inside the viewport through
`rinch-components/src/floating.rs` (`floating::arm`, called from each
component's `render` after the effect that shows the dropdown). The dropdown
stays an `absolute` child of the component root — no portal, so nested menus,
the `--opened` CSS hooks and scrolling with the page are unchanged.

- **Decision:** `floating::decide` (pure, unit-tested): preferred side if the
  dropdown fits inside the viewport there, else the opposite side if it fits,
  else the side with more room keeping `EDGE_MARGIN` (8px) with a cap (top and
  bottom sides only); on the cross axis, a dropdown that runs past an edge is
  clamped to `[8, viewport - 8 - size]`, one that fits is not moved.
- **Application:** inline insets on the dropdown element (the side at `100%`,
  the cross axis in px) plus a pinned `width` and `box-sizing: border-box`
  (an absolute box's shrink-to-fit width depends on its `left`), and
  `max-height`/`overflow-y` for a cap. Each property written is recorded with
  the inline value it replaced (`floating::write`); the next open, and an
  uncapped re-place for the cap, take back only those (`floating::restore`):
  the caller's value comes back, or the property goes, and one the caller
  wrote since is left alone. So the dropdown is measured under the caller's
  own `max-height`/`width`, and a cap is written only below it (review of
  #1812). Writing the insets inline on every open,
  not only when something moved, is what keeps a nested menu's panel immune
  to the outer root's position class: the position rules are descendant
  rules, so an outer menu flipped to `--top` would otherwise pull an inner
  `--bottom` panel up as well. The root's class is swapped to the side used
  (the caller's class or its flipped twin). Never a `transform` on the
  `DropdownMenu` panel: in a browser a transformed panel contains a nested
  menu's `position: fixed` backdrop. `Popover`'s offset moved from
  `translateY`/`translateX` to a margin so a flip does not run its `transform`
  transition.
- **Timing:** on each rising edge of the open state the previous placement is
  cleared and the dropdown measured with `bounding_rect()`. In a browser that
  lays out on demand, so it is placed in the same task. On desktop
  `layout_pending()` is true right after the open's class write; the dropdown
  is then measured from its `bounds_signal()` after the next layout, and
  `RinchApp::settle_bounds_writes` lays the placement out before the frame is
  painted. Both signals (dropdown and root) are owned by that open's effect
  run, so they are watched only while open; the root's re-places the dropdown
  when the root moves, from the size measured at open.
- **Not done:** clipping ancestors as a boundary; a window resize that does not
  move the root; content changes while open (the width is pinned); a scaled
  ancestor (the shift is written in CSS px); `DropdownMenu` `left`/`right`
  positions (no rules); capping a `left`/`right` popover. A host with no
  geometry (`viewport_size()` `None` and no layout pending, the mock document)
  places nothing.

Pins: `rinch/src/app/dropdown_viewport_1764_tests.rs` (desktop, the issue's
measured cases, nested menus), `rinch-web/tests/dropdown_viewport_1764.rs`
(Chrome 155, Firefox 155), `floating::tests`.

## Field labels and ids

**A labelled field component names its field by id** (issue #2034).
`TextInput`, `Textarea`, `NumberInput`, `PasswordInput`, `ColorInput` and
`Select` render their `<label>` *beside* the field, so without `for` the label
labels nothing (it has no labelable descendant) and a press on it focused
nothing on either backend. `src/field_ids.rs` (`FieldIds`) is the one copy of
Mantine's `Input.Wrapper` markup: the field gets the `id` prop, else a
generated `rinch-field-{n}` from one process-wide `AtomicU64` (unique across
instances, component kinds, re-renders and windows); the label
`id="{id}-label"` and `for="{id}"`; the description and error
`{id}-description` / `{id}-error`; the field `aria-describedby` naming the
error then the description, and `aria-invalid="true"` while an error is shown.
A quoted `"id"` root attribute still lands on the wrapper (the macro writes it
after `render`), so the field's id is the bare `id` prop.

`Select`'s trigger is a `div role="combobox"`, not a labelable element, so a
`for` would reach it on neither backend: `wire(.., labelable: false, ..)`
writes no `for`, the trigger names the label in `aria-labelledby`, and the
label carries its own `data-rid` that focuses the trigger (not when disabled;
it does not open the list). The other five rely on the backend's label
activation — the browser's own on the web, #866's on desktop.

So the two answer a label press differently. A press on `ColorInput`'s label
**opens its dropdown**, on both backends: the activation's synthetic click on
the `<input>` bubbles to the input group's toggle `data-rid`. A press on
`Select`'s label only focuses the trigger and leaves the list shut, as native
`<select>` and the ARIA combobox pattern do (Mantine's `Select` is an `<input>`
whose `htmlFor` click reaches its toggle, so it opens there; rinch does not
match Mantine on this).

Pins: `rinch-components/tests/field_label_for_2034.rs` (markup, uniqueness,
app id), `rinch-web/tests/field_label_focus_2034.rs` (Chrome and Firefox),
`rinch/src/app/field_label_2034_tests.rs` (desktop, through #866's label
activation).

`Slider::label` is the value bubble over the thumb, not a field label, and is
left alone; `Checkbox`, `Radio` and `Switch` wrap their `<input>` in the
`<label>`, so they need no `for`.
