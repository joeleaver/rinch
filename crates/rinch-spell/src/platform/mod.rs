//! The operating system's own spellchecker behind [`SpellChecker`] (#2862,
//! stage 4): `NSSpellChecker` on macOS, the Spell Checking API
//! (`ISpellCheckerFactory` / `ISpellChecker`) on Windows. Chrome's choice:
//! where the platform has a checker it is the default backend
//! ([`crate::runtime::Backend::Auto`]), so words the user taught the system
//! (in TextEdit, Mail, Word) are accepted, and the system's languages are
//! checked with the system's dictionaries.
//!
//! [`PlatformChecker`] is the same on both. It keeps what rinch owns:
//!
//! - **The personal dictionary.** [`add_to_dictionary`](SpellChecker::add_to_dictionary)
//!   adds the word to rinch's [`PersonalDictionary`] (whose listeners the app
//!   uses to save it, as with Hunspell) **and**, while
//!   [`add_to_system_dictionary`](PlatformChecker::add_to_system_dictionary)
//!   is on (the default, as Chrome does), to the system's user dictionary:
//!   `-[NSSpellChecker learnWord:]`, `ISpellChecker::Add` for every language.
//!   A word the app removes from the personal dictionary is unlearned there
//!   too (`unlearnWord:`, `ISpellChecker2::Remove`). Words the app loads into
//!   the personal dictionary (`with_words`, `extend`) are accepted by rinch
//!   and never written to the system.
//! - **The session ignore list.** Kept by rinch (so
//!   [`clear_ignored`](PlatformChecker::clear_ignored) works and the change
//!   listeners hear it). On macOS the word is also given to
//!   `ignoreWord:inSpellDocumentWithTag:` with this checker's own document
//!   tag; Windows' `ISpellChecker::Ignore` cannot be undone, so it is not
//!   called there.
//! - **Script coverage.** A word with no letter in a script one of the
//!   checked languages writes in is correct ([`language_scripts`]), as with
//!   Hunspell. A language this crate has no script for (a rare one) turns the
//!   rule off: every word goes to the platform, which decides.
//!
//! The languages are the system's by default (macOS: the user's preferred
//! spelling languages, with the system's automatic language detection;
//! Windows: the languages of the user's language list that have a
//! spellchecker installed), or the app's list where the platform has them
//! ([`PlatformChecker::with_languages`]): a word is correct when any of them
//! accepts it.
//!
//! **Threads.** The trait is `Send + Sync` and the runtime checks on its
//! `rinch-spellcheck` thread, but neither engine may be called from just any
//! thread:
//!
//! - macOS: every `NSSpellChecker` call runs on the **main thread**, as
//!   Chromium makes them (its `SpellCheckHostChromeImpl` calls
//!   `spellcheck_platform::CheckSpelling` on the browser UI thread). Called on
//!   the main thread (rinch's UI thread on macOS), a call runs at once; from
//!   another thread it is queued on the main dispatch queue and the caller
//!   waits, at most [`PlatformOptions::main_thread_wait`] (default
//!   [`MAIN_THREAD_WAIT`], so a main thread that never comes round, or one
//!   blocked on the caller, cannot deadlock it). A batch of words is checked
//!   in slices of about [`PlatformOptions::main_thread_slice`] (default
//!   [`MAIN_THREAD_SLICE`]) each, so the
//!   UI thread is never held for long. Suggestions are asked on the calling
//!   (UI) thread ([`SpellChecker::suggest_on_calling_thread`]).
//! - Windows: the COM objects live on one thread of their own,
//!   `rinch-spell-windows`, as Chromium's `windows_spell_checker.cc` keeps
//!   them on one COM task runner. That thread joins the multithreaded
//!   apartment (Chromium's runner is an STA): the factory is registered
//!   `ThreadingModel = Free`, so in the MTA its objects are called directly,
//!   with no proxy and no message pump. Every call is posted to the thread;
//!   checks and suggestions wait for its answer, adding and removing a word
//!   do not (the queue is in order, so a later check sees them).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use icu_properties::props::Script;

use crate::listeners::{ListenerId, Listeners};
use crate::locale::normalize_locale;
use crate::personal::{PersonalDictionary, PersonalDictionaryChange, WordSet};
use crate::unicode::{letter_scripts, nfc};
use crate::{SpellChange, SpellChecker};

#[cfg(all(feature = "platform", target_os = "macos"))]
mod mac;
#[cfg(all(feature = "platform", target_os = "windows"))]
mod windows;

/// How long a call made off the main thread waits for the main thread on
/// macOS before it gives up (the words then count as correct, and nothing is
/// marked for them).
pub const MAIN_THREAD_WAIT: Duration = Duration::from_secs(2);

/// About how long one slice of a batch check holds the main thread on macOS
/// (at least one word per slice). The web editor's slice budget, for the
/// same reason: the UI thread keeps drawing between slices.
pub const MAIN_THREAD_SLICE: Duration = Duration::from_millis(3);

/// How long the script rule waits before asking the platform for its
/// languages again after it could not answer (macOS: the main thread did
/// not come round in time). Until then the rule is off and the platform
/// judges every word.
pub const LANGUAGES_RETRY: Duration = Duration::from_secs(30);

/// Tuning of the platform checker's thread hops (macOS; Windows ignores
/// them). [`Default`] is [`MAIN_THREAD_WAIT`] and [`MAIN_THREAD_SLICE`];
/// [`crate::runtime::SpellcheckConfig`] carries both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatformOptions {
    /// How long a call made off the main thread waits for it before giving
    /// up (the words then count as correct).
    pub main_thread_wait: Duration,
    /// About how long one slice of a batch check holds the main thread (at
    /// least one word per slice).
    pub main_thread_slice: Duration,
}

impl Default for PlatformOptions {
    fn default() -> Self {
        Self {
            main_thread_wait: MAIN_THREAD_WAIT,
            main_thread_slice: MAIN_THREAD_SLICE,
        }
    }
}

/// Whether this build has a platform spellchecker: macOS or Windows, with
/// the `platform` feature (on by default).
pub const AVAILABLE: bool = cfg!(all(
    feature = "platform",
    any(target_os = "macos", target_os = "windows")
));

/// What a platform engine does; the rest is [`PlatformChecker`]'s.
pub(crate) trait Native: Send + Sync + 'static {
    /// The languages checked, in rinch's form (`en_US`). `None` when the
    /// platform could not be asked just now (macOS: the main thread did not
    /// answer in time).
    fn languages(&self) -> Option<Vec<String>>;
    /// One answer per word, in order.
    fn check_words(&self, words: Vec<String>) -> Vec<bool>;
    /// Up to `max` corrections, best first.
    fn suggest(&self, word: &str, max: usize) -> Vec<String>;
    /// Adds a word to the system's user dictionary.
    fn learn(&self, word: &str);
    /// Removes a word from the system's user dictionary.
    fn unlearn(&self, word: &str);
    /// Ignores a word for this checker's session, where the platform can.
    fn ignore(&self, word: &str) {
        let _ = word;
    }
    /// Forgets [`ignore`](Self::ignore)'s words.
    fn clear_ignored(&self) {}
    /// See [`SpellChecker::suggest_on_calling_thread`].
    fn suggest_on_calling_thread(&self) -> bool {
        false
    }
}

/// The operating system's spellchecker as a [`SpellChecker`]; see the
/// [module documentation](self).
pub struct PlatformChecker {
    native: Arc<dyn Native>,
    personal: Arc<PersonalDictionary>,
    personal_link: ListenerId,
    changes: Arc<Listeners<SpellChange>>,
    ignored: WordSet,
    to_system: Arc<AtomicBool>,
    /// The scripts of the checked languages; see [`Scripts`].
    scripts: Mutex<Scripts>,
}

/// What the script rule knows of the checked languages.
#[derive(Clone, Debug, Default)]
enum Scripts {
    /// Not asked yet.
    #[default]
    Unasked,
    /// The platform named its languages: their scripts, or `None` when one
    /// has no known script (rule off for good).
    Known(Option<Vec<Script>>),
    /// The platform could not answer; rule off, asked again after this.
    Unanswered(Instant),
}

impl std::fmt::Debug for PlatformChecker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformChecker")
            .field("personal", &self.personal)
            .field("add_to_system", &self.to_system.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl PlatformChecker {
    /// The system's spellchecker for the user's languages, `None` where
    /// there is none ([`AVAILABLE`] is false). Cheap: nothing is asked of
    /// the platform until the first word.
    pub fn new() -> Option<Self> {
        Self::with_options(None, PlatformOptions::default())
    }

    /// [`new`](Self::new) (`languages: None`) or
    /// [`with_languages`](Self::with_languages), with the thread-hop tuning.
    pub fn with_options(languages: Option<Vec<String>>, options: PlatformOptions) -> Option<Self> {
        let languages = languages.map(|l| l.iter().filter_map(|l| normalize_locale(l)).collect());
        Self::native(languages, options).map(Self::from_native)
    }

    /// The system's spellchecker for `languages` (`en_US`, `de`, `pt-BR`):
    /// those the platform has a dictionary for are checked, the others left
    /// out. With none of them available, nothing is marked. `None` where
    /// there is no platform checker.
    pub fn with_languages<I, S>(languages: I) -> Option<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let languages: Vec<String> = languages.into_iter().map(Into::into).collect();
        Self::with_options(Some(languages), PlatformOptions::default())
    }

    #[allow(unused_variables)]
    fn native(languages: Option<Vec<String>>, options: PlatformOptions) -> Option<Arc<dyn Native>> {
        #[cfg(all(feature = "platform", target_os = "macos"))]
        return Some(Arc::new(mac::MacSpeller::new(languages, options)));
        #[cfg(all(feature = "platform", target_os = "windows"))]
        return windows::WindowsSpeller::new(languages).map(|s| Arc::new(s) as Arc<dyn Native>);
        #[allow(unreachable_code)]
        None
    }

    pub(crate) fn from_native(native: Arc<dyn Native>) -> Self {
        let personal = Arc::new(PersonalDictionary::new());
        let changes = Arc::new(Listeners::default());
        let to_system = Arc::new(AtomicBool::new(true));
        Self {
            personal_link: link(&personal, &changes, &native, &to_system),
            native,
            personal,
            changes,
            ignored: WordSet::default(),
            to_system,
            scripts: Mutex::new(Scripts::Unasked),
        }
    }

    /// Shares `personal` as this checker's personal dictionary (the app
    /// loads and saves it).
    pub fn with_personal_dictionary(mut self, personal: Arc<PersonalDictionary>) -> Self {
        self.personal.remove_listener(self.personal_link);
        self.personal_link = link(&personal, &self.changes, &self.native, &self.to_system);
        self.personal = personal;
        self
    }

    /// Whether a word added to the personal dictionary (by
    /// [`add_to_dictionary`](SpellChecker::add_to_dictionary) or by the app)
    /// is also added to the system's user dictionary, and a word removed
    /// from it removed there. Default `true`, as Chrome does on macOS and
    /// Windows. Off, the word stays rinch's: accepted in this app only.
    pub fn add_to_system_dictionary(self, on: bool) -> Self {
        self.to_system.store(on, Ordering::Relaxed);
        self
    }

    /// The personal dictionary [`add_to_dictionary`](SpellChecker::add_to_dictionary)
    /// writes to.
    pub fn personal_dictionary(&self) -> &Arc<PersonalDictionary> {
        &self.personal
    }

    /// Forgets every word ignored with [`ignore`](SpellChecker::ignore);
    /// change listeners hear [`SpellChange::IgnoresCleared`] if any was.
    pub fn clear_ignored(&self) {
        if self.ignored.clear() {
            self.native.clear_ignored();
            self.changes.notify(&SpellChange::IgnoresCleared);
        }
    }

    /// The platform's languages, `None` when it could not answer just now
    /// (macOS: the main thread did not come round in time).
    pub(crate) fn platform_languages(&self) -> Option<Vec<String>> {
        self.native.languages()
    }

    /// Whether some checked language writes in a script of one of `word`'s
    /// letters (always `true` while the languages are unknown, or one of them
    /// has no known script: the platform decides then).
    pub fn covers(&self, word: &str) -> bool {
        covered(self.scripts().as_deref(), word)
    }

    /// The scripts of the checked languages (`None`: rule off), asking the
    /// platform at most once, or once per [`LANGUAGES_RETRY`] while it cannot
    /// answer. A batch calls it once, not per word: each ask may be a hop to
    /// the main thread.
    fn scripts(&self) -> Option<Vec<Script>> {
        let mut cache = self.scripts.lock().unwrap_or_else(|e| e.into_inner());
        match &*cache {
            Scripts::Known(scripts) => return scripts.clone(),
            Scripts::Unanswered(retry) if Instant::now() < *retry => return None,
            _ => {}
        }
        *cache = match self.native.languages() {
            Some(languages) => Scripts::Known(scripts_of_languages(&languages)),
            None => Scripts::Unanswered(Instant::now() + LANGUAGES_RETRY),
        };
        match &*cache {
            Scripts::Known(scripts) => scripts.clone(),
            _ => None,
        }
    }

    /// Whether rinch itself accepts `word` (NFC): ignored or in the personal
    /// dictionary, or in no covered script (`scripts`, from [`Self::scripts`]).
    fn accepts_here(&self, scripts: Option<&[Script]>, word: &str) -> bool {
        !covered(scripts, word) || self.ignored.contains(word) || self.personal.contains(word)
    }
}

impl Drop for PlatformChecker {
    fn drop(&mut self) {
        self.personal.remove_listener(self.personal_link);
    }
}

/// Forwards `personal`'s changes to `changes`, and to the system dictionary
/// while `to_system` is on.
fn link(
    personal: &PersonalDictionary,
    changes: &Arc<Listeners<SpellChange>>,
    native: &Arc<dyn Native>,
    to_system: &Arc<AtomicBool>,
) -> ListenerId {
    let changes = Arc::clone(changes);
    let native = Arc::clone(native);
    let to_system = Arc::clone(to_system);
    personal.add_listener(move |change| {
        if to_system.load(Ordering::Relaxed) {
            match change {
                PersonalDictionaryChange::Added(word) => native.learn(word),
                PersonalDictionaryChange::Removed(word) => native.unlearn(word),
            }
        }
        changes.notify(&SpellChange::Personal(change.clone()));
    })
}

impl SpellChecker for PlatformChecker {
    fn check(&self, word: &str) -> bool {
        self.check_batch(&[word])[0]
    }

    fn check_batch(&self, words: &[&str]) -> Vec<bool> {
        let mut out = vec![true; words.len()];
        let mut ask: Vec<String> = Vec::new();
        let mut at: Vec<usize> = Vec::new();
        let scripts = self.scripts();
        for (i, word) in words.iter().enumerate() {
            let word = nfc(word);
            if !word.is_empty() && !self.accepts_here(scripts.as_deref(), &word) {
                ask.push(word.into_owned());
                at.push(i);
            }
        }
        if ask.is_empty() {
            return out;
        }
        let answers = self.native.check_words(ask);
        for (i, ok) in at.into_iter().zip(answers) {
            out[i] = ok;
        }
        out
    }

    fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        if max == 0 || word.is_empty() {
            return Vec::new();
        }
        let mut out = self.native.suggest(&nfc(word), max);
        out.truncate(max);
        out
    }

    fn add_to_dictionary(&self, word: &str) {
        // The link above forwards it to the system dictionary.
        self.personal.add(word);
    }

    fn ignore(&self, word: &str) {
        if !word.is_empty() && self.ignored.insert(word) {
            let word = nfc(word).into_owned();
            self.native.ignore(&word);
            self.changes.notify(&SpellChange::Ignored(word));
        }
    }

    fn languages(&self) -> Vec<String> {
        self.native.languages().unwrap_or_default()
    }

    fn add_change_listener(
        &self,
        listener: Box<dyn Fn(&SpellChange) + Send + Sync>,
    ) -> Option<ListenerId> {
        Some(self.changes.add(Arc::from(listener)))
    }

    fn remove_change_listener(&self, id: ListenerId) -> bool {
        self.changes.remove(id)
    }

    fn suggest_on_calling_thread(&self) -> bool {
        self.native.suggest_on_calling_thread()
    }
}

/// Whether `word` has a letter in `scripts` (always, when `None`).
fn covered(scripts: Option<&[Script]>, word: &str) -> bool {
    match scripts {
        Some(scripts) => letter_scripts(word).any(|s| scripts.binary_search(&s).is_ok()),
        None => true,
    }
}

/// The union of [`language_scripts`] over `languages`, sorted; `None` when
/// one of them has no known script.
fn scripts_of_languages(languages: &[String]) -> Option<Vec<Script>> {
    let mut out: Vec<Script> = Vec::new();
    for language in languages {
        out.extend_from_slice(language_scripts(language)?);
    }
    out.sort();
    out.dedup();
    Some(out)
}

/// The scripts a language is written in, for the script-coverage rule:
/// `en_US` → Latin, `ru` → Cyrillic, `sr` → Cyrillic and Latin, `ja` → Han,
/// Hiragana and Katakana. `None` for a language not in this crate's table
/// (the rule is then off for the checker: the platform decides).
pub fn language_scripts(language: &str) -> Option<&'static [Script]> {
    use Script as S;
    let tag = normalize_locale(language)?;
    let lang = tag.split('_').next().unwrap_or(&tag);
    const LATIN: &[&str] = &[
        "af", "az", "br", "bs", "ca", "co", "cs", "cy", "da", "de", "en", "eo", "es", "et", "eu",
        "fi", "fil", "fo", "fr", "fy", "ga", "gd", "gl", "ha", "haw", "hr", "ht", "hu", "id", "ig",
        "is", "it", "jv", "ku", "la", "lb", "lt", "lv", "mg", "mi", "ms", "mt", "nb", "nl", "nn",
        "no", "ny", "oc", "pl", "pt", "qu", "rm", "ro", "rw", "sk", "sl", "sm", "sn", "so", "sq",
        "st", "su", "sv", "sw", "tk", "tl", "tn", "to", "tr", "uz", "vi", "wo", "xh", "yo", "zu",
    ];
    if LATIN.contains(&lang) {
        return Some(&[S::Latin]);
    }
    Some(match lang {
        "ru" | "uk" | "be" | "bg" | "mk" | "kk" | "ky" | "tg" | "mn" | "tt" | "ba" | "cv" => {
            &[S::Cyrillic]
        }
        "sr" => &[S::Cyrillic, S::Latin],
        "el" => &[S::Greek],
        "hy" => &[S::Armenian],
        "ka" => &[S::Georgian],
        "he" | "iw" | "yi" => &[S::Hebrew],
        "ar" | "fa" | "ur" | "ps" | "ug" | "sd" | "ckb" => &[S::Arabic],
        "hi" | "mr" | "ne" | "sa" | "kok" => &[S::Devanagari],
        "bn" | "as" => &[S::Bengali],
        "pa" => &[S::Gurmukhi],
        "gu" => &[S::Gujarati],
        "or" => &[S::Oriya],
        "ta" => &[S::Tamil],
        "te" => &[S::Telugu],
        "kn" => &[S::Kannada],
        "ml" => &[S::Malayalam],
        "si" => &[S::Sinhala],
        "th" => &[S::Thai],
        "lo" => &[S::Lao],
        "km" => &[S::Khmer],
        "my" => &[S::Myanmar],
        "bo" | "dz" => &[S::Tibetan],
        "am" | "ti" => &[S::Ethiopic],
        "zh" => &[S::Han],
        "ja" => &[S::Han, S::Hiragana, S::Katakana],
        "ko" => &[S::Hangul, S::Han],
        _ => return None,
    })
}

/// A rinch language tag as macOS names its spelling languages, Chromium's
/// `ConvertLanguageCodeToMac`: `en_US` → `en` and `pt_PT` → `pt` (macOS lists
/// them bare), `pl` → `pl_PL`, any other `xx_YY` as it is, a bare language
/// as it is.
pub fn to_mac_language(tag: &str) -> Option<String> {
    let tag = normalize_locale(tag)?;
    Some(match tag.as_str() {
        "en_US" => "en".to_owned(),
        "pt_PT" => "pt".to_owned(),
        "pl" => "pl_PL".to_owned(),
        _ => tag,
    })
}

/// A macOS spelling language in rinch's form, Chromium's
/// `ConvertLanguageCodeFromMac` with `_`: `en` → `en_US`, `pt` → `pt_PT`,
/// `pl_PL` → `pl`, `en_GB` → `en_GB`, `zh-Hans` → `zh`.
pub fn from_mac_language(code: &str) -> Option<String> {
    Some(match code {
        "en" => "en_US".to_owned(),
        "pt" => "pt_PT".to_owned(),
        "pl_PL" => "pl".to_owned(),
        _ => normalize_locale(code)?,
    })
}

/// A rinch language tag as a BCP 47 tag, as Windows names languages:
/// `en_US` → `en-US`, `de` → `de`.
pub fn to_bcp47(tag: &str) -> Option<String> {
    Some(normalize_locale(tag)?.replace('_', "-"))
}

/// The strings of a list of NUL-terminated UTF-16 strings ending in an
/// empty one (a `REG_MULTI_SZ` value, `GetUserPreferredUILanguages`'s
/// buffer), empty ones skipped.
#[allow(dead_code)]
pub(crate) fn split_multi_sz(buffer: &[u16]) -> Vec<String> {
    buffer
        .split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// The platform languages to check for `wanted` (rinch or BCP 47 tags, most
/// preferred first), given the languages the platform has a spellchecker
/// for: each wanted tag takes the supported one with the same name (any
/// case, `-` or `_`), or, when it names no region or its region is not
/// supported, the first supported one of the same language. In `wanted`'s
/// order, without repeats. Windows' language list holds `en-US` and `ko`
/// while its spellcheckers say `en-US` and `ko-KR`.
pub fn choose_languages(wanted: &[String], supported: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in wanted {
        let Some(want) = normalize_locale(tag) else {
            continue;
        };
        let lang = want.split('_').next().unwrap_or(&want);
        let normal = |s: &String| normalize_locale(s);
        let found = supported
            .iter()
            .find(|s| normal(s).as_deref() == Some(want.as_str()))
            .or_else(|| {
                supported.iter().find(|s| {
                    normal(s).is_some_and(|n| n == lang || n.split('_').next() == Some(lang))
                })
            });
        if let Some(found) = found
            && !out.contains(found)
        {
            out.push(found.clone());
        }
    }
    out
}

/// The words `ok` says are correct, as the trait's batch answer, padded with
/// `true` (correct: never marked) to `len` when a platform call failed
/// part-way. Shared by both engines.
#[allow(dead_code)]
pub(crate) fn pad_answers(mut ok: Vec<bool>, len: usize) -> Vec<bool> {
    ok.resize(len, true);
    ok
}

/// Interleaves several languages' suggestion lists, first of each, then the
/// second of each, …, without repeats, up to `max` (Chromium's
/// `spellcheck::FillSuggestions`, which spreads the room across languages).
pub fn interleave_suggestions(lists: &[Vec<String>], max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
    for i in 0..longest {
        for list in lists {
            if out.len() == max {
                return out;
            }
            if let Some(s) = list.get(i)
                && !out.contains(s)
            {
                out.push(s.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
