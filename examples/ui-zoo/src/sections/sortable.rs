//! Sortable section — a small kanban on `SortableList` (#2091): lanes of
//! columns of cards, every level draggable, a WIP limit judged live.

use std::rc::Rc;

use rinch::prelude::*;

#[derive(Clone, Debug, PartialEq)]
struct Column {
    id: String,
    title: String,
    /// Cards a column takes before it refuses more (0: no limit).
    limit: usize,
    cards: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Lane {
    id: String,
    title: String,
    columns: Vec<Column>,
}

fn column(id: &str, title: &str, limit: usize, cards: &[&str]) -> Column {
    Column {
        id: id.into(),
        title: title.into(),
        limit,
        cards: cards.iter().map(|c| c.to_string()).collect(),
    }
}

fn initial_board() -> Vec<Lane> {
    vec![
        Lane {
            id: "lane-product".into(),
            title: "Product".into(),
            columns: vec![
                column(
                    "todo",
                    "To do",
                    0,
                    &["Sketch the board", "Pick colours", "Write copy"],
                ),
                column(
                    "doing",
                    "Doing (max 2)",
                    2,
                    &["Wire the API", "Fix the build"],
                ),
                column("done", "Done", 0, &["Ship the beta"]),
                column("later", "Later", 0, &[]),
            ],
        },
        Lane {
            id: "lane-ops".into(),
            title: "Ops".into(),
            columns: vec![
                column("ops-todo", "To do", 0, &["Rotate keys"]),
                column("ops-done", "Done", 0, &[]),
            ],
        },
    ]
}

/// Put `moved` at `place` in `list`, by its keys: after `after`, else first.
fn insert_at<T>(list: &mut Vec<T>, moved: Vec<T>, place: &SortPlace, id: impl Fn(&T) -> &str) {
    let at = place
        .after
        .as_ref()
        .and_then(|a| list.iter().position(|t| id(t) == a))
        .map_or(0, |i| i + 1);
    for (i, t) in moved.into_iter().enumerate() {
        list.insert(at + i, t);
    }
}

/// The app applies a move to its own data; the lists re-render from it.
fn apply(board: &mut Vec<Lane>, m: &SortMove) {
    match m.group.as_str() {
        "cards" => {
            for c in board.iter_mut().flat_map(|l| l.columns.iter_mut()) {
                c.cards.retain(|k| !m.keys.contains(k));
            }
            if let Some(c) = board
                .iter_mut()
                .flat_map(|l| l.columns.iter_mut())
                .find(|c| c.id == m.to_list)
            {
                insert_at(&mut c.cards, m.keys.clone(), &m.place, |k| k);
            }
        }
        "columns" => {
            let mut moved = Vec::new();
            for key in &m.keys {
                for lane in board.iter_mut() {
                    if let Some(i) = lane.columns.iter().position(|c| &c.id == key) {
                        moved.push(lane.columns.remove(i));
                    }
                }
            }
            if let Some(lane) = board.iter_mut().find(|l| l.id == m.to_list) {
                insert_at(&mut lane.columns, moved, &m.place, |c| &c.id);
            }
        }
        "lanes" => {
            let mut moved = Vec::new();
            for key in &m.keys {
                if let Some(i) = board.iter().position(|l| &l.id == key) {
                    moved.push(board.remove(i));
                }
            }
            insert_at(board, moved, &m.place, |l| &l.id);
        }
        _ => {}
    }
}

fn describe(m: &SortMove) -> String {
    let place = match (&m.place.after, &m.place.before) {
        (None, None) => "into the empty list".to_string(),
        (None, Some(b)) => format!("first, before {b}"),
        (Some(a), None) => format!("last, after {a}"),
        (Some(a), _) => format!("after {a}"),
    };
    format!("{} → {} ({place})", m.keys.join(", "), m.to_list)
}

/// The WIP limit, judged live: the refusal's reason shows by the caret.
fn wip_judge(board: Signal<Vec<Lane>>) -> SortJudge {
    Rc::new(move |m: &SortMove| {
        let Some(col) = board
            .get()
            .into_iter()
            .flat_map(|l| l.columns)
            .find(|c| c.id == m.to_list)
        else {
            return SortVerdict::Accept;
        };
        let staying = col.cards.iter().filter(|k| !m.keys.contains(k)).count();
        if col.limit > 0 && staying + m.keys.len() > col.limit {
            SortVerdict::Refuse(format!("{} takes at most {}", col.title, col.limit))
        } else {
            SortVerdict::Accept
        }
    })
}

/// A drag from a selected card carries the whole selection, in board order.
fn selection_keys(board: Signal<Vec<Lane>>, selection: Signal<Vec<String>>) -> SortKeys {
    Rc::new(move |key: &str| {
        let sel = selection.get();
        if !sel.iter().any(|k| k == key) {
            return Vec::new();
        }
        board
            .get()
            .into_iter()
            .flat_map(|l| l.columns)
            .flat_map(|c| c.cards)
            .filter(|k| sel.contains(k))
            .collect()
    })
}

fn dropped(board: Signal<Vec<Lane>>, status: Signal<String>, m: SortMove) {
    status.set(format!("Moved {}", describe(&m)));
    board.update(|b| apply(b, &m));
}

fn offered(status: Signal<String>, offer: Option<(SortMove, SortVerdict)>) {
    match offer {
        Some((m, SortVerdict::Accept)) => status.set(format!("Drop: {}", describe(&m))),
        Some((m, SortVerdict::Refuse(why))) => {
            status.set(format!("Refused: {} — {why}", describe(&m)))
        }
        None => {}
    }
}

#[component]
pub fn sortable_section() -> NodeHandle {
    let board = Signal::new(initial_board());
    let selection: Signal<Vec<String>> = Signal::new(Vec::new());
    let status = Signal::new(String::from(
        "Drag a card, a column header or a lane handle.",
    ));
    let opened = Signal::new(String::new());

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Sortable" }
                Text { size: "lg", color: "dimmed",
                    "Lists whose items drag within and between them. Lanes, columns and cards are three groups that never fight; the app applies every drop to its own data."
                }
                Text { size: "sm", color: "dimmed",
                    "Click a card to open it, Ctrl-click to select several and drag them together. Focus an item and press Space to lift it, arrows to move it, Space to drop, Escape to cancel."
                }
            }
            Space { h: "md" }
            Text { size: "sm", class: "sortable-demo-status", {|| status.get()} }
            Text { size: "sm", color: "dimmed", class: "sortable-demo-opened",
                {|| if opened.get().is_empty() { String::new() } else { format!("Opened: {}", opened.get()) }}
            }
            Space { h: "sm" }

            SortableList {
                group: "lanes", id: "board", label: "Board", class: "sortable-demo-lanes",
                style: "--rinch-sortable-gap: 12px",
                ondrop: move |m: SortMove| dropped(board, status, m),
                ondropchange: move |o: Option<(SortMove, SortVerdict)>| offered(status, o),
                for lane in board.get() {
                    SortableItem {
                        key: lane.id.clone(), value: lane.id.clone(), handle: true, label: lane.title.clone(),
                        style: "border: 1px solid var(--rinch-color-border); border-radius: 8px; padding: 8px",
                        SortableHandle {
                            style: "font-weight: 600; padding: 4px 0 8px",
                            {format!("⠿ {}", lane.title)}
                        }
                        SortableList {
                            group: "columns", id: lane.id.clone(), axis: SortAxis::Horizontal, label: lane.title.clone(),
                            class: "sortable-demo-columns",
                            style: "--rinch-sortable-gap: 8px; overflow-x: auto; padding-bottom: 4px",
                            ondrop: move |m: SortMove| dropped(board, status, m),
                ondropchange: move |o: Option<(SortMove, SortVerdict)>| offered(status, o),
                            for col in lane.columns.clone() {
                                SortableItem {
                                    key: col.id.clone(), value: col.id.clone(), handle: true, label: col.title.clone(),
                                    style: "flex: 0 0 200px; display: flex; flex-direction: column; background: var(--rinch-color-surface-alt, rgba(127,127,127,0.08)); border-radius: 6px; padding: 6px",
                                    SortableHandle {
                                        style: "font-size: 13px; font-weight: 600; padding: 2px 4px 6px",
                                        {format!("{} · {}", col.title, col.cards.len())}
                                    }
                                    SortableList {
                                        group: "cards", id: col.id.clone(), label: col.title.clone(),
                                        class: "sortable-demo-cards",
                                        style: "flex: 1; min-height: 40px; max-height: 220px; overflow-y: auto; padding: 2px",
                                        judge: Some(wip_judge(board)),
                                        drag_keys: Some(selection_keys(board, selection)),
                                        ondrop: move |m: SortMove| dropped(board, status, m),
                ondropchange: move |o: Option<(SortMove, SortVerdict)>| offered(status, o),
                                        for card in col.cards.clone() {
                                            SortableItem {
                                                key: card.clone(), value: card.clone(), label: card.clone(),
                                                style: {
                                                    let card = card.clone();
                                                    move || format!(
                                                        "padding: 8px; border-radius: 4px; font-size: 13px; background: var(--rinch-color-body); border: 1px solid {}",
                                                        if selection.get().contains(&card) { "var(--rinch-primary-color)" } else { "var(--rinch-color-border)" }
                                                    )
                                                },
                                                div {
                                                    onclick: {
                                                        let card = card.clone();
                                                        move || {
                                                            let ctx = get_click_context();
                                                            if ctx.modifiers.ctrl || ctx.modifiers.meta {
                                                                selection.update(|s| {
                                                                    if let Some(i) = s.iter().position(|k| *k == card) {
                                                                        s.remove(i);
                                                                    } else {
                                                                        s.push(card.clone());
                                                                    }
                                                                });
                                                            } else {
                                                                selection.set(Vec::new());
                                                                opened.set(card.clone());
                                                            }
                                                        }
                                                    },
                                                    {card.clone()}
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
