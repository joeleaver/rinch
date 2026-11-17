use super::*;
use std::rc::Rc;

crate::doc_keyed! {
    static MAP: DocMap<u32> = DocMap::new();
    static LIST: DocList<(Option<u64>, u32)> = DocList::new(|e| e.0);
    static NEVER_WRITTEN: DocMap<u32> = DocMap::new();
    static REENTERED: DocMap<OnDrop> = DocMap::new();
}

/// The names the registries above get: their statics' paths.
const MAP_NAME: &str = "rinch_core::doc_keyed::tests::MAP";
const NEVER_WRITTEN_NAME: &str = "rinch_core::doc_keyed::tests::NEVER_WRITTEN";

fn counts(doc: u64) -> Vec<(&'static str, usize)> {
    let mut counts = doc_entries(doc);
    counts.sort_unstable();
    counts
}

#[test]
fn a_map_keeps_one_value_per_document_and_forgets_one_document() {
    MAP.with_mut(|m| {
        m.insert(1, 10);
        m.insert(2, 20);
        assert_eq!(m.insert(1, 11), Some(10));
    });
    assert_eq!(MAP.with(|m| m.get(1).copied()), Some(11));
    assert_eq!(MAP.with(|m| m.entries_for_doc(1)), 1);
    forget_doc(1);
    assert_eq!(MAP.with(|m| m.get(1).copied()), None);
    assert_eq!(
        MAP.with(|m| m.get(2).copied()),
        Some(20),
        "another document's stays"
    );
    forget_doc(1);
    assert_eq!(
        MAP.with(|m| m.get(2).copied()),
        Some(20),
        "a second sweep finds nothing"
    );
}

#[test]
fn a_list_forgets_one_documents_entries_and_keeps_the_order_of_the_rest() {
    LIST.with_mut(|l| {
        l.push((Some(1), 1));
        l.push((Some(2), 2));
        l.push((None, 3));
        l.push((Some(1), 4));
        l.push((Some(2), 5));
    });
    assert_eq!(LIST.with(|l| l.entries_for_doc(1)), 2);
    forget_doc(1);
    assert_eq!(
        LIST.with(|l| l.iter().map(|e| e.1).collect::<Vec<_>>()),
        [2, 3, 5],
        "the other document's and the documentless entries, in order"
    );
}

#[test]
fn a_registry_joins_the_inventory_on_its_first_write_once() {
    // Reads do not register.
    assert!(NEVER_WRITTEN.with(|m| m.is_empty()));
    assert!(
        !counts(1)
            .iter()
            .any(|(name, _)| *name == NEVER_WRITTEN_NAME),
        "a registry only read is not in the inventory"
    );
    MAP.with_mut(|m| m.insert(9, 1));
    MAP.with_mut(|m| m.insert(9, 2));
    let named: Vec<_> = counts(9)
        .into_iter()
        .filter(|(name, _)| *name == MAP_NAME)
        .collect();
    assert_eq!(
        named,
        [(MAP_NAME, 1)],
        "in the inventory once, with its entry counted"
    );
    forget_doc(9);
    assert_eq!(
        counts(9).into_iter().find(|(name, _)| *name == MAP_NAME),
        Some((MAP_NAME, 0))
    );
}

/// A value whose drop writes the registry it was taken from: a taken
/// callback's captures are app code and may do so.
struct OnDrop(Rc<dyn Fn()>);

impl Drop for OnDrop {
    fn drop(&mut self) {
        (self.0)();
    }
}

#[test]
fn a_taken_entry_is_dropped_after_its_registry_is_let_go() {
    REENTERED.with_mut(|m| {
        m.insert(
            3,
            OnDrop(Rc::new(|| {
                // Registers again, for another document, from the drop.
                REENTERED.with_mut(|m| m.insert(4, OnDrop(Rc::new(|| {}))));
            })),
        )
    });
    forget_doc(3);
    assert!(REENTERED.with(|m| !m.contains(3)));
    assert!(REENTERED.with(|m| m.contains(4)), "the drop's write landed");
    forget_doc(4);
}

#[test]
fn each_thread_has_its_own_inventory() {
    MAP.with_mut(|m| m.insert(5, 1));
    let other = std::thread::spawn(|| {
        // This thread has written nothing: its inventory is empty, and the
        // other thread's entry is not in its instance of the registry.
        let before = doc_entries(5);
        MAP.with_mut(|m| m.insert(5, 2));
        forget_doc(5);
        (before, MAP.with(|m| m.get(5).copied()))
    })
    .join()
    .unwrap();
    assert_eq!(other, (Vec::new(), None));
    assert_eq!(
        MAP.with(|m| m.get(5).copied()),
        Some(1),
        "the other thread's sweep left this thread's entry"
    );
    forget_doc(5);
}

#[test]
fn document_zero_is_no_document() {
    MAP.with_mut(|m| m.insert(0, 1));
    forget_doc(0);
    assert_eq!(MAP.with(|m| m.get(0).copied()), Some(1));
    MAP.with_mut(|m| m.remove(0));
}

/// A sweep that finds a registry borrowed sweeps the others and reports that
/// one, naming it and the document, rather than skipping it in silence.
#[test]
fn a_sweep_reports_a_borrowed_registry_and_sweeps_the_rest() {
    MAP.with_mut(|m| m.insert(6, 1));
    LIST.with_mut(|l| l.push((Some(6), 1)));
    let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MAP.with(|_| forget_doc(6));
    }));
    let message = report
        .expect_err("a borrowed registry is reported")
        .downcast::<String>()
        .map(|m| *m)
        .unwrap_or_default();
    assert!(
        message.contains(MAP_NAME) && message.contains("document 6"),
        "names the registry and the document: {message}"
    );
    assert_eq!(
        LIST.with(|l| l.entries_for_doc(6)),
        0,
        "the others are swept"
    );
    assert_eq!(
        MAP.with(|m| m.get(6).copied()),
        Some(1),
        "the borrowed one is not"
    );
    forget_doc(6);
}
