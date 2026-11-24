# Spellcheck (`rinch-spell`)

Read this page when your task touches spellchecking: `crates/rinch-spell`, or
the stages of #2862 that build on it (marking misspelled words in fields and
the editor, the context menu's suggestions, platform backends). Stage 1 is
the engine; stage 2 marks misspelled words in text fields ("Marking text
fields" below); stage 3 puts suggestions and "Add to dictionary" in the text
context menu ("The context menu" below); stage 4 adds the platform
spellcheckers of macOS and Windows ("Platform backends" below). `rinch` re-exports the crate as `rinch::spellcheck`.

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
- `src/locale.rs`: `normalize_locale` (always compiled; `system` re-exports it).
- `src/platform/`: `PlatformChecker` (`mod.rs`, target-independent logic and
  the language/script tables), `mac.rs` (`NSSpellChecker`), `windows.rs`
  (`ISpellChecker`). The bindings are target-specific optional dependencies
  behind the `platform` feature (default on), so Linux, Android and wasm32
  build none of them.
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
every public type and checks from a spawned thread. The platform backends
meet the bound by forwarding every native call to the one thread that may
make it ("Platform backends" below).

Two defaulted methods serve those backends: `check_batch(&[&str]) ->
Vec<bool>` (default: `check` per word), which `Session::check_ranges` uses
for all the words of a check at once, so an engine that must hop threads
answers many words per hop; and `suggest_on_calling_thread()` (default
`false`), which makes `runtime::suggest` run the request on the calling (UI)
thread instead of the `rinch-spell-suggest` thread (bumping the supersede
counter first, so queued older requests are still dropped).

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
(`editor_plugins`) only when the thread has a configuration (on wasm: a
session, below), so an editor in a test with none, and every existing editor perf scenario,
has no plugin at all. In `apply`: a document change bumps `generation`,
maps the marks through `tr.mapping()` (start assoc 1, end assoc -1),
drops a mark any changed range touches (`changed_ranges`: each step map's
replaced range, mapped through the later maps; a range that only abuts a mark
with inserted white space leaves it, as for fields) and, once activated,
adds the changed ranges to `dirty`. The handle talks to it through one meta
key, `"spellcheck"`, on meta-only transactions (`add_to_history(false)`, no
steps, so no history entry, no `on_change`, nothing sent to collaborators):
`Activate` (whole document dirty), `Reset` (a configuration change: marks
dropped, generation bumped) and `Result { generation, checked, found, upto }`,
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

### The web

**Fields:** `rinch-web` never runs the checker on an `<input>` or
`<textarea>`: the browser does. `AppExt::run` calls
`rinch_web::apply_spellcheck`, which puts `spellcheck="false"` on `<body>`
when the app turned it off and registers a configuration listener that keeps
`<body>` in step with later `configure` / `set_enabled` calls; attributes and
props pass through as written.

**The editor** (Joe's decision on #2862, 2026-11-23): it is not
`contenteditable`, so the browser cannot check it, and rinch does, with the
same plugin and the same handle code as the desktop, when the app supplied
dictionaries. Differences, all in `crates/rinch-editor-view/src/spellcheck.rs`
behind `web_path()` (`cfg!(target_family = "wasm")`, or a test switch that
puts a native unit test on the web's path):

- **No work without dictionaries.** `set_config` settles
  `Dictionaries::System` to "nothing" at once when the target has no system
  dictionaries (`SYSTEM_DICTIONARIES`, false on wasm32), so `session()` is
  `None` and `is_active()` false from the start. `create_editor` adds the
  plugin on the web only when `session()` is `Some` (`wanted`); without it,
  no plugin state, no `apply`, no decorations, no timer. An editor made with
  no session (dictionaries still being fetched, or no configuration at all
  yet: a handle made before `App::run`, an island whose app configures after
  its fetch) registers one configuration listener (`watch_for_dictionaries`;
  `set_config` tells its listeners on the first configuration too); when a
  later `configure` makes a session, the listener adds the plugin with
  `EditorHandle::install_plugin(.., keep = true)`, which uses the new
  `EditorState::reconfigure` (ProseMirror's: every installed plugin keeps its
  state, so the undo history survives; `add_plugin` still rebuilds). A press
  or an edit before that is remembered (`Driver::interacted`) and checks the
  editor once the plugin is in.
- **Slices, not a thread.** `spawn` runs inline on wasm, so `spell_check`
  does not call it there: `spell_start_slices` takes the dirty textblocks
  (`dirty_blocks`, positions and node clones: cheap, no text built) into a
  `Slicer` and runs `spell_slice` from `rinch_core::yield_now`, one browser
  task per slice, the first one too (a press returns at once). On the web
  `yield_now` posts to one `MessageChannel` (`web_runtime::web_yield`), not a
  `setTimeout(0)`, which browsers clamp to 4 ms once nested. A slice builds
  the next block's text (`block_job`) only when it reaches it, cuts each
  dirty range at white space into pieces of `slice_chunk_bytes` (512)
  (`pieces`), and checks pieces until `slice_budget_ms` (3 ms) less what
  applying the previous slice's result took has passed (at least one piece),
  or `slice_max_found` (100) misspellings are found (all three are
  `SpellcheckConfig` options, copied into `Session::slices`); byte offsets
  become positions by counting chars
  forward from the last one asked, so a long block is not re-counted. Its
  result goes in at once as `SpellMeta::Result { upto: Some(p) }`: the plugin
  replaces the marks in `checked` and trims its dirty list to what lies past
  `p`, so the words already checked are off the list. The last slice sends
  `upto: None` (dirty cleared, the desktop's only form). A slice whose plugin
  generation moved (an edit between slices) is dropped with the rest of its
  check: the edit armed its own debounce, and the trimmed dirty list,
  mapped through the edit, still holds what was not checked. A debounce that
  fires while a check for the same generation is running leaves it be.
- **The squiggle** is the browser's: the view writes the same
  `<span data-pm-deco class="pm-spell-error">`, and `styles.rs`'s rule
  (`text-decoration: underline var(--rinch-spellcheck-style, wavy)
  var(--rinch-spellcheck-color, #ff0000)`) is drawn by the browser.
- **Interaction**: `handle_mousedown` (`rinch-web/src/editor_input/pointer.rs`) calls
  `spell_user_focus` after it gives the editor the keyboard; typing activates
  through `spell_after_dispatch`, as on the desktop. The web editor is no Tab
  stop of its own, so there is no Tab path.

- **The feature.** `rinch-editor-view`'s `spellcheck` feature (forwarded
  and on by default in `rinch-web`) gates it on wasm: off, the const `ON` is
  false in `wanted`, `watch_for_dictionaries` and `spell_session`, nothing
  reaches the checking code, and the linker drops it. Off the web `ON` is
  always true.

Measured (Chrome 157 on nico-top, `cargo test --release`, the
`a_long_document_is_checked_in_slices` page and a 650,000-character document
of ordinary prose with the system en_US): with the apply counted in the
budget, 55 slices averaging 3.0 ms on the prose document (longest 7.7 ms,
one apply of 6.5 ms), 209 averaging 2.6 ms (longest 5.9 ms) on the 16,400
marks of the test page. Applying a result is most of a long slice (the
decoration set and view are rebuilt from all marks, O(marks); before
`slice_max_found`, a slice that added 6,400 spans took 25 ms). In a debug
build an apply alone outlasts the budget, so each slice checks one piece. `spell_user_focus` on the 650,000-character document took
2 ms; `HunspellChecker::from_bytes` for en_US 33 ms (350 ms in a debug build).
`slice_stats()` / `reset_slice_stats()` (doc-hidden) report it.
`examples/editor-web` (release, before `wasm-opt`): 5,252,139 bytes on main
before this, 5,375,913 with the feature, 5,267,767 without.

**Known gap:** no spelling suggestions or "Add to dictionary" in the web
editor's context menu. rinch-web leaves the browser's own menu on editable
targets (#812), and that menu knows nothing of rinch's marks. Recorded on
#2862 for later.

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
inherited attribute; on the web's path: a long document in slices ending
with the desktop's marks, an edit between slices, dictionaries arriving
later), `crates/rinch/src/app/editor_spellcheck_tests.rs`
(a press marks the words as red waves over the right IFC bytes, a script
focus marks nothing, the custom properties, the prop) and
`crates/rinch-web/tests/editor_spellcheck_2862.rs` (in the browser: a
genuine press marks the words after the press, the computed wavy red
underline and the custom properties, read-only and the switch clearing,
`spellcheck="false"`, no dictionaries no slices, a late `configure`, and
the slice counts and costs of a long document, printed).

## The context menu (stage 3)

### Chrome's menu, measured

Chrome 158 on Linux, headful under a nested headless `sway` with its own
`Xwayland` (`--ozone-platform=x11`, so `xdotool` can drive the native menu),
a fresh profile with the `en-US-10-1.bdic` dictionary, `grim` screenshots
(2026-11-23). A `<textarea>` holding `the helo wrld qwzxqvj ok`:

| Step | Chrome |
|---|---|
| right press on `helo` (marked) | the word is selected (`select` event, 4..8); menu: **hole, help, helot**, separator, **Add to dictionary**, Use enhanced spell check, Search Google for "helo", separator, Cut, Copy, Paste, Open in reading mode, … |
| right press on `qwzxqvj` (no suggestion) | word selected; menu starts with **Add to dictionary**: no "No suggestions" row |
| right press on an unmarked word | caret at the press, plain menu (Undo, Redo, Cut … Select all, Spell check ›, Writing Direction ›) |
| right press on `helo` inside a selection 0..22 | selection kept, plain menu, no spelling rows |
| choose `help` | `beforeinput` + `input`, `inputType` `insertReplacementText`, data `help`; value `the help wrld …`; caret 8..8 (after the word) |
| Ctrl+Z after it | one `historyUndo` brings `helo` back, selected (4..8) |
| Add to dictionary on `wrld` | every `wrld` mark goes, in the textarea and a `contenteditable`; selection kept; no input event |

So three suggestions (although the engine is asked for five,
`kMaxSuggestions`), then "Add to dictionary". Left out, recorded here: "Use
enhanced spell check" and "Search Google for" (Google services); Chrome's
spelling menu has no "Ignore" row, and its "Spell check ›" submenu (languages,
"Check the spelling of text fields") appears only on the plain menu: rinch's
switch and languages are the app's (`SpellcheckConfig`), not the user's
menu. Chrome's own spelling menu also drops Undo/Redo/Select all; rinch keeps
its four edit rows as they are.

### The rule

`text_context_menu.rs`: `prepare_target` runs the caret rule as before, then
`spell_select_in_input` / `spell_select_in_editor`: the mark at the press
(`RinchApp::spell_misspelling_at`, a field's marks while it still shows the
text they were made for; `EditorHandle::misspelling_at`), ends included. A
collapsed selection (or one that is exactly the word) becomes the word; any
other selection the press kept stays, with no spelling. The word, its range and
its suggestions are `RinchApp::text_spell` (`SpellTarget`), reported by
`text_edit_state().spelling` (`TextSpelling`) only while the selection is
still the word and the word still marked (`live_spell_target`). A field's
spelling costs nothing when no field was ever checked
(`FieldSpelling::is_empty`).

### Suggestions without blocking

`new_spell_target` asks the loaded checker (`Session::loaded_checker`, never
loading on the UI thread) for `menu_suggestions` words through
`runtime::suggest`: one reusable thread (`rinch-spell-suggest`, apart from the
`rinch-spellcheck` thread, which may be busy with a whole document), where each
request supersedes the ones before it, so a queued older one is dropped unrun
and repeated presses never pile up (one already running finishes). The press
waits up to `SpellcheckConfig::menu_suggest_wait_ms` (default 100 ms) for
them. Past that, the menu opens with `suggestions_pending` (only "Add to
dictionary" shows) and the answer arrives through the app's inbox
(`text_spell_arrived`, which takes only the answer to the current request),
redrawing the open menu at the same point with the highlighted row kept (by
its action). Hunspell's suggest is the slow call; check is cheap. Under
`set_inline_jobs` (tests, and always on wasm32) it runs at the
press.

### Choosing a suggestion, adding a word

`TextEditAction::Suggestion(i)` replaces the selected word through the typing
path: `handle_input_edit_command(EditCommand::InsertText)` in a field (one undo
step, `oninput`, the caret after it, the dirty flag for `onchange`), and
`EditorHandle::insert_text` in the editor (one history event whose selection
is the word, so undo brings it back selected, as in Chrome; `on_change`). The
edit drops the word's mark at once (`map_through_edit` / the plugin's
`changed_ranges`) and the re-check after the pause finds the replacement
correct.

`TextEditAction::AddToDictionary` runs `runtime::add_to_dictionary(word)`:
the loaded checker's `add_to_dictionary` (the app's `PersonalDictionary`
listener hears `Added`), then `deliver_changes`, which calls this thread's
`on_words_accepted` listeners: each editor drops the marks whose word the
checker now takes (`SpellMeta::Drop`, a meta-only transaction), and the fields'
listener does the same through the inbox; the menu also runs
`spell_words_accepted` itself, so every field's marks go in the same event.
Only the marked words are asked about, on the loaded checker.

Changes the app makes itself (on the `PersonalDictionary`, `ignore`, from any
thread) reach the marks too: the runtime subscribes to the checker's
`add_change_listener` when it resolves (`Changes`), the listener sets a flag
and calls the wake rinch installs (`set_change_wake`, a
`dispatch_main_callback` of `deliver_changes`). A word **removed** (or the
ignores cleared) starts a new generation and calls the `on_config_change`
listeners instead: everything is checked again. A replaced configuration
unsubscribes from its checker.

### Configuration and the seam

`SpellcheckConfig::menu_suggestions` (default `DEFAULT_MENU_SUGGESTIONS`, 3;
`0` asks for none), `menu_add_to_dictionary` (default on),
`menu_suggest_wait_ms` (default 100), `menu_no_suggestions_row` (default off, Chrome's; on shows a disabled "No
spelling suggestions" row, Chrome's string, that is not an item). The rows use
the menu's classes (`rinch-tcm-item`, `rinch-tcm-label`, `rinch-tcm-sep`)
plus `rinch-tcm-suggestion`, `rinch-tcm-add-to-dictionary` and
`rinch-tcm-no-suggestions` for an app's own rules. A shell (Android) reads
`TextEditState::spelling` and runs `perform_text_edit(Suggestion(i))` /
`AddToDictionary`; Android's toolbar does not show them yet (no UI built).

Pins: `crates/rinch/src/app/spellcheck_menu_tests.rs` (rows and order, the
count, the no-suggestions cases, an unmarked word and a wider selection, a
suggestion in an `<input>`, a `<textarea>` and the editor with one undo, Add to
dictionary everywhere, row and seam agreeing, slow suggestions filled in),
`rinch-spell` `runtime::tests::add_to_dictionary_reaches_the_app_and_the_marks`
and `rinch-editor-view` `spellcheck::tests::add_to_dictionary_drops_the_word_in_every_editor`.

## What stage 2 builds on

- Run `words()` over a changed text run and `check()` each range off the
  keystroke path; `misspelled()` is the one-call form.
- The editor plugin turns ranges into wavy decorations (decoration ranges map
  through transactions, so only changed words need re-checking).
- `rinch-web` never runs this engine on fields (the browser checks them);
  it runs it on the rich-text editor when the app supplied dictionaries.
- The context menu (stage 3) calls `suggest(word, menu_suggestions)` and
  `runtime::add_to_dictionary` (see "The context menu" above).

## Platform backends (stage 4)

### Choice

`SpellcheckConfig::backend` (`Backend::Auto` default, `Hunspell`,
`Platform`) decides what `Dictionaries::System` resolves to;
`Backend::engine(platform, hunspell)` is the pure table
(`runtime::tests::backends_resolve_to_engines`): Auto is the platform where
`platform::AVAILABLE` (macOS or Windows with the `platform` feature), else
Hunspell when `system-dictionaries` is compiled; `Hunspell` never the
platform; `Platform` nothing where there is none. `set_config` settles
`Engine::Nothing` to "no checker" at once (as it did for wasm), so a forced
platform on Linux does no work. `Dictionaries::Checker` (the app's own) is
untouched by the choice. `load` builds `PlatformChecker::with_options(locales,
PlatformOptions)`, shares the configured personal dictionary and applies
`add_to_system_dictionary` (default `true`).

**Fallback** (`platform_or_fallback`, review of #2941): under `Backend::Auto`,
when the platform checker does not start (Windows with no spellchecking
service) or checks no language (`platform_languages()` answers an empty
list: none of the app's languages installed, or none of the user's has a
spellchecker), the session uses Hunspell over the system's dictionaries
instead, as Chrome checks with Hunspell the languages Windows cannot. A
platform that could not name its languages yet (macOS timing out, `None`)
is kept. `Backend::Platform` never falls back. Partial coverage (some of the
app's languages installed) keeps the platform for those only; mixing the two
engines per language is not done.

**Tuning:** `SpellcheckConfig::main_thread_wait_ms` (default 2000) and
`main_thread_slice_ms` (default 3) become `PlatformOptions` (macOS only;
Windows ignores them).

### `PlatformChecker` (`platform/mod.rs`)

Engine-independent, over an internal `Native` trait (`languages() ->
Option<Vec<String>>`, `check_words`, `suggest`, `learn`, `unlearn`, `ignore`,
`clear_ignored`, `suggest_on_calling_thread`). `platform/tests.rs` drives it
with a fake engine on any host. In order, a word is correct when:

1. it has no letter in a script of the checked languages (`covers`). The
   languages come from the engine; `language_scripts(tag)` maps them (a
   hand table: Latin for the usual European and other Latin-script
   languages, Cyrillic, Greek, Arabic, Hebrew, the Indic scripts, Thai, Han,
   kana, Hangul, …). A language not in the table turns the rule off (every
   word goes to the engine), and so does an engine that could not name its
   languages. The scripts are worked out once per `check_batch`, never per
   word (review of #2941: each ask may be a main-queue hop with a 2 s wait),
   and cached: an answer for the checker's life, "could not answer" for
   `LANGUAGES_RETRY` (30 s) before the next ask
   (`unanswered_languages_are_asked_once_per_batch_and_then_cached`);
2. the session ignore list or the personal dictionary accepts it (NFC, the
   casing rule);
3. the engine accepts it. `check_batch` sends only the words 1 and 2 left, in
   one `check_words` call.

"Add to dictionary" is `PersonalDictionary::add`; the checker's listener on
the personal dictionary forwards `Added` to `Native::learn` and `Removed` to
`unlearn` while `add_to_system_dictionary` is on, then reports
`SpellChange::Personal` as the Hunspell checker does. So the app's own
`add`/`remove` reach the system too, words loaded with `with_words`/`extend`
(no notification) never do, and the app's listener fires either way.
Chrome's choice is the same on both platforms (`spellcheck_platform::AddWord`
→ `learnWord:`; `WindowsSpellChecker::AddWordForAllLanguages`), with its own
custom dictionary file kept beside it. A word in the system dictionary but
not the personal one is accepted (by the engine) and not listed.

Ignore is rinch's list (so `clear_ignored` works and the listeners hear
it); on macOS it is also handed to `ignoreWord:inSpellDocumentWithTag:` with
the checker's own document tag and cleared with `setIgnoredWords:@[]`.
Windows' `ISpellChecker::Ignore` has no undo, so it is not called.

Language tags: `to_mac_language` / `from_mac_language` are Chromium's
`ConvertLanguageCodeToMac` / `FromMac` (`en_US` ↔ `en`, `pt_PT` ↔ `pt`,
`pl` ↔ `pl_PL`) in rinch's underscore form; `to_bcp47` for Windows;
`choose_languages(wanted, supported)` matches a wanted tag to an installed
spellchecker exactly, else to the first of the same language (`ko` →
`ko-KR`, `fr-CA` → `fr-FR`). Suggestions from several languages are
interleaved (`interleave_suggestions`, Chromium's `FillSuggestions`).

### macOS (`platform/mac.rs`)

**Threading: the main thread.** Chromium makes every `NSSpellChecker` call
on the browser UI thread (the main thread on macOS):
`SpellCheckHostChromeImpl::CheckSpelling` and `FillSuggestionList`
(`chrome/browser/spellchecker/spell_check_host_chrome_impl.cc`, whose other
entry points `CHECK_CURRENTLY_ON(BrowserThread::UI)`) call
`spellcheck_platform::CheckSpelling` in
`components/spellcheck/browser/spellcheck_platform_mac.mm`, which also posts
panel work with `performSelectorOnMainThread`. Apple's header does not mark
the class main-actor (objc2-app-kit 0.3's generated `NSSpellChecker` is not
`MainThreadOnly`; only its panels take a `MainThreadMarker`), but rinch
follows Chromium rather than rely on undocumented thread safety.

`on_main(f)`: on the main thread (`MainThreadMarker::new()`), `f` runs at
once; elsewhere it is queued with `DispatchQueue::main().exec_async` and the
caller waits on a channel for at most `main_thread_wait` (2 s by default). Not
`exec_sync`: a main thread that never drains the main queue (no run loop, a
test harness) or that is itself blocked on the caller would deadlock it; with
the timeout the words count as correct (unmarked) and the late answer is
dropped. `f` runs under `catch_unwind` (a panic must not unwind into
libdispatch). rinch's UI thread is the main thread on macOS, so the menu's
suggestions (`suggest_on_calling_thread() == true`), "Add to dictionary"
and a check under `set_inline_jobs` all run directly. The checking thread's
`check_words` is sliced: each hop checks words until `main_thread_slice`
(3 ms by default, the web editor's slice budget) has passed, then returns, so the UI
draws between slices. Constructing `MacSpeller` touches no AppKit (the
checking thread builds it inside the session's `OnceLock`; nothing on the UI
thread waits on that lock today, and this keeps it so).

Calls: `checkSpellingOfString:startingAt:0 language:<lang or nil>
wrap:NO inSpellDocumentWithTag:wordCount:NULL`, correct when the range's
length is 0 (Chromium's `CheckSpelling`); `guessesForWordRange:inString:
language:inSpellDocumentWithTag:` (Chromium passes `[checker language]`);
`learnWord:`, `unlearnWord:`; `uniqueSpellDocumentTag` on first use,
`closeSpellDocumentWithTag:` on drop. Languages: with no app list, `language:
nil` (the system's choice, with automatic identification when the user has
it on), and `languages()` reports `userPreferredLanguages` (or `[language]`
when automatic identification is off) for the script rule; with an app list,
each tag maps to a code in `availableLanguages` (exact, else the first of
the same language) and a word is correct if any accepts it (none available:
nothing is marked).

**Known limit:** `+sharedSpellChecker` can raise an Objective-C exception
(Chromium wraps it in `@try`, citing pasteboard state). objc2's `exception`
feature compiles a C helper, which the Linux cross-check of `check-macos`
cannot build, so rinch does not catch it.

### Windows (`platform/windows.rs`)

**Threading: one MTA thread of its own.** Chromium keeps every call on one
COM task runner (`windows_spell_checker.cc`, `BackgroundHelper`: "All
invocations of these methods must be posted to the same COM
SingleThreadTaskRunner", and it asserts an STA there). rinch keeps the one
thread, `rinch-spell-windows`, but joins the **multithreaded** apartment: the
factory's registration says `ThreadingModel = Free`
(`HKCR\CLSID\{7AB36653-1796-484B-BDFA-E74F1DB7C1DC}\InprocServer32`,
`MsSpellCheckingFacility.dll`; read on nico-top, Windows 11), so in the MTA its
objects are called directly, with no proxy and no message pump (an STA
client would get a proxy into the MTA). The UI thread is an STA (winit's
OLE drag and drop) and must not use the objects itself. Callers post a
`Request` over an `mpsc` channel and wait for the reply; the COM thread waits
on nobody, so nothing deadlocks. Adding and removing a word are not waited
for (review of #2941: the UI thread would wait behind a whole textarea's
check); the queue is in order, so a check sent later sees the word.
**The MTA is kept alive** (`keep_mta_alive`, `CoIncrementMTAUsage` once per
process, never released): each checker's thread leaves the MTA when its
checker drops, and when it was the MTA's last member a checker made just
then (a configuration replacing the old one) failed with
`CO_E_SERVER_STOPPING` (0x80080008, "Cannot create a WinRT object during
host MTA shutdown"), seen on nico-top in 7 of 12 runs of the crate's tests
once the Auto fallback made the empty checker visible.
`WindowsSpeller::new` waits for the thread
to create the factory (`None` if it cannot: no spellchecking service).

Calls: `CoCreateInstance(SpellCheckerFactory)`; languages by default from
`HKCU\Control Panel\International\User Profile\Languages` (the Settings
language list; `en-US\0ko` on nico-top), else `GetUserPreferredUILanguages`,
matched against `SupportedLanguages` (`choose_languages`; nico-top has no
Korean spellchecker, so it checks `en_US`); `CreateSpellChecker` per language;
`ComprehensiveCheck` per word (as Chromium's `RequestTextCheckForAllLanguages`),
misspelled only when every language reports an error whose
`CorrectiveAction` is `GET_SUGGESTIONS` or `REPLACE` (Chromium's
`DELETE` is a repeated word); `Suggest` per language, interleaved; `Add` and
`ISpellChecker2::Remove` on every language.

### What ran where

- Linux (CI and here): `platform/tests.rs` (the logic over a fake engine,
  the tag and script tables) and the runtime tests (`backends_resolve_to_engines`,
  `a_forced_platform_backend_without_a_platform_does_nothing`,
  `a_check_asks_one_batch`, `a_main_thread_checker_suggests_on_the_calling_thread`).
- **Windows, for real** (2026-11-23, nico-top, Windows 11 with `en-US` and
  `ko` in its language list): the crate's unit tests cross-built for
  `x86_64-pc-windows-gnu` in WSL (mingw-w64) and run as a Windows `.exe`
  from `C:\…\Temp` (WSL interop refuses an `.exe` on the Linux file system,
  "Invalid argument"): `platform::windows::tests` (`hello` correct, `helo`
  and `wrld` not, suggestions for `helo` `hello, halo, helot, hele, help`,
  four threads at once, `Add` reaching the system dictionary as seen by a
  second checker and `Remove` taking it out, the default configuration
  marking through `Session::check_ranges`), plus every other unit test and
  `tests/hunspell.rs`. (`tests/system.rs` fails there only because its
  fixtures path is the Linux one, `CARGO_MANIFEST_DIR` baked at build time.)
- The Windows run was repeated after the review fixes (`ComprehensiveCheck`,
  unwaited `Add`/`Remove`).
- **macOS: compiled only** (`cargo clippy -p rinch-spell --all-targets
  --target {aarch64,x86_64}-apple-darwin`, and CI's `check-macos`). Nothing of
  `mac.rs` has run on a Mac.

