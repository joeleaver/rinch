use super::*;
use std::cell::Cell;
use std::rc::Rc;

struct ReadsSelfOnDrop(Signal<ReadsSelfOnDrop>, Rc<Cell<bool>>);
impl Drop for ReadsSelfOnDrop {
    fn drop(&mut self) {
        let _ = self.0.try_with(|_| ());
        self.1.set(true);
    }
}

#[test]
fn a_displaced_value_that_reads_its_own_signal_on_drop_does_not_panic() {
    let ran = Rc::new(Cell::new(false));
    // `Signal<T>` is just an id/generation pair with a `PhantomData<T>` —
    // no invariant beyond that, so an all-zero handle is a valid (if
    // meaningless) bit pattern to seed the self-reference with before
    // `holder`'s real id exists.
    let holder: Signal<ReadsSelfOnDrop> =
        Signal::new(ReadsSelfOnDrop(unsafe { std::mem::zeroed() }, ran.clone()));
    holder.set(ReadsSelfOnDrop(holder, ran.clone()));
    ran.set(false);
    holder.set(ReadsSelfOnDrop(holder, ran.clone()));
    assert!(
        ran.get(),
        "the self-referential value's Drop must have run and read `holder`"
    );
    holder.free_for_tests();
}
