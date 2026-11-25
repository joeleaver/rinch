use super::*;

/// Review of #2780: the generation keeps its 32 bits — the tag does not
/// shorten its period. A slot freed and reused at the old 2^24 period's
/// wrap point still tells its stale handle from the new signal.
/// (`tests/stale_handle_wrap_2780.rs` does it the long way, `--ignored`.)
#[test]
fn a_signals_generation_keeps_its_32_bits() {
    let mut store = SignalStore::new();
    // Untagged, so this scratch store takes no thread tag (its `Drop`
    // would give it back under the thread's own store).
    store.tagged = true;
    let (a, gen_a) = store.alloc(1u8);
    assert_eq!(gen_a, 1, "a fresh store's first generation");
    drop(store.free(a, gen_a));
    // As if 2^24 - 3 allocations had run since `a`; then reuse the slot
    // across the old period's wrap point, one allocation at a time.
    store.next_gen = (1 << 24) - 2;
    for _ in 0..4 {
        let (b, gen_b) = store.alloc(2u8);
        assert_eq!(b, a, "the same slot, reused");
        assert!(
            store.get_inner(a, gen_a).is_none(),
            "the stale handle stays dead (generation {gen_b:#x})"
        );
        drop(store.free(b, gen_b));
    }
}
