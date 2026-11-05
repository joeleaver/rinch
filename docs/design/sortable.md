# Sortable lists (#2091)

Drag items within and between ordered lists — a kanban's cards across
columns, the columns themselves, lanes of columns — with a judge that can
refuse a drop (and say why) before the release, an insertion caret, edge
auto-scroll on both axes, and a keyboard flow. `rinch-components/src/sortable.rs`.

## Requirements from the kanban use

Pimble's kanban viewer is the first user. Its requirements, all general:

1. **Drag source.** The whole item drags by default; an item can instead drag
   by a handle inside it (columns by their header, cards by the whole card).
   A press that never passes the drag threshold stays a click.
2. **Not optimistic.** A drop never reorders the DOM. The app applies the move
   to its own data (a co-edited document) and re-renders from it; a refused
   write leaves nothing to undo. The caret exists only during the drag.
3. **Key-relative place.** First / Last / After(key) (and Before(key)), an
   empty list included.
4. **Judge.** Synchronous, given the item keys, the from list, the to list and
   the place. Its refusal reason shows near the caret during the drag, in an
   app-styleable slot.
5. **Three levels that never fight.** Lanes (a vertical list), columns (a
   horizontal list per lane), cards (a vertical list per column). Each level is
   its own group; a drag only targets lists of its own group.
6. **Multi-item drag.** Several selected cards moved together, in order.
7. **Plugins.** Pimble plugins run plain rinch in a `wasm32-wasip1` guest whose
   DOM ops the host applies and whose `data-*` handler attributes the host
   proxies. So the drag runs on standard element handlers only — no `Drag`
   pointer capture, no window hook — and adds no new handler attribute kind.

## Shape

Three components and a handful of plain types; no controller object.

```rust
SortableList {
    group: "cards", id: column.id.clone(), axis: SortAxis::Vertical,
    judge: Some(judge.clone()),          // Rc<dyn Fn(&SortMove) -> SortVerdict>
    ondrop: move |m: SortMove| apply(m), // accepted drops into this list
    for card in cards_of(&column) {
        SortableItem { key: card.id.clone(), value: card.id.clone(), label: card.title.clone(),
            CardView { .. }
        }
    }
}
```

- `SortableList` — `group`, `id`, `axis` (`Vertical` | `Horizontal`),
  `judge`, `ondrop`, `ondropchange`, `drag_keys`, `label`, `disabled`,
  `lift_keys`, `class`, `style`.
  Its element carries `data-sortable-list` (a per-render token) and the
  `rinch-sortable` classes; an `aria-live` region inside it announces the
  keyboard flow.
- `SortableItem` — `value` (the key), `handle` (drag only by a
  `SortableHandle` inside), `label` (for announcements), `disabled`, `class`,
  `style`. Its
  element carries `data-sortable-key`, `tabindex="0"` and the drag handlers.
- `SortableHandle` — `draggable="true"` and nothing else: the handlers are the
  item's, found by the runtime's walk up from the draggable node.
- `SortMove { group, keys, from_list, to_list, place }`,
  `SortPlace { after: Option<String>, before: Option<String>, index }`
  (`is_first()` = no `after`, `is_last()` = no `before`; both `None` in an
  empty list; `index` is where the first carried item would sit in `to_list`
  once the carried items are taken out), `SortVerdict::{Accept, Refuse(reason)}`.

Every list of one group is one board: lists are registered (thread-local, per
render token, removed with the list's scope) so a handler finds a list's
config from the token on its element.

## Pointer drag

On the DOM drag suite, as `Tree` is (#1589): items are `draggable`, and items
and lists are drop targets (`data-ondrop` / `data-ondragover` /
`data-ondragleave`). That suite already gives everything the issue asks the
pointer side for, on both backends: the 5px threshold (a shorter press stays a
click), touch long-press activation on the web, Escape and `PointerCancel`
cancels, the missed-release heals, drag auto-scroll. `Drag` (pointer capture)
gives none of it and reaches the window, so it is not used.

**Resolving a target.** `data-ondragover` arrives at the nearest drop target,
whatever its group. The handler walks up from it: an item is remembered, a
list of another group forgets it, and the first list of the dragged group is
the target, with the remembered item (if any) the one under the pointer. So a
column dragged over a card resolves to the column the card is in, and a card
dragged over a column's header offers nothing.

**The place.** Over an item: before it when the pointer is in the first half
of it along the list's axis, after it otherwise. Over the list itself (its
padding, an empty list): the nearest gap by the items' midpoints. Places are
taken with the carried items left out, so `after`/`before` never name a carried
item; a place that would leave the list as it is offers nothing.

**Judge and feedback.** The target list's `judge` decides every place on offer,
as it changes. The caret is a class on the item (`--drop-before` /
`--drop-after`; `--drop-into` on an empty list) drawn by its `::after`;
`--refused` turns it red, and the reason is written into a
`rinch-sortable__reason` element at the caret's edge, inside the item (app-styleable; a box hanging outside the item would grow a scrolling list's overflow).
The carried items take `rinch-sortable__item--dragging` (since #2154 only
with `source: SortSource::Dim`; the default is a browser's: a translucent copy
following the pointer, the source unchanged — see the guide's "How a drag
looks"). All of it is taken
down when the drag ends. The source list's `ondropchange` hears every change
of offer (`None` when nothing is on offer).

**Drop.** An accepted offer on release calls the target list's `ondrop` with
the `SortMove`. Nothing else happens to the DOM.

## Keyboard

Element `onkeydown` on the item (`data-onkeydown`, already in the closed list),
acting only when the item itself is focused:

- Space / Enter lifts the item (and what `drag_keys` adds).
- Arrows move the caret. Along the list's axis it steps through the list's
  places; past either end, and across the axis, it moves to the nearest list of
  the same group in that direction (by on-screen boxes), at the same index or
  the near end. The caret's item is scrolled into view.
- Space / Enter drops (an accepted place calls `ondrop`; the item is focused
  again once the app re-rendered it), Escape cancels.

Escape is answered by the item's own handler rather than by a dismiss-stack
entry: element handlers run before the dismiss stack, so it still beats an
enclosing `Modal`, and it works where only `data-*` handlers are proxied
(requirement 7). Focus leaving the lifted item cancels the lift
(`push_key_handler`'s `on_focus_leave`; desktop and rinch-web, not a plugin
guest). Each step is announced in the list's `aria-live` region.

## Auto-scroll on both axes

The DOM drag suite's auto-scroll was vertical only. It now scrolls each axis
on its own: the nearest container overflowing vertically with the pointer in a
top/bottom band, and the nearest overflowing horizontally with the pointer in a
left/right band (same curve, `drag_auto_scroll_velocity`). Desktop
(`tick_drag_auto_scroll`, with the wheel's horizontal write shared as
`scroll_horizontally_by`) and rinch-web (`AutoScrollLoop`, a remainder and a
`stuck` mark per axis).

## After review (#2093)

A co-edited board re-renders whole lanes under a drag, so nothing may hang on
a node or a registration token surviving:

- the drag ends with the runtime's element drag
  (`rinch_core::on_element_drag_end`, called by both backends), not with the
  dragged node's `data-ondragend`;
- offers and lifts name their list by group and id and look it up again;
- a re-rendered carried item is dimmed again, and a re-rendered lifted item
  takes over the lift and the focus;
- the drop settles the place again by its `after` key and judges it again;
- a list inside a carried item of the same group is never a target;
- without `bounding_rect`, over an item the drag passes it in the direction
  it is moving, and lists follow document order;
- a key counts only when sent to the item itself (`KeyEventData::target`);
- the dropped item is focused at its render in its new list (no timer needed);
- styling is inherited custom properties (a guest's only lever), and the
  parts stay quiet without the stylesheet;
- `data-drag-ghost="false"` lets a painting host hide its ghost (on the list
  since #2154).

## Not built

- Wrapping lists (a grid of items in rows): the place is taken along one axis.
- A placeholder that opens a gap (the caret is a line), and an animated
  reorder.
- Migrating `Tree` onto this. Its drop rules (inside a row, depth by the
  pointer's column) are not a list's; it could share the target walk and the
  caret drawing later.
