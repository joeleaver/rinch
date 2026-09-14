//! Node tree data structures for rinch-dom.

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
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
use style::Atom;
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
    ///
    /// **The IFC leaf invariant (#466).** Taffy 0.12 consults a measure
    /// function only on a node with zero children (`taffy_tree.rs:303-327`,
    /// the `(_, false)` arm of the `match (display_mode, has_children)`
    /// dispatch). Therefore the Taffy node carrying a live `InlineRoot` must
    /// be childless: the IFC root's own node when inline detachment emptied
    /// it, or its dedicated measure-leaf when out-of-flow children remain
    /// attached. After `setup_inline_formatting_contexts`, no Taffy node with
    /// children carries `InlineRoot`, and every root discovered this pass has
    /// its context on exactly one childless node.
    ///
    /// A non-leaf carrying this context does not merely measure wrong — the
    /// measure is *structurally unreachable* (Taffy runs the block algorithm
    /// instead), so an auto-height IFC root collapses to `h = 0`: the block
    /// algorithm sums in-flow children, of which a root whose inline content
    /// was detached has none. Block virtualization depends on the same
    /// invariant — `estimated_height`'s early return lives *inside* the
    /// measure closure (`layout_engine.rs`), so a non-leaf virtualized root
    /// would silently report 0 instead of its estimate.
    ///
    /// `setup_inline_formatting_contexts` enforces this: it sweeps the stale
    /// context off any non-leaf not (re)marked a root this pass, and a
    /// `debug_assertions` validator
    /// ([`crate::RinchDocument::ifc_leaf_invariant_violations`]) checks the
    /// invariant after every setup pass.
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
///
/// `x`/`y` are the node's border-box origin relative to its **parent's border
/// box** (Taffy's `Layout.location`: an inset is resolved against the
/// containing block's padding box and the parent's border is added back), so
/// an absolutely-positioned child with `left: 0` inside a parent with a `5px`
/// left border has `x == 5`. The one exception is `position: fixed`, which
/// `read_layout_results` rewrites to be **viewport**-relative with no border
/// or margin applied (CSS puts a fixed box's *margin* edge at its inset, so
/// ignoring the margin is a known deviation of that override, not of Taffy).
/// Every consumer — paint, hit testing,
/// `compute_absolute_position` — accumulates this field up the parent chain,
/// so only Taffy (or that fixed override) may write it (#236).
///
/// An absolutely positioned box with no positioned ancestor is corrected too
/// (#204), but *without* leaving this space: `read_layout_results` writes the
/// parent-relative **delta** that lands it on the initial containing block, so
/// the field stays parent-relative and every consumer above keeps working
/// unchanged. See `crate::out_of_flow`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LayoutResult {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// How a box participates in the formatting context around it — a
/// **coarsening** of `display`, not a copy of it.
///
/// [`crate::computed_style::ComputedStyle::display`] is the authority on the
/// declared value; this enum answers the two questions the layout passes
/// actually ask (`is_inline_level`, `is_block_container`), and it answers them
/// for several `DisplayValue`s at once. `none`, `contents` and block-level
/// `grid` all arrive here as [`DisplayMode::Block`], so a site that needs to
/// tell those apart reads `computed_style.display` instead — `ifc.rs` has
/// several such guards for `Contents`, spelled that way for exactly this
/// reason.
///
/// **Every variant is distinguished only where a layout pass branches on it.**
/// Adding one is therefore a change to the three predicates below and nothing
/// else — none of the `matches!`/`==` sites in the crate is exhaustive, so the
/// compiler will not find them for you; they all read a predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayMode {
    /// Block-level element (div, p, etc.) — default for elements. Also where
    /// block-level `display: grid`, `contents` and `none` land (see the type's
    /// doc). `inline-grid` does **not** land here — it is
    /// [`DisplayMode::InlineGrid`] (#607).
    #[default]
    Block,
    /// Block-level flex container (`display: flex`).
    Flex,
    /// Inline element (span, em, strong, etc.) — participates in IFC.
    Inline,
    /// Inline-block element — inline-level, with a block container inside.
    InlineBlock,
    /// `display: inline-flex` — inline-level, with a flex container inside
    /// (#595).
    ///
    /// Told apart from [`DisplayMode::Flex`] because the *outside* differs:
    /// an `inline-flex` box joins the line around it instead of ending it, and
    /// shrink-wraps rather than filling its container. Told apart from
    /// [`DisplayMode::InlineBlock`] by exactly **one** predicate below —
    /// [`Self::is_block_container`], which admits `inline-block` and not this,
    /// because an `inline-block`'s inside is a block container and an
    /// `inline-flex`'s is a flex container (#592). That was no predicate at all
    /// until #592: the two used to answer everything here identically and their
    /// insides were distinguished only by `DisplayValue::to_taffy`, which is
    /// what builds the Taffy style. Kept distinct also so `display_mode` does not report
    /// `InlineBlock` for a box whose `display` is `inline-flex` (it is dumped
    /// to the MCP **`get_node`** tool — `testing::get_node_detail` is the one
    /// place `display_mode` is serialized; `dom_tree` goes through
    /// `serialize_tree_full` and carries `computed_styles` only), and so a
    /// future consumer that does care has something to read.
    InlineFlex,
    /// `display: inline-grid` — inline-level, with a grid container inside
    /// (#607).
    ///
    /// The third atomic inline, and it reached this enum later than the other
    /// two for a reason worth keeping: the value used to be folded into
    /// [`crate::computed_style::values::DisplayValue::Grid`] by
    /// `display_from_stylo`, so there was nothing left for
    /// `style_resolution` to classify — `inline-grid` and `grid` arrived here
    /// as the same `Block`, and a separate `DisplayMode` variant could not be
    /// reached at all until `DisplayValue` grew an `InlineGrid` of its own.
    ///
    /// Answers every predicate below exactly as [`DisplayMode::InlineFlex`]
    /// and [`DisplayMode::InlineBlock`] do; the *inside* that tells the three
    /// apart comes from `DisplayValue::to_taffy`. Kept distinct for the same
    /// reason `InlineFlex` is: `display_mode` is dumped verbatim to the MCP
    /// **`get_node`** tool (`testing::get_node_detail`), so folding it into
    /// another variant would make that surface report a `display` the node does
    /// not have.
    InlineGrid,
}

impl DisplayMode {
    /// Whether a box in this mode is **inline-level** — content an inline
    /// formatting context lays out, rather than a box its container's block
    /// formatting context does.
    ///
    /// The one authority for that question. [`Node::is_inline`] answers it for
    /// a node (adding the text/comment cases), and `apply_to_taffy` compares it
    /// across a restyle to decide whether the IFC pass has to run again — a
    /// crossing the Taffy style cannot be asked about, because
    /// [`crate::computed_style::values::DisplayValue::to_taffy`] is not
    /// injective: `inline`, `block` and — since #592 — `inline-block` all map
    /// to `taffy::Display::Block`, `flex`, `inline-flex` and `contents` all map
    /// to `taffy::Display::Flex` (#597), and `grid` and `inline-grid` both map
    /// to `taffy::Display::Grid` (#607) — each pair differing only in its
    /// *outside*, which is the half this enum carries and the Taffy style does
    /// not.
    ///
    /// **This predicate is no longer what the `ifc_dirty` trigger asks**, and
    /// #592 is why: `inline ↔ inline-block` is inline-level on both sides *and*
    /// equal on every Taffy field, so neither test could see it. That trigger
    /// compares the whole `DisplayMode` now.
    pub fn is_inline_level(self) -> bool {
        matches!(self, DisplayMode::Inline) || self.is_atomic_inline()
    }

    /// Whether this is an **atomic inline-level box** (css-display-3 §2.6): a
    /// box the surrounding IFC only *measures and places*, whose interior is an
    /// independent formatting context laid out and painted by Taffy.
    ///
    /// `inline-block`, `inline-flex` and `inline-grid`. The one authority for
    /// the question five passes ask of a node they found in an IFC — "is this a
    /// box I should measure standalone, keep the IFC's position for, and bridge
    /// from the root's content box?": `inline_block_measure_roots`,
    /// `resolve_percentage_inline_blocks`, `read_layout_results`,
    /// [`crate::paint::ifc_content_box_offset`] and `layer_bounds`' inline-box
    /// gate. They spelled it `== InlineBlock`, which is how `inline-flex`
    /// managed to be inline-level in one pass and not in the next.
    ///
    /// Adding `inline-grid` here (#607) is **half** of its flow fix — the half
    /// those five passes read. The other half is that `DisplayMode::InlineGrid`
    /// is not `Block`, so [`Self::is_block_container`] answers `false` and the
    /// anonymous-box generation — that predicate's only four readers, all in
    /// `ifc.rs`; grep it — stops treating the box as a container. The two
    /// predicates are **independently** load-bearing: a mutant that admits
    /// `InlineGrid` into `is_block_container` while leaving this function alone
    /// still breaks
    /// `an_inline_grid_box_joins_the_line_exactly_as_an_inline_block_does`, so
    /// neither line can be described as the whole fix.
    ///
    /// **Not** the same question as [`Self::is_inline_level`]: a
    /// `display: inline` box is inline-level and *not* atomic — the IFC walks
    /// into it and lays its text out as part of the same line.
    ///
    /// **Nor the complement of [`Self::is_block_container`] any more** (#592).
    /// `inline-block` answers `true` to both: atomic on the outside, a block
    /// container on the inside. The two predicates ask about opposite halves of
    /// the box and the intersection is exactly that one variant, which is why
    /// `apply_empty_block_line_floor` — the one caller that means "block-level
    /// block container" — has to say `is_block_container() && !is_atomic_inline()`
    /// rather than either alone.
    pub fn is_atomic_inline(self) -> bool {
        matches!(
            self,
            DisplayMode::InlineBlock | DisplayMode::InlineFlex | DisplayMode::InlineGrid
        )
    }

    /// Whether a box in this mode lays its children out as a **block
    /// container** — so it can establish an inline formatting context of its
    /// own and mint anonymous block boxes around runs of inline children.
    ///
    /// **The one predicate here that asks about the box's INSIDE**, where the
    /// rest of this enum answers about its outside. That is why
    /// [`DisplayMode::InlineBlock`] is in the set (#592) and
    /// [`DisplayMode::InlineFlex`] / [`DisplayMode::InlineGrid`] are not: all
    /// three are inline-level boxes, and only the first is a block container
    /// inside (css-display-3 §2.5). It used to be spelled as the complement of
    /// "inline-level or a flex container" at the four IFC sites that ask it,
    /// which is how `inline-block` came to be excluded: an
    /// `inline-block` with mixed content laid its children out in a row and
    /// generated no anonymous boxes at all, and — because it was therefore
    /// never an IFC root — measured its own box with the *ancestor* IFC's text
    /// style (#625) and left its inner `display: inline` element unmarked
    /// (#630).
    ///
    /// **`is_block_container` and [`Self::is_atomic_inline`] are not
    /// complements.** They intersect at `inline-block`. A caller that means
    /// "block-level block container" — `apply_empty_block_line_floor`, the one
    /// site of the four that is a rinch divergence rather than a CSS rule —
    /// must say `is_block_container() && !is_atomic_inline()`; the floor
    /// applied to a childless `inline-block` made every source-less `<img>` one
    /// line tall, `<img>` being an `inline-block` in the UA sheet.
    ///
    /// It answers from this enum alone, so it inherits the coarsening in the
    /// type's doc: block-level `display: grid` arrives as
    /// [`DisplayMode::Block`] and gets `true` here, which is wrong about grid
    /// and has been since before #595 (one doc comment in `layout_engine.rs`
    /// says so too — grep it for `display: grid` rather than trusting a line
    /// number, which is how this pointer went stale once already);
    /// `display: contents` and
    /// `display: none` get `true` too, and every caller guards those from
    /// `computed_style.display` separately. `inline-grid` is **not** among them
    /// since #607 — it is [`DisplayMode::InlineGrid`] and answers `false`.
    pub fn is_block_container(self) -> bool {
        matches!(self, DisplayMode::Block | DisplayMode::InlineBlock)
    }
}

/// How a box participates in its parent's inline formatting context.
///
/// Returned by [`Node::inline_flow_role`], the one classifier every IFC
/// decision consumes (#366) — see its doc for the precedence contract.
///
/// Whether a [`InlineFlowRole::Contents`] wrapper is *transparent* to the
/// surrounding IFC is deliberately a separate, recursive question
/// (`contents_is_inline_transparent` in `ifc.rs`): answering it eagerly here
/// would scan the wrapper's subtree at every classification, and the
/// recursive consumers (the contents scan and collector) would re-scan every
/// level of a nested-wrapper chain once per ancestor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineFlowRole {
    /// A comment node: invisible and boxless. It flows *with* inline content
    /// — it must neither split a run nor break the inline walk — but it is
    /// not inline content itself: it renders nothing, has no Taffy node, and
    /// must not establish an IFC on its own (#490).
    Comment,
    /// `display: none` — no box at all. Neither inline content nor a block:
    /// it cannot force anonymous-box generation, split a run, or break the
    /// inline flow.
    NoBox,
    /// `display: contents` — no box of its own; its children flatten into
    /// the parent. Whether the wrapper is transparent to the surrounding IFC
    /// (wraps no in-flow block-level box) is the separate recursive question
    /// above.
    Contents,
    /// Inline-level content: a text node, or an element with display
    /// `inline` / `inline-block` / `inline-flex`.
    Inline,
    /// An out-of-flow box — `position: absolute`/`fixed` (CSS 2.1 §9.3).
    /// Not inline content, yet it neither forces anonymous boxes (§9.2.1.1)
    /// nor breaks an inline formatting context (§9.4.2): inline siblings
    /// carry on across it, on the same line.
    OutOfFlow,
    /// An in-flow block-level box — the one thing that breaks the inline
    /// flow.
    InFlowBlock,
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
    ///
    /// Read freely. Any write of the **`id`** key must go through
    /// [`Node::write_attribute`] / [`Node::erase_attribute`], which keep
    /// [`Node::id_atom`] in step. Other keys may be written directly and
    /// several sites under `crates/rinch/src/app/` do (`value`, `data-preedit`,
    /// the `data-text-sel*` trio, …); none of them writes `id`.
    pub attributes: HashMap<String, String>,
    /// The `id` attribute, interned (#675).
    ///
    /// Stylo's `SelectorMap` files a rule whose rightmost compound carries an
    /// `#id` into the **id bucket only** — `Bucket::ID` writes `id_hash`,
    /// `Bucket::Universal` writes `other`, and `find_bucket` keeps exactly one.
    /// `get_all_matching_rules` then consults that bucket behind
    /// `if let Some(id) = rule_hash_target.id() { … }`, with no `else` and no
    /// fallback, so an element whose `TElement::id()` answers `None` is never
    /// offered a single id-keyed rule. `has_id` is the *predicate*, reached
    /// only after some bucket hands the rule over — so it was implemented
    /// correctly the whole time and never ran **for an id-bucketed rule**.
    ///
    /// An id on the *ancestor* side is a different rule and always worked:
    /// `#a > p` is bucketed by the rightmost compound, `p`, so it was offered
    /// to every `<p>` whatever `id()` answered, and `has_id` then ran on the
    /// parent and matched. That asymmetry is why the bug was hard to see, and
    /// `tests/id_selector_tests.rs` labels its `#a > p` assertion as the fixed
    /// point it is.
    ///
    /// `id()` returns a **reference**, so the atom has to be stored — this is
    /// the one attribute that cannot be interned per call the way
    /// `each_class` interns classes. Three consumers read it:
    /// `selector_map`'s bucket lookup, `bloom.rs`'s
    /// `each_relevant_element_hash`, and stylo's id-based invalidation.
    ///
    /// **Invariant:** `id_atom == attributes.get("id").map(Atom::from)`.
    /// [`Node::write_attribute`] and [`Node::erase_attribute`] are the only two
    /// places that write the `id` key, and they maintain it. That is a fact
    /// about today's call sites, not something the type system holds —
    /// `attributes` is `pub` — so [`Node::id_atom`] debug-asserts the invariant
    /// instead: a direct write of `id` fails loudly in every debug test rather
    /// than silently un-fixing #675. The assert is compiled out in release.
    id_atom: Option<Atom>,
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
    /// A past `mark_inline_descendants` pass removed this node's Taffy node
    /// from the list that should hold it — the **departure record** for the IFC
    /// detach (#597), and the exact counterpart of [`Self::contents_spliced`]
    /// for the `display: contents` splice (#520).
    ///
    /// The detach itself is correct: inline content is laid out by Parley and
    /// drawn by its IFC root, so it must not also be a Taffy child. What was
    /// missing is the other direction. Nothing put a box back when the reason
    /// for taking it out went away, so an element restyled from inline-level to
    /// block-level at runtime ended with no Taffy parent, its parent's child
    /// list empty, and whatever box it had while it was inline — laid out by
    /// nobody, and (its `ifc_root` having been cleared) drawn by nobody either.
    ///
    /// Read by `reattach_departed_ifc_children`, which rebuilds the departed
    /// node's effective Taffy parent's child list. Set only where a
    /// `remove_child` actually happened, so a node this pass never held is not
    /// claimed as departed.
    pub ifc_detached: bool,
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
    /// Whether this node is the focused node (for CSS :focus). Set on exactly
    /// one node — `update_focus` never propagates it to ancestors (unlike
    /// `update_active`).
    pub is_focused: bool,
    /// Whether focus arrived via the keyboard (for CSS :focus-visible).
    /// Only ever set on the focused node: Tab-driven focus sets it,
    /// pointer-driven focus clears it, and losing focus clears it.
    pub is_focus_visible: bool,
    /// Whether this node is currently being pressed (for CSS :active).
    pub is_active: bool,
    /// Whether this is an anonymous block box created by the layout engine.
    /// These wrap runs of inline children in mixed-content block containers
    /// (CSS "anonymous block boxes"). Transparent to editing operations.
    ///
    /// **Such a box is deliberately not in the DOM tree** (#566): its `parent`
    /// is `None`, no node's `children` holds it, and it reaches its run through
    /// [`Self::run_members`] instead. See that field.
    pub is_anonymous_block_box: bool,
    /// The inline run an anonymous block box stands for, in document order —
    /// empty on every other node (#566).
    ///
    /// An anonymous box is a **box-tree** construct (CSS 2.1 §9.2.1.1), and
    /// CSS inheritance and selector matching both operate on the **element**
    /// tree. Putting the box in `children` and reparenting its run into it —
    /// which is what this engine used to do — therefore made the box lie about
    /// the author's tree to everything that reads `parent` or `children`:
    /// `remove_child`'s `retain` became a no-op, `insert_before`'s `position()`
    /// failed and fell through to `push`, `next_sibling` answered with the box,
    /// `:nth-child` counted it, and a re-cascaded descendant inherited from it.
    ///
    /// So the run is recorded here rather than adopted. Members keep their real
    /// parent and their real slot; each one points back through
    /// [`Self::run_box`]. Everything that walks the *box* tree rather than the
    /// element tree — paint, hit testing, stacking, the Taffy child lists —
    /// goes through [`crate::RinchDocument::box_tree_children`], which is the
    /// one place those two trees are reconciled.
    pub run_members: Vec<RawNodeId>,
    /// The anonymous block boxes this node is the container of (#566).
    ///
    /// The **downward** half of a box's edge, and the reason invariant A in
    /// [`crate::RinchDocument::dom_tree_violations`] is total rather than
    /// exempted. A box has `parent = Some(container)` and is deliberately
    /// absent from `children`, which is exactly the state A forbids for an
    /// ordinary node — so without this list the box could only be *carved out*
    /// of the check that exists to catch its own historical defect class. With
    /// it, A says the same thing about every node in the slab:
    ///
    /// > `n.parent == Some(p)` ⟺ `n` appears in **exactly one** of
    /// > `p.children` or `p.run_boxes`.
    ///
    /// Boxes stay out of `children` because `children` is the **author's**
    /// tree — what `remove_child`, `insert_before`, `next_sibling` and every
    /// selector index read. A second list keeps the box-tree edge recorded
    /// without putting it anywhere those look.
    ///
    /// It also makes [`crate::RinchDocument::box_tree_children`]'s common case
    /// O(1) (`run_boxes.is_empty()`) rather than a per-child slab scan. That
    /// was the original motivation and it did **not** survive measurement — the
    /// scan it replaces is indistinguishable from this on a 2,000-child
    /// container with no run, in both the paint and layout paths. Treat the
    /// bound as structural insurance, not a speedup; the invariant is what
    /// earns the field.
    ///
    /// **This does not replace [`Self::run_box`]**, which answers a different
    /// question. This one is *membership* — has this container any runs at all.
    /// That one is *ordering* — which child the box stands at, so a run that is
    /// non-contiguous in `children` still emits its box exactly once.
    pub run_boxes: Vec<RawNodeId>,
    /// The anonymous block box whose run this node belongs to, if any (#566).
    ///
    /// The inverse of [`Self::run_members`], and the thing that lets
    /// [`crate::RinchDocument::box_tree_children`] answer without a search: a
    /// child carrying `Some(b)` is drawn by `b`'s inline formatting context, so
    /// the box tree shows `b` in its place.
    ///
    /// **Not `ifc_root`**, which looks like it would serve and does not:
    /// `setup_inline_formatting_contexts` resets every `ifc_root` to `None`
    /// *after* `create_anonymous_block_boxes` has run, so at the moment the
    /// Taffy child lists are rebuilt it still holds the previous pass's value.
    pub run_box: Option<RawNodeId>,
    /// Whether the boxes this node contributes to its parent's flow include an
    /// **in-flow block-level** one — recursing through the two kinds of node
    /// that contribute their children's boxes rather than one of their own
    /// (#513).
    ///
    /// Total over the slab, and derived state: recomputed from scratch at the
    /// top of every `ifc_dirty` pass by
    /// `RinchDocument::recompute_contributes_in_flow_block`, exactly as
    /// `ifc_root` is. Never invalidated per mutation site — creating or
    /// destroying an in-flow block-level box takes a structural change or a
    /// `display`/`position` change, and every one of those sets `ifc_dirty`.
    ///
    /// The rule, in one place:
    ///
    /// ```text
    /// contributes_in_flow_block(n) =
    ///     role(n) == InFlowBlock
    ///  || (role(n) == Contents             && any child contributes)
    ///  || (n is a `display: inline` element && any child contributes)
    /// ```
    ///
    /// where `role` is [`Self::inline_flow_role`]. The recursion **stops at an
    /// atomic inline** — an `inline-block`, `inline-flex` or `inline-grid` is
    /// a formatting context in its own right, so a block inside one is that
    /// box's business and not its parent's (#592) — and `OutOfFlow`, `NoBox`
    /// and `Comment` contribute nothing, which is the same three-way rule every
    /// other IFC decision consumes.
    ///
    /// # What it is for
    ///
    /// Two questions, one answer, and they were two recursive scans before:
    ///
    /// * [`Self::is_split_inline`] — whether a `display: inline` element is
    ///   **broken around** block-level content (CSS 2.1 §9.2.1.1), which is
    ///   #513's subject; and
    /// * whether a `display: contents` wrapper is *transparent* to the
    ///   surrounding inline formatting context, which `ifc.rs`'
    ///   `contents_is_inline_transparent` answers by a per-call subtree walk —
    ///   so a chain of nested wrappers is rescanned once per ancestor.
    ///
    /// The second is deliberately **not** switched over to this field yet. The
    /// two answers differ, measured rather than reasoned: this field recurses
    /// through a `display: inline` element and that scan does not, so a wrapper
    /// holding `<a>x<div/>y</a>` is opaque here and transparent there. Until
    /// the split lands, that difference is observable — see
    /// `split_inline_predicate_tests`.
    pub contributes_in_flow_block: bool,
    /// The block container whose **box-tree and Taffy child lists hold this
    /// out-of-flow box**, when that is not its DOM parent (#591).
    ///
    /// `Some(host)` exactly when this node is an out-of-flow box
    /// ([`InlineFlowRole::OutOfFlow`]) and at least one non-atomic
    /// `display: inline` element stands between it and `host` — the nearest
    /// ancestor that is neither such an inline element nor a `display:
    /// contents` wrapper. Such an inline is detached into its IFC whole, so a
    /// box left in its Taffy child list is laid out by nobody; CSS 2.1 §9.4.2
    /// says the box does not split the inline either, so no anonymous box
    /// takes it. It is therefore made a **unit of the host** — `collect_run_units`
    /// emits it right after the inline element it sits in, `box_tree_children`
    /// of the inline omits it, and [`crate::RinchDocument::box_tree_parent`]
    /// answers the host — which is what gives it the host's Taffy list (its
    /// containing block in CSS, or the ICB through #204), the host's paint
    /// sequence, and the host's `layer_bounds`.
    ///
    /// Recomputed from scratch every `ifc_dirty` pass by
    /// `RinchDocument::recompute_contributes_in_flow_block`, which reports every
    /// box whose value changed; `RinchDocument::rehome_hoisted_out_of_flow` then
    /// rebuilds the Taffy lists of the old and new host — a span restyled to
    /// `inline-block` un-hoists its box, a span restyled back re-hoists it. An atomic inline (`inline-block`, `inline-flex`,
    /// `inline-grid`) resets the walk: a box inside one belongs to it, as the
    /// `DropdownMenu` root's panels do.
    pub hoisted_out_of_flow_to: Option<RawNodeId>,
    /// Whether a descendant reached through non-atomic inline elements and
    /// `display: contents` wrappers carries [`Self::hoisted_out_of_flow_to`]
    /// (#591). `true` on the host and on every transparent node between it and
    /// the box; `false` above the host. It is the O(1) gate that sends
    /// `box_tree_children` down its unit-collecting path for exactly the
    /// containers and inlines that need it, and nothing else.
    pub hosts_hoisted_out_of_flow: bool,
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

    /// True while a `sync_display_contents` pass has this node's children
    /// spliced into an ancestor's Taffy child list (and the node's own Taffy
    /// node detached) because it computed `display: contents` — and no later
    /// pass has healed that state (#520).
    ///
    /// Set by `sync_display_contents` on every node it treats as contents;
    /// cleared by the same function when the node no longer computes
    /// `Contents` (after rebuilding the node's own Taffy child list and its
    /// flattening ancestor's). Between the toggle and that healing pass, the
    /// computed display says "box" while the Taffy tree still holds the
    /// splice — this flag is the only record of that, which is why
    /// `taffy_detach_contribution` consults it: computed display alone
    /// cannot distinguish a healed wrapper from a stale-spliced one.
    pub contents_spliced: bool,
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
    /// Write one attribute, keeping the interned `id` in step (#675).
    ///
    /// The write path that keeps the interned `id` correct, and therefore the
    /// only one an `id` write may take. It is **not** the only writer of
    /// [`Node::attributes`] — that field is `pub` and other code writes
    /// non-`id` keys directly — so nothing but [`Node::id_atom`]'s debug assert
    /// enforces this. `id` is the one attribute Stylo needs as a stored `Atom`
    /// rather than a `String` (see the field's own docs), because
    /// `TElement::id()` hands back a reference.
    pub fn write_attribute(&mut self, name: &str, value: &str) {
        if name == "id" {
            self.id_atom = Some(Atom::from(value));
        }
        self.attributes.insert(name.to_string(), value.to_string());
    }

    /// Remove one attribute, keeping the interned `id` in step (#675).
    ///
    /// Clearing the atom is the half that makes `#a { … }` stop applying when
    /// the `id` is removed at runtime; leaving it set would keep the element in
    /// the id bucket forever.
    pub fn erase_attribute(&mut self, name: &str) {
        if name == "id" {
            self.id_atom = None;
        }
        self.attributes.remove(name);
    }

    /// The interned `id` attribute, or `None` when the node carries none.
    ///
    /// This is what `TElement::id()` hands to Stylo's bucket lookup (#675).
    pub fn id_atom(&self) -> Option<&Atom> {
        debug_assert_eq!(
            self.id_atom.as_deref(),
            self.attributes.get("id").map(|s| s.as_str()),
            "Node::id_atom drifted from the `id` attribute — something wrote \
             `attributes` directly instead of through `write_attribute` / \
             `erase_attribute`, which silently un-fixes #675",
        );
        self.id_atom.as_ref()
    }

    /// Create a new document root node.
    pub fn document(id: RawNodeId, guard: SharedRwLock) -> Self {
        Self {
            id,
            kind: NodeKind::Document,
            parent: None,
            children: Vec::new(),
            attributes: HashMap::new(),
            id_atom: None,
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
            is_focus_visible: false,
            is_active: false,
            is_anonymous_block_box: false,
            run_members: Vec::new(),
            run_box: None,
            run_boxes: Vec::new(),
            contributes_in_flow_block: false,
            hoisted_out_of_flow_to: None,
            hosts_hoisted_out_of_flow: false,
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
            contents_spliced: false,
            ifc_detached: false,
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
            id_atom: None,
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
            is_focus_visible: false,
            is_active: false,
            is_anonymous_block_box: false,
            run_members: Vec::new(),
            run_box: None,
            run_boxes: Vec::new(),
            contributes_in_flow_block: false,
            hoisted_out_of_flow_to: None,
            hosts_hoisted_out_of_flow: false,
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
            contents_spliced: false,
            ifc_detached: false,
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
            id_atom: None,
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
            is_focus_visible: false,
            is_active: false,
            is_anonymous_block_box: false,
            run_members: Vec::new(),
            run_box: None,
            run_boxes: Vec::new(),
            contributes_in_flow_block: false,
            hoisted_out_of_flow_to: None,
            hosts_hoisted_out_of_flow: false,
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
            contents_spliced: false,
            ifc_detached: false,
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
            id_atom: None,
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
            is_focus_visible: false,
            is_active: false,
            is_anonymous_block_box: false,
            run_members: Vec::new(),
            run_box: None,
            run_boxes: Vec::new(),
            contributes_in_flow_block: false,
            hoisted_out_of_flow_to: None,
            hosts_hoisted_out_of_flow: false,
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
            contents_spliced: false,
            ifc_detached: false,
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
    /// - `position` is `fixed` or `sticky`, whatever the `z-index`
    /// - `opacity < 1.0`
    /// - `transform` is non-identity
    ///
    /// **Not the whole CSS list, and the shortfall is not only an
    /// expressibility one.** `clip-path`, `mask`, `isolation`,
    /// `mix-blend-mode`, `contain: paint` and `will-change` are absent from
    /// `ComputedStyle` altogether, so those need new style plumbing per
    /// property. But two creators are representable **today** and still
    /// missing, measured rather than assumed:
    ///
    /// - **a non-`none` `filter`** (CSS Filter Effects §2.1). `filter:
    ///   brightness(0.5)` reaches `ComputedStyle::filter_brightness` and paint
    ///   consumes it, and this function still answers `false`. (`blur()` is the
    ///   genuinely unexpressed part — only the four scalars survive
    ///   `from_stylo`.)
    /// - **a flex or grid item with a `z-index` other than `auto`**, even at
    ///   `position: static` (css-flexbox-1 §5.4, css-grid-1 §6). Both the
    ///   `z_index` and the parent's `display` are already here.
    ///
    /// Neither is folded in here, because adding a creator changes which boxes
    /// hoist — the very axis stage B is re-founding — and landing both at once
    /// would make a regression impossible to attribute. **Tracked as #542.**
    /// The six properties `ComputedStyle` does not carry at all (and `blur()`,
    /// which really is unexpressed) need per-property style plumbing and are a
    /// separate piece of work again.
    ///
    /// Related, and probably to be fixed together: **#415**, this same function
    /// answering `false` for a `transform` that composes to the identity, where
    /// CSS keys on `not none`. Same class of gap — a creator this predicate can
    /// see and does not count.
    ///
    /// One honest consequence of stage B: a box declaring **both** a filter and
    /// a clipping `overflow` used to get a stacking context by accident, via
    /// the `overflow` arm this function no longer has. Its clipping survives —
    /// the chain carries that — but its ordering does not, so stage B slightly
    /// widens #542's exposure rather than leaving it untouched. A filter box
    /// without an `overflow` was already wrong before.
    ///
    /// **`overflow` is not on the list.** It used to be, so that a hoisted
    /// descendant stayed inside the clip bracket paint opened around one
    /// stacking context's sequence; that cost the same user-visible bug twice
    /// (#317's dropdown backdrops, #534's menu-bar overlay), because two
    /// `z-index` values in different contexts were compared when CSS says they
    /// never are. #324 stage B decoupled the two: a hoisted
    /// [`crate::stacking::PaintEntry`] now carries the chain of clipping
    /// ancestors it was lifted past, and its consumer re-applies them. Clipping
    /// is [`Node::clips_overflow`] and stacking is this, and nothing needs them
    /// to be the same question.
    pub fn creates_stacking_context(&self) -> bool {
        use crate::computed_style::PositionValue;

        match self.computed_style.position {
            // A fixed box is viewport-level content and a sticky box is
            // repositioned during scroll; both create a stacking context
            // unconditionally, so their descendants travel with them rather
            // than being hoisted out into a sequence they no longer share a
            // coordinate space with.
            PositionValue::Fixed | PositionValue::Sticky => return true,
            PositionValue::Static => {}
            _ => {
                if self.computed_style.z_index.is_some() {
                    return true;
                }
            }
        }
        if self.computed_style.opacity < 1.0 {
            return true;
        }
        if !self.computed_style.transform.is_identity {
            return true;
        }
        false
    }

    /// Whether this box clips content that overflows it.
    ///
    /// **The** clip predicate: see [`crate::paint::clip`] for why it reads both
    /// axes and what still deviates from CSS. `overflow: clip` counts, which
    /// is the half the paint-side spellings used to miss (#324).
    ///
    /// This is a question about *clipping* and deliberately not about
    /// *scrollability* — `visible` and `clip` are the two non-scrollable
    /// values and only one of them clips. Code looking for the nearest scroll
    /// container (sticky positioning, wheel routing, the scrollbar overlays)
    /// wants a different predicate and must not borrow this one.
    ///
    /// **A non-atomic `display: inline` element never clips**, whatever its
    /// `overflow` computes to. `overflow` applies to block containers, flex
    /// containers and grid containers (css-overflow-3 §3) — an `inline-block`
    /// is a block container and still clips; an inline *box* is none of those,
    /// and a browser ignores `overflow: hidden` on a `<span>`. rinch has a
    /// second reason to say so here rather than leave it to the spec: a
    /// *flowed* inline element owns no box (its fragments' geometry is the
    /// line's — see [`Self::is_flowed_inline_element`]), so a clip derived from
    /// its `layout` was a `0x0` rect at its parent's origin, and
    /// `stacking::Collector::descend` pushed exactly that onto the clip chain of
    /// a positioned box hoisted out from under it whose entry carries the live
    /// chain — a `position: relative` box, or an `absolute` whose containing
    /// block is the span itself. (Not an `absolute` whose containing block is
    /// above the span: `Collector::span` truncates its chain at the containing
    /// block, so #591's own absolutely positioned child never carried this clip
    /// — measured `[]` on the base. Not a `fixed` box: its chain is empty by
    /// rule.) An `inline-block` button inside an `overflow: hidden` span was
    /// untappable.
    ///
    /// **This guard is deliberately wider than the boxless set.** It keys on
    /// "non-atomic inline element", not on [`Self::is_flowed_inline_element`]:
    /// a split inline (#513, `ifc_root` unset) and an *unmarked* inline element
    /// — one no IFC has claimed, which therefore keeps a real Taffy box — are
    /// inline boxes too, and `overflow` applies to neither. Narrowing the guard
    /// to the flowed predicate reads like a tidy unification and silently
    /// restores the clip on both; `clip_predicate_tests` pins each.
    ///
    /// The unmarked case used to have a natural producer and no longer does:
    /// the inner `<span>` of a re-measured `inline-block` was never marked,
    /// which was #630, and since #592 an `inline-block` is a block container
    /// whose inner inline **is** its IFC content (`ifc_root == Some(the
    /// inline-block)`, box `0x0` — measured). Keep the width anyway: the guard
    /// is about what `overflow` means on an inline box, not about which shapes
    /// happen to reach it today, and
    /// `taffy_reachability_tests::an_inline_inside_an_inline_block_is_ifc_content_and_an_unmarked_one_keeps_its_box`
    /// constructs the unmarked state rather than relying on one. Guarded in the predicate rather
    /// than in `clip_shape`, because everything that asks "does this clip" is
    /// required to ask here (`paint::clip`'s module doc), and a guard one layer
    /// down would leave hit testing's `check_children` gate and the dirty-region
    /// prune still believing the span clips.
    pub fn clips_overflow(&self) -> bool {
        use crate::computed_style::OverflowValue;

        if self.is_element() && self.display_mode == DisplayMode::Inline {
            return false;
        }
        !matches!(self.computed_style.overflow_x, OverflowValue::Visible)
            || !matches!(self.computed_style.overflow_y, OverflowValue::Visible)
    }

    /// Whether this node establishes a containing block for absolutely
    /// positioned descendants.
    ///
    /// Per CSS that is any *positioned* element — `position` other than
    /// `static` — plus, since a transform makes an element the containing block
    /// for all its descendants, any element with a non-identity `transform`.
    /// Overflow deliberately does not count. It never established a containing
    /// block; it used to form a *stacking context*, and this line used to cite
    /// that as the reason the two questions are separate. Since #324 stage B it
    /// forms neither — see [`Node::creates_stacking_context`] — so `overflow` is
    /// simply absent from both lists, and the answer here is unchanged.
    ///
    /// This is what stops the walk in `out_of_flow::out_of_flow_kind`, which is
    /// how issue #204's ICB case is told apart from a layout Taffy already gets
    /// right.
    pub fn establishes_abs_containing_block(&self) -> bool {
        !matches!(
            self.computed_style.position,
            crate::computed_style::PositionValue::Static
        ) || !self.computed_style.transform.is_identity
    }

    /// Whether this box is taken **out of flow** — CSS 2.1 §9.3.
    ///
    /// An out-of-flow box is neither inline content nor in-flow block content,
    /// and every inline-formatting-context decision has to say so explicitly:
    /// per §9.2.1.1 it does not force anonymous block box generation, and per
    /// §9.4.2 it does not break an inline formatting context — its inline
    /// siblings carry on across it, on the same line.
    ///
    /// `ifc.rs` used to have no such predicate at all, and excluded these boxes
    /// only *incidentally*: Stylo blockifies an out-of-flow box, so
    /// [`Self::is_inline`] answers `false`. That is the right answer where the
    /// question is "is this inline content" and the **wrong** one where it is
    /// "does this break the flow" — which is issues #406 and #289.
    ///
    /// Keys on `position` alone. `PositionValue::Static` is the default, so a
    /// text node — which never goes through style resolution — answers `false`,
    /// which is what #342 was about when a wrong default hoisted text nodes.
    pub fn is_out_of_flow(&self) -> bool {
        matches!(
            self.computed_style.position,
            crate::computed_style::PositionValue::Absolute
                | crate::computed_style::PositionValue::Fixed
        )
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
            NodeKind::Element(_) => self.display_mode.is_inline_level(),
            _ => false,
        }
    }

    /// The children an inline formatting context should walk for this node.
    ///
    /// [`Self::children`] for every ordinary node, and [`Self::run_members`]
    /// for an anonymous block box — which has no `children`, because it is not
    /// in the element tree (#566).
    ///
    /// **Every IFC site asks this, and there are four**: root discovery in
    /// `build_ifc_layouts` and in `setup_inline_formatting_contexts`, the
    /// marking pass, and the inline walk. They must agree about which nodes an
    /// IFC owns — that is [`Self::inline_flow_role`]'s contract (#366), and it
    /// now includes *where the list comes from*. Three of the four answering
    /// this from `children` and one from `run_members` is the same shape of
    /// defect as #518 and #476: one question, several sites, different answers.
    pub fn ifc_children(&self) -> &[RawNodeId] {
        if self.is_anonymous_block_box {
            &self.run_members
        } else {
            &self.children
        }
    }

    /// Whether this node is a comment node.
    pub fn is_comment(&self) -> bool {
        matches!(&self.kind, NodeKind::Comment(_))
    }

    /// How this box participates in a parent's inline formatting context —
    /// THE shared classifier for every IFC decision (#366).
    ///
    /// Seven sites used to hand-roll this classification — `has_inline`,
    /// `has_block` and the run-grouping loop in `create_anonymous_block_boxes`,
    /// the decision loop in `setup_inline_formatting_contexts`,
    /// `scan_contents_children` (since folded into
    /// [`Self::contributes_in_flow_block`], #513), `collect_contents_out_of_flow`,
    /// `mark_inline_descendants` and `walk_inline_children` — and they were
    /// caught disagreeing twice in one review cycle (#466): once on
    /// display-vs-position precedence (a `debug_assert` panic on markup `main`
    /// rendered fine), once on continue-vs-break at a block-level child
    /// (#366 itself). They all consume this method now; any new IFC decision
    /// must too, not paraphrase it.
    ///
    /// The precedence is **display before position, always**:
    ///
    /// 1. A comment is a [`InlineFlowRole::Comment`] — it has no display and
    ///    no box, and [`Self::is_inline`] answers `true` for it, so it must
    ///    be told apart before the inline check.
    /// 2. `display: none` is a [`InlineFlowRole::NoBox`] — whatever
    ///    `position` says, since a boxless element has no box to take out of
    ///    flow.
    /// 3. `display: contents` is a [`InlineFlowRole::Contents`] — again
    ///    whatever `position` says: Stylo does not blockify
    ///    `display: contents` (`equivalent_block_display` maps
    ///    `DisplayOutside::None` to itself), so `position: absolute` on such
    ///    a wrapper leaves [`Self::is_out_of_flow`] answering `true` while
    ///    the element generates **no box** (browsers ignore `position` on
    ///    it). Classifying it out-of-flow instead rooted containers whose
    ///    flattened in-flow boxes stayed attached — the non-leaf carrier the
    ///    #466 validator exists to catch.
    /// 4. Inline-level content is [`InlineFlowRole::Inline`].
    /// 5. Only now does `position` speak: `absolute`/`fixed` is
    ///    [`InlineFlowRole::OutOfFlow`].
    /// 6. Everything left is an in-flow block-level box,
    ///    [`InlineFlowRole::InFlowBlock`].
    ///
    /// A text node never goes through style resolution, so its
    /// `computed_style.display` keeps the default (`Flex`) and it falls
    /// through steps 2–3 to `Inline` — the same #342 hazard note as
    /// [`Self::is_out_of_flow`].
    pub fn inline_flow_role(&self) -> InlineFlowRole {
        use crate::computed_style::values::DisplayValue;
        if self.is_comment() {
            return InlineFlowRole::Comment;
        }
        match self.computed_style.display {
            DisplayValue::None => return InlineFlowRole::NoBox,
            DisplayValue::Contents => return InlineFlowRole::Contents,
            _ => {}
        }
        if self.is_inline() {
            return InlineFlowRole::Inline;
        }
        if self.is_out_of_flow() {
            return InlineFlowRole::OutOfFlow;
        }
        InlineFlowRole::InFlowBlock
    }

    /// Whether this is a `display: inline` element **broken around** in-flow
    /// block-level content — CSS 2.1 §9.2.1.1's block-in-inline, #513.
    ///
    /// A named predicate rather than a fourth inline `matches!` on
    /// [`DisplayMode`], per #595: no site that asks this enum a question is
    /// exhaustive, so a new variant gets no compiler help and a spelled-out
    /// test is how `inline-flex` managed to be inline-level in one pass and not
    /// in the next.
    ///
    /// Three conditions, and each excludes a case that looks like this one:
    ///
    /// * **an element** — a text node's `computed_style` never goes through
    ///   Stylo, so it keeps the default `display` (the #342 hazard noted on
    ///   [`Self::is_out_of_flow`]);
    /// * **`DisplayMode::Inline` exactly**, not [`DisplayMode::is_inline_level`]
    ///   — an atomic inline ([`DisplayMode::is_atomic_inline`]) establishes a
    ///   formatting context of its own, does any anonymous-box generation
    ///   *inside itself*, and is never split (#592 is that shape, and is a
    ///   different defect);
    /// * **[`Self::contributes_in_flow_block`]**, which is where the recursion
    ///   and the memoization live.
    pub fn is_split_inline(&self) -> bool {
        self.is_element()
            && self.display_mode == DisplayMode::Inline
            && self.contributes_in_flow_block
    }

    /// Whether this is a non-atomic `display: inline` element that an inline
    /// formatting context flows — the `<span>`, `<a>`, `<b>` whose `ifc_root`
    /// the marking pass set.
    ///
    /// Such an element **owns no box**. Parley lays out its *fragments* as part
    /// of the line, its background is a span over the flat text
    /// (`InlineBackgroundSpan`), and nothing ever writes its `layout`:
    /// `write_inline_positions` writes `InlineBox` items — atomic inlines — and
    /// the root's direct text children, and `read_layout_results` reads a Taffy
    /// node the marking pass detached. So the only values its `layout` can hold
    /// are `0x0`, or a **stale** box from a pass when it was block-level (a
    /// `<span style="display: block">` restyled to `inline` keeps its
    /// `400x20`, measured — Taffy serves a detached node the layout it last
    /// computed, #543's mechanism). `read_layout_results` zeroes it and
    /// `taffy_tree_violations` puts it in `E ghost box`'s boxless set, exactly
    /// as for a split inline and a `display: contents` wrapper, so that a
    /// coordinate sum stepping through it — `compute_absolute_position` reaches
    /// every descendant through `box_tree_parent`, which keeps the element in
    /// the chain — adds nothing (#591).
    ///
    /// Three conditions, each excluding a look-alike:
    ///
    /// * **an element** — a text node's `display_mode` is `Inline` too, and a
    ///   text node that is a direct child of its IFC root *does* carry a box
    ///   (`write_inline_positions` stretches it to the line block for
    ///   scroll-height);
    /// * **`DisplayMode::Inline` exactly** — an atomic inline (`inline-block`,
    ///   `inline-flex`) is measured by Taffy and positioned by the IFC and
    ///   carries a real box (`taffy_reachability_tests::inline_content_keeps_a_real_box_and_is_not_a_ghost`);
    /// * **`ifc_root` is set** — an **unmarked** `display: inline` element
    ///   carries a real box, which Taffy laid out and through which every
    ///   descendant's painted position is summed, so zeroing it would move that
    ///   whole subtree. Its one natural producer was the inner `<span>` of a
    ///   re-measured `inline-block` — never marked, `(13, 11, 50x50)` under
    ///   `padding: 11px 13px` — and **#592 removed it**: an `inline-block` is a
    ///   block container now, so that span is its IFC content
    ///   (`ifc_root == Some(the inline-block)`, box `0x0`). Four other routes
    ///   were tried and every one blockifies the element instead, so the
    ///   condition is pinned by a *constructed* state in
    ///   `taffy_reachability_tests::an_inline_inside_an_inline_block_is_ifc_content_and_an_unmarked_one_keeps_its_box`,
    ///   which is the only thing in `-p rinch-dom -p rinch` that kills the
    ///   mutant dropping it. A split inline is never marked either (#513) and
    ///   has its own zeroing branch. (A `display: inline` element blockified into a flex or grid
    ///   item, or by `position: absolute`, is excluded by the *second*
    ///   condition, not this one: Stylo's blockification reaches
    ///   `computed_style.display`, which `style_resolution` syncs into
    ///   `display_mode`, so it is `Block` there.)
    pub fn is_flowed_inline_element(&self) -> bool {
        self.is_element() && self.display_mode == DisplayMode::Inline && self.ifc_root.is_some()
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
    /// The `display: inline` elements the last IFC pass **split** around
    /// block-level content (#513) — [`Node::is_split_inline`].
    ///
    /// Recreated from scratch each `ifc_dirty` pass, exactly like
    /// [`Self::anonymous_block_boxes`] and [`Self::ifc_measure_leaves`], and for
    /// exactly their reason: splitting takes the element's boxes out of its own
    /// Taffy child list and puts them in its block container's, and **nothing
    /// else would put them back** when the element stops being split. That is
    /// [`Node::contents_spliced`]'s shape one pass along (#520) and
    /// `reattach_departed_ifc_children`'s one arm over (#597): a departure the
    /// tree records so a later pass can undo it.
    ///
    /// A list rather than a per-node flag because the undo is unconditional —
    /// `restore_split_inlines` rebuilds each entry's own Taffy child list and its
    /// block container's — so there is no gate to get wrong, and the entries a
    /// pass still wants are simply re-recorded by `split_inline_boxes`.
    pub split_inlines: Vec<RawNodeId>,
    /// Taffy-only measure leaves for IFC roots whose out-of-flow children stay
    /// attached (#466): IFC root DOM id → the childless Taffy node carrying its
    /// [`NodeContext::InlineRoot`]. These nodes have **no DOM identity** — they
    /// are absent from `taffy_map` and from the slab — so every consumer that
    /// maps a Taffy id back to a DOM node must `get`-and-skip, never index.
    /// Recreated each `ifc_dirty` pass exactly like `anonymous_block_boxes`.
    pub ifc_measure_leaves: HashMap<RawNodeId, taffy::NodeId>,
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
    /// How many times `run_taffy_compute` has run over this tree.
    ///
    /// Instrumentation, not state: `resolve_layout`'s `!layout_dirty` early
    /// return is a **behaviour** — a paint-only restyle must not pay for a
    /// compute — and nothing observable distinguishes "took the cheap path" from
    /// "recomputed and got the same answer". This counter is what lets a fixture
    /// pin the cheap path (issue #678, whose repair widens what sets
    /// `layout_dirty` and so could have swallowed it). One `u64` increment per
    /// compute.
    pub taffy_computes: u64,
    /// Text nodes whose Taffy measure context (`NodeContext::Text`) no longer
    /// matches the typography their parent now computes (issue #678).
    ///
    /// `sync_dirty_text_contexts` refreshes the contexts of text nodes in
    /// `dirty_nodes`, which records DOM *mutations* — a recascade is not one, so
    /// a text node whose parent's `font-size` changed kept a context built from
    /// the old one. That is invisible wherever the measure goes through
    /// `NodeContext::InlineRoot`, because `build_inline_layout` reads the
    /// computed styles directly; it is the whole answer wherever it does not,
    /// which is every text node that is a **flex or grid item** — including the
    /// interior of an `inline-flex` or `inline-grid`, the two atomic inlines
    /// #661 leaves frozen after its own repair.
    ///
    /// Read (and emptied) by `sync_dirty_text_contexts`; emptied unread by the
    /// full `sync_text_contexts`, which refreshes every text node anyway.
    pub dirty_text_contexts: HashSet<RawNodeId>,
    /// Atomic inlines (`inline-block`, `inline-flex`, `inline-grid`) whose own
    /// box may have changed size since the last layout pass (issue #661).
    ///
    /// An atomic inline is **detached from its parent's Taffy child list** so
    /// the enclosing IFC measures it as an `InlineBox`, which means the root
    /// Taffy compute never reaches it: the only thing that ever gives it a size
    /// is `compute_inline_block_layouts`, and that runs only on an `ifc_dirty`
    /// pass. A style change or a text change sets neither flag, so the box was
    /// measured once and frozen while paint re-laid its text at the new style.
    ///
    /// This is the scoped repair: a **set**, not a flag, so a text change in one
    /// row of a 500-row document re-measures that row's atomic inlines and not
    /// the document's. Consumed (and emptied) by
    /// `RinchDocument::remeasure_dirty_atomic_inlines` on a pass that runs Taffy
    /// without rebuilding the IFC structure; cleared unconsumed on an
    /// `ifc_dirty` pass, which re-measures every atomic inline anyway.
    ///
    /// Entries are node ids and are **not** validated on insert — a node may be
    /// removed before the set is read, so the consumer `get`s and skips.
    ///
    /// **A `BTreeSet`, and it is the *fixture* that needs it rather than the
    /// code.** The consumer sorts these deepest-first, which is a real ordering
    /// requirement — an outer atomic inline is sized from an inner one's
    /// `Node::layout` — and that sort is correct whatever order it is handed.
    /// What a `HashSet` broke was the ability to *pin* it: the unsorted order
    /// was per-process random, so the fixture that pins the sort caught a
    /// sort-deleted mutant 8 times in 25 runs and missed it the other 17
    /// (measured by the review of #694). Ascending node id is creation order,
    /// which for a tree built parent-first is shallowest-first — exactly the
    /// order the sort has to undo — so the mutant now fails every run.
    ///
    /// Same-depth entries tie, and `sort_by_key` is stable, so their relative
    /// order is this set's order. That cannot matter: two atomic inlines at
    /// equal depth are siblings, and neither is measured from the other.
    ///
    /// Do not swap it back for a `HashSet`. The set holds a handful of entries
    /// and its `O(log n)` insert is not on any path the cost harness measures.
    pub dirty_atomic_inlines: BTreeSet<RawNodeId>,
    /// Cached IFC measure results from previous frames.
    /// Key: (ifc_root_node_id, wrap_width_bits) → (width, height).
    /// Invalidated per-root when text content changes.
    pub ifc_measure_cache: HashMap<(RawNodeId, u32), (f32, f32)>,
    /// Nodes that have requested scroll-into-view (deferred until after layout).
    pub scroll_into_view_requests: Vec<RawNodeId>,
    /// Scroll offsets clamped by the layout engine, pending event dispatch.
    /// Layout must not mutate observable scroll state silently (#144), but it
    /// also can't fire handlers mid-resolve (the facade holds the document
    /// borrow), so `clamp_scroll_offsets` queues (node, clamped offset) pairs
    /// here — coalesced per node, last value wins, since layout may resolve
    /// more than once per frame — for the facade to drain after layout.
    pub pending_scroll_clamps: Vec<(RawNodeId, f64)>,
    /// Scale factor for text rendering (1.0 on desktop, >1.0 on HiDPI/mobile).
    /// Applied to Parley font sizes so glyphs rasterize at physical pixel resolution.
    pub text_scale: f32,
    /// How many times a Taffy attachment has had to be repaired since this
    /// tree was created (#477): an insertion index clamped into range, or an
    /// insert Taffy refused. Monotonic, never reset.
    ///
    /// A non-zero value means the DOM and the Taffy tree disagreed about a
    /// shape one of them had already accepted, which is never benign — the
    /// affected node is attached in the wrong place. Every increment is also
    /// `tracing::warn!`-logged with both node ids; this counter exists so a
    /// test or a devtools surface can assert on the *absence* of divergence
    /// without scraping logs.
    ///
    /// **In shipped code this is provably zero**, not hopefully zero:
    /// `compute_taffy_child_index` returns an in-range index by construction,
    /// and the clamp makes `insert_child_at_index` total (`taffy 0.12.2` can
    /// only fail it with `ChildIndexOutOfBounds`). So a non-zero value is not a
    /// tolerable condition to be handled — it is evidence that a regression of
    /// the #477 class has been reintroduced.
    ///
    /// **Monotonic, never reset.** It answers *"has this tree ever diverged"*,
    /// not *"did it diverge this frame"* — a per-frame question would need a
    /// snapshot-and-compare at the call site, which nothing needs yet.
    ///
    /// See [`crate::RinchDocument::attach_taffy_child_at`].
    //
    // Note for a future field: `NodeTree` is `pub` with all-`pub` fields and is
    // not `#[non_exhaustive]`, so adding one is strictly breaking for any
    // downstream struct literal. Nothing in-tree constructs it that way
    // (`NodeTree::new` is the constructor), which is why this was acceptable.
    pub taffy_attach_faults: usize,
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
            split_inlines: Vec::new(),
            ifc_measure_leaves: HashMap::new(),
            active_transitions: HashMap::new(),
            active_animations: HashMap::new(),
            transitions_enabled: false,
            image_cache: ImageCache::new(),
            image_loader: None,
            dirty_ifc_text_roots: HashSet::new(),
            taffy_computes: 0,
            dirty_text_contexts: HashSet::new(),
            dirty_atomic_inlines: BTreeSet::new(),
            ifc_measure_cache: HashMap::new(),
            scroll_into_view_requests: Vec::new(),
            pending_scroll_clamps: Vec::new(),
            text_scale: 1.0,
            taffy_attach_faults: 0,
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

// ── The `disabled` rule ─────────────────────────────────────────────────────
//
// Two consumers that must not drift: the desktop focus machinery in `rinch`
// (Tab order, the mousedown claim, the edit gate — issue #315) and CSS
// `:disabled`/`:enabled` matching in `stylo_impl` (issue #429). It lives here,
// below both, because a second hand-rolled copy is exactly how a control comes
// to refuse input while still *styling* itself as enabled.
//
// Two *spellings*, though, with one rule each (issue #612): HTML `disabled` by
// presence, like a browser; rinch's `data-disabled` with rinch's `"false"`
// escape. Both entry points below are private — everything outside asks
// `node_is_disabled`, which is their union.

/// Whether a node carries a **disabled** marker, in either spelling.
///
/// `data-disabled` is what rinch's own widgets write (`select_widget.rs`, for a
/// disabled `<option>`); the plain HTML `disabled` is what the component library
/// writes — `Button`, `ActionIcon`, `CloseButton`, `TextInput`, `Textarea`,
/// `NumberInput`, `PasswordInput`, `Checkbox`, `Radio`, `Switch`, `NavLink`,
/// `Pagination`, `Tabs`, `Accordion`, `DropdownMenu`, `Select`, `Fieldset`.
///
/// **The two spellings are read by two different rules**, each spelled once
/// below, because they answer to two different authorities:
///
/// - HTML `disabled` is read by **presence alone**, the way a browser reads it.
/// - `data-disabled` is rinch's own, and keeps rinch's `"false"` escape.
///
/// They used to share the escape, which made `disabled="false"` *enable* a
/// control on desktop and disable it in a browser — the same markup, opposite
/// behaviour, and the whole of issue #612.
pub fn node_is_disabled(node: &Node) -> bool {
    html_disabled_attribute_is_set(node) || data_disabled_attribute_is_on(node)
}

/// HTML's `disabled`, read by **presence alone**.
///
/// HTML gives a boolean attribute no falsey spelling: a present `disabled`
/// disables whatever string it holds, so `disabled="false"` is disabled.
/// Measured in Chrome 150 rather than recalled — `<button disabled="false">`
/// answers `.disabled === true` and matches `:disabled`
/// (`crates/rinch-web/tests/boolean_attributes.rs::html_reads_a_present_boolean_attribute_as_true_whatever_its_value`).
/// `rinch-web` inherits that for free by handing the attribute to the browser,
/// so reading it any other way here is a pure desktop/web divergence (#612).
///
/// There is therefore nothing to "opt out" with: removing the attribute is how
/// markup says *enabled*, which is what [`NodeHandle::write_attribute`] does for
/// a falsey reactive binding (#551).
///
/// [`NodeHandle::write_attribute`]: rinch_core::dom::NodeHandle::write_attribute
fn html_disabled_attribute_is_set(node: &Node) -> bool {
    node.attributes.contains_key("disabled")
}

/// rinch's own `data-disabled`, which **keeps** the `"false"` escape.
///
/// Not an HTML attribute, so the browser rule above has no jurisdiction: this is
/// rinch's convention, documented as "present unless the value is `false`" since
/// it was written. `data-disabled` has no web reader at all — the browser does not
/// know the attribute — so the escape's two-backend half is carried by its
/// siblings `data-nofocus` and `data-trap-focus`, whose web selectors are
/// `[data-nofocus]:not([data-nofocus="false" i])` and
/// `[data-trap-focus]:not([data-trap-focus="false" i])`.
///
/// The rule itself is [`rinch_core::dom::data_attr_is_on`], shared with
/// `RinchApp::node_is_nofocus` so the two spell one rule. Shared, not enforced:
/// each reader still picks its own helper, and `"0"` is the only value that can
/// tell which one it picked — pinned at both readers by
/// `computed_style_tests::the_data_escape_excuses_only_false_at_the_reader` and
/// `nofocus_tests::only_false_opts_out_not_zero`, because with every fixture
/// sampling `""` / `"false"` / `"true"` a reader could revert to
/// [`rinch_core::dom::attr_is_truthy`] with the whole suite green.
fn data_disabled_attribute_is_on(node: &Node) -> bool {
    node.attributes
        .get("data-disabled")
        .is_some_and(|v| rinch_core::dom::data_attr_is_on(v))
}

/// [`node_is_disabled`] for the node itself, **or** an enclosing
/// `<fieldset disabled>`.
///
/// `<fieldset>` is the one element whose `disabled` reaches past itself: HTML
/// disables every descendant control, which is the whole reason the element
/// exists. The exception HTML also carves — controls inside the fieldset's
/// **first `<legend>`** stay enabled, so a form can put its own "enable this
/// section" checkbox there — is honoured too.
///
/// Everything else disables only itself (a disabled `<button>` does not
/// disable a `<span>` inside it).
pub fn node_is_disabled_in_tree(tree: &NodeTree, node_id: RawNodeId) -> bool {
    let mut cur = Some(node_id);
    let mut child = None;
    while let Some(nid) = cur {
        let Some(node) = tree.get(nid) else {
            return false;
        };
        let is_fieldset = node.tag() == Some("fieldset");
        // Skip the fieldset's own check when we arrived through its first
        // <legend>: that subtree is exempt.
        let exempt =
            is_fieldset && child.is_some_and(|c| first_legend_child(tree, node) == Some(c));
        if !exempt && (nid == node_id || is_fieldset) && node_is_disabled(node) {
            return true;
        }
        child = Some(nid);
        cur = node.parent;
    }
    false
}

/// The id of `parent`'s first `<legend>` child, if it has one.
///
/// Public because the Tab collector needs the same exemption while walking
/// top-down with an inherited-disable flag on its stack, rather than by asking
/// [`node_is_disabled_in_tree`] per node.
pub fn first_legend_child(tree: &NodeTree, parent: &Node) -> Option<RawNodeId> {
    parent
        .children
        .iter()
        .copied()
        .find(|&c| tree.get(c).and_then(|n| n.tag()) == Some("legend"))
}

/// Whether a tag names an element HTML lets be disabled — the set `:disabled`
/// and `:enabled` are defined over.
///
/// CSS is deliberately narrower here than the focus machinery, which applies
/// [`node_is_disabled`] to *any* node because rinch lets any `tabindex` node
/// opt out of the Tab order. `:disabled` is specified over form controls only,
/// so a `<div data-disabled>` must not match it — matching would style on
/// desktop what `rinch-web` leaves unstyled in a real browser.
pub fn tag_is_disableable(tag: Option<&str>) -> bool {
    matches!(
        tag,
        Some("button" | "input" | "select" | "textarea" | "option" | "optgroup" | "fieldset")
    )
}
