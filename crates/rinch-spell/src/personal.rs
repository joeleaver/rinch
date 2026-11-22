//! The personal dictionary (the app's, persisted by the app) and the session
//! ignore list (never persisted).

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, RwLock};

use crate::listeners::{ListenerId, Listeners};
use crate::unicode::nfc;

/// A set of words matched with Hunspell's casing rule: a word stored in
/// lowercase also accepts its Titlecase and ALL-CAPS forms ("rinch" accepts
/// "Rinch" and "RINCH"), a Titlecase word also accepts ALL CAPS ("Paris"
/// accepts "PARIS"), and nothing accepts a form with less capitalisation than
/// it was stored with ("Paris" does not accept "paris"). Words are stored
/// and looked up in NFC, so "cafe\u{301}" and "café" are one word.
#[derive(Default)]
pub(crate) struct WordSet {
    words: RwLock<HashSet<String>>,
}

impl WordSet {
    pub(crate) fn contains(&self, word: &str) -> bool {
        let word = &*nfc(word);
        let words = read(&self.words);
        if words.is_empty() {
            return false;
        }
        if words.contains(word) {
            return true;
        }
        let lower = word.to_lowercase();
        if is_title_case(word) {
            return words.contains(&lower);
        }
        if is_upper_case(word) {
            return words.contains(&lower) || words.contains(&title_case(&lower));
        }
        false
    }

    pub(crate) fn insert(&self, word: &str) -> bool {
        write(&self.words).insert(nfc(word).into_owned())
    }

    pub(crate) fn remove(&self, word: &str) -> bool {
        write(&self.words).remove(&*nfc(word))
    }

    /// Empties the set; returns whether it held anything.
    pub(crate) fn clear(&self) -> bool {
        let mut words = write(&self.words);
        let had = !words.is_empty();
        words.clear();
        had
    }

    pub(crate) fn extend<I: IntoIterator<Item = String>>(&self, words: I) {
        write(&self.words).extend(words.into_iter().map(|w| nfc(&w).into_owned()));
    }

    pub(crate) fn sorted(&self) -> Vec<String> {
        let mut out: Vec<String> = read(&self.words).iter().cloned().collect();
        out.sort();
        out
    }

    pub(crate) fn len(&self) -> usize {
        read(&self.words).len()
    }
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// First letter uppercase, every other letter lowercase, at least one of each
/// kind of cased letter not required ("A" is title case).
fn is_title_case(word: &str) -> bool {
    let mut letters = word.chars().filter(|c| c.is_alphabetic());
    match letters.next() {
        Some(first) if first.is_uppercase() => letters.all(|c| !c.is_uppercase()),
        _ => false,
    }
}

/// Two or more cased letters, none lowercase.
fn is_upper_case(word: &str) -> bool {
    !word.chars().any(char::is_lowercase) && word.chars().filter(|c| c.is_uppercase()).count() >= 2
}

fn title_case(lower: &str) -> String {
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A change to a [`PersonalDictionary`], as its listener receives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PersonalDictionaryChange {
    /// The word was added (it was not there before).
    Added(String),
    /// The word was removed (it was there before).
    Removed(String),
}

/// The user's own words, which every checker sharing it accepts.
///
/// It lives in memory; persistence is the app's. Load the saved list with
/// [`with_words`](Self::with_words) (or [`extend`](Self::extend)), and either
/// save on each change through [`add_listener`](Self::add_listener) or read
/// [`words`](Self::words) when it suits. Loading never calls a listener.
/// Several listeners may subscribe (the app saving the list, the framework
/// clearing marks), each removed by its [`ListenerId`].
///
/// Words match with Hunspell's casing rule: "rinch" also accepts "Rinch" and
/// "RINCH"; "Rinch" accepts "RINCH" but not "rinch".
///
/// It is `Send + Sync` and is shared by `Arc`, so a checker on another
/// thread sees words the UI thread adds.
#[derive(Default)]
pub struct PersonalDictionary {
    words: WordSet,
    listeners: Listeners<PersonalDictionaryChange>,
}

impl fmt::Debug for PersonalDictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersonalDictionary")
            .field("len", &self.words.len())
            .finish_non_exhaustive()
    }
}

impl PersonalDictionary {
    /// An empty dictionary.
    pub fn new() -> Self {
        Self::default()
    }

    /// A dictionary holding `words` (the app's saved list).
    pub fn with_words<I, S>(words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let dict = Self::new();
        dict.extend(words);
        dict
    }

    /// Adds `words` without calling the listener (loading, not editing).
    pub fn extend<I, S>(&self, words: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.words
            .extend(words.into_iter().map(Into::into).filter(|w| !w.is_empty()));
    }

    /// Calls `listener` after every [`add`](Self::add) or
    /// [`remove`](Self::remove) that changed the dictionary, after the
    /// listeners added before it. It runs on the thread that made the
    /// change, with no lock held, so it may read the dictionary.
    pub fn add_listener(
        &self,
        listener: impl Fn(&PersonalDictionaryChange) + Send + Sync + 'static,
    ) -> ListenerId {
        self.listeners.add(Arc::new(listener))
    }

    /// Removes a listener; returns whether it was subscribed.
    pub fn remove_listener(&self, id: ListenerId) -> bool {
        self.listeners.remove(id)
    }

    /// Adds `word` (in NFC). Returns whether it was new; listeners hear only
    /// a new word. An empty word is ignored.
    pub fn add(&self, word: &str) -> bool {
        if word.is_empty() || !self.words.insert(word) {
            return false;
        }
        self.notify(PersonalDictionaryChange::Added(nfc(word).into_owned()));
        true
    }

    /// Removes exactly `word` (no casing rule; NFC). Returns whether it was
    /// there.
    pub fn remove(&self, word: &str) -> bool {
        if !self.words.remove(word) {
            return false;
        }
        self.notify(PersonalDictionaryChange::Removed(nfc(word).into_owned()));
        true
    }

    /// Whether `word` is accepted, with the casing rule.
    pub fn contains(&self, word: &str) -> bool {
        self.words.contains(word)
    }

    /// Every word, sorted: the list to save.
    pub fn words(&self) -> Vec<String> {
        self.words.sorted()
    }

    /// How many words it holds.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether it holds no word.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn notify(&self, change: PersonalDictionaryChange) {
        self.listeners.notify(&change);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn casing_rule_follows_hunspell() {
        let set = WordSet::default();
        set.insert("rinch");
        set.insert("Paris");
        assert!(set.contains("rinch"));
        assert!(set.contains("Rinch"));
        assert!(set.contains("RINCH"));
        assert!(!set.contains("rInch"));
        assert!(set.contains("Paris"));
        assert!(set.contains("PARIS"));
        assert!(!set.contains("paris"));
    }

    #[test]
    fn listener_hears_changes_but_not_loads() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let dict = PersonalDictionary::with_words(["loaded"]);
        let sink = Arc::clone(&heard);
        dict.add_listener(move |c| sink.lock().unwrap().push(c.clone()));
        dict.extend(["also-loaded"]);
        assert!(dict.add("rinch"));
        assert!(!dict.add("rinch"), "a second add is not new");
        assert!(!dict.add(""));
        assert!(dict.remove("loaded"));
        assert!(!dict.remove("loaded"));
        assert_eq!(
            *heard.lock().unwrap(),
            [
                PersonalDictionaryChange::Added("rinch".into()),
                PersonalDictionaryChange::Removed("loaded".into()),
            ]
        );
        assert_eq!(dict.words(), ["also-loaded", "rinch"]);
    }

    #[test]
    fn listener_may_read_the_dictionary() {
        // The listener runs with no lock held, so it can export the list.
        let dict = Arc::new(PersonalDictionary::new());
        let saved = Arc::new(Mutex::new(Vec::new()));
        let (d, s) = (Arc::downgrade(&dict), Arc::clone(&saved));
        dict.add_listener(move |_| {
            if let Some(d) = d.upgrade() {
                *s.lock().unwrap() = d.words();
            }
        });
        dict.add("b");
        dict.add("a");
        assert_eq!(*saved.lock().unwrap(), ["a", "b"]);
    }

    #[test]
    fn several_listeners_each_removable() {
        let dict = PersonalDictionary::new();
        let a = Arc::new(Mutex::new(0));
        let b = Arc::new(Mutex::new(0));
        let (sa, sb) = (Arc::clone(&a), Arc::clone(&b));
        let ida = dict.add_listener(move |_| *sa.lock().unwrap() += 1);
        dict.add_listener(move |_| *sb.lock().unwrap() += 1);
        dict.add("one");
        assert!(dict.remove_listener(ida));
        assert!(!dict.remove_listener(ida));
        dict.add("two");
        assert_eq!((*a.lock().unwrap(), *b.lock().unwrap()), (1, 2));
    }
}
