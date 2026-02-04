# Platform Features

Rinch provides optional platform integration features that can be enabled via Cargo features.

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

fn app() -> Element {
    let text = use_signal(|| String::new());
    let text_copy = text.clone();
    let text_paste = text.clone();

    rsx! {
        Window { title: "Clipboard Demo",
            input {
                value: {text.get()},
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

System tray icon with menu support.

### Basic Tray Icon

```rust
use rinch::tray::{TrayIconBuilder, TrayMenu, TrayMenuItem};

// Create a tray menu
let menu = TrayMenu::new()
    .add_item(TrayMenuItem::new("Show Window"))
    .add_separator()
    .add_item(TrayMenuItem::new("Settings"))
    .add_separator()
    .add_item(TrayMenuItem::new("Quit"));

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

// From file path
let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_icon_path("assets/icon.png", None)?
    .build()?;

// From RGBA data (32x32 icon)
let rgba = vec![255u8; 32 * 32 * 4]; // White icon
let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_icon_rgba(rgba, 32, 32)?
    .build()?;
```

### Menu Callbacks

```rust
use rinch::tray::{TrayIconBuilder, TrayMenu, TrayMenuItem};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

let should_quit = Arc::new(AtomicBool::new(false));
let quit_flag = should_quit.clone();

let menu = TrayMenu::new()
    .add_item(
        TrayMenuItem::new("Show Window")
            .on_click(|| println!("Show clicked!"))
    )
    .add_separator()
    .add_item(
        TrayMenuItem::new("Quit")
            .on_click(move || {
                quit_flag.store(true, Ordering::SeqCst);
            })
    );

let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_menu(menu)
    .build()?;

// Poll events in your event loop
tray.poll_events();
```

### Nested Submenus

```rust
use rinch::tray::{TrayMenu, TrayMenuItem};

let submenu = TrayMenu::new()
    .add_item(TrayMenuItem::new("Option 1"))
    .add_item(TrayMenuItem::new("Option 2"))
    .add_item(TrayMenuItem::new("Option 3"));

let menu = TrayMenu::new()
    .add_item(TrayMenuItem::new("Main Action"))
    .add_submenu("More Options", submenu)
    .add_separator()
    .add_item(TrayMenuItem::new("Quit"));
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
