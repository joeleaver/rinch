//! The Hunspell backend against a tiny dictionary vendored for these tests
//! (`tests/fixtures/en_TEST.*`), never the host's: CI's machine has other
//! dictionaries, or none.

use std::sync::{Arc, Mutex};

use rinch_spell::{
    DictionaryError, HunspellChecker, HunspellDictionary, PersonalDictionary,
    PersonalDictionaryChange, Script, SkipRules, SpellChange, SpellChecker, misspelled,
};

const EN_AFF: &[u8] = include_bytes!("fixtures/en_TEST.aff");
const EN_DIC: &[u8] = include_bytes!("fixtures/en_TEST.dic");
const XX_AFF: &[u8] = include_bytes!("fixtures/xx_TEST.aff");
const XX_DIC: &[u8] = include_bytes!("fixtures/xx_TEST.dic");

fn en() -> HunspellChecker {
    HunspellChecker::from_bytes("en_TEST", EN_AFF, EN_DIC).unwrap()
}

fn bad<'a>(checker: &dyn SpellChecker, text: &'a str) -> Vec<&'a str> {
    misspelled(checker, text, &SkipRules::default())
        .map(|r| &text[r])
        .collect()
}

#[test]
fn checks_stems_and_affixed_forms() {
    let c = en();
    for word in ["hello", "cats", "cities", "walked", "baked", "jumps", "the"] {
        assert!(c.check(word), "{word} should be correct");
    }
    for word in ["helo", "citys", "bakeed", "wlak", "teh"] {
        assert!(!c.check(word), "{word} should be misspelled");
    }
}

#[test]
fn casing_follows_hunspell() {
    let c = en();
    assert!(c.check("Hello"), "sentence-initial capital");
    assert!(c.check("HELLO"), "all caps of a lowercase word");
    assert!(c.check("Paris"));
    assert!(!c.check("paris"), "a proper noun stays capitalised");
}

#[test]
fn misspelled_ranges_are_byte_offsets() {
    let c = en();
    let text = "Thé quick brwn fox jumpd over the lazzy dog.";
    let ranges: Vec<_> = misspelled(&c, text, &SkipRules::default()).collect();
    // "Thé" is two bytes longer than three chars would suggest.
    assert_eq!(ranges, vec![0..4, 11..15, 20..25, 35..40]);
    assert_eq!(bad(&c, text), ["Thé", "brwn", "jumpd", "lazzy"]);
}

#[test]
fn typographic_apostrophe_falls_back_to_ascii() {
    // The fixture lists only "don't"; Chrome and most dictionaries do too.
    let c = en();
    assert!(c.check("don't"));
    assert!(c.check("don’t"));
    assert!(!c.check("dont’t"));
    assert_eq!(bad(&c, "I don’t see it’s wrong"), Vec::<&str>::new());
}

#[test]
fn joined_words_are_correct_when_each_part_is() {
    let c = en();
    assert!(c.check("hello:world"));
    assert!(c.check("apple.house"));
    assert!(!c.check("hello:wurld"));
    let strict = en().contraction_fallback(false);
    assert!(!strict.check("hello:world"));
    assert!(strict.check("don’t"), "the apostrophe fallback is separate");
}

#[test]
fn skip_rules_apply_before_the_dictionary() {
    let c = en();
    assert_eq!(
        bad(&c, "teh ABCD x86 https://wrold.example/teh jon@exmaple.com"),
        ["teh"]
    );
    let everything = SkipRules::none();
    let text = "ABCD x86";
    let ranges: Vec<_> = misspelled(&c, text, &everything)
        .map(|r| &text[r])
        .collect();
    assert_eq!(ranges, ["ABCD", "x86"]);
}

#[test]
fn suggestions_are_bounded_and_ranked() {
    let c = en();
    let s = c.suggest("wrold", 5);
    assert_eq!(s.first().map(String::as_str), Some("world"), "{s:?}");
    assert!(c.suggest("wrold", 1).len() <= 1);
    assert!(c.suggest("wrold", 0).is_empty());
    // REP f -> ph.
    assert!(c.suggest("fone", 5).contains(&"phone".to_owned()));
    // Chrome asks for no suggestion on a word over 24 bytes.
    let long = "helloooooooooooooooooooooo"; // 26 bytes
    assert!(c.suggest(long, 5).is_empty());
    let unlimited = en().max_suggest_len(None);
    assert!(!unlimited.suggest("helloo", 5).is_empty());
}

#[test]
fn personal_dictionary_is_shared_and_reports_changes() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let personal = Arc::new(PersonalDictionary::with_words(["rinch"]));
    let sink = Arc::clone(&heard);
    personal.add_listener(move |change| sink.lock().unwrap().push(change.clone()));

    let c = en().with_personal_dictionary(Arc::clone(&personal));
    assert!(c.check("rinch"), "loaded words are accepted");
    assert!(c.check("Rinch"));
    assert!(!c.check("taffy"));
    c.add_to_dictionary("taffy");
    assert!(c.check("taffy"));
    assert!(personal.contains("taffy"), "the app's handle sees the add");
    assert_eq!(
        *heard.lock().unwrap(),
        [PersonalDictionaryChange::Added("taffy".into())]
    );
    assert_eq!(personal.words(), ["rinch", "taffy"]);
    personal.remove("taffy");
    assert!(!c.check("taffy"));
}

#[test]
fn session_ignores_are_separate_from_the_personal_dictionary() {
    let c = en();
    assert!(!c.check("parley"));
    c.ignore("parley");
    assert!(c.check("parley"));
    assert!(
        c.personal_dictionary().is_empty(),
        "an ignore is never saved"
    );
    c.clear_ignored();
    assert!(!c.check("parley"));
}

#[test]
fn several_languages_any_accepts() {
    let xx = HunspellDictionary::from_bytes("xx_TEST", XX_AFF, XX_DIC).unwrap();
    let c = en().with_dictionary(xx);
    assert_eq!(c.languages(), ["en_TEST", "xx_TEST"]);
    assert_eq!(bad(&c, "hello monde, bonjour wrold"), ["wrold"]);
    let mut c = c;
    assert!(c.remove_language("xx_TEST"));
    assert!(!c.check("monde"));
}

#[test]
fn no_dictionary_marks_nothing() {
    let c = HunspellChecker::new();
    assert!(c.check("qwzx"));
    assert!(c.suggest("qwzx", 5).is_empty());
    assert!(c.languages().is_empty());
}

#[test]
fn latin1_dictionaries_load() {
    let aff = include_bytes!("fixtures/la_TEST.aff");
    let dic = include_bytes!("fixtures/la_TEST.dic");
    let c = HunspellChecker::from_bytes("la_TEST", aff, dic).unwrap();
    assert!(c.check("café"));
    assert!(!c.check("cafe"));
}

#[test]
fn bad_dictionaries_are_errors() {
    assert!(matches!(
        HunspellDictionary::from_bytes("x", b"SET UTF-8\n", b"1\ncaf\xe9\n"),
        Err(DictionaryError::InvalidEncoding { file: ".dic" })
    ));
    assert!(matches!(
        HunspellDictionary::from_bytes("x", b"SET KOI8-R\n", b"1\n\xc1\n"),
        Err(DictionaryError::UnsupportedEncoding(_))
    ));
    assert!(matches!(
        HunspellDictionary::from_bytes("x", b"FLAG bogus\n", b"1\nword/A\n"),
        Err(DictionaryError::Parse(_))
    ));
}

#[test]
fn checker_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<HunspellChecker>();
    assert_send_sync::<HunspellDictionary>();
    assert_send_sync::<PersonalDictionary>();
    // spellbook's own dictionary, which stage 2 may check from a thread.
    let checker: Arc<dyn SpellChecker> = Arc::new(en());
    let remote = Arc::clone(&checker);
    let from_thread = std::thread::spawn(move || remote.check("wrold"))
        .join()
        .unwrap();
    assert!(!from_thread);
}

// ---- Review fixes (#2863) ----

const RU_AFF: &[u8] = include_bytes!("fixtures/ru_TEST.aff");
const RU_DIC: &[u8] = include_bytes!("fixtures/ru_TEST.dic");

#[test]
fn words_in_scripts_no_dictionary_covers_are_not_marked() {
    let c = en();
    // Russian, Japanese, Hebrew, Arabic, Thai, Greek: no Latin letter.
    for text in [
        "Привет мир",
        "日本語の文章",
        "שלום עולם",
        "مرحبا بالعالم",
        "สวัสดีครับ",
        "γεια σου",
    ] {
        assert_eq!(bad(&c, text), Vec::<&str>::new(), "{text}");
    }
    // A word with a Latin letter is still checked.
    assert_eq!(bad(&c, "Привет wrold"), ["wrold"]);
}

#[test]
fn a_dictionary_for_the_script_checks_it() {
    let ru = HunspellDictionary::from_bytes("ru_TEST", RU_AFF, RU_DIC).unwrap();
    let c = en().with_dictionary(ru);
    assert_eq!(bad(&c, "Привет мир, Првиет wrold"), ["Првиет", "wrold"]);
}

#[test]
fn decomposed_text_is_normalised_before_checking() {
    let aff = include_bytes!("fixtures/la_TEST.aff");
    let dic = include_bytes!("fixtures/la_TEST.dic");
    let c = HunspellChecker::from_bytes("la_TEST", aff, dic).unwrap();
    assert!(c.check("café"));
    assert!(c.check("cafe\u{301}"), "e + COMBINING ACUTE is café");
    let p = en();
    p.add_to_dictionary("nai\u{308}ve");
    assert!(p.check("naïve"), "the personal dictionary normalises too");
}

#[test]
fn suggestions_keep_a_typographic_apostrophe() {
    let c = en();
    let s = c.suggest("do’nt", 5);
    assert!(s.contains(&"don’t".to_owned()), "{s:?}");
    assert!(!s.iter().any(|w| w.contains('\'')), "{s:?}");
    let ascii = c.suggest("do'nt", 5);
    assert!(ascii.contains(&"don't".to_owned()), "{ascii:?}");
}

#[test]
fn scripts_are_derived_and_overridable() {
    let en = HunspellDictionary::from_bytes("en_TEST", EN_AFF, EN_DIC).unwrap();
    assert_eq!(en.scripts(), [Script::Latin]);
    let ru = HunspellDictionary::from_bytes("ru_TEST", RU_AFF, RU_DIC).unwrap();
    assert_eq!(ru.scripts(), [Script::Cyrillic]);
    // An app may say an English dictionary also judges Cyrillic words.
    let wide = en.with_scripts([Script::Latin, Script::Cyrillic]);
    let c = HunspellChecker::new().with_dictionary(wide);
    assert_eq!(bad(&c, "Привет wrold"), ["Привет", "wrold"]);
}

#[test]
fn change_listeners_hear_personal_and_ignore_changes() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let personal = Arc::new(PersonalDictionary::new());
    let c: Arc<dyn SpellChecker> = Arc::new(en().with_personal_dictionary(Arc::clone(&personal)));
    let sink = Arc::clone(&heard);
    let id = c
        .add_change_listener(Box::new(move |change| {
            sink.lock().unwrap().push(change.clone())
        }))
        .expect("the Hunspell checker reports changes");
    // The app persisting the list listens alongside the checker.
    let saved = Arc::new(Mutex::new(0));
    let s = Arc::clone(&saved);
    personal.add_listener(move |_| *s.lock().unwrap() += 1);

    c.add_to_dictionary("taffy");
    personal.add("parley"); // an add the app makes itself is heard too
    c.ignore("stylo");
    c.ignore("stylo"); // not new: not heard
    assert!(c.remove_change_listener(id));
    c.ignore("vello");
    assert_eq!(
        *heard.lock().unwrap(),
        [
            SpellChange::Personal(PersonalDictionaryChange::Added("taffy".into())),
            SpellChange::Personal(PersonalDictionaryChange::Added("parley".into())),
            SpellChange::Ignored("stylo".into()),
        ]
    );
    assert_eq!(*saved.lock().unwrap(), 2);
}

#[test]
fn clearing_ignores_is_heard_and_a_dropped_checker_unsubscribes() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let personal = Arc::new(PersonalDictionary::new());
    let c = en().with_personal_dictionary(Arc::clone(&personal));
    let sink = Arc::clone(&heard);
    c.add_change_listener(Box::new(move |change| {
        sink.lock().unwrap().push(change.clone())
    }));
    c.clear_ignored(); // nothing ignored: nothing heard
    c.ignore("stylo");
    c.clear_ignored();
    drop(c);
    personal.add("after-drop");
    assert_eq!(
        *heard.lock().unwrap(),
        [
            SpellChange::Ignored("stylo".into()),
            SpellChange::IgnoresCleared
        ]
    );
}
