//! Type-safe icon system for Rinch widgets.
//!
//! The `Icon` enum provides a discoverable, type-safe way to specify icons
//! in widgets. This replaces arbitrary HTML strings with a curated icon library.
//!
//! # Benefits
//!
//! - **Type safety**: Compiler catches typos and invalid icon names
//! - **Discoverability**: IDE autocomplete shows available icons
//! - **Consistency**: All icons use the same SVG style and sizing
//! - **Performance**: Icons render as structured nodes, enabling differential updates
//!
//! # Example
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! rsx! {
//!     Alert { icon: Icon::InfoCircle, color: "blue", title: "Information",
//!         "This is an informational message."
//!     }
//!     Button { left_section: Icon::Search, "Search" }
//! }
//! ```

/// Available icons in the Rinch icon library.
///
/// Icons are rendered as SVG elements with consistent styling (24x24 viewBox,
/// stroke-based, currentColor for theming).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    // ─────────────────────────────────────────────────────────────────────────
    // Navigation
    // ─────────────────────────────────────────────────────────────────────────
    /// Chevron pointing up (^)
    ChevronUp,
    /// Chevron pointing down (v)
    ChevronDown,
    /// Chevron pointing left (<)
    ChevronLeft,
    /// Chevron pointing right (>)
    ChevronRight,
    /// Double chevron pointing left (<<)
    ChevronsLeft,
    /// Double chevron pointing right (>>)
    ChevronsRight,
    /// Arrow pointing up
    ArrowUp,
    /// Arrow pointing down
    ArrowDown,
    /// Arrow pointing left
    ArrowLeft,
    /// Arrow pointing right
    ArrowRight,

    // ─────────────────────────────────────────────────────────────────────────
    // Actions
    // ─────────────────────────────────────────────────────────────────────────
    /// Close/X icon
    Close,
    /// Checkmark
    Check,
    /// Plus sign (+)
    Plus,
    /// Minus sign (-)
    Minus,
    /// Magnifying glass
    Search,
    /// Gear/cog
    Settings,
    /// Pencil
    Edit,
    /// Trash can
    Trash,

    // ─────────────────────────────────────────────────────────────────────────
    // Status/Alerts
    // ─────────────────────────────────────────────────────────────────────────
    /// Information icon (i in circle)
    InfoCircle,
    /// Success icon (checkmark in circle)
    CheckCircle,
    /// Error icon (! in circle)
    AlertCircle,
    /// Warning icon (! in triangle)
    AlertTriangle,
    /// Error icon (X in circle)
    XCircle,

    // ─────────────────────────────────────────────────────────────────────────
    // Content
    // ─────────────────────────────────────────────────────────────────────────
    /// Person silhouette
    User,
    /// Envelope
    Mail,
    /// Telephone
    Phone,
    /// Calendar
    Calendar,
    /// Clock face
    Clock,
    /// Document/file
    File,
    /// Folder
    Folder,
    /// Image/picture
    Image,
    /// Chain link
    Link,
    /// Link with arrow pointing out
    ExternalLink,

    // ─────────────────────────────────────────────────────────────────────────
    // UI
    // ─────────────────────────────────────────────────────────────────────────
    /// Eye (visible/show)
    Eye,
    /// Eye with slash (hidden/hide)
    EyeOff,
    /// Hamburger menu (three lines)
    Menu,
    /// Three horizontal dots (...)
    MoreHorizontal,
    /// Three vertical dots
    MoreVertical,
    /// Loading spinner
    Loader,
    /// Quotation marks
    Quote,
}

impl Icon {
    /// Get the icon name as a string (useful for debugging/logging).
    pub fn name(&self) -> &'static str {
        match self {
            // Navigation
            Icon::ChevronUp => "chevron-up",
            Icon::ChevronDown => "chevron-down",
            Icon::ChevronLeft => "chevron-left",
            Icon::ChevronRight => "chevron-right",
            Icon::ChevronsLeft => "chevrons-left",
            Icon::ChevronsRight => "chevrons-right",
            Icon::ArrowUp => "arrow-up",
            Icon::ArrowDown => "arrow-down",
            Icon::ArrowLeft => "arrow-left",
            Icon::ArrowRight => "arrow-right",

            // Actions
            Icon::Close => "close",
            Icon::Check => "check",
            Icon::Plus => "plus",
            Icon::Minus => "minus",
            Icon::Search => "search",
            Icon::Settings => "settings",
            Icon::Edit => "edit",
            Icon::Trash => "trash",

            // Status/Alerts
            Icon::InfoCircle => "info-circle",
            Icon::CheckCircle => "check-circle",
            Icon::AlertCircle => "alert-circle",
            Icon::AlertTriangle => "alert-triangle",
            Icon::XCircle => "x-circle",

            // Content
            Icon::User => "user",
            Icon::Mail => "mail",
            Icon::Phone => "phone",
            Icon::Calendar => "calendar",
            Icon::Clock => "clock",
            Icon::File => "file",
            Icon::Folder => "folder",
            Icon::Image => "image",
            Icon::Link => "link",
            Icon::ExternalLink => "external-link",

            // UI
            Icon::Eye => "eye",
            Icon::EyeOff => "eye-off",
            Icon::Menu => "menu",
            Icon::MoreHorizontal => "more-horizontal",
            Icon::MoreVertical => "more-vertical",
            Icon::Loader => "loader",
            Icon::Quote => "quote",
        }
    }
}

impl std::fmt::Display for Icon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
