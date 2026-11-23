# Spellcheck (`rinch-spell`)

Read this page when your task touches spellchecking: `crates/rinch-spell`, or
the stages of #2862 that build on it (marking misspelled words in fields and
the editor, the context menu's suggestions, platform backends). Stage 1 is
the engine; stage 2 marks misspelled words in text fields ("Marking text
fields" below). `rinch` re-exports the crate as `rinch::spellcheck`.

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
- `src/listeners.rs`: the multi-subscriber listener list (`ListenerId`).
- `src/unicode.rs`: scripts (`icu_properties`), marks, NFC (`icu_normalizer`),
  and deriving a dictionary's scripts. Both ICU crates were already in the
  graph (rinch-dom, parley, url) with compiled data.
- `src/system.rs`: installed-dictionary discovery by locale. Compiled only with
  the `system-dictionaries` feature (default) and never on wasm32.

## The trait

`SpellChecker: Send + Sync` with `check`, `suggest(word, max)`,
`add_to_dictionary`, `ignore`, `languages`, and `add_change_listener` /
`remove_change_listener` (default: `None`/`false`, a checker that reports
nothing). The Hunspell checker reports `SpellChange::Personal(..)` for every
personal-dictionary add/remove (including ones the app makes on the shared
`PersonalDictionary` directly), `Ignored(word)` and `IgnoresCleared`; stage 2
clears marks from them. It forwards the personal dictionary's changes through
a listener it removes when the dictionary is replaced or the checker dropped. The mutating calls take `&self`
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
2. **Script coverage.** A word with no letter in a script some loaded
   dictionary writes in is correct (`HunspellChecker::covers`). With only
   English loaded, Russian, Hebrew, Arabic, Greek, Japanese, Thai text is left
   alone; Chrome gets the same result because its word iterator only yields
   words in the dictionary's script (`$ALetter = [\p{script=…}]` in
   `spellcheck_worditerator.cc`). A dictionary's scripts are derived when it
   loads: every script of a letter on the `.aff`'s `TRY` line, plus every
   script used by at least 1% of the `.dic`'s words (so a few loanwords, a
   "µ" or "Ω", do not claim Greek). The host's en_US derives `[Latin]`.
   `HunspellDictionary::with_scripts` overrides the set.
3. The word is put in NFC ("cafe\u{301}" is "café"); the personal dictionary
   and ignore list store and look up NFC as well.
4. The session ignore list, then the personal dictionary (both with the casing
   rule below), then any loaded dictionary (`spellbook`'s `check`, which
   applies Hunspell's own casing: "Hello" and "HELLO" for "hello", not
   "paris" for "Paris").
5. The same with every `’` replaced by `'`: most dictionaries list only the
   ASCII apostrophe (en_US has an `ICONV ’ '` line, the test fixture
   deliberately does not).
6. The contraction fallback (Chrome's `IsValidContraction` in
   `spellcheck_language.cc`): a word UAX #29 kept whole across punctuation
   ("hello:world", "in'n'out") is correct when each part is. Off with
   `contraction_fallback(false)`. Combining marks (a virama, a vowel sign),
   ZWNJ (U+200C) and ZWJ (U+200D) stay inside a part.

Several dictionaries are several languages: a word is correct if any accepts
it. `suggest` merges each dictionary's suggestions in load order, without
repeats, up to `max`, and gives none for a word over 24 bytes (Chrome's
`kMaxSuggestLen`; `max_suggest_len(None)` lifts it). A word typed with `’`
(and no `'`) is looked up with `'` and its suggestions come back with `’`, so
a stage-3 replacement keeps the user's apostrophe. Suggestion search is the
slow part of Hunspell; check is cheap.

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
| `urls` (`scheme://…`, `www.…`, `mailto:…`, whole whitespace token, wrapping punctuation trimmed) | on | rinch (Joe, #2862); Chrome checks |
| `emails` (`local@domain.tld`, whole token) | on | rinch (Joe, #2862); Chrome checks |
| `digits` (word contains a digit) | on | rinch (Joe, #2862); Chrome checks |
| `all_caps` (two or more cased letters, none lowercase) | on | rinch (Joe, #2862); Chrome checks |
| `max_len` (UTF-8 bytes) | 64 | Chrome `kMaxCheckedLen`, `hunspell_engine.cc` |
| a run with no letter (punctuation, numbers) | always skipped | — |

Chrome checks these; rinch skips them by default for app UI, Joe
2026-11-22, #2862. Chromium's `components/spellcheck/renderer` has no digit,
all-caps, URL or email rule (read 2026-11-22): its word iterator treats ASCII
digits as word characters and hands the word to Hunspell, which accepts pure
numbers only. In app UI fields (codes, identifiers, acronyms, addresses) those
words are mostly false errors, so the four rules are on by default, a
deliberate exception to "default = Chrome". `SkipRules::none()` (or the single
field) gives Chrome's behaviour back.

### Known limit: scripts UAX #29 cannot segment

`unicode-segmentation` has no dictionary, so Thai, Lao, Khmer and Myanmar
text comes out as pieces of a letter or two (a consonant with its marks:
"สวัสดี", one word, is four pieces; `segment.rs`
`scripts_without_spaces_split_into_pieces` pins it), and Han/kana one
character at a time. A Hunspell dictionary for Thai therefore cannot be used
usefully with `words()`: every piece would be checked and most marked. Under
the default (no dictionary for those scripts) they are never marked, by the
script rule. Checking Thai needs a dictionary-based segmenter (`rinch-words`'
optional `icu_segmenter`) feeding the checker; not done.

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
`en_US`; a BCP 47 script subtag is dropped, `zh-Hans-CN` → `zh_CN`,
`es-419` → `es_419`), takes the exact name, then the bare language, then the first
regional dictionary of that language by name. `user_locales()` reads
`$LANGUAGE`, `$LC_ALL`, `$LC_MESSAGES`, `$LANG`. Tests use `find_in` over
`tests/fixtures`; the one host test (`host_en_us_smoke`) returns early when
the host has no `en_US`.

## The personal dictionary

In memory, app-owned. `PersonalDictionary::with_words`/`extend` load the
app's saved list without notifying; `add`/`remove` notify the listener set
through `add_listener` (any number, each removed by its `ListenerId` with
`remove_listener`; one `PersonalDictionaryChange` per real change, called
with no lock held, so a listener may call `words()` to export). Share one `Arc` between
the app and the checker (`with_personal_dictionary`). The session ignore list
lives in the checker (`ignore`, `clear_ignored`) and is never exported.

## Tests

`tests/fixtures/en_TEST.{aff,dic}` is a 39-word dictionary written for these
tests (two suffix rules, a `REP`), with `xx_TEST` (a second Latin
language), `ru_TEST` (Cyrillic) and `la_TEST` (ISO 8859-1). Never assert against the host's dictionaries.

## Marking text fields (stage 2)

### Chrome's rule, measured

Chrome 158 (`--headless=new`, a fresh profile with an `en-US` `.bdic`
dictionary copied in and `browser.enable_spellchecking` on), probed over CDP:
red squiggle pixels counted per element in screenshots after each step
(2026-11-22). The probe script is not kept in the tree; the steps and
answers:

| Step | Marks |
|---|---|
| page load, pre-filled `<textarea>`, `<input>`, `contenteditable`, 5 s | none |
| `textarea.focus()` from script, 2.5 s | none |
| press in the pre-filled `<textarea>` | the whole field, within 300 ms |
| Tab onto it | the whole field |
| script `focus()`, then type `" qq "` at the end | the whole field, after the pause |
| blur a `<textarea>` / a `contenteditable` | marks kept |
| blur an `<input>` | marks hidden; pressing it again shows them |
| type `helo` (no separator): 100 ms / 500 ms / 1 s after the last key | none / none / marked |
| type a space after a marked word | the word stays marked |
| `<input type=email>` | checked (rinch does not: Joe, #2862) |

So: a field is checked once the **user** focuses (pointer or keyboard) or
edits it; the whole value then, only the edited words after a ~1 s idle; the
word being typed is marked once typing pauses even with the caret at its
end; a single-line field hides its marks while blurred.

### Which fields

`rinch_spell::runtime` holds the rules both the fields and the editor use:
`attribute_state` (HTML's enumerated states: `""`/`true` on, `false` off,
anything else inherits), `resolve_inherited` (nearest ancestor-or-self that
names a state, default on) and `input_type_is_checked` (no type, `text`,
`search`, or an unknown type; never `password`/`email`/`url`/`number`/`tel`
or the other known types). `RinchApp::field_is_spellchecked`
(`crates/rinch/src/app/spellcheck.rs`) adds the tag (`<textarea>`,
`<input>`) and `readonly`.

### Configuration

`SpellcheckConfig` (switch, `Dictionaries::System { locales, personal }` /
`Checker(Arc<dyn SpellChecker>)` / `None`, `SkipRules`, `debounce_ms`) is held
**per UI thread** in `rinch_spell::runtime` (`set_config`, `config`,
`generation`, `on_config_change`). `App::try_run` / `run_android` install the
builder's; `RinchContext::new` installs the default when the host set none;
a thread that never had one (`RinchApp::new` in a test, `TestApp`) has no
`session()`, so nothing is checked and host dictionaries are never read.
`session()` answers `None` when off or when the dictionaries resolved to
nothing (`OnceLock<Option<Arc<dyn SpellChecker>>>`, filled on the checking
thread by the first check); every entry point in the runtime returns at that
first line, which is what makes an app with spellcheck off, or with no text
field touched, do zero work: no thread, no listener, no record.

### The checking thread

`runtime::spawn` runs jobs on one lazily started `rinch-spellcheck` thread
(an `mpsc` queue, each job under `catch_unwind`). A field's job is
`Session::check_ranges(text, dirty)`: `words()` over each dirty range,
checked. The result goes back through the app's deferred-work inbox
(`AppWorkSender`, the same route the async paste takes), so it is applied on
the UI thread with `&mut RinchApp`. `set_inline_jobs(true)` runs jobs
synchronously (the tests). Loading the system dictionaries happens in the
first job, never on the UI thread.

### Per field (`crates/rinch/src/app/spellcheck.rs`)

`RinchApp::spelling` (`FieldSpelling`) maps a field's node id to its `text`,
`marks`, `dirty` ranges, a `generation` and a debounce deadline. Entry
points:

- `spell_user_focus` — called after a press installs the field
  (`click_handling.rs`), a press on its `<label>`, and a Tab onto it
  (`handle_tab`). Not from `try_focus_input` itself, which programmatic focus
  also reaches. The first one checks the whole value at once.
- `spell_user_edit` — called from `handle_input_edit_command` after the
  `oninput` rewrite is adopted, with the field's final value.
  `runtime::text_change` finds the changed bytes; `map_through_edit` moves
  the other marks (dropping the edited word's, but keeping a word a typed
  space only abuts); `edit_region` widens the change to the white space
  around it (not past inserted white space) and joins `dirty`; the deadline
  is now + `debounce_ms`, and one `set_timeout` (armed `unowned`, re-armed
  only for an earlier deadline) sends `spell_tick` through the inbox.
- `spell_apply` — drops a result whose field generation or configuration
  generation moved on (a newer edit is pending and its own check will come),
  replaces the marks inside the checked ranges, clears `dirty`.
- A configuration change (`on_config_change`, registered at the first
  interaction) clears every mark and re-checks the remembered fields, or
  keeps them unchecked while off so turning it back on marks them again.

Marks reach the DOM as `rinch_dom::SpellingMarks { text, ranges }` on the
node (`RinchDocument::set_spelling_marks`): a node field, not an attribute,
so no style invalidation; an unchanged write does nothing; a changed one is a
paint-dirty mark (`spelling_mark_writes` counts it). Paint
(`paint/contenteditable.rs` `paint_input_value`) draws them only while the
control shows exactly `text` (so an app's `value` write never shows a mark
under the wrong letters before the next check), never on the placeholder or
a password, only while focused for an `<input>`, and shifted past an IME
preedit. The wave is `paint/text.rs` `paint_layout_mark`, the
`text-decoration-style: wavy` geometry (`wavy_wave_in`, `push_wave`) over
the control's device-px layout. Colour and line come from the inherited
custom properties `--rinch-spellcheck-color` (default `#ff0000`) and
`--rinch-spellcheck-style` (`wavy` / `solid` / `none`).

`spell_frame` (called from `resolve_and_repaint`, returning at once while
no field is remembered and `require_interaction` is on) forgets a field that
is no longer connected, drops the marks of one that stopped being checked
(`readonly`, an inherited `spellcheck="false"`: `Field::off`) and re-checks it
when it is checked again. With `require_interaction(false)` it also walks the
document's `<input>`/`<textarea>` elements each frame, starts every checked
one, and treats a value the app wrote as an edit. `hide_input_marks_on_blur`
reaches paint as `SpellingMarks::show_when_blurred`. When the session goes
away (off, or the system dictionaries resolved to none) `spell_tick` clears
every pending deadline and arms nothing (review of #2876: it used to re-arm a
0 ms timer for a deadline already past, for ever). The edit hook in
`handle_input_edit_command` asks `runtime::is_active()` (no allocation)
before cloning the value.

Known limit: with `require_interaction` on, a field whose value the app
rewrites while it is unfocused is re-checked at the next interaction (until
then its stale marks are simply not drawn).

### The editor (`crates/rinch-editor-view/src/spellcheck.rs`)

`SpellcheckPlugin` (key `"spellcheck"`, public with `SPELLCHECK_KEY` and
`SPELL_ERROR_CLASS`, so an editor built with `EditorHandle::new` and its own
plugin list can add it) holds `SpellState { activated, marks, dirty,
generation, skip }` as plugin state. `generation` comes from one process-wide
counter (`fresh_generation`) at every document change, `Activate`, `Reset`
and `Clear`, so it never repeats, not across a `load_html` (which builds a
fresh state; review of #2877: a per-state counter restarted at 0 and a check
for the old document landed on the new one) nor between editors; a `Result`
is applied only while activated and at the same generation.
`SpellcheckPlugin::new().skip_nodes(["code_block"]).skip_marks(["code"])`
leaves text out (default: everything checked, as Chrome): a skipped node is
not descended into, text under a skipped mark becomes U+FFFC in the checked
text. A textblock whose host element, or an element between it and the
container, says `spellcheck="false"` is skipped too (`spell_block_off`, as
Chrome honours it inside a contenteditable). `create_editor` installs it
(`editor_plugins`) only when the thread has a configuration, never on wasm,
so an editor in a test with none, and every existing editor perf scenario,
has no plugin at all. In `apply`: a document change bumps `generation`,
maps the marks through `tr.mapping()` (start assoc 1, end assoc -1),
drops a mark any changed range touches (`changed_ranges`: each step map's
replaced range, mapped through the later maps; a range that only abuts a mark
with inserted white space leaves it, as for fields) and, once activated,
adds the changed ranges to `dirty`. The handle talks to it through one meta
key, `"spellcheck"`, on meta-only transactions (`add_to_history(false)`, no
steps, so no history entry, no `on_change`, nothing sent to collaborators):
`Activate` (whole document dirty), `Reset` (a configuration change: marks
dropped, generation bumped) and `Result { generation, checked, found }`,
applied only when `generation` still matches. `decorations` hands out one
`Decoration::inline` per mark with class `pm-spell-error`, so a result is a
decoration-only update the view applies without a document diff.

The handle (`EditorCore::spell`, a `Driver` with the debounce timer and the
config listener): `spell_user_focus` (called by the desktop runtime from
`try_new_editor_click` and `handle_tab`) activates and checks at once;
`spell_after_dispatch` (end of `dispatch_inner`) activates on the user's own
input (`input == true`; an app's `update` does not). The debounce is armed
from the one place every state change passes: `EditorCore::take_owed`, run
when a mutable borrow ends, compares the plugin's generation with the last one
it saw (`spell_seen`) and owes `spell_owed` when it moved and words are dirty.
So `command` (Backspace, Delete, Enter, undo, cut), `insert_image`,
`toggle_link` and `collab_receive` arm it as typing does (review of #2877:
only `dispatch_inner` did). A result that comes back with words still dirty
(a stale one dropped) re-arms it. `refresh_spellcheck` (public; called by
`set_read_only` and the `Editor` component, and by an app after changing an
ancestor's `spellcheck`) drops every mark (`Clear`) when the editor is not to
be checked and re-checks an activated one (`Reset`) when it is again. `spell_session` returns
`None` unless the plugin is installed, `session()` is `Some`, the editor is
not read-only and its container's inherited `spellcheck` is on (the
`Editor { spellcheck }` prop writes the container's attribute). A check
(`jobs`) takes each textblock a dirty range reaches, its text one char per
position (an inline leaf is U+FFFC), the dirty part widened to white space;
`run_jobs` on the checking thread turns byte ranges back into positions. The
reply is a parked main callback (`park_main_callback` +
`dispatch_main_callback`, or a direct resume when the job ran inline on the
UI thread), so it reaches the handle on the UI thread. The squiggle's CSS is
`text-decoration: underline var(--rinch-spellcheck-style, wavy)
var(--rinch-spellcheck-color, #ff0000)` in `styles.rs` (it was `wavy
#d1242f`); `--rinch-spellcheck-style: none` makes it invalid at computed
time, so nothing is drawn.

The web editor is not `contenteditable`, so the browser cannot check it, and
rinch's checker does not run on wasm: the web editor is not marked (a known
gap, recorded on #2862).

### The web

`rinch-web` never runs the checker: the browser does. `AppExt::run` calls
`rinch_web::apply_spellcheck`, which puts `spellcheck="false"` on `<body>`
when the app turned it off and registers a configuration listener that keeps
`<body>` in step with later `configure` / `set_enabled` calls; attributes and
props pass through as written.

### Tests

`crates/rinch-spell/src/runtime.rs` (states, inheritance, types, edit
mapping), `crates/rinch-dom/tests/spelling_marks_tests.rs` (the wave under
the named words, colour/style properties, hidden on a blurred input, stale
text not drawn), `crates/rinch/src/app/spellcheck_tests.rs` (press / Tab /
script focus, inheritance, types, the switch, an edit re-checking only its
word, an unconfigured app doing nothing),
`crates/rinch-components/tests/spellcheck_prop_2862.rs`,
`crates/rinch-web/tests/spellcheck_2862.rs` (the browser's resolved
`HTMLElement.spellcheck`), `crates/rinch-editor-view/src/spellcheck.rs`
tests (marks through unrelated edits, a stale result dropped, the container's
inherited attribute) and `crates/rinch/src/app/editor_spellcheck_tests.rs`
(a press marks the words as red waves over the right IFC bytes, a script
focus marks nothing, the custom properties, the prop).

## What stage 2 builds on

- Run `words()` over a changed text run and `check()` each range off the
  keystroke path; `misspelled()` is the one-call form.
- The editor plugin turns ranges into wavy decorations (decoration ranges map
  through transactions, so only changed words need re-checking).
- `rinch-web` never runs this engine: the browser checks.
- The context menu (stage 3) calls `suggest(word, DEFAULT_MAX_SUGGESTIONS)` and
  `add_to_dictionary`.
