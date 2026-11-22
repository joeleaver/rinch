//! Unicode properties the checker needs: scripts, marks, NFC.

use std::borrow::Cow;
use std::collections::BTreeMap;

use icu_normalizer::ComposingNormalizerBorrowed;
use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, Script};

/// The script of `c` (UAX #24 `Script`).
pub(crate) fn script_of(c: char) -> Script {
    CodePointMapData::<Script>::new().get(c)
}

/// A script that says which writing system a letter belongs to (not
/// `Common`, `Inherited` or `Unknown`).
fn is_specific(script: Script) -> bool {
    script != Script::Common && script != Script::Inherited && script != Script::Unknown
}

/// The specific scripts of the letters of `word`.
pub(crate) fn letter_scripts(word: &str) -> impl Iterator<Item = Script> + '_ {
    word.chars()
        .filter(|c| c.is_alphabetic())
        .map(script_of)
        .filter(|s| is_specific(*s))
}

/// A combining mark (general category Mn, Mc or Me): a virama, a vowel sign,
/// an accent. It belongs to the letter before it.
pub(crate) fn is_mark(c: char) -> bool {
    matches!(
        CodePointMapData::<GeneralCategory>::new().get(c),
        GeneralCategory::NonspacingMark
            | GeneralCategory::SpacingMark
            | GeneralCategory::EnclosingMark
    )
}

/// `text` in Normalization Form C ("e" + U+0301 becomes "é"), borrowed when
/// it already is.
pub(crate) fn nfc(text: &str) -> Cow<'_, str> {
    ComposingNormalizerBorrowed::new_nfc().normalize(text)
}

/// The scripts a dictionary writes in, from its text: every script of a
/// letter on the `.aff`'s `TRY` line (the language's alphabet, by Hunspell's
/// convention), and every script that at least 1% of the `.dic`'s words use
/// (so a handful of loanwords in another script, "µ" or "Ω" in an English
/// list, does not claim that script). Sorted.
pub(crate) fn dictionary_scripts(aff: &str, dic: &str) -> Vec<Script> {
    let mut scripts: Vec<Script> = Vec::new();
    for line in aff.lines() {
        if let Some(letters) = line.trim_start().strip_prefix("TRY")
            && letters.starts_with(char::is_whitespace)
        {
            scripts.extend(letter_scripts(letters));
        }
    }
    let mut counts: BTreeMap<Script, usize> = BTreeMap::new();
    let mut total = 0usize;
    // The first line is the word count.
    for line in dic.lines().skip(1) {
        // The word ends at an unescaped '/' (flags) or whitespace (morphology).
        let word = line.split(['/', '\t', ' ']).next().unwrap_or("");
        if word.is_empty() {
            continue;
        }
        total += 1;
        let mut seen: Vec<Script> = Vec::new();
        for script in letter_scripts(word) {
            if !seen.contains(&script) {
                seen.push(script);
                *counts.entry(script).or_default() += 1;
            }
        }
    }
    scripts.extend(
        counts
            .into_iter()
            .filter(|&(_, n)| n * 100 >= total)
            .map(|(script, _)| script),
    );
    scripts.sort();
    scripts.dedup();
    scripts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_come_from_try_and_the_bulk_of_the_word_list() {
        // 200 Latin words and one Greek one: Greek is under 1%.
        let mut dic = String::from("201\n");
        for i in 0..200 {
            dic.push_str(&format!("word{i}/S\n"));
        }
        dic.push_str("Ωmega\n");
        assert_eq!(dictionary_scripts("SET UTF-8\n", &dic), [Script::Latin]);
        // The TRY line counts whatever the word list says.
        assert_eq!(
            dictionary_scripts("TRY abcж\n", "1\nx\n"),
            [Script::Cyrillic, Script::Latin]
        );
        // A bilingual list keeps both.
        assert_eq!(
            dictionary_scripts("", "4\nмир\nкот\nhello\nworld\n"),
            [Script::Cyrillic, Script::Latin]
        );
    }

    #[test]
    fn marks_and_nfc() {
        assert!(is_mark('\u{094D}'), "Devanagari virama");
        assert!(is_mark('\u{0301}'));
        assert!(!is_mark('a'));
        assert_eq!(nfc("cafe\u{301}"), "café");
        assert!(matches!(nfc("café"), Cow::Borrowed(_)));
    }
}
