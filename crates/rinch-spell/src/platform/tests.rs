//! `PlatformChecker`'s own logic over a fake engine (the real ones run only on
//! macOS and Windows): what reaches the platform, the personal dictionary and
//! the system dictionary, ignores, script coverage, batching; and the
//! language-tag and script tables.

use std::sync::Mutex;

use super::*;

/// A platform engine that knows a fixed word list and records every call.
#[derive(Default)]
struct Fake {
    languages: Option<Vec<String>>,
    language_calls: std::sync::atomic::AtomicUsize,
    known: Mutex<Vec<String>>,
    calls: Mutex<Vec<String>>,
    batches: Mutex<Vec<usize>>,
}

impl Fake {
    fn new(languages: &[&str], known: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            languages: Some(languages.iter().map(|s| s.to_string()).collect()),
            known: Mutex::new(known.iter().map(|s| s.to_string()).collect()),
            ..Self::default()
        })
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Native for Fake {
    fn languages(&self) -> Option<Vec<String>> {
        self.language_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.languages.clone()
    }
    fn check_words(&self, words: Vec<String>) -> Vec<bool> {
        self.batches.lock().unwrap().push(words.len());
        let known = self.known.lock().unwrap();
        words
            .iter()
            .map(|w| {
                self.calls.lock().unwrap().push(format!("check {w}"));
                known.contains(w)
            })
            .collect()
    }
    fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("suggest {word} {max}"));
        ["one", "two", "three", "four"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
    fn learn(&self, word: &str) {
        self.calls.lock().unwrap().push(format!("learn {word}"));
        self.known.lock().unwrap().push(word.to_owned());
    }
    fn unlearn(&self, word: &str) {
        self.calls.lock().unwrap().push(format!("unlearn {word}"));
        self.known.lock().unwrap().retain(|w| w != word);
    }
    fn ignore(&self, word: &str) {
        self.calls.lock().unwrap().push(format!("ignore {word}"));
    }
    fn clear_ignored(&self) {
        self.calls.lock().unwrap().push("clear".into());
    }
}

fn checker(fake: &Arc<Fake>) -> PlatformChecker {
    PlatformChecker::from_native(Arc::clone(fake) as Arc<dyn Native>)
}

#[test]
fn the_platform_judges_words_in_covered_scripts() {
    let fake = Fake::new(&["en_US"], &["hello"]);
    let c = checker(&fake);
    assert!(c.check("hello"));
    assert!(!c.check("helo"));
    // Russian with English only: never asked, never marked (Hunspell's rule).
    assert!(c.check("привет"));
    assert_eq!(fake.calls(), ["check hello", "check helo"]);
    assert_eq!(c.languages(), ["en_US"]);
}

#[test]
fn a_language_with_no_known_script_hands_every_word_to_the_platform() {
    let fake = Fake::new(&["en_US", "xq"], &[]);
    let c = checker(&fake);
    assert!(!c.check("привет"));
    assert_eq!(fake.calls(), ["check привет"]);
}

#[test]
fn unknown_languages_are_asked_again_later() {
    use std::sync::atomic::Ordering::SeqCst;
    // The platform could not answer (macOS: the main thread was busy).
    let fake = Arc::new(Fake::default());
    let c = checker(&fake);
    assert!(
        c.covers("привет"),
        "rule off while the languages are unknown"
    );
    assert!(c.covers("x"));
    assert_eq!(
        fake.language_calls.load(SeqCst),
        1,
        "not before the retry time"
    );
    // Once the retry time has passed, asked again.
    *c.scripts.lock().unwrap() = Scripts::Unanswered(Instant::now());
    c.covers("x");
    assert_eq!(fake.language_calls.load(SeqCst), 2);
}

/// The review's probe: a platform that cannot name its languages (macOS,
/// the main thread busy past the wait) is asked once per batch, not once per
/// word, and not again on the next batch soon after: each ask may be a
/// main-queue hop with a 2 s wait.
#[test]
fn unanswered_languages_are_asked_once_per_batch_and_then_cached() {
    use std::sync::atomic::Ordering::SeqCst;
    let fake = Arc::new(Fake::default());
    let c = checker(&fake);
    let answers = c.check_batch(&["one", "two", "three", "four", "five"]);
    assert_eq!(answers.len(), 5);
    assert_eq!(fake.language_calls.load(SeqCst), 1, "one ask for the batch");
    c.check_batch(&["six", "seven"]);
    c.check("eight");
    assert_eq!(
        fake.language_calls.load(SeqCst),
        1,
        "the unanswered result is cached"
    );
}

/// A known answer is asked once for the checker's life.
#[test]
fn answered_languages_are_asked_once() {
    use std::sync::atomic::Ordering::SeqCst;
    let fake = Fake::new(&["en_US"], &["cat"]);
    let c = checker(&fake);
    c.check_batch(&["cat", "dog", "мир"]);
    c.check_batch(&["cow"]);
    assert_eq!(fake.language_calls.load(SeqCst), 1);
}

#[test]
fn a_batch_reaches_the_platform_in_one_call_without_what_rinch_accepts() {
    let fake = Fake::new(&["en_US"], &["cat"]);
    let c = checker(&fake);
    c.ignore("zork");
    c.personal_dictionary().extend(["rinch"]);
    let answers = c.check_batch(&["cat", "zork", "Rinch", "dgo", "мир", ""]);
    assert_eq!(answers, [true, true, true, false, true, true]);
    assert_eq!(*fake.batches.lock().unwrap(), [2]);
    assert_eq!(fake.calls(), ["ignore zork", "check cat", "check dgo"]);
}

#[test]
fn words_are_asked_in_nfc() {
    let fake = Fake::new(&["fr"], &["café"]);
    let c = checker(&fake);
    assert!(c.check("cafe\u{301}"));
    assert_eq!(fake.calls(), ["check café"]);
}

#[test]
fn add_goes_to_the_personal_and_the_system_dictionary() {
    let fake = Fake::new(&["en_US"], &[]);
    let personal = Arc::new(PersonalDictionary::new());
    let heard = Arc::new(Mutex::new(Vec::new()));
    let h = Arc::clone(&heard);
    personal.add_listener(move |c| h.lock().unwrap().push(c.clone()));
    let c = checker(&fake).with_personal_dictionary(Arc::clone(&personal));
    let spell = Arc::new(Mutex::new(Vec::new()));
    let s = Arc::clone(&spell);
    c.add_change_listener(Box::new(move |c| s.lock().unwrap().push(c.clone())));

    c.add_to_dictionary("rinch");
    assert!(c.check("rinch"));
    assert_eq!(personal.words(), ["rinch"]);
    assert_eq!(fake.calls(), ["learn rinch"], "rinch's dictionary answered");
    assert_eq!(
        *heard.lock().unwrap(),
        [PersonalDictionaryChange::Added("rinch".into())]
    );
    assert_eq!(
        *spell.lock().unwrap(),
        [SpellChange::Personal(PersonalDictionaryChange::Added(
            "rinch".into()
        ))]
    );

    // The app removing it unlearns it from the system too.
    personal.remove("rinch");
    assert!(!c.check("rinch"));
    assert_eq!(
        fake.calls(),
        ["learn rinch", "unlearn rinch", "check rinch"]
    );
}

#[test]
fn add_to_system_off_keeps_the_word_rinchs() {
    let fake = Fake::new(&["en_US"], &[]);
    let c = checker(&fake).add_to_system_dictionary(false);
    let spell = Arc::new(Mutex::new(0));
    let s = Arc::clone(&spell);
    c.add_change_listener(Box::new(move |_| *s.lock().unwrap() += 1));
    c.add_to_dictionary("rinch");
    assert!(c.check("rinch"));
    c.personal_dictionary().remove("rinch");
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
    assert_eq!(*spell.lock().unwrap(), 2, "the listeners fire either way");
}

#[test]
fn a_replaced_personal_dictionary_no_longer_forwards() {
    let fake = Fake::new(&["en_US"], &[]);
    let old = Arc::new(PersonalDictionary::new());
    let c = checker(&fake)
        .with_personal_dictionary(Arc::clone(&old))
        .with_personal_dictionary(Arc::new(PersonalDictionary::new()));
    old.add("stale");
    assert!(fake.calls().is_empty());
    drop(c);
}

#[test]
fn ignores_are_rinchs_and_clear() {
    let fake = Fake::new(&["en_US"], &[]);
    let c = checker(&fake);
    let spell = Arc::new(Mutex::new(Vec::new()));
    let s = Arc::clone(&spell);
    c.add_change_listener(Box::new(move |c| s.lock().unwrap().push(c.clone())));
    c.ignore("zork");
    c.ignore("zork");
    assert!(c.check("zork") && c.check("ZORK"));
    c.clear_ignored();
    c.clear_ignored();
    assert!(!c.check("zork"));
    assert_eq!(fake.calls(), ["ignore zork", "clear", "check zork"]);
    assert_eq!(
        *spell.lock().unwrap(),
        [
            SpellChange::Ignored("zork".into()),
            SpellChange::IgnoresCleared
        ]
    );
}

#[test]
fn suggestions_are_capped() {
    let fake = Fake::new(&["en_US"], &[]);
    let c = checker(&fake);
    assert_eq!(c.suggest("helo", 3), ["one", "two", "three"]);
    assert!(c.suggest("helo", 0).is_empty());
    assert_eq!(fake.calls(), ["suggest helo 3"]);
}

#[test]
fn language_tags_map_as_chromium_maps_them() {
    assert_eq!(to_mac_language("en_US").as_deref(), Some("en"));
    assert_eq!(to_mac_language("en-GB").as_deref(), Some("en_GB"));
    assert_eq!(to_mac_language("pt_PT").as_deref(), Some("pt"));
    assert_eq!(to_mac_language("pt-BR").as_deref(), Some("pt_BR"));
    assert_eq!(to_mac_language("pl").as_deref(), Some("pl_PL"));
    assert_eq!(to_mac_language("de").as_deref(), Some("de"));
    assert_eq!(from_mac_language("en").as_deref(), Some("en_US"));
    assert_eq!(from_mac_language("pt").as_deref(), Some("pt_PT"));
    assert_eq!(from_mac_language("pl_PL").as_deref(), Some("pl"));
    assert_eq!(from_mac_language("en_GB").as_deref(), Some("en_GB"));
    assert_eq!(from_mac_language("zh-Hans").as_deref(), Some("zh"));
    assert_eq!(to_bcp47("en_US.UTF-8").as_deref(), Some("en-US"));
    assert_eq!(to_bcp47("de").as_deref(), Some("de"));
    assert_eq!(to_bcp47("C"), None);
}

#[test]
fn languages_map_to_scripts() {
    use Script as S;
    assert_eq!(language_scripts("en_US"), Some(&[S::Latin][..]));
    assert_eq!(language_scripts("ru-RU"), Some(&[S::Cyrillic][..]));
    assert_eq!(language_scripts("sr"), Some(&[S::Cyrillic, S::Latin][..]));
    assert_eq!(
        language_scripts("ja"),
        Some(&[S::Han, S::Hiragana, S::Katakana][..])
    );
    assert_eq!(language_scripts("he"), Some(&[S::Hebrew][..]));
    assert_eq!(language_scripts("xq"), None);
    assert_eq!(
        scripts_of_languages(&["en_US".into(), "ru".into(), "de".into()]),
        Some(vec![S::Cyrillic, S::Latin])
    );
    assert_eq!(scripts_of_languages(&["en".into(), "xq".into()]), None);
}

#[test]
fn suggestions_interleave_across_languages() {
    let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let lists = [v(&["hole", "help", "helot"]), v(&["hallo", "help", "hell"])];
    assert_eq!(
        interleave_suggestions(&lists, 4),
        ["hole", "hallo", "help", "helot"]
    );
    assert_eq!(interleave_suggestions(&lists, 0), Vec::<String>::new());
    assert_eq!(interleave_suggestions(&[], 3), Vec::<String>::new());
}

#[test]
fn no_platform_checker_off_macos_and_windows() {
    if !AVAILABLE {
        assert!(PlatformChecker::new().is_none());
        assert!(PlatformChecker::with_languages(["en_US"]).is_none());
    }
}

#[test]
fn windows_languages_are_matched_to_installed_spellcheckers() {
    let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let supported = v(&["en-GB", "en-US", "fr-FR", "ko-KR"]);
    assert_eq!(
        choose_languages(&v(&["en-US", "ko", "de-DE", "en_us"]), &supported),
        v(&["en-US", "ko-KR"])
    );
    // A region the platform lacks takes the language's first one.
    assert_eq!(choose_languages(&v(&["fr-CA"]), &supported), v(&["fr-FR"]));
    assert_eq!(choose_languages(&v(&["en"]), &supported), v(&["en-GB"]));
    assert!(choose_languages(&v(&["C", ""]), &supported).is_empty());
}

#[test]
fn multi_sz_buffers_split() {
    let buf: Vec<u16> = "en-US\0ko\0\0".encode_utf16().collect();
    assert_eq!(split_multi_sz(&buf), ["en-US", "ko"]);
    assert!(split_multi_sz(&[0, 0]).is_empty());
}
