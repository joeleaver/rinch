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

Supported formats: PNG, JPEG, GIF, WebP.

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
2. If not cached, loading is dispatched to a background thread via the `ImageLoader` trait
3. The image bytes are decoded (using the `image` crate) into RGBA8 pixel data
4. On the next layout pass, decoded images are picked up from a pending queue and inserted into the cache
5. The element is marked dirty for re-layout/re-paint with the image's intrinsic dimensions

### Custom Image Loader

You can implement the `ImageLoader` trait for custom loading strategies (e.g., embedded assets, authenticated downloads):

```rust
use rinch_core::image::{ImageLoader, ImageLoadResult};

struct AssetLoader;

impl ImageLoader for AssetLoader {
    fn load(&self, src: &str) -> ImageLoadResult {
        match load_from_assets(src) {
            Ok(bytes) => ImageLoadResult::Loaded(bytes),
            Err(e) => ImageLoadResult::Failed(e.to_string()),
        }
    }
}
```

---

## File Dialogs

Enable with: `features = ["file-dialogs"]`

Native file dialogs for opening, saving, and folder selection.

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

---

## Clipboard

Enable with: `features = ["clipboard"]`

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
`desktop`-only), so the toolbar covers `<input>` and `<textarea>`.

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

The tray lives exactly as long as the `TrayIcon` that `build()` returned. Dropping it removes the icon from the panel and releases its menu callbacks, in that order, so an icon still on screen never has items that do nothing. On Linux the drop shuts ksni's D-Bus service down and returns once it has closed its connection. Keep the handle for as long as you want the tray, for example as a local in `main` that lives until the event loop exits: `let _tray = …` keeps it, while a bare `let _ = …` drops it at once.

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

---

## Enabling Features

Add features to your `Cargo.toml`:

```toml
[dependencies]
rinch = { version = "0.1", features = ["file-dialogs", "clipboard", "system-tray"] }
```

## Platform Support

| Feature | Windows | macOS | Linux | Android |
|---------|---------|-------|-------|---------|
| File Dialogs | ✓ | ✓ | ✓ | SAF (`rinch_android::file_picker`) |
| Clipboard (Text) | ✓ | ✓ | ✓ | ✓ |
| Clipboard (Image) | ✓ | ✓ | ✓* | – |
| Text-selection toolbar | DOM menu | DOM menu | DOM menu | native (`ActionMode`) |
| System Tray | ✓ | ✓ | ✓** | – |

\* Linux image clipboard requires X11 or Wayland clipboard support.

\** Linux system tray requires a system tray implementation (e.g., libappindicator).
