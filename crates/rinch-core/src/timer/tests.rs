use super::*;
use std::rc::Rc;

/// Drive timers synchronously: record what was scheduled, fire it on demand.
/// (Mirrors what a real backend does, minus the waiting.)
fn install_manual_backend() {
    fn manual(id: u64, delay_ms: u32) {
        SCHEDULED.with(|s| s.borrow_mut().push(id));
        DELAYS.with(|d| d.borrow_mut().push(delay_ms));
    }
    set_timer_backend(manual);
}

thread_local! {
    static SCHEDULED: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
    static DELAYS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn drain_and_fire() {
    let ids: Vec<u64> = SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut()));
    for id in ids {
        fire_timeout(id);
    }
}

/// Outside a harness, [`now`] is the wall clock (#2839): it lands between
/// two `Instant::now()` read either side of it, here and after a sleep.
#[test]
fn now_is_the_wall_clock_without_a_harness() {
    for _ in 0..2 {
        let before = Instant::now();
        let n = now();
        let after = Instant::now();
        assert!(
            before <= n && n <= after,
            "{before:?} <= {n:?} <= {after:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Under a virtual clock [`now`] moves by exactly what the clock is
/// advanced by, and not with the wall clock. The clock is the thread's:
/// another thread reads the wall clock meanwhile, and once uninstalled
/// this one does again (#2839).
#[test]
fn now_follows_the_threads_virtual_clock() {
    use std::time::Duration;
    virtual_time::install();
    let t0 = now();
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(now(), t0, "the wall clock moved; the virtual one did not");
    virtual_time::advance_to(Duration::from_millis(299));
    assert_eq!(now() - t0, Duration::from_millis(299));
    virtual_time::advance_to(Duration::from_millis(300));
    assert_eq!(now() - t0, Duration::from_millis(300));

    let (before, other, after) = std::thread::spawn(|| (Instant::now(), now(), Instant::now()))
        .join()
        .unwrap();
    assert!(
        before <= other && other <= after,
        "another thread reads the wall clock"
    );

    virtual_time::uninstall();
    let before = Instant::now();
    let n = now();
    assert!(
        before <= n && n <= Instant::now(),
        "uninstalled: the wall clock again"
    );
}

#[test]
fn fires_once_and_can_be_cancelled() {
    install_manual_backend();

    // Fires when the deadline arrives.
    let hits = Rc::new(std::cell::Cell::new(0));
    let h = hits.clone();
    set_timeout(10, move || h.set(h.get() + 1));
    drain_and_fire();
    assert_eq!(hits.get(), 1);

    // One-shot: firing again does nothing (the callback was taken).
    drain_and_fire();
    assert_eq!(hits.get(), 1);

    // Cancelled timers never run — the debounce case.
    let h2 = hits.clone();
    let t = set_timeout(10, move || h2.set(h2.get() + 100));
    clear_timeout(t);
    drain_and_fire();
    assert_eq!(hits.get(), 1, "cleared timeout must not fire");

    // clear_timeout after firing is a harmless no-op.
    clear_timeout(t);
}

/// A timer fires outside any document — the shells drain it between
/// events — but it runs as the code of the document that armed it (issue
/// #963). So an interceptor a timer armed in document 1 registers, and one
/// it clears, is document 1's: document 2's own interceptor is untouched
/// by both. Before, the fire ran unmarked, and an unmarked install or clear
/// reaches every document on the thread (#960's slot rule).
#[test]
fn a_timer_registers_and_clears_interceptors_for_the_document_that_armed_it() {
    use crate::context::push_dispatching_doc;
    use crate::events::{
        KeyEventData, clear_keyboard_interceptor, dispatch_keyboard_event, set_keyboard_interceptor,
    };
    use std::cell::RefCell;

    install_manual_backend();
    clear_keyboard_interceptor();
    let hits: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
    let key = KeyEventData::new("x".to_string(), "KeyX".to_string());
    let dispatch_as = |doc: u64| {
        let _d = push_dispatching_doc(doc);
        dispatch_keyboard_event(&key)
    };

    {
        let _b = push_dispatching_doc(2);
        let h = hits.clone();
        set_keyboard_interceptor(move |_| {
            h.borrow_mut().push("doc2");
            true
        });
    }
    {
        let _a = push_dispatching_doc(1);
        let h = hits.clone();
        set_timeout(0, move || {
            set_keyboard_interceptor(move |_| {
                h.borrow_mut().push("doc1-timer");
                true
            })
        });
    }
    drain_and_fire();
    assert!(dispatch_as(1));
    assert!(dispatch_as(2));
    assert_eq!(*hits.borrow(), ["doc1-timer", "doc2"]);

    {
        let _a = push_dispatching_doc(1);
        set_timeout(0, clear_keyboard_interceptor);
    }
    drain_and_fire();
    hits.borrow_mut().clear();
    assert!(!dispatch_as(1), "document 1's timer cleared document 1's");
    assert!(dispatch_as(2), "and left document 2's alone");
    assert_eq!(*hits.borrow(), ["doc2"]);

    clear_keyboard_interceptor();
}

#[test]
fn callback_need_not_be_send_and_may_be_re_armed() {
    install_manual_backend();

    // An `Rc` capture — the whole point of parking main-thread-side.
    let log = Rc::new(std::cell::RefCell::new(Vec::<&'static str>::new()));

    let l = log.clone();
    let first = set_timeout(5, move || l.borrow_mut().push("first"));
    // Debounce: cancel and re-arm before the deadline.
    clear_timeout(first);
    let l = log.clone();
    set_timeout(5, move || l.borrow_mut().push("second"));

    drain_and_fire();
    assert_eq!(*log.borrow(), vec!["second"]);
}

fn take_delays() -> Vec<u32> {
    DELAYS.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

fn intervals_alive() -> usize {
    INTERVALS.with(|m| m.borrow().len())
}

#[test]
fn an_interval_fires_every_period_until_cleared() {
    install_manual_backend();
    let hits = Rc::new(std::cell::Cell::new(0));
    let h = hits.clone();
    let tick = set_interval(10, move || h.set(h.get() + 1));
    for expected in 1..=3 {
        drain_and_fire();
        assert_eq!(hits.get(), expected, "tick {expected} fired and re-armed");
    }
    // (The manual backend fires each tick early, so the delays it saw
    // are not periods; the two rate tests below wait for real.)
    take_delays();

    clear_interval(tick);
    assert_eq!(intervals_alive(), 0);
    drain_and_fire();
    assert_eq!(hits.get(), 3, "a cleared interval does not fire");
    assert!(
        SCHEDULED.with(|s| s.borrow().is_empty()),
        "and nothing re-armed it"
    );
    clear_interval(tick); // idempotent
}

#[test]
fn an_interval_may_clear_itself_from_its_own_callback() {
    install_manual_backend();
    let hits = Rc::new(std::cell::Cell::new(0));
    let me: Rc<std::cell::Cell<Option<IntervalHandle>>> = Rc::new(std::cell::Cell::new(None));
    let (h, m) = (hits.clone(), me.clone());
    let tick = set_interval(5, move || {
        h.set(h.get() + 1);
        if h.get() == 2 {
            clear_interval(m.get().unwrap());
        }
    });
    me.set(Some(tick));
    drain_and_fire();
    drain_and_fire();
    assert_eq!(hits.get(), 2);
    assert!(
        SCHEDULED.with(|s| s.borrow().is_empty()),
        "the tick that cleared it armed no further tick"
    );
    assert_eq!(intervals_alive(), 0);
}

#[test]
fn an_interval_armed_in_a_component_stops_when_it_unmounts() {
    install_manual_backend();
    let hits = Rc::new(std::cell::Cell::new(0));
    let scope = crate::reactive::Scope::new();
    let h = hits.clone();
    scope.run(|| set_interval(10, move || h.set(h.get() + 1)));
    drain_and_fire();
    assert_eq!(hits.get(), 1);
    assert_eq!(intervals_alive(), 1);

    scope.dispose();
    assert_eq!(intervals_alive(), 0, "unmount drops the interval");
    drain_and_fire();
    assert_eq!(hits.get(), 1, "and it does not fire again");
}

/// Fixed rate, not fixed delay: the next tick is scheduled from the last
/// tick's deadline, so a callback that takes 25ms of a 40ms period leaves
/// about 15ms to the next tick (a fixed-delay timer would wait 40).
#[test]
fn a_slow_callback_does_not_push_the_next_tick_back() {
    install_manual_backend();
    set_interval(40, || {
        std::thread::sleep(std::time::Duration::from_millis(25))
    });
    take_delays();
    std::thread::sleep(std::time::Duration::from_millis(40));
    drain_and_fire();
    let next = take_delays();
    assert_eq!(next.len(), 1);
    assert!(
        next[0] < 30,
        "the next tick keeps the rate, got {}ms",
        next[0]
    );
}

/// Between one and two periods late (here ~1.5): the tick it missed is
/// skipped too, not fired at once. Off the fixed points: exactly one or
/// two periods late is where a `<` / `<=` or a `+ period` slip agrees.
#[test]
fn a_tick_one_and_a_half_periods_late_skips_the_missed_tick() {
    install_manual_backend();
    set_interval(40, || {
        std::thread::sleep(std::time::Duration::from_millis(60))
    });
    take_delays();
    std::thread::sleep(std::time::Duration::from_millis(40));
    drain_and_fire();
    let next = take_delays();
    assert_eq!(next.len(), 1);
    assert!(
        next[0] >= 20,
        "the tick due 20ms ago is skipped; the next is a period from now \
         (got {}ms)",
        next[0]
    );
}

/// A tick more than a period late skips the ticks it missed instead of
/// firing them back to back.
#[test]
fn a_tick_more_than_a_period_late_skips_the_missed_ticks() {
    install_manual_backend();
    set_interval(20, || {
        std::thread::sleep(std::time::Duration::from_millis(60))
    });
    take_delays();
    std::thread::sleep(std::time::Duration::from_millis(20));
    drain_and_fire();
    let next = take_delays();
    assert_eq!(next.len(), 1);
    assert!(
        next[0] >= 10,
        "two ticks were missed; the next comes a period from now, not at once \
         (got {}ms)",
        next[0]
    );
}

/// A cleared timer wakes nothing at its deadline (#2832): neither the
/// timer thread nor the main thread. These run on a native scheduler of
/// the test's own (a real thread on the wall clock), counting the thread's
/// deadline wakes and the ids it hands to the main thread; the test plays
/// the main thread, passing each id it is handed to `fire_timeout`.
/// Deadlines are short and the bounds generous: what is pinned is a
/// count, and the order of the deadlines, not their exact time.
#[cfg(not(target_arch = "wasm32"))]
mod cleared_timers_2832 {
    use super::super::native::{Scheduler, install_test_scheduler};
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::Ordering::SeqCst;
    use std::sync::mpsc::Receiver;
    use std::time::{Duration, Instant};

    /// Be the main thread for `dur`: fire each id the scheduler hands
    /// over, and answer them with when each was handed over.
    fn pump(rx: &Receiver<(u64, Instant)>, dur: Duration) -> Vec<(u64, Instant)> {
        pump_until(rx, dur, || false)
    }

    /// [`pump`], stopping early once `done` says so.
    fn pump_until(
        rx: &Receiver<(u64, Instant)>,
        dur: Duration,
        done: impl Fn() -> bool,
    ) -> Vec<(u64, Instant)> {
        let end = Instant::now() + dur;
        let mut posts = Vec::new();
        while !done() {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            if let Ok((id, at)) = rx.recv_timeout(left) {
                fire_timeout(id);
                posts.push((id, at));
            }
        }
        posts
    }

    fn wakes(s: &Scheduler) -> (usize, usize) {
        (s.deadline_wakes.load(SeqCst), s.main_wakes.load(SeqCst))
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Give the timer thread time to take the timer just armed and go to
    /// sleep until its deadline, so a clear after this has a sleeping
    /// thread to re-arm (cleared before the thread ever ran, it would
    /// never have seen the timer, and the test would pin nothing).
    fn let_it_arm() {
        std::thread::sleep(ms(10));
    }

    #[test]
    fn a_cleared_timeout_wakes_nothing_at_its_deadline() {
        let (sched, rx) = install_test_scheduler();
        let hit = Rc::new(Cell::new(false));
        let h = hit.clone();
        let t = set_timeout(40, move || h.set(true));
        let_it_arm();
        clear_timeout(t);
        let posts = pump(&rx, ms(150));
        assert!(posts.is_empty(), "nothing reached the main thread");
        assert_eq!(
            wakes(sched),
            (0, 0),
            "(timer-thread deadline wakes, main-thread wakes)"
        );
        assert!(!hit.get());
    }

    #[test]
    fn clearing_the_earliest_of_two_wakes_once_at_the_second() {
        let (sched, rx) = install_test_scheduler();
        let start = Instant::now();
        let hit = Rc::new(Cell::new(0));
        let (h1, h2) = (hit.clone(), hit.clone());
        let first = set_timeout(30, move || h1.set(h1.get() + 1));
        let second = set_timeout(90, move || h2.set(h2.get() + 10));
        let_it_arm();
        clear_timeout(first);
        let posts = pump(&rx, ms(250));
        assert_eq!(wakes(sched), (1, 1));
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, second.0.raw(), "the wake is the second one's");
        assert!(
            posts[0].1 >= start + ms(90),
            "at the second deadline, not the first ({:?} after start)",
            posts[0].1 - start
        );
        assert_eq!(hit.get(), 10, "and the second one fired");
    }

    #[test]
    fn a_cleared_interval_wakes_nothing_at_its_deadline() {
        let (sched, rx) = install_test_scheduler();
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let tick = set_interval(40, move || h.set(h.get() + 1));
        let_it_arm();
        clear_interval(tick);
        let posts = pump(&rx, ms(150));
        assert!(posts.is_empty());
        assert_eq!(wakes(sched), (0, 0));
        assert_eq!(hits.get(), 0);
    }

    #[test]
    fn clearing_the_earliest_of_two_intervals_wakes_only_for_the_second() {
        let (sched, rx) = install_test_scheduler();
        let start = Instant::now();
        let (a_hits, b_hits) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let (a, b) = (a_hits.clone(), b_hits.clone());
        let ia = set_interval(30, move || a.set(a.get() + 1));
        let ib = set_interval(100, move || b.set(b.get() + 1));
        let_it_arm();
        clear_interval(ia);
        let posts = pump(&rx, ms(160));
        clear_interval(ib);
        let (deadline_wakes, main_wakes) = wakes(sched);
        assert!(!posts.is_empty(), "the second interval ticked");
        assert_eq!(main_wakes, posts.len());
        assert_eq!(
            deadline_wakes, main_wakes,
            "every deadline wake was a tick of the second interval"
        );
        for &(_, at) in &posts {
            assert!(
                at >= start + ms(100),
                "no wake at the cleared interval's deadline ({:?} after start)",
                at - start
            );
        }
        assert_eq!(a_hits.get(), 0);
        assert_eq!(b_hits.get(), posts.len());
    }

    #[test]
    fn a_timeout_cleared_from_another_timers_callback_wakes_nothing() {
        let (sched, rx) = install_test_scheduler();
        let later: Rc<Cell<Option<TimeoutHandle>>> = Rc::new(Cell::new(None));
        let hit = Rc::new(Cell::new(false));
        let l = later.clone();
        let first = set_timeout(30, move || clear_timeout(l.get().unwrap()));
        let h = hit.clone();
        later.set(Some(set_timeout(90, move || h.set(true))));
        let posts = pump(&rx, ms(250));
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, first.0.raw());
        assert_eq!(wakes(sched), (1, 1), "only the first one's deadline woke");
        assert!(!hit.get(), "the cleared one did not fire");
    }

    /// After the clear the thread sleeps with no deadline at all; a timer
    /// armed then must still wake it, and at its own (earlier) deadline,
    /// not the cleared one's — and the cleared one's deadline, passed
    /// later, still wakes nothing.
    #[test]
    fn a_sooner_timeout_set_after_a_clear_fires_on_time() {
        let (sched, rx) = install_test_scheduler();
        let start = Instant::now();
        let cleared = set_timeout(300, || panic!("cleared"));
        let_it_arm();
        clear_timeout(cleared);
        let fired: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let f = fired.clone();
        set_timeout(40, move || f.set(Some(Instant::now())));
        let posts = pump(&rx, ms(450));
        let at = fired.get().expect("the new timeout fired");
        assert!(at >= start + ms(40), "not early: {:?}", at - start);
        assert!(
            at < start + ms(250),
            "on time, not at the cleared deadline: {:?}",
            at - start
        );
        assert_eq!(posts.len(), 1, "nothing at the cleared deadline");
        assert_eq!(wakes(sched), (1, 1));
    }

    /// Two timers on one deadline (one `Instant`, so the set orders them by
    /// id alone): both fire, and clearing one leaves the other. A
    /// scheduler keyed by deadline alone would keep only one of them, and
    /// a cancel that matched by deadline would take both.
    #[test]
    fn timers_sharing_a_deadline_fire_and_clear_independently() {
        let (sched, rx) = install_test_scheduler();
        let hits = Rc::new(Cell::new(0));
        let (h1, h2) = (hits.clone(), hits.clone());
        // Through the scheduler directly, so both get the very same
        // deadline rather than two `Instant::now()`s.
        let a = park_main_callback::<()>(move |()| h1.set(h1.get() + 1));
        let b = park_main_callback::<()>(move |()| h2.set(h2.get() + 10));
        sched.schedule_at_for_test(a.raw(), b.raw(), ms(40));
        let posts = pump(&rx, ms(150));
        assert_eq!(hits.get(), 11, "both fired");
        assert_eq!(posts.len(), 2);
        assert_eq!(wakes(sched), (1, 2), "one deadline wake for both");

        let (h3, h4) = (hits.clone(), hits.clone());
        let c = park_main_callback::<()>(move |()| h3.set(h3.get() + 100));
        let d = park_main_callback::<()>(move |()| h4.set(h4.get() + 1000));
        sched.schedule_at_for_test(c.raw(), d.raw(), ms(40));
        let_it_arm();
        clear_timeout(TimeoutHandle(c));
        let posts = pump(&rx, ms(150));
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, d.raw(), "the other one is still there");
        assert_eq!(hits.get(), 1011, "and it fired; the cleared one did not");
        assert_eq!(wakes(sched), (2, 3));
    }

    /// The test harness's virtual clock drops a cleared timer too, so an
    /// advance has no deadline of it to stop at.
    #[test]
    fn a_cleared_timeout_leaves_the_virtual_clock() {
        virtual_time::install();
        let first = set_timeout(50, || panic!("cleared"));
        let second = set_timeout(80, || {});
        clear_timeout(first);
        let due = virtual_time::take_due(Duration::from_millis(100), u64::MAX);
        assert_eq!(due, Some(second.0.raw()), "the cleared one is not due");
        assert_eq!(
            virtual_time::take_due(Duration::from_millis(100), u64::MAX),
            None
        );
        virtual_time::uninstall();
    }
}

/// Native always runs a timeout: the built-in scheduler stands in for a
/// missing backend, so code that defers a check may rely on it here.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn timers_fire_on_native() {
    assert!(timers_fire());
}
