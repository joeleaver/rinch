# Showing HTML You Did Not Write

Mail, feed items, help pages from a server, rich chat: HTML someone else
wrote, to show inside your app. `rinch::html` (the `html` feature) does the
two things that takes:

- **`clean`** reduces the document to what is safe to put into your app's
  DOM.
- **`HtmlView`** shows it, and tells you which link was clicked.

```toml
rinch = { version = "…", features = ["desktop", "components", "html"] }
```

```rust
use rinch::html::Policy;
use rinch::prelude::*;

#[component]
fn message(document: String) -> NodeHandle {
    rsx! {
        HtmlView {
            document: document,
            policy: Policy::default(),
            onlink: move |href: String| {
                if href.starts_with("https://") || href.starts_with("mailto:") {
                    let _ = open_url(&href);
                }
            },
        }
    }
}
```

Given the document as written, the view cleans it itself, **at its own
width**, and again when a resize crosses one of the document's `@media`
breakpoints (see below). Given HTML already cleaned instead (`html` and
`body_style` from `clean`), it shows that at any width.

## What `clean` does

- **Parses as a browser does** (html5ever) and keeps only an allow-list:
  no scripts, forms, frames, objects, event attributes, or `<style>` and
  `<link>` elements.
- **Applies the document's style sheets.** Its `<style>` rules are copied
  onto the elements they match and the sheets dropped: a sheet in your DOM
  would style your app's own elements. A linked sheet is never fetched.
  `@media` blocks are evaluated at a width: `clean_at(document, policy,
  width)` applies those that match there (`clean` leaves them all out).
- **Filters inline styles**, parsed as the style engine parses them (so an
  escaped or commented property name is read as the browser reads it):
  nothing that lifts content out of its own box (`position` other than
  `static`/`relative`, `z-index`), no `expression()` or `javascript:`, no
  `behavior`, `-moz-binding`, `cursor` or `pointer-events`, no absurd
  lengths.
- **Cuts deep nesting**: content nested more than 512 elements deep, as the
  HTML parser builds it, is left out (mail is a few dozen deep; tens of
  thousands crash a recursive serializer), and `@media` blocks nested more
  than 16 deep.
- **Asks you about every picture.** `Policy::picture` sees each `<img src>`
  and each `url()` in a style (`image-set()` and the other ways to name a
  picture are left out, as the rule would never see them) and answers the source to load (the same, or
  rewritten) or `None` to leave it out. The default keeps `data:image/`
  pictures only, so a cleaned document fetches nothing unless you say so.
- **Limits links** to `Policy::link_schemes` (`http`, `https`, `mailto` by
  default) and gives them `rel="noopener noreferrer"`.
- **Keeps legacy layout attributes** (`bgcolor`, `width`, `align`,
  `valign`, `cellpadding`, `border`, `<font color>`…): rinch maps them as
  presentational hints, as a browser does.
- **Drops `title` and `cite`**: a tooltip the document wrote could say
  anything ("your account is verified"), and a citation is a URL nobody
  follows.
- **Keeps the body's own look.** `Cleaned::body_style` is the `<body>`'s
  style (and its `bgcolor`/`text` attributes as CSS), for the element that
  stands for the body; `HtmlView` puts it there.

Plain text has `Cleaned::from_text`: escaped, line breaks and runs of
spaces kept.

## Pictures

A mail client loads pictures from the web only when the reader allows it,
and shows the ones the message carries (`cid:`) inline:

```rust
use std::sync::Arc;

let carried: HashMap<String, String> = /* Content-ID -> data: URL */;
let policy = Policy {
    picture: Arc::new(move |src| {
        if let Some(id) = src.strip_prefix("cid:") {
            return carried.get(id).cloned();
        }
        if src.starts_with("data:image/") {
            return Some(src.to_owned());
        }
        (allow_web && src.starts_with("https://")).then(|| src.to_owned())
    }),
    ..Policy::default()
};
```

The rule runs during `clean`, once per picture, so it can also count what it
held back ("3 pictures are hidden").

## Links

`HtmlView` calls `onlink` with a clicked link's `href`, as written (after
`clean`'s scheme check), whether the link was clicked, activated with Enter,
or clicked by assistive technology. It never navigates or opens anything
itself, and on the web the browser does not either: the view marks itself
`data-claim-links`, and a click on a link inside an element so marked has
its default cancelled. A middle click comes to `onlink` too
(`click_context().button` is `Middle`, for "open in a new tab"). The
browser's own link menu and dragging a link to the tab bar would open it
without asking you, so they are off; `link_menu: true` allows them. Check
the scheme before opening anything.

The view learns which link was clicked from `click_link()` (in the prelude;
`rinch_core::click_link`), which any click handler can read: the `href` of
the nearest `<a href>` the click went through on its way to the handler that
runs, on desktop and on the web. A handler on a container whose content you
set with `set_inner_html` can use it directly.

## Widths and `@media`

HTML email lays itself out for a phone with `@media (max-width: 600px)`
rules: columns stack, pictures fill the width, a desktop-only block hides.
Inlined, a sheet's rules cannot be left for the style engine to evaluate (and
the engine would ask the window's width, not the view's), so `clean_at`
evaluates each query at the width given, unwraps the blocks that match and
drops the rest. Supported: media types (`all`, `screen`; `print` never
matches), `only`, `not`, comma lists, `and`, `width`/`min-width`/`max-width`
and their `-device-` forms in `px`, `em` and `rem`, and
`prefers-color-scheme: light`. `signature(document, width)` says which
blocks match, which is how `HtmlView` knows a resize needs a new clean.

## Links say where they go

By default every link in an `HtmlView` gets its address as its tooltip
(`title`), with a warning when the address hides or disguises where it goes:
a user name before the host (`https://paypal.com@evil.example/`), a host
that mixes look-alike alphabets or passes Cyrillic for Latin (shown in
punycode; an international name in its own script, `münchen.de`, is fine),
or link text naming another site than the one it opens. Addresses are read
with the `url` crate, as a browser reads them, and sites compared by the
Public Suffix List (`evil.github.io` is not `good.github.io`). A Latin-only look-alike (`paypa1.com`, `rnicrosoft.com`) is flagged only when the link's text names the real site; on its own it reads as just another address. `check_link(href, text)` gives the same answer as data
(`LinkCheck`, `Warning`); `HtmlView`'s `tooltips` field chooses the words:
`Tooltips::Default` (English), `Tooltips::Off`, or `Tooltips::Custom` with
your own function.

## The view's box

Its text can be selected and copied, as in a browser, though your app's own
text is not ([Selecting text](theming.md#selecting-text-desktop)): the view
sets `user-select: text` on itself. `selectable: false` turns that off. Both
come from rules of zero specificity, so your own `user-select` on the view
wins: give it a class (`HtmlView { class: "mail", … }`) and style
`.mail { user-select: none }`, per view.

Nothing a document draws reaches outside its view: the view clips (a
document wider than it scrolls sideways) and is the stacking context and
containing block of everything in it, so a relative offset, a transform or
a negative margin cannot put the document's link over your app's own
controls. (A `position: fixed` box is placed against the window whatever
contains it; `clean` never lets one through, so this holds for what it
returns. HTML you pass as `html` or `body_style` is taken as it is.) All of
this is the view's own CSS on `.rinch-html-view`: restyle it through the
cascade (your font, your page colour) as you need. A rule of yours reaches the view's own element,
not the document inside it ([below](#your-apps-styles-stop-at-the-view)).

## Your app's styles stop at the view

A document in an `HtmlView` is laid out as a browser lays out the same HTML
put in a `<div>` of a page with no style sheet of its own: by the browser's
own (user-agent) sheet, its presentational attributes (`width`, `bgcolor`,
`cellpadding`…) and its own styles. Your theme's base styles, your
components' CSS and your app's own CSS do not match its elements, so a
theme's `* { box-sizing: border-box }` cannot turn a mail's `<td
style="width: 200pt; padding: 0 6pt">` into a narrower cell, and a theme's
`h5 { line-height: 1.5 }` cannot override the `line-height` a heading
inherits. Its links have the browser's look (blue, underlined), not your
theme's. On the web the view keeps the document in a shadow root; on
desktop rinch matches it the same way (issue #2357).

Inherited properties still come in from the view, as they would from the
`<div>`: put `color`, `font-family`, `font-size` or `line-height` on the
view or on an element around it, and a document that sets none uses yours.
The view sets no colours of its own, so a document that sets none takes
your text colour and shows your background.

**A dark-themed app colours the view's host, or sets `paper: true`.**
Most mail is written for a white page; under a dark theme its text would
otherwise come out light on your dark background, with its own coloured
tables beside it. Give the element around the view the page you want
(Postrinch's `.mail-html` is `background: #fff; color: #222`), or set
`paper: true` and the view draws the document black on white whatever your
theme, as a browser draws a page with no colours of its own:

```rust
rsx! { HtmlView { document: mail, paper: true } }
```

`paper` is a rule of zero specificity, so your own `color` or
`background-color` on the view still wins. (Until #2357 every view was
black on white.)

To have your styles reach the document anyway, for HTML of your own that
should look like the rest of your app, set `app_styles: true`:

```rust
rsx! { HtmlView { html: help_page, app_styles: true } }
```

A `<style>` element is not something `clean` lets through, and `html` is
meant for cleaned HTML; if you put one there anyway, the two backends
differ. On desktop its rules join your app's sheets (they style your app
and, being author rules, do not match inside the view); on the web it sits
in the view's shadow root and styles the document only.

On the web, a selector you run from outside the view
(`query_selector`, `query_selector_all` on the view or above) does not
see into the document, as a browser selector stops at a shadow root; run
it on the view's body (`.rinch-html-body`, the view's last child in
`children()`) or below, which works on both backends. `text_content` and `children` on the view do include the
document on both.

## Text, and hidden text

`text(&cleaned.html)` gives the document as text: `visible`, what a reader
sees (blocks apart by a blank line), and `hidden`, each stretch of text the
document hides from its reader with `display: none`, `visibility: hidden`,
`opacity: 0`, a 1px font, a box of (nearly) no height or width with
`overflow: hidden`, a large negative offset or `text-indent`, `scale(0)`, a
`clip-path`, a transparent colour, or a colour the same as or very near its
background. `text` assumes a white page; `text_on(html, page)` takes yours.
Newsletters hide preview lines that way; an
attacker hides instructions to an AI reading the mail. Hand an agent the
visible text as the document, and the hidden text apart, marked as hidden.

## Showing another document

`HtmlView` draws its document once. To show another, render a new view: key
it on the document in a `for` loop.
