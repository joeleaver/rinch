//! Listener lists: several subscribers, each removable by id.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Names a listener so it can be removed. Ids are unique per list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ListenerId(u64);

type Listener<E> = Arc<dyn Fn(&E) + Send + Sync>;

pub(crate) struct Listeners<E> {
    next: AtomicU64,
    list: RwLock<Vec<(u64, Listener<E>)>>,
}

impl<E> Default for Listeners<E> {
    fn default() -> Self {
        Self {
            next: AtomicU64::new(0),
            list: RwLock::new(Vec::new()),
        }
    }
}

impl<E> Listeners<E> {
    pub(crate) fn add(&self, listener: Listener<E>) -> ListenerId {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.list
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .push((id, listener));
        ListenerId(id)
    }

    pub(crate) fn remove(&self, id: ListenerId) -> bool {
        let mut list = self.list.write().unwrap_or_else(|p| p.into_inner());
        let before = list.len();
        list.retain(|(i, _)| *i != id.0);
        list.len() != before
    }

    /// Calls every listener in the order added, with no lock held (a listener
    /// may add or remove listeners, or read what changed).
    pub(crate) fn notify(&self, event: &E) {
        let snapshot: Vec<Listener<E>> = self
            .list
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(_, l)| Arc::clone(l))
            .collect();
        for listener in snapshot {
            listener(event);
        }
    }
}
