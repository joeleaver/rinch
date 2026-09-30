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
    /// itself throw — the caller settles what it leaves.
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
/// Logs nothing itself: [`jni_ok`] and a `jni_try` caller report the failure
/// they settled, and [`ExceptionScope`] reports one that reached it.
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
/// cannot leak into the next call, and the text is given up.
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
    let mut text = env.throwable_text(&throwable);
    if env.exception_pending() {
        env.clear_exception();
        text = None;
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
