# Platform Features

Rinch provides optional platform integration features that can be enabled via Cargo features.

## Image Loading

Images work out of the box for local files. Both `<img>` elements and `background-image: url(...)` CSS are supported. Images load asynchronously on background threads and render when decoded.

### Local Files (built-in)

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div {
            // img element with object-fit
            Image { src: "photo.png", width: "200", height: "150", fit: "cover" }

            // Avatar with image
            Avatar { src: "avatar.png", size: "lg" }

            // background-image via CSS
            div { style: "width: 300px; height: 200px; background-image: url(photo.png); background-size: cover;" }
        }
    }
}
```

Supported formats: PNG, JPEG, GIF, WebP, and SVG.

**EXIF orientation** (#2785). A photograph stored sideways with an EXIF
orientation (a phone held upright saves orientation 6) is drawn upright, as
browsers draw it by default (`image-orientation: from-image`). Its natural size
is the turned size: a 4000x3000 file with orientation 6 lays out 3000x4000. As
in Chrome, the orientation is read from a JPEG's EXIF, from a PNG's `eXIf`
chunk only when it comes before the image data, and never from a WebP. The
pixels are turned once, when the file is decoded. `image-orientation: none` is
not supported yet (Stylo, the CSS engine, does not parse the property in
rinch's build; #2802), so the orientation is always applied. On the web backend
(rinch-web) the browser decodes and orients `<img>` itself.

**Colour profiles** (#2779). A file tagged with an ICC colour profile that is
not sRGB, such as a Display P3 or Adobe RGB photograph, is converted to sRGB
when it is decoded, as Chrome converts it (rinch draws in sRGB). The profile is
read from a JPEG's `APP2`, a PNG's `iCCP` or a WebP's `ICCP`. A greyscale
file with a grey profile (a gamma or curve other than sRGB's) is converted
through that curve too (#2807). A file with 16 bits a channel is converted at
16 bits and rounded to 8 once, after the conversion, so a smooth wide-gamut
gradient does not pick up steps (#2807; Chrome rounds such a file to 8 bits
first, so rinch's result can differ from Chrome's by a code or two). Untagged and
sRGB-tagged files are not converted and cost nothing extra. A tagged file costs
about 2.5 ns a pixel more to decode (20 ms for a 3840x2160 picture). A file
or network load decodes on a background thread, but a `data:` URL, and every
load in the wasm32 embed build, decodes on the UI thread, so there that time
is spent in the frame. The conversion is
the `color-management` feature of rinch-dom, which `rinch`'s `windowing`
(and so `desktop`), `android` and `embed` features turn on. To turn it off for the whole process at
run time, for example when the app's files are already converted, call
`rinch::image::set_color_management(false)` before the images load.
Pixels handed over decoded (`ImageLoadResult::Decoded`, `register_rgba`) are
never converted. On rinch-web the browser does its own colour management.

**SVG files** (#2111) work wherever an image does — `<img src="icon.svg">`,
`background-image`, `mask-image`, an app scheme's bytes, and a `data:` URL in
either form (`data:image/svg+xml;base64,…` or the percent-encoded
`data:image/svg+xml,%3Csvg…`). One is recognised by its content (an `<svg>`
root element), whatever the source is called. It is drawn **at the size it
covers on the screen**, DPI scale and CSS transforms included, so an icon stays
sharp at any size; its rasters are kept per size (64 MB of them at most, the
ones on screen always kept). It is laid
out as a browser lays it out: its natural size is the root's `width` and
`height` when they are absolute lengths, a missing one follows from the
`viewBox`'s ratio, and an SVG with only a `viewBox` and no CSS size fills its
container's width (an inline one is 300px wide; see below). Drawn into a box of
another shape, the `viewBox` is fitted by the file's `preserveAspectRatio`
(centred, by default); a file with no `viewBox` is stretched.

#### The SVG profile

An SVG file is untrusted input. rinch draws one only if everything in it is
inside a **profile** of SVG, the features icon sets and editors use that are
cheap and bounded to draw. A file that uses anything outside the profile is
**refused whole**: it draws nothing (a failed image: an `<img>` shows its
[placeholder](#missing-and-failed-images-placeholders), a
background is not painted, a mask hides its box), and the reason is logged
once. It is never drawn in part. Measured on 11,843 real files (all 5,986
Tabler icons; samples of KDE Breeze, Oxygen, GNOME Adwaita and hicolor app
icons; 530 Inkscape and 7 Illustrator files among them; wallpapers), 99.97%
draw.

**Drawn:** `svg`, `g`, `defs`, `symbol`, `use`, `a`, `switch`, `path`,
`rect`, `circle`, `ellipse`, `line`, `polyline`, `polygon`,
`linearGradient` and `radialGradient` (at most 64 `stop`s each), `title`,
`desc`, `metadata`, `style`; `clipPath`, `mask`, `pattern` and `marker`
one level deep; and `filter` with at most 6 primitives, each one of
`feGaussianBlur`, `feOffset`, `feFlood`, `feColorMatrix`, `feComposite`,
`feBlend`, `feDropShadow` and `feMerge` (at most 8 inputs). That covers drop
shadows and glows, including Inkscape's five-primitive drop shadow.
Presentation attributes, `style=""`, `class`, gradients, opacity, strokes and
dashes all work.

**Ignored** (drawn as nothing; the rest of the file draws): `<text>` (rinch
gives the SVG renderer no fonts, so convert text to paths), `<image>` (an
SVG file reads nothing outside itself: not a file, a URL or a `data:` URL),
`<foreignObject>`, and elements an SVG renderer does not draw, such as an
editor's `sodipodi:namedview` or Inkscape's `flowRoot`.

**`<style>` sheets** may hold rules whose selectors are a type, `*`, `.class`
or `#id` (or several together, `rect.a#b`), comma-separated, with at most one
combinator (`g rect`, `g > .a`, `a + b`, `a ~ b`). rinch applies them itself,
by specificity and order, between the presentation attributes and the
element's `style=""` (`!important` honoured).

**References** (`href`, and `url(#…)` in `fill`, `stroke`, `clip-path`,
`mask`, `filter` and the markers) resolve as Chrome 156 resolves them: to
the first element with that id **of the kind the property takes**, and a
reference to nothing of that kind is ignored (a `fill` falls back to its
fallback colour, or `none`; a missing clip path, mask or filter draws the
element unclipped, unmasked, unfiltered). An `href` is read only on `<use>`
and on a gradient (to inherit another's stops).

**Refused, and why:**
- Any other element an SVG renderer would draw: the other filter primitives
  (`feTurbulence`, `feMorphology`, `feConvolveMatrix`, the lighting filters,
  `feDisplacementMap`, `feTile`, `feImage`, `feComponentTransfer`), each of
  which can cost seconds per frame from a file of a few hundred bytes.
- A `clipPath`, `mask`, `pattern` or `marker` that itself uses one (an
  `href` on one of them is ignored). A chain of masks is a full-size layer
  per link.
- A filter of more than 6 primitives, an `feMerge` of more than 8 inputs, or a gradient of more than
  64 stops.
- In `<style>`: an `@` rule (`@media`, `@import`), a pseudo-class, an
  attribute selector, more than one combinator, more than 1,024 selectors.
  A long descendant chain makes a matcher backtrack for seconds.
- A doctype whose internal subset holds anything but flat
  `<!ENTITY name "value">` declarations, each name once. Illustrator's
  `&ns_svg;` entities are fine; an entity that refers to another is not.
  References expand to at most 1 MB in all.
- Nesting deeper than 128 (as written, or once `<use>` is expanded), or more
  than 100,000 elements once `<use>` and other references are expanded.
- At draw time: a pattern tile over 4M device pixels or a filter layer over
  16M pixels refuses that size of the image; so does a raster estimated at
  more than about half a second of work on one core (a backstop: millions
  of dashes, dozens of canvas-sized translucent or many-stop gradient
  shapes at 2x). Every icon and illustration in the 11,843-file corpus
  draws at 48px; at 1024px, 9 of a 1,469-file sample (filter-heavy app
  icons and cursors, 0.1 to 0.55 s each) are refused at that size.

Inline `<svg>` elements in the DOM are a different path and are unaffected.

Two differences from Chrome: rinch rounds a layout to whole pixels (an SVG of
`width="10.5"` is 11 wide), and an *inline* `<img>` of an SVG with only a
`viewBox` is 300px wide where Chrome fills its container (a block one fills it
in both).

**Cost.** The file is parsed on the load thread, but it is rasterised on the UI
thread, inside the paint that first draws it at each size. An icon costs tens
of microseconds. A full-window illustration costs what its shapes cover:
measured (release build, one core of a loaded desktop), 2,000 small opaque
shapes took 31 ms at 1200x800 and 72 ms at 2x; 2,000 half-transparent shapes
each spanning the canvas took 0.65 s and 1.8 s. An SVG drawn at a size that
keeps changing — a full-window `background-size: cover` while the window is
resized, a scale animation — is rasterised again at each new size, so keep
large or complex SVG backgrounds at a fixed size, or use a raster image.

A `data:` URL is decoded as Chrome decodes one: its data is percent-decoded,
and base64 may leave out its padding and contain white space.

An `<img>` is sized as a browser sizes a replaced element: with no CSS size it
is its natural size — `display: block` included, so `margin: 0 auto` centres
it rather than it filling its container — and a lone `width` or `height` (or a
`min-*`/`max-*` clamp) gives the other dimension through the image's aspect
ratio. A flex container still stretches an `<img>`, as in a browser; a grid
container also still stretches it, where a browser does not (#1280). An
image not yet loaded has no natural size: with no size from anywhere it is 0x0.

The `width` and `height` **attributes** reserve the box before the image
arrives, as in a browser (#684):

```rust
img { src: "photo.jpg", width: "640", height: "480" }
```

Each is a length in CSS pixels (`"640"`) or a percentage (`"50%"`), and is a
*presentational hint*: it has less weight than any declaration in your own
CSS, so a stylesheet rule or an inline `style` for `width`/`height` wins (it
still outranks the built-in browser-default sheet). Together they
also give the image an aspect ratio until it loads, so the common responsive
reset `img { width: 100%; height: auto }` keeps the right height from the
first frame; once the image has loaded its own ratio is used. The ratio is
a hint too (`aspect-ratio: auto w / h`), so your own `aspect-ratio` replaces
it: `aspect-ratio: auto` drops it, and `aspect-ratio: 1` makes even a loaded
image square. An image with no picture (no `src`, or a failed load) keeps
its attributes' box and draws a placeholder in it
([below](#missing-and-failed-images-placeholders)). `<video>` reads
the same two attributes with the same ratio, and `<iframe>` reads them with
no ratio. The other legacy attributes (`hspace`, `vspace`, `border`, `align`,
a table's `width`/`bgcolor`/`cellspacing`/`cellpadding`, …) map to CSS as a
browser maps them, and their values are read as Chrome reads them:
`cellspacing="10px"` is 10px of spacing, and `border="abc"` on a table is a
1px border.

### Inline SVG

An `<svg>` is sized as a browser sizes it (#1797). Its `width` and `height`
attributes are presentation attributes: each is a CSS length (a bare number
is pixels, `"1em"` and `"50%"` work too) with less weight than any
declaration in your own CSS, so an icon needs no `style` to show:

```rust
svg { width: "14", height: "14", viewBox: "0 0 16 16",
    path { d: "M2 8h12", stroke: "currentColor" }
}
```

With `width`/`height` left `auto` by CSS, the attributes are its natural
size and their ratio (or, with one attribute or none, the `viewBox`'s)
carries one dimension to the other: `width: "14"` with `viewBox="0 0 16 8"`
is 14x7. With no size from anywhere an `<svg>` is 300x150, as in a browser;
a block-level one with only a `viewBox` fills its container's width. An
inline one with only a `viewBox` is 300 wide on desktop, where a browser
fills the containing block's width: give it a size.

On desktop rinch draws the shapes itself: `path`, `rect` (with `rx`/`ry`
rounded corners, SVG 2's rules: one radius alone sets both, each is clamped
to half its side, unequal radii give elliptical corners), `circle`,
`ellipse`, `line`, `polyline` and `polygon`, at any depth inside `<g>`
groups. A group's `fill`, `stroke`, `stroke-width`, `stroke-linecap` and
`stroke-linejoin` are inherited by its content (a child's own value wins, and
an inherited `currentColor` takes each shape's own `color`), and a
`transform` attribute (`matrix`, `translate`, `scale`, `rotate` with an
optional centre, `skewX`, `skewY`, as a list) moves a group or a shape; a
stroke is scaled with it, as in a browser. Exported logos usually need
exactly this. Not drawn: `<text>`, `<use>`, `<image>`, gradients and other
`url()` paint (a `url(#id) fallback` paints the fallback), clip paths, masks,
filters, `opacity`/`fill-opacity`/`stroke-opacity` on a shape or group,
`fill-rule`, and dashes. Content outside the `<svg>` box is not clipped as a
browser clips it (#1990).

### Background layers

A background image — a `url(...)` or a `linear-gradient`/`radial-gradient` —
is sized, placed and tiled the way a browser does it, for **every** layer of a
comma-separated list (#468, #1142): the colour is painted first, then the
layers from the last to the first, each with its own entry of the size,
position, repeat and origin lists (a shorter list repeats). A
`background-position` transition or animation moves only the first layer; the
others take their new value at once (#1742). Per layer:

- `background-size`: lengths, percentages, `auto`, `cover`, `contain`. An
  image's `auto` is its intrinsic size; a gradient's is the positioning area.
- `background-position` (and `-x`/`-y`): lengths, percentages, keywords and
  edge offsets (`right 10px top 0`). A percentage is a share of the area
  *less* the tile, so `100%` puts the tile's far edge on the area's.
- `background-repeat`: `repeat`, `no-repeat`, `repeat-x`, `repeat-y`. `space`
  and `round` are painted as `repeat`.
- `background-origin`: `padding-box` (the default), `border-box`,
  `content-box`.
- `background-clip` (and `-webkit-background-clip`): `text` paints the layer
  only inside the glyphs of the element's text and its in-flow descendants'
  (see *Gradient and outlined text* below). Every other value paints the
  border box, as `border-box` does.
- `background-color` is painted under the image.
- `background-blend-mode` (#2221): every mode of the CSS list (`multiply`,
  `screen`, `overlay`, `darken`, `lighten`, `color-dodge`, `color-burn`,
  `hard-light`, `soft-light`, `difference`, `exclusion`, `hue`, `saturation`,
  `color`, `luminosity`; `normal` is the default). Each layer blends with the
  layers and the colour under it, never with what is behind the element, as
  in a browser.

Tinting an image by a colour, the way a game UI multiplies an image by its
tint, is an image layer multiplied by its `background-color` and masked by the
same image, so its transparent parts stay clear:

```rust
div { style: "width: 64px; height: 64px; \
              background-image: url(icon.png); background-size: 100% 100%; \
              background-color: #ff8040; background-blend-mode: multiply; \
              mask-image: url(icon.png); mask-size: 100% 100%;" }
```

`mix-blend-mode` and `isolation` are not read on desktop. On the web, the
browser reads all of these itself.

Gradients follow CSS Images (#1798): stop positions may be lengths or
percentages, and a stop may carry two positions (`transparent 0 31px`);
`repeating-linear-gradient`, `repeating-radial-gradient` and
`repeating-conic-gradient` repeat their stops; a radial gradient takes its
shape, size (`circle 40px`, `120% 60%`, `closest-side`, `farthest-corner`, …)
and `at` position; `conic-gradient` (#2719) takes `from <angle>`,
`at <position>` and stops at angles or percentages, hard stops included, so a
pie chart or a gauge is CSS:

```rust
div { style: "width: 96px; height: 96px; border-radius: 50%; \
              background: conic-gradient(from 240deg, #0c6, #fc0 120deg, \
                          #f33 200deg, #444 200deg 240deg, transparent 240deg);" }
```

On the GPU renderer a conic gradient's hard stops snap to steps of about 0.7°
(Vello reads gradients from a 512-entry colour ramp), so a gauge whose stop
is driven by a value moves in those steps there, and smoothly on the software
renderer.

Colours fade through transparency as in a browser (premultiplied). Not
supported: interpolation hints and `in <colorspace>`; `to top right` is always
45°.

### Gradient and outlined text

The CSS a game HUD's numerals are drawn with works on both backends (#2719):

```rust
div { style: "font-size: 48px; \
              background: linear-gradient(#fff6b0, #ffb000 46%, #e04000 54%, #7a1000); \
              -webkit-background-clip: text; background-clip: text; color: transparent; \
              -webkit-text-stroke: 5px #1a1a2e; paint-order: stroke fill; \
              text-shadow: 3px 4px 0 #000;",
      "P1 1:23.456" }
```

- `background-clip: text` (and `-webkit-background-clip: text`), per layer: a
  gradient, colour or image clipped to the text. The text is drawn over it as
  usual, so `color: transparent` (or `-webkit-text-fill-color: transparent`)
  shows the background alone. A stroke widens the clip, as in Chrome;
  decorations and shadows do not.
- `-webkit-text-fill-color`: the glyphs' fill, instead of `color`. Decorations
  keep `color`.
- `-webkit-text-stroke` (`-webkit-text-stroke-width`,
  `-webkit-text-stroke-color`): an outline centred on each glyph's outline,
  mitred. A width in `px`, `em`, `rem`, `pt` and the other absolute units, or
  `thin`/`medium`/`thick`; `currentcolor` is the text's `color`.
- `paint-order: stroke fill` (or any list putting `stroke` before `fill`): the
  stroke under the fill, so it does not eat the glyph. The default draws it
  over the fill, as Chrome does.
- A `text-shadow` is cast by the fill, as in Chrome: under a stroke drawn over
  the fill, over one `paint-order` draws first.

All four inherit; `background-clip` does not. Hit testing, selection and
layout are those of the plain text. A `var()` works in all of them, the
`-webkit-text-stroke` shorthand included (#2820), and a value it leaves
invalid inherits the parent's, as in Chrome; a CSS-wide keyword from a
`var()` fallback (`var(--x, initial)`) applies, and a `calc()` stroke width
is read (#2870). Not supported: an `em` width is resolved where it is used, where Chrome resolves it
where it is declared and inherits the length; transitions and animations of
the four (they change at once); `background-clip: text` on an inline element
with a gradient background (inline elements paint only a background colour)
or in the `background` shorthand; the "…" of `text-overflow: ellipsis` is
drawn unstroked; and a selection does not restyle the text (Chrome draws
selected text in the `::selection` style, which drops the stroke under an
author `::selection` rule).

On the web the browser draws all of these itself.

### Masks

`mask-image` takes gradients (#1798) and images (#2099), as `mask-image` or
`-webkit-mask-image`, alone or in the `mask` / `-webkit-mask` shorthand. The
element and everything it paints are kept where the mask is opaque and clipped
to its border box. This is the CSS way to tint a single-colour icon:

```rust
div { style: "width: 24px; height: 24px; background-color: #ff4040; \
              mask-image: url(heart.png); mask-size: 100% 100%;" }
```

Each mask layer is sized, placed and tiled like a background layer, by its own
entry of `mask-size`, `mask-position`, `mask-repeat` and `mask-origin`. The
difference is that `mask-origin` starts at `border-box`. The layers are added
together (`mask-composite: add`). An image mask loads like a `background-image`:
from files, from an app's image scheme (`register_image_scheme`), and over the
network with `image-network`. Until it has loaded, or if it fails, the element
is masked away entirely, as in a browser.

Not read on desktop:
- `mask-mode`: every layer is an alpha mask, so `luminance` is drawn as
  `alpha`.
- `mask-clip` (always the border box), `mask-composite` (always `add`), and
  `mask-type`.
- A `url(#id)` reference to an SVG `<mask>` element.

An SVG file is a mask image like any other (#2111): `background-color` under
`mask-image: url(icon.svg)` is the usual way to tint an icon, and the icon is
rasterised at the size the mask draws it. A `data:` URL works, as it does for
`background-image`. On the web, the browser reads all of these itself.

### Filters

`filter` takes `blur()` and `drop-shadow()` (#2719) as well as the colour
functions, in any order and chained as in a browser: each function is applied
to what the one before it drew.

```rust
// Big numerals with a soft shadow that follows the glyphs, not the box.
div { style: "font: bold 52px/64px sans-serif; color: #fc3; \
              filter: drop-shadow(2px 3px 2px rgba(0, 0, 0, 0.6));", "218" }
// A glow behind a panel.
div { style: "background: rgba(80, 160, 255, 0.5); filter: blur(6px);" }
```

`drop-shadow()` casts the shadow of what the element draws — its text, an
icon, a rounded box, its children — rather than of its box, which is what sets
it apart from `box-shadow`. Its third length is the blur's standard deviation,
as in Chrome. Any `filter` other than `none`, a `blur(0)` included, makes the
element a stacking context and the containing block of its absolutely
positioned descendants. A filter does not change hit testing: the element is hit
in its border box, and its shadow and blur are not hit.

How it is drawn, on both painters: an element whose list holds a `blur()` or a
`drop-shadow()` is painted, with everything inside it, into an offscreen image
the size of the part that can show, grown by how far the filter spreads. The
list is then applied to that image, and the result is composited back at the
element's `opacity`. On the GPU painter, desktop and the embed API alike, that
image is drawn on the CPU and handed to Vello. A list of `drop-shadow()`s alone,
at full opacity, takes only the shadows from that image: the element itself is
drawn by the painter as usual. The blur is Chrome's software one (three box
blurs), so the result is within a few levels of Chrome's pixels. The colour
functions in such a list are applied exactly; a list of colour functions alone
is drawn with the overlays it always was. The image is
kept: a later paint draws it again as an image, uploaded once on the GPU,
until something inside the element changes, the element moves or resizes, or
its filter, transform or scale changes. A HUD whose readout ticks redraws the
readout's filter each frame and draws the others from their kept images, the
embed overlay's whole-scene paint included. A partial repaint that shows only a
small part of a large filtered element draws just that part and keeps nothing.
Kept images together hold at most 48 MiB per document (each window has its
own), the least recently drawn dropped first, and an element scrolled more than
a window away from the window drops its image. Filtered content that needs more
than that on screen at once is rasterised again on every full repaint: the
images evict each other. Registering a font (or a system-font refresh) drops
every kept image. `filter_rasters`, `filter_raster_pixels` and `filter_cache_hits` in
`RINCH_PERF` count the work. An element without one of these filters costs
nothing extra.

A `filter` on the root `<html>` element is not applied (put it on `<body>`
or a wrapper), and a `url()` filter is not parsed, so it is no filter at all.
Not drawn inside a filtered element: a GPU frame (`RenderSurface` drawing with
the GPU, `GameViewport`, a host texture), which is not in the CPU image. A
`url()` filter reference is not read. A `drop-shadow()` offset is rounded to
whole device pixels. On the web the browser draws `filter` itself.

### Backdrop filters

`backdrop-filter` (#2719) filters what is painted **behind** an element, inside
its border box (rounded by its `border-radius`), under the element's own
background and content. It takes the same functions as `filter`; `blur()` is
the frosted-glass look:

```rust
div { style: "border-radius: 14px; background: rgba(255, 255, 255, 0.18); \
              backdrop-filter: blur(10px) saturate(1.4);", "218 km/h" }
```

What counts as "behind" is what the CSS spec calls the backdrop root: the
nearest ancestor with a `filter`, an `opacity` below 1, a `mask-image` or a
`backdrop-filter` of its own, or else the whole page. A glass panel inside a
translucent group blurs only that group's content, and is then faded with it,
as in Chrome. The blur reads nothing outside the element's box: at its edges
the backdrop is mirrored. The element's `opacity` fades the filtered backdrop
with the rest of it. Like `filter`, a `backdrop-filter` makes the element a
stacking context and the containing block of its absolutely positioned
descendants, and does not change hit testing. A list that changes nothing
(`blur(0)`) costs nothing; the component library's `Modal` and
`LoadingOverlay` declare one by default, and their `overlay_blur` prop now
blurs.

How it is drawn, on both painters: what is behind the element, inside its
box, is painted a second time into an offscreen image, from the backdrop root
up to the element, on the CPU; it is filtered and drawn under the element. The
result is kept and drawn again as an image until something behind the element,
within the blur's reach, changes (or the element moves, resizes, or its filter
or the scale changes). Changes inside the element, or in a later positioned
sibling drawn over it — a modal's panel over its blurred overlay — keep it.
(Over text, any restyle anywhere repaints it for now, #2734.) A miss costs that
second paint and the filter over the part of the box that shows (`backdrop_rasters`, `backdrop_raster_pixels` and `backdrop_cache_hits`
in `RINCH_PERF`).

What the backdrop does not include: a GPU frame (`RenderSurface` drawing with
the GPU, `GameViewport`, a host texture), and anything the host draws under the
embed overlay. An in-game HUD drawn with the embed API blurs only what rinch
itself paints behind the panel, not the game. Backdrop elements may nest four
deep in one another's backdrops; past that one draws none. On the web the
browser draws `backdrop-filter` itself.

### Missing and failed images: placeholders

An `<img>` with no picture — no `src` (or an empty one), or a load that
failed — draws a **placeholder** in its place (#2309). One still loading
draws nothing until it arrives.

| The image | What it draws |
|---|---|
| has both dimensions without its picture (its `width` and `height` attributes or CSS, or one of them and the attributes' ratio) | a box of that size with a faint 1px `silver` border inside its content box, and its `alt` text inside that (1px in from the border), wrapped to the box and clipped to it — but nothing when the box is thinner than **3px** on either axis (no `src`) or **18px** (a failed load), as in Chrome, so spacers and dividers stay invisible (one with alt text shows the text inline instead, its size ignored, as Chrome does) |
| has `alt` text and not both dimensions | the alt text as text, an inline-block of it: as wide as the text up to the room it has (a long one wraps), or its declared width; as tall as its lines, or its declared height, and at least 16px (Chrome's alt box holds a 16x16 icon), with its text clipped to it |
| has neither, or every declared size is 1px or less (a tracking pixel) | nothing, and it takes up only its declared size |

The alt text is drawn in the image's own font, `line-height` and `color`, so
`img { color: gray; font-size: 12px }` styles it. It is shaped once, when the
image is styled or its `alt` changes, not on each paint. Reloading a failed
source (`reload_image`) keeps its placeholder up until the answer lands. A sized placeholder keeps
the ratio of its `width` and `height` attributes under the usual responsive
rule (`max-width: 100%; height: auto`), so a page does not jump when the
picture arrives; a `src` set later replaces the placeholder.

This is for mail above all: a mail client that holds back remote pictures
(Postrinch strips their `src` and keeps `width`, `height`, `alt` and `style`)
gets a layout with every picture's place kept and labelled.

**Restyling or turning it off.** The look is read from custom properties,
only while the image is a placeholder. They inherit, so a rule on `:root`
sets them for the whole app, and a rule on one image sets its own. The
`img { border: 0 }` resets mail is full of do not touch them.

```css
:root {
  --rinch-img-placeholder-border-color: #ddd;   /* a colour; default silver */
  --rinch-img-placeholder-border-width: 2px;    /* an absolute length or em, 0 for none; default 1px */
  --rinch-img-placeholder-background: #f6f6f6;  /* a colour; default transparent */
}
/* The escape hatch: nothing drawn, and an alt-only image 0x0, as before. */
.no-placeholders { --rinch-img-placeholder: none }
.no-placeholders .keep { --rinch-img-placeholder: auto }
```

An image with no `src` is also matched by `img:not([src])`; a failed load has
no state a selector can see.

**How this differs from Chrome.** Measured in Chrome 156 (standards mode):
the bordered box — 1px `#c0c0c0` inside the content box, 1px of padding — is
what Chrome draws for a sized image with **no** alt text, and only when the
box is at least 3px on each axis with no `src`, 18px after a failed load
(from 18px it adds a 16x16 broken-image icon in the corner, which rinch does
not draw). A sized image **with** alt text is shown as its alt text inline,
its size ignored, after the 16px icon, whether its `src` failed or is
missing; rinch keeps the size and draws the alt text in the box (Joe's rule,
and what Chrome does in quirks mode), so held-back pictures keep their place.
An image with alt text and one dimension is laid out as Chrome lays it out
(`width: 100%; height: auto` with no ratio is the column's width by the
text's lines). An unsized image with alt text is, in Chrome, an inline-block
of the icon and the text; in rinch of the text alone. A sized image too small
for its box that has alt text (a failed 16x16 `alt="Facebook"` icon) shows
the alt text inline in both, its size ignored. A failed image with no alt and
one or no dimension is, in Chrome, a 16x16 box holding the broken-image icon;
rinch draws nothing there and keeps its declared size (0x0 with none).
Chrome copies no alt text (`getSelection().toString()` of
`a<img alt="Hello">b` is `"ab"`), and neither does rinch. **On the web**
(rinch-web) the browser draws
its own `<img>`, so there you get the browser's fallback and these properties
do nothing.

### `image-rendering`

`image-rendering: pixelated` and `crisp-edges` (#2098) sample an image
nearest-neighbour, so a 16x16 pixel-art icon drawn at 48px keeps hard
pixels. They apply to an `<img>`, a `background-image: url(...)` and a
`mask-image: url(...)`. As in Chrome, both keywords are nearest when scaling
up and when scaling down. The property is inherited. `auto` is smooth
(bilinear) on the GPU renderer. The software renderer has always drawn a
scaled `<img>` nearest, where Chrome's `auto` is smooth, so there the
keywords change only repeating backgrounds and masks at a fractional
position, and minified pictures (below). `smooth` is not parsed, so the declaration is dropped and the
inherited value stays. A `RenderSurface`, `<video>` or `<canvas>` frame does
not read the property.

**Pictures drawn below their natural size** (#2456, #2534). Under `auto`, a
decoded picture (an `<img>` or a `background-image: url(...)`, repeating
tiles included) is drawn the way Chrome draws it, on both renderers: at
about half its size or less (on both axes), from a mipmap level of it (level
`L` is the picture's size divided by `2^L`, rounded up, picked as the
smallest level still at least the drawn size on both axes, and filled as
Chrome fills it), then smoothly the rest of the way; at less than full size
on some axis otherwise, smoothly from the picture itself. So 1px lines and the thin strokes of small
text in it survive, and the result matches Chrome's pixels (exactly at 1/2
and 1/8 of an even size; within about 1.4 levels per channel on average
elsewhere, odd sizes included). A
level is made the first time the picture is drawn at it and kept while it is
drawn (up to 64 MB of levels per window thread, let go after 60 frames
unused); a picture zoomed by an animation makes a new one only every octave.
`pixelated` and `crisp-edges` are never filtered this way. A frame makes
levels from at most 16M source pixels; a picture past that is drawn from its
full pixels for that frame and filtered on the next. A picture larger than that
on its own (a 4200x4200 photo is 17.6M) has its level made over a few frames,
a band of rows each, drawn unfiltered until the level is done; then the window
goes idle as usual.

`background-position` is animatable, by `transition` and `@keyframes` alike;
it moves paint only, never layout. This is what makes the striped `Progress`
bar's stripes move and the `Skeleton` pulse.

A single tile that covers the whole box (`background-size: cover`, or a
gradient at `auto`) is one draw. Any other layer that repeats (on either axis)
is painted as **one repeating pattern**, whatever its tile count. An image
tile is the image itself, scaled to the tile and sampled smoothly once per
device pixel, as Chrome samples it (well below its size, a filtered smaller
copy, as Chrome uses a mipmap). A gradient tile is rasterised once, at the size
it covers on the device, and cached until the gradient or tile size changes; a
tile that is a fractional number of device pixels (10px at 125% scaling) is
rasterised at the next whole size and sampled back down smoothly. An axis that
does not repeat clips the pattern to its one-tile strip, and its edge pixels
take the image's edge, not the far side's. A build
without the software painter (`embed`) has nothing to rasterise a tile with:
it draws a repeating `url()` image as one pattern from the image itself, and a
repeating gradient as one fill per visible tile. No layer is ever dropped for having too
many tiles.

Tiles start from the element's box rounded to a whole pixel, as in Chrome, so
a box at a fractional layout position (`left: 10.5px`) still draws crisp tiles.
A tile on an axis that repeats keeps a fractional
`background-position` (or a fractional translate) and blends the pixel on its
edge, as Chrome does, so a `background-position` animation — the striped
`Progress` bar — moves smoothly (#1151). A full window of such a background
costs about 2.7x more to paint at a fractional position than at a whole one.
This holds on the GPU renderer as well (#1755): it used to move such a
pattern in whole-device-pixel steps, because its image sampler did not blend
across a tile boundary (#1661). On an axis that does not
repeat, a fractional `background-position` is rounded to a device pixel, as
Chrome rounds it; rinch also rounds a fractional translate there, where Chrome
blends. A build without the software painter rounds every tile of a repeating
gradient to a device pixel; a tile over 4 million device pixels rounds every
tile too.

An image used to be stretched over the element's border box whatever its
`background-size`; one that relied on that now tiles at its natural size.
`background-size: 100% 100%` keeps the stretch on a box without a border; on a
bordered box add `background-origin: border-box`, since `100%` is otherwise the
padding box, and the tile repeats under the border.

### Network Images (optional)

Enable with: `features = ["image-network"]`

This adds HTTP(S) URL support using `ureq`. Non-URL paths fall back to local file loading.

```rust
// With image-network feature enabled, URLs work in src:
Image { src: "https://example.com/photo.jpg", width: "200", height: "150" }
Avatar { src: "https://example.com/avatar.png", size: "lg" }
```

### How It Works

1. When an `<img>` element or `background-image: url(...)` is encountered, the source is checked against an in-memory cache
2. If not cached, loading is dispatched to a background thread via the `ImageLoader` trait. A `rinch-image:` source the built-in `register_rgba` registry answers is the exception: its pixels are in memory already, so it is taken in at once, in the frame that names it (an unknown name fails in that frame too)
3. The image bytes are decoded (using the `image` crate) into RGBA8 pixel data, or, for decoded pixels a loader or `register_rgba` handed over, only checked
4. On the next layout pass, decoded images are picked up from a pending queue and inserted into the cache
5. The element is marked dirty for re-layout/re-paint with the image's intrinsic dimensions

**One decode per process.** A picture shown in several windows is loaded and
decoded once, and every window holds that one copy (and the one half-size
copy made when it is drawn small): two windows showing a 3840x2160 wallpaper
hold 39.6 MiB for it, not 79.1 MiB. Windows share a picture when the same
loader answers the same source: an app scheme loader (process-wide), the
built-in file or network loader, or one loader the app set on several
documents. A loader an app sets on one document alone is asked for that
document alone. Windows asking at the same time share one load. The picture
is freed when the last window showing it closes, and a window opened while
another still shows it gets it at once, without asking the loader (call
`reload_image` when the bytes behind a source have changed).

### Custom Image Loader

An app that keeps pictures somewhere only it can reach (embedded assets, a blob
store, an authenticated download) answers for a URL scheme of its own. Sources
with that scheme go to the app's loader; every other source still goes to
rinch's default loader (files, plus HTTP(S) with `image-network`), so there is
nothing to forward:

```rust
use rinch::image::ImageLoadResult;
use rinch::prelude::*;

fn main() {
    App::new(app)
        .image_scheme("asset", |src: &str| match load_from_assets(src) {
            Ok(bytes) => ImageLoadResult::Loaded(bytes),
            Err(e) => ImageLoadResult::Failed(e.to_string()),
        })
        .run();
}

// Anywhere in the UI:
// img { src: "asset:logo.png" }
```

`rinch::image::register_image_scheme(scheme, loader)` is the same registration
without the builder. A loader is any `Fn(&str) -> ImageLoadResult + Send + Sync`
closure, or a type that implements `rinch::image::ImageLoader`.

What a loader must know:

- **It returns encoded bytes or decoded pixels.** `ImageLoadResult::Loaded`
  carries the contents of a PNG, JPEG, GIF, WebP or SVG file, which rinch
  decodes. `ImageLoadResult::Decoded { width, height, rgba }` carries pixels
  rinch only checks (see "Pictures the app holds as pixels" below).
- **On native targets it runs on a background thread** spawned for that one
  load, never the UI thread, so it may block on I/O. Several loads run at once.
  On wasm32 (an embedded `RinchContext` in the browser, a WASI guest) there are
  no threads: it runs on the UI thread, inside the frame that asks, and must
  answer from memory (see "Images in a context" in the game-engine guide).
- **Its answer is cached by source string, a failure included.** A source that
  failed is not asked for again on its own.
- **On desktop, a panic is a failed load.** It is caught on the load's thread and
  cached as a failure carrying the panic's message, so `reload_image` can ask
  again. In the browser (and on a native build with `panic = "abort"`) a
  panicking loader or resolver aborts the whole app.
- **The `data` scheme cannot be registered** (`register_image_scheme("data", ..)`
  panics). No app loader is ever offered a `data:` URL; an `<img>`'s and a
  `background-image`'s are decoded in place.
- **A registration lasts until `unregister_image_scheme`.** It is process-wide
  and is not undone when the component that made it unmounts.

#### A picture that arrives later

When the bytes may not exist yet (still downloading, not synced), answer
`ImageLoadResult::Failed` straight away rather than parking the thread, and tell
rinch when they are there:

```rust
// From any thread, once "asset:photo-42" can be loaded:
rinch::image::reload_image("asset:photo-42");
```

Every `<img>` and `background-image` naming that source, in every window, loads
it again and takes its size. The same call refreshes a picture whose bytes
changed under an unchanged source: the old picture stays on screen until the new
one has decoded. A window that never named the source is not woken by the call.

- **Pass the source exactly as the element spelled it.** The source string is
  the key, scheme case included: `reload_image("asset:photo-42")` does not reach
  an element whose `src` is `Asset:photo-42`.
- **Reloads of one source are ordered and coalesced.** A reload asked for while
  a load of that source is still out (its first load or an earlier reload)
  starts nothing at once: the answer on its way is dropped when it lands and the
  source is loaded once more. So the picture shown is one the loader was asked
  for after the last `reload_image`, and ten reloads during one slow load cost
  one more load, not ten.
- **One load serves every window.** The windows a reload reaches share one new
  load. `RinchDocument::reload_image`, for a host that holds one document,
  reloads that document alone: the others keep the picture they show until a
  `reload_image` reaches them.
- **Do not call `reload_image(src)` from inside the loader's answer for `src`**:
  each call discards the answer being given and asks again, and keeps a reload
  of `src` queued every frame, so the app never goes idle.


#### Pictures the app holds as pixels

Some pictures never were files: a desktop notification's raw image, a frame
the app rendered, a thumbnail it made in memory. Re-encoding them to PNG for
rinch to decode straight back is wasted work. Hand over the pixels instead,
by name:

```rust
use std::sync::Arc;

// Straight-alpha RGBA8, width * height * 4 bytes, rows top to bottom.
let pixels: Arc<[u8]> = make_pixels();
let src = rinch::image::register_rgba("notification-17", width, height, pixels.clone())?;
// src == "rinch-image:notification-17"

// Anywhere in the UI:
// img { src: "rinch-image:notification-17", alt: "" }

// The picture changed: every element showing it repaints.
rinch::image::register_rgba("notification-17", w2, h2, new_pixels)?;

// Another registered picture, under another name: the frame that applies
// this draws it, with no frame of nothing in between.
// img { src: "rinch-image:notification-18", alt: "" }

// The notification closed: elements still naming it draw no picture.
rinch::image::unregister_rgba("notification-17");
```

or from a loader of your own scheme, as `ImageLoadResult::Decoded { width,
height, rgba }`.

- **Straight alpha, RGBA8, no row padding.** `rgba` is exactly
  `width * height * 4` bytes, colour not multiplied by alpha (the form PNG and
  the freedesktop notification `image-data` hint use; convert a
  premultiplied or BGRA or padded-row buffer first). Premultiplied input is
  not accepted.
- **Shown in the frame that names it.** An element given a registered name
  (a new `src`, a new `background-image`) draws the picture in the frame that
  applies the change: the registry answers at once, not from a load thread.
  An unknown or unregistered name fails in that frame too. An app that
  registers its own loader for the `rinch-image` scheme takes this over, and
  its loader is asked on the load thread like any other.
- **It is checked, never decoded.** A buffer of the wrong length, a width or
  height of 0, or a picture past `rinch::image::MAX_DECODED_IMAGE_BYTES`
  (512 MiB, the same allocation limit rinch's decoder gives an encoded file)
  is refused: `register_rgba` returns the reason (`DecodedImageError`) and
  changes nothing, and a loader's answer becomes a failed load (the
  broken-image state), never a panic. `rinch::image::check_rgba` is the same
  check, for an app that wants it first.
- **It is not copied.** The cache, the software painter (when every pixel is
  opaque; otherwise it makes one premultiplied copy, once) and the GPU
  painter read the `Arc` you passed. Keep your own clone of it at the cost of
  a reference count.
- **From then on it is any picture**: an `<img>`'s natural size, `object-fit`,
  `image-rendering`, `background-image: url(rinch-image:…)`, minification,
  both painters, **with the same size limit on the GPU**: the GPU painter does
  not draw a picture with a side over 8192 px (its image atlas is 8192x8192),
  and says nothing, exactly as for an encoded file that size. The software
  painter draws it. Bound what you hand over well below that (a notification
  image bounded to 4096 px a side is a sensible size).
- **Registering a name again** replaces the picture: every element showing it,
  in every window, repaints (and is laid out again if its size changed), with
  the old picture kept until the new one is in. A name registered after an
  element asked for it is shown too.
- **Unregistering a name** takes the picture away from every element still
  showing it: an `<img>` shows the broken-image state, as one whose source
  failed (its `alt` text, or nothing), and a background draws nothing. This is
  unlike `reload_image` with a failing loader, which keeps the old picture.
- **Any thread.** Both calls are safe from a background thread (a D-Bus
  handler, a decoder's worker); the repaint is queued for each window's next
  frame, which they wake.
- **Untrusted pictures.** The 512 MiB limit is rinch's ceiling, not a safe
  bound for input from strangers. An app showing untrusted pictures, encoded
  or raw, should bound them itself (dimensions, bytes, format), decode them
  with its own limits, and hand over the pixels this way.
- An app's own loader for the `rinch-image` scheme takes the scheme over, and
  the registry is then not asked.

#### In the browser

The same two calls work in a `rinch-web` build. The browser loads an `<img>`
itself, so there the loader's bytes are wrapped in an object URL (`blob:…`) and
that is what the element shows. The loader is called on the browser's one
thread, so it must answer from what it already holds (`Failed` for "not yet",
then `reload_image`).

The app is never asked from inside a DOM write, so a resolver or loader may
write signals. An element given a source with no remembered answer shows nothing
until a microtask asks for it, once per source, before the next paint; sources
given while a root mounts are asked for before `mount_into` returns.
`reload_image` asks at once.

An app that already has a URL to show (it made the object URL itself), or whose
state lives in `Rc`s a `Send + Sync` loader cannot capture, uses the
browser-only form instead:

```rust
rinch_web::register_image_url_scheme("asset", move |src| {
    store.borrow().object_url(src)   // Some(url), or None for "not yet"
});
// later:
rinch::image::reload_image("asset:photo-42");
```

Either way the element keeps the source it was given: it is in
`data-rinch-src`, `NodeHandle::get_attribute("src")` answers it, and an editor
document's `image` node never holds the resolved URL. While the answer is "not
yet" the element has no `src` at all. A reload reaches every `<img>` rinch
was given that source, including one in a branch that is hidden right now, and
never an `<img>` the page made outside rinch. It asks nothing for a source no
element was given, and forgets its answer. To make rinch forget a URL you
revoked, call `reload_image` and answer `None`: pictures already shown stay, and
an element given the source later shows nothing until the next reload. "Not yet"
is remembered per source in the browser as on desktop: only `reload_image` asks
again, never an element that is given the source again. `<img>` markup set with
`set_inner_html` is resolved like any other. Only `<img src>` is resolved in the
browser; a `background-image: url(…)` there is the browser's own.

Decoded pixels (`register_rgba`, `ImageLoadResult::Decoded`) work in the
browser too: they are drawn into a canvas once and the `<img>` is given the
canvas's PNG `data:` URL. A canvas stores premultiplied pixels, so a colour
under a very small alpha can come back rounded there, and a picture larger than
the browser allows a canvas is shown as "not yet". Registering a name again
re-points every element at once; unregistering it removes the elements' `src`
(their `alt` text, or nothing), so no stale picture stays.

---

## Files From the User (Web and Desktop)

A file the user picks or drops reaches the app as a `LocalFile` (in the
prelude, and `rinch::file`) on the desktop and in the browser alike: its
`name()`, its `size()`, its `path()` where the platform has one (the desktop;
`None` in a browser), and its bytes **when you ask** — nothing is read before
that, so a large drop costs nothing until it is wanted and `size()` can decide
whether it is.

```rust
use rinch::prelude::*;
use rinch::file::FilePicker;

#[component]
fn importer() -> NodeHandle {
    let status = Signal::new(String::from("pick or drop a model"));
    let load = move |files: Vec<LocalFile>| {
        for file in files {
            spawn_local(async move {
                match file.bytes().await {
                    Ok(bytes) => status.set(format!("{}: {} bytes", file.name(), bytes.len())),
                    Err(e) => status.set(format!("{}: {e}", file.name())),
                }
            });
        }
    };
    rsx! {
        div {
            button {
                onclick: move || FilePicker::new().add_filter("glTF", &["glb", "gltf"]).open(load),
                "Import…"
            }
            div { ondropfiles: load, style: "height: 120px", "drop a model here" }
            p { {|| status.get()} }
        }
    }
}
```

- **Reading.** `file.bytes()` is a future: await it inside `spawn_local` (see
  [Async Tasks](./async.md)). `file.read(|result| …)` is the callback form; it
  runs later on the UI thread, and is dropped unrun if the component that
  called it unmounts first. Either reads the whole file, again on each call:
  on the desktop on a worker thread, in a browser with a `FileReader`. There is
  no partial or streamed read: check `size()` first for files that may be
  large. On the desktop only a regular file is read; a folder, a FIFO or a
  device dropped from the OS arrives as a file whose read fails (a folder does
  in a browser too).
- **The picker.** `FilePicker::new()` (or `rinch::file::pick_files()`), with
  `add_filter(name, &["ext", …])`, `multiple(true)` and, on the desktop,
  `set_title`. `pick()` returns a future of the files (empty when the user
  cancelled); `open(callback)` hands them to a callback. Both open the picker
  **when called**, so call them in the event handler: a browser shows a picker
  only during a user gesture. Outside one the browser refuses, and the picker
  logs a warning and resolves to no files. A touch or pen tap runs its click
  on contact (unless the element can scroll), before the browser counts the
  tap as a gesture; the picker then opens when the finger lifts, and gives no
  files if the touch turns into a pan. Where a browser has no `cancel` event
  for a file input, a picker with no answer a second after the window's focus
  comes back is taken as cancelled. On the web a filter's extensions
  are the input's `accept`, which the browser treats as a suggestion. On the
  desktop the picker is the native dialog of [File Dialogs](#file-dialogs) and
  needs the `file-dialogs` feature; it is modal, so `pick()` returns once it
  closes. The browser build needs no feature.
- **Drops.** `ondropfiles: move |files: Vec<LocalFile>| …` on any element
  takes files dropped from the OS onto it or a descendant, on the desktop and
  on the web, and reads the drop point through `get_click_context()` as
  `onfiledrop` does: `mouse_x`/`mouse_y` where the files were dropped (the
  frame clicks use: logical window px on the desktop, client px in a browser),
  `element_*` the box of the element carrying `ondropfiles`, the viewport size
  and the modifiers held. The nearest element with a drop handler takes the drop;
  one carrying both `onfiledrop` (paths, desktop only) and `ondropfiles` gets
  both. Nested zones resolve the same way on both platforms: a drop on an
  inner `onfiledrop`-only zone inside an `ondropfiles` one goes to the inner
  zone, which in a browser takes nothing (the drop is still kept from the
  browser). An editor that takes pictures (`EditorHandle::on_image_input`) keeps
  the image files of a drop on it and hands the rest to the handler around it.
  In a browser, dropping a file anywhere without a handler still opens it in
  place of the page, as it does on any web page. rinch listens for drops on
  the document, after the event has passed through the page: a host page
  script of its own that cancels the `drop` (`preventDefault`) before it gets
  there silences `ondropfiles`.

`onfiledrop` (`Vec<PathBuf>`) and the `rinch::dialogs` file pickers are unchanged and remain
the desktop-only, path-based forms.

**Testing a drop.** With the `debug` feature, the rinch MCP server's
`drop_files { x, y, paths }` drops files on a running desktop app at a point
(logical px, as `click`) through the same path as an OS drop: the drag enters
there, then the drop reaches an editor that takes pictures and the
`onfiledrop` / `ondropfiles` handler under the point. The paths are passed
through unchecked, so a missing file fails in the app's own read. There is no
web equivalent yet. The app's debug server takes one client at a time: a script that talks to it
directly needs the MCP server disconnected first.

---

## File Dialogs

Enable with: `features = ["file-dialogs"]` (natively; a browser build has the
message dialogs without it)

Native file dialogs for opening, saving, and folder selection, and message
dialogs (alert, confirm). In the browser only the message dialogs exist — see
[Message Dialogs](#message-dialogs).

### Opening Files

```rust
use rinch::dialogs::{open_file, MessageLevel};

// Open a single file with filters
if let Some(path) = open_file()
    .set_title("Select an image")
    .add_filter("Images", &["png", "jpg", "gif"])
    .add_filter("All Files", &["*"])
    .set_directory("/home/user/pictures")
    .pick_file()
{
    println!("Selected: {}", path.display());
}

// Open multiple files
if let Some(paths) = open_file()
    .add_filter("Rust Files", &["rs"])
    .pick_files()
{
    for path in paths {
        println!("Selected: {}", path.display());
    }
}
```

### Saving Files

```rust
use rinch::dialogs::save_file;

if let Some(path) = save_file()
    .set_title("Save document")
    .set_file_name("untitled.txt")
    .add_filter("Text Files", &["txt"])
    .set_directory("/home/user/documents")
    .save()
{
    println!("Save to: {}", path.display());
}
```

### Picking Folders

```rust
use rinch::dialogs::pick_folder;

if let Some(path) = pick_folder()
    .set_title("Select output folder")
    .pick()
{
    println!("Folder: {}", path.display());
}
```

### Message Dialogs

```rust
use rinch::dialogs::{message, MessageLevel};

// Simple alert
message("File saved successfully!")
    .set_title("Success")
    .show();

// Warning with OK/Cancel
let proceed = message("This will overwrite existing files.")
    .set_title("Warning")
    .set_level(MessageLevel::Warning)
    .confirm();

if proceed {
    // User clicked OK
}

// Yes/No question
let delete = message("Are you sure you want to delete this file?")
    .set_title("Confirm Delete")
    .set_level(MessageLevel::Warning)
    .ask();

if delete {
    // User clicked Yes
}
```

**In the browser** (`wasm32`), `message(..)` with `show`, `confirm` and `ask` is
the page's own `window.alert` / `window.confirm`: the same call, synchronous as
natively. In a browser build `rinch::dialogs` needs no feature: a web build
without `file-dialogs` has it, and `file-dialogs` links no crate there. The browser draws no title
(it becomes the message's first line) and no level icon, and `ask`'s buttons
read OK / Cancel (OK is yes). `confirm` answers `false` when the browser
suppresses the dialog. The file and folder pickers are native-only: a browser
file picker is asynchronous and returns no path — use an `<input type="file">`.

---

## Clipboard

Enable with: `features = ["clipboard"]`

Without it, the built-in text fields and the rich-text editor have no
clipboard either: Cut and Copy (the chords and the context menu alike) do
nothing — a Cut does not delete text it could not copy — and Paste inserts
nothing. (`android` turns the feature on.)

Cross-platform clipboard support for text and images.

### Text Operations

```rust
use rinch::clipboard::{copy_text, paste_text, has_text, clear};

// Copy text to clipboard
copy_text("Hello, clipboard!").unwrap();

// Check if clipboard has text
if has_text() {
    // Paste text from clipboard
    match paste_text() {
        Ok(text) => println!("Clipboard: {}", text),
        Err(e) => println!("Failed to paste: {}", e),
    }
}

// Clear the clipboard
clear().unwrap();
```

### Image Operations

```rust
use rinch::clipboard::{copy_image, paste_image, has_image, ImageData};

// Copy an image (RGBA format)
let image = ImageData::new(
    100,  // width
    100,  // height
    vec![255; 100 * 100 * 4],  // RGBA data (white image)
);
copy_image(image).unwrap();

// Check and paste image
if has_image() {
    let image = paste_image().unwrap();
    println!("Image size: {}x{}", image.width, image.height);
    println!("Bytes: {}", image.bytes.len());
}
```

### Using with Hooks

```rust
use rinch::prelude::*;
use rinch::clipboard::{copy_text, paste_text};

#[component]
fn app() -> NodeHandle {
    let text = Signal::new(String::new());
    let text_copy = text.clone();
    let text_paste = text.clone();

    rsx! {
        div {
            input {
                value: {|| text.get()},
                oninput: move |e| text.set(e.value())
            }
            button {
                onclick: move || {
                    let _ = copy_text(text_copy.get());
                },
                "Copy"
            }
            button {
                onclick: move || {
                    if let Ok(pasted) = paste_text() {
                        text_paste.set(pasted);
                    }
                },
                "Paste"
            }
        }
    }
}
```

### Reading without freezing the UI

A clipboard *read* is a request to whichever application owns the clipboard. On
X11 that application can be hung, and the read waits up to **four seconds** for
it; the browser cannot be read synchronously at all. So `paste_text()` called
straight from an event handler — as in the example above — can block the UI
thread: no repaint, no input, and on some window managers a "not responding"
badge (issue #149).

Every read therefore comes in three shapes, on every platform:

| Function | Blocks the caller? | Use for |
|---|---|---|
| `paste_text()` | yes, indefinitely | scripts, background threads, existing code |
| `paste_text_timeout(Duration)` | yes, bounded — `Err(TimedOut)` after that | an interactive path that can accept a bounded hiccup |
| `paste_text_async(callback)` | no | an interactive path that must stay responsive |

`paste_html` / `paste_image` have the same three. `paste_rich` resolves
`text/html` → bitmap → `text/plain` in **one** read and answers with a
`RichPaste`, so a rich-paste consumer never stacks three worst-case waits.
`paste_rich_with_text` (and its `_timeout` / `_async`) answers the same plus the
`text/plain` offered beside an html answer, `(RichPaste, Option<String>)`, still in
one read — it is what the built-in editor's Ctrl+V uses, so the editor's paste hook
(`Plugin::handle_paste`) can tell a pasted URL by its text even when the copy also
carried html.

On native, all of them are served by a single clipboard worker thread that owns
the system clipboard. That is what makes the timeout worth having: giving up does
not cancel the request, it only stops waiting for it, so an abandoned read
finishes on the worker instead of wedging every later caller behind a lock.

**Which thread does the callback run on?** Not necessarily the UI thread — on
native it is the clipboard worker, which is why the callback must be `Send`.
rinch UI state is thread-local, so hop back before touching it:

```rust
use rinch::clipboard::{paste_text_async, ClipboardResult};
use rinch::prelude::*;

let text = Signal::new(String::new());
button {
    onclick: move || {
        // `Signal::send` marshals to the UI thread from anywhere.
        paste_text_async(move |result| {
            if let Ok(pasted) = result {
                text.send(pasted);
            }
        });
    },
    "Paste"
}
```

For a `!Send` continuation (an `EditorHandle`, an `Rc`), park it on the UI thread
and send only its id across:

```rust
let id = rinch_core::park_main_callback::<ClipboardResult<String>>(move |result| {
    // Runs on the UI thread; free to touch the DOM and any Rc-based handle.
});
paste_text_async(move |result| {
    rinch_core::run_on_main_thread(move || rinch_core::resume_main_callback(id, result));
});
```

The built-in fields already work this way. Ctrl+V — and the context menu's or
Android toolbar's Paste — in the rich-text editor *and* in a plain `<input>` or
`<textarea>` returns at once, reads on the worker, and inserts when the read
answers (issue #149 for the editor, #328 for the plain fields). A plain field
takes the paste at its **current** selection if it still holds the keyboard
from the same focus gesture; if focus moved on while the read was in flight,
the paste is dropped rather than aimed at a field the user did not paste into.
A read-only field starts no read at all.

A single-line field (an `<input>`, anything but a `<textarea>`) folds the line
breaks of pasted or IME-committed text the way Chrome does: line breaks at the
end are dropped and every other one becomes a space, so `"a\nb"` pastes as
`"a b"`. A `<textarea>` keeps them. A value the *app* writes (a `value`
attribute, `value_fn`) follows HTML's value sanitization instead, on desktop as
in a browser: an `<input>` of a text type strips its line breaks, so `"a\nb"`
shows and edits as `"ab"` (`url` and `email` also trim surrounding
whitespace), while the attribute keeps what was written.

### Knowing whether a copy worked

`copy_text` answers `Ok` on the web as soon as rinch has stored the text in its
in-app buffer — before the browser has answered `navigator.clipboard.writeText`.
A refused permission, a write made with no user gesture or from an unfocused
page, or an insecure origin all still answer `Ok`. To show "Copied" honestly, use
`copy_text_then` (or `copy_html_then`), which calls back with the platform's own
answer on every target:

```rust
use rinch::clipboard::copy_text_then;

let status = Signal::new(String::new());
rsx! {
    button {
        onclick: move || copy_text_then(link.get(), move |result| {
            // `send`, not `set`: on native the callback runs on the clipboard
            // worker thread (on the web, `send` applies at once).
            status.send(if result.is_ok() { "Copied".into() } else { "Could not copy".into() });
        }),
        "Copy link"
    }
}
```

- **Web:** the callback runs when the browser's promise settles, on the UI
  thread: `Ok(())` when it accepted the write, `Err(ClipboardError::AccessFailed(..))`
  carrying the browser's `DOMException` (`NotAllowedError: …`) when it refused,
  `Err(ClipboardError::NotAvailable)` (before the call returns) when the page has
  no `navigator.clipboard`. Browsers accept a write made while handling a click
  or key press, so call it from the handler itself, not from a later timer.
  `copy_html_then` writes a `ClipboardItem` holding `text/html` and the
  plain-text fallback (as `copy_html` now does too); where that write cannot be
  made or the browser refuses it, the fallback is written alone through
  `writeText` and its answer is the one reported.
- **Native:** the write is queued on the clipboard worker and the callback runs
  **on that worker thread** with its result, like `paste_text_async`'s, so a
  signal write from it goes through `Signal::send`, as in the example.
- **Android:** the callback runs before the call returns.

### Web: reaching content copied outside the app

The browser has no synchronous system-clipboard read, so on `wasm32`
`paste_text()` answers from an internal buffer. rinch-web fills that buffer from
the document's `paste` ClipboardEvent — the only synchronous channel to content
copied in another app or tab — so a web app can paste from outside itself
(issue #150). `paste_text_async` additionally tries
`navigator.clipboard.readText()`, which needs a secure context and usually a user
gesture, and falls back to the buffer.

Because the browser's `paste` event arrives *after* the keydown that caused it,
app paste logic on the web should hang off the paste rather than off Ctrl+V:

```rust
use rinch_core::{set_paste_interceptor, PasteEventData};

set_paste_interceptor(|data: &PasteEventData| {
    // The clipboard buffers are already filled when this runs, so
    // `rinch::clipboard::paste_text()` works here too.
    if let Some(text) = &data.text {
        insert_into_my_editor(text);
        return true; // handled: the browser should not also insert it
    }
    false
});
```

The interceptor is a single slot **per document** — plus a thread-global
fallback for registrations made outside any dispatch, which is where every
registration on rinch-web lands (like `set_keyboard_interceptor`; issues #340,
#478) — and is dispatched by rinch-web; desktop reads the clipboard directly
when Ctrl+V arrives and has no OS paste event to hang it off.

Registering it from inside a render ties it to that component: unmounting
releases the slot, so an interceptor that captured a `Signal` can never outlive
it (issue #183). Registering from `main` or startup code has no owner and keeps
app lifetime. See
[Lifetimes](./hooks.md#global-callback-registries-are-released-too).

### Android: the text-selection toolbar and paste

A long press on an `<input>` or `<textarea>` shows Android's own floating
text-selection toolbar — Cut / Copy / Paste / Select all, the platform's
strings, the platform's look — floating over the selected word, or over the
caret when nothing is selected: the rect the runtime's `TextEditState.anchor`
reports (issue #813). It is an
`ActionMode.TYPE_FLOATING` started by `RinchActivity` on the window's decor
view, and it is what the desktop's built-in DOM context menu becomes on
Android: the shell sets `TextContextMenuPresentation::Shell`, so the runtime
prepares the caret and selection and hands the presentation to the platform.
A `data-oncontextmenu` handler on the field or an ancestor still wins, as on
desktop.

The long press follows the platform convention: it selects the word under the
finger, a press inside an existing selection keeps that selection, and an
empty field (or a press on whitespace) keeps its caret and offers only Paste.
Cut and Copy appear only over a selection, Paste is hidden on a `readonly`
field and while the clipboard is empty (`hasPrimaryClip()`, asked once per
long press, which reads no clip and raises no clipboard-access notice), and
Select all needs content.
The clipboard itself is read only when Paste is tapped. Each item runs
exactly the code its keyboard shortcut runs
(`RinchApp::perform_text_edit`), so a toolbar Paste, a hardware Ctrl+V and an
IME's paste are one path.

**The `android` feature implies `clipboard`.** Paste is not optional on a
phone, and without the feature `handle_paste` has nothing to read: a
hardware Ctrl+V inserts nothing and an IME's paste request is answered "done"
with nothing pasted (the toolbar hides its Paste instead). Every route
measured on an API 34 emulator with one Gboard build lands: the toolbar's
Paste; the clipboard chip Gboard puts on its suggestion strip after a copy;
an item in Gboard's clipboard panel; the Paste key of Gboard's Text Editing
panel; and `Ctrl+V` from a hardware keyboard. That Gboard build delivers its
chip and clipboard-panel pastes as ordinary committed text
(`InputConnection.commitText`), and its Text Editing panel's Paste as
`performContextMenuAction(android.R.id.paste)` — which `BaseInputConnection`
would otherwise drop, since rinch's connection keeps no `Editable`, and which
reaches the same path through `RinchInputConnection`'s override.

The toolbar goes when the user taps or scrolls elsewhere, types, edits from
the keyboard, cuts, copies or pastes — from the toolbar or through the IME's
own editing keys, as over a platform text view — or when the field loses
focus; the activity also finishes it on its own in `onPause` and on
window-focus loss. An item that finishes the toolbar (Cut, Copy, Paste) is
finished on the Java side *before* the item is reported, so a refresh the
shell decides after hearing of the item never re-prepares it; and **a refresh
never starts a toolbar** — it only moves and re-prepares one that is up — so a
refresh already on its way to the UI thread when the item finished the mode
finds nothing to update and does nothing, where it used to start a new
toolbar that nothing could take down. An IME's Cut, Copy or Paste finishes
nothing on the Java side, and the shell asks for the finish itself.
(Gboard's Text Editing panel enables its Cut and Copy only over a selection
the IME can see, and rinch's input connection reports none, so from that
panel only Paste and Select all reach a rinch field — measured.) The
rich-text `Editor` is not part of an Android build today (it is
`windowing`-only), so the toolbar covers `<input>` and `<textarea>`.

---

## System Tray

Enable with: `features = ["system-tray"]`

System tray icon with menu support. Uses the same unified `Menu`/`MenuItem` types as native window menus.

### Basic Tray Icon

```rust
use rinch::tray::TrayIconBuilder;
use rinch::menu::{Menu, MenuItem};

// Create a tray menu using the unified Menu API
let menu = Menu::new()
    .item(MenuItem::new("Show Window").on_click(show_current_window))
    .separator()
    .item(MenuItem::new("Settings"))
    .separator()
    .item(MenuItem::new("Quit").on_click(close_current_window));

// Create the tray icon
let tray = TrayIconBuilder::new()
    .with_tooltip("My Application")
    .with_menu(menu)
    .build()
    .unwrap();
```

**No tray host yet.** On Linux the tray is drawn by the desktop's panel (its
StatusNotifierWatcher). An app started with the session can come up before the
panel; an isolated or minimal session may have none. `build()` still succeeds
then: the item registers as soon as a host appears, and again if the host
restarts. To treat a missing host as an error instead (to keep a window open
rather than hide to a tray nobody can see), ask for it:

```rust
match TrayIconBuilder::new().with_menu(menu).wait_for_host(false).build() {
    Ok(tray) => { /* the icon is up */ }
    Err(TrayError::NoHost(_)) => { /* no panel shows tray icons */ }
    Err(e) => eprintln!("tray: {e}"), // no D-Bus session, a hung host, …
}
```

`TrayError` is `#[non_exhaustive]`: a `match` on it needs a `_` arm.

### Tray Icon with Image

```rust
use rinch::tray::TrayIconBuilder;

// From PNG data (e.g., include_bytes!)
let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_icon_png(include_bytes!("../assets/icon.png"))?
    .build()?;

// From file path
let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_icon_path("assets/icon.png")?
    .build()?;

// From RGBA data (32x32 icon)
let rgba = vec![255u8; 32 * 32 * 4]; // White icon
let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_icon_rgba(rgba, 32, 32)?
    .build()?;
```

### Menu Callbacks

Callbacks are `impl Fn() + 'static` — no `Send`/`Sync` required. They run on the main thread via push-based event delivery (no polling):

```rust
use rinch::tray::TrayIconBuilder;
use rinch::menu::{Menu, MenuItem};
use rinch::prelude::*;

let menu = Menu::new()
    .item(MenuItem::new("Show Window").on_click(show_current_window))
    .separator()
    .item(MenuItem::new("Quit").on_click(close_current_window));

let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_icon_png(include_bytes!("../assets/icon.png"))?
    .with_menu(menu)
    .build()?;
```

### Changing the Tray at Runtime

Every part of a built tray can change while it runs (issue #1520). Each setter
takes `&self`, so a tray kept in an `Rc` or a `thread_local!` can be changed
from a menu callback, a timer or an effect:

```rust
use rinch::tray::{TrayIconBuilder, TrayIconImage};

// Decode each image once; swapping is then cheap.
let idle = TrayIconImage::from_png(include_bytes!("idle.png"))?;
let busy = TrayIconImage::from_png(include_bytes!("busy.png"))?;

let tray = TrayIconBuilder::new().with_icon(idle.clone()).build()?;

tray.set_icon(&busy)?;                 // also set_icon_png / set_icon_rgba
tray.set_tooltip("Working… 43%")?;
tray.set_menu(Some(new_menu));         // or None to remove the menu
```

None of them blocks the calling thread. On Windows and macOS `tray-icon`
applies a change on the spot. On Linux each change goes to a thread of rinch's
own, which applies it to the ksni service: a tray host that has stopped
answering holds that thread, never yours. Changes made while it is held are
coalesced — the newest icon, tooltip and menu win, and only the latest state of
each check mark is kept — so an app swapping icons several times a second
queues neither work nor memory behind a hung host. (Measured: 200 icon and
tooltip swaps on a KDE tray return in about 1 ms, and the item's tooltip then
reads the last one.)

On Linux the item's `Title`, which Plasma sorts the tray by, is a stable name:
`TrayIconBuilder::with_title(..)`, else the tray's id. `set_tooltip` changes
only the `ToolTip`, so the icon does not move when the text does. Windows and
macOS have no title property and ignore `with_title`.

`set_menu` releases the previous menu's callbacks at once, like a rebuilt menu
bar; it is safe to call from one of the old menu's own callbacks ("Start"
replacing itself with "Stop"). To move a check mark, write the item's signal
instead: checkbox and radio items ([Menus](menus.md#checkbox-and-radio-items))
follow their signals in a tray menu too.

### Item Id

On Linux the tray item reports an id to the panel (the StatusNotifierItem
`Id`), which panels use to remember per-item settings such as whether the icon
is shown and where. It is the app's own (issue #1529; it was `"rinch-app"` for
every rinch app): the executable's file name, or the `WindowProps::app_id` the
app declared if the tray is built after `App::run` has started. Set it
explicitly with `with_id`, which an app that declares an `app_id` and builds its
tray in `main` should do with the same string. Rebuilding the tray after
`App::run` has started can change its id (the declared `app_id` is seen from
then on), so pass `with_id` to keep it fixed. Upgrading to this changes an
existing app's id, so a panel forgets any visibility or position the user had
set for its icon, once.

```rust
let tray = TrayIconBuilder::new()
    .with_id("com.example.notes")
    .with_icon(icon)
    .build()?;
```

Windows and macOS have no such id (they identify the icon by the app), so
`with_id` is not used there.

### Left Click (Activate)

By default a primary click on the icon shows the window. `on_activate` replaces
that:

```rust
let listening = Signal::new(false);
let tray = TrayIconBuilder::new()
    .with_icon(idle)
    .on_activate(move || listening.update(|l| *l = !*l))
    .build()?;
```

On Linux this is the StatusNotifierItem `Activate` request (which click sends
it, and whether a panel opens the menu instead, is the panel's choice); on
Windows and macOS it is a left-button release on the icon. The callback runs on
the main thread and follows a menu callback's lifetime rule. Pass `|| {}` for a
click that does nothing.

### Tray-Only Apps

An app whose whole UI is the tray starts with `WindowProps { visible: false, .. }`
and shows its one window ("Settings…") with `show_current_window()`; on macOS
it also sets `App::activation_policy(ActivationPolicy::Accessory)` so it has no
Dock icon. See
[Windows](windows.md#starting-with-no-window-tray-only-apps) and
`examples/tray-only`, which swaps icons on a timer, changes its tooltip, has a
checkbox and a radio group, and opens a settings window from the menu.

### Nested Submenus

```rust
use rinch::menu::{Menu, MenuItem};

let submenu = Menu::new()
    .item(MenuItem::new("Option 1"))
    .item(MenuItem::new("Option 2"))
    .item(MenuItem::new("Option 3"));

let menu = Menu::new()
    .item(MenuItem::new("Main Action"))
    .submenu("More Options", submenu)
    .separator()
    .item(MenuItem::new("Quit").on_click(close_current_window));
```

### Lifetime and Threads

On Linux, `build()` waits for the tray host to register the icon, which on a healthy desktop takes a few milliseconds. If the host holds its D-Bus name but never answers (a frozen `kded` or `plasmashell`), `build()` gives up after five seconds and returns `TrayError::CreateFailed` rather than hanging your app's startup; treat that error as "no tray this session". Do not retry in a tight loop: the timed-out registration cannot be cancelled, so each failed attempt under a still-hung host leaves a background thread and a D-Bus connection waiting on it until the host answers or goes away.

The tray lives exactly as long as the `TrayIcon` that `build()` returned. Dropping it removes the icon from the panel and then releases its menu callbacks, so an icon that can still be on screen keeps working items. On Linux the drop shuts ksni's D-Bus service down and waits up to one second for it to close its connection, which on a healthy desktop takes a few milliseconds. If the tray host has stopped answering, the drop returns after that second anyway. The callbacks then stay registered until the service has closed, and are released by the next tray build or drop on the same thread. Keep the handle for as long as you want the tray, for example as a local in `main` that lives until the event loop exits: `let _tray = …` keeps it, while a bare `let _ = …` drops it at once.

A `TrayIcon` is **neither `Send` nor `Sync`**, on every platform. Keep it on the thread that built it. A `static OnceLock<TrayIcon>`, an `Arc<Mutex<TrayIcon>>`, or moving it into `std::thread::spawn` does not compile; a `thread_local!` works. This is deliberate. Menu callbacks capture `Signal`s (which are `!Send`) and always run on the main thread, so they live in a thread-local registry, and dropping the tray releases them from the registry of the thread it is dropped on. Dropped on another thread, it would release nothing. On Linux this has been the case since the tray started releasing its callbacks on drop (issue #183); before that the Linux handle was `Send`.

A disabled item (`MenuItem::new("…").enabled(false)`) is shown greyed out and registers no callback. Its `on_click` never runs.

### Minimize-to-Tray Pattern

Combine system tray with `on_close_requested` to hide instead of quit:

```rust
use rinch::prelude::*;
use rinch::tray::TrayIconBuilder;
use rinch::menu::{Menu, MenuItem};
use std::sync::Arc;

// Set up tray icon
let menu = Menu::new()
    .item(MenuItem::new("Show Window").on_click(show_current_window))
    .separator()
    .item(MenuItem::new("Quit").on_click(close_current_window));

let _tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_icon_png(include_bytes!("../assets/icon.png"))?
    .with_menu(menu)
    .build()?;

// Hide to tray on close instead of quitting
let window_props = WindowProps {
    on_close_requested: Some(Arc::new(|| {
        hide_current_window();
        false // Don't exit
    })),
    ..Default::default()
};
```

## Notifications

Enable with: `features = ["notifications"]` (off by default; it needs no windowing feature).

`rinch::notification` shows an operating-system notification: a freedesktop notification on Linux (KDE, GNOME, dunst, mako), a toast on Windows, a UserNotifications banner on macOS.

```rust
use rinch::notification::{Notification, NotificationError};
use rinch::prelude::*;

let shown = Notification::new("Hotkey unavailable")
    .body("Ctrl+Alt+Space is taken by another app.")
    .icon_png(include_bytes!("icon.png"))        // optional
    .on_click(|| show_current_window())          // optional
    .on_dismiss(|why| tracing::info!("gone: {why:?}"))
    .on_error(|e: &NotificationError| tracing::warn!("{e}"))
    .show();                                     // Result<NotificationHandle, NotificationError>
```

`examples/tray-only`'s "Test notification" item raises one; clicking it opens the settings window.

### Threads and callbacks

`show()` makes no IPC call on the calling thread. The D-Bus calls (Linux) and the WinRT calls (Windows) run on a worker thread started by the first `show()`, and macOS's calls answer through completion handlers. The only work done in place is writing an `icon_png` image to a file, once per distinct image, in a directory only the user can write (`$XDG_RUNTIME_DIR` on Linux, else the user's cache directory; the per-user temporary directory on macOS and Windows). Call `show()` on the main thread: the callbacks are `!Send` and stay there.

`on_click`, `on_dismiss` and `on_error` run on the main thread, each in one batch, like a menu callback. They follow the same lifetime rule: a callback set while a component renders belongs to that component, and does not run once it has unmounted; one set outside any render (from `main`, a timer) keeps app lifetime.

A click ends the notification: `on_dismiss` does not run after `on_click`. `on_dismiss` gets a `DismissReason`: `Expired`, `Dismissed` (by the user), `Closed` (by `NotificationHandle::close`) or `Unknown`.

### Errors

`show()` returns `Err` for what is known at once: `Unsupported` on a platform with no backend (Android; the feature does not build for the web, see below) and on an unbundled macOS executable. What only the platform can answer arrives later through `on_error`, on the main thread:

- `NoService`: no D-Bus session bus (or it went away), or no `org.freedesktop.Notifications` server on it.
- `PermissionDenied`: the user refused the app notifications (macOS).
- `Platform`: anything else (Windows would not show the toast, D-Bus failed).

A notification with no `on_error` logs the error with `tracing::warn!`.

### App identity

The desktop files notifications under the app that sent them. rinch uses the identity the window has: `WindowProps::app_id` if the window sets one, else the executable's file name (without `.exe` on Windows), the same derivation as the Wayland `app_id`. `rinch::notification::set_app_id` overrides it and `set_app_name` sets the name shown as the source (default: the app id).

- **Linux:** the id goes in the `desktop-entry` hint, so the server can take the app's name and icon from `<app id>.desktop` if it is installed. rinch writes no file for notifications. The image is passed as the `image-path` hint. A click needs a server with actions (KDE and GNOME have them). If the notification server restarts (plasmashell, dunst), the notifications it was showing are gone: each gets `on_dismiss(DismissReason::Unknown)`, and a signal from the new server that reuses an old id never reaches them.
- **Windows:** the id is the toast's AppUserModelID (AUMID). Windows shows toasts from an unpackaged app only for an AUMID it knows, either from a Start-menu shortcut that carries it or from a registry key. On the first `show()` rinch writes `HKCU\Software\Classes\AppUserModelId\<AUMID>` with a `DisplayName` (the `set_app_name` name), unless that key already exists. No shortcut is needed or created. Call `set_register_app(false)` if your installer registers the AUMID itself. An AUMID may not contain spaces, so an executable named `My App.exe` needs `set_app_id("com.example.MyApp")`; an explicit id also keeps two unpackaged apps with the same file name apart. A click is reported while the toast is up; once it has timed out into the Action Center (`on_dismiss(Expired)` has run), a click there is not reported. A toast clicked in the Action Center after the app has exited does nothing: relaunching on click needs a COM activator, which rinch does not register.
- **macOS:** the identity is the bundle identifier. The UserNotifications framework works only inside an `.app` bundle; run unbundled (`cargo run`), `show()` returns `Unsupported`. The first `show()` asks the user for permission. macOS does not report a dismissal, so `on_dismiss` runs there only for `NotificationHandle::close`. The image is attached to the notification; the app icon is the bundle's. rinch installs its `UNUserNotificationCenter` delegate on the first `show()`: a click that launches the app is not reported, and an app or crate that sets its own delegate replaces rinch's (one delegate per process).

### Not covered

- The web: the feature brings in tokio's multi-threaded runtime (the workspace's tokio), which does not build for `wasm32`. A web app can use the browser's `Notification` API through `web-sys`.
- Android.
- Buttons other than the click itself, replies, sounds, urgency and timeouts.

---

## Global Hotkeys

Enable with `features = ["global-hotkeys"]` (off by default; it implies `windowing`, not the native menus: see [Menus → Without native menus](menus.md#without-native-menus)). `rinch::hotkey` registers **system-wide** hotkeys: they fire while another application has the keyboard focus, and they report **press and release**, so a hold-to-talk binding works.

```rust
use rinch::hotkey::{self, HotkeySpec, HotkeyState};

let listening = Signal::new(false);
let hk = hotkey::register(
    HotkeySpec::new("Ctrl+Alt+Space").id("listen").description("Start listening"),
    move |ev| listening.set(ev.state == HotkeyState::Pressed),
)?;
hk.rebind("RightCtrl")?; // hold Right Ctrl alone
drop(hk);                // unregisters
```

- **Callbacks run on the main thread**, each in its own `batch`, so they can set signals directly. Register on the main thread; a `Hotkey` is neither `Send` nor `Sync`.
- **Lifetime.** A hotkey registered while a component renders belongs to that component: it is unregistered when the component unmounts and its callback does not run after that. One registered outside any render (in `main`) lives until its `Hotkey` is dropped. Dropping the handle always unregisters. This is the menu-callback rule.
- **Conflicts are errors.** `register` and `rebind` return `HotkeyError::Conflict` when another application holds a plain chord (on the platforms that can tell, see below) or when this app already registered it. A failed `rebind` keeps the old binding. Rebinding while the hotkey is held delivers its `Released` first.
- **Auto-repeat** does not press a hotkey again: a held hotkey is one `Pressed` and one `Released`, on every backend.
- A report the backend made for a binding that has since been rebound is dropped, so a rebind never leaves the hotkey pressed.

### Shortcut strings

A shortcut names **physical keys**, as a menu shortcut does: `"Ctrl+/"` is the key labelled `/` on a US layout, and a shifted character such as `?` is an error that tells you to write `Shift+/`. Every key a menu shortcut accepts works here. On top of those:

- **Sided modifiers:** `LeftCtrl`, `RightCtrl` (also `RCtrl`, `ControlRight`), `RightAlt` (`AltGr`), `LeftShift`, `RightSuper`. A plain `Ctrl` matches either key.
- **Modifier-only bindings:** a shortcut whose last key is a modifier. `"RightCtrl"` fires when Right Ctrl is pressed with no other key held and releases when it comes up. `"Ctrl+Shift"` fires when Shift goes down while Ctrl is held, and releases when either comes up. Pressing Right Ctrl and then C (copy) does fire `"RightCtrl"`: the press is reported the moment Right Ctrl goes down, because a hold-to-talk app cannot wait to find out whether a chord follows. Its `Released` then carries `chorded: true` (another key went down during the hold). Only a key that was *already* held when the modifier went down stops the press. Caps Lock, Num Lock and Scroll Lock never count as another key.

A **toggle** app (press once to start, once to stop) should act on the release and ignore a chorded one, so that Right Ctrl+C copies text instead of toggling:

```rust
hotkey::register("RightCtrl", move |ev| {
    if ev.state == HotkeyState::Released && !ev.chorded {
        listening.update(|l| *l = !*l);
    }
})?;
```

A hold-to-talk app keeps acting on `Pressed` and `Released`, and can drop what it recorded when the release is chorded. `chorded` is always `false` on the Wayland portal and for macOS plain chords, which do not see the other keys.
- `Super` (also `Win`, `Cmd`, `Meta`) is its own modifier, separate from Ctrl. `CmdOrCtrl` is Cmd on macOS and Ctrl elsewhere.
- Any W3C key `code` name: `F13`…`F24`, `Insert`, `Pause`, `PrintScreen`, `Numpad1`.

Matching is exact on modifiers: `Ctrl+Space` does not fire for `Ctrl+Shift+Space`. `Shortcut::parse` checks a string without registering it, and its `Display` is the canonical spelling.

### Letting the user choose: `HotkeyCapture`

`HotkeyCapture` turns the keys pressed in your own window into a shortcut. Feed it every key event while a settings field is waiting, for example from a keyboard interceptor installed for the duration:

```rust
use rinch::core::{set_keyboard_interceptor, clear_keyboard_interceptor};
use rinch::hotkey::{CaptureStep, HotkeyCapture};

let capture = RefCell::new(HotkeyCapture::new());
set_keyboard_interceptor(move |k| {
    match capture.borrow_mut().feed(k) {
        CaptureStep::Pending(held) => preview.set(held),        // "Ctrl+Alt+"
        CaptureStep::Done(sc) => { let _ = hk.rebind(&sc.to_string()); clear_keyboard_interceptor(); }
        CaptureStep::Cancelled => clear_keyboard_interceptor(), // Escape alone
    }
    true
});
```

A non-modifier key completes a chord, captured with unsided modifiers (`Ctrl+Alt+Space`). Modifiers pressed and released with nothing else are captured as a modifier-only binding, with sides (`RightCtrl`). The capture reads only your app's own key events, so it needs no permission. On the desktop it sees only the keys rinch's desktop shell names (letters, digits, F1–F12, the editing and arrow keys, `=`, `-`, Ctrl/Shift/Alt). Other keys, Super included, are ignored there; write those shortcuts by hand.

### Platforms

`hotkey::backend()` says which mechanism the process uses.

| Platform | Plain chord (`Ctrl+Alt+Space`) | Modifier-only or sided (`RightCtrl`, `LeftShift+Space`) |
|---|---|---|
| Windows | `RegisterHotKey` reserves it: a conflict is an error, and Windows **swallows** the chord (the focused app does not get it). A `WH_KEYBOARD_LL` hook reports press and release. | The hook alone. **Not swallowed** (Right Ctrl still reaches the focused app), and no conflict check. |
| macOS | Carbon `RegisterEventHotKey`: press and release, conflicts reported, swallowed, **no permission needed**. | A listen-only `CGEventTap`, which needs the **Input Monitoring** permission. Not swallowed, no conflict check. |
| Linux, X11 | `XGrabKey` reserves it: a conflict is an error, swallowed. XInput2 raw key events report press and release. | XInput2 raw key events alone. Not swallowed, no conflict check. |
| Linux, Wayland | The `org.freedesktop.portal.GlobalShortcuts` portal (KDE Plasma, recent GNOME). | The same portal. |

**Windows.** A low-level hook sees no input while an elevated (administrator) window has the focus. If a release is missed that way, a 100 ms check of the keys the hook believes are down releases the binding, so a hold never sticks.

**macOS.** The event tap starts only when the first modifier-only or sided binding is registered, so an app that uses only chords never asks for a permission. Without Input Monitoring that registration returns `HotkeyError::PermissionMissing`, and `hotkey::permission()` answers `Permission::Missing`. `hotkey::request_permission()` shows the system prompt. After the user grants it, register the binding again; some macOS versions apply the grant only after the app restarts.

**Wayland.** A Wayland compositor gives no application another application's keyboard, so the portal is the only route, and the **user** decides the binding. The shortcut you pass is the *preferred* trigger; the desktop shows its own dialog, and remembers the user's choice per application and id. That is why `register` answers `Binding::Pending` at once instead of waiting for the dialog, and why `Hotkey::binding()` later becomes `Binding::SystemChosen { description }`, with the desktop's own text for the trigger, or `Binding::Declined` if the user dismissed the dialog. Hear the outcome with `Hotkey::on_binding_change`. Give every hotkey a stable `id`, or the desktop treats a rebound one as a new shortcut. On the portal, **changing the binding is the desktop's job**: the desktop keeps the user's choice for an id, so `rebind`'s new shortcut is only a preference, and it is used only for an id the desktop has no choice for yet. After a rebind, a portal of version 2 or later opens its own dialog (ConfigureShortcuts) so the user can change it there. All hotkeys share one portal session; registrations made in one turn are bound together, with one dialog. A modifier-only shortcut has no portal spelling, so it is registered with no preferred trigger and the user picks one. `HotkeySpec::system_chosen(id)` asks for a binding with no preference at all.

The portal files shortcuts under an **app id**, and it accepts only an app id that has an installed desktop entry (KDE Plasma refuses an app without one). rinch names the app with the id its window carries (`WindowProps::app_id`, else the executable's name; an id without dots is accepted, checked against KDE's registry); `hotkey::set_app_id` overrides that, for example when hotkeys are registered before the window exists. **When no entry is installed, rinch writes one:** a minimal `<app id>.desktop` with `NoDisplay=true` (so it shows in no menu) in `$XDG_DATA_HOME/applications` (`~/.local/share/applications`). It does so only when the portal refused the id for want of an entry, never when a system directory or the user's directory already has one, and it never replaces a file. The entry carries `X-Rinch-Stub=true`, the key every entry rinch writes carries; when the app has a window icon, rinch rewrites such an entry to name it, keeping its `Exec=`, and never touches an entry without the key (the app's own launcher, or one the user hid). An app that installs its own entry, or must not write to the user's home directory, calls `hotkey::set_write_desktop_entry(false)` before registering; without an entry, registration then fails with `HotkeyError::Unsupported`, and the message names the file the desktop needs.

In an X11 session (no `WAYLAND_DISPLAY`) rinch uses X11. `RINCH_HOTKEY_BACKEND=x11` or `=portal` overrides the choice; X11 in a Wayland session sees only keys typed into XWayland windows.

The feature adds no crate to `Cargo.lock`. On Windows and macOS it adds tokio to the build (two crates; only the Linux portal uses it). On Linux it builds the D-Bus stack (zbus) and tokio, which the portal thread runs in; with `file-dialogs` already on, that is two more crates (tokio and its macros).

---

## Timers

`set_timeout` runs a closure once after a delay; `set_interval` runs one every
period until `clear_interval` stops it. Both are in the prelude and work the
same natively and in the browser (`window.setTimeout` underneath).

```rust
let seconds = Signal::new(0u32);
let tick = set_interval(1000, move || seconds.update(|s| *s += 1));
// ...
clear_interval(tick);
```

- The callback runs on the main thread, as one batch, and need not be `Send`.
- The rate is fixed, like a browser's `setInterval`: each tick is scheduled from
  the previous tick's due time, so a slow callback does not make the interval
  drift. A tick more than one period late (a busy main thread, a sleeping
  laptop, a background tab the browser throttles) skips the ticks it missed
  instead of firing them back to back.
- An interval (or timeout) armed while a component renders stops when that
  component unmounts. One armed outside any render runs until cleared. An
  interval may clear itself from its own callback.
- For any elapsed-time judgement that has to agree with the timers (a grace
  period, "has it been 300 ms since this was heard of"), read the time with
  `rinch::time::now()`, not `Instant::now()`. It is the clock the timers fire
  on: the wall clock in an app, and the virtual clock that
  `TestApp::advance_time` moves under the test harness (see
  [Testing](testing.md#time)).

## Polled signals

`poll_signal(source, rate)` mirrors a value that something outside the
reactive graph changes — an atomic an audio thread writes, a hardware reading —
into a `Signal`, by calling `source` on the main thread at `rate`
(`PollRate::EveryFrame`, `Hz(n)` or `Millis(n)`) and setting the signal when the
value changed. Natively the runtime samples it once per frame. In the browser
rinch keeps a drain scheduled while any poll is registered: a
`requestAnimationFrame` loop while an `EveryFrame` poll exists, else a
`setTimeout` until the soonest timed poll is due. A hidden tab runs no animation
frames, so there an `EveryFrame` poll is not sampled until the tab is shown. A
poll stops when its signal is freed (its component unmounts).

## Storage

`rinch-storage` is a key → bytes store with one backend per target: `FsStore`
(a directory of files) natively and `IdbStore` (an IndexedDB object store) in
the browser. `open_app_store` opens one the same way on both, so an app needs no
`cfg` split:

```rust
use rinch_storage::{open_app_store, Store};

let store = open_app_store("com.example.notes", "drafts").await?;
store.put("current", b"hello").await?;
```

Natively the store is the directory `<user data dir>/<app_id>/<store_name>`
(`$XDG_DATA_HOME` or `~/.local/share` on Linux, `~/Library/Application Support`
on macOS, `%APPDATA%` on Windows); where the platform names no data directory
it answers `StorageError::Unavailable`, and on Android it always does (there is
no user data directory an app can write): open an `FsStore` under the app's
files directory there. In the browser it is the IndexedDB database
`"<app_id>/<store_name>"`, one per store. Both names must be usable directory
names everywhere, which is Windows' rule: non-empty, none of
`/ \ : * ? " < > |` or a control character, not ending in `.` or a space, and
not `.`, `..` or a device name (`CON`, `NUL`, `COM1`, … with any extension) —
checked on every target, so a name that works on one works on all.
On Linux and macOS an `FsStore`'s files are readable by the user alone (mode
`0600`, the directories it creates `0700`, whatever the umask), so a store can
keep a session token; `open_app_store` also makes its store directory `0700` if
it already exists, and an `FsStore` opened on an existing directory leaves that
directory's mode as it is. A file written before is made `0600` by its next
`put`. On Windows the files take the permissions of the directory they are in,
and the user's data directory is the user's own.
The native future never waits, so
any executor (or a one-poll `block_on`) runs it.

## Logging

`rinch::init_logging()` turns on logging for whatever target the app is built
for. Call it first thing in `main` or the wasm start function:

- **Native:** `tracing` events and `log` records go to stderr, coloured only
  when stderr is a terminal (and `NO_COLOR` is unset); `RUST_LOG`, when set,
  chooses what is shown. An app that does not call it still logs to stderr:
  `App::run` installs the same output with `warn` as the default level.
- **Browser:** each event goes to the console method of its level
  (`console.error`, `.warn`, `.info`, `.log` for debug, `.debug` for trace), and
  a panic is written to `console.error` with its message and the JavaScript
  stack — what `console_log` and `console_error_panic_hook` used to be added
  for.
- **Android:** `log` records go to logcat under the tag `rinch`.

The default level is `info`; `init_logging_with_filter("warn,my_app=debug")`
takes another filter: a level and `target=level` directives (`RUST_LOG` still
wins natively). The desktop build reads all of `tracing`'s `EnvFilter` syntax;
the browser reads levels and targets only, so a web app's dependencies gain
nothing. A second call changes nothing. The `tracing` subscriber and the `log`
logger are two global slots, each filled only if empty: if the app set up a
`log` logger first (`env_logger`, `console_log`), `init_logging` still installs
its `tracing` output, and `log` records keep going to the app's logger; if a
`tracing` subscriber was set first, it keeps the `tracing` events.

## DevTools

On the desktop, **F12** opens and closes the DevTools window (the element
tree, computed styles, frame timing) and **Alt+I** turns inspect mode on and
off, which outlines the element under the pointer. **They are on in debug builds
and off in release builds** unless the app says otherwise
(`rinch::app::DEVTOOLS_DEFAULT`, `cfg!(debug_assertions)`), so a shipped app
has no DevTools window by accident. Both keys are a fallback: a menu shortcut, a focused widget's
`on_key`, an element's key handler and an app-wide `on_app_key` listener all
see the key first, and any of them that takes it keeps it from DevTools.

`App::devtools` sets them for the app in either kind of build:

```rust
App::new(app)
    .devtools(false)   // never, not even in a debug build
    // .devtools(true) // also in a release build
    .run();
```

Off, the keys are not consumed: F12 and Alt+I go on to the rest of rinch's key
handling like any other key (an Alt+I that typed a character goes into a
focused text field), so the app can use them for itself. A DevTools window
already open is not closed by turning the keys off. Change the setting while
the app runs with `RinchApp::set_devtools`; an embedded `RinchContext` has
`set_devtools` too, and a host with no DevTools window of its own should turn
it off, since the keys otherwise end as `AppAction::ToggleDevTools` and
`AppAction::ToggleInspectMode` in what `update` returns.

**`RINCH_DEVTOOLS` overrides the app's choice**, both ways, so a developer can
debug an app that ships with DevTools off without rebuilding it:

```bash
RINCH_DEVTOOLS=on ./my-app    # F12 and Alt+I work, whatever the app chose
RINCH_DEVTOOLS=off ./my-app   # neither works, whatever the app chose
```

`on`, `1`, `true` and `yes` turn them on, `off`, `0`, `false` and `no` turn
them off (in any case); an empty variable leaves the app's choice, and any
other value is ignored with a warning in the log.

## Enabling Features

Add features to your `Cargo.toml`:

```toml
[dependencies]
rinch = { version = "0.1", features = ["file-dialogs", "clipboard", "system-tray", "notifications"] }
```

## Platform Support

| Feature | Windows | macOS | Linux | Android | Web |
|---------|---------|-------|-------|---------|-----|
| File Dialogs | ✓ | ✓ | ✓ | SAF (`rinch_android::file_picker`) | – |
| Message Dialogs | ✓ | ✓ | ✓ | – | `alert` / `confirm` |
| Timers, polled signals, `init_logging` | ✓ | ✓ | ✓ | ✓ | ✓ |
| `open_app_store` | ✓ | ✓ | ✓ | `Unavailable`: use `FsStore` (see [Storage](#storage)) | IndexedDB |
| Clipboard (Text) | ✓ | ✓ | ✓ | ✓ | see [Web](#web-reaching-content-copied-outside-the-app) |
| Clipboard (Image) | ✓ | ✓ | ✓* | – | – |
| Text-selection toolbar | DOM menu | DOM menu | DOM menu | native (`ActionMode`) | browser |
| System Tray | ✓ | ✓ | ✓** | – | – |
| Notifications | toast | ✓ (in an `.app` bundle) | ✓ (freedesktop server) | – | – |
| Global hotkeys | ✓ | ✓ | ✓ (X11, Wayland portal) | – | – |

\* Linux image clipboard requires X11 or Wayland clipboard support.

\** Linux system tray requires a system tray implementation (e.g., libappindicator).
