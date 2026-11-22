//! The Hunspell backend, on the `spellbook` crate.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use icu_properties::props::Script;

use crate::listeners::{ListenerId, Listeners};
use crate::personal::{PersonalDictionary, WordSet};
use crate::unicode::{dictionary_scripts, is_mark, letter_scripts, nfc};
use crate::{SpellChange, SpellChecker};

/// Chrome's longest word it asks Hunspell for suggestions on, in UTF-8 bytes
/// (`kMaxSuggestLen` in `components/spellcheck/renderer/hunspell_engine.cc`).
/// Hunspell's suggestion search grows quickly with word length.
pub const CHROME_MAX_SUGGEST_LEN: usize = 24;

/// Why a dictionary could not be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DictionaryError {
    /// The `.aff` names an encoding (its `SET` line) this crate does not
    /// decode. UTF-8 and ISO 8859-1 are decoded.
    UnsupportedEncoding(String),
    /// The bytes are not valid in the encoding the `.aff` names.
    InvalidEncoding {
        /// The `.aff` or `.dic` file.
        file: &'static str,
    },
    /// spellbook rejected the dictionary; the message is spellbook's.
    Parse(String),
    /// A dictionary file could not be read.
    Io(String),
    /// No dictionary was found for the locale.
    NotFound(String),
}

impl fmt::Display for DictionaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedEncoding(name) => {
                write!(f, "unsupported dictionary encoding `{name}`")
            }
            Self::InvalidEncoding { file } => {
                write!(f, "the {file} file is not valid in its declared encoding")
            }
            Self::Parse(message) => write!(f, "malformed dictionary: {message}"),
            Self::Io(message) => write!(f, "could not read dictionary: {message}"),
            Self::NotFound(locale) => write!(f, "no dictionary found for `{locale}`"),
        }
    }
}

impl std::error::Error for DictionaryError {}

/// One loaded Hunspell dictionary and the language it is for.
///
/// Parsing a full dictionary takes a noticeable time and some megabytes, so
/// an app may build it off the UI thread: it is `Send + Sync`.
///
/// It knows the scripts it writes in ([`scripts`](Self::scripts)), derived
/// from its files; a checker leaves alone a word with no letter in a script
/// some loaded dictionary writes in, as Chrome's word iterator does.
pub struct HunspellDictionary {
    language: String,
    scripts: Vec<Script>,
    dict: spellbook::Dictionary,
}

impl fmt::Debug for HunspellDictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HunspellDictionary")
            .field("language", &self.language)
            .field("scripts", &self.scripts)
            .finish_non_exhaustive()
    }
}

impl HunspellDictionary {
    /// Loads a dictionary from the bytes of its `.aff` and `.dic` files (what
    /// an app bundles, `include_bytes!` or fetched). `language` is the tag the
    /// checker reports, such as `"en_US"`.
    ///
    /// The `.aff`'s `SET` line names the encoding of both files; UTF-8 (or no
    /// `SET`) and ISO 8859-1 are decoded. A UTF-8 byte-order mark is skipped.
    pub fn from_bytes(
        language: impl Into<String>,
        aff: &[u8],
        dic: &[u8],
    ) -> Result<Self, DictionaryError> {
        let encoding = declared_encoding(aff);
        let aff = decode(aff, &encoding, ".aff")?;
        let dic = decode(dic, &encoding, ".dic")?;
        Self::from_text(language, &aff, &dic)
    }

    /// Loads a dictionary from the text of its `.aff` and `.dic` files.
    ///
    /// Its scripts are every script on the `.aff`'s `TRY` line plus every
    /// script at least 1% of the `.dic`'s words use; override them with
    /// [`with_scripts`](Self::with_scripts).
    pub fn from_text(
        language: impl Into<String>,
        aff: &str,
        dic: &str,
    ) -> Result<Self, DictionaryError> {
        let dict = spellbook::Dictionary::new(aff, dic)
            .map_err(|e| DictionaryError::Parse(e.to_string()))?;
        Ok(Self {
            language: language.into(),
            scripts: dictionary_scripts(aff, dic),
            dict,
        })
    }

    /// Replaces the scripts this dictionary is taken to check (Unicode
    /// `Script` values, such as `Script::Latin`).
    pub fn with_scripts(mut self, scripts: impl IntoIterator<Item = Script>) -> Self {
        self.scripts = scripts.into_iter().collect();
        self.scripts.sort();
        self.scripts.dedup();
        self
    }

    /// The scripts this dictionary checks, sorted.
    pub fn scripts(&self) -> &[Script] {
        &self.scripts
    }

    /// The language tag given when it was loaded.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Whether this dictionary alone accepts `word` (no personal dictionary,
    /// no fallbacks).
    pub fn check(&self, word: &str) -> bool {
        self.dict.check(word)
    }
}

/// The encoding named on the `.aff`'s `SET` line, uppercased, or `"UTF-8"`.
fn declared_encoding(aff: &[u8]) -> String {
    let aff = aff.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(aff);
    for line in aff.split(|&b| b == b'\n') {
        let line = line.trim_ascii();
        if let Some(rest) = line.strip_prefix(b"SET")
            && rest.first().is_some_and(u8::is_ascii_whitespace)
        {
            return String::from_utf8_lossy(rest.trim_ascii()).to_ascii_uppercase();
        }
    }
    "UTF-8".to_owned()
}

fn decode<'a>(
    bytes: &'a [u8],
    encoding: &str,
    file: &'static str,
) -> Result<Cow<'a, str>, DictionaryError> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match encoding {
        "UTF-8" | "UTF8" => std::str::from_utf8(bytes)
            .map(Cow::Borrowed)
            .map_err(|_| DictionaryError::InvalidEncoding { file }),
        // ISO 8859-1 maps each byte to the code point of the same value.
        "ISO8859-1" | "ISO-8859-1" | "LATIN1" => Ok(match std::str::from_utf8(bytes) {
            Ok(ascii) if ascii.is_ascii() => Cow::Borrowed(ascii),
            _ => Cow::Owned(bytes.iter().map(|&b| b as char).collect()),
        }),
        // Another encoding is fine while the text is plain ASCII.
        other => match std::str::from_utf8(bytes) {
            Ok(ascii) if ascii.is_ascii() => Ok(Cow::Borrowed(ascii)),
            _ => Err(DictionaryError::UnsupportedEncoding(other.to_owned())),
        },
    }
}

/// A [`SpellChecker`] over one or more Hunspell dictionaries.
///
/// A word is correct when the session ignore list or the personal dictionary
/// accepts it, or when **any** loaded dictionary does (so a checker with
/// `en_US` and `de_DE` accepts English and German words alike). With no
/// dictionary loaded every word is correct: nothing is marked.
///
/// A word with no letter in any loaded dictionary's
/// [scripts](HunspellDictionary::scripts) is correct too: with only English
/// loaded, Russian, Hebrew or Japanese text is left unmarked (Chrome's word
/// iterator yields only words in the dictionary's script).
///
/// Words are compared in NFC, so a decomposed "cafe\u{301}" is "café".
///
/// Two fallbacks follow Chrome:
/// - A word with a typographic apostrophe (`’`, U+2019) is also tried with
///   `'`, since most dictionaries list only the ASCII form; its suggestions
///   come back with `’`.
/// - A word joined by internal punctuation ("in'n'out", "hello:hello") is
///   correct when each part is (Chrome's `IsValidContraction`). Turn it off
///   with [`contraction_fallback`](Self::contraction_fallback).
///
/// [`suggest`](SpellChecker::suggest) gives no suggestion for a word longer
/// than [`CHROME_MAX_SUGGEST_LEN`] bytes, as Chrome does; change it with
/// [`max_suggest_len`](Self::max_suggest_len).
///
/// [`add_change_listener`](SpellChecker::add_change_listener) reports every
/// change to what it accepts beyond its dictionaries: personal-dictionary
/// adds and removes, session ignores, and clearing the ignores.
pub struct HunspellChecker {
    dictionaries: Vec<HunspellDictionary>,
    personal: Arc<PersonalDictionary>,
    /// This checker's subscription on `personal`, forwarding its changes to
    /// `changes`; removed when the personal dictionary is replaced or the
    /// checker dropped.
    personal_link: ListenerId,
    changes: Arc<Listeners<SpellChange>>,
    ignored: WordSet,
    contraction_fallback: bool,
    max_suggest_len: Option<usize>,
}

impl fmt::Debug for HunspellChecker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HunspellChecker")
            .field("dictionaries", &self.dictionaries)
            .field("personal", &self.personal)
            .field("contraction_fallback", &self.contraction_fallback)
            .field("max_suggest_len", &self.max_suggest_len)
            .finish_non_exhaustive()
    }
}

impl Default for HunspellChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl HunspellChecker {
    /// A checker with no dictionary and an empty personal dictionary of its
    /// own.
    pub fn new() -> Self {
        let personal = Arc::new(PersonalDictionary::new());
        let changes = Arc::new(Listeners::default());
        Self {
            dictionaries: Vec::new(),
            personal_link: link(&personal, &changes),
            personal,
            changes,
            ignored: WordSet::default(),
            contraction_fallback: true,
            max_suggest_len: Some(CHROME_MAX_SUGGEST_LEN),
        }
    }

    /// A checker over one dictionary loaded from `.aff`/`.dic` bytes; see
    /// [`HunspellDictionary::from_bytes`].
    pub fn from_bytes(
        language: impl Into<String>,
        aff: &[u8],
        dic: &[u8],
    ) -> Result<Self, DictionaryError> {
        Ok(Self::new().with_dictionary(HunspellDictionary::from_bytes(language, aff, dic)?))
    }

    /// Adds a dictionary (another language).
    pub fn with_dictionary(mut self, dictionary: HunspellDictionary) -> Self {
        self.add_dictionary(dictionary);
        self
    }

    /// Adds a dictionary (another language).
    pub fn add_dictionary(&mut self, dictionary: HunspellDictionary) {
        self.dictionaries.push(dictionary);
    }

    /// Removes every dictionary for `language`; returns whether any was.
    pub fn remove_language(&mut self, language: &str) -> bool {
        let before = self.dictionaries.len();
        self.dictionaries.retain(|d| d.language != language);
        self.dictionaries.len() != before
    }

    /// Shares `personal` as this checker's personal dictionary (several
    /// checkers, or the app and a checker, can hold the same one).
    pub fn with_personal_dictionary(mut self, personal: Arc<PersonalDictionary>) -> Self {
        self.personal.remove_listener(self.personal_link);
        self.personal_link = link(&personal, &self.changes);
        self.personal = personal;
        self
    }

    /// The personal dictionary [`add_to_dictionary`](SpellChecker::add_to_dictionary)
    /// writes to.
    pub fn personal_dictionary(&self) -> &Arc<PersonalDictionary> {
        &self.personal
    }

    /// Whether to accept a word whose punctuation-separated parts are each
    /// correct (default `true`, as Chrome).
    pub fn contraction_fallback(mut self, on: bool) -> Self {
        self.contraction_fallback = on;
        self
    }

    /// The longest word, in UTF-8 bytes, to suggest corrections for; `None`
    /// for every length. Default [`CHROME_MAX_SUGGEST_LEN`].
    pub fn max_suggest_len(mut self, max: Option<usize>) -> Self {
        self.max_suggest_len = max;
        self
    }

    /// Forgets every word ignored with [`ignore`](SpellChecker::ignore);
    /// change listeners hear [`SpellChange::IgnoresCleared`] if any was.
    pub fn clear_ignored(&self) {
        if self.ignored.clear() {
            self.changes.notify(&SpellChange::IgnoresCleared);
        }
    }

    /// Whether some loaded dictionary writes in a script of one of `word`'s
    /// letters.
    pub fn covers(&self, word: &str) -> bool {
        letter_scripts(word).any(|script| {
            self.dictionaries
                .iter()
                .any(|d| d.scripts.binary_search(&script).is_ok())
        })
    }

    /// The loaded dictionaries, in the order they were added.
    pub fn dictionaries(&self) -> &[HunspellDictionary] {
        &self.dictionaries
    }

    fn accepts(&self, word: &str) -> bool {
        self.ignored.contains(word)
            || self.personal.contains(word)
            || self.dictionaries.iter().any(|d| d.dict.check(word))
    }

    fn accepts_with_apostrophe_fallback(&self, word: &str) -> bool {
        self.accepts(word) || (word.contains('’') && self.accepts(&word.replace('’', "'")))
    }
}

impl Drop for HunspellChecker {
    fn drop(&mut self) {
        self.personal.remove_listener(self.personal_link);
    }
}

/// Subscribes `changes` to `personal`'s changes.
fn link(personal: &PersonalDictionary, changes: &Arc<Listeners<SpellChange>>) -> ListenerId {
    let changes = Arc::clone(changes);
    personal.add_listener(move |change| changes.notify(&SpellChange::Personal(change.clone())))
}

/// Splits on the punctuation UAX #29 lets inside a word, the way Chrome's
/// contraction iterator (word rules without contractions) does. Combining
/// marks (a virama, a vowel sign) and the zero-width joiner and non-joiner
/// are part of the word.
fn contraction_parts(word: &str) -> impl Iterator<Item = &str> {
    word.split(|c: char| {
        !(c.is_alphanumeric() || c == '_' || c == '\u{200C}' || c == '\u{200D}' || is_mark(c))
    })
    .filter(|part| !part.is_empty())
}

impl SpellChecker for HunspellChecker {
    fn check(&self, word: &str) -> bool {
        if self.dictionaries.is_empty() || !self.covers(word) {
            return true;
        }
        let word = &*nfc(word);
        if self.accepts_with_apostrophe_fallback(word) {
            return true;
        }
        if self.contraction_fallback {
            let mut parts = contraction_parts(word).peekable();
            let first = parts.next();
            // A word that does not split is the word itself, checked above.
            if let Some(first) = first
                && parts.peek().is_some()
            {
                return self.accepts(first) && parts.all(|p| self.accepts(p));
            }
        }
        false
    }

    fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        if max == 0 || self.max_suggest_len.is_some_and(|limit| word.len() > limit) {
            return Vec::new();
        }
        let word = nfc(word);
        // A word typed with ’ (and no ') is looked up with ' and its
        // suggestions given back with ’, so a replacement keeps the style.
        let curly = word.contains('’') && !word.contains('\'');
        let word = if curly {
            Cow::Owned(word.replace('’', "'"))
        } else {
            word
        };
        let word = &*word;
        let restyle = |s: String| if curly { s.replace('\'', "’") } else { s };
        let mut out: Vec<String> = Vec::new();
        let mut scratch = Vec::new();
        for dictionary in &self.dictionaries {
            dictionary.dict.suggest(word, &mut scratch);
            for suggestion in scratch.drain(..).map(restyle) {
                if out.len() == max {
                    return out;
                }
                if !out.contains(&suggestion) {
                    out.push(suggestion);
                }
            }
        }
        out.truncate(max);
        out
    }

    fn add_to_dictionary(&self, word: &str) {
        self.personal.add(word);
    }

    fn ignore(&self, word: &str) {
        if !word.is_empty() && self.ignored.insert(word) {
            self.changes
                .notify(&SpellChange::Ignored(nfc(word).into_owned()));
        }
    }

    fn languages(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for d in &self.dictionaries {
            if !out.contains(&d.language) {
                out.push(d.language.clone());
            }
        }
        out
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_encoding_reads_the_set_line() {
        assert_eq!(declared_encoding(b"SET UTF-8\nTRY abc\n"), "UTF-8");
        assert_eq!(
            declared_encoding(b"\xEF\xBB\xBF# c\nSET ISO8859-1\n"),
            "ISO8859-1"
        );
        assert_eq!(declared_encoding(b"TRY abc\nSETTING x\n"), "UTF-8");
    }

    #[test]
    fn latin1_is_decoded_byte_for_byte() {
        let text = decode(b"caf\xE9", "ISO8859-1", ".dic").unwrap();
        assert_eq!(text, "café");
        assert_eq!(
            decode(b"caf\xE9", "UTF-8", ".dic"),
            Err(DictionaryError::InvalidEncoding { file: ".dic" })
        );
        assert_eq!(
            decode(b"caf\xE9", "KOI8-R", ".dic"),
            Err(DictionaryError::UnsupportedEncoding("KOI8-R".into()))
        );
        assert_eq!(decode(b"plain", "KOI8-R", ".dic").unwrap(), "plain");
    }

    #[test]
    fn contraction_parts_split_on_inner_punctuation() {
        let parts: Vec<_> = contraction_parts("in'n'out").collect();
        assert_eq!(parts, ["in", "n", "out"]);
        let parts: Vec<_> = contraction_parts("hello:world").collect();
        assert_eq!(parts, ["hello", "world"]);
        let parts: Vec<_> = contraction_parts("cafe\u{301}").collect();
        assert_eq!(parts, ["cafe\u{301}"]);
        // A virama (U+094D) and ZWNJ/ZWJ are inside a word, not separators.
        for word in ["क्षत्रिय", "می\u{200C}خواهم", "क्\u{200D}ष"] {
            let parts: Vec<_> = contraction_parts(word).collect();
            assert_eq!(parts, [word]);
        }
    }
}
