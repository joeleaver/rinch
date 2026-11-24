//! Spellchecking engine for Rinch (#2862, stage 1): no UI, no DOM.
//!
//! - [`SpellChecker`] is the engine trait: check a word, suggest corrections,
//!   add a word to the personal dictionary, ignore a word for the session,
//!   report the languages. It is object-safe and `Send + Sync`, so one
//!   `Arc<dyn SpellChecker>` can be shared with a checking thread.
//! - [`HunspellChecker`] implements it over Hunspell dictionaries with the
//!   pure-Rust `spellbook` crate. Dictionaries come from bytes the app bundles
//!   ([`HunspellDictionary::from_bytes`]) or, on desktop, from the system by
//!   locale ([`system`]). Several dictionaries mean several languages: a word
//!   is correct when any of them accepts it.
//! - [`words`] segments text on Unicode word boundaries (UAX #29) into the
//!   byte ranges worth checking, skipping what [`SkipRules`] says (URLs,
//!   email addresses, words with digits, all-caps words, over-long words).
//!   [`misspelled`] runs a checker over those ranges. A word in a script no
//!   loaded dictionary writes in is never marked.
//! - [`PlatformChecker`] implements it over the operating system's own
//!   spellchecker: `NSSpellChecker` on macOS, the Spell Checking API
//!   (`ISpellChecker`) on Windows. It is the default backend there, as in
//!   Chrome ([`runtime::Backend`]); [`platform`] has the details.
//! - [`PersonalDictionary`] holds the user's words in memory; the app loads
//!   and saves it. Words ignored for the session are kept apart and never
//!   persisted.
//!
//! Every choice above is the app's: the skip rules are a plain struct, the
//! personal dictionary is the app's to fill and store, the languages are
//! whatever dictionaries the app loads.
//!
//! ```
//! use rinch_spell::{HunspellChecker, SkipRules, SpellChecker, misspelled};
//!
//! // What an app bundles: an `.aff`/`.dic` pair (here a two-word one).
//! let aff = b"SET UTF-8\nTRY esianrtolcdugmfphbyvkw\n";
//! let dic = b"2\nhello\nworld\n";
//! let checker = HunspellChecker::from_bytes("en_US", aff, dic).unwrap();
//!
//! let text = "hello wurld, see https://exmple.com";
//! let bad: Vec<&str> = misspelled(&checker, text, &SkipRules::default())
//!     .map(|r| &text[r])
//!     .collect();
//! assert_eq!(bad, ["wurld", "see"]);
//! assert_eq!(checker.suggest("wurld", 5), ["world"]);
//!
//! // The user adds a word; the app saves the list however it likes.
//! checker.personal_dictionary().add_listener(|change| println!("{change:?}"));
//! checker.add_to_dictionary("see");
//! assert!(checker.check("see"));
//! assert_eq!(checker.personal_dictionary().words(), ["see"]);
//! ```

mod hunspell;
mod listeners;
mod locale;
mod personal;
pub mod platform;
pub mod runtime;
mod segment;
#[cfg(all(feature = "system-dictionaries", not(target_family = "wasm")))]
pub mod system;
mod unicode;

use std::ops::Range;

pub use hunspell::{CHROME_MAX_SUGGEST_LEN, DictionaryError, HunspellChecker, HunspellDictionary};
/// A Unicode script (UAX #24), as [`HunspellDictionary::scripts`] reports
/// and [`HunspellDictionary::with_scripts`] takes them: `Script::Latin`,
/// `Script::Cyrillic`, …
pub use icu_properties::props::Script;
pub use listeners::ListenerId;
pub use locale::normalize_locale;
pub use personal::{PersonalDictionary, PersonalDictionaryChange};
pub use platform::PlatformChecker;
pub use segment::{CHROME_MAX_CHECKED_LEN, SkipRules, words};

/// How many suggestions Chrome offers (`kMaxSuggestions` in
/// `components/spellcheck/common/spellcheck_common.h`).
pub const DEFAULT_MAX_SUGGESTIONS: usize = 5;

/// A change to the words a checker accepts beyond its dictionaries, as
/// [`SpellChecker::add_change_listener`] reports it. A UI holding marks clears
/// the ones a change makes correct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpellChange {
    /// The personal dictionary gained or lost a word.
    Personal(PersonalDictionaryChange),
    /// The word was ignored for the session.
    Ignored(String),
    /// Every session ignore was forgotten.
    IgnoresCleared,
}

/// A spellchecking engine.
///
/// It checks single words (a segment from [`words`]); breaking text into
/// words is not its job. The mutating calls take `&self` so that one checker
/// can be shared (`Arc<dyn SpellChecker>`) between the UI and a checking
/// thread; implementations use interior mutability.
pub trait SpellChecker: Send + Sync {
    /// Whether `word` is spelled correctly. A checker with nothing to check
    /// against (no dictionary) answers `true`.
    fn check(&self, word: &str) -> bool;

    /// Up to `max` corrections for `word`, best first. May be empty.
    fn suggest(&self, word: &str, max: usize) -> Vec<String>;

    /// Adds `word` to the user's personal dictionary (which the app persists).
    fn add_to_dictionary(&self, word: &str);

    /// Accepts `word` for the rest of the session without saving it.
    fn ignore(&self, word: &str);

    /// The languages checked, as their dictionaries name them (`en_US`).
    fn languages(&self) -> Vec<String>;

    /// Calls `listener` after each change to the words accepted beyond the
    /// dictionaries (personal dictionary, session ignores), on the thread that
    /// made it. `None` when this checker reports no changes (the default).
    fn add_change_listener(
        &self,
        listener: Box<dyn Fn(&SpellChange) + Send + Sync>,
    ) -> Option<ListenerId> {
        let _ = listener;
        None
    }

    /// Removes a change listener; returns whether it was subscribed.
    fn remove_change_listener(&self, id: ListenerId) -> bool {
        let _ = id;
        false
    }

    /// Whether each of `words` is spelled correctly, in order: one answer per
    /// word, as [`check`](Self::check) gives it. The checking thread asks in
    /// batches, so a backend whose every call is costly (a platform engine
    /// that must hop to another thread) can answer many words per hop. The
    /// default asks [`check`](Self::check) for each.
    fn check_batch(&self, words: &[&str]) -> Vec<bool> {
        words.iter().map(|w| self.check(w)).collect()
    }

    /// Whether [`suggest`](Self::suggest) should run on the thread that wants
    /// the suggestions (the UI thread, for the text context menu) rather than
    /// on a background thread. `true` for an engine bound to the main thread
    /// (macOS's `NSSpellChecker`), whose calls from elsewhere wait for the
    /// main thread anyway. Default `false`.
    fn suggest_on_calling_thread(&self) -> bool {
        false
    }
}

impl<T: SpellChecker + ?Sized> SpellChecker for std::sync::Arc<T> {
    fn check(&self, word: &str) -> bool {
        (**self).check(word)
    }
    fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        (**self).suggest(word, max)
    }
    fn add_to_dictionary(&self, word: &str) {
        (**self).add_to_dictionary(word)
    }
    fn ignore(&self, word: &str) {
        (**self).ignore(word)
    }
    fn languages(&self) -> Vec<String> {
        (**self).languages()
    }
    fn add_change_listener(
        &self,
        listener: Box<dyn Fn(&SpellChange) + Send + Sync>,
    ) -> Option<ListenerId> {
        (**self).add_change_listener(listener)
    }
    fn remove_change_listener(&self, id: ListenerId) -> bool {
        (**self).remove_change_listener(id)
    }
    fn check_batch(&self, words: &[&str]) -> Vec<bool> {
        (**self).check_batch(words)
    }
    fn suggest_on_calling_thread(&self) -> bool {
        (**self).suggest_on_calling_thread()
    }
}

/// The byte ranges of the misspelled words in `text`: [`words`] under
/// `rules`, kept where `checker` rejects the word.
pub fn misspelled<'a, C: SpellChecker + ?Sized>(
    checker: &'a C,
    text: &'a str,
    rules: &'a SkipRules,
) -> impl Iterator<Item = Range<usize>> + 'a {
    words(text, rules).filter(move |range| !checker.check(&text[range.clone()]))
}
