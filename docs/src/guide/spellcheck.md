# Spellcheck

Misspelled words in text fields get a red wavy underline, as they do in
Chrome, and spellchecking is **on by default**. Nothing needs to be written
to get it; this page is about turning it off, configuring it, and styling it.

## What is checked

- A `<textarea>`, and an `<input>` with no `type`, `type="text"` or
  `type="search"` (an unknown type counts as text, as in HTML).
- Never `password`, `email`, `url`, `number`, `tel` or the other non-prose
  types, and never a `readonly` field.
- A field is checked once the **user has interacted with it**: pressed in
  it, tabbed onto it, or typed in it. A screen of pre-filled fields shows no
  squiggles until the user touches one. A focus from code
  (`NodeHandle::focus`, a dialog's initial focus) does not count on its own,
  but the first edit after it does. This is Chrome's rule, measured in
  Chrome 158.
- On that first interaction the whole field is checked at once. After that,
  an edit re-checks only the words it touched, once typing has paused for a
  second (Chrome's idle delay): the word you are typing is not marked while
  you type it.
- A `<textarea>` keeps its marks when it loses focus; a single-line
  `<input>` hides them until it is focused again (as Chrome does).
- Setting `readonly` or `spellcheck="false"` on a checked field (or an
  ancestor) drops its marks at the next frame; setting it back checks it
  again. A field removed from the page is forgotten.
- Both Chrome rules above are the app's to change:
  `SpellcheckConfig::require_interaction(false)` checks every field as soon
  as it is laid out (and a value the app writes after the pause), and
  `hide_input_marks_on_blur(false)` keeps an `<input>`'s marks while blurred.
- The rich-text `Editor` is checked the same way: its first interaction (a
  press in it, a Tab onto it, typing) checks the whole document, and each
  edit re-checks the words it touched after the pause. Its marks map through
  every edit, so typing elsewhere never moves or re-checks them. A read-only
  editor is not checked (its marks go when it turns read-only and come back
  when it is editable again). Inside the editor, a block whose element says
  `spellcheck="false"` is left out, as Chrome does in an editable region;
  to leave out code, build the editor with
  `SpellcheckPlugin::new().skip_nodes(["code_block"]).skip_marks(["code"])`
  in its plugin list. After changing `spellcheck` on an ancestor of an
  editor, call `EditorHandle::refresh_spellcheck()`. The editor always waits for
  the user's interaction (`require_interaction` applies to fields).

Some words are never checked, a deliberate difference from Chrome for app
UI, where they are mostly codes and names: URLs, email addresses, words
containing a digit, and words in all capitals. The skip rules are
configurable (below); `SkipRules::none()` gives Chrome's behaviour.

## Turning it off

For one field or a whole subtree, set `spellcheck` to `false`. It is
inherited, as in HTML: the nearest ancestor that sets it decides, so a form
or a toolbar can turn it off for everything inside, and a field inside can
turn it back on.

```rust
rsx! {
    div { spellcheck: "false",
        input { }                        // not checked
        textarea { spellcheck: "true" }  // checked
    }
    TextInput { label: "Username", spellcheck: false }
}
```

The text components (`TextInput`, `Textarea`, `ColorInput`) and the `Editor`
take a `spellcheck: Option<bool>` prop; `PasswordInput` is never checked.

```rust
rsx! { Editor { editor: notes.clone(), spellcheck: false } }
```

For the whole app:

```rust
App::new(app).spellcheck(false).run();
```

## Configuring it

`App::spellcheck` takes a `SpellcheckConfig` as well as a `bool`:

```rust
use std::sync::Arc;
use rinch::spellcheck::{PersonalDictionary, SkipRules, SpellcheckConfig};

let personal = Arc::new(PersonalDictionary::with_words(load_saved_words()));
App::new(app)
    .spellcheck(
        SpellcheckConfig::new()
            .languages(["en_GB", "de"])        // the system's dictionaries for these
            .personal_dictionary(personal.clone())
            .skip_rules(SkipRules { all_caps: false, ..Default::default() })
            .debounce_ms(500),                 // the pause after typing
    )
    .run();
```

- **Dictionaries.** By default the system's Hunspell dictionaries for the
  user's locale (`$LANGUAGE`, `$LC_ALL`, `$LC_MESSAGES`, `$LANG`), from
  `$DICPATH`, `~/.local/share/hunspell`, `/usr/share/hunspell` and the other
  usual places. When none is installed nothing is marked; it is never an
  error. `.languages(..)` names the locales instead.
- **Your own checker.** `.checker(..)` takes any `rinch::spellcheck::SpellChecker`,
  for example a `HunspellChecker` over dictionaries the app bundles
  (`HunspellChecker::from_bytes("en_US", AFF, DIC)`).
- **The personal dictionary** is the app's to load and save:
  `PersonalDictionary::with_words` fills it, `add_listener` reports each
  change so the app can persist it.

Change the settings while the app runs with `rinch::spellcheck::configure(..)`
or `rinch::spellcheck::set_enabled(..)`: every mark is dropped and the fields
the user has already interacted with are checked again under the new
settings.

The dictionaries are loaded, and words checked, on a background thread
(`rinch-spellcheck`), started the first time a field is checked. An app that
turns spellcheck off, or whose user never touches a text field, starts
nothing and loads nothing. (On the web there is no such thread: see below.)

## Styling the squiggle

The colour and the line are CSS custom properties, inherited, so they can be
set for the whole app, a section, or one field:

```css
.notes { --rinch-spellcheck-color: #d1242f; }
.code-field { --rinch-spellcheck-style: none; }   /* no marks drawn */
```

| Property | Values | Default |
|---|---|---|
| `--rinch-spellcheck-color` | any CSS colour | Chrome's red (`#ff0000`) |
| `--rinch-spellcheck-style` | `wavy`, `solid`, `none` (the editor also takes `dotted`, `dashed`, `double`) | `wavy` |

In the editor a misspelled word is a `<span data-pm-deco class="pm-spell-error">`,
styled by the editor's stylesheet with those two properties; a rule of your own
for `[data-pm-editor] .pm-spell-error` replaces it entirely.

## On the web

**Text fields** (`<input>`, `<textarea>`) are the browser's: it checks them
with its own dictionaries, rules and squiggle, and rinch's checker never runs
on them. The `spellcheck` attributes and props are passed through to the real
DOM, and `App::spellcheck(false)` puts `spellcheck="false"` on `<body>`, which
the page's fields inherit; `rinch::spellcheck::configure` and `set_enabled`
while the app runs keep it in step.

**The rich-text `Editor`** is not `contenteditable` on the web (it takes
input through a hidden field), so the browser does not check it. rinch checks
it, with the same rules as on the desktop (the first press or edit checks the
whole document, an edit re-checks its words after the pause, `spellcheck` is
inherited, a read-only editor is not marked), **when the app supplies
dictionaries**. A browser lends a page none of its own, and rinch never
bundles any (a dictionary is about a megabyte of `.dic`), so with none
supplied the web editor is simply not checked, and no checking code runs.

Supply them as `.aff`/`.dic` bytes through `SpellcheckConfig::checker`, either
built into the binary:

```rust
use rinch::spellcheck::{HunspellChecker, SpellcheckConfig};

const AFF: &[u8] = include_bytes!("../dictionaries/en_US.aff");
const DIC: &[u8] = include_bytes!("../dictionaries/en_US.dic");

App::new(app)
    .spellcheck(
        SpellcheckConfig::new()
            .checker(HunspellChecker::from_bytes("en_US", AFF, DIC).unwrap()),
    )
    .run();
```

or fetched once the app is running, which keeps them out of the `.wasm` (an
editor already on the page is checked as soon as they arrive, its undo
history kept; one the user already pressed in is marked then):

```rust
use rinch::spellcheck::{HunspellChecker, SpellcheckConfig, configure};

async fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    let window = web_sys::window()?;
    let resp: web_sys::Response =
        wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
            .await.ok()?.dyn_into().ok()?;
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().ok()?).await.ok()?;
    Some(js_sys::Uint8Array::new(&buf).to_vec())
}

wasm_bindgen_futures::spawn_local(async {
    if let (Some(aff), Some(dic)) = (
        fetch_bytes("/dictionaries/en_US.aff").await,
        fetch_bytes("/dictionaries/en_US.dic").await,
    ) && let Ok(checker) = HunspellChecker::from_bytes("en_US", &aff, &dic)
    {
        configure(SpellcheckConfig::new().checker(checker));
    }
});
```

Parsing a dictionary happens on the page's thread when you call
`from_bytes`: en_US took 33 ms in a release build (350 ms in a debug one),
so do it once, at startup. Hunspell dictionaries for most languages are in
LibreOffice's `dictionaries` repository and in every Linux distribution's
`hunspell-*` packages; mind each one's licence.

A browser page has no second thread, so the checking is cut into slices of
about 3 ms of words each, one browser task apiece, so typing and scrolling are
never held up: a whole long document is marked over a few frames (a
650,000-character document took 42 slices, the longest 6.6 ms, in Chrome
157). Each slice's marks appear as it finishes.

The squiggle is the browser's own `text-decoration: underline wavy`, on the
same `<span class="pm-spell-error">` and with the same
`--rinch-spellcheck-color` / `--rinch-spellcheck-style` properties as on the
desktop.

**Not yet on the web:** spelling suggestions and "Add to dictionary" in the
editor's context menu. rinch-web leaves the browser's own menu on editable
targets, and the browser's menu knows nothing of rinch's marks.

## Testing

`rinch::testing::TestApp` and a `RinchApp` built directly have no spellcheck
configuration, so a test never reads the host's dictionaries and marks
nothing. A test that wants marks configures a checker over a dictionary it
ships (`rinch::spellcheck::configure(SpellcheckConfig::new().checker(..))`).
