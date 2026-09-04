# Fonts

An app that wants a particular typeface has two ways to get one: the platform's
font list, or its own bytes.

The platform list is not a promise anyone can keep. A phone has whatever its
OEM shipped. A Linux desktop has whatever the user installed. So a build with a
designed identity, or one whose content needs a *monospaced* face — a chord
chart, a diff, a table of figures, anything where the columns lining up **is**
the meaning and not decoration — has to carry the file.

## Shipping a face

Bundled fonts are one method on the [`App`] builder, so they compose with every
other startup option:

```rust,ignore
use rinch::prelude::*;

const FONTS: &[AppFont] = &[
    AppFont::serif(include_bytes!("../assets/fonts/Newsreader.ttf")),
    AppFont::sans_serif(include_bytes!("../assets/fonts/Karla.ttf")),
    AppFont::monospace(include_bytes!("../assets/fonts/DejaVuSansMono.ttf")),
];

fn main() {
    App::new(app)
        .title("My App")
        .size(900, 1200)
        .fonts(FONTS)
        .run();
}
```

and on Android, the same chain with a different terminal:

```rust,ignore
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    App::new(app).fonts(FONTS).run_android(android_app);
}
```

`.fonts()` is part of the startup configuration rather than something you
register beforehand because the faces have to be in place **before the first
layout pass**. A face that arrives after it cannot un-measure the frame that has
already been drawn: the first frame comes out in a fallback and then reflows.
Configuring them on the builder leaves no ordering to get wrong.

Calling `.fonts()` more than once registers all of them, in call order — unlike
`.title()` or `.menu()`, a second call adds rather than replaces.

`AppFont::data` is `&'static [u8]`, which is what `include_bytes!` gives you.
The file is a TrueType (`.ttf`), OpenType (`.otf`) or collection (`.ttc`).

## What a face is reachable *as*

This is the part that catches people out. Take a stack:

```css
font-family: Newsreader, Georgia, serif;
```

Every name in it is looked up one of two ways.

| In the stack | How it resolves | What a bundled face needs |
| --- | --- | --- |
| `Newsreader`, `Georgia` | by **family name** — the name inside the font file's own `name` table | nothing; registering the file is enough |
| `serif`, `sans-serif`, `monospace`, `system-ui`, … | by **generic** — a slot the platform fills, which is not the name of anything | the face has to be **declared** for that slot |

So `AppFont::new(bytes)` is reachable, but only by an author who spells its
name. If the stack's last entry is a generic — and essentially every real stack
ends in one — that generic is what has to resolve when the names ahead of it
miss, and it will not resolve to a bundled face by accident.

The constructors say which slot a face is claiming:

| | claims |
| --- | --- |
| `AppFont::new` | nothing — reachable by family name only |
| `AppFont::serif` | `serif`, `ui-serif` |
| `AppFont::sans_serif` | `sans-serif`, `system-ui`, `ui-sans-serif` |
| `AppFont::monospace` | `monospace`, `ui-monospace` |

Anything else is a struct literal, over the full set of CSS generics:

```rust,ignore
AppFont {
    generics: &[GenericFamily::Cursive, GenericFamily::Fantasy],
    ..AppFont::new(bytes)
}
```

A bundled face goes **ahead of** the platform's own entry for a slot —
including the face rinch's Android repair filled an empty slot with (below).
An app that ships a face for `monospace` means that one. What was in the slot
stays behind the claim rather than being discarded, so a character the claimed
face lacks can still fall through to the platform's entry.

Claim only what is true. A serif declared `sans-serif` will be picked for
`sans-serif`, which is precisely the problem.

## Android and `monospace`

Worth knowing if you are shipping to a phone. The text stack's Android backend
(`fontique`) maps the `monospace` generic by looking up a *font family named
`monospace`*, and its scan of `/system/fonts` indexes files under the family
names in their own `name` tables — where the device's actual monospaced file
(`/system/fonts/DroidSansMono.ttf`) is `Droid Sans Mono`. Nothing is called
`monospace`, so upstream the slot is left empty, and `font-family: monospace`
would resolve to no face at all.

rinch repairs this at font-context construction (issue #322): a generic slot
the platform left empty is filled with the best **platform** face the device
actually has — `Droid Sans Mono` on stock Android for `monospace` — so
`<code>` and `<pre>` render monospaced out of the box, with no bundling.

What the repair cannot do is pick *your* face: it answers an empty slot with
whatever the OEM shipped, whichever that is. An app whose content depends on a
particular monospaced design bundles the file and declares it —
`AppFont::monospace(...)` — and the declared face is picked ahead of both the
repair's choice and the platform's own map.

That precedence is the reason a claim is a **prepend** rather than an append.
The repair runs at context construction, before any app font can register, so
an appended claim would sit behind the repair's platform face and silently
lose — on Android only, which is the platform nobody would test it on.

## Platforms with no fonts at all

Wasm has no system font source: an unregistered app renders **zero glyphs**, not
a fallback. That case wants a face in the last-resort fallback chain as well as
in a generic slot, which is what [`RinchApp::register_font_data`] and
[`RinchContextConfig::fonts`] do, and what `AppFont::script_fallback` turns on.

Do **not** turn it on where the platform does have fonts. The text stack
resolves a script's fallback from the app's own list first and the platform's
second — as an alternative, not a chain — so an entry in the app's list
*replaces* that script's platform fallback rather than extending it. One
bundled Latin face becomes what CJK, Arabic and emoji fall back to.

The effect is measured, not hypothetical, and the tests pin it: with
`script_fallback` set on a Latin face, laying out `漢字` in a stack headed by
that face resolves to **the bundled face, at glyph 0** — `.notdef`, the empty
box — where the same text without it resolves to the platform's CJK face with
real glyphs. You will not always *see* it: if something else in the author's
stack already covers the character, fallback is never consulted. That makes it
worse, not better — the breakage shows up only for the scripts and stacks
nobody tested.

## Where the bytes come from

`include_bytes!` puts the file in the binary — in the `.so`, for an Android
build — and needs no packaging. On Android the more idiomatic alternative is to
put the files in the APK's `assets/` and read them through `AndroidApp`'s asset
manager, which keeps the shared object smaller; it is also more machinery, it
is Android-only, and the bytes it produces are owned rather than `'static`. The
API here takes `&'static [u8]`, so `include_bytes!` is the supported route
today.

[`App`]: ./windows.md
[`RinchApp::register_font_data`]: https://docs.rs/rinch/latest/rinch/app/struct.RinchApp.html#method.register_font_data
[`RinchContextConfig::fonts`]: https://docs.rs/rinch/latest/rinch/embed/struct.RinchContextConfig.html#structfield.fonts
