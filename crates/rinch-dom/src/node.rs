//! Node tree data structures for rinch-dom.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rinch_core::image::ImageLoader;

use crate::animation::types::{ActiveAnimation, AnimationSpec};
use crate::image_cache::ImageCache;
use crate::transition::{ActiveTransition, TransitionProperty, TransitionSpec};

use atomic_refcell::AtomicRefCell;
use bitflags::bitflags;
use peniko::Brush;
use peniko::color::{AlphaColor, Srgb};
use selectors::matching::ElementSelectorFlags;
use servo_arc::Arc as ServoArc;
use style::properties::PropertyDeclarationBlock;
use style::shared_lock::{Locked, SharedRwLock};

use crate::computed_style::ComputedStyle;

/// Raw node ID (index into slab).
pub type RawNodeId = usize;

bitflags! {
    /// Tracks what needs updating for a node.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct DirtyFlags: u8 {
        /// Needs style recomputation.
        const STYLE    = 0b0001;
        /// Needs layout.
        const LAYOUT   = 0b0010;
        /// Needs repaint.
        const PAINT    = 0b0100;
        /// Children changed (structural mutation).
        const CHILDREN = 0b1000;
    }
}

/// The type of a DOM node.
#[derive(Debug, Clone)]
pub enum NodeKind {
    /// Document root.
    Document,
    /// An HTML element (div, span, p, etc).
    Element(ElementData),
    /// A text node.
    Text(TextData),
    /// A comment node (used as marker/placeholder).
    Comment(String),
}

/// Data specific to element nodes.
#[derive(Debug, Clone)]
pub struct ElementData {
    /// Tag name (e.g., "div", "span", "button").
    pub tag: String,
}

/// Data specific to text nodes.
#[derive(Debug, Clone)]
pub struct TextData {
    /// Text content.
    pub content: String,
}

/// Context stored in Taffy nodes for measurement.
#[derive(Debug, Clone)]
pub enum NodeContext {
    /// Text content that needs Parley measurement.
    Text(TextMeasure),
    /// Element (no custom measurement needed).
    Element,
    /// Image element with intrinsic dimensions (0x0 while loading).
    Image {
        src: String,
        /// Intrinsic width (0 while loading).
        width: u32,
        /// Intrinsic height (0 while loading).
        height: u32,
    },
    /// IFC root that needs Parley TreeBuilder measurement.
    InlineRoot(usize), // stores the RawNodeId of the IFC root
}

/// Text measurement context for Parley.
///
/// All text-relevant CSS properties are stored here so that both the Taffy
/// measure callback and the paint code use identical parameters, preventing
/// layout/paint mismatches.
#[derive(Debug, Clone)]
pub struct TextMeasure {
    /// The text content to measure.
    pub content: String,
    /// Font size in pixels (inherited from parent).
    pub font_size: f32,
    /// Font weight (inherited from parent, default 400).
    pub font_weight: f32,
    /// Font family CSS value (inherited from parent).
    pub font_family: String,
    /// Raw CSS line-height value (e.g. "1.6", "24px", "normal").
    /// Empty means use font metrics default.
    pub line_height_css: String,
    /// DOM node ID for caching the layout after measurement.
    pub node_id: usize,
    /// Text color (inherited from parent).
    pub color: AlphaColor<Srgb>,
    /// Whether text wrapping is disabled (white-space: nowrap/pre).
    pub no_wrap: bool,
    /// Overflow wrap mode (controls emergency line-breaking).
    pub overflow_wrap: crate::computed_style::OverflowWrapValue,
    /// Text overflow mode (clip or ellipsis).
    pub text_overflow: crate::computed_style::TextOverflowValue,
    /// Whether parent has overflow: hidden (needed for text-overflow to apply).
    pub parent_overflow_hidden: bool,
}

/// Layout result for a node after Taffy computation.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LayoutResult {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// CSS display mode for inline layout detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayMode {
    /// Block-level element (div, p, etc.) — default for elements.
    #[default]
    Block,
    /// Flex container.
    Flex,
    /// Inline element (span, em, strong, etc.) — participates in IFC.
    Inline,
    /// Inline-block element — inline positioning but block-level content.
    InlineBlock,
}

/// Maps a range of bytes in the IFC flat text to a specific DOM node.
///
/// Built during `walk_inline_children()` so that IFC byte offsets can be
/// converted to/from `(node_id, offset_within_node)` pairs.
#[derive(Debug, Clone)]
pub struct IfcTextRange {
    /// Byte range start (inclusive) in the IFC `text_content`.
    pub flat_start: usize,
    /// Byte range end (exclusive) in the IFC `text_content`.
    pub flat_end: usize,
    /// DOM node ID: text node for text, `<br>` element for `<br>`.
    pub node_id: usize,
    /// Byte offset within the text node (0 for most; nonzero when a node
    /// is split across multiple style spans).
    pub node_offset: usize,
    /// True for `<br>` entries (which map to `"\n"` in the flat text).
    pub is_br: bool,
    /// Length of the original DOM text content (before tab expansion).
    /// When equal to `flat_end - flat_start`, no tabs were expanded.
    pub dom_text_len: usize,
    /// Original DOM text content (before tab expansion). Only set when
    /// tabs were expanded; empty string when no tabs are present.
    pub dom_text: String,
}

/// A background span for inline elements within an IFC.
///
/// Records the byte range in the flat IFC text that has a background color,
/// along with padding values for visual extension of the background rect.
pub struct InlineBackgroundSpan {
    /// Byte range start in the IFC `text_content`.
    pub start: usize,
    /// Byte range end (exclusive) in the IFC `text_content`.
    pub end: usize,
    /// Background color.
    pub color: peniko::Color,
    /// Padding left in pixels.
    pub padding_left: f32,
    /// Padding right in pixels.
    pub padding_right: f32,
    /// Padding top in pixels.
    pub padding_top: f32,
    /// Padding bottom in pixels.
    pub padding_bottom: f32,
    /// Border radius (top-left) in pixels.
    pub border_radius: f32,
}

/// Cached Parley inline layout for an IFC (Inline Formatting Context) root.
///
/// Stored on the IFC root element. Rebuilt when any inline child mutates.
pub struct InlineLayout {
    /// The Parley text layout covering all inline content.
    pub layout: parley::layout::Layout<Brush>,
    /// The concatenated text content that was laid out.
    pub text_content: String,
    /// Map from inline child RawNodeId → computed position within the layout.
    pub child_positions: Vec<(RawNodeId, LayoutResult)>,
    /// Map from IFC flat byte ranges to DOM text nodes / `<br>` elements.
    pub text_ranges: Vec<IfcTextRange>,
    /// Background spans for inline elements (code, mark, etc.).
    pub background_spans: Vec<InlineBackgroundSpan>,
    /// The max_width used to build this layout (for cache invalidation).
    pub max_width: f32,
}

impl std::fmt::Debug for InlineLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InlineLayout")
            .field("text_len", &self.text_content.len())
            .field("child_positions", &self.child_positions.len())
            .finish()
    }
}

/// A node in the DOM tree.
pub struct Node {
    /// This node's ID (its slab key).
    pub id: RawNodeId,
    /// What kind of node this is.
    pub kind: NodeKind,
    /// Parent node ID.
    pub parent: Option<RawNodeId>,
    /// Child node IDs in order.
    pub children: Vec<RawNodeId>,
    /// Attributes (name → value).
    pub attributes: HashMap<String, String>,
    /// Dirty flags for incremental updates.
    pub dirty: DirtyFlags,
    /// Scroll offset (x, y).
    pub scroll_offset: (f64, f64),
    /// Taffy layout node ID.
    pub taffy_id: Option<taffy::NodeId>,
    /// Computed layout result.
    pub layout: LayoutResult,
    /// Previous frame's layout (for dirty region computation).
    pub prev_layout: LayoutResult,
    /// CSS display mode (parsed from style attribute).
    pub display_mode: DisplayMode,
    /// If this node is an inline child, which IFC root owns it.
    /// Derived cache — cleared on any structural mutation.
    pub ifc_root: Option<RawNodeId>,
    /// Cached Parley inline layout (only set on IFC root nodes).
    /// Derived cache — cleared on any mutation to inline children.
    pub text_layout: Option<Box<InlineLayout>>,
    /// Cached Parley layout for standalone text nodes (not part of IFC).
    /// Built after Taffy layout using the computed width.
    pub cached_text_parley: Option<Box<parley::layout::Layout<Brush>>>,
    /// Cached computed style string (merged class + inline styles).
    /// Populated during style recomputation; used by inline text layout.
    pub computed_style_str: String,
    /// Whether this node is currently under the cursor (for CSS :hover).
    pub is_hovered: bool,
    /// Whether this node or an ancestor has focus (for CSS :focus).
    pub is_focused: bool,
    /// Whether this node is currently being pressed (for CSS :active).
    pub is_active: bool,
    /// Whether this is an anonymous block box created by the layout engine.
    /// These wrap runs of inline children in mixed-content block containers
    /// (CSS "anonymous block boxes"). Transparent to editing operations.
    pub is_anonymous_block_box: bool,
    /// Whether this node is a CSS pseudo-element (::before or ::after).
    /// Pseudo-element nodes are synthetic children created during style resolution
    /// and are cleaned up before re-resolution to avoid duplicates.
    pub is_pseudo_element: bool,
    /// Typed computed style derived from Stylo.
    /// This is the primary source for layout and paint after Stylo migration.
    pub computed_style: ComputedStyle,
    /// Parsed CSS transition specs for this node.
    pub transition_specs: Vec<TransitionSpec>,
    /// Parsed CSS animation specs for this node.
    pub animation_specs: Vec<AnimationSpec>,
    /// Whether this node has been styled at least once.
    /// Prevents transitions from firing on initial style application.
    pub has_been_styled: bool,

    // === Stylo CSS engine fields ===
    /// Stylo element data containing computed CSS values.
    /// This is the primary source of truth for CSS after Stylo migration.
    pub stylo_element_data: AtomicRefCell<Option<style::data::ElementData>>,
    /// Selector flags set during CSS matching.
    pub selector_flags: AtomicRefCell<ElementSelectorFlags>,
    /// Whether this node has a snapshot for animation/transition.
    pub has_snapshot: bool,
    /// Whether the snapshot has been handled.
    pub snapshot_handled: AtomicBool,
    /// Shared lock for Stylo (reference to document's lock).
    pub guard: SharedRwLock,
    /// Cached parsed inline style attribute (Stylo PropertyDeclarationBlock).
    /// Populated when style attribute is set, used by Stylo for cascade.
    pub style_attribute_cache: Option<ServoArc<Locked<PropertyDeclarationBlock>>>,
    /// Set during Stylo selector matching when a `:hover` pseudo-class is
    /// evaluated against this node. Nodes without this flag can skip style
    /// invalidation on hover changes because no CSS rule depends on their
    /// hover state. Cleared before each style re-resolution so stale flags
    /// don't persist after class changes.
    pub hover_sensitive: Cell<bool>,
    /// Set by Stylo when `:active` is evaluated during selector matching.
    pub active_sensitive: Cell<bool>,
    /// Set by Stylo when `:focus` is evaluated during selector matching.
    pub focus_sensitive: Cell<bool>,

    /// When set, this block uses a fixed estimated height in Taffy instead of
    /// measuring via Parley. Used by contenteditable block virtualization to
    /// collapse off-screen blocks. IFC building and painting are skipped.
    pub estimated_height: Option<f32>,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("parent", &self.parent)
            .field("children", &self.children)
            .field("attributes", &self.attributes)
            .field("dirty", &self.dirty)
            .field("layout", &self.layout)
            .field("display_mode", &self.display_mode)
            .field("is_hovered", &self.is_hovered)
            .field("has_snapshot", &self.has_snapshot)
            // Skip non-Debug fields: stylo_element_data, selector_flags, snapshot_handled, guard
            .finish_non_exhaustive()
    }
}

impl Node {
    /// Create a new document root node.
    pub fn document(id: RawNodeId, guard: SharedRwLock) -> Self {
        Self {
            id,
            kind: NodeKind::Document,
            parent: None,
            children: Vec::new(),
            attributes: HashMap::new(),
            dirty: DirtyFlags::empty(),
            scroll_offset: (0.0, 0.0),
            taffy_id: None,
            layout: LayoutResult::default(),
            prev_layout: LayoutResult::default(),
            display_mode: DisplayMode::Block,
            ifc_root: None,
            text_layout: None,
            cached_text_parley: None,
            computed_style_str: String::new(),
            is_hovered: false,
            is_focused: false,
            is_active: false,
            is_anonymous_block_box: false,
            is_pseudo_element: false,
            computed_style: ComputedStyle::default(),
            transition_specs: Vec::new(),
            animation_specs: Vec::new(),
            has_been_styled: false,
            // Stylo fields
            stylo_element_data: AtomicRefCell::new(None),
            selector_flags: AtomicRefCell::new(ElementSelectorFlags::empty()),
            has_snapshot: false,
            snapshot_handled: AtomicBool::new(false),
            guard,
            style_attribute_cache: None,
            hover_sensitive: Cell::new(false),
            active_sensitive: Cell::new(false),
            focus_sensitive: Cell::new(false),
            estimated_height: None,
        }
    }

    /// Create a new element node.
    pub fn element(id: RawNodeId, tag: &str, guard: SharedRwLock) -> Self {
        let display_mode = default_display_for_tag(tag);
        Self {
            id,
            kind: NodeKind::Element(ElementData {
                tag: tag.to_string(),
            }),
            parent: None,
            children: Vec::new(),
            attributes: HashMap::new(),
            dirty: DirtyFlags::STYLE | DirtyFlags::LAYOUT,
            scroll_offset: (0.0, 0.0),
            taffy_id: None,
            layout: LayoutResult::default(),
            prev_layout: LayoutResult::default(),
            display_mode,
            ifc_root: None,
            text_layout: None,
            cached_text_parley: None,
            computed_style_str: String::new(),
            is_hovered: false,
            is_focused: false,
            is_active: false,
            is_anonymous_block_box: false,
            is_pseudo_element: false,
            computed_style: ComputedStyle::default(),
            transition_specs: Vec::new(),
            animation_specs: Vec::new(),
            has_been_styled: false,
            // Stylo fields
            stylo_element_data: AtomicRefCell::new(None),
            selector_flags: AtomicRefCell::new(ElementSelectorFlags::empty()),
            has_snapshot: false,
            snapshot_handled: AtomicBool::new(false),
            guard,
            style_attribute_cache: None,
            hover_sensitive: Cell::new(false),
            active_sensitive: Cell::new(false),
            focus_sensitive: Cell::new(false),
            estimated_height: None,
        }
    }

    /// Create a new text node.
    pub fn text(id: RawNodeId, content: &str, guard: SharedRwLock) -> Self {
        Self {
            id,
            kind: NodeKind::Text(TextData {
                content: content.to_string(),
            }),
            parent: None,
            children: Vec::new(),
            attributes: HashMap::new(),
            dirty: DirtyFlags::LAYOUT,
            scroll_offset: (0.0, 0.0),
            taffy_id: None,
            layout: LayoutResult::default(),
            prev_layout: LayoutResult::default(),
            display_mode: DisplayMode::Inline,
            ifc_root: None,
            text_layout: None,
            cached_text_parley: None,
            computed_style_str: String::new(),
            is_hovered: false,
            is_focused: false,
            is_active: false,
            is_anonymous_block_box: false,
            is_pseudo_element: false,
            computed_style: ComputedStyle::default(),
            transition_specs: Vec::new(),
            animation_specs: Vec::new(),
            has_been_styled: false,
            // Stylo fields
            stylo_element_data: AtomicRefCell::new(None),
            selector_flags: AtomicRefCell::new(ElementSelectorFlags::empty()),
            has_snapshot: false,
            snapshot_handled: AtomicBool::new(false),
            guard,
            style_attribute_cache: None,
            hover_sensitive: Cell::new(false),
            active_sensitive: Cell::new(false),
            focus_sensitive: Cell::new(false),
            estimated_height: None,
        }
    }

    /// Create a new comment node.
    pub fn comment(id: RawNodeId, text: &str, guard: SharedRwLock) -> Self {
        Self {
            id,
            kind: NodeKind::Comment(text.to_string()),
            parent: None,
            children: Vec::new(),
            attributes: HashMap::new(),
            dirty: DirtyFlags::empty(),
            scroll_offset: (0.0, 0.0),
            taffy_id: None,
            layout: LayoutResult::default(),
            prev_layout: LayoutResult::default(),
            display_mode: DisplayMode::Inline,
            ifc_root: None,
            text_layout: None,
            cached_text_parley: None,
            computed_style_str: String::new(),
            is_hovered: false,
            is_focused: false,
            is_active: false,
            is_anonymous_block_box: false,
            is_pseudo_element: false,
            computed_style: ComputedStyle::default(),
            transition_specs: Vec::new(),
            animation_specs: Vec::new(),
            has_been_styled: false,
            // Stylo fields
            stylo_element_data: AtomicRefCell::new(None),
            selector_flags: AtomicRefCell::new(ElementSelectorFlags::empty()),
            has_snapshot: false,
            snapshot_handled: AtomicBool::new(false),
            guard,
            style_attribute_cache: None,
            hover_sensitive: Cell::new(false),
            active_sensitive: Cell::new(false),
            focus_sensitive: Cell::new(false),
            estimated_height: None,
        }
    }

    /// Whether this is an element node.
    pub fn is_element(&self) -> bool {
        matches!(self.kind, NodeKind::Element(_))
    }

    /// Whether this is a text node.
    pub fn is_text(&self) -> bool {
        matches!(self.kind, NodeKind::Text(_))
    }

    /// Get the tag name if this is an element.
    pub fn tag(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Element(el) => Some(&el.tag),
            _ => None,
        }
    }

    /// Check whether this node creates a CSS stacking context.
    ///
    /// A stacking context is formed when any of:
    /// - `position` is not `static` AND `z-index` is explicitly set (not `auto`)
    /// - `opacity < 1.0`
    /// - `transform` is non-identity
    pub fn creates_stacking_context(&self) -> bool {
        use crate::computed_style::{OverflowValue, PositionValue};

        if !matches!(self.computed_style.position, PositionValue::Static)
            && self.computed_style.z_index.is_some()
        {
            return true;
        }
        if self.computed_style.opacity < 1.0 {
            return true;
        }
        if !self.computed_style.transform.is_identity {
            return true;
        }
        // Overflow clipping must create a stacking context so that descendant
        // SCs (e.g. z-indexed children) are collected and painted within the
        // clip layer rather than escaping to an ancestor SC.
        if matches!(
            self.computed_style.overflow_y,
            OverflowValue::Hidden | OverflowValue::Scroll | OverflowValue::Auto
        ) {
            return true;
        }
        false
    }

    /// Get the text content if this is a text node.
    pub fn text_content(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Text(t) => Some(&t.content),
            _ => None,
        }
    }

    /// Whether this node participates in inline flow (text, inline elements).
    pub fn is_inline(&self) -> bool {
        match &self.kind {
            NodeKind::Text(_) => true,
            NodeKind::Comment(_) => true, // comments are invisible but inline
            NodeKind::Element(_) => matches!(
                self.display_mode,
                DisplayMode::Inline | DisplayMode::InlineBlock
            ),
            _ => false,
        }
    }

    /// Whether this node is a comment node.
    pub fn is_comment(&self) -> bool {
        matches!(&self.kind, NodeKind::Comment(_))
    }
}

/// Default display mode based on HTML tag name.
fn default_display_for_tag(tag: &str) -> DisplayMode {
    match tag {
        "span" | "a" | "em" | "strong" | "b" | "i" | "u" | "s" | "sub" | "sup" | "small"
        | "mark" | "abbr" | "cite" | "code" | "kbd" | "samp" | "var" | "q" | "dfn" | "time"
        | "label" | "br" | "wbr" => DisplayMode::Inline,
        "img" | "input" | "button" | "select" | "textarea" => DisplayMode::InlineBlock,
        _ => DisplayMode::Block,
    }
}

/// The node tree, stored in a slab for stable IDs.
pub struct NodeTree {
    /// All nodes, indexed by RawNodeId.
    pub nodes: slab::Slab<Node>,
    /// The root document node ID.
    pub root_id: RawNodeId,
    /// The html element node ID.
    pub html_id: RawNodeId,
    /// The body element node ID.
    pub body_id: RawNodeId,
    /// IDs of nodes that have been mutated since last take_dirty_nodes.
    pub dirty_nodes: HashSet<RawNodeId>,
    /// IDs of nodes that need repainting. Persists across layout resolve
    /// until consumed by the paint phase for dirty region computation.
    pub paint_dirty_nodes: Vec<RawNodeId>,
    /// Absolute rects of removed nodes whose areas need repainting.
    /// Stored at removal time because the nodes are deleted from the tree
    /// before `compute_dirty_region` runs.
    pub paint_dirty_removed_rects: Vec<(f64, f64, f64, f64)>,
    /// IDs of nodes whose styles were recomputed and need Taffy sync.
    pub style_dirty_nodes: Vec<RawNodeId>,
    /// Roots of subtrees needing style resolution. When non-empty,
    /// `resolve_styles()` resolves only these subtrees instead of the
    /// full tree — turning O(tree) into O(changed_subtree).
    pub style_roots: Vec<RawNodeId>,
    /// True if any style-affecting change occurred since last resolve.
    pub styles_dirty: bool,
    /// True if any layout-affecting Taffy style changed since last compute.
    /// When false, `resolve_layout()` can skip Taffy compute + IFC rebuild.
    pub layout_dirty: bool,
    /// When true, `append_child` skips inline `recompute_node_styles_recursive`.
    /// Used during bulk DOM operations (block re-render) to batch style resolution
    /// into a single pass instead of one per child.
    pub suppress_inline_restyle: bool,
    /// True when an absolute/fixed element moved via the inset fast path.
    /// The app checks this to force a full scene repaint (clearing old position).
    pub full_repaint_needed: bool,
    /// True if the tree structure changed (node insert/remove) or display mode
    /// changed since last IFC setup. When false, IFC rebuild is skipped and
    /// Taffy's internal cache is preserved — only dirty nodes get re-measured.
    pub ifc_dirty: bool,
    /// Taffy layout tree.
    pub taffy: taffy::TaffyTree<NodeContext>,
    /// Reverse map from Taffy node ID to slab node ID.
    pub taffy_map: HashMap<taffy::NodeId, RawNodeId>,
    /// Viewport dimensions for resolving vh/vw CSS units.
    pub viewport: crate::layout::Viewport,
    /// Currently hovered node ID (for CSS :hover).
    pub hovered_node: Option<RawNodeId>,
    /// Currently focused node ID (for CSS :focus).
    pub focused_node: Option<RawNodeId>,
    /// Currently active (mouse-pressed) node ID (for CSS :active).
    pub active_node: Option<RawNodeId>,
    /// Shared lock for Stylo CSS engine.
    pub guard: SharedRwLock,
    /// IDs of anonymous block box nodes created during layout.
    /// Tracked for cleanup at the start of each layout pass.
    pub anonymous_block_boxes: Vec<RawNodeId>,
    /// Active CSS transitions per node, keyed by property.
    pub active_transitions: HashMap<RawNodeId, HashMap<TransitionProperty, ActiveTransition>>,
    /// Active CSS animations per node.
    pub active_animations: HashMap<RawNodeId, Vec<ActiveAnimation>>,
    /// Whether transitions are enabled (false until first layout completes).
    pub transitions_enabled: bool,
    /// Cache of loaded and decoded images.
    pub image_cache: ImageCache,
    /// Image loader for fetching image data (file, network, etc.).
    pub image_loader: Option<Arc<dyn ImageLoader>>,
    /// IFC roots whose text content changed since last layout.
    /// Used to skip expensive Parley rebuilds for unchanged IFC roots.
    pub dirty_ifc_text_roots: HashSet<RawNodeId>,
    /// Cached IFC measure results from previous frames.
    /// Key: (ifc_root_node_id, wrap_width_bits) → (width, height).
    /// Invalidated per-root when text content changes.
    pub ifc_measure_cache: HashMap<(RawNodeId, u32), (f32, f32)>,
    /// Nodes that have requested scroll-into-view (deferred until after layout).
    pub scroll_into_view_requests: Vec<RawNodeId>,
}

impl Default for NodeTree {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeTree {
    /// Create a new node tree with root and body nodes.
    pub fn new() -> Self {
        let mut nodes = slab::Slab::new();
        let mut taffy = taffy::TaffyTree::new();
        let mut taffy_map = HashMap::new();

        // Create shared lock for Stylo CSS engine
        let guard = SharedRwLock::new();

        // Create root (document) node
        let root_id = nodes.vacant_key();
        let mut root = Node::document(root_id, guard.clone());
        let root_taffy = taffy
            .new_leaf(taffy::Style {
                display: taffy::Display::Flex,
                flex_direction: taffy::FlexDirection::Column,
                size: taffy::Size {
                    width: taffy::Dimension::percent(1.0),
                    height: taffy::Dimension::percent(1.0),
                },
                ..Default::default()
            })
            .unwrap();
        root.taffy_id = Some(root_taffy);
        taffy_map.insert(root_taffy, root_id);
        nodes.insert(root);

        // Create html element
        let html_id = nodes.vacant_key();
        let mut html = Node::element(html_id, "html", guard.clone());
        html.parent = Some(root_id);
        let html_taffy = taffy
            .new_leaf(taffy::Style {
                display: taffy::Display::Flex,
                flex_direction: taffy::FlexDirection::Column,
                size: taffy::Size {
                    width: taffy::Dimension::percent(1.0),
                    height: taffy::Dimension::percent(1.0),
                },
                ..Default::default()
            })
            .unwrap();
        html.taffy_id = Some(html_taffy);
        taffy_map.insert(html_taffy, html_id);
        taffy.add_child(root_taffy, html_taffy).unwrap();
        nodes.insert(html);
        nodes[root_id].children.push(html_id);

        // Create body element
        let body_id = nodes.vacant_key();
        let mut body = Node::element(body_id, "body", guard.clone());
        body.parent = Some(html_id);
        let body_taffy = taffy
            .new_leaf(taffy::Style {
                display: taffy::Display::Flex,
                flex_direction: taffy::FlexDirection::Column,
                size: taffy::Size {
                    width: taffy::Dimension::percent(1.0),
                    height: taffy::Dimension::auto(),
                },
                flex_grow: 1.0,
                ..Default::default()
            })
            .unwrap();
        body.taffy_id = Some(body_taffy);
        taffy_map.insert(body_taffy, body_id);
        taffy.add_child(html_taffy, body_taffy).unwrap();
        nodes.insert(body);
        nodes[html_id].children.push(body_id);

        Self {
            nodes,
            root_id,
            html_id,
            body_id,
            dirty_nodes: HashSet::new(),
            paint_dirty_nodes: Vec::new(),
            paint_dirty_removed_rects: Vec::new(),
            style_dirty_nodes: Vec::new(),
            style_roots: Vec::new(),
            styles_dirty: true, // Initial render needs styles
            layout_dirty: true, // Initial render needs layout
            suppress_inline_restyle: false,
            full_repaint_needed: false,
            ifc_dirty: true, // Initial render needs IFC setup
            taffy,
            taffy_map,
            viewport: crate::layout::Viewport::default(),
            hovered_node: None,
            focused_node: None,
            active_node: None,
            guard,
            anonymous_block_boxes: Vec::new(),
            active_transitions: HashMap::new(),
            active_animations: HashMap::new(),
            transitions_enabled: false,
            image_cache: ImageCache::new(),
            image_loader: None,
            dirty_ifc_text_roots: HashSet::new(),
            ifc_measure_cache: HashMap::new(),
            scroll_into_view_requests: Vec::new(),
        }
    }

    /// Check if a node ID is valid.
    pub fn contains(&self, id: RawNodeId) -> bool {
        self.nodes.contains(id)
    }

    /// Get a reference to a node.
    pub fn get(&self, id: RawNodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Get a mutable reference to a node.
    pub fn get_mut(&mut self, id: RawNodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    /// Push a node ID to the dirty list (deduplicated).
    pub fn push_dirty(&mut self, id: RawNodeId) {
        self.dirty_nodes.insert(id);
        self.paint_dirty_nodes.push(id);
    }

    /// Remove a node and all its descendants from the slab.
    pub fn remove_subtree(&mut self, id: RawNodeId) {
        // Collect all descendant IDs first
        let mut to_remove = Vec::new();
        self.collect_descendants(id, &mut to_remove);
        for node_id in &to_remove {
            self.active_transitions.remove(node_id);
            self.active_animations.remove(node_id);
        }
        for node_id in to_remove {
            self.nodes.remove(node_id);
        }
    }

    fn collect_descendants(&self, id: RawNodeId, out: &mut Vec<RawNodeId>) {
        out.push(id);
        if let Some(node) = self.nodes.get(id) {
            let children: Vec<_> = node.children.clone();
            for child in children {
                self.collect_descendants(child, out);
            }
        }
    }
}
