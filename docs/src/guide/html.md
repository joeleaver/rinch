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
use rinch::html::{clean, Policy};
use rinch::prelude::*;

#[component]
fn message(document: String) -> NodeHandle {
    let cleaned = clean(&document, &Policy::default());
    rsx! {
        HtmlView {
            html: cleaned.html,
            body_style: cleaned.body_style,
            onlink: move |href: String| {
                if href.starts_with("https://") || href.starts_with("mailto:") {
                    let _ = rinch::window::open_url(&href);
                }
            },
        }
    }
}
```

## What `clean` does

- **Parses as a browser does** (html5ever) and keeps only an allow-list:
  no scripts, forms, frames, objects, event attributes, or `<style>` and
  `<link>` elements.
- **Applies the document's style sheets.** Its `<style>` rules are copied
  onto the elements they match and the sheets dropped: a sheet in your DOM
  would style your app's own elements. Rules inside `@media` blocks are left
  out (they are mostly phone layouts), and a linked sheet is never fetched.
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

## Showing another document

`HtmlView` draws its document once. To show another, render a new view: key
it on the document in a `for` loop.
