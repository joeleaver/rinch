//! Which runs of a text are words to check.
//!
//! [`words`] walks a `&str` on Unicode word boundaries (UAX #29, through
//! `unicode-segmentation`) and yields the byte range of every word a
//! spellchecker should look at, dropping what [`SkipRules`] says to skip.
//! UAX #29 already keeps an apostrophe between letters inside the word, both
//! the ASCII `'` and the typographic `’` (U+2019), so "don't" and "don’t" are
//! one word each; a leading or trailing apostrophe ("dogs'") is not part of it.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// Chrome's longest checked word, in UTF-8 bytes (`kMaxCheckedLen` in
/// `components/spellcheck/renderer/hunspell_engine.cc`): a longer word is
/// reported correct, since no suggestion could be offered for it either.
pub const CHROME_MAX_CHECKED_LEN: usize = 64;

/// What [`words`] leaves out. Every rule is the app's to change.
///
/// [`Default`] skips URLs, email addresses, words with digits and all-caps
/// words, and words longer than [`CHROME_MAX_CHECKED_LEN`] bytes. A run with
/// no letter at all (punctuation, a number) is never a word, whatever the
/// rules say.
///
/// The URL, email, digit and all-caps rules are a deliberate exception to
/// rinch's "Chrome is the default" rule: Chrome checks these; rinch skips them
/// by default for app UI, where they mark false errors in fields (Joe,
/// 2026-11-22, #2862). [`SkipRules::none`] turns all of them off, for Chrome's
/// behaviour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkipRules {
    /// Skip URLs: a whitespace-delimited token with a `scheme://`, or one
    /// starting `www.` or `mailto:`.
    pub urls: bool,
    /// Skip email addresses: a token `local@domain` whose domain has a dot.
    pub emails: bool,
    /// Skip a word that contains a digit ("mp3", "4th", "abc123").
    pub digits: bool,
    /// Skip a word of two or more letters that are all uppercase ("NASA",
    /// "HTTP's" is not all caps).
    pub all_caps: bool,
    /// Skip a word longer than this many UTF-8 bytes; `None` checks every
    /// length.
    pub max_len: Option<usize>,
}

impl Default for SkipRules {
    fn default() -> Self {
        Self {
            urls: true,
            emails: true,
            digits: true,
            all_caps: true,
            max_len: Some(CHROME_MAX_CHECKED_LEN),
        }
    }
}

impl SkipRules {
    /// Rules that skip nothing but runs without a letter.
    pub fn none() -> Self {
        Self {
            urls: false,
            emails: false,
            digits: false,
            all_caps: false,
            max_len: None,
        }
    }

    /// Whether `word` (one segment from UAX #29) is skipped by the per-word
    /// rules: no letter, a digit, all caps, too long.
    pub fn skips_word(&self, word: &str) -> bool {
        if !word.chars().any(char::is_alphabetic) {
            return true;
        }
        if self.max_len.is_some_and(|max| word.len() > max) {
            return true;
        }
        if self.digits && word.chars().any(char::is_numeric) {
            return true;
        }
        if self.all_caps && is_all_caps(word) {
            return true;
        }
        false
    }
}

/// Two or more cased letters, none of them lowercase.
fn is_all_caps(word: &str) -> bool {
    let mut cased = 0usize;
    for c in word.chars() {
        if c.is_lowercase() {
            return false;
        }
        if c.is_uppercase() {
            cased += 1;
        }
    }
    cased >= 2
}

/// The byte ranges of the words in `text` that a spellchecker should check,
/// in order. Each range is a byte offset into `text` and lies on `char`
/// boundaries.
pub fn words<'a>(text: &'a str, rules: &'a SkipRules) -> impl Iterator<Item = Range<usize>> + 'a {
    let skipped = skipped_tokens(text, rules);
    let mut next_skip = 0usize;
    text.split_word_bound_indices()
        .filter_map(move |(start, word)| {
            let range = start..start + word.len();
            while next_skip < skipped.len() && skipped[next_skip].end <= range.start {
                next_skip += 1;
            }
            if let Some(skip) = skipped.get(next_skip)
                && skip.start < range.end
            {
                return None;
            }
            (!rules.skips_word(word)).then_some(range)
        })
}

/// The whitespace-delimited tokens (punctuation around them trimmed) that are
/// URLs or email addresses under `rules`, in order.
fn skipped_tokens(text: &str, rules: &SkipRules) -> Vec<Range<usize>> {
    if !rules.urls && !rules.emails {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut pos = 0usize;
    for token in text.split(char::is_whitespace) {
        let start = pos;
        pos += token.len()
            + text[start + token.len()..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        let lead = token.len() - token.trim_start_matches(is_wrapper).len();
        let trimmed = token
            .trim_start_matches(is_wrapper)
            .trim_end_matches(is_trailer);
        if trimmed.is_empty() {
            continue;
        }
        if (rules.urls && is_url(trimmed)) || (rules.emails && is_email(trimmed)) {
            let s = start + lead;
            out.push(s..s + trimmed.len());
        }
    }
    out
}

/// Opening punctuation a URL or address may sit inside.
fn is_wrapper(c: char) -> bool {
    matches!(c, '(' | '[' | '{' | '<' | '"' | '\'' | '“' | '‘' | '«')
}

/// Closing punctuation after a URL or address that is not part of it.
fn is_trailer(c: char) -> bool {
    matches!(
        c,
        ')' | ']' | '}' | '>' | '"' | '\'' | '”' | '’' | '»' | '.' | ',' | ';' | ':' | '!' | '?'
    )
}

fn is_url(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("www.") || lower.starts_with("mailto:") {
        return true;
    }
    match token.find("://") {
        Some(i) if i > 0 => {
            let scheme = &token[..i];
            scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        _ => false,
    }
}

fn is_email(token: &str) -> bool {
    let Some((local, domain)) = token.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.contains('@')
        && domain
            .split_once('.')
            .is_some_and(|(head, tail)| !head.is_empty() && !tail.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found<'a>(text: &'a str, rules: &SkipRules) -> Vec<&'a str> {
        words(text, rules).map(|r| &text[r]).collect()
    }

    #[test]
    fn splits_on_word_boundaries_with_byte_ranges() {
        let text = "Héllo, wörld!  ünd";
        let ranges: Vec<_> = words(text, &SkipRules::default()).collect();
        assert_eq!(ranges, vec![0..6, 8..14, 17..21]);
        assert_eq!(
            found(text, &SkipRules::default()),
            ["Héllo", "wörld", "ünd"]
        );
    }

    #[test]
    fn apostrophes_inside_a_word_keep_it_whole() {
        let rules = SkipRules::default();
        assert_eq!(found("I don't know", &rules), ["I", "don't", "know"]);
        assert_eq!(found("I don’t know", &rules), ["I", "don’t", "know"]);
        // A leading or trailing apostrophe is not part of the word.
        assert_eq!(found("the dogs' 'bone'", &rules), ["the", "dogs", "bone"]);
    }

    #[test]
    fn skips_urls_and_emails_as_whole_tokens() {
        let rules = SkipRules::default();
        assert_eq!(
            found("see https://exampel.com/pathh?q=wurd now", &rules),
            ["see", "now"]
        );
        assert_eq!(found("(www.exampel.org), ok", &rules), ["ok"]);
        assert_eq!(found("mail jon.doe@exampel.com.", &rules), ["mail"]);
        assert_eq!(found("mailto:jon@x.io hi", &rules), ["hi"]);
        // Not an address without a dotted domain; not a URL without a scheme.
        assert_eq!(found("a@b", &rules), ["a", "b"]);
        assert_eq!(found("ratio 3://x", &rules), ["ratio", "x"]);
        let off = SkipRules {
            urls: false,
            emails: false,
            ..SkipRules::default()
        };
        assert_eq!(found("https://exampel.com", &off), ["https", "exampel.com"]);
        assert_eq!(found("jon@exampel.com", &off), ["jon", "exampel.com"]);
    }

    #[test]
    fn skips_digits_caps_and_long_words_by_rule() {
        let rules = SkipRules::default();
        assert_eq!(found("mp3 4th abc123 42 3.14 ok", &rules), ["ok"]);
        assert_eq!(found("NASA HTTP's I Ok", &rules), ["HTTP's", "I", "Ok"]);
        let long = "a".repeat(65);
        let text = format!("{long} b");
        assert_eq!(found(&text, &rules), ["b"]);
        let exactly = "a".repeat(64);
        assert_eq!(found(&exactly, &rules), [exactly.as_str()]);

        let none = SkipRules::none();
        assert_eq!(found("mp3 NASA 42", &none), ["mp3", "NASA"]);
        assert_eq!(found(&text, &none), [long.as_str(), "b"]);
    }

    #[test]
    fn punctuation_and_numbers_are_never_words() {
        let none = SkipRules::none();
        assert_eq!(found("— ... 1,000 !!", &none), Vec::<&str>::new());
    }

    #[test]
    fn scripts_without_spaces_split_into_pieces() {
        // The known limit in docs/internals/spellcheck.md: no dictionary-based
        // segmentation, so one Thai word is several pieces.
        let pieces = found("สวัสดี", &SkipRules::none());
        assert_eq!(pieces.len(), 4, "{pieces:?}");
        assert_eq!(pieces.concat(), "สวัสดี");
    }
}
