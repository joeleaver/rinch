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
