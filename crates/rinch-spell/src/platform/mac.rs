//! macOS: `NSSpellChecker`, every call on the main thread.
//!
//! Chromium calls `NSSpellChecker` on the browser UI thread (the main thread
//! on macOS): `SpellCheckHostChromeImpl::CheckSpelling` and
//! `FillSuggestionList` in `chrome/browser/spellchecker/
//! spell_check_host_chrome_impl.cc`, whose other entry points assert
//! `CHECK_CURRENTLY_ON(BrowserThread::UI)`, reaching
//! `components/spellcheck/browser/spellcheck_platform_mac.mm`. rinch does the
//! same through [`on_main`]: at once on the main thread, else queued on the
//! main dispatch queue with a bounded wait.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2_app_kit::NSSpellChecker;
use objc2_foundation::{NSArray, NSRange, NSString};

use super::{
    Native, PlatformOptions, from_mac_language, interleave_suggestions, pad_answers,
    to_mac_language,
};

/// Runs `f` on the main thread and returns its answer: at once when called
/// there, else through the main dispatch queue, waiting at most `wait`
/// ([`PlatformOptions::main_thread_wait`]). `None` when the main thread did not run it in time
/// (it may still run later; its answer is then dropped) or it panicked.
fn on_main<R: Send + 'static>(wait: Duration, f: impl FnOnce() -> R + Send + 'static) -> Option<R> {
    if MainThreadMarker::new().is_some() {
        return catch_unwind(AssertUnwindSafe(f)).ok();
    }
    let (tx, rx) = mpsc::sync_channel(1);
    DispatchQueue::main().exec_async(move || {
        // A panic must not unwind into libdispatch.
        if let Ok(answer) = catch_unwind(AssertUnwindSafe(f)) {
            let _ = tx.send(answer);
        }
    });
    rx.recv_timeout(wait).ok()
}

/// What the main-thread side needs; shared with each queued call.
struct State {
    /// The app's languages in rinch's form; `None` for the system's.
    wanted: Option<Vec<String>>,
    /// The macOS codes to check with, once resolved on the main thread:
    /// `None` inside for the system's own choice (`language: nil`, with its
    /// automatic language identification).
    resolved: Mutex<Option<Option<Vec<String>>>>,
    /// This checker's spell document tag (0 until the first call).
    tag: AtomicIsize,
    options: PlatformOptions,
}

impl State {
    /// Main thread only.
    fn checker(&self) -> objc2::rc::Retained<NSSpellChecker> {
        NSSpellChecker::sharedSpellChecker()
    }

    /// Main thread only.
    fn tag(&self) -> isize {
        let tag = self.tag.load(Ordering::Acquire);
        if tag != 0 {
            return tag;
        }
        let tag = NSSpellChecker::uniqueSpellDocumentTag();
        self.tag.store(tag, Ordering::Release);
        tag
    }

    /// The macOS codes to check with (main thread only): the app's languages
    /// the system has, or `None` for the system's choice.
    fn codes(&self) -> Option<Vec<String>> {
        let mut resolved = self.resolved.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(codes) = &*resolved {
            return codes.clone();
        }
        let codes = self.wanted.as_ref().map(|wanted| {
            let available: Vec<String> = self
                .checker()
                .availableLanguages()
                .iter()
                .map(|s| s.to_string())
                .collect();
            let mut codes: Vec<String> = Vec::new();
            for tag in wanted {
                let Some(code) = to_mac_language(tag) else {
                    continue;
                };
                // `de` asks for any German macOS has (`de`, `de_CH`, …).
                let found = available.iter().find(|a| **a == code).or_else(|| {
                    available
                        .iter()
                        .find(|a| a.split('_').next() == Some(&code))
                });
                if let Some(found) = found
                    && !codes.contains(found)
                {
                    codes.push(found.clone());
                }
            }
            codes
        });
        *resolved = Some(codes.clone());
        codes
    }

    /// Main thread only.
    fn check(&self, word: &str) -> bool {
        let checker = self.checker();
        let ns = NSString::from_str(word);
        let tag = self.tag();
        let correct = |language: Option<&NSString>| {
            // SAFETY: a null `wordCount` is allowed.
            let range = unsafe {
                checker.checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(
                    &ns,
                    0,
                    language,
                    false,
                    tag,
                    ptr::null_mut(),
                )
            };
            range.length == 0
        };
        match self.codes() {
            None => correct(None),
            Some(codes) => {
                // None of the app's languages installed: nothing to judge by.
                codes.is_empty()
                    || codes
                        .iter()
                        .any(|code| correct(Some(&NSString::from_str(code))))
            }
        }
    }
}

pub(crate) struct MacSpeller {
    state: Arc<State>,
}

impl MacSpeller {
    /// Asks nothing of AppKit (the checking thread builds it, and the main
    /// thread may be waiting for that thread just then).
    pub(crate) fn new(languages: Option<Vec<String>>, options: PlatformOptions) -> Self {
        Self {
            state: Arc::new(State {
                wanted: languages,
                resolved: Mutex::new(None),
                tag: AtomicIsize::new(0),
                options,
            }),
        }
    }

    fn run<R: Send + 'static>(&self, f: impl FnOnce(&State) -> R + Send + 'static) -> Option<R> {
        let state = Arc::clone(&self.state);
        on_main(self.state.options.main_thread_wait, move || f(&state))
    }
}

impl Drop for MacSpeller {
    fn drop(&mut self) {
        let tag = self.state.tag.load(Ordering::Acquire);
        if tag == 0 {
            return;
        }
        let close = move || NSSpellChecker::sharedSpellChecker().closeSpellDocumentWithTag(tag);
        if MainThreadMarker::new().is_some() {
            close();
        } else {
            DispatchQueue::main().exec_async(close);
        }
    }
}

impl Native for MacSpeller {
    fn languages(&self) -> Option<Vec<String>> {
        self.run(|state| match state.codes() {
            Some(codes) => codes.iter().filter_map(|c| from_mac_language(c)).collect(),
            None => {
                let checker = state.checker();
                let codes: Vec<String> = if checker.automaticallyIdentifiesLanguages() {
                    checker
                        .userPreferredLanguages()
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                } else {
                    vec![checker.language().to_string()]
                };
                let mut out: Vec<String> = Vec::new();
                for tag in codes.iter().filter_map(|c| from_mac_language(c)) {
                    if !out.contains(&tag) {
                        out.push(tag);
                    }
                }
                out
            }
        })
    }

    fn check_words(&self, words: Vec<String>) -> Vec<bool> {
        let len = words.len();
        let words = Arc::new(words);
        let mut out: Vec<bool> = Vec::with_capacity(len);
        // Slices of about `main_thread_slice` each, so the main thread draws
        // between them.
        while out.len() < len {
            let from = out.len();
            let w = Arc::clone(&words);
            let Some(slice) = self.run(move |state| {
                let start = Instant::now();
                let mut ok = Vec::new();
                for word in &w[from..] {
                    ok.push(state.check(word));
                    if start.elapsed() >= state.options.main_thread_slice {
                        break;
                    }
                }
                ok
            }) else {
                break; // the main thread did not answer: the rest count as correct
            };
            if slice.is_empty() {
                break;
            }
            out.extend(slice);
        }
        pad_answers(out, len)
    }

    fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let word = word.to_owned();
        self.run(move |state| {
            let checker = state.checker();
            let ns = NSString::from_str(&word);
            let range = NSRange::new(0, ns.length());
            let tag = state.tag();
            let guesses = |language: &NSString| -> Vec<String> {
                checker
                    .guessesForWordRange_inString_language_inSpellDocumentWithTag(
                        range,
                        &ns,
                        Some(language),
                        tag,
                    )
                    .map(|a| a.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_default()
            };
            match state.codes() {
                // Chromium's FillSuggestionList asks in the checker's language.
                None => {
                    let mut out = guesses(&checker.language());
                    out.truncate(max);
                    out
                }
                Some(codes) => {
                    let lists: Vec<Vec<String>> = codes
                        .iter()
                        .map(|code| guesses(&NSString::from_str(code)))
                        .collect();
                    interleave_suggestions(&lists, max)
                }
            }
        })
        .unwrap_or_default()
    }

    fn learn(&self, word: &str) {
        let word = word.to_owned();
        self.run(move |state| state.checker().learnWord(&NSString::from_str(&word)));
    }

    fn unlearn(&self, word: &str) {
        let word = word.to_owned();
        self.run(move |state| state.checker().unlearnWord(&NSString::from_str(&word)));
    }

    fn ignore(&self, word: &str) {
        let word = word.to_owned();
        self.run(move |state| {
            let tag = state.tag();
            state
                .checker()
                .ignoreWord_inSpellDocumentWithTag(&NSString::from_str(&word), tag);
        });
    }

    fn clear_ignored(&self) {
        self.run(|state| {
            let tag = state.tag();
            let none: objc2::rc::Retained<NSArray<NSString>> = NSArray::new();
            state
                .checker()
                .setIgnoredWords_inSpellDocumentWithTag(&none, tag);
        });
    }

    fn suggest_on_calling_thread(&self) -> bool {
        true
    }
}
