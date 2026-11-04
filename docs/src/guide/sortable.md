# Sortable Lists

`SortableList` and `SortableItem` let the user drag items within a list and
between lists — a kanban's cards across columns, the columns themselves, lanes
of columns, a playlist, a tab strip. While a drag is over a place the list
draws an insertion caret, asks your `judge` whether the move may happen (and
shows its reason when it may not), and on an accepted drop calls your `ondrop`.

**The component never moves anything.** `ondrop` hands you a `SortMove`; you
apply it to your own data, and the lists re-render from it. If your write is
refused later (a co-edited document, a server), there is nothing to undo: the
caret existed only during the drag.

## A board

```rust
use rinch::prelude::*;
use std::rc::Rc;

#[derive(Clone, PartialEq)]
struct Column { id: String, cards: Vec<String> }

/// Refuse a third card into "doing".
fn limit(columns: Signal<Vec<Column>>) -> SortJudge {
    Rc::new(move |m: &SortMove| {
        let doing = columns.get().into_iter().find(|c| c.id == "doing").unwrap();
        let staying = doing.cards.iter().filter(|k| !m.keys.contains(k)).count();
        if m.to_list == "doing" && staying + m.keys.len() > 2 {
            SortVerdict::Refuse("Doing takes at most 2".into())
        } else {
            SortVerdict::Accept
        }
    })
}

/// Apply a move to the app's own data; the lists re-render from it.
fn apply(columns: Signal<Vec<Column>>, m: SortMove) {
    columns.update(|cols| {
        for c in cols.iter_mut() {
            c.cards.retain(|k| !m.keys.contains(k));
        }
        let Some(to) = cols.iter_mut().find(|c| c.id == m.to_list) else { return };
        // Insert by key: right after `after`; right before `before`; `index`
        // only when neither names an item the list still has.
        let at = match (&m.place.after, &m.place.before) {
            (None, _) => 0,
            (Some(a), b) => to.cards.iter().position(|k| k == a).map(|i| i + 1)
                .or_else(|| b.as_ref().and_then(|b| to.cards.iter().position(|k| k == b)))
                .unwrap_or(m.place.index.min(to.cards.len())),
        };
        for (i, k) in m.keys.iter().enumerate() {
            to.cards.insert(at + i, k.clone());
        }
    });
}

#[component]
fn board() -> NodeHandle {
    let columns = Signal::new(vec![
        Column { id: "todo".into(), cards: vec!["Sketch".into(), "Copy".into()] },
        Column { id: "doing".into(), cards: vec!["API".into()] },
        Column { id: "done".into(), cards: vec![] },
    ]);
    rsx! {
        div { style: "display: flex; gap: 12px",
            for col in columns.get() {
                SortableList {
                    key: col.id.clone(),
                    group: "cards", id: col.id.clone(), label: col.id.clone(),
                    style: "width: 200px; min-height: 40px",
                    judge: Some(limit(columns)),
                    ondrop: move |m: SortMove| apply(columns, m),
                    for card in col.cards.clone() {
                        SortableItem { key: card.clone(), value: card.clone(), {card.clone()} }
                    }
                }
            }
        }
    }
}
```

`value` is the item's key, the one the move names. Inside a `for`, `key:` is
the loop's reconciliation key; give both.

## Groups, lists and keys

Every list has a **group** and an **id**; every item a **key** (`value`). A
drag only ever targets lists of its own group. That is what lets sortables
nest without fighting: put a `SortableList { group: "cards" }` inside each
`SortableItem` of a `SortableList { group: "columns", axis: SortAxis::Horizontal }`,
and those inside the items of a `SortableList { group: "lanes" }`:

- a card dragged over a column's header offers nothing (the header is not in a
  list of cards);
- a column dragged over cards is placed among the columns, before or after the
  column the cards are in;
- a press on a card drags the card, never its column;
- in an outline, where an item holds a list of its own group (a list of
  sections whose sections hold sections), a drag never lands in a list inside
  an item it carries — not by pointer, not by keyboard.

Within a group, list `id`s are unique in a document, and so are item keys.

Give an item `handle: true` to drag it only by a `SortableHandle` inside it —
a column by its header, a lane by a grip:

```rust
SortableItem { key: col.id.clone(), value: col.id.clone(), handle: true,
    SortableHandle { {col.title.clone()} }
    SortableList { group: "cards", id: col.id.clone(), /* … */ }
}
```

A press that never passes the drag threshold (5 px) stays a click, so an
`onclick` on a card still opens it.

## The move

```rust
pub struct SortMove {
    pub group: String,
    pub keys: Vec<String>,     // what is carried, in order (as drag_keys gave them)
    pub from_list: String,     // the id of the pressed item's list
    pub to_list: String,       // the id of the list it lands in
    pub place: SortPlace,
}
pub struct SortPlace {
    pub after: Option<String>,  // the item before the drop; None: first
    pub before: Option<String>, // the item after it; None: last
    pub index: usize,           // the position, as the list is at the drop
}
```

The place names the **neighbours by key**, with the carried items left out:
`after: None` is the start of the list, `before: None` its end, and both
`None` an empty list (`is_first()`, `is_last()`). An index alone would go
stale when something else reorders the list between the offer and your write
(a collaborator's edit); a key does not. Insert after `after` (or before
`before`), and use `index` only as a fallback.

A place that would leave the list as it is (an item over its own position) is
not offered, and nothing is delivered for it.

**At the drop the place is settled again.** The offer was worked out at the
last pointer move or key press; the lists may have been re-rendered since (a
collaborator's edit). So at the release (or the keyboard drop) the target list
is found again by its id, the place again by its `after` key (first when it had
none) — `before` and `index` are read off the list as it is then — and the
`judge` is asked again. A place whose `after` item has left the list, one that
became a no-op, or one now refused is not delivered. Your `ondrop` still runs
later than that check, against your data rather than the DOM: **judge the move
again there** before you write it.

## Judging

`judge` belongs to the list a move would land in. It is called synchronously,
untracked, for every place on offer as it changes, and again at the drop, so
the UI shows the verdict before the user lets go:

- a refused place draws the caret in `--rinch-sortable-refused-color`, and the
  reason (`SortVerdict::Refuse(reason)`, when not empty) in a
  `.rinch-sortable__reason` box at the caret's edge, inside the item (see
  [Styling](#styling));
- the source list's `ondropchange` hears `Some((move, verdict))` each time the
  offer changes and `None` when nothing is on offer, for an app that describes
  the drop elsewhere;
- a refused place is never delivered.

## Several items at once

`drag_keys` on a list says what a drag from one of its items carries:

```rust
drag_keys: Some(Rc::new(move |key: &str| {
    let sel = selection.get();
    if sel.iter().any(|k| k == key) { sel } else { Vec::new() }
}) as SortKeys),
```

The keys arrive in `SortMove::keys` in the order you returned them; an answer
that leaves the pressed item out carries the item alone. Every carried item
takes the `source` look during the drag (dimmed with `SortSource::Dim`), a
re-rendered one included. **The keys are not
checked**: one that names no item is passed on as it is, and when the
selection spans several lists `from_list` still names only the pressed item's
list — your `ondrop` takes each key out of wherever it is.

## Keyboard

A `SortableItem` is focusable. With the item itself focused:

| Key | Does |
|-----|------|
| A lift key: Space or Enter | Lift the item (and what `drag_keys` carries) |
| Arrow along the list | Move the caret one place; past either end, into the next list of the group that way |
| Arrow across the list | Move to the nearest list of the group that way, at the same position |
| A lift key (lifted) | Drop where the caret is, settled and judged again; the moved item gets the focus back once your re-render draws it in its new list, now or later |
| Escape | Cancel |

`lift_keys` on a list chooses the keys (`KeyEventData::key` values): an app
that opens an item on Enter writes `lift_keys: vec![" ".into()]`. Only a key
sent to the item itself counts; one typed in a field inside it is the field's.

A lift goes on through re-renders: of the list it is over (found again by id)
and of the lifted item itself (the new node takes the ring and the focus).
Moving the focus to another element (Tab, a click elsewhere) cancels it. Each
step is announced in the `aria-live` region of the list it started from, as
drawn now, with the list's `label` and the item's `label` (their ids when
empty); the same sentence twice in a row is still announced. Items carry
`aria-roledescription="sortable item"` and an `aria-description` with the
keys.

## Read-only

`disabled: true` on a `SortableList` makes it read-only: no drag starts from
its items, none lands in it, no lift. Give the items `disabled: true` too and
they lose their drag source altogether (a press is only ever a press). A
disabled item in a live list keeps its place and takes drops around it.

## Scrolling

A drag near the edge of a scrolling list scrolls it, on both axes: a column's
card list near its top or bottom, a horizontally scrolling row of columns near
its left or right edge, both at once in a corner. This is the drag
auto-scroll every element drag (`draggable` + `data-ondrag*`) gets.

## How a drag looks

> **Changed in #2154.** With no presentation props, a drag now shows a
> translucent copy of the item following the pointer and leaves the item as it
> is, as a browser's native drag does. Before, nothing followed the pointer and
> the item was dimmed. For the old look:
> `preview: SortPreview::None, source: SortSource::Dim`.

Every part of a drag's presentation is an option on `SortableList`, each on
its own. The defaults do what a browser's native drag does where it has one,
and draw a caret where it has none:

| Prop | Decided by | Values (default first) |
|------|-----------|------------------------|
| `preview: SortPreview` | the list the drag starts in | `Clone` — a translucent copy of the item that follows the pointer, keeping the point it was grabbed at under it (as a browser's drag image); `None`; `Snapshot` — the runtime's built-in picture (desktop only: the web runtime has none, so nothing shows there); `Custom(f)` — the app's element |
| `indicator: SortIndicator` | the list the place is in | `Caret`; `Gap` — the items from the place on slide apart to open a gap the size of what is carried; `Both`; `None` |
| `source: SortSource` | the list the drag starts in | `Unchanged` (as a browser leaves a drag's source); `Dim` (`--rinch-sortable-dragging-opacity`); `Collapse` — hidden, and the items after it slide up over its space; `Hide` — hidden, its space kept |

```rust
SortableList {
    group: "cards", id: col.id.clone(),
    indicator: SortIndicator::Gap,
    source: SortSource::Collapse,
    preview: SortPreview::Custom(Rc::new(|__scope: &mut RenderScope, key: &str| {
        rsx! { div { class: "card-preview", {key.to_string()} } }
    })),
    …
}
```

- **A refused place never opens a gap.** It shows the refused caret and the
  reason in every mode but `SortIndicator::None`, which still shows the reason.
- **The preview** (`Clone`, `Custom`) is styled **as if it were still where the
  item is**: set a `--rinch-sortable-ghost-*` property (or `color`, `font`, a
  class whose rule sets them) on the list, on any ancestor of it, or on the
  item itself, and that list's — or that item's — preview takes it, by the
  ordinary cascade; a descendant selector such as `.my-board .card` matches the
  copy too. Its one box, `.rinch-sortable__ghost-body` (`--clone` / `--custom`),
  is `position: fixed` and `pointer-events: none`, so it is never what a drag
  is over; select it as `.my-list .rinch-sortable__ghost-body` to restyle one
  list's preview. Inside a scaled ancestor the preview is the size the item is
  painted at. A `Custom` function gets a scope and the dragged item's key;
  what it makes (signals, effects) is owned by that scope, which is disposed
  when the drag ends. A keyboard lift has no pointer and shows no preview.

  How: the preview is mounted on the document's `<body>` (rinch-web: the
  mount's root), inside `display: contents` stand-ins for every element between
  there and the item — their tags, classes and attributes, inline style
  included — so it inherits and matches selectors as the item does, while no
  ancestor's own box (its size, padding, `overflow`, `transform`) reaches it.
  An ancestor's own opacity, filter or transform is not applied to the
  preview (its stand-in draws no box).

  **What does not carry over:** `id`s are not copied, so an id-scoped selector
  (`#board .card`) does not match the preview; and each stand-in is an only
  child, so structural selectors (`:nth-child`, `:first-child`, `+`, `~`) see
  it as one. Style the preview with classes, or with the list's
  `--rinch-sortable-ghost-*` properties. A copy also leaves out everything the
  runtime acts on — handlers, `id`, `tabindex`, `draggable`, `contenteditable`,
  `name`, `checked`, `form`, and `data-` attributes such as
  `data-trap-focus`, `data-viewport`, `data-render-surface` and the editor's
  `data-pm-*` — while other `data-` attributes stay, as style hooks. A text
  field shows what it holds now (a `<select>` shows its default option).
- **The gap** and a collapse are a `transform` on the items (the component
  writes `--rinch-sortable-shift-x` / `-y`, the stylesheet turns them into a
  `translate`), so layout does not move. The place under the pointer is read
  from the boxes as painted, gap included; a shift always moves the next
  threshold away from the pointer, so a slow sweep changes the place once
  each way. Over its own place a collapsed item's list opens the gap there
  again (with a gap indicator).
- **In the source's own list**, unless it collapses, a gap is the list laid out
  again in the order the drop would give: the items between the place and the
  source make room and the source itself is drawn at the place, so nothing is
  painted over it. Over the moved source the place stays.
- **A keyboard lift** rings the lifted items (whatever `source` says) and moves
  the gap with the arrows like a pointer does.
- **An app that draws everything itself** sets `preview: SortPreview::None`,
  `indicator: SortIndicator::None`, `source: SortSource::Unchanged` and reads
  the drag reactively: `sortable_drag()` gives a `SortDragState` (group, keys,
  `source_key`, `from_list`, `keyboard`, `over` — the place under the pointer,
  its own place included —, `target` — the place a drop would land —, and its
  `verdict` with a refusal's reason), and `sortable_drag_pointer()` a
  `SortPointer` (the pointer, the grab point in the item, the item's size),
  its own signal because it changes on every move.

## Styling

The list is a flex column (`Vertical`) or row (`Horizontal`) with
`--rinch-sortable-gap` between items; override with `class:` / `style:`.
Items are `position: relative` (the caret and the reason are drawn against
them).

The look is custom properties read with fallbacks and never defined by the
stylesheet, so they inherit: set them inline on a `SortableList` (or on
anything around it). That is the one lever a sandboxed plugin, which cannot
add a stylesheet, has.

| Property | Default |
|----------|---------|
| `--rinch-sortable-gap` | `--rinch-spacing-xs` |
| `--rinch-sortable-caret-color` | `--rinch-primary-color` |
| `--rinch-sortable-caret-size` | `2px` (the caret line, and an empty list's outline) |
| `--rinch-sortable-refused-color` | `--rinch-color-red-6` |
| `--rinch-sortable-dragging-opacity` | `0.4` (`SortSource::Dim`) |
| `--rinch-sortable-reflow-duration` | `150ms` (the gap's and a collapse's slide; `0ms` turns it off) |
| `--rinch-sortable-reflow-easing` | `ease` |
| `--rinch-sortable-ghost-opacity` | `0.75` (each `-ghost-` one is read where the item is: on the list, an ancestor or the item) |
| `--rinch-sortable-ghost-tilt` | `0deg` (a `rotate`) |
| `--rinch-sortable-ghost-scale` | `1` |
| `--rinch-sortable-ghost-shadow` | `none` (on a copy) |
| `--rinch-sortable-ghost-duration` | `0ms` (the preview follows the pointer exactly; more makes it trail) |
| `--rinch-sortable-ghost-easing` | `ease` |
| `--rinch-sortable-ghost-z-index` | `1000` |
| `--rinch-sortable-reason-bg` | the refused colour |
| `--rinch-sortable-reason-color` | white |
| `--rinch-sortable-reason-font-size` | `--rinch-font-size-xs` |
| `--rinch-sortable-reason-radius` | `--rinch-radius-sm` |
| `--rinch-sortable-reason-inset` | `4px` (from the caret's edge and the item's right) |
| `--rinch-sortable-reason-padding` | `2px 6px` |
| `--rinch-sortable-reason-white-space` | `nowrap` |
| `--rinch-sortable-reason-max-width` | the item's width less 8px |

Without the component stylesheet (a host pinned to an older rinch) the parts
stay quiet: the live region and the reason slots carry inline styles that keep
them out of the flow and out of sight, and the caret is not drawn.

The classes, all restylable (the reason's `display` is set inline, to show and
hide it):

| Class | On |
|-------|----|
| `rinch-sortable__item--dragging` | the carried items during a pointer drag, with `source: SortSource::Dim` |
| `rinch-sortable__item--collapsed` / `--drag-hidden` | the same, with `Collapse` / `Hide` (`visibility: hidden`) |
| `rinch-sortable__item--shifted` | an item the gap or a collapse moved (until the drag ends) |
| `rinch-sortable--reflowing` | a list items slide in, while the drag lasts (it carries the `transition`) |
| `rinch-sortable__ghost` | the preview's wrapper on `<body>` (`display: contents`) |
| `rinch-sortable__ghost-body` (`--clone`, `--custom`) | the preview's one box |
| `rinch-sortable__item--lifted` | the carried items during a keyboard lift |
| `rinch-sortable__item--drop-before` / `--drop-after` | the item the caret is drawn at (`--drop-horizontal` in a horizontal list) |
| `rinch-sortable__item--refused`, `rinch-sortable--refused` | a refused place |
| `rinch-sortable--drop-into` | an empty list on offer |
| `rinch-sortable__reason--shown` (`--above`) | the refusal reason |

## Backends and plugins

Desktop and rinch-web behave the same. A touch drag on the web starts with a
long press (350 ms), as every element drag does there.

Everything runs from element handler attributes — `data-ondragstart`,
`data-ondragover`, `data-ondragleave`, `data-ondrop`, `data-ondragmove`,
`data-ondragend` and `data-onkeydown` — with no pointer capture and no
window-level hook, so a host that proxies those attributes (a plugin sandbox)
drives it. What such a host must also give the guest:

- **The click context** for `data-ondragover`: the pointer and the box of the
  element carrying the handler, as the runtime sets it.
- **`NodeHandle::bounding_rect`**, for a drag delegated to an outer list (a
  column dragged over its own cards), a pointer over a list's padding, and the
  keyboard's moves between lists. It is the element's **painted box in the
  viewport's frame** — the frame of the click context's pointer — not a
  parent-relative layout box. Without it: over an item the drag is moving
  past, after it, else before it; over padding, the end; between lists,
  document order.
- **`KeyEventData::target`** (the element a key was sent to), or the
  document's active element, so a key typed in a field inside an item is not
  taken for the item's.
- **Timers**, to give the focus back after a keyboard drop when the item is
  already drawn; without them it is focused only at its next render.
- **The drag ghost.** Lists carry `data-drag-ghost="false"` (all but a
  `preview: SortPreview::Snapshot` one): the component draws the drag itself,
  and a host that paints a ghost should read the attribute on the source or
  an ancestor (the desktop runtime does) — or forward the guest's
  `suppress_drag_ghost()`, which otherwise reaches nothing that paints.
- **The press point** in `data-ondragstart`'s click context (`mouse_x/y`; both
  runtimes set it, #2154), for the preview's grab offset; without it the
  preview is held by the item's middle.
- **`DomDocument::attribute_list`, `node_type` and `text_content`** for a
  `SortPreview::Clone` copy; a host without them copies nothing (the preview
  is an empty box).
- **The end of a drag.** Each backend tells `rinch_core::on_element_drag_end`
  listeners when an element drag ends, which is what ends a drag whose dragged
  item was re-rendered away (its own `data-ondragend` goes to the gone node).
  A host should call `notify_element_drag_ended()` in the guest when its drag
  ends.

Not in a guest: the focus-leave cancel of a keyboard lift (it rides the
host-side dismiss stack's focus notifications); Escape, Tab and the arrows all
work.

## Not built

- Wrapping lists (a grid of items over several rows): the place is read along
  one axis.
- A placeholder that opens a gap, and an animated reorder: the caret is a line.
