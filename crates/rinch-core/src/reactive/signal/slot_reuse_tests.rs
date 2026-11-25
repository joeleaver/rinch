use super::*;

#[test]
fn a_stale_handle_after_slot_reuse_reports_dead_not_the_new_occupant() {
    let a = Signal::new(100i32);
    a.free_for_tests();
    let b = Signal::new(200i32);
    assert_eq!(
        b.debug_id(),
        a.debug_id(),
        "need slot reuse for this probe to mean anything"
    );
    assert!(
        !a.is_alive(),
        "a stale handle after slot reuse must report dead"
    );
    assert_eq!(
        a.try_get(),
        None,
        "a stale handle must not read the new occupant's value"
    );
}
