//! Keyed list reconciliation algorithm.
//!
//! This module implements an efficient LIS-based (Longest Increasing Subsequence)
//! diffing algorithm for keyed lists. This is the same approach used by SolidJS,
//! Inferno, and Leptos for optimal DOM updates.
//!
//! # Algorithm Overview
//!
//! 1. Find common prefix and suffix (items that haven't moved)
//! 2. Handle simple cases: all items added, all removed, or empty
//! 3. For the remaining items, use LIS to find the maximum stable subsequence
//! 4. Generate minimal move/insert/remove operations
//!
//! # Example
//!
//! ```ignore
//! use rinch_core::reconcile::diff_keyed;
//!
//! let old = vec!["a", "b", "c", "d"];
//! let new = vec!["a", "c", "b", "e"];
//!
//! let ops = diff_keyed(&old, &new);
//! // ops contains: Remove("d"), Move("c" before "b"), Insert("e")
//! ```

use std::collections::HashMap;
use std::hash::Hash;

/// Operations to transform old list into new list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListOp<K> {
    /// Insert a new item at the given index in the new list.
    Insert {
        /// The key of the item to insert.
        key: K,
        /// The index in the new list where this item should appear.
        new_index: usize,
    },
    /// Remove an item from the old list.
    Remove {
        /// The key of the item to remove.
        key: K,
        /// The index in the old list where this item was.
        old_index: usize,
    },
    /// Move an item from one position to another.
    Move {
        /// The key of the item to move.
        key: K,
        /// The original index in the old list.
        old_index: usize,
        /// The target index in the new list.
        new_index: usize,
    },
}

/// Diff two keyed lists and return the minimal operations to transform old into new.
///
/// The algorithm is O(n log n) where n is the length of the lists, using the
/// Longest Increasing Subsequence approach.
///
/// # Arguments
///
/// * `old` - The original list of keys
/// * `new` - The desired list of keys
///
/// # Preconditions
///
/// **`old` and `new` must each be duplicate-free.** The index maps below are
/// built with `collect`, so a repeated key silently keeps only its *last*
/// index, and the prefix/suffix scan and the `common_items` range filter then
/// reason against that one index — placing a later insert against the wrong
/// sibling. `for_each_dom` guarantees the precondition by deduplicating its
/// item list at the source; do not relax it here (issue #185).
///
/// # Returns
///
/// A vector of operations that, when applied in order, will transform old into new.
#[allow(clippy::needless_range_loop)]
pub fn diff_keyed<K>(old: &[K], new: &[K]) -> Vec<ListOp<K>>
where
    K: Clone + Eq + Hash,
{
    // Handle empty cases
    if old.is_empty() && new.is_empty() {
        return Vec::new();
    }

    if old.is_empty() {
        // All insertions
        return new
            .iter()
            .enumerate()
            .map(|(i, key)| ListOp::Insert {
                key: key.clone(),
                new_index: i,
            })
            .collect();
    }

    if new.is_empty() {
        // All removals (in reverse order for stable indices)
        return old
            .iter()
            .enumerate()
            .rev()
            .map(|(i, key)| ListOp::Remove {
                key: key.clone(),
                old_index: i,
            })
            .collect();
    }

    // Build index maps
    let old_map: HashMap<&K, usize> = old.iter().enumerate().map(|(i, k)| (k, i)).collect();
    let new_map: HashMap<&K, usize> = new.iter().enumerate().map(|(i, k)| (k, i)).collect();

    let mut ops = Vec::new();

    // Find common prefix
    let mut prefix_len = 0;
    while prefix_len < old.len() && prefix_len < new.len() && old[prefix_len] == new[prefix_len] {
        prefix_len += 1;
    }

    // Find common suffix
    let mut suffix_len = 0;
    while suffix_len < (old.len() - prefix_len)
        && suffix_len < (new.len() - prefix_len)
        && old[old.len() - 1 - suffix_len] == new[new.len() - 1 - suffix_len]
    {
        suffix_len += 1;
    }

    // The range we need to process
    let old_start = prefix_len;
    let old_end = old.len() - suffix_len;
    let new_start = prefix_len;
    let new_end = new.len() - suffix_len;

    // If nothing to process after prefix/suffix, we're done
    if old_start >= old_end && new_start >= new_end {
        return ops;
    }

    // Collect removed items (in old but not in new, within the range)
    for i in old_start..old_end {
        if !new_map.contains_key(&old[i]) {
            ops.push(ListOp::Remove {
                key: old[i].clone(),
                old_index: i,
            });
        }
    }

    // Collect items that exist in both lists (need to check for moves)
    let mut common_items: Vec<(usize, usize)> = Vec::new();
    for i in new_start..new_end {
        if let Some(&old_idx) = old_map.get(&new[i])
            && old_idx >= old_start
            && old_idx < old_end
        {
            common_items.push((old_idx, i));
        }
    }

    // Find LIS based on old indices to identify which items don't need to move
    let lis_set = find_lis_indices(&common_items);

    // Items not in LIS need to be moved
    for (idx, &(old_idx, new_idx)) in common_items.iter().enumerate() {
        if !lis_set.contains(&idx) {
            ops.push(ListOp::Move {
                key: old[old_idx].clone(),
                old_index: old_idx,
                new_index: new_idx,
            });
        }
    }

    // Collect inserted items (in new but not in old)
    for i in new_start..new_end {
        if !old_map.contains_key(&new[i]) {
            ops.push(ListOp::Insert {
                key: new[i].clone(),
                new_index: i,
            });
        }
    }

    ops
}

/// Find the indices in `items` that form the Longest Increasing Subsequence
/// based on old_idx ordering.
///
/// Returns a set of indices into the input vector.
fn find_lis_indices(items: &[(usize, usize)]) -> std::collections::HashSet<usize> {
    if items.is_empty() {
        return std::collections::HashSet::new();
    }

    let n = items.len();

    // Build sequence of old indices
    let seq: Vec<usize> = items.iter().map(|&(old_idx, _)| old_idx).collect();

    // LIS using binary search - O(n log n)
    // tails[i] holds the index in seq of the smallest tail element for LIS of length i+1
    let mut tails: Vec<usize> = Vec::new();
    // predecessor[i] holds the predecessor index for seq[i] in the LIS
    let mut predecessor: Vec<Option<usize>> = vec![None; n];
    // indices[i] holds the index in seq corresponding to tails[i]
    let mut indices: Vec<usize> = Vec::new();

    for i in 0..n {
        let val = seq[i];

        // Binary search for the position where val should go
        let pos = tails
            .binary_search_by(|&tail_idx| seq[tail_idx].cmp(&val))
            .unwrap_or_else(|x| x);

        // Set predecessor
        if pos > 0 {
            predecessor[i] = Some(indices[pos - 1]);
        }

        // Update or extend tails
        if pos >= tails.len() {
            tails.push(i);
            indices.push(i);
        } else {
            tails[pos] = i;
            indices[pos] = i;
        }
    }

    // Reconstruct LIS indices
    let mut lis_indices = std::collections::HashSet::new();
    if let Some(&last_idx) = indices.last() {
        let mut current = Some(last_idx);
        while let Some(idx) = current {
            lis_indices.insert(idx);
            current = predecessor[idx];
        }
    }

    lis_indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_empty_lists() {
        let old: Vec<&str> = vec![];
        let new: Vec<&str> = vec![];
        let ops = diff_keyed(&old, &new);
        assert!(ops.is_empty());
    }

    #[test]
    fn test_diff_append() {
        let old = vec!["a", "b", "c"];
        let new = vec!["a", "b", "c", "d", "e"];
        let ops = diff_keyed(&old, &new);

        // Should have 2 insertions
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        assert_eq!(inserts.len(), 2);

        // Verify the keys
        assert!(
            ops.iter()
                .any(|op| matches!(op, ListOp::Insert { key, new_index: 3 } if *key == "d"))
        );
        assert!(
            ops.iter()
                .any(|op| matches!(op, ListOp::Insert { key, new_index: 4 } if *key == "e"))
        );
    }

    #[test]
    fn test_diff_prepend() {
        let old = vec!["c", "d"];
        let new = vec!["a", "b", "c", "d"];
        let ops = diff_keyed(&old, &new);

        // Should have 2 insertions
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        assert_eq!(inserts.len(), 2);
    }

    #[test]
    fn test_diff_remove() {
        let old = vec!["a", "b", "c", "d", "e"];
        let new = vec!["a", "c", "e"];
        let ops = diff_keyed(&old, &new);

        // Should have 2 removals
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();
        assert_eq!(removes.len(), 2);

        // Verify the keys
        assert!(
            ops.iter()
                .any(|op| matches!(op, ListOp::Remove { key, .. } if *key == "b"))
        );
        assert!(
            ops.iter()
                .any(|op| matches!(op, ListOp::Remove { key, .. } if *key == "d"))
        );
    }

    #[test]
    fn test_diff_remove_middle() {
        let old = vec!["a", "b", "c"];
        let new = vec!["a", "c"];
        let ops = diff_keyed(&old, &new);

        assert_eq!(ops.len(), 1);
        assert!(matches!(&ops[0], ListOp::Remove { key, old_index: 1 } if *key == "b"));
    }

    #[test]
    fn test_diff_reorder() {
        let old = vec!["a", "b", "c"];
        let new = vec!["c", "b", "a"];
        let ops = diff_keyed(&old, &new);

        // Should have moves, no insertions or removals
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();

        assert!(inserts.is_empty());
        assert!(removes.is_empty());

        // All items are still present, just reordered
        let moves: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Move { .. }))
            .collect();
        assert!(!moves.is_empty());
    }

    #[test]
    fn test_diff_swap_two() {
        let old = vec!["a", "b"];
        let new = vec!["b", "a"];
        let ops = diff_keyed(&old, &new);

        // Should have exactly 1 move (the other stays in place via LIS)
        let moves: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Move { .. }))
            .collect();
        assert_eq!(moves.len(), 1);
    }

    #[test]
    fn test_diff_mixed() {
        let old = vec!["a", "b", "c", "d"];
        let new = vec!["b", "e", "c", "a"];
        let ops = diff_keyed(&old, &new);

        // Should have: remove "d", insert "e", move "a" (or "b" depending on LIS)
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();

        assert_eq!(inserts.len(), 1);
        assert!(
            ops.iter()
                .any(|op| matches!(op, ListOp::Insert { key, .. } if *key == "e"))
        );

        assert_eq!(removes.len(), 1);
        assert!(
            ops.iter()
                .any(|op| matches!(op, ListOp::Remove { key, .. } if *key == "d"))
        );
    }

    #[test]
    fn test_diff_all_new() {
        let old = vec!["a", "b"];
        let new = vec!["c", "d", "e"];
        let ops = diff_keyed(&old, &new);

        // Should have 2 removals and 3 insertions
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();

        assert_eq!(inserts.len(), 3);
        assert_eq!(removes.len(), 2);
    }

    #[test]
    fn test_diff_with_duplicates_in_new_position() {
        // Test where same key appears in new position
        let old = vec!["a", "b", "c"];
        let new = vec!["c", "a", "b"];
        let ops = diff_keyed(&old, &new);

        // No insertions or removals, only moves
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();

        assert!(inserts.is_empty());
        assert!(removes.is_empty());
    }

    #[test]
    fn test_diff_identical() {
        let old = vec!["a", "b", "c"];
        let new = vec!["a", "b", "c"];
        let ops = diff_keyed(&old, &new);

        // No operations needed
        assert!(ops.is_empty());
    }

    #[test]
    fn test_diff_common_prefix() {
        let old = vec!["a", "b", "c", "d"];
        let new = vec!["a", "b", "x", "y"];
        let ops = diff_keyed(&old, &new);

        // Common prefix "a", "b" should be skipped
        // Should remove "c", "d" and insert "x", "y"
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();

        assert_eq!(inserts.len(), 2);
        assert_eq!(removes.len(), 2);
    }

    #[test]
    fn test_diff_common_suffix() {
        let old = vec!["a", "b", "c", "d"];
        let new = vec!["x", "y", "c", "d"];
        let ops = diff_keyed(&old, &new);

        // Common suffix "c", "d" should be skipped
        // Should remove "a", "b" and insert "x", "y"
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();

        assert_eq!(inserts.len(), 2);
        assert_eq!(removes.len(), 2);
    }

    #[test]
    fn test_lis_basic() {
        // Test with items that need sorting: (old_idx, new_idx)
        // old: [a=0, b=1, c=2]  new: [c=0, a=1, b=2]
        // Mapping: a: (0, 1), b: (1, 2), c: (2, 0)
        // In new order: [(2, 0), (0, 1), (1, 2)]
        // LIS by old_idx: We want items where old_idx is increasing
        // 2, 0, 1 -> LIS could be [0, 1] (indices 1, 2) or just [2] (index 0)
        let items = vec![(2, 0), (0, 1), (1, 2)];
        let lis = find_lis_indices(&items);

        // LIS of [2, 0, 1] is [0, 1] (length 2), so indices 1 and 2 in items
        assert!(lis.contains(&1) && lis.contains(&2));
    }

    #[test]
    fn test_lis_already_sorted() {
        // Items already in order by old_idx
        let items = vec![(0, 0), (1, 1), (2, 2)];
        let lis = find_lis_indices(&items);

        // All items are in LIS
        assert_eq!(lis.len(), 3);
    }
}
