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
- **Filters inline styles**: nothing that lifts content out of its own box
  (`position` other than `static`/`relative`, `z-index`), no
  `expression()` or `javascript:`, no `behavior`, `-moz-binding`, `cursor`
  or `pointer-events`.
- **Asks you about every picture.** `Policy::picture` sees each `<img src>`
  and each `url()` in a style and answers the source to load (the same, or
  rewritten) or `None` to leave it out. The default keeps `data:image/`
  pictures only, so a cleaned document fetches nothing unless you say so.
- **Limits links** to `Policy::link_schemes` (`http`, `https`, `mailto` by
  default) and gives them `rel="noopener noreferrer"`.
- **Keeps legacy layout attributes** (`bgcolor`, `width`, `align`,
  `valign`, `cellpadding`, `border`, `<font color>`…): rinch maps them as
  presentational hints, as a browser does.
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
`clean`'s scheme check). It never navigates or opens anything itself: check
the scheme, and, for a document from outside, consider showing the real
address before opening it.

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

Every link in an `HtmlView` gets its address as its tooltip (`title`), and
when its text names another site than the one it opens (`www.paypal.com`
written over a link elsewhere), the tooltip says so.

## Text, and hidden text

`text(&cleaned.html)` gives the document as text: `visible`, what a reader
sees (blocks apart by a blank line), and `hidden`, each stretch of text the
document hides from its reader with `display: none`, `visibility: hidden`,
`opacity: 0`, a 1px font, a zero height with `overflow: hidden`, or a colour
the same as its background. Newsletters hide preview lines that way; an
attacker hides instructions to an AI reading the mail. Hand an agent the
visible text as the document, and the hidden text apart, marked as hidden.

## Showing another document

`HtmlView` draws its document once. To show another, render a new view: key
it on the document in a `for` loop.
