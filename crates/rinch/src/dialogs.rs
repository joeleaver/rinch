//! File dialogs (open, save, folder) and message dialogs (alert, confirm).
//!
//! Natively both use the `rfd` crate. **In the browser** (wasm32) only the
//! message dialogs exist (#1773): [`message`] with [`MessageDialogBuilder::show`],
//! [`confirm`](MessageDialogBuilder::confirm) and [`ask`](MessageDialogBuilder::ask)
//! are the page's own `window.alert` / `window.confirm`, synchronous as they are
//! natively, so the same call compiles and behaves alike on both targets. The
//! browser draws no title (it is written as the message's first line), no level,
//! and labels `ask`'s buttons OK / Cancel. The file and folder pickers are
//! native-only: a browser picker is asynchronous and gives no path.
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

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
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
}

#[cfg(target_arch = "wasm32")]
pub use web::*;

/// The browser's message dialogs: `window.alert` / `window.confirm`.
#[cfg(target_arch = "wasm32")]
mod web {
    /// The severity of a message. Natively it picks the dialog's icon; the
    /// browser draws none, so on the web it is accepted and ignored.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum MessageLevel {
        /// Information.
        #[default]
        Info,
        /// A warning.
        Warning,
        /// An error.
        Error,
    }

    /// Builder for message dialogs (alerts, confirmations), drawn by the
    /// browser with `window.alert` / `window.confirm`.
    pub struct MessageDialogBuilder {
        message: String,
        title: Option<String>,
    }

    impl MessageDialogBuilder {
        /// Create a new message dialog with the given message.
        pub fn new(message: impl Into<String>) -> Self {
            Self {
                message: message.into(),
                title: None,
            }
        }

        /// Set the dialog title. The browser has no title line of its own, so
        /// it is shown as the message's first line.
        pub fn set_title(mut self, title: impl Into<String>) -> Self {
            self.title = Some(title.into());
            self
        }

        /// Set the message level. Ignored in the browser.
        pub fn set_level(self, _level: MessageLevel) -> Self {
            self
        }

        fn text(&self) -> String {
            match &self.title {
                Some(title) if !title.is_empty() => format!("{title}\n\n{}", self.message),
                _ => self.message.clone(),
            }
        }

        /// Show the message with an OK button (`window.alert`).
        pub fn show(self) {
            if let Some(window) = web_sys::window() {
                let _ = window.alert_with_message(&self.text());
            }
        }

        /// Show OK / Cancel (`window.confirm`) and return whether OK was
        /// clicked. `false` when the page cannot show one (no window, or the
        /// browser suppressed the dialog).
        pub fn confirm(self) -> bool {
            web_sys::window()
                .and_then(|window| window.confirm_with_message(&self.text()).ok())
                .unwrap_or(false)
        }

        /// Ask a yes / no question and return whether it was answered yes.
        /// The browser's buttons read OK / Cancel; OK is yes.
        pub fn ask(self) -> bool {
            self.confirm()
        }
    }

    /// Create a message dialog builder.
    pub fn message(text: impl Into<String>) -> MessageDialogBuilder {
        MessageDialogBuilder::new(text)
    }
}
