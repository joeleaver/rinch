//! System dictionary discovery, against the vendored fixtures directory (a
//! stand-in for `/usr/share/hunspell`), plus one smoke test of the host's
//! `en_US` that skips when the host has none.
#![cfg(all(feature = "system-dictionaries", not(target_family = "wasm")))]

use std::path::PathBuf;

use rinch_spell::{HunspellChecker, Script, SkipRules, SpellChecker, misspelled, system};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn finds_dictionaries_by_locale() {
    let dirs = [PathBuf::from("/nonexistent/rinch-spell"), fixtures()];
    let found = system::find_in(&dirs, "en-TEST.UTF-8").expect("en_TEST");
    assert_eq!(found.name, "en_TEST");
    assert_eq!(found.aff, fixtures().join("en_TEST.aff"));
    // A bare language takes the first regional dictionary.
    assert_eq!(system::find_in(&dirs, "xx").unwrap().name, "xx_TEST");
    assert_eq!(system::find_in(&dirs, "zz_ZZ"), None);
    assert_eq!(system::find_in(&dirs, "C"), None);

    let names: Vec<_> = system::list_in(&dirs).into_iter().map(|f| f.name).collect();
    assert_eq!(names, ["en_TEST", "la_TEST", "ru_TEST", "xx_TEST"]);

    let dict = found.load().unwrap();
    assert_eq!(dict.language(), "en_TEST");
    assert!(dict.check("cities"));
}

#[test]
fn host_en_us_smoke() {
    // Host-only: proves the real loader reads a distribution's dictionary.
    // Asserts nothing about words beyond the most common ones.
    let Some(files) = system::find("en_US") else {
        eprintln!("no en_US Hunspell dictionary on this host; skipped");
        return;
    };
    let dict = files.load().unwrap();
    eprintln!("host en_US scripts: {:?}", dict.scripts());
    assert_eq!(dict.scripts(), [Script::Latin]);
    let checker = HunspellChecker::new().with_dictionary(dict);
    assert!(checker.check("the"));
    assert!(!checker.check("qwzxv"));
    // The review's repro: Russian under an English dictionary is unmarked.
    let marked = misspelled(&checker, "Привет мир", &SkipRules::default()).count();
    assert_eq!(marked, 0);
}
