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
        let to = cols.iter_mut().find(|c| c.id == m.to_list).unwrap();
        // Insert by key: after `after`, else first.
        let at = m.place.after.as_ref()
            .and_then(|a| to.cards.iter().position(|k| k == a))
            .map_or(0, |i| i + 1);
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
- a press on a card drags the card, never its column.

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
    pub keys: Vec<String>,     // what is carried, in order
    pub from_list: String,     // the id of the list the drag started in
    pub to_list: String,       // the id of the list it lands in
    pub place: SortPlace,
}
pub struct SortPlace {
    pub after: Option<String>,  // the item before the drop; None: first
    pub before: Option<String>, // the item after it; None: last
    pub index: usize,           // the position, at the time of the offer
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

## Judging

`judge` belongs to the list a move would land in. It is called synchronously,
untracked, for every place on offer as it changes — not only at the release —
so the UI shows the verdict before the user lets go:

- a refused place draws the caret in `--rinch-sortable-refused-color`, and the
  reason (`SortVerdict::Refuse(reason)`, when not empty) in a
  `.rinch-sortable__reason` box at the caret's edge of the item — restyle that class;
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
that leaves the pressed item out carries the item alone. Every carried item is
dimmed during the drag.

## Keyboard

A `SortableItem` is focusable. With the item itself focused:

| Key | Does |
|-----|------|
| Space or Enter | Lift the item (and what `drag_keys` carries) |
| Arrow along the list | Move the caret one place; past either end, into the next list of the group that way |
| Arrow across the list | Move to the nearest list of the group that way, at the same position |
| Space or Enter (lifted) | Drop where the caret is (an accepted place calls `ondrop`); the moved item gets the focus back once your re-render has drawn it |
| Escape | Cancel |

Moving the focus away (Tab, a click elsewhere) cancels the lift. Each step is
announced in an `aria-live` region inside the list it started from, with the
list's `label` and the item's `label` (their ids when empty).

## Scrolling

A drag near the edge of a scrolling list scrolls it, on both axes: a column's
card list near its top or bottom, a horizontally scrolling row of columns near
its left or right edge, both at once in a corner. This is the drag
auto-scroll every element drag (`draggable` + `data-ondrag*`) gets.

## Styling

The list is a flex column (`Vertical`) or row (`Horizontal`) with
`--rinch-sortable-gap` between items; override with `class:` / `style:`.
Items are `position: relative` (the caret and the reason are drawn against
them). The classes, all restylable:

| Class | On |
|-------|----|
| `rinch-sortable__item--dragging` | the carried items during a pointer drag |
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
drives it unchanged. The place under the pointer is read from the click
context's pointer and element box; resolving a drag delegated to an outer list
(a column dragged over its cards) and the keyboard's moves between lists use
`NodeHandle::bounding_rect`.

## Not built

- Wrapping lists (a grid of items over several rows): the place is read along
  one axis.
- A placeholder that opens a gap, and an animated reorder: the caret is a line.
