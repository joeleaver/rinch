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
`RichPaste`, so a rich-paste consumer never stacks three worst-case waits — it is
what the built-in editor's Ctrl+V uses.

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

The interceptor is a single slot per thread (like `set_keyboard_interceptor`) and
is dispatched by rinch-web; desktop reads the clipboard directly when Ctrl+V
arrives and has no OS paste event to hang it off.

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

| Feature | Windows | macOS | Linux |
|---------|---------|-------|-------|
| File Dialogs | ✓ | ✓ | ✓ |
| Clipboard (Text) | ✓ | ✓ | ✓ |
| Clipboard (Image) | ✓ | ✓ | ✓* |
| System Tray | ✓ | ✓ | ✓** |

\* Linux image clipboard requires X11 or Wayland clipboard support.

\** Linux system tray requires a system tray implementation (e.g., libappindicator).
