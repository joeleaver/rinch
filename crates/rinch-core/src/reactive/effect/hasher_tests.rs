use super::*;

/// Every write folds, so a key that hashes more than one word is not
/// reduced to its last word.
///
/// `ObserverId` writes exactly one `usize` today, so this is a guard on the
/// hasher's stated contract rather than on current behaviour: an assigning
/// `write_usize` collides `(1, 2)` with `(9, 2)` outright, silently, the day
/// the key type grows a field.
#[test]
fn writes_fold_rather_than_overwrite() {
    fn hash_words(words: &[usize]) -> u64 {
        let mut h = ObserverIdHasher::default();
        for &w in words {
            h.write_usize(w);
        }
        h.finish()
    }

    assert_ne!(hash_words(&[1, 2]), hash_words(&[9, 2]));
    assert_ne!(hash_words(&[1, 2]), hash_words(&[2, 1]));
    // The single-word case is unchanged by folding: the state starts at 0.
    assert_eq!(hash_words(&[7]), 7u64.wrapping_mul(0x9E37_79B9_7F4A_7C15));
}

/// Consecutive ids — the only keys this map ever sees — land in distinct
/// buckets *and* distinct control bytes.
///
/// The multiply is what buys the second half: an identity hash leaves the
/// top 7 bits (hashbrown's control byte) constant across every id a real
/// program allocates.
#[test]
fn consecutive_ids_spread_across_buckets_and_control_bytes() {
    use std::collections::HashSet;

    let hashes: Vec<u64> = (0..256usize)
        .map(|n| {
            let mut h = ObserverIdHasher::default();
            h.write_usize(n);
            h.finish()
        })
        .collect();

    let low: HashSet<u64> = hashes.iter().map(|h| h & 0xFF).collect();
    assert_eq!(low.len(), 256, "bucket index must be a permutation");
    let control: HashSet<u64> = hashes.iter().map(|h| h >> 57).collect();
    assert!(
        control.len() > 64,
        "control bytes must vary, got {}",
        control.len()
    );
}
