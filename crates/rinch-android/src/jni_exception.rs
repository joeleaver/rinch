//! One place that settles a Java exception a JNI call left pending (issue #419).
//!
//! **Why this exists.** `jni` 0.21 turns a pending Java exception into
//! `Err(Error::JavaException)` — or, for a failed method lookup,
//! `Err(MethodNotFound)` — and **leaves the exception pending**: its
//! `check_exception!` macro returns early without calling `ExceptionClear`, and
//! nothing else in the crate calls it. A pending exception makes the *next* JNI
//! call on the thread illegal. Under CheckJNI (every debuggable build, and many
//! devices) ART aborts the process at that next call; without it the behaviour
//! is undefined. Either way the crash lands on the next call, which belongs to
//! somebody else — an unrelated wrapper, the frame loop — and is blamed on
//! them. A wrapper that logs a failure and carries on has therefore not handled
//! it: it has handed it to a stranger.
//!
//! So every failure is **settled where it happens**: the Java stack trace is
//! printed (`ExceptionDescribe`, to logcat, the only place it would otherwise
//! ever appear) and the exception is cleared. Three entry points, from most to
//! least specific:
//!
//! - [`jni_try`] runs one JNI step and settles on `Err`, handing the error on
//!   as a `String` for the caller to `?` or report. When the failure was a
//!   Java exception the string ends with its `toString()` — the class and
//!   message, e.g. `java.lang.SecurityException: Permission Denial: …` —
//!   which `jni`'s own error ("Java exception was thrown") never says
//!   (issue #1205).
//! - [`jni_ok`] is `jni_try` for a step whose failure the caller swallows: it
//!   also logs the failure and answers `None`.
//! - [`ExceptionScope`] settles whatever is still pending when it is dropped.
//!   `bridge::with_jni_env` / `with_activity` hold one around every closure,
//!   and each `extern "C"` entry point Java calls holds one around its body,
//!   so a site that forgets the two above — or a `?` that leaves a closure
//!   early — still cannot leak an exception past the scope that caused it.
//!
//! The scope is the net, not the rule: it runs only when the scope ends, so a
//! closure that swallows one failure and then makes *another* JNI call would
//! still make it with the exception pending. That is what `jni_try`/`jni_ok`
//! are for. In this crate a JNI call either goes through one of them or
//! carries its error straight out of its scope with `?` (`display`'s
//! `night_mode`, `wallpaper_primary`, `system_accent`), where the scope settles
//! it before anything else runs; `bridge::init` panics on a failure instead,
//! and `DeleteLocalRef` cannot throw.
//!
//! The same bodies also run in a JNI local frame ([`LocalFrame`], issue
//! #1217), popped when they return, so no local reference a body made — nor
//! one made settling its exception — outlives it on a thread that never
//! returns to Java.
//!
//! The logic is generic over [`PendingException`] so it is unit-tested on the
//! host against a recording fake; the one real implementation, for
//! `jni::JNIEnv`, is Android-only because `jni` is.

use std::fmt::Display;

/// The JNI functions settling an exception needs. All but
/// [`throwable_text`](Self::throwable_text) are legal while one is pending.
pub(crate) trait PendingException {
    /// A local reference to a thrown `java.lang.Throwable`.
    type Throwable;
    /// `ExceptionCheck`. An error from the check itself reads as "nothing
    /// pending": there is nothing more useful to do with it.
    fn exception_pending(&mut self) -> bool;
    /// `ExceptionOccurred`: a new local reference to the pending exception,
    /// or `None` when there is none.
    fn take_throwable(&mut self) -> Option<Self::Throwable>;
    /// `ExceptionDescribe`: print the exception and its Java stack trace.
    /// The JNI specification says it also clears the exception.
    fn describe_exception(&mut self);
    /// `ExceptionClear`.
    fn clear_exception(&mut self);
    /// `throwable.toString()`: the class name and message. A Java method
    /// call, so **illegal while any exception is pending**, and it may
    /// itself throw: then it answers `None`, makes no further call, and the
    /// caller settles what it leaves.
    fn throwable_text(&mut self, throwable: &Self::Throwable) -> Option<String>;
    /// `DeleteLocalRef` on a reference [`take_throwable`](Self::take_throwable)
    /// answered.
    fn release_throwable(&mut self, throwable: Self::Throwable);
}

/// Describe and clear a pending exception, if there is one. Answers whether
/// there was.
///
/// Describe comes first because it needs the exception to print, and clear
/// comes after it unconditionally: the JNI specification says describe clears
/// as a side effect, but that is one more thing to rely on for no saving.
///
/// Logs nothing itself: [`ExceptionScope`] reports one that reached it.
/// [`jni_try`] settles through [`settle_pending_with_text`] instead, which
/// also reads the exception's text for the error it hands on.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn settle_pending<E: PendingException + ?Sized>(env: &mut E) -> bool {
    if !env.exception_pending() {
        return false;
    }
    env.describe_exception();
    env.clear_exception();
    true
}

/// Run one JNI step; on failure settle any exception it left pending before
/// answering the error as a `String` prefixed with `context` and, for a Java
/// exception, followed by that exception's `toString()`
/// ([`settle_pending_with_text`]).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn jni_try<E, T, Er>(
    env: &mut E,
    context: &str,
    step: impl FnOnce(&mut E) -> Result<T, Er>,
) -> Result<T, String>
where
    E: PendingException + ?Sized,
    Er: Display,
{
    step(env).map_err(|e| match settle_pending_with_text(env) {
        Some(text) => format!("{context}: {e}: {text}"),
        None => format!("{context}: {e}"),
    })
}

/// [`settle_pending`], and answer the settled exception's `toString()` —
/// its class and message — so a caller's error can say which exception it
/// was (issue #1205). `None` when nothing was pending, or when the text
/// could not be had.
///
/// The order is forced. `ExceptionOccurred` must come before describe,
/// which clears the exception as it prints it. `toString` is a Java call,
/// illegal while the exception is pending, so it must come after the
/// clear. And it may throw in turn: that exception is cleared at once —
/// the stack trace printed is the one the caller's failure threw — so it
/// cannot leak into the next call, and there is no text.
pub(crate) fn settle_pending_with_text<E: PendingException + ?Sized>(
    env: &mut E,
) -> Option<String> {
    if !env.exception_pending() {
        return None;
    }
    let throwable = env.take_throwable();
    env.describe_exception();
    env.clear_exception();
    let throwable = throwable?;
    let text = env.throwable_text(&throwable);
    if env.exception_pending() {
        env.clear_exception();
    }
    env.release_throwable(throwable);
    text
}

/// [`jni_try`] for a step whose failure the caller gives up on: the failure
/// is logged, any pending exception settled, and the answer is `None`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn jni_ok<E, T, Er>(
    env: &mut E,
    context: &str,
    step: impl FnOnce(&mut E) -> Result<T, Er>,
) -> Option<T>
where
    E: PendingException + ?Sized,
    Er: Display,
{
    match jni_try(env, context, step) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("{e}");
            None
        }
    }
}

/// Settles whatever exception is pending when it is dropped — the net under
/// [`jni_try`]/[`jni_ok`]. See the module doc.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
///
/// `context` names the scope in the log line — an entry point's name, or the
/// source location of a `bridge::with_activity` call.
pub(crate) struct ExceptionScope<E: PendingException, C: Display = &'static str> {
    env: E,
    context: C,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
impl<E: PendingException, C: Display> ExceptionScope<E, C> {
    pub(crate) fn new(env: E, context: C) -> Self {
        Self { env, context }
    }
}

impl<E: PendingException, C: Display> Drop for ExceptionScope<E, C> {
    fn drop(&mut self) {
        if settle_pending(&mut self.env) {
            // Reaching here means a failure left its scope unsettled: a `?`
            // carried it out, or a site skipped `jni_ok`/`jni_try`.
            log::warn!(
                "{}: cleared a Java exception left pending (stack trace above)",
                self.context
            );
        }
    }
}

#[cfg(target_os = "android")]
impl<'local> PendingException for jni::JNIEnv<'local> {
    type Throwable = jni::objects::JThrowable<'local>;
    fn exception_pending(&mut self) -> bool {
        self.exception_check().unwrap_or(false)
    }
    fn take_throwable(&mut self) -> Option<Self::Throwable> {
        self.exception_occurred().ok().filter(|t| !t.is_null())
    }
    fn describe_exception(&mut self) {
        let _ = self.exception_describe();
    }
    fn clear_exception(&mut self) {
        let _ = self.exception_clear();
    }
    fn throwable_text(&mut self, throwable: &Self::Throwable) -> Option<String> {
        // Each `?` leaves on an error, which may be a pending exception:
        // no further call is made here, and the caller settles it.
        let obj = self
            .call_method(throwable, "toString", "()Ljava/lang/String;", &[])
            .ok()?
            .l()
            .ok()?;
        if obj.is_null() {
            return None;
        }
        let jstr = jni::objects::JString::from(obj);
        let text = self.get_string(&jstr).ok().map(String::from);
        // DeleteLocalRef is legal with an exception pending.
        let _ = self.delete_local_ref(jstr);
        text
    }
    fn release_throwable(&mut self, throwable: Self::Throwable) {
        let _ = self.delete_local_ref(throwable);
    }
}

/// The JNI functions a local reference frame needs (issue #1217).
///
/// **Why a frame.** A local reference lives until the native method that made
/// it returns to Java — and the `android_main` thread never returns:
/// android-activity attaches it once, for good, so every local made on it
/// through [`bridge::with_jni_env`](crate::bridge::with_jni_env) used to live
/// for the life of the app. Not only the ones this crate makes and forgets
/// (`new_string`, a `call_method` result): `jni` 0.21's own `get_string` makes
/// two (`FindClass("java/lang/String")` and `GetObjectClass`, for its type
/// check) and deletes neither. So every `clipboard::paste_text` leaked two, and
/// so did every Java exception `jni_try` reads the text of. A frame pushed
/// around each body and popped after it frees all of them at once, whichever
/// call made them.
pub(crate) trait LocalFrames: PendingException {
    /// `PushLocalFrame`. Answers whether a frame was pushed; a failure
    /// (out of memory) throws `OutOfMemoryError` and pushes nothing.
    fn enter_local_frame(&mut self, capacity: i32) -> bool;
    /// `PopLocalFrame(NULL)`: free every local reference made since the
    /// matching push. Legal while an exception is pending.
    fn leave_local_frame(&mut self);
}

/// The capacity each body's frame asks for. Only a floor: ART grows a frame
/// past it on demand, so a body that makes more locals is not refused.
pub(crate) const BODY_FRAME_CAPACITY: i32 = 16;

/// A local reference frame, popped when dropped — on a panic too, which
/// `jni`'s own `with_local_frame` does not do.
pub(crate) struct LocalFrame<E: LocalFrames> {
    /// `None` when the push failed: there is no frame to pop.
    env: Option<E>,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
impl<E: LocalFrames> LocalFrame<E> {
    pub(crate) fn push(mut env: E, capacity: i32) -> Self {
        if env.enter_local_frame(capacity) {
            return Self { env: Some(env) };
        }
        // The failed push left an `OutOfMemoryError` pending, which would
        // abort the body's first call. Settle it and run the body without a
        // frame: its locals then live as they did before #1217.
        settle_pending(&mut env);
        log::warn!("PushLocalFrame failed; running without a local frame");
        Self { env: None }
    }
}

impl<E: LocalFrames> Drop for LocalFrame<E> {
    fn drop(&mut self) {
        if let Some(env) = self.env.as_mut() {
            env.leave_local_frame();
        }
    }
}

/// Run `f` inside a [`LocalFrame`] and an [`ExceptionScope`] — the body of
/// [`bridge::with_jni_env`](crate::bridge::with_jni_env). `frame` and `scope`
/// are further handles to `env`'s thread.
///
/// The scope is declared after the frame, so it drops first: whatever it
/// does to settle an exception happens inside the frame, and any local it
/// makes goes with the frame.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn run_framed<E: LocalFrames, C: Display, R>(
    env: &mut E,
    frame: E,
    scope: E,
    context: C,
    f: impl FnOnce(&mut E) -> R,
) -> R {
    let _frame = LocalFrame::push(frame, BODY_FRAME_CAPACITY);
    let _scope = ExceptionScope::new(scope, context);
    f(env)
}

#[cfg(target_os = "android")]
impl LocalFrames for jni::JNIEnv<'_> {
    fn enter_local_frame(&mut self, capacity: i32) -> bool {
        jni::JNIEnv::push_local_frame(self, capacity).is_ok()
    }
    fn leave_local_frame(&mut self) {
        // SAFETY: pops the frame `enter_local_frame` pushed on this thread,
        // keeping no result. Every local made in it is dead after this, and
        // none can be named: `with_jni_env`'s `R` cannot borrow the env.
        let _ = unsafe { jni::JNIEnv::pop_local_frame(self, &jni::objects::JObject::null()) };
    }
}

/// An [`ExceptionScope`] over `env`'s thread, for a function that is handed a
/// `JNIEnv` — an `extern "C"` entry point Java calls — rather than taking one
/// through `bridge`.
///
/// Hold it for the whole body, declared first (`let _scope = …;` — never `_`,
/// which drops at once). A Rust function returning with an exception pending
/// throws it into its Java caller, which for a callback on the UI thread is an
/// uncaught exception; every entry point here gives up on a failure instead,
/// so none of them means to.
#[cfg(target_os = "android")]
pub(crate) fn native_scope<'local, C: Display>(
    env: &jni::JNIEnv<'local>,
    context: C,
) -> ExceptionScope<jni::JNIEnv<'local>, C> {
    // SAFETY: the clone is a second handle to the same thread's `JNIEnv`
    // pointer. It is used only from this thread (the scope is not `Send`,
    // because `JNIEnv` is not) and only in `Drop`, by which point every
    // borrow of the original that could be mid-call has ended.
    ExceptionScope::new(unsafe { env.unsafe_clone() }, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// What the fake saw, in order, and whether an exception is pending.
    #[derive(Default)]
    struct State {
        pending: bool,
        calls: Vec<&'static str>,
        /// What the pending exception's `toString()` answers.
        text: Option<&'static str>,
        /// Whether `toString()` itself throws.
        to_string_throws: bool,
        /// Local references `take_throwable` made and nobody released.
        live_refs: usize,
        /// Every other local reference alive on the thread (issue #1217):
        /// the ones a body makes and never deletes, as `jni`'s
        /// `get_string` does.
        locals: usize,
        /// The `locals` count at each live `PushLocalFrame`, innermost last.
        frames: Vec<usize>,
        /// Whether `PushLocalFrame` fails (and throws).
        push_fails: bool,
    }

    /// A `JNIEnv` stand-in that records every call. Cloneable (the state is
    /// shared) so a scope can hold one while the test inspects another, which
    /// is how the real scope holds an `unsafe_clone`.
    #[derive(Clone, Default)]
    struct FakeEnv(Rc<RefCell<State>>);

    impl FakeEnv {
        /// A JNI call: illegal with an exception pending, which is the whole
        /// bug — the fake panics where ART's CheckJNI would abort.
        fn call(&mut self, name: &'static str, throws: bool) -> Result<(), &'static str> {
            let mut s = self.0.borrow_mut();
            assert!(
                !s.pending,
                "JNI call `{name}` made with an exception pending (CheckJNI would abort)"
            );
            s.calls.push(name);
            if throws {
                s.pending = true;
                Err("java exception")
            } else {
                Ok(())
            }
        }
        /// A JNI call that throws an exception whose `toString()` is `text`.
        fn throw(&mut self, name: &'static str, text: &'static str) -> Result<(), &'static str> {
            self.0.borrow_mut().text = Some(text);
            self.call(name, true)
        }
        /// A JNI call that makes `n` local references and deletes none —
        /// `get_string`'s `FindClass` and `GetObjectClass` are `make_locals(2)`.
        fn make_locals(&mut self, n: usize) {
            self.call("makeLocals", false).unwrap();
            self.0.borrow_mut().locals += n;
        }
        fn locals(&self) -> usize {
            self.0.borrow().locals
        }
        fn calls(&self) -> Vec<&'static str> {
            self.0.borrow().calls.clone()
        }
        fn pending(&self) -> bool {
            self.0.borrow().pending
        }
    }

    impl PendingException for FakeEnv {
        type Throwable = &'static str;
        fn exception_pending(&mut self) -> bool {
            self.0.borrow_mut().calls.push("ExceptionCheck");
            self.0.borrow().pending
        }
        fn take_throwable(&mut self) -> Option<&'static str> {
            let mut s = self.0.borrow_mut();
            s.calls.push("ExceptionOccurred");
            // ExceptionOccurred answers null once nothing is pending — which
            // is what it answers after a describe, since describe clears.
            if !s.pending {
                return None;
            }
            s.live_refs += 1;
            Some(s.text.unwrap_or("java.lang.RuntimeException"))
        }
        fn describe_exception(&mut self) {
            let mut s = self.0.borrow_mut();
            // Describe needs the exception it prints...
            assert!(s.pending, "ExceptionDescribe with nothing pending");
            s.calls.push("ExceptionDescribe");
            // ...and, per the JNI specification, clears it as it prints it.
            s.pending = false;
        }
        fn clear_exception(&mut self) {
            let mut s = self.0.borrow_mut();
            s.calls.push("ExceptionClear");
            s.pending = false;
        }
        fn throwable_text(&mut self, throwable: &&'static str) -> Option<String> {
            let mut s = self.0.borrow_mut();
            assert!(
                !s.pending,
                "toString() called with an exception pending (CheckJNI would abort)"
            );
            s.calls.push("toString");
            // The real one reads the answer with `jni`'s `get_string`, whose
            // type check leaves two locals behind (issue #1217).
            s.locals += 2;
            if s.to_string_throws {
                s.pending = true;
                return None;
            }
            Some(throwable.to_string())
        }
        fn release_throwable(&mut self, _: &'static str) {
            let mut s = self.0.borrow_mut();
            s.calls.push("DeleteLocalRef");
            s.live_refs -= 1;
        }
    }

    impl LocalFrames for FakeEnv {
        fn enter_local_frame(&mut self, _capacity: i32) -> bool {
            let mut s = self.0.borrow_mut();
            s.calls.push("PushLocalFrame");
            if s.push_fails {
                s.pending = true;
                return false;
            }
            let mark = s.locals;
            s.frames.push(mark);
            true
        }
        fn leave_local_frame(&mut self) {
            let mut s = self.0.borrow_mut();
            s.calls.push("PopLocalFrame");
            let mark = s.frames.pop().expect("PopLocalFrame with no frame pushed");
            s.locals = mark;
        }
    }

    /// `bridge::with_jni_env`'s body, over the fake.
    fn with_env<R>(env: &FakeEnv, f: impl FnOnce(&mut FakeEnv) -> R) -> R {
        run_framed(&mut env.clone(), env.clone(), env.clone(), "test", f)
    }

    #[test]
    fn a_body_releases_every_local_it_made() {
        // Issue #1217: the `android_main` thread never returns to Java, so a
        // local made on it lives until something deletes it. 1000 pastes,
        // each a `get_string` leaving two behind.
        let env = FakeEnv::default();
        for _ in 0..1000 {
            with_env(&env, |e| e.make_locals(2));
        }
        assert_eq!(env.locals(), 0, "the bodies' locals leaked");
        assert_eq!(env.0.borrow().frames.len(), 0, "a frame was left pushed");
    }

    #[test]
    fn locals_made_while_settling_a_failure_are_released_too() {
        // `jni_try` reads the exception's text with `toString` and
        // `get_string`, whose two locals it cannot delete.
        let env = FakeEnv::default();
        let r: Result<(), String> = with_env(&env, |e| {
            jni_try(e, "readContentUri", |e| e.throw("readContentUri", "x"))?;
            Ok(())
        });
        assert!(r.is_err());
        assert_eq!(env.locals(), 0, "toString's locals leaked");
        let calls = env.calls();
        assert_eq!(calls.first(), Some(&"PushLocalFrame"));
        assert_eq!(calls.last(), Some(&"PopLocalFrame"));
        assert!(!env.pending());
    }

    #[test]
    fn a_panicking_body_still_pops_its_frame() {
        let env = FakeEnv::default();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_env(&env, |e| {
                e.make_locals(2);
                panic!("body panicked");
            })
        }));
        assert!(r.is_err());
        assert_eq!(env.locals(), 0);
        assert_eq!(env.0.borrow().frames.len(), 0);
    }

    #[test]
    fn nested_bodies_pop_their_own_frames() {
        let env = FakeEnv::default();
        with_env(&env, |e| {
            e.make_locals(3);
            with_env(e, |e| e.make_locals(5));
            assert_eq!(e.locals(), 3, "the inner frame freed only its own");
        });
        assert_eq!(env.locals(), 0);
    }

    #[test]
    fn a_failed_push_is_settled_and_the_body_runs_without_a_frame() {
        let env = FakeEnv::default();
        env.0.borrow_mut().push_fails = true;
        // The fake panics on a call made with an exception pending, which
        // is what the push's `OutOfMemoryError` would be left as.
        with_env(&env, |e| e.make_locals(2));
        assert!(!env.pending());
        assert_eq!(env.locals(), 2, "no frame, so nothing was freed");
        assert!(
            !env.calls().contains(&"PopLocalFrame"),
            "popped a frame that was never pushed"
        );
    }

    #[test]
    fn a_swallowed_failure_leaves_nothing_pending_for_the_next_call() {
        let mut env = FakeEnv::default();
        // The #419 shape: one wrapper gives up on a throwing call...
        let first = jni_ok(&mut env, "startSensor", |e| e.call("startSensor", true));
        assert_eq!(first, None);
        // ...and the next, unrelated call must not find it pending.
        let second = jni_ok(&mut env, "getResources", |e| e.call("getResources", false));
        assert_eq!(second, Some(()));
        assert_eq!(
            env.calls(),
            [
                "startSensor",
                "ExceptionCheck",
                "ExceptionOccurred",
                "ExceptionDescribe",
                "ExceptionClear",
                "toString",
                "ExceptionCheck",
                "DeleteLocalRef",
                "getResources",
            ]
        );
    }

    #[test]
    fn jni_try_names_the_exception_in_its_error() {
        // Issue #1205: the app's `Err` must say which exception was thrown
        // and why, not only jni's "Java exception was thrown".
        let mut env = FakeEnv::default();
        let r: Result<(), String> = jni_try(&mut env, "readContentUri", |e| {
            e.throw(
                "readContentUri",
                "java.lang.SecurityException: Permission Denial: reading uri",
            )
        });
        assert_eq!(
            r,
            Err("readContentUri: java exception: \
                 java.lang.SecurityException: Permission Denial: reading uri"
                .to_string())
        );
        assert!(!env.pending());
    }

    #[test]
    fn jni_try_settles_then_hands_the_error_on() {
        let mut env = FakeEnv::default();
        let r: Result<(), String> = jni_try(&mut env, "readContentUri", |e| e.call("x", true));
        assert_eq!(
            r,
            Err("readContentUri: java exception: java.lang.RuntimeException".to_string())
        );
        assert!(!env.pending());
    }

    #[test]
    fn the_throwable_is_taken_before_describe_and_read_after_the_clear() {
        // Issue #1205's ordering. `ExceptionOccurred` before describe, which
        // clears as it prints (so the fake would answer null after it);
        // `toString` after the clear, since a Java call with an exception
        // pending is illegal (the fake panics); the reference released.
        let mut env = FakeEnv::default();
        let r: Result<(), String> = jni_try(&mut env, "copyToClipboard", |e| {
            e.throw("copyToClipboard", "java.lang.IllegalStateException: gone")
        });
        assert_eq!(
            r,
            Err("copyToClipboard: java exception: java.lang.IllegalStateException: gone".into())
        );
        assert_eq!(
            env.calls(),
            [
                "copyToClipboard",
                "ExceptionCheck",
                "ExceptionOccurred",
                "ExceptionDescribe",
                "ExceptionClear",
                "toString",
                "ExceptionCheck",
                "DeleteLocalRef",
            ]
        );
        assert_eq!(
            env.0.borrow().live_refs,
            0,
            "the throwable's local ref leaked"
        );
    }

    #[test]
    fn a_to_string_that_throws_is_settled_and_the_text_given_up() {
        let mut env = FakeEnv::default();
        env.0.borrow_mut().to_string_throws = true;
        let r: Result<(), String> = jni_try(&mut env, "readImageUri", |e| {
            e.throw("readImageUri", "java.io.FileNotFoundException: x")
        });
        assert_eq!(r, Err("readImageUri: java exception".into()));
        assert!(!env.pending(), "toString's own exception must not leak");
        assert_eq!(env.0.borrow().live_refs, 0);
        // And the thread is usable: the fake panics on a call made pending.
        assert_eq!(
            jni_ok(&mut env, "next", |e| e.call("next", false)),
            Some(())
        );
        assert_eq!(
            env.calls(),
            [
                "readImageUri",
                "ExceptionCheck",
                "ExceptionOccurred",
                "ExceptionDescribe",
                "ExceptionClear",
                "toString",
                "ExceptionCheck",
                "ExceptionClear",
                "DeleteLocalRef",
                "next",
            ]
        );
    }

    #[test]
    fn a_success_touches_no_exception_function() {
        let mut env = FakeEnv::default();
        assert_eq!(jni_ok(&mut env, "ok", |e| e.call("ok", false)), Some(()));
        assert_eq!(env.calls(), ["ok"]);
    }

    #[test]
    fn a_failure_with_nothing_pending_neither_describes_nor_clears() {
        // A `jni` error that is not a Java exception (a bad signature, a null
        // argument, a wrong return type) leaves nothing to settle.
        let mut env = FakeEnv::default();
        let r: Option<()> = jni_ok(&mut env, "bad sig", |_| Err("InvalidArgList"));
        assert_eq!(r, None);
        assert_eq!(env.calls(), ["ExceptionCheck"]);
    }

    #[test]
    fn the_scope_settles_what_a_question_mark_left_pending() {
        let env = FakeEnv::default();
        let run = |mut e: FakeEnv| -> Result<(), &'static str> {
            let _scope = ExceptionScope::new(e.clone(), "clipboard");
            e.call("pasteFromClipboard", true)?;
            e.call("never reached", false)
        };
        assert!(run(env.clone()).is_err());
        assert!(!env.pending(), "the scope must clear on the way out");
        assert_eq!(
            env.calls(),
            [
                "pasteFromClipboard",
                "ExceptionCheck",
                "ExceptionDescribe",
                "ExceptionClear",
            ]
        );
    }

    #[test]
    fn the_scope_leaves_a_clean_thread_alone() {
        let env = FakeEnv::default();
        drop(ExceptionScope::new(env.clone(), "clean"));
        assert_eq!(env.calls(), ["ExceptionCheck"]);
    }
}
