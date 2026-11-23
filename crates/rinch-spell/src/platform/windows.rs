//! Windows: the Spell Checking API (`ISpellCheckerFactory`, `ISpellChecker`)
//! on a COM thread of its own.
//!
//! Chromium keeps every call to these objects on one COM task runner
//! (`components/spellcheck/browser/windows_spell_checker.cc`,
//! `BackgroundHelper`: "All invocations of these methods must be posted to the
//! same COM SingleThreadTaskRunner"). rinch does the same with one thread,
//! `rinch-spell-windows`. Its apartment is the multithreaded one: the factory
//! is registered `ThreadingModel = Free` (`HKCR\CLSID\{7AB36653-1796-484B-
//! BDFA-E74F1DB7C1DC}\InprocServer32`, `MsSpellCheckingFacility.dll`), so in
//! the MTA its objects are called directly, with no proxy and no message
//! pump; Chromium's STA reaches the same objects through COM's marshalling.
//! A caller (the checking thread, the suggestion thread, the UI thread adding
//! a word) posts a request and waits for the answer; the COM thread never
//! waits on any of them, so nothing can deadlock.

use std::sync::mpsc;

use windows::Win32::Foundation::{ERROR_SUCCESS, S_OK};
use windows::Win32::Globalization::{
    CORRECTIVE_ACTION_GET_SUGGESTIONS, CORRECTIVE_ACTION_REPLACE, GetUserPreferredUILanguages,
    ISpellChecker, ISpellChecker2, ISpellCheckerFactory, MUI_LANGUAGE_NAME, SpellCheckerFactory,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, IEnumString,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_MULTI_SZ, RegGetValueW};
use windows::core::{HSTRING, Interface, PWSTR, w};

use super::{
    Native, choose_languages, interleave_suggestions, normalize_locale, pad_answers, split_multi_sz,
};

enum Request {
    Languages(mpsc::Sender<Vec<String>>),
    Check(Vec<String>, mpsc::Sender<Vec<bool>>),
    Suggest(String, usize, mpsc::Sender<Vec<String>>),
    Learn(String, mpsc::Sender<()>),
    Unlearn(String, mpsc::Sender<()>),
}

pub(crate) struct WindowsSpeller {
    tx: mpsc::Sender<Request>,
}

impl WindowsSpeller {
    /// Starts the COM thread and waits for it to create the factory and
    /// choose the languages; `None` when Windows has no spellchecking service
    /// (the factory cannot be created) or the thread cannot start.
    pub(crate) fn new(languages: Option<Vec<String>>) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<Request>();
        let (ready_tx, ready_rx) = mpsc::channel::<bool>();
        std::thread::Builder::new()
            .name("rinch-spell-windows".into())
            .spawn(move || {
                // SAFETY: this thread joins the MTA for its whole life and
                // leaves it after dropping every COM object it made.
                let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
                {
                    let engine = joined.then(|| Engine::new(languages)).flatten();
                    let _ = ready_tx.send(engine.is_some());
                    if let Some(engine) = engine {
                        for request in rx {
                            // A failing call must not take the thread down.
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                engine.serve(request)
                            }));
                        }
                    }
                }
                if joined {
                    unsafe { CoUninitialize() };
                }
            })
            .ok()?;
        ready_rx.recv().ok()?.then_some(Self { tx })
    }

    fn ask<R>(&self, make: impl FnOnce(mpsc::Sender<R>) -> Request) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(make(tx)).ok()?;
        rx.recv().ok()
    }
}

impl Native for WindowsSpeller {
    fn languages(&self) -> Option<Vec<String>> {
        Some(self.ask(Request::Languages).unwrap_or_default())
    }

    fn check_words(&self, words: Vec<String>) -> Vec<bool> {
        let len = words.len();
        pad_answers(
            self.ask(|tx| Request::Check(words, tx)).unwrap_or_default(),
            len,
        )
    }

    fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        self.ask(|tx| Request::Suggest(word.to_owned(), max, tx))
            .unwrap_or_default()
    }

    // Answered, so a check right after an add sees it.
    fn learn(&self, word: &str) {
        self.ask(|tx| Request::Learn(word.to_owned(), tx));
    }

    fn unlearn(&self, word: &str) {
        self.ask(|tx| Request::Unlearn(word.to_owned(), tx));
    }
}

/// The COM objects; lives on the COM thread only.
struct Engine {
    /// `(rinch tag, checker)` per language, most preferred first.
    checkers: Vec<(String, ISpellChecker)>,
}

impl Engine {
    fn new(languages: Option<Vec<String>>) -> Option<Self> {
        // SAFETY: COM is initialised on this thread (MTA).
        let factory: ISpellCheckerFactory =
            unsafe { CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER) }.ok()?;
        // SAFETY: a live factory.
        let supported = unsafe { factory.SupportedLanguages() }
            .map(|e| strings(&e))
            .unwrap_or_default();
        let wanted = languages.unwrap_or_else(user_languages);
        let checkers = choose_languages(&wanted, &supported)
            .into_iter()
            .filter_map(|tag| {
                // SAFETY: a live factory; a supported tag.
                let checker = unsafe { factory.CreateSpellChecker(&HSTRING::from(&tag)) }.ok()?;
                Some((normalize_locale(&tag)?, checker))
            })
            .collect();
        Some(Self { checkers })
    }

    fn serve(&self, request: Request) {
        match request {
            Request::Languages(tx) => {
                let _ = tx.send(self.checkers.iter().map(|(t, _)| t.clone()).collect());
            }
            Request::Check(words, tx) => {
                let _ = tx.send(words.iter().map(|w| self.check(w)).collect());
            }
            Request::Suggest(word, max, tx) => {
                let word = HSTRING::from(&word);
                let lists: Vec<Vec<String>> = self
                    .checkers
                    .iter()
                    // SAFETY: a live checker.
                    .map(|(_, c)| {
                        unsafe { c.Suggest(&word) }
                            .map(|e| strings(&e))
                            .unwrap_or_default()
                    })
                    .collect();
                let _ = tx.send(interleave_suggestions(&lists, max));
            }
            // `AddWordForAllLanguages` / `RemoveWordForAllLanguages`, as
            // Chromium does.
            Request::Learn(word, done) => {
                let word = HSTRING::from(&word);
                for (_, c) in &self.checkers {
                    // SAFETY: a live checker.
                    let _ = unsafe { c.Add(&word) };
                }
                let _ = done.send(());
            }
            Request::Unlearn(word, done) => {
                let word = HSTRING::from(&word);
                for (_, c) in &self.checkers {
                    if let Ok(c2) = c.cast::<ISpellChecker2>() {
                        // SAFETY: a live checker.
                        let _ = unsafe { c2.Remove(&word) };
                    }
                }
                let _ = done.send(());
            }
        }
    }

    /// Correct unless every language finds an error it would correct
    /// (Chromium's `RequestTextCheckForAllLanguages` keeps a misspelling only
    /// when all languages agree, and only `GET_SUGGESTIONS` / `REPLACE`
    /// errors; `DELETE` is a repeated word, not a spelling).
    fn check(&self, word: &str) -> bool {
        if self.checkers.is_empty() {
            return true;
        }
        let text = HSTRING::from(word);
        self.checkers.iter().any(|(_, checker)| {
            // SAFETY: a live checker.
            let Ok(errors) = (unsafe { checker.Check(&text) }) else {
                return true;
            };
            loop {
                let mut error = None;
                // SAFETY: `error` is a valid out pointer.
                let hr = unsafe { errors.Next(&mut error) };
                let Some(error) = error.filter(|_| hr == S_OK) else {
                    return true; // no (more) errors
                };
                // SAFETY: a live error.
                if let Ok(action) = unsafe { error.CorrectiveAction() }
                    && (action == CORRECTIVE_ACTION_GET_SUGGESTIONS
                        || action == CORRECTIVE_ACTION_REPLACE)
                {
                    return false;
                }
            }
        })
    }
}

/// Every string of an `IEnumString`, freeing each.
fn strings(e: &IEnumString) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        let mut item = [PWSTR::null()];
        let mut fetched = 0u32;
        // SAFETY: one slot, a valid count pointer.
        let hr = unsafe { e.Next(&mut item, Some(&mut fetched)) };
        if hr != S_OK || fetched == 0 || item[0].is_null() {
            break;
        }
        // SAFETY: a NUL-terminated string the enumerator allocated with
        // CoTaskMemAlloc; freed once read.
        unsafe {
            if let Ok(s) = item[0].to_string() {
                out.push(s);
            }
            CoTaskMemFree(Some(item[0].0 as *const _));
        }
    }
    out
}

/// The user's languages, most preferred first: the language list of
/// Settings (`HKCU\Control Panel\International\User Profile`, `Languages`,
/// which is what installs a language's spellchecker), else the preferred UI
/// languages.
fn user_languages() -> Vec<String> {
    let profile = registry_languages();
    if !profile.is_empty() {
        return profile;
    }
    let mut count = 0u32;
    let mut len = 0u32;
    // SAFETY: the size query, then a buffer of the size asked for.
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len).is_err() {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
        .is_err()
        {
            return Vec::new();
        }
        split_multi_sz(&buf)
    }
}

fn registry_languages() -> Vec<String> {
    let key = w!("Control Panel\\International\\User Profile");
    let value = w!("Languages");
    let mut size = 0u32;
    // SAFETY: the size query, then a buffer of that many bytes.
    unsafe {
        if RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_MULTI_SZ,
            None,
            None,
            Some(&mut size),
        ) != ERROR_SUCCESS
        {
            return Vec::new();
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        if RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_MULTI_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        ) != ERROR_SUCCESS
        {
            return Vec::new();
        }
        split_multi_sz(&buf)
    }
}

#[cfg(test)]
mod tests {
    use crate::SpellChecker;
    use crate::platform::PlatformChecker;

    /// Runs on a real Windows (a cross-built test binary): the system's own
    /// checker answers through the COM thread. What it knows depends on the
    /// machine's languages, so only English is asserted, when present.
    #[test]
    fn the_system_checker_answers() {
        let Some(checker) = PlatformChecker::new() else {
            eprintln!("no Windows spellchecking service");
            return;
        };
        let languages = checker.languages();
        eprintln!("languages: {languages:?}");
        if !languages.iter().any(|l| l.starts_with("en")) {
            return;
        }
        assert!(checker.check("hello"));
        assert!(!checker.check("helo"));
        assert_eq!(
            checker.check_batch(&["hello", "wrld", "world"]),
            [true, false, true]
        );
        let suggestions = checker.suggest("helo", 5);
        eprintln!("suggestions for helo: {suggestions:?}");
        assert!(!suggestions.is_empty() && suggestions.len() <= 5);
        // Chrome's word iterator would never hand Hunspell this; neither does
        // the script rule.
        assert!(checker.check("привет") || languages.iter().any(|l| l.starts_with("ru")));

        // Off the system dictionary, so the test leaves the machine as it was.
        let checker = checker.add_to_system_dictionary(false);
        assert!(!checker.check("qzxrinchword"));
        checker.add_to_dictionary("qzxrinchword");
        assert!(checker.check("qzxrinchword"));
        checker.ignore("zzqword");
        assert!(checker.check("zzqword"));
    }

    /// From several threads at once, none of them in COM.
    #[test]
    fn many_threads_share_the_checker() {
        let Some(checker) = PlatformChecker::with_languages(["en-US"]) else {
            return;
        };
        if checker.languages().is_empty() {
            return;
        }
        let checker = std::sync::Arc::new(checker);
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let c = std::sync::Arc::clone(&checker);
                std::thread::spawn(move || (0..50).all(|_| c.check("hello") && !c.check("helo")))
            })
            .collect();
        assert!(handles.into_iter().all(|h| h.join().unwrap()));
    }

    /// Learning and unlearning reach the system dictionary (and are undone).
    #[test]
    fn learn_and_unlearn_reach_the_system_dictionary() {
        let Some(checker) = PlatformChecker::with_languages(["en-US"]) else {
            return;
        };
        if checker.languages().is_empty() {
            return;
        }
        let word = "qzxlearnrinch";
        let fresh = PlatformChecker::with_languages(["en-US"]).unwrap();
        assert!(!fresh.check(word));
        checker.add_to_dictionary(word);
        // Another checker, with an empty personal dictionary of its own,
        // accepts it: it is the system's now.
        let other = PlatformChecker::with_languages(["en-US"]).unwrap();
        assert!(other.check(word), "Add reached the system dictionary");
        checker.personal_dictionary().remove(word);
        let after = PlatformChecker::with_languages(["en-US"]).unwrap();
        assert!(!after.check(word), "Remove took it back out");
    }
}
