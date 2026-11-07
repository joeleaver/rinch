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

The file is a TrueType (`.ttf`), OpenType (`.otf`) or collection (`.ttc`).
The constructors above take `&'static [u8]`, which is what `include_bytes!`
gives you; a face whose bytes arrive while the app runs is owned instead (see
[Where the bytes come from](#where-the-bytes-come-from)).

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

Anything else names its generics, over the full set of CSS generics:

```rust,ignore
AppFont::new(bytes).with_generics(&[GenericFamily::Cursive, GenericFamily::Fantasy])
```

(`AppFont::SERIF`, `AppFont::SANS_SERIF` and `AppFont::MONOSPACE` are the lists
the three constructors claim.)

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

## A stack that names nothing installed

CSS falls back to the browser's default font when every family in a
`font-family` list is missing. rinch finishes such a stack with the first face
of `sans-serif` — so `font-family: Helvetica` on a Linux machine without it
draws its letters *and digits* in the default sans face, or in the face your
app registered for `sans-serif` (`AppFont::sans_serif`) if it did. (Chrome resolves
`Helvetica`, `Times` and `Courier` through fontconfig aliases, and an unknown
name in its serif default; rinch knows only the families actually installed or
registered.) A stack that finds any family, or names a generic that has one, is
used exactly as written — end your stacks in a generic to choose the fallback
yourself.

## Emoji

An emoji-presentation character — 😀, ⬜, a flag, or an emoji character
followed by U+FE0F (❤️, ©️) — is drawn from the `emoji` generic unless a family
in your stack covers it first, where each generic in the stack counts as its
**first face** only; the generics' other faces come after the `emoji` generic,
so a character the emoji face lacks still finds them. U+FE0F after a character
that is not an emoji (𝄞, Thai ก) changes nothing.
So under `font-family: sans-serif` (or the theme's default stack) an emoji is in
colour even on a machine whose `sans-serif` list goes on to text faces that
also have it (DejaVu Sans, FreeSans), as in Chrome; a family you *name* that
covers the emoji still draws it, as in Chrome, and an emoji face you name after
a generic (`…, sans-serif, 'Segoe UI Emoji'`) wins it. A character that is text
by default — ❤ without U+FE0F, ©, the digits — follows the stack as written.

## Platforms with no fonts at all

Wasm has no system font source: an unregistered app renders **zero glyphs**, not
a fallback. That case wants a face in the last-resort fallback chain as well as
in a generic slot, which is what [`RinchApp::register_font_data`] and
[`RinchContextConfig::fonts`] do, and what `AppFont::script_fallback` turns on.

Font data registered that way declares nothing about what it is for. It fills
a generic only where the **plain** slot (`sans-serif`, `serif`, `monospace`)
is empty, and only the slots of its own class, read from the font's own
tables:

| The face | Fills, when the plain slot is empty |
| --- | --- |
| sans serif, or no declared class | `sans-serif`, `system-ui`, `ui-sans-serif` |
| serif | `serif`, `ui-serif` |
| fixed pitch | `monospace`, `ui-monospace` |
| script, decorative or symbol class; color emoji (`COLR`, `CBDT`, `sbix` or `SVG ` tables); no `A` or `a` in its cmap (icon fonts) | nothing |

The `ui-*` and `system-ui` slots are filled only together with their plain
slot, never on their own, and a slot already holding a face is never
changed. So on wasm the first sans-serif text face you register is what
`sans-serif` means, whatever you register before or after it. On a desktop
the platform maps the plain generics (fontconfig maps no `ui-*` slot, but
those are left empty as well), so **registering font data changes no generic
slot**: `sans-serif` is the platform's sans-serif face, and the theme's
monospace stack, which starts with `ui-monospace`, still reaches the
platform's `monospace`. To make a face *the* `sans-serif` there, claim it
(`AppFont::sans_serif`, or `register_app_font` in an embed; see
[Game engine integration](./game-engine.md)).

### Migrating from before #2102

Until issue #2102 every such face was put at the head of `sans-serif` and
`system-ui`, so the face registered **last** — a script face as easily as a
text face — was what every `sans-serif` element drew in. An app that relied
on that, on a platform with fonts, now gets the platform's face instead. To
keep a registered face as a generic, either claim it —
`ctx.register_app_font(AppFont::sans_serif(BYTES))` in an embed, or
`App::fonts(&[AppFont::sans_serif(BYTES)])` — or name its family in the stack
(`font-family: "Liberation Sans", sans-serif`).

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

## Text rendering

How glyphs are rasterised is an app option with OS defaults, as in Chrome
(issue #2286). `App::text_rasterization` takes a `TextRasterization`, every field of
which defaults to `Auto`:

```rust
use rinch::prelude::*;

App::new(app)
    .text_rasterization(
        TextRasterization::default()
            .stem_darkening(StemDarkening::Off) // no thickening of light text
            .hinting(TextHinting::None),        // exact outlines, whatever the OS says
    )
    .run();
```

| Field | `Auto` means | Other values |
|---|---|---|
| `hinting` | The OS's hint style; `Slight` where it says nothing, `None` on macOS | `None`, `Slight`, `Medium`, `Full` (the last three all hint today) |
| `stem_darkening` | Light text is grown by up to 0.10 px a side | `Off`, `Px(amount)` (up to 0.5) |
| `antialias` | The OS's: subpixel where it names a subpixel order, else grayscale (macOS: always grayscale) | `Grayscale`, `Subpixel`, `None` (drawn grayscale) |
| `subpixel_order` | `None`: the OS's (fontconfig `rgba`, ClearType's orientation) | `Some(Rgb)`, `Bgr`, `VRgb`, `VBgr`; `Some(SubpixelOrder::None)` turns subpixel text off |
| `lcd_filter` | `None`: the OS's (fontconfig `lcdfilter`), FreeType's default filter where it says nothing | `Some(Default)`, `Light`, `Legacy`, `None` |
| `gpu_glyph_outlines` | `false`: the GPU draws glyphs from the glyph-mask atlas | `true`: from outlines (below) |

**Stem darkening.** Both renderers blend a glyph's coverage in sRGB-encoded
space, as FreeType's usual callers do, and that makes light text on a dark
background look about a quarter thinner than the same text dark on light. So
light text is thickened a little: by up to 0.10 device pixels on every side for
text whose colour is light (its sRGB luma, over black, from 0.5 to 0.85 ramps
from none to all of it), at sizes up to 24 device pixels, tapering to none at 64.
Only the text colour decides, as in Skia: paint does not know what lies under
the text, so light-grey text on a light background (the light theme's
placeholder grey, `#adb5bd` on white) is thickened too. Dark text is untouched.
The amount and the size taper are in device pixels, under CSS transforms too
(13px text under `scale(2)` is treated as 26px), and both renderers apply the
same amount, so the GPU and software renderers still draw the same glyphs.
Blurred `text-shadow`s follow their own colour: a dark shadow is not thickened.

**The OS's settings** are read once, on a thread of their own while the window
starts, and `rinch::text_rasterization::system_text_settings()` returns them:

- **Linux**: fontconfig's match for `sans-serif` — `antialias`, `hinting`,
  `hintstyle`, `rgba`, `lcdfilter`. KDE's font settings write these into the
  user's `fonts.conf`. GNOME's own font settings reach only GTK apps (through
  XSETTINGS) and are not read; fontconfig's defaults are GNOME's defaults.
  libfontconfig is opened at run time, so a system without it reads nothing.
- **Windows**: font smoothing and ClearType, with ClearType's subpixel order
  (`SystemParametersInfo`).
- **macOS**: grayscale and unhinted.

**How glyphs are drawn.** Both renderers draw text from one cache of glyph
masks (the glyph-mask atlas, #2286): each glyph is rasterised once per font,
device size and quarter-pixel horizontal position, and drawn at a whole device
pixel, so letters keep their fractional spacing and the GPU and software
renderers draw the same pixels. Text under a rotation, a skew or an uneven
scale, emoji and other colour glyphs, text larger than 64 device pixels and
synthetic italics (below) are drawn from outlines on the GPU instead; a uniform scale
(a HiDPI display, CSS `scale()`) is rasterised at the scaled size. Text whose
size is animating (a `font-size` or `scale()` transition or animation) is
drawn from outlines while it changes, since each size would be a new set of
masks, and goes back to the atlas once it settles. The two rasterisers differ
slightly, so on the frame the text starts or stops animating its glyph edges
shift by up to about half a pixel and its weight by about 1%; the animation
itself hides the first, and the second is one frame. An app can keep outlines
for everything with `TextRasterization::default().gpu_glyph_outlines(true)`.

**Synthetic bold and italic.** A family with no bold face, set in
`font-weight: bold` (any weight more than 200 over the face's), is drawn from
its regular face emboldened, as Chrome draws it: each side of a glyph's outline
moves out by `size / 48` up to 9px, `size / 64` from 36px and in between
proportionally (an `l` at 16px is 0.62 px wider), the same on both renderers,
and on top of any stem darkening. A family with no italic face (and no
italic or slant axis), set in `font-style: italic`, is slanted by 14 degrees.
Shadows of such text are emboldened and slanted with it. Ship the real face
(`App::font`) where the weight matters: a synthesised bold is heavier than the
regular but is not the designer's bold, and its advances are the regular
face's.

**Subpixel (LCD) text.** Where the OS draws subpixel text — fontconfig's `rgba`
on Linux (KDE's "Sub-pixel rendering: RGB" writes `rgba=rgb`), ClearType on
Windows — rinch does too, on both renderers: each glyph is rasterised at three
times the resolution across the panel's subpixels, filtered with the OS's LCD
filter (FreeType's `lcddefault` or `lcdlight`, in FreeType's arithmetic;
`lcdlegacy` is `lcddefault`: FreeType 2.14 no longer has the legacy filter) and drawn per channel, so its edges use the red, green and blue
subpixels of a pixel separately, as KDE's and Chrome's text does. macOS has
drawn no subpixel text since 10.14, and rinch never does there. Subpixel text
needs to know the pixel under it, so, as Chrome and Skia do, rinch draws grey
text instead:

- inside an opacity group (any `opacity` below 1), a `mask-image`, a `filter`,
  or anything else drawn into a layer of its own;
- over a pixel that is not opaque: in a transparent window where nothing opaque
  is under the text, or over a translucent background (decided per pixel, so
  text on an opaque panel in a transparent window is still subpixel);
- under a rotation, a skew or an uneven scale, and while its size animates
  (the outlines those draw are grey); a uniform scale (HiDPI, `scale()`) is
  rasterised at the device size and stays subpixel;
- for emoji and other colour glyphs, and text larger than 64 device pixels on
  the GPU.

While a window shows a `RenderSurface`, a `GameViewport` or a video layer, the
GPU renders the UI over a transparent base (the layers show through it), so
text with no opaque background of its own under it turns grey for as long as
the layer is on screen, and back when it goes; give such text an opaque
background (most pages have one on `body`) to keep it subpixel. **Embedding
hosts** (`RinchContext`): subpixel text is right only when the UI texture
reaches the screen pixel for pixel. A host that scales the texture, draws it
at a fractional offset or maps it onto a surface in 3D should ask for
`TextAntialias::Grayscale` through `set_text_rasterization`.

Clips (`overflow: hidden`, rounded or not) do not stop it. Text that fades in
with an `opacity` animation is grey while it fades and turns subpixel when the
animation ends at `opacity: 1`, as in Chrome. At a fractional scale factor
(125%, 150%) the masks are still rasterised for the device pixels the window is
drawn at, so the subpixel grid lines up with the panel as long as the
compositor shows the window's pixels unscaled (Wayland's fractional-scale
protocol, Windows' per-monitor DPI). An app can set the order for a panel the
OS reports wrongly, or not at all (a BGR monitor, one rotated to portrait,
`VRgb`), or turn subpixel text off:

```rust
App::new(app)
    .text_rasterization(
        TextRasterization::default()
            .subpixel_order(Some(SubpixelOrder::Bgr)) // or Some(SubpixelOrder::None): grey
            .lcd_filter(Some(LcdFilter::Light)),
    )
    .run();
```

The colour fringes are visible when a screenshot is scaled or shown on another
display; an app that takes screenshots for others to see can ask for
`TextAntialias::Grayscale`. Subpixel coverage is blended in sRGB-encoded space
like grey coverage (and like FreeType's usual callers, KDE's included), and
takes the same stem darkening. `antialias: None` (fontconfig `antialias=false`)
is drawn grayscale: rinch has no aliased text.

An
embedding host that wants the OS's settings calls
`rinch::text_rasterization::set_text_rasterization(TextRasterization::default())` before its
first frame; without it an embedded document renders as if the OS said nothing
(grayscale, slight hinting, stem darkening on).

## Where the bytes come from

`include_bytes!` puts the file in the binary — in the `.so`, for an Android
build — and needs no packaging. Bytes that arrive while the app runs — a font
the user imports, a game's own fonts, a file read from the APK's `assets/`
through `AndroidApp`'s asset manager — are owned, and every registration takes
them as [`FontData`]: a `Vec<u8>`, a `Box<[u8]>`, an `Arc<[u8]>` or a parley
`Blob<u8>`, each with `.into()`, as well as `&'static [u8]`:

```rust,ignore
let bytes: Vec<u8> = std::fs::read("UserFont.ttf")?;
let face = AppFont::from_data(bytes).with_generics(AppFont::SANS_SERIF);
App::new(app).fonts(&[face]).run();
```

Owned bytes are **not leaked**: rinch keeps a reference for as long as an app
(or an embed's `RinchContext`) that registered them lives, and drops it with
that app. The tests pin it by the `Arc`'s count after a mount that laid out and
painted text in the face (issue #2101). Before that, the API took only
`&'static [u8]`, and an app with run-time fonts had to leak each one.

A registered face cannot be **unregistered** from a running app. fontique has
`Collection::unregister_font` (per family, width, style and weight), but the
face also lives in every layout, shaping and glyph cache that used it, so
removing it means relaying out and repainting everything; rinch does not do
that. A face's bytes are freed when the app or context that registered them is
dropped, so an app whose fonts change for good builds a new context.

[`FontData`]: https://docs.rs/rinch/latest/rinch/font/struct.FontData.html

[`App`]: ./windows.md
[`RinchApp::register_font_data`]: https://docs.rs/rinch/latest/rinch/app/struct.RinchApp.html#method.register_font_data
[`RinchContextConfig::fonts`]: https://docs.rs/rinch/latest/rinch/embed/struct.RinchContextConfig.html#structfield.fonts
