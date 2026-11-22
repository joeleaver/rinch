# Spellcheck (`rinch-spell`)

Read this page when your task touches spellchecking: `crates/rinch-spell`, or
the stages of #2862 that build on it (marking misspelled words in fields and
the editor, the context menu's suggestions, platform backends). Stage 1, the
engine, is all that exists so far: no UI, no DOM, nothing in `rinch` uses it
yet and `rinch` does not re-export it.

## Shape

`rinch-spell` depends on no rinch crate. It builds for wasm32 and Android:
the core path never touches the filesystem.

- `src/lib.rs`: the `SpellChecker` trait, `misspelled()` (segment, then
  check), `DEFAULT_MAX_SUGGESTIONS` (5, Chrome's `kMaxSuggestions`).
- `src/segment.rs`: `words(text, &SkipRules)`, the byte ranges to check.
- `src/hunspell.rs`: `HunspellDictionary` (one parsed `.aff`/`.dic` pair and
  its language tag) and `HunspellChecker` (the trait over any number of them).
- `src/personal.rs`: `PersonalDictionary` and the internal `WordSet` that
  the session ignore list also uses.
- `src/system.rs`: installed-dictionary discovery by locale. Compiled only with
  the `system-dictionaries` feature (default) and never on wasm32.

## The trait

`SpellChecker: Send + Sync` with `check`, `suggest(word, max)`,
`add_to_dictionary`, `ignore`, `languages`. The mutating calls take `&self`
(interior mutability, `RwLock`s), so one `Arc<dyn SpellChecker>` can be shared
by the UI thread and a checking thread; `Arc<T>` implements the trait too.
The trait checks single words; segmentation is `words()`'s job.

`spellbook::Dictionary` (0.4.2) is `Send + Sync`: the type that holds a raw pointer,
`UmbraSlice`, has `unsafe impl Send/Sync`, and the rest is `hashbrown` maps
and vectors. `tests/hunspell.rs` `checker_is_send_and_sync` asserts it for
every public type and checks from a spawned thread. Platform backends (stage 4)
whose native API is main-thread-only will need a forwarding wrapper to meet
the bound.

## Checking a word (`HunspellChecker::check`)

In order, the first that accepts wins:

1. No dictionary loaded: every word is correct (nothing to mark).
2. The session ignore list, then the personal dictionary (both with the casing
   rule below), then any loaded dictionary (`spellbook`'s `check`, which
   applies Hunspell's own casing: "Hello" and "HELLO" for "hello", not
   "paris" for "Paris").
3. The same with every `’` replaced by `'`: most dictionaries list only the
   ASCII apostrophe (en_US has an `ICONV ’ '` line, the test fixture
   deliberately does not).
4. The contraction fallback (Chrome's `IsValidContraction` in
   `spellcheck_language.cc`): a word UAX #29 kept whole across punctuation
   ("hello:world", "in'n'out") is correct when each part is. Off with
   `contraction_fallback(false)`.

Several dictionaries are several languages: a word is correct if any accepts
it. `suggest` merges each dictionary's suggestions in load order, without
repeats, up to `max`, and gives none for a word over 24 bytes (Chrome's
`kMaxSuggestLen`, `max_suggest_len(None)` lifts it). Suggestion search is
the slow part of Hunspell; check is cheap.

**Casing rule** of the personal dictionary and the ignore list (Hunspell's):
a lowercase entry accepts Titlecase and ALL CAPS, a Titlecase entry accepts
ALL CAPS, nothing accepts less capitalisation than it was stored with.

## Segmentation and skip rules

`words()` walks `unicode-segmentation`'s `split_word_bound_indices` (UAX #29;
the crate was already in the graph through `rinch-words` and winit). UAX #29
keeps `'` and `’` between letters inside the word, so "don't"/"don’t" are one
word and "dogs'" is "dogs". Ranges are byte offsets into the input on char
boundaries; stage 2 maps them onto editor positions and field text.

`SkipRules` (a plain struct, every field public, `Default` and `none()`):

| Rule | Default | Source |
|---|---|---|
| `urls` (`scheme://…`, `www.…`, `mailto:…`, whole whitespace token, wrapping punctuation trimmed) | on | issue #2862 |
| `emails` (`local@domain.tld`, whole token) | on | issue #2862 |
| `digits` (word contains a digit) | on | issue #2862 |
| `all_caps` (two or more cased letters, none lowercase) | on | issue #2862 |
| `max_len` (UTF-8 bytes) | 64 | Chrome `kMaxCheckedLen`, `hunspell_engine.cc` |
| a run with no letter (punctuation, numbers) | always skipped | — |

Chrome itself has no digit, all-caps, URL or email rule in
`components/spellcheck/renderer` (checked 2026-11-22): its word iterator
treats ASCII digits as word characters and hands the word to Hunspell, which
accepts pure numbers only. The four defaults above come from the issue's
decision, not from Chrome; an app that wants Chrome's exact behaviour turns
them off.

## Dictionaries

`HunspellDictionary::from_bytes(lang, aff, dic)` is the bundling path. The
`.aff`'s `SET` line names the encoding of both files: UTF-8 (or no `SET`) and
ISO 8859-1 are decoded; any other encoding works only while the bytes are
ASCII, else `DictionaryError::UnsupportedEncoding`. spellbook itself takes
only `&str`. Loading a real dictionary is not free (the host smoke test, loading
en_US in a debug build, takes about 0.2 s), so stage 2 should load off the UI
thread.

`system` searches, in order: `$DICPATH`, `$XDG_DATA_HOME/hunspell`
(`~/.local/share/hunspell`), then `hunspell`, `myspell`, `myspell/dicts` under
each `$XDG_DATA_DIRS` entry (`/usr/local/share:/usr/share` by default).
`find_in(dirs, locale)` normalises the locale (`en-US`, `en_US.UTF-8` →
`en_US`), takes the exact name, then the bare language, then the first
regional dictionary of that language by name. `user_locales()` reads
`$LANGUAGE`, `$LC_ALL`, `$LC_MESSAGES`, `$LANG`. Tests use `find_in` over
`tests/fixtures`; the one host test (`host_en_us_smoke`) returns early when
the host has no `en_US`.

## The personal dictionary

In memory, app-owned. `PersonalDictionary::with_words`/`extend` load the
app's saved list without notifying; `add`/`remove` notify the listener set
with `set_listener` (one `PersonalDictionaryChange` per real change, called
with no lock held, so it may call `words()` to export). Share one `Arc` between
the app and the checker (`with_personal_dictionary`). The session ignore list
lives in the checker (`ignore`, `clear_ignored`) and is never exported.

## Tests

`tests/fixtures/en_TEST.{aff,dic}` is a 39-word dictionary written for these
tests (two suffix rules, a `REP`), with `xx_TEST` (a second language) and
`la_TEST` (ISO 8859-1). Never assert against the host's dictionaries.

## What stage 2 builds on

- Run `words()` over a changed text run and `check()` each range off the
  keystroke path; `misspelled()` is the one-call form.
- The editor plugin turns ranges into wavy decorations (decoration ranges map
  through transactions, so only changed words need re-checking).
- `rinch-web` never runs this engine: the browser checks.
- The context menu (stage 3) calls `suggest(word, DEFAULT_MAX_SUGGESTIONS)` and
  `add_to_dictionary`.
