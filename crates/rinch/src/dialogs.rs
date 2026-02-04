//! Native file dialogs for open, save, and folder selection.
//!
//! This module provides cross-platform file dialog support using the `rfd` crate.
//!
//! # Example
//!
//! ```ignore
//! use rinch::dialogs::{open_file, save_file, pick_folder};
//!
//! // Open a single file
//! if let Some(path) = open_file()
//!     .add_filter("Images", &["png", "jpg", "gif"])
//!     .add_filter("All Files", &["*"])
//!     .pick_file()
//! {
//!     println!("Selected: {}", path.display());
//! }
//!
//! // Save a file
//! if let Some(path) = save_file()
//!     .set_file_name("document.txt")
//!     .add_filter("Text Files", &["txt"])
//!     .save()
//! {
//!     println!("Save to: {}", path.display());
//! }
//!
//! // Pick a folder
//! if let Some(path) = pick_folder().pick() {
//!     println!("Folder: {}", path.display());
//! }
//! ```

use rfd::{FileDialog, MessageButtons, MessageDialog};
use std::path::{Path, PathBuf};

// Re-export MessageLevel for convenience
pub use rfd::MessageLevel;

/// Builder for opening files.
///
/// # Example
///
/// ```ignore
/// use rinch::dialogs::open_file;
///
/// let path = open_file()
///     .set_title("Select an image")
///     .add_filter("Images", &["png", "jpg", "gif"])
///     .set_directory("/home/user/pictures")
///     .pick_file();
/// ```
pub struct OpenFileDialog {
    dialog: FileDialog,
}

impl OpenFileDialog {
    /// Create a new open file dialog.
    pub fn new() -> Self {
        Self {
            dialog: FileDialog::new(),
        }
    }

    /// Set the dialog title.
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.dialog = self.dialog.set_title(title);
        self
    }

    /// Set the starting directory.
    pub fn set_directory(mut self, path: impl AsRef<Path>) -> Self {
        self.dialog = self.dialog.set_directory(path);
        self
    }

    /// Add a file filter (e.g., "Images", &["png", "jpg"]).
    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.dialog = self.dialog.add_filter(name, extensions);
        self
    }

    /// Show the dialog and pick a single file.
    pub fn pick_file(self) -> Option<PathBuf> {
        self.dialog.pick_file()
    }

    /// Show the dialog and pick multiple files.
    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        self.dialog.pick_files()
    }
}

impl Default for OpenFileDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for saving files.
///
/// # Example
///
/// ```ignore
/// use rinch::dialogs::save_file;
///
/// let path = save_file()
///     .set_title("Save document")
///     .set_file_name("untitled.txt")
///     .add_filter("Text Files", &["txt"])
///     .save();
/// ```
pub struct SaveFileDialog {
    dialog: FileDialog,
}

impl SaveFileDialog {
    /// Create a new save file dialog.
    pub fn new() -> Self {
        Self {
            dialog: FileDialog::new(),
        }
    }

    /// Set the dialog title.
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.dialog = self.dialog.set_title(title);
        self
    }

    /// Set the starting directory.
    pub fn set_directory(mut self, path: impl AsRef<Path>) -> Self {
        self.dialog = self.dialog.set_directory(path);
        self
    }

    /// Set the default file name.
    pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
        self.dialog = self.dialog.set_file_name(name);
        self
    }

    /// Add a file filter (e.g., "Text Files", &["txt"]).
    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.dialog = self.dialog.add_filter(name, extensions);
        self
    }

    /// Show the dialog and get the save path.
    pub fn save(self) -> Option<PathBuf> {
        self.dialog.save_file()
    }
}

impl Default for SaveFileDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for picking folders.
///
/// # Example
///
/// ```ignore
/// use rinch::dialogs::pick_folder;
///
/// let path = pick_folder()
///     .set_title("Select output folder")
///     .set_directory("/home/user")
///     .pick();
/// ```
pub struct FolderDialog {
    dialog: FileDialog,
}

impl FolderDialog {
    /// Create a new folder picker dialog.
    pub fn new() -> Self {
        Self {
            dialog: FileDialog::new(),
        }
    }

    /// Set the dialog title.
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.dialog = self.dialog.set_title(title);
        self
    }

    /// Set the starting directory.
    pub fn set_directory(mut self, path: impl AsRef<Path>) -> Self {
        self.dialog = self.dialog.set_directory(path);
        self
    }

    /// Show the dialog and pick a folder.
    pub fn pick(self) -> Option<PathBuf> {
        self.dialog.pick_folder()
    }

    /// Show the dialog and pick multiple folders.
    pub fn pick_multiple(self) -> Option<Vec<PathBuf>> {
        self.dialog.pick_folders()
    }
}

impl Default for FolderDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for message dialogs (alerts, confirmations).
///
/// # Example
///
/// ```ignore
/// use rinch::dialogs::{message, MessageLevel};
///
/// // Simple alert
/// message("File saved successfully!")
///     .set_title("Success")
///     .show();
///
/// // Confirmation dialog
/// let confirmed = message("Are you sure you want to delete this file?")
///     .set_title("Confirm Delete")
///     .set_level(MessageLevel::Warning)
///     .confirm();
/// ```
pub struct MessageDialogBuilder {
    dialog: MessageDialog,
}

impl MessageDialogBuilder {
    /// Create a new message dialog with the given message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            dialog: MessageDialog::new().set_description(message),
        }
    }

    /// Set the dialog title.
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.dialog = self.dialog.set_title(title);
        self
    }

    /// Set the message level (Info, Warning, Error).
    pub fn set_level(mut self, level: MessageLevel) -> Self {
        self.dialog = self.dialog.set_level(level);
        self
    }

    /// Show an OK button only.
    pub fn show(self) {
        self.dialog.set_buttons(MessageButtons::Ok).show();
    }

    /// Show OK/Cancel buttons and return whether OK was clicked.
    pub fn confirm(self) -> bool {
        self.dialog.set_buttons(MessageButtons::OkCancel).show() == rfd::MessageDialogResult::Ok
    }

    /// Show Yes/No buttons and return whether Yes was clicked.
    pub fn ask(self) -> bool {
        self.dialog.set_buttons(MessageButtons::YesNo).show() == rfd::MessageDialogResult::Yes
    }
}

/// Create an open file dialog builder.
///
/// # Example
///
/// ```ignore
/// use rinch::dialogs::open_file;
///
/// if let Some(path) = open_file()
///     .add_filter("Rust Files", &["rs"])
///     .pick_file()
/// {
///     println!("Opening: {}", path.display());
/// }
/// ```
pub fn open_file() -> OpenFileDialog {
    OpenFileDialog::new()
}

/// Create a save file dialog builder.
///
/// # Example
///
/// ```ignore
/// use rinch::dialogs::save_file;
///
/// if let Some(path) = save_file()
///     .set_file_name("output.txt")
///     .save()
/// {
///     println!("Saving to: {}", path.display());
/// }
/// ```
pub fn save_file() -> SaveFileDialog {
    SaveFileDialog::new()
}

/// Create a folder picker dialog builder.
///
/// # Example
///
/// ```ignore
/// use rinch::dialogs::pick_folder;
///
/// if let Some(path) = pick_folder().pick() {
///     println!("Selected folder: {}", path.display());
/// }
/// ```
pub fn pick_folder() -> FolderDialog {
    FolderDialog::new()
}

/// Create a message dialog builder.
///
/// # Example
///
/// ```ignore
/// use rinch::dialogs::message;
///
/// message("Operation completed!")
///     .set_title("Info")
///     .show();
/// ```
pub fn message(text: impl Into<String>) -> MessageDialogBuilder {
    MessageDialogBuilder::new(text)
}
