//! The stores of the kept walks' records ([`Records`]) and entries,
//! indexed by the node's slab slot (#2054).

use super::deep_record::Rec;
use crate::node::RawNodeId;
use crate::node_slab::slot_of;

/// A slot holding no record.
const VACANT: u32 = u32::MAX;

/// The records of every kept walk, by box: a dense list of records and, for
/// each slab slot, where its box's record is in it. A check looks up a few
/// records per box it was pushed (`row_shift_damage` in `rinch-bench`: each
/// of 2000 rows that moved is marked, climbed from and visited), so a lookup
/// is an index and a compare of the whole id, not a hash; it was FxHash,
/// and before that the std `SipHash`, whose inserts were most of a walk's
/// cost. The slot index costs four bytes per slot up to the highest slot
/// recorded.
///
/// A record lives as long as its box is where the walk met it: a box taken
/// out of the tree (`DeepOverflow::forget_detached`) or freed
/// (`DeepOverflow::forget_freed`) has its records dropped then, and a walk
/// dropped to be taken again drops the records it made. So the store holds
/// at most one record per box in the kept walks. A lookup compares the
/// whole id, generation included, so a freed slot's new node (whose id
/// differs only in its generation) finds none; an insert for it replaces
/// whatever record the slot held.
pub(super) type Records = Slots<Rec>;

/// Values by node id, indexed by slab slot ([`Records`]; the entries of
/// `DeepOverflowState`, which a check looks up for each box pushed too).
#[derive(Debug)]
pub(super) struct Slots<T> {
    /// For each slab slot, the index in `recs` of its box's record, or
    /// [`VACANT`].
    slots: Vec<u32>,
    /// The records, each with its box's id.
    recs: Vec<(RawNodeId, T)>,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            recs: Vec::new(),
        }
    }
}

impl<T> Slots<T> {
    /// The index in `recs` of box `id`'s record.
    #[inline]
    fn index(&self, id: RawNodeId) -> Option<usize> {
        let i = *self.slots.get(slot_of(id))?;
        if i == VACANT {
            return None;
        }
        let i = i as usize;
        (self.recs[i].0 == id).then_some(i)
    }

    #[inline]
    pub(super) fn get(&self, id: &RawNodeId) -> Option<&T> {
        self.index(*id).map(|i| &self.recs[i].1)
    }

    #[inline]
    pub(super) fn get_mut(&mut self, id: &RawNodeId) -> Option<&mut T> {
        self.index(*id).map(|i| &mut self.recs[i].1)
    }

    #[inline]
    pub(super) fn contains_key(&self, id: &RawNodeId) -> bool {
        self.index(*id).is_some()
    }

    /// Record box `id`, replacing its record — or the record of a box its
    /// slot held before (one never forgotten, which the lifetime rules
    /// above leave none of).
    pub(super) fn insert(&mut self, id: RawNodeId, rec: T) {
        let slot = slot_of(id);
        if slot >= self.slots.len() {
            self.slots.resize(slot + 1, VACANT);
        }
        match self.slots[slot] {
            VACANT => {
                self.slots[slot] = u32::try_from(self.recs.len())
                    .expect("fewer records than slab slots, which are u32");
                self.recs.push((id, rec));
            }
            i => self.recs[i as usize] = (id, rec),
        }
    }

    /// Drop box `id`'s record; the record, if it had one.
    #[inline]
    pub(super) fn remove(&mut self, id: &RawNodeId) -> Option<T> {
        let i = self.index(*id)?;
        self.slots[slot_of(*id)] = VACANT;
        let (_, rec) = self.recs.swap_remove(i);
        if let Some(&(moved, _)) = self.recs.get(i) {
            // The last record took the removed one's place.
            self.slots[slot_of(moved)] = i as u32;
        }
        Some(rec)
    }

    /// Each id with its value, in no particular order.
    pub(super) fn iter(&self) -> impl Iterator<Item = (&RawNodeId, &T)> {
        self.recs.iter().map(|(id, v)| (id, v))
    }

    /// Each id with a value, in no particular order.
    pub(super) fn keys(&self) -> impl Iterator<Item = &RawNodeId> {
        self.recs.iter().map(|(id, _)| id)
    }

    /// Drop every value; the slot index keeps its length, all vacant.
    pub(super) fn clear(&mut self) {
        self.slots.fill(VACANT);
        self.recs.clear();
    }

    pub(super) fn len(&self) -> usize {
        self.recs.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.recs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::Slots;

    /// A removal moves the last value into the removed one's place and
    /// keeps finding it; a new generation in a slot finds no value, and its
    /// insert replaces the old one.
    #[test]
    fn values_follow_their_slots_and_generations() {
        let mut r = Slots::<u32>::default();
        let (a, b, c) = (3, 7, 1u64 << 32 | 3);
        r.insert(a, 10);
        r.insert(b, 20);
        assert_eq!(r.len(), 2);
        assert_eq!(r.get(&b), Some(&20));
        assert!(r.get(&c).is_none(), "another generation of a's slot");
        assert_eq!(r.remove(&a), Some(10));
        assert_eq!(r.get(&b), Some(&20), "moved, found");
        assert!(r.remove(&a).is_none());
        r.insert(b, 21);
        assert_eq!((r.len(), r.get(&b)), (1, Some(&21)));
        r.insert(a, 11);
        r.insert(c, 30);
        assert_eq!(r.len(), 2, "c replaced a in their slot");
        assert!(!r.contains_key(&a));
        assert_eq!(r.get_mut(&c), Some(&mut 30));
        assert_eq!(r.remove(&b), Some(21));
        assert_eq!(r.get(&c), Some(&30));
        assert!(!r.is_empty());
        assert_eq!(r.keys().copied().collect::<Vec<_>>(), vec![c]);
        assert_eq!(
            r.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
            vec![(c, 30)]
        );
        r.clear();
        assert!(r.is_empty() && r.get(&c).is_none());
        r.insert(b, 22);
        assert_eq!(r.get(&b), Some(&22), "inserted after a clear");
    }
}
