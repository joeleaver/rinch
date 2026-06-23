//! Position remapping: [`StepMap`] and [`Mapping`].
//!
//! When a step changes the document it shifts every position after the edited
//! range. A [`StepMap`] is the compact description of that shift — a flat list of
//! `[start, old_size, new_size]` triples — and knows how to map any old position
//! to its new home (and, inverted, back again). A [`Mapping`] is an ordered
//! pipeline of step maps with *mirror* bookkeeping, so that mapping a position
//! through a step **and its inverse** (collab rebase, undo) recovers the original
//! position instead of collapsing it into a deleted range.
//!
//! This is a faithful port of ProseMirror's `transform/src/map.ts`; the
//! association (`assoc`) and deletion-tracking semantics match exactly, because
//! selection mapping, history, and collaborative rebase all depend on them.

/// `pos` falls inside content that was deleted, on the side **before** it.
const DEL_BEFORE: u8 = 1;
/// `pos` falls inside content that was deleted, on the side **after** it.
const DEL_AFTER: u8 = 2;
/// `pos` was inside a deleted range (content removed on both sides).
const DEL_ACROSS: u8 = 4;
/// The position itself was inside deleted content.
const DEL_SIDE: u8 = DEL_BEFORE | DEL_AFTER;

/// An opaque token (produced by [`StepMap::map_result`], consumed by the *mirror*
/// map's [`StepMap::recover`]) that records where, inside a step's replaced range,
/// a position sat — so the inverse map can restore it precisely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recover {
    /// Which range (triple index) of the step map the position fell in.
    index: usize,
    /// The offset of the position from that range's start.
    offset: usize,
}

/// The result of mapping a position through a [`StepMap`]/[`Mapping`]: the new
/// position plus information about whether it landed in deleted content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapResult {
    /// The mapped position.
    pub pos: usize,
    del_info: u8,
    recover: Option<Recover>,
}

impl MapResult {
    /// True if the position was inside content that the step deleted.
    pub fn deleted(&self) -> bool {
        self.del_info & DEL_SIDE > 0
    }
    /// True if the content immediately before the position was deleted.
    pub fn deleted_before(&self) -> bool {
        self.del_info & (DEL_BEFORE | DEL_ACROSS) > 0
    }
    /// True if the content immediately after the position was deleted.
    pub fn deleted_after(&self) -> bool {
        self.del_info & (DEL_AFTER | DEL_ACROSS) > 0
    }
    /// True if a range *spanning* the position was deleted.
    pub fn deleted_across(&self) -> bool {
        self.del_info & DEL_ACROSS > 0
    }
}

/// A map of the positions affected by a single step: a flat list of
/// `[start, old_size, new_size]` triples (sorted by `start`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepMap {
    /// Flattened `[start, old_size, new_size]` triples.
    ranges: Vec<usize>,
    /// When true, the map is read in the inverse direction (new ↔ old).
    inverted: bool,
}

impl StepMap {
    /// Build a step map from a flat list of `[start, old_size, new_size]` triples.
    pub fn new(ranges: Vec<usize>) -> StepMap {
        debug_assert!(
            ranges.len().is_multiple_of(3),
            "step map ranges must be triples"
        );
        StepMap {
            ranges,
            inverted: false,
        }
    }

    /// The identity map (no position changes) — for steps that don't move content
    /// (mark/attr steps).
    pub fn empty() -> StepMap {
        StepMap {
            ranges: Vec::new(),
            inverted: false,
        }
    }

    /// The inverse map (swaps the old/new direction). Cheap: shares the ranges.
    pub fn invert(&self) -> StepMap {
        StepMap {
            ranges: self.ranges.clone(),
            inverted: !self.inverted,
        }
    }

    /// Map a position, returning just the new position. `assoc` (`-1` or `1`)
    /// chooses which side a position on a boundary biases toward.
    pub fn map(&self, pos: usize, assoc: i32) -> usize {
        self.map_full(pos, assoc).pos
    }

    /// Map a position, returning the new position plus deletion information.
    pub fn map_result(&self, pos: usize, assoc: i32) -> MapResult {
        self.map_full(pos, assoc)
    }

    fn map_full(&self, pos: usize, assoc: i32) -> MapResult {
        let pos = pos as isize;
        let mut diff: isize = 0;
        let (old_index, new_index) = if self.inverted { (2, 1) } else { (1, 2) };
        let mut i = 0;
        while i < self.ranges.len() {
            let start = self.ranges[i] as isize - if self.inverted { diff } else { 0 };
            if start > pos {
                break;
            }
            let old_size = self.ranges[i + old_index] as isize;
            let new_size = self.ranges[i + new_index] as isize;
            let end = start + old_size;
            if pos <= end {
                let side = if old_size == 0 {
                    assoc
                } else if pos == start {
                    -1
                } else if pos == end {
                    1
                } else {
                    assoc
                };
                let result = start + diff + if side < 0 { 0 } else { new_size };
                let recover = if pos == (if assoc < 0 { start } else { end }) {
                    None
                } else {
                    Some(Recover {
                        index: i / 3,
                        offset: (pos - start) as usize,
                    })
                };
                let mut del_info = if pos == start {
                    DEL_AFTER
                } else if pos == end {
                    DEL_BEFORE
                } else {
                    DEL_ACROSS
                };
                let on_side = if assoc < 0 { pos != start } else { pos != end };
                if on_side {
                    del_info |= DEL_SIDE;
                }
                return MapResult {
                    pos: result as usize,
                    del_info,
                    recover,
                };
            }
            diff += new_size - old_size;
            i += 3;
        }
        MapResult {
            pos: (pos + diff) as usize,
            del_info: 0,
            recover: None,
        }
    }

    /// Recover the absolute position encoded by a [`Recover`] token (produced by
    /// this map's mirror). Used by [`Mapping`] to skip a deletion/re-insertion pair.
    fn recover(&self, rec: Recover) -> usize {
        let mut diff: isize = 0;
        if !self.inverted {
            for i in 0..rec.index {
                diff += self.ranges[i * 3 + 2] as isize - self.ranges[i * 3 + 1] as isize;
            }
        }
        (self.ranges[rec.index * 3] as isize + diff + rec.offset as isize) as usize
    }
}

/// An ordered pipeline of [`StepMap`]s with *mirror* bookkeeping.
///
/// A transaction accumulates one step map per step; mapping a position through
/// the whole `Mapping` walks every map in order. The mirror table records that
/// map *i* is the inverse of map *j* (e.g. a delete that a later step re-inserts,
/// or undo), so a position that a map would drop into a deleted hole is *recovered*
/// in the mirror map instead of being clamped.
#[derive(Clone, Debug, Default)]
pub struct Mapping {
    maps: Vec<StepMap>,
    /// Flat pairs `[a, b, a, b, …]` meaning map `a` mirrors map `b`.
    mirror: Vec<usize>,
    from: usize,
    to: usize,
}

impl Mapping {
    /// An empty mapping.
    pub fn new() -> Mapping {
        Mapping {
            maps: Vec::new(),
            mirror: Vec::new(),
            from: 0,
            to: 0,
        }
    }

    /// The accumulated step maps.
    pub fn maps(&self) -> &[StepMap] {
        &self.maps
    }

    /// Number of step maps.
    pub fn len(&self) -> usize {
        self.maps.len()
    }

    /// True if no maps have been appended.
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }

    /// A view over the maps `from..to`, sharing the underlying maps and mirror
    /// table. The state-layer [`Transaction`](crate::state::Transaction) uses this
    /// to map a selection forward through only the maps added since it was last
    /// set (PM's `Mapping.slice`). The mirror indices remain absolute into the
    /// shared `maps`, so recover/mirror behavior is preserved within the window.
    pub fn slice(&self, from: usize, to: usize) -> Mapping {
        Mapping {
            maps: self.maps.clone(),
            mirror: self.mirror.clone(),
            from,
            to,
        }
    }

    /// Append a step map to the pipeline.
    pub fn append_map(&mut self, map: StepMap) {
        self.maps.push(map);
        self.to = self.maps.len();
    }

    /// Append a step map that mirrors (is the inverse of) the map at index
    /// `mirror`.
    pub fn append_map_mirrored(&mut self, map: StepMap, mirror: usize) {
        self.maps.push(map);
        self.to = self.maps.len();
        let n = self.maps.len() - 1;
        self.mirror.push(n);
        self.mirror.push(mirror);
    }

    /// Append all of `mapping`'s maps (preserving mirror relations).
    pub fn append_mapping(&mut self, mapping: &Mapping) {
        let start_size = self.maps.len();
        for i in 0..mapping.maps.len() {
            let mirr = mapping.get_mirror(i);
            match mirr {
                Some(m) if m < i => {
                    self.append_map_mirrored(mapping.maps[i].clone(), start_size + m)
                }
                _ => self.append_map(mapping.maps[i].clone()),
            }
        }
    }

    /// Append the *inverse* of every map in `mapping`, in reverse order — the
    /// basis for inverting an accumulated mapping (undo, collab rebase).
    pub fn append_mapping_inverted(&mut self, mapping: &Mapping) {
        if mapping.maps.is_empty() {
            return;
        }
        let total_size = self.maps.len() + mapping.maps.len();
        let mut i = mapping.maps.len() as isize - 1;
        while i >= 0 {
            let idx = i as usize;
            let mirror_arg = match mapping.get_mirror(idx) {
                Some(m) if m > idx => Some(total_size - m - 1),
                _ => None,
            };
            match mirror_arg {
                Some(mv) => self.append_map_mirrored(mapping.maps[idx].invert(), mv),
                None => self.append_map(mapping.maps[idx].invert()),
            }
            i -= 1;
        }
    }

    /// A new mapping that inverts this one.
    pub fn invert(&self) -> Mapping {
        let mut inverse = Mapping::new();
        inverse.append_mapping_inverted(self);
        inverse
    }

    fn get_mirror(&self, n: usize) -> Option<usize> {
        let mut i = 0;
        while i < self.mirror.len() {
            if self.mirror[i] == n {
                let partner = if i % 2 == 1 { i - 1 } else { i + 1 };
                return Some(self.mirror[partner]);
            }
            i += 1;
        }
        None
    }

    /// Map a position through the whole pipeline, returning the new position.
    pub fn map(&self, pos: usize, assoc: i32) -> usize {
        if !self.mirror.is_empty() {
            return self.map_inner(pos, assoc).pos;
        }
        let mut pos = pos;
        for i in self.from..self.to {
            pos = self.maps[i].map(pos, assoc);
        }
        pos
    }

    /// Map a position through the whole pipeline, returning deletion info too.
    pub fn map_result(&self, pos: usize, assoc: i32) -> MapResult {
        self.map_inner(pos, assoc)
    }

    fn map_inner(&self, pos: usize, assoc: i32) -> MapResult {
        let mut del_info = 0u8;
        let mut pos = pos;
        let mut i = self.from;
        while i < self.to {
            let result = self.maps[i].map_result(pos, assoc);
            if let Some(rec) = result.recover
                && let Some(corr) = self.get_mirror(i)
                && corr > i
                && corr < self.to
            {
                pos = self.maps[corr].recover(rec);
                i = corr + 1;
                continue;
            }
            del_info |= result.del_info;
            pos = result.pos;
            i += 1;
        }
        MapResult {
            pos,
            del_info,
            recover: None,
        }
    }
}
