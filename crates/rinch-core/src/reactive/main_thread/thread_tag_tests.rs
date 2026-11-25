use super::*;
use crate::reactive::Signal;

/// #2778: a thread tag given back is taken again with the generation
/// counter where its last thread left it, so a handle to the ended
/// thread's signal never names the new thread's signal in the same slot;
/// past the last tag, threads go untagged.
#[test]
fn a_reused_thread_tag_continues_its_generation_counter() {
    let mut pool = TagPool {
        fresh: 1,
        free: VecDeque::new(),
    };
    assert_eq!(pool.claim(), (1, 1));
    assert_eq!(pool.claim(), (2, 1));
    pool.release(1, 40);
    pool.release(0, 99); // untagged: nothing to give back
    assert_eq!(pool.claim(), (1, 40), "tag 1 again, counting on from 40");
    assert_eq!(pool.claim(), (3, 1));
    pool.fresh = MAX_THREAD_TAG + 1;
    assert_eq!(pool.claim(), (0, 1), "no tag left");
}

/// #2778: a signal's id names the thread whose store holds it, and a
/// store finds nothing for another thread's id in the same slot.
#[test]
fn a_signals_id_names_its_thread() {
    let here = Signal::new(5u8);
    let tag = current_thread_tag();
    assert_ne!(tag, 0, "this thread took a tag at its first signal");
    assert_eq!(signal_owner(here.identity().0), tag);
    let (id, generation) = here.identity();
    let there = std::thread::spawn(move || {
        let s = Signal::new(0u8);
        let alias = SIGNAL_STORE.with(|store| store.borrow().get_inner(id, generation));
        (
            signal_owner(s.identity().0),
            current_thread_tag(),
            alias.is_none(),
        )
    })
    .join()
    .unwrap();
    assert_eq!(there.0, there.1);
    assert_ne!(there.1, tag, "two live threads never share a tag");
    assert!(there.2, "another thread's id names no signal here");
}
