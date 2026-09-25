//! Layout engine: Taffy layout resolution, text measurement, display:contents, and IFC invalidation.

use std::collections::HashMap;

use peniko::Brush;
use peniko::color::{AlphaColor, Srgb};

use crate::RinchDocument;
use crate::layout;
use crate::node::{LayoutResult, NodeContext, NodeKind};

/// The parent-inherited text properties a text node's [`TextMeasure`] carries.
///
/// A named struct rather than the wide tuple this used to be, because **two
/// functions fill it and they must agree field for field**:
/// [`RinchDocument::sync_text_contexts`] refreshes every text node and
/// [`RinchDocument::sync_dirty_text_contexts`] refreshes the ones a DOM
/// mutation or a restyle touched. As eleven positional elements spelled out
/// twice, a property added to one and not the other was a silent disagreement
/// between a full relayout and an incremental one — and #698 had to add two.
///
/// The list is the `TextMeasure` half of the "one list" documented at
/// [`crate::computed_style::ComputedStyle::same_text_layout_inputs`]: a
/// property read here must be listed there too, or a restyle that changes it
/// keeps the layout it invalidated.
struct TextContextFields {
    font_size: f32,
    font_weight: f32,
    font_family: String,
    line_height_css: String,
    color: AlphaColor<Srgb>,
    no_wrap: bool,
    letter_spacing: f32,
    word_spacing: f32,
    overflow_wrap: crate::computed_style::OverflowWrapValue,
    text_overflow: crate::computed_style::TextOverflowValue,
    parent_overflow_hidden: bool,
}

impl TextContextFields {
    /// Read them off the text node's parent element, or fall back to the
    /// document defaults when it has none.
    fn from_parent(parent: Option<&crate::node::Node>) -> Self {
        use crate::computed_style::{OverflowValue, WhiteSpaceValue};
        let Some(parent) = parent else {
            return Self {
                font_size: 16.0,
                font_weight: 400.0,
                font_family: "sans-serif".to_string(),
                line_height_css: String::new(),
                color: AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255),
                no_wrap: false,
                letter_spacing: 0.0,
                word_spacing: 0.0,
                overflow_wrap: crate::computed_style::OverflowWrapValue::default(),
                text_overflow: crate::computed_style::TextOverflowValue::default(),
                parent_overflow_hidden: false,
            };
        };
        let cs = &parent.computed_style;
        Self {
            font_size: cs.font_size,
            font_weight: cs.font_weight,
            font_family: if cs.font_family.is_empty() {
                "sans-serif".to_string()
            } else {
                cs.font_family.clone()
            },
            line_height_css: match &cs.line_height {
                crate::computed_style::LineHeightValue::Normal => String::new(),
                crate::computed_style::LineHeightValue::Absolute(v) => format!("{}px", v),
                crate::computed_style::LineHeightValue::Relative(v) => v.to_string(),
            },
            color: cs
                .color
                .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255)),
            // Whether white-space prevents wrapping.
            no_wrap: matches!(
                cs.white_space,
                WhiteSpaceValue::NoWrap | WhiteSpaceValue::Pre
            ),
            letter_spacing: cs.letter_spacing,
            word_spacing: cs.word_spacing,
            overflow_wrap: cs.overflow_wrap,
            text_overflow: cs.text_overflow,
            parent_overflow_hidden: matches!(
                cs.overflow_x,
                OverflowValue::Hidden | OverflowValue::Clip
            ),
        }
    }

    /// Write them into the Taffy node context a text node measures out of.
    fn apply_to(self, tm: &mut crate::node::TextMeasure, node_id: usize) {
        tm.font_size = self.font_size;
        tm.font_weight = self.font_weight;
        tm.font_family = self.font_family;
        tm.line_height_css = self.line_height_css;
        tm.node_id = node_id;
        tm.color = self.color;
        tm.no_wrap = self.no_wrap;
        tm.letter_spacing = self.letter_spacing;
        tm.word_spacing = self.word_spacing;
        tm.overflow_wrap = self.overflow_wrap;
        tm.text_overflow = self.text_overflow;
        tm.parent_overflow_hidden = self.parent_overflow_hidden;
    }
}

/// How `RINCH_TREE_CHECK` makes the post-layout sweep behave (#584).
///
/// The sweep is a debug-build invariant check, so `Off` is the whole cost in a
/// release build: the `cfg(debug_assertions)` block around its only reader is
/// compiled out.
#[cfg(debug_assertions)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeCheckMode {
    /// Unset. The sweep does not run.
    Off,
    /// `RINCH_TREE_CHECK=warn` — print violations and carry on, which is what
    /// the flag did for its whole life before #584. Kept for driving a real app
    /// under the flag: a window that keeps running and complains is more use
    /// there than a panic inside a frame.
    Warn,
    /// Any other value, `1` included. A violation **fails the layout pass**,
    /// which fails the test it happened in without anybody having to pass
    /// `--nocapture`.
    Fail,
}

#[cfg(debug_assertions)]
impl TreeCheckMode {
    fn from_env() -> Self {
        match std::env::var("RINCH_TREE_CHECK") {
            Err(_) => Self::Off,
            Ok(v) if v.eq_ignore_ascii_case("warn") => Self::Warn,
            Ok(_) => Self::Fail,
        }
    }
}

impl RinchDocument {
    /// Resolve layout using Taffy.
    ///
    /// Computes layout for the entire tree given a viewport size,
    /// then reads layout results back into each node's `layout` field.
    /// Text nodes are measured using Parley for accurate text layout.
    pub fn resolve_layout(&mut self, width: f32, height: f32) {
        use crate::perf::Counter;
        // Layout time is this call's wall clock minus the style time spent
        // inside it (`resolve_styles` / `apply_stylo_styles_to_taffy` time
        // themselves), so the two phases never double count.
        let t0 = web_time::Instant::now();
        let style_before = self.tree.perf.get(Counter::TimeStyleNs);
        self.tree.perf.bump(Counter::LayoutResolves);
        self.resolve_layout_inner(width, height);
        let total = t0.elapsed().as_nanos() as u64;
        let style = self
            .tree
            .perf
            .get(Counter::TimeStyleNs)
            .wrapping_sub(style_before);
        self.tree
            .perf
            .add(Counter::TimeLayoutNs, total.saturating_sub(style));
    }

    fn resolve_layout_inner(&mut self, width: f32, height: f32) {
        use crate::perf::Counter;

        let old_viewport = self.tree.viewport;
        self.tree.viewport = crate::layout::Viewport { width, height };

        // The hit tester's memo describes the boxes as they are, so every path
        // below that can move one drops it. The one that cannot — the
        // paint-only skip, which runs no Taffy compute and rebuilds no text
        // layout — keeps it (#911): a scroll notch's frame takes that path, and
        // dropping the memo there made every notch rebuild every row's extent.
        // A restyle on that path invalidates in `resolve_styles`, and a
        // background image landing in `push_dirty`.
        let viewport_changed =
            (old_viewport.width - width).abs() > 0.5 || (old_viewport.height - height).abs() > 0.5;

        // A viewport change restyles only what the new size reaches — a
        // flipped media query restyles everything, a viewport unit its users —
        // and always re-runs layout. See `restyle_for_viewport_change`.
        if viewport_changed {
            self.restyle_for_viewport_change(width, height);
        }

        // Drain completed image loads and update intrinsic dimensions.
        //
        // A newly decoded image changes a Taffy node's *context*, not its Taffy
        // style, so the `mark_dirty` inside `drain_pending_images` is invisible
        // to the `if !layout_dirty { return }` below and the whole compute is
        // skipped — leaving the `<img>` at the 0x0 intrinsic size it was
        // created with, laid out as nothing and painted as nothing, however
        // many frames follow. That is the same class of miss the viewport
        // branch above records, and an image landing is the other thing that
        // needs a recompute without any style having changed. (The drain
        // answers `false` for a `background-image`, which changes no box — it
        // publishes those by marking their users paint-dirty instead.)
        if self.drain_pending_images() {
            self.tree.layout_dirty = true;
        }

        // Resolve Stylo styles and apply to Taffy nodes (only if dirty)
        if self.tree.styles_dirty {
            self.resolve_styles();
            self.apply_stylo_styles_to_taffy();
            self.tree.styles_dirty = false;
        }
        // Everything styled so far is rendered by this frame; a later
        // cascade has a before-change style to transition from.
        for id in std::mem::take(&mut self.tree.styled_unrendered) {
            if let Some(n) = self.tree.nodes.get_mut(id) {
                n.styled_unrendered = false;
            }
        }

        // Trigger loads for any background-image URLs not yet in the cache
        self.request_background_image_loads();

        // Skip full layout recompute when no layout-affecting properties changed.
        // Paint-only changes (background-color, opacity, cursor on hover) set
        // styles_dirty but NOT layout_dirty, so we resolve styles above but
        // skip the expensive Taffy compute + IFC rebuild below.
        //
        // However, text-affecting style changes (font-size, font-weight, font-style,
        // text-decoration) don't change Taffy styles so layout_dirty won't be set.
        // For these, skip Taffy but still rebuild the affected IFC text layouts.
        if !self.tree.layout_dirty {
            if !self.tree.dirty_ifc_text_roots.is_empty() {
                self.tree.hit_cache.invalidate();
                self.tree.perf.bump(Counter::LayoutSkippedTextOnly);
                self.sync_dirty_text_contexts();
                let t = web_time::Instant::now();
                let mut temp_layout_cx = std::mem::take(&mut self.layout_cx);
                self.build_ifc_layouts(&mut temp_layout_cx);
                self.layout_cx = temp_layout_cx;
                self.tree.perf.add_elapsed(Counter::TimeBuildIfcNs, t);
                self.tree.dirty_ifc_text_roots.clear();
            } else {
                if viewport_changed {
                    self.tree.hit_cache.invalidate();
                }
                self.tree.perf.bump(Counter::LayoutSkippedPaintOnly);
            }
            return;
        }
        self.tree.hit_cache.invalidate();
        self.tree.layout_dirty = false;

        let root_taffy = match self.tree.nodes[self.tree.root_id].taffy_id {
            Some(id) => id,
            None => return,
        };

        // IFC setup passes only need to run when tree structure or display modes
        // changed. Text-only changes (e.g. slider label "50%" → "19%") preserve the
        // existing IFC structure and Taffy's internal cache — only the dirty text
        // node gets re-measured, avoiding the 80ms full-tree Parley rebuild.
        if self.tree.ifc_dirty {
            self.tree.ifc_setup_passes += 1;
            self.tree.perf.bump(Counter::IfcFullPasses);
            match self.tree.ifc_full_reason.take() {
                Some(reason) => self.tree.perf.bump(reason.counter()),
                None => self.tree.perf.bump(Counter::IfcFullUnattributed),
            }
            // A whole-document pass covers every seed recorded since the last.
            self.tree.ifc_seeds.clear();
            // Structural change — clear dirty_ifc_text_roots so
            // build_ifc_layouts considers ALL IFC roots (and rebuilds every
            // one without a valid `text_layout`). Without this, stale entries
            // from set_text_content calls during rendering (before
            // setup_inline_formatting_contexts assigns correct ifc_root
            // values) cause build_ifc_layouts to skip newly created IFC roots
            // — making their text invisible.
            //
            // The measure cache is **not** cleared any more. Every insert
            // into `dirty_ifc_text_roots` dropped that root's sizes when it
            // was made, so forgetting the set loses nothing; and which roots'
            // *content* this change touched is answered below, by
            // `refresh_ifc_signatures`, once the passes that decide it have
            // run. Clearing the whole cache here re-shaped every IFC root in
            // the document to measure one appended row.
            let t_ifc = web_time::Instant::now();
            self.tree.perf.bump(Counter::IfcSetupPasses);
            self.tree.dirty_ifc_text_roots.clear();

            // Handle display:contents by rebuilding taffy children for affected nodes
            self.sync_display_contents(None);

            // Detect and set up inline formatting contexts
            self.setup_inline_formatting_contexts(None);

            // Sync font-size and the rest of the inherited text properties from
            // parent elements into text node contexts.
            //
            // **Before `compute_inline_block_layouts`, and that ordering is
            // #625.** A `NodeContext::Text` is created with `font_size: 16.0`,
            // `font_weight: 400.0` and empty `font_family`/`line_height_css` —
            // placeholders this pass replaces. An atomic inline whose interior
            // is a *flex* or *grid* container measures its text through that
            // context, so running the measure first measured every one of them
            // with the placeholders: `font-size: 32px` produced the same 211x22
            // box as 16px, and the declared `line-height: 20px` came out 22,
            // Parley's default for an undeclared 16px line, while paint used the
            // real style and overflowed the box by 240px.
            //
            // It ran after for as long as the pass has existed, and only
            // `inline-block` hid it: **that** interior is an inline formatting
            // context (#592), and `build_inline_layout` reads the computed
            // styles directly rather than the Taffy context, so the inline-block
            // arm of #625 is fixed twice over. `inline-flex` and `inline-grid`
            // have no second route and this line is the whole of their fix —
            // measured, by putting the two calls back in their old order with
            // everything else in #592 kept: `inline-block` stays correct and the
            // other two regress to 211x22.
            self.sync_text_contexts();

            // Pre-compute layout for inline-block children that were detached from Taffy.
            // They need their own subtree measured so walk_inline_children can read dimensions.
            self.compute_inline_block_layouts();

            // Which roots did this change actually reach? Last, because the
            // signature folds in each atomic inline's size, which the line
            // above has only just decided.
            self.refresh_ifc_signatures(None);

            self.tree.ifc_dirty = false;
            self.tree.perf.add_elapsed(Counter::TimeIfcSetupNs, t_ifc);
        } else if !self.tree.ifc_seeds.is_empty() {
            // The scoped structural pass (`crate::ifc_scope`): the same passes,
            // in the same order, over only the formatting containers the
            // recorded mutations reached. Everything else keeps its splices,
            // boxes, marks and measures, and so its Taffy cache.
            let t_ifc = web_time::Instant::now();
            let scope = self.compute_ifc_scope();
            if !scope.is_empty() {
                self.tree.ifc_setup_passes += 1;
                self.tree.perf.bump(Counter::IfcSetupPasses);
                self.tree.perf.bump(Counter::IfcScopedPasses);
                self.tree
                    .perf
                    .add(Counter::IfcScopeContainers, scope.containers.len() as u64);
                self.tree
                    .perf
                    .add(Counter::IfcScopeNodes, scope.nodes.len() as u64);
                self.sync_display_contents(Some(&scope));
                self.setup_inline_formatting_contexts(Some(&scope));
                // Before the atomic inlines are sized, for #625's reason (see
                // the whole-document branch above).
                self.sync_scoped_text_contexts(&scope);
                self.measure_scoped_atomic_inlines(&scope);
                self.refresh_ifc_signatures(Some(&scope));
            } else {
                self.sync_dirty_text_contexts();
            }
            // What the scope queued (the atomic inlines around its
            // containers), what `refresh_ifc_signatures` queued, and what any
            // other change queued since the last pass.
            self.remeasure_dirty_atomic_inlines();
            self.tree.perf.add_elapsed(Counter::TimeIfcSetupNs, t_ifc);
        } else {
            // IFC structure unchanged — only sync text contexts for dirty nodes
            self.sync_dirty_text_contexts();

            // An atomic inline is detached from its parent's Taffy child list,
            // so the compute below cannot reach it and the branch above — the
            // only thing that ever sized one — has been skipped. Re-measure the
            // ones something actually changed under (issue #661), before the
            // compute, because the enclosing IFC line-breaks against the box
            // this produces.
            self.remeasure_dirty_atomic_inlines();
        }

        let available_space = taffy::Size {
            width: taffy::AvailableSpace::Definite(width),
            height: taffy::AvailableSpace::Definite(height),
        };

        let mut text_layout_cache = self.run_taffy_compute(root_taffy, available_space);

        // #120: an inline-block with a percentage main size is pre-measured detached
        // from Taffy under `MaxContent` (see `compute_inline_block_layouts`), where it
        // has no containing block to resolve the percentage against — so it collapses
        // to min-content. Its containing block only has a width once the compute above
        // has run. Re-measure those inline-blocks against that width now, and if any
        // changed size, re-run the compute so the enclosing IFCs line-break against the
        // corrected boxes. Costs nothing when no percentage inline-block exists.
        if self.resolve_percentage_inline_blocks() {
            text_layout_cache = self.run_taffy_compute(root_taffy, available_space);
        }

        // #278: a mixed `calc(%, px)` value has no Taffy representation (see
        // `calc_layout.rs`), so its style carries a seed until the containing
        // block has a size. Resolve every such value against the sizes the
        // compute above produced and re-run until nothing moves — a calc
        // container whose child is calc-sized converges one level per pass.
        // On the converged path no layout result is read (and nothing
        // painted) from a seed value. Percentage cycles are not what the cap
        // is for — those are broken the way browsers and Taffy break them,
        // by resolving a percentage against an *indefinite* basis as
        // zero/auto (`calc_axis_definite`). The cap bounds the residual
        // content-feedback corner (e.g. `min-size: auto` growing a nominally
        // definite axis): a capped run lays out from the last iterate — a
        // wrong but bounded answer after 8 extra computes — and says so on
        // stderr once per process rather than hiding it.
        let mut calc_passes = 0;
        while self.resolve_layout_calcs() {
            self.tree.perf.bump(Counter::CalcFixpointPasses);
            text_layout_cache = self.run_taffy_compute(root_taffy, available_space);
            calc_passes += 1;
            if calc_passes >= 8 {
                static CAP_WARNING: std::sync::Once = std::sync::Once::new();
                CAP_WARNING.call_once(|| {
                    eprintln!(
                        "[rinch] calc() layout fixpoint hit its iteration cap; a mixed                          calc() in this document is feeding back into its own basis and                          its layout is approximate (reported once per process)"
                    );
                });
                break;
            }
        }

        // Read layout results back into nodes
        self.read_layout_results(self.tree.root_id);
        // The walk above is over the **element** tree, and an anonymous block
        // box is not in it (#566) — so nothing above visits one, and its
        // `layout` would stay at the origin while its line is measured and
        // painted from it. Read them back here rather than teaching the
        // recursion a second child list: the recursion must keep visiting the
        // run's *members* (their IFC-assigned position is preserved inside it),
        // so a box tree walk that replaced them would lose that, and a walk
        // that unioned them would cost every node a merge for the sake of a
        // handful of boxes. This is O(boxes) and off the per-node path.
        for anon_id in self.tree.anonymous_block_boxes.clone() {
            self.read_layout_results_for_box(anon_id);
        }

        // Build inline layouts for IFC roots (rebuild with final widths and store)
        // Temporarily take layout_cx out to avoid borrow conflict
        let t = web_time::Instant::now();
        let mut temp_layout_cx = std::mem::take(&mut self.layout_cx);
        self.build_ifc_layouts(&mut temp_layout_cx);
        self.layout_cx = temp_layout_cx;
        self.tree.perf.add_elapsed(Counter::TimeBuildIfcNs, t);
        self.tree.dirty_ifc_text_roots.clear();

        // Copy cached text layouts to nodes (use the exact layouts from
        // measurement) — the root compute's text leaves and, since #904, the
        // ones the detached atomic-inline computes measured this pass (an
        // `inline-flex` label), which the root compute never reaches.
        let mut text_layout_cache = text_layout_cache;
        text_layout_cache.extend(std::mem::take(&mut self.tree.atomic_leaf_layouts));
        self.copy_cached_text_layouts(text_layout_cache);

        // Clamp scroll offsets to the valid range after layout. When a scroll
        // container shrinks (e.g., window resize makes max-height smaller) or
        // grows (content now fits), the old scroll offset may exceed the new
        // max; clamping here ensures paint and hit-testing use valid values.
        // **After** the two text passes above: the range is `content_extents`,
        // which reads an IFC root's and an anonymous box's `text_layout`, and
        // before them that is last pass's lines or none (review of #1045 — a
        // bottom-pinned text scroller snapped to 0 when text was appended).
        self.clamp_scroll_offsets();

        // Arm transitions now that the first layout has completed, so nothing
        // transitions into existence on page load.
        //
        // `Node::has_been_styled` is not enough on its own: a tree is cascaded
        // more than once before its first layout — appending a `<style>`
        // element re-resolves the whole document on the spot
        // (`maybe_load_style_css`) — so a component that appends its own
        // stylesheet after building its markup leaves every node already
        // styled, and the next rule it loads is a *change* on an already-styled
        // node.
        //
        // **This arms transitions and nothing else** (issue #762). A
        // `@keyframes` animation has no before-change style to be wrong about
        // and runs on the very first frame, exactly as in a browser; the
        // animation half of `apply_stylo_styles_to_taffy` deliberately reads no
        // flag. It used to read this one, and an animation present in the first
        // frame therefore never started at all.
        if !self.tree.transitions_enabled {
            self.tree.transitions_enabled = true;
        }

        // `RINCH_TREE_CHECK=1` sweeps every Taffy-tree inconsistency, every
        // orphaned box (#476), every unreachable subtree (#589) and every
        // DOM-tree inconsistency (#578) after each layout, so the invariant can
        // be checked across a whole suite rather than only where a fixture
        // thought to ask. Debug builds only, and the env read is cached —
        // release compiles the whole thing out.
        //
        // **A violation FAILS (#584).** It used to be an `eprintln!` beside an
        // assertion that never ran, and `cargo test` captures stderr for every
        // test that *passes* — which was all of them. So the sweep ran, found
        // whatever was there, and printed none of it unless someone remembered
        // `-- --nocapture`: for the whole life of the flag, including the
        // #476/#477 work it was built for, and including the 16 lines #597's
        // defect was hiding inside. Measured while wiring #578 — a deliberately
        // injected violation printed 149 times with `--nocapture` and **0**
        // times without it.
        //
        // Failing is what removes the thing you have to remember: libtest prints
        // a *failing* test's captured output in its summary, so the reason
        // arrives with the failure and `--nocapture` becomes unnecessary rather
        // than merely advisable. Two things could still absorb it, and neither
        // does today (checked, not assumed): a `catch_unwind` that swallows
        // rather than re-propagates — the four in `crates/rinch/tests` all
        // re-propagate, by design and by their own docs — and a `#[should_panic]`
        // around a layout pass, of which the two crates have exactly one
        // (`ifc_leaf_invariant_tests`), pinned to a different message, so this
        // panic fails it rather than satisfying it.
        //
        // `tree_check_verdict` is what fails; its docs keep the history of the
        // waived arm it had while #513 and then #591 were open. `RINCH_TREE_CHECK=warn` keeps
        // the old print-only behaviour, for driving a real app under the flag
        // where a panic mid-frame is less use than a running window.
        //
        // `dom_tree_violations` is in the sweep because the paragraph above was,
        // until #578, false of the one invariant that most needed it. The Taffy
        // check compares the Taffy tree against itself, so it is structurally
        // blind to DOM corruption — both of #566's reconciler failures had a
        // *consistent* Taffy tree over a broken DOM and this hook reported
        // all-clear through every one of them. The check added because no
        // fixture thought to ask was the check no sweep was asking.
        //
        // **`run_bookkeeping_violations` (R) is deliberately not here.** It is
        // the one of the three that is not any-time-true: it describes the run
        // bookkeeping, which `create_anonymous_block_boxes` rebuilds, so it only
        // means anything after a pass that actually ran that rebuild. This hook
        // sits at the end of `resolve_layout`, which looks like such a point and
        // is not always one — the IFC passes above are skipped whenever
        // `ifc_dirty` is false, so a text-only pass reaches here having re-minted
        // nothing. Sweeping R would put a check that can report correct state as
        // a violation into the one place whose value depends on a clean run
        // meaning something. That is what invariant E was deleted for; do not
        // "finish the job" by adding the third.
        #[cfg(debug_assertions)]
        {
            static MODE: std::sync::OnceLock<TreeCheckMode> = std::sync::OnceLock::new();
            match *MODE.get_or_init(TreeCheckMode::from_env) {
                TreeCheckMode::Off => {}
                TreeCheckMode::Warn => {
                    for line in self.taffy_tree_violations() {
                        eprintln!("TREECHECK {line}");
                    }
                    for line in self.dom_tree_violations() {
                        eprintln!("TREECHECK {line}");
                    }
                }
                TreeCheckMode::Fail => {
                    let verdict = self.tree_check_verdict();
                    assert!(
                        verdict.fatal.is_empty(),
                        "RINCH_TREE_CHECK: resolve_layout left {} layout-tree \
                         violation(s):\n  {}\n\nThese are invariant failures, not \
                         test expectations — a box no compute pass can reach, a \
                         double-claimed Taffy edge, an element that generates no \
                         box while carrying one, or a DOM/box-tree disagreement. \
                         Nothing is waived (see RinchDocument::tree_check_verdict). \
                         RINCH_TREE_CHECK=warn downgrades this to a print.",
                        verdict.fatal.len(),
                        verdict.fatal.join("\n  "),
                    );
                }
            }
        }
    }

    /// Run the root Taffy compute with the Parley measure function.
    ///
    /// Returns the text layouts built during measurement, keyed by
    /// `(node_id, wrap_width_bits)`, so paint can reuse the exact layouts that
    /// measurement produced. Safe to call more than once per frame — see the
    /// percentage inline-block second pass in `resolve_layout`.
    fn run_taffy_compute(
        &mut self,
        root_taffy: taffy::NodeId,
        available_space: taffy::Size<taffy::AvailableSpace>,
    ) -> HashMap<(usize, u32), parley::layout::Layout<Brush>> {
        use crate::perf::Counter;
        self.tree.taffy_computes += 1;
        self.tree.perf.bump(Counter::TaffyRootComputes);
        // The measure closure borrows `self.tree.nodes` while the compute
        // borrows `self.tree.taffy` mutably, so it counts into locals that are
        // folded into `self.tree.perf` after the compute.
        let measure_calls = std::cell::Cell::new(0u64);
        let shape_ifc = std::cell::Cell::new(0u64);
        let hang = std::cell::Cell::new(crate::ifc::HangStats::default());
        let shape_text = std::cell::Cell::new(0u64);
        let cache_hits = std::cell::Cell::new(0u64);
        let font_cx = &mut self.font_cx;
        let layout_cx = &mut self.layout_cx;
        let nodes = &self.tree.nodes;
        // A root in `dirty_ifc_text_roots` must not be answered from a size
        // measured before it went dirty. That used to be spelled as a bypass —
        // every measure of a dirty root shaped, however many times Taffy asked
        // it the same width in one compute (four shapes for one edited row
        // where one would do). Dropping the dirty roots' old sizes here says
        // the same thing and lets this compute reuse what it shapes itself.
        // (Most routes into the set already dropped them; the virtualized
        // editor's materialisation is one that does not.)
        for root in &self.tree.dirty_ifc_text_roots {
            if let Some(entry) = self.tree.ifc_measure_cache.get_mut(root) {
                entry.sizes.clear();
            }
        }

        // Cache for text layouts built during measurement.
        // Key: (node_id, wrap_width as bits) - wrap width is part of key since layout depends on it
        // Value: Parley layout
        use std::cell::RefCell;
        let text_layout_cache: RefCell<HashMap<(usize, u32), parley::layout::Layout<Brush>>> =
            RefCell::new(HashMap::new());
        // Persistent IFC measure cache — survives across frames.
        // Only invalidated when an IFC root's text content changes.
        let ifc_measure_cache = RefCell::new(std::mem::take(&mut self.tree.ifc_measure_cache));

        let t = web_time::Instant::now();
        self.tree
            .taffy
            .compute_layout_with_measure(
                root_taffy,
                available_space,
                |known_dims, avail_space, _node_id, context, _style| {
                    measure_calls.set(measure_calls.get() + 1);
                    let max_width = match avail_space.width {
                        taffy::AvailableSpace::Definite(w) => Some(w),
                        taffy::AvailableSpace::MaxContent => None,
                        taffy::AvailableSpace::MinContent => Some(0.0),
                    };

                    match context {
                        Some(NodeContext::Text(text)) => {
                            if text.content.is_empty() {
                                return taffy::Size {
                                    width: 0.0,
                                    height: 0.0,
                                };
                            }

                            // Skip Parley measurement for text in collapsed blocks
                            if nodes[text.node_id].estimated_height.is_some()
                                || nodes[text.node_id]
                                    .parent
                                    .is_some_and(|p| nodes[p].estimated_height.is_some())
                            {
                                return taffy::Size::ZERO;
                            }

                            shape_text.set(shape_text.get() + 1);
                            let mut builder =
                                layout_cx.ranged_builder(font_cx, &text.content, 1.0, true);
                            builder.push_default(parley::style::StyleProperty::FontSize(
                                text.font_size,
                            ));
                            if (text.font_weight - 400.0).abs() > 1.0 {
                                builder.push_default(parley::style::StyleProperty::FontWeight(
                                    parley::style::FontWeight::new(text.font_weight),
                                ));
                            }
                            if let Some(lh) =
                                layout::css_line_height_to_parley(&text.line_height_css)
                            {
                                builder.push_default(parley::style::StyleProperty::LineHeight(lh));
                            }
                            let font_stack = if !text.font_family.is_empty() {
                                std::borrow::Cow::Owned(text.font_family.clone())
                            } else {
                                std::borrow::Cow::Borrowed("sans-serif")
                            };
                            builder.push_default(parley::style::StyleProperty::FontFamily(
                                parley::style::FontFamily::Source(font_stack),
                            ));
                            // Add brush so the cached layout can be rendered with color
                            builder.push_default(parley::style::StyleProperty::Brush(
                                Brush::Solid(text.color),
                            ));
                            // Apply overflow-wrap for emergency line-breaking
                            builder.push_default(parley::style::StyleProperty::OverflowWrap(
                                text.overflow_wrap.to_parley(),
                            ));
                            // letter-/word-spacing (#698). The builder's scale
                            // is 1.0 here, so these are CSS pixels either way.
                            builder.push_default(parley::style::StyleProperty::LetterSpacing(
                                text.letter_spacing,
                            ));
                            builder.push_default(parley::style::StyleProperty::WordSpacing(
                                text.word_spacing,
                            ));
                            let mut layout = builder.build(&text.content);
                            // If no_wrap is set (white-space: nowrap), don't constrain width
                            let wrap_width = if text.no_wrap {
                                None
                            } else {
                                known_dims.width.or(max_width)
                            };
                            layout.break_all_lines(wrap_width);

                            // Cache the layout for use during paint
                            // Use wrap_width bits as part of the key since layout depends on it
                            let wrap_bits = wrap_width.map(|w| w.to_bits()).unwrap_or(u32::MAX);
                            text_layout_cache
                                .borrow_mut()
                                .insert((text.node_id, wrap_bits), layout);

                            taffy::Size {
                                width: known_dims.width.unwrap_or_else(|| {
                                    text_layout_cache
                                        .borrow()
                                        .get(&(text.node_id, wrap_bits))
                                        .map(|l| l.width())
                                        .unwrap_or(0.0)
                                }),
                                height: known_dims.height.unwrap_or_else(|| {
                                    text_layout_cache
                                        .borrow()
                                        .get(&(text.node_id, wrap_bits))
                                        .map(|l| l.height())
                                        .unwrap_or(0.0)
                                }),
                            }
                        }
                        Some(NodeContext::Image { width, height, .. }) => {
                            let iw = *width as f32;
                            let ih = *height as f32;
                            if iw == 0.0 || ih == 0.0 {
                                // Image still loading — return zero size
                                return taffy::Size::ZERO;
                            }
                            let aspect = iw / ih;
                            // Use intrinsic dimensions as default, but respect
                            // CSS width/height if set (via known_dims from Taffy style).
                            // Maintain aspect ratio when only one dimension is constrained.
                            let w = match (known_dims.width, known_dims.height) {
                                (Some(kw), _) => kw,
                                (None, Some(kh)) => kh * aspect,
                                (None, None) => iw,
                            };
                            let h = match (known_dims.height, known_dims.width) {
                                (Some(kh), _) => kh,
                                (None, Some(kw)) => kw / aspect,
                                (None, None) => ih,
                            };
                            taffy::Size {
                                width: w,
                                height: h,
                            }
                        }
                        Some(NodeContext::InlineRoot(root_id)) => {
                            let root_id = *root_id;

                            // Collapsed block (virtualized) — return estimated size
                            // without doing any Parley work.
                            //
                            // This early return only runs at all because the
                            // node is a Taffy leaf — Taffy never consults a
                            // measure function on a node with children (the
                            // IFC leaf invariant, #466; see
                            // `NodeContext::InlineRoot`). A non-leaf
                            // virtualized root would silently get 0 from the
                            // block algorithm instead of its estimate.
                            if let Some(est_h) = nodes[root_id].estimated_height {
                                return taffy::Size {
                                    width: known_dims.width.unwrap_or(0.0),
                                    height: known_dims.height.unwrap_or(est_h),
                                };
                            }

                            // Use wrap_width bits as cache key
                            let wrap_bits = max_width.map(|w| w.to_bits()).unwrap_or(u32::MAX);

                            // Check persistent IFC measure cache — skip expensive
                            // Parley rebuild if this root's text hasn't changed.
                            {
                                let cached = ifc_measure_cache
                                    .borrow()
                                    .get(&root_id)
                                    .and_then(|e| e.get(wrap_bits));
                                if let Some((cached_w, cached_h)) = cached {
                                    cache_hits.set(cache_hits.get() + 1);
                                    return taffy::Size {
                                        width: known_dims.width.unwrap_or(cached_w),
                                        height: known_dims.height.unwrap_or(cached_h),
                                    };
                                }
                            }

                            // Full Parley rebuild (text changed or cache miss)
                            shape_ifc.set(shape_ifc.get() + 1);
                            let inline_layout = Self::build_inline_layout(
                                nodes, root_id, max_width, 1.0, font_cx, layout_cx,
                            );
                            let mut h = hang.get();
                            h.passes += inline_layout.hang.passes;
                            h.lines += inline_layout.hang.lines;
                            hang.set(h);
                            let w = inline_layout.measured_width();
                            let h = inline_layout.layout.height();

                            // Store in persistent cache
                            ifc_measure_cache
                                .borrow_mut()
                                .entry(root_id)
                                .or_default()
                                .insert(wrap_bits, (w, h));

                            // Measure callback for IFC root
                            taffy::Size {
                                width: known_dims.width.unwrap_or(w),
                                height: known_dims.height.unwrap_or(h),
                            }
                        }
                        _ => taffy::Size::ZERO,
                    }
                },
            )
            .unwrap();

        // Restore the persistent IFC measure cache (dirty_ifc_text_roots cleared after build_ifc)
        self.tree.ifc_measure_cache = ifc_measure_cache.into_inner();

        let perf = &self.tree.perf;
        perf.add(Counter::TaffyMeasureCalls, measure_calls.get());
        perf.add(Counter::ShapeMeasureIfc, shape_ifc.get());
        hang.get().record(perf);
        perf.add(Counter::ShapeMeasureText, shape_text.get());
        perf.add(Counter::IfcMeasureCacheHits, cache_hits.get());
        perf.add_elapsed(Counter::TimeTaffyComputeNs, t);

        text_layout_cache.into_inner()
    }

    /// Incremental version of `sync_text_contexts` — only processes text nodes
    /// that are in `dirty_nodes` or `dirty_text_contexts`. Used when IFC
    /// structure is unchanged (ifc_dirty=false) to avoid walking all text nodes.
    ///
    /// The second set is the one a **restyle** fills (#678): `dirty_nodes`
    /// records DOM mutations, and a recascade is not one, so a text node whose
    /// parent's `font-size` changed was measured out of a context built from the
    /// old one. See `NodeTree::dirty_text_contexts`.
    pub(crate) fn sync_dirty_text_contexts(&mut self) {
        let mut updates: Vec<(taffy::NodeId, usize, TextContextFields)> = Vec::new();

        for id in self
            .tree
            .dirty_nodes
            .iter()
            .copied()
            .chain(self.tree.dirty_text_contexts.iter().copied())
        {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            if !matches!(&node.kind, NodeKind::Text(_)) {
                continue;
            }
            let Some(taffy_id) = node.taffy_id else {
                continue;
            };
            let parent = node.parent.and_then(|p| self.tree.nodes.get(p));
            updates.push((taffy_id, id, TextContextFields::from_parent(parent)));
        }
        self.tree.dirty_text_contexts.clear();

        for (taffy_id, node_id, fields) in updates {
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id)
                && let NodeContext::Text(tm) = ctx
            {
                fields.apply_to(tm, node_id);
            }
        }
    }

    /// Refresh every text node's [`TextMeasure`] from its parent's computed
    /// style — the full pass, run when the IFC structure changed.
    ///
    /// [`TextContextFields`] is the property list, shared with
    /// [`Self::sync_dirty_text_contexts`] so the full and incremental passes
    /// cannot disagree about what a text node is measured with.
    /// [`Self::sync_text_contexts`] over a scoped pass's regions (the text
    /// nodes whose parent — and so whose inherited text style — can have
    /// changed with the structure), plus whatever a restyle queued in
    /// `dirty_text_contexts`.
    pub(crate) fn sync_scoped_text_contexts(&mut self, scope: &crate::ifc_scope::IfcScope) {
        let mut updates: Vec<(taffy::NodeId, usize, TextContextFields)> = Vec::new();
        for &(id, _, _) in &scope.region {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            if !matches!(&node.kind, NodeKind::Text(_)) {
                continue;
            }
            let Some(taffy_id) = node.taffy_id else {
                continue;
            };
            let parent = node.parent.and_then(|p| self.tree.nodes.get(p));
            updates.push((taffy_id, id, TextContextFields::from_parent(parent)));
        }
        for (taffy_id, node_id, fields) in updates {
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id)
                && let NodeContext::Text(tm) = ctx
            {
                fields.apply_to(tm, node_id);
            }
        }
        self.sync_dirty_text_contexts();
    }

    pub(crate) fn sync_text_contexts(&mut self) {
        // Every text node is refreshed below, so nothing stays owed.
        self.tree.dirty_text_contexts.clear();
        let mut updates: Vec<(taffy::NodeId, usize, TextContextFields)> = Vec::new();

        for (id, node) in &self.tree.nodes {
            if let NodeKind::Text(_) = &node.kind {
                let Some(taffy_id) = node.taffy_id else {
                    continue;
                };
                // Read from the parent's parsed computed_style, not from CSS strings.
                let parent = node.parent.and_then(|p| self.tree.nodes.get(p));
                updates.push((taffy_id, id, TextContextFields::from_parent(parent)));
            }
        }

        for (taffy_id, node_id, fields) in updates {
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id)
                && let NodeContext::Text(tm) = ctx
            {
                fields.apply_to(tm, node_id);
            }
        }
    }

    /// Clamp scroll offsets for all scroll containers to their valid range.
    /// After layout changes (e.g., viewport resize), a container's content or
    /// visible area may have changed, making the old scroll offset too large.
    fn clamp_scroll_offsets(&mut self) {
        use crate::computed_style::OverflowValue;
        // Collect (node_id, max_scroll) for nodes that need clamping
        let mut clamps: Vec<(usize, f64)> = Vec::new();
        for (node_id, _) in self.tree.nodes.iter() {
            let node = &self.tree.nodes[node_id];
            if !matches!(
                node.computed_style.overflow_y,
                OverflowValue::Auto | OverflowValue::Scroll
            ) {
                continue;
            }
            if node.scroll_offset == (0.0, 0.0) {
                continue;
            }
            let cs = &node.computed_style;
            // The one extent walk (#995): the range the wheel, the bars and
            // `scroll_height` answer. A walk of its own here — it was the
            // direct `children` only — took back on every layout pass the
            // range that one grants: an anonymous box's lines, a `display:
            // contents` wrapper's children, an IFC root's inline content.
            let content_height = crate::paint::scrollbar::content_extents(&self.tree, node_id).1;
            let pad_v = (cs.padding_top.to_px() + cs.padding_bottom.to_px()) as f64;
            let border_v = (cs.border_top_width.to_px() + cs.border_bottom_width.to_px()) as f64;
            let visible_h = (node.layout.height as f64 - pad_v - border_v).max(0.0);
            let max_scroll = (content_height - visible_h).max(0.0);
            if node.scroll_offset.1 > max_scroll {
                clamps.push((node_id, max_scroll));
            }
        }
        for (node_id, max_scroll) in clamps {
            self.tree.nodes[node_id].scroll_offset.1 = max_scroll;
            // Queue a deferred scroll notification so the clamp isn't a silent
            // mutation (#144). Coalesce per node (last value wins): layout can
            // resolve more than once per frame, and a consumer must see one
            // event per drain.
            if let Some(pending) = self
                .tree
                .pending_scroll_clamps
                .iter_mut()
                .find(|(id, _)| *id == node_id)
            {
                pending.1 = max_scroll;
            } else {
                self.tree.pending_scroll_clamps.push((node_id, max_scroll));
            }
        }
    }

    /// Read one anonymous block box's Taffy layout back into its `layout`.
    ///
    /// The box is outside the element tree (#566), so
    /// [`Self::read_layout_results`]'s recursion never reaches it — but it has
    /// a `taffy_id` and a real box, and paint draws its line at that rect.
    /// Its run's members are reached by the ordinary walk, through their real
    /// parent, so this deliberately does **not** recurse.
    fn read_layout_results_for_box(&mut self, anon_id: usize) {
        let Some(taffy_id) = self.tree.nodes.get(anon_id).and_then(|n| n.taffy_id) else {
            return;
        };
        let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id) else {
            return;
        };
        let new_layout = LayoutResult {
            x: taffy_layout.location.x,
            y: taffy_layout.location.y,
            width: taffy_layout.size.width,
            height: taffy_layout.size.height,
        };
        let node = &mut self.tree.nodes[anon_id];
        if node.layout != new_layout {
            node.layout = new_layout;
            self.tree.paint_dirty_nodes.push(anon_id);
        }
    }

    /// Clear the laid-out box of `node_id` and every node beneath it.
    ///
    /// For a subtree that generates no boxes at all — a `display: none` element
    /// and its descendants (#543). Keeps [`Self::read_layout_results`]'s
    /// bookkeeping: a node whose box actually changed is pushed to
    /// `paint_dirty_nodes` (its `prev_layout` still names the painted box), so
    /// the frame that hides something repaints where it used to be.
    ///
    /// Iterative: a hidden subtree is arbitrary author markup and may be deep.
    fn zero_subtree_layout(&mut self, node_id: usize) {
        let zero = LayoutResult::default();
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            let Some(node) = self.tree.nodes.get_mut(id) else {
                continue;
            };
            if node.layout != zero {
                node.layout = zero;
                self.tree.paint_dirty_nodes.push(id);
            }
            let children = &self.tree.nodes[id].children;
            stack.extend_from_slice(children);
        }
    }

    /// Recursively read Taffy layout results into node LayoutResult fields.
    pub(crate) fn read_layout_results(&mut self, node_id: usize) {
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();

        // A `display: contents` element generates no box. Force its layout to the
        // origin so every parent-chain accumulation (paint tree-walk, hit-testing,
        // compute_absolute_position) treats it as fully transparent — its
        // children's positions are already relative to the nearest real ancestor.
        // `sync_display_contents` detaches the wrapper's Taffy node and marks it
        // display:none; that detached node can retain a stale non-zero `location`
        // (or make `taffy.layout()` return `Err`, skipping the read below), either
        // of which would otherwise be double-counted onto every descendant.
        if self.tree.nodes[node_id].computed_style.display
            == crate::computed_style::DisplayValue::Contents
        {
            let node = &mut self.tree.nodes[node_id];
            let zero = LayoutResult::default();
            if node.layout != zero {
                node.layout = zero;
                self.tree.paint_dirty_nodes.push(node_id);
            }
            for child_id in children {
                self.read_layout_results(child_id);
            }
            return;
        }

        // A **split inline** generates no box of its own either (#513). CSS 2.1
        // §9.2.1.1 breaks it into one fragment per side of the block-level
        // content; rinch models the fragments' *geometry*, through the anonymous
        // block boxes that lay each run out, and does not model fragment
        // identity — so there is no single rect this element could honestly
        // carry, and it carries none.
        //
        // This has to be said here, and the reason is the reason `Contents`
        // above needs the same line: the element's Taffy node is detached, and
        // Taffy keeps serving a detached node the layout it last computed. On a
        // `block → inline` restyle that stale box is real, and every upward
        // coordinate sum goes through this node (`box_tree_parent` deliberately
        // still steps through it, so a click or a `data-nofocus` region on the
        // element is still found), so the stale rect would be added to every
        // descendant's painted position. Zeroed, the sum is exact.
        //
        // `E ghost box` in `taffy_tree_violations` enforces it rather than
        // trusting it — which is the point: "the element happens to be 0x0" is
        // the fixed point mutants in this region hide on, and an assertion is
        // not a fixed point.
        //
        // Recurses rather than returning, like `Contents` and unlike
        // `display: none`: the boxes *inside* a split inline are real and are
        // laid out by the container, so every descendant still needs its own
        // read.
        if self.tree.nodes[node_id].is_split_inline() {
            let node = &mut self.tree.nodes[node_id];
            let zero = LayoutResult::default();
            if node.layout != zero {
                node.layout = zero;
                self.tree.paint_dirty_nodes.push(node_id);
            }
            for child_id in children {
                self.read_layout_results(child_id);
            }
            return;
        }

        // A **flowed inline element** — a `<span>` whose content an IFC lays out
        // — owns no box either (#591, [`crate::node::Node::is_flowed_inline_element`]).
        // Same hazard as the two branches above, same cure: the marking pass
        // detached its Taffy node, Taffy keeps serving a detached node the
        // layout it last computed, and nothing ever writes this element's
        // `layout` on purpose. So `<span style="display: block">` restyled to
        // `inline` read back its block pass's `400x20` every pass (measured),
        // and under a padded container that stale `(17,13)` origin is added to
        // every descendant's painted position — measured on the base with an
        // `inline-block` child: it lands at `(92,23)` where the declared twin's
        // lands at `(75,10)`, a clean `(17,13)` double-count
        // (`ifc_reattach_tests::a_wrapper_restyled_to_inline_drops_the_box_its_block_pass_left`).
        // The same sum is what #591's absolutely positioned child reaches the
        // stacking root through once PR 2 hoists it: with the hoist spiked and
        // this zeroing absent it landed at `(34,46)` against Chrome's `(17,33)`.
        // `E ghost box` enforces the zero.
        //
        // Recurses: an atomic inline inside the span carries a real IFC-assigned
        // box and a direct text child of the root is stretched, and both of those
        // reads happen below for the descendants.
        if self.tree.nodes[node_id].is_flowed_inline_element() {
            let node = &mut self.tree.nodes[node_id];
            let zero = LayoutResult::default();
            if node.layout != zero {
                node.layout = zero;
                self.tree.paint_dirty_nodes.push(node_id);
            }
            for child_id in children {
                self.read_layout_results(child_id);
            }
            return;
        }

        // A `display: none` element generates no box, and neither does anything
        // inside it (CSS 2.1 §9.2.4) — so the whole subtree's `layout` is zero,
        // and this is the place that has to say so (#543).
        //
        // Taffy normally says it for us: a hidden node that is still a Taffy
        // child is laid out `0x0` and the read below picks that up. But a hidden
        // child of an IFC root is **detached** from Taffy on purpose
        // (`mark_inline_descendants`' `NoBox` arm, for #466's leaf invariant),
        // and `taffy.layout()` keeps serving a detached node the layout it last
        // computed. This walk is over the **DOM**, so it reaches the node anyway
        // and wrote that stale box straight back onto it, every pass, for as long
        // as the element stayed hidden. A closed `DropdownMenu` therefore stayed
        // on screen indefinitely — and, since `hit_test_node` reads `layout` and
        // tests `visibility` rather than `display`, stayed clickable: a click in
        // the ghost ran a menu item's handler.
        //
        // The subtree, not the node. `check_children` is
        // `!clips_overflow() || point_in_bounds`, so a non-clipping box with a
        // zero rect is still descended into and a descendant that kept its own
        // box still answers. Recursing with the zero applied at every level is
        // what makes the whole thing gone rather than just its root.
        //
        // It returns rather than recursing, unlike the `Contents` branch above,
        // and that is safe for the one reason the other branch could not use:
        // every node below a hidden one gets the *same* answer, zero, so there
        // is nothing further down for the normal path to compute. A
        // `display: contents` node inside the hidden subtree is zeroed by the
        // helper exactly as that branch would zero it. The helper carries this
        // function's bookkeeping (`paint_dirty_nodes`) so the
        // frame that hides something still repaints where it used to be.
        if self.tree.nodes[node_id].computed_style.display
            == crate::computed_style::DisplayValue::None
        {
            self.zero_subtree_layout(node_id);
            return;
        }

        if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id
            && let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id)
        {
            let mut new_layout = LayoutResult {
                x: taffy_layout.location.x,
                y: taffy_layout.location.y,
                width: taffy_layout.size.width,
                height: taffy_layout.size.height,
            };

            // position: fixed elements use the viewport as their containing block.
            // Taffy treats them as absolute (relative to parent), so we override
            // their layout to be viewport-relative with proper sizing from insets.
            // Skip when:
            //   - display is none (element itself is hidden)
            //   - Taffy computed 0x0 size (ancestor has display:none — the node's
            //     own display may be Block but it's inside a hidden subtree)
            {
                let node = &self.tree.nodes[node_id];
                if node.computed_style.position == crate::computed_style::PositionValue::Fixed
                    && !matches!(
                        node.computed_style.display,
                        crate::computed_style::DisplayValue::None
                    )
                    && (new_layout.width > 0.0 || new_layout.height > 0.0)
                {
                    let vw = self.tree.viewport.width;
                    let vh = self.tree.viewport.height;
                    let style = &node.computed_style;

                    // Resolve insets (top/right/bottom/left)
                    let top = style.top.resolve(vh);
                    let right = style.right.resolve(vw);
                    let bottom = style.bottom.resolve(vh);
                    let left = style.left.resolve(vw);
                    let width_auto = style.width.lays_out_as_auto();
                    let height_auto = style.height.lays_out_as_auto();

                    // Horizontal positioning
                    if let (Some(l), Some(r)) = (left, right) {
                        new_layout.x = l;
                        if width_auto {
                            new_layout.width = (vw - l - r).max(0.0);
                        }
                    } else if let Some(l) = left {
                        new_layout.x = l;
                    } else if let Some(r) = right {
                        new_layout.x = (vw - new_layout.width - r).max(0.0);
                    } else {
                        new_layout.x = 0.0;
                    }

                    // Vertical positioning
                    if let (Some(t), Some(b)) = (top, bottom) {
                        new_layout.y = t;
                        if height_auto {
                            new_layout.height = (vh - t - b).max(0.0);
                        }
                    } else if let Some(t) = top {
                        new_layout.y = t;
                        // Taffy may compute wrong height for fixed elements
                        // (their Taffy parent differs from the CSS containing
                        // block which should be the viewport).  Re-derive
                        // content height from children.
                        if height_auto {
                            let _ = node;
                            new_layout.height = self.compute_content_height(node_id, &new_layout);
                        }
                    } else if let Some(b) = bottom {
                        if height_auto {
                            let _ = node;
                            new_layout.height = self.compute_content_height(node_id, &new_layout);
                        }
                        new_layout.y = (vh - new_layout.height - b).max(0.0);
                    } else {
                        new_layout.y = 0.0;
                    }
                }
            }

            // An absolutely positioned box with no positioned ancestor resolves
            // against the initial containing block — the viewport at the origin
            // — not against its direct parent, which is the only containing
            // block Taffy knows (issue #204). Its *size* was already baked from
            // the viewport before layout (`out_of_flow`); this places it.
            //
            // The correction is written as a **parent-relative delta**, not as
            // a viewport-absolute coordinate the way `fixed` above is: with
            //     abs(node) = layout.x + abs(parent) - parent.scroll_offset.x
            // writing `target - abs(parent) + parent.scroll_offset.x` leaves
            // `LayoutResult` parent-relative, so every coordinate walk in the
            // codebase — paint, stacking, hit testing, ClickContext, the MCP
            // `absolute` contract — keeps working untouched, and layout agrees
            // with paint by construction because it reuses paint's own sum.
            {
                let node = &self.tree.nodes[node_id];
                if node.computed_style.position == crate::computed_style::PositionValue::Absolute
                    && (new_layout.width > 0.0 || new_layout.height > 0.0)
                    && crate::out_of_flow::out_of_flow_kind(&self.tree, node_id)
                        == Some(crate::out_of_flow::OutOfFlowKind::IcbAbsolute)
                {
                    let vw = self.tree.viewport.width;
                    let vh = self.tree.viewport.height;
                    // The **box-tree** parent (#591): a hoisted out-of-flow box's
                    // `layout` is relative to its host, not its DOM parent.
                    let (parent_abs, parent_scroll) =
                        match Self::box_tree_parent(&self.tree.nodes, node_id) {
                            Some(parent_id) => {
                                let (px, py) = crate::paint::compute_absolute_position(
                                    &self.tree, parent_id, 1.0,
                                );
                                let scroll = self.tree.nodes[parent_id].scroll_offset;
                                ((px as f32, py as f32), (scroll.0 as f32, scroll.1 as f32))
                            }
                            None => ((0.0, 0.0), (0.0, 0.0)),
                        };

                    let style = &node.computed_style;
                    let left = style.left.resolve(vw);
                    let right = style.right.resolve(vw);
                    let top = style.top.resolve(vh);
                    let bottom = style.bottom.resolve(vh);
                    // Percentage margins resolve against the containing block's
                    // *width* on both axes, per CSS.
                    let margin_left = style.margin_left.resolve(vw).unwrap_or(0.0);
                    let margin_right = style.margin_right.resolve(vw).unwrap_or(0.0);
                    let margin_top = style.margin_top.resolve(vw).unwrap_or(0.0);
                    let margin_bottom = style.margin_bottom.resolve(vw).unwrap_or(0.0);

                    // Only correct an axis that has a real inset. With both
                    // insets `auto` the target is `None` and the box keeps
                    // Taffy's static position — which CSS *does* take from the
                    // flow position in the DOM parent, so Taffy's answer is the
                    // right one there.
                    let target_x = match (left, right) {
                        (Some(l), _) => Some(l + margin_left),
                        (None, Some(r)) => Some(vw - r - margin_right - new_layout.width),
                        (None, None) => None,
                    };
                    if let Some(x) = target_x {
                        new_layout.x = x - parent_abs.0 + parent_scroll.0;
                    }
                    let target_y = match (top, bottom) {
                        (Some(t), _) => Some(t + margin_top),
                        (None, Some(b)) => Some(vh - b - margin_bottom - new_layout.height),
                        (None, None) => None,
                    };
                    if let Some(y) = target_y {
                        new_layout.y = y - parent_abs.1 + parent_scroll.1;
                    }
                }
            }

            // An atomic inline child of an IFC has its *position* assigned by the IFC
            // (`write_inline_positions`), not Taffy: it is detached from its parent's
            // Taffy tree and measured standalone (Taffy location 0,0). Keep the IFC's
            // x/y here — only the size comes from the standalone measure. Without
            // this, a non-structural re-layout (which doesn't rebuild the IFC) snaps
            // every inline-block back to the line origin, collapsing e.g. a row of
            // inline-block buttons into a pile.
            {
                let node = &self.tree.nodes[node_id];
                if node.display_mode.is_atomic_inline() && node.ifc_root.is_some() {
                    new_layout.x = node.layout.x;
                    new_layout.y = node.layout.y;
                }
            }

            // `prev_layout` is deliberately not written: it is the box this
            // node was last *painted* in, and only the paint that consumes
            // `paint_dirty_nodes` may move it (`NodeTree::consume_paint_dirty`).
            // A second resolve before that paint must not overwrite it, or the
            // old rect is lost and the move ghosts.
            let node = &mut self.tree.nodes[node_id];
            if node.layout != new_layout {
                node.layout = new_layout;
                self.tree.paint_dirty_nodes.push(node_id);
            }
        }

        for child_id in children {
            self.read_layout_results(child_id);
        }
    }

    /// Compute the intrinsic content height of a node from its children's
    /// Taffy-computed sizes. Used for position:fixed elements where Taffy's
    /// parent-relative sizing gives the wrong result.
    fn compute_content_height(&self, node_id: usize, _parent_layout: &LayoutResult) -> f32 {
        let node = &self.tree.nodes[node_id];
        let pad_top = node.computed_style.padding_top.to_px();
        let pad_bottom = node.computed_style.padding_bottom.to_px();
        let border_top = node.computed_style.border_top_width.to_px();
        let border_bottom = node.computed_style.border_bottom_width.to_px();
        let gap = node.computed_style.gap_row.to_px();
        // Children stack vertically (heights sum) when the container is a block
        // box or a flex column; a flex row lays them side by side (take the max).
        // Without the `Block` case a `position: fixed` block with auto height —
        // e.g. a popup/menu appended to <body> — collapses to one child's height.
        let is_column = matches!(
            node.computed_style.display,
            crate::computed_style::DisplayValue::Block
        ) || matches!(
            node.computed_style.flex_direction,
            crate::computed_style::FlexDirectionValue::Column
                | crate::computed_style::FlexDirectionValue::ColumnReverse
        );

        let mut content_h: f32 = 0.0;
        let child_count = node.children.len();

        for (i, &child_id) in node.children.iter().enumerate() {
            if let Some(child_taffy) = self.tree.nodes.get(child_id).and_then(|c| c.taffy_id) {
                if let Ok(child_layout) = self.tree.taffy.layout(child_taffy) {
                    let ch = child_layout.size.height;
                    if is_column {
                        content_h += ch;
                        if i > 0 && i < child_count {
                            content_h += gap;
                        }
                    } else {
                        content_h = content_h.max(ch);
                    }
                }
            }
        }

        let mut h = content_h + pad_top + pad_bottom + border_top + border_bottom;

        // Respect the element's own min/max-height (border-box, like the rest of
        // the box model here) so a fixed scroll container clamps and scrolls its
        // overflow instead of growing past its cap.
        use crate::computed_style::DimensionValue;
        let vh = self.tree.viewport.height;
        let resolve = |d: &DimensionValue| -> Option<f32> {
            match d {
                DimensionValue::Length(v) => Some(*v),
                DimensionValue::Percent(p) => Some(p * vh),
                DimensionValue::Calc { px, pct } => Some(px + pct * vh),
                // An intrinsic keyword lays out as `auto` (#626), so it
                // constrains nothing here either.
                DimensionValue::Auto | DimensionValue::Intrinsic(_) => None,
            }
        };
        if let Some(max_h) = resolve(&node.computed_style.max_height) {
            h = h.min(max_h);
        }
        if let Some(min_h) = resolve(&node.computed_style.min_height) {
            h = h.max(min_h);
        }
        h
    }

    /// Handle display:contents nodes by reparenting their taffy children
    /// to the nearest non-display-contents ancestor in the taffy tree.
    ///
    /// This function is **idempotent**: it rebuilds the taffy children list from
    /// the DOM structure each time, so calling it multiple times produces the
    /// same result. Nested display:contents (e.g. from `else if` chains) are
    /// handled by recursively flattening.
    ///
    /// A parent is rebuilt when its contents-descendant status changed in
    /// **either direction** (#520): entering (a child now computes `Contents`)
    /// or leaving (a child a previous pass spliced no longer does —
    /// `Node::contents_spliced`). A departed wrapper additionally gets its own
    /// Taffy child list rebuilt, because the splice moved its children's Taffy
    /// ids into the ancestor's list and nothing else gives them back.
    ///
    /// With a `scope` (`crate::ifc_scope`), only the wrappers in the scope's
    /// regions are looked at. A `display: contents` wrapper is never a
    /// formatting container, so every wrapper whose splice could have moved —
    /// its children changed, it moved, it crossed into or out of `contents` —
    /// is in the region of a container of the scope, and so is the ancestor
    /// that holds its splice. Everything else keeps its splice, and the rows
    /// that hold one are not dirtied (they used to be: every wrapper in the
    /// document was re-spliced, with a `set_children` and a `mark_dirty` per
    /// child, on every structural change).
    pub(crate) fn sync_display_contents(&mut self, scope: Option<&crate::ifc_scope::IfcScope>) {
        use crate::computed_style::values::DisplayValue;

        // Find all display:contents nodes and their nearest non-contents ancestors.
        // We rebuild the taffy children of each affected ancestor from scratch.
        //
        // The check uses the resolved `computed_style.display`, not the raw inline
        // `style` attribute, so that `display: contents` set via a CSS class (e.g.
        // `.rinch-context-menu { display: contents }`) is treated the same as the
        // inline form. Without this, class-based contents elements stay in the
        // Taffy tree as ordinary boxes and trap their children inside a 0×0 (or
        // 2×2 text-sized) parent — see issue #25.
        let mut affected_parents: Vec<usize> = Vec::new();
        let mut all_contents_nodes: Vec<usize> = Vec::new();
        // Nodes a previous pass spliced as `display: contents` that no longer
        // compute `Contents` (#520): the wrapper's box must come back and its
        // children must stop contributing to the flattening ancestor.
        let mut departed_nodes: Vec<usize> = Vec::new();

        let candidates: Vec<usize> = match scope {
            None => self.tree.nodes.iter().map(|(id, _)| id).collect(),
            // Ascending, as the slab walk visits them. Order matters here and
            // should not: a wrapper's splice reads `contributes_in_flow_block`
            // from the *previous* pass (this runs before it is recomputed), so
            // an element that just stopped being a flex container can read as a
            // split inline and have its children flattened into the ancestor
            // — and it is the order in which the affected parents are rebuilt
            // that decides whether its own list gets them back. The scope's
            // set iterates in hash order; the whole-document pass in id order.
            // (Found by the random differential, which tripped over exactly
            // that and orphaned a text node.)
            Some(scope) => {
                let mut v: Vec<usize> = scope.nodes.iter().copied().collect();
                v.sort_unstable();
                v
            }
        };
        for id in candidates {
            let Some(node) = self.tree.nodes.get(id) else {
                continue;
            };
            // This pass maintains the DOM flattening of **author** wrappers,
            // and asking an anonymous block box which wrapper holds it is not
            // an unanswerable question — it is the wrong one (#566). Its Taffy
            // attachment is `create_anonymous_block_boxes`' own job, rebuilt
            // explicitly there. Same guard, same reason, as the one in that
            // function's own scan.
            //
            // The guard holds for **both** shapes this design has had, by two
            // different mechanisms, which is why it keys on
            // `is_anonymous_block_box` and not on either of them: with an
            // earlier `parent: None` the walk terminated immediately and
            // recorded nothing; with `parent = Some(container)`, which is what
            // it does now, the walk *succeeds* and double-adds the container to
            // `affected_parents`.
            //
            // **Defence, not a fix, and that is measured**: the mutant that
            // deletes it survives the entire workspace. I predicted the
            // opposite in the design audit ("a real regression if left alone")
            // and was wrong.
            //
            // Read that survival precisely, because it is weaker than it
            // looks. What is established is the **absence of a distinguishing
            // fixture**, not a demonstration that the guard is inert. The
            // account I have for why — a box reaches the `is_contents` arm
            // only by inheriting `display: contents` from a boxless container,
            // and in that state the collector flattens it to its run so
            // nothing consults its Taffy node — is an argument, and the
            // suite's silence is consistent with it being wrong in a shape
            // nobody has written down. The guard stays because it is cheap and
            // because that argument is the only thing standing between the
            // double-add and a `parents_affected` list with a duplicate in it.
            if node.is_anonymous_block_box {
                continue;
            }
            let is_contents = node.computed_style.display == DisplayValue::Contents;
            if is_contents {
                all_contents_nodes.push(id);
            } else if node.contents_spliced {
                departed_nodes.push(id);
            } else {
                continue;
            }

            // Walk up to find nearest non-display-contents ancestor. For a
            // departed node this is the ancestor still holding the splice (or
            // about to hold the wrapper's own box) — it needs the same rebuild
            // a current contents node's ancestor does.
            let mut ancestor = node.parent;
            while let Some(anc_id) = ancestor {
                let anc_is_contents =
                    self.tree.nodes[anc_id].computed_style.display == DisplayValue::Contents;
                if !anc_is_contents {
                    if !affected_parents.contains(&anc_id) {
                        affected_parents.push(anc_id);
                    }
                    break;
                }
                ancestor = self.tree.nodes[anc_id].parent;
            }
        }

        // Rebuild each departed wrapper's own Taffy child list (#520). Taffy's
        // `set_children` removes each adopted child from its previous parent,
        // so this both restores the wrapper's box contents and pulls the
        // spliced ids out of whatever list still holds them — including a
        // parent the wrapper was already detached from (a toggle followed by a
        // removal before any layout pass), which the affected-parents rebuild
        // below cannot reach because a detached wrapper has no ancestors.
        for &node_id in &departed_nodes {
            self.tree.nodes[node_id].contents_spliced = false;
            let node_taffy = match self.tree.nodes[node_id].taffy_id {
                Some(t) => t,
                None => continue,
            };
            let new_children = Self::collect_effective_taffy_children(&self.tree.nodes, node_id);
            let _ = self.tree.taffy.set_children(node_taffy, &new_children);
            for &child_taffy in &new_children {
                let _ = self.tree.taffy.mark_dirty(child_taffy);
            }
            let _ = self.tree.taffy.mark_dirty(node_taffy);
        }

        // For each affected parent, rebuild its taffy children by flattening
        // display:contents nodes recursively.
        for parent_id in &affected_parents {
            let parent_taffy = match self.tree.nodes[*parent_id].taffy_id {
                Some(t) => t,
                None => continue,
            };

            let new_children = Self::collect_effective_taffy_children(&self.tree.nodes, *parent_id);
            let _ = self.tree.taffy.set_children(parent_taffy, &new_children);

            // Also mark each reparented child dirty so their own caches are
            // cleared (they may have stale entries from their old position).
            for &child_taffy in &new_children {
                let _ = self.tree.taffy.mark_dirty(child_taffy);
            }
        }

        // Force the root Taffy node dirty to guarantee a full recompute.
        // Taffy's mark_dirty propagation stops at already-empty ancestors,
        // which can leave stale cached layouts when available space hasn't
        // changed but children have been swapped.
        if !affected_parents.is_empty() || !departed_nodes.is_empty() {
            if let Some(root_taffy) = self.tree.nodes[self.tree.root_id].taffy_id {
                let _ = self.tree.taffy.mark_dirty(root_taffy);
            }
        }

        // Set all display:contents nodes' taffy to display:none so they don't
        // participate in layout themselves. `contents_spliced` records that
        // this pass owns the node's Taffy contribution, so a later pass (or a
        // detach path) can tell a healed wrapper from a stale-spliced one.
        for node_id in all_contents_nodes {
            self.tree.nodes[node_id].contents_spliced = true;
            if let Some(node_taffy) = self.tree.nodes[node_id].taffy_id {
                let _ = self
                    .tree
                    .taffy
                    .set_style(node_taffy, crate::node::display_contents_taffy_style());
            }
        }
    }

    /// `node_id`'s parent as the **box tree** sees it — the companion to
    /// [`Self::box_tree_children`], and required by exactly the same rule
    /// (#566).
    ///
    /// A run's member has the anonymous box as its box-tree parent even though
    /// its DOM parent is the container. The box's own parent needs no
    /// correction — it keeps a real upward edge to its container and is merely
    /// absent from that container's `children`.
    ///
    /// **Every coordinate accumulation that walks upward must use this.** A
    /// member's `layout` is positioned by the IFC *relative to the box*, so a
    /// parent-chain sum that steps straight to the container drops the box's
    /// own offset — the run lands at the container's origin instead of the
    /// run's. That is invisible for a document's first run, whose box is at
    /// `y = 0`, and wrong for every one after it: it was found by a focus test
    /// clicking a toolbar `<input>` and hitting the node above it.
    pub fn box_tree_parent(nodes: &slab::Slab<crate::node::Node>, node_id: usize) -> Option<usize> {
        let node = nodes.get(node_id)?;
        // A hoisted out-of-flow box is held by its host's lists, not its DOM
        // parent's (#591): its `layout` is relative to the host, so every
        // coordinate sum steps there. `Node::hoisted_out_of_flow_to`.
        if let Some(host) = node.hoisted_out_of_flow_to {
            return Some(host);
        }
        if let Some(b) = node.run_box {
            return Some(b);
        }
        // A box needs no arm of its own: it keeps a real `parent` edge to its
        // container (it is simply absent from that container's `children`), so
        // the ordinary answer is already right.
        node.parent
    }

    /// `node_id`'s children as the **box tree** sees them: its DOM children,
    /// with each inline run replaced — at the position of its first member — by
    /// the anonymous block box that stands for it (#566).
    ///
    /// # Why this exists
    ///
    /// There are two trees here and they are not the same tree. The **element**
    /// tree is the author's: it is what `parent`/`children` hold, what CSS
    /// inheritance and selector matching walk, and what `insert_before` and
    /// `next_sibling` answer from. The **box** tree is what gets laid out and
    /// painted, and it contains anonymous block boxes, which are not elements
    /// (CSS 2.1 §9.2.1.1).
    ///
    /// This engine used to conflate them by putting the box *in* `children` and
    /// reparenting the run into it. That is #566, #579 and the inheritance
    /// defect, all three: every single-level read of the element tree got the
    /// box where the container should be. The box is now outside the element
    /// tree entirely, and **this function is the only place the two trees are
    /// reconciled**.
    ///
    /// # Everything that walks boxes must come through here
    ///
    /// Paint's descent, hit testing, the stacking sequence, `layer_bounds`, the
    /// viewport-hole walk and [`Self::collect_effective_taffy_children`] all
    /// used to walk `node.children` directly and get the box for free. They no
    /// longer can. Routing them through one function rather than open-coding
    /// the interleave six times is deliberate: four sites disagreeing about
    /// "does this node clip" produced #324, and four disagreeing about "what
    /// are this node's Taffy children" produced #476. **A new question with six
    /// consumers gets one answer, not six.**
    ///
    /// # The rule, and why a run is not a contiguous range
    ///
    /// A member carries [`crate::node::Node::run_box`]. The first member of a
    /// run yields its box; later members of the *same* box yield nothing. It is
    /// written that way rather than as "replace a contiguous slice" because a
    /// run is **not** contiguous in `children`: an out-of-flow or `display:
    /// none` child sits inside one without joining it (#406, #366), so
    /// `text <abs/> text` is one run with a non-member between its members.
    ///
    /// **Borrows** for the overwhelmingly common case of a node with no run
    /// among its children, so the per-frame walks that call it pay nothing.
    pub fn box_tree_children(
        nodes: &slab::Slab<crate::node::Node>,
        node_id: usize,
    ) -> std::borrow::Cow<'_, [usize]> {
        use std::borrow::Cow;
        let Some(node) = nodes.get(node_id) else {
            return Cow::Borrowed(&[]);
        };
        // An anonymous box's box-tree children **are** its run, verbatim — no
        // substitution, or every member would be replaced by the box that owns
        // it and the walk would name itself.
        //
        // This is reached wherever something descends into a box: the collector
        // does when the box computes `display: contents` (which it inherits
        // from a boxless container, and which this design deliberately does not
        // change — that is #568's business, not #566's), and paint does on the
        // same path. Returning `children` there — empty, since the run is not
        // adopted — would drop the whole run from layout and from paint.
        if node.is_anonymous_block_box {
            return Cow::Borrowed(&node.run_members);
        }
        // **Borrow unless a run is actually present, and decide that in O(1).**
        // These walks run per node per frame — paint's descent, the stacking
        // sequence, `layer_bounds`, every Taffy rebuild — and mixed-content
        // containers are a small minority of nodes. The allocation this avoids
        // is the part that matters; the *scan* it also avoids — an earlier form
        // asked every child whether it carried a `run_box` — measured as
        // nothing, so `run_boxes.is_empty()` is a structural bound rather than
        // a speedup. That field is here for invariant A (see its own doc).
        //
        // **A split inline has to be flattened whether or not a run exists**
        // (#513). `<a><div>card</div></a>` — the commonest real shape, a block
        // link — holds no inline content at all, so `has_inline` is false, no
        // anonymous box is minted, and `run_boxes` stays empty. Returning
        // `children` there would put the `<a>`'s own box back in the box tree
        // and leave the block inside it exactly as orphaned as before the fix.
        // So the borrow is conditional on both, and the extra test is the same
        // per-child scan the comment above records as measuring at nothing.
        //
        // **And whenever a hoisted out-of-flow box is involved** (#591): a child
        // that hosts one (`hosts_hoisted_out_of_flow`) must have the box emitted
        // after it, and a child that *is* one (`hoisted_out_of_flow_to`) must be
        // omitted — this node is the inline element or wrapper the box sits in,
        // and the box is its host's. Both are one bit per child.
        if node.run_boxes.is_empty()
            && !node.children.iter().any(|&c| {
                nodes.get(c).is_some_and(|child| {
                    child.is_split_inline()
                        || child.hosts_hoisted_out_of_flow
                        || child.hoisted_out_of_flow_to.is_some()
                })
            })
        {
            return Cow::Borrowed(&node.children);
        }

        // **The units, not the children** (#568). A run is grouped over the
        // container's *flattened* child list, so a member may live behind a
        // `display: contents` wrapper and not appear in `children` at all —
        // iterating `children` would then find no member, emit no box, and drop
        // a whole run out of the box tree.
        //
        // **Three sites read this list and all three call the same function.**
        // `create_anonymous_block_boxes` groups runs out of it, this emits the
        // boxes into it, and `RinchDocument::run_bookkeeping_violations` checks
        // that every member is still a unit of it. They are one authority
        // rather than three spellings kept in step, because #518, #476 and #568
        // were each *two sites asking one question two ways*. If you change what
        // a unit is, those are the two that move with this one — and the third
        // is the one that fails loudly if they drift, which is why it asks
        // membership of this very list rather than some property a member
        // happens to have.
        //
        // A wrapper that survives as one unit is a run member itself and is
        // replaced by its box like any other; a wrapper that was broken up
        // vanishes here exactly as it does from the Taffy child list, since it
        // generates no box (CSS 2.1 §9.2.1.1) — its units stand in its place.
        let mut units: Vec<usize> = Vec::new();
        Self::collect_run_units(nodes, node_id, &mut units);

        let mut out: Vec<usize> = Vec::with_capacity(units.len());
        // A run is **not** contiguous in this list either: a comment or a
        // `display: none` child sits between two members of one run, so the box
        // must be emitted once for the whole run rather than once per
        // maximal stretch. Resetting on a non-member emits it twice and Taffy
        // panics on the duplicate in `set_children`.
        let mut last_box: Option<usize> = None;
        for unit in units {
            match nodes.get(unit).and_then(|c| c.run_box) {
                Some(b) => {
                    if last_box != Some(b) {
                        out.push(b);
                        last_box = Some(b);
                    }
                }
                None => out.push(unit),
            }
        }
        Cow::Owned(out)
    }

    /// **THE** answer to "which Taffy nodes are this DOM node's Taffy
    /// children" — the effective list, in DOM order, with every
    /// `display: contents` child replaced by the boxes it flattens (#476).
    ///
    /// A `display: contents` element generates no box, so its own `taffy_id`
    /// is never in the list and its grandchildren appear directly in the
    /// ancestor's; the recursion handles wrappers nested to any depth. The
    /// flattening key is `computed_style.display == Contents`, which selects
    /// the same nodes [`crate::node::Node::inline_flow_role`] answers
    /// `Contents` for — display before position, always (#366) — so for every
    /// node that can carry a box the flattening and every IFC decision agree.
    /// (`inline_flow_role` short-circuits on `is_comment()` *before* the
    /// display match, so a comment declaring `display: contents` would be the
    /// one node they classify differently. It cannot occur: a comment never
    /// goes through style resolution, so its `computed_style.display` keeps
    /// the default and its `taffy_id` is `None`.)
    ///
    /// **Every whole-list rebuild must derive its order from here.** There
    /// are four, and they run in this order inside `resolve_layout`'s
    /// `ifc_dirty` block:
    ///
    /// 1. [`Self::sync_display_contents`] — departed wrappers
    /// 2. [`Self::sync_display_contents`] — affected parents
    /// 3. `cleanup_anonymous_block_boxes` (`ifc.rs`) — parents that held an
    ///    anonymous box last pass
    /// 4. `create_anonymous_block_boxes` (`ifc.rs`) — mixed-content block
    ///    containers, and the anonymous boxes it mints
    ///
    /// 3 and 4 used to rebuild from **raw `nodes[parent].children`**, which
    /// cannot see the flattening, so they ran right after 1/2 and undid it:
    /// they re-added the wrapper's own boxless Taffy node and dropped the
    /// grandchildren it stands for. Because a flattened grandchild is not a
    /// DOM child of the parent, nothing re-added it anywhere — it was left
    /// **orphaned** in Taffy, laid out `0x0` and painted not at all, stably,
    /// on every subsequent pass (#476). What was spared is exactly what
    /// `create_anonymous_block_boxes` skips — `DisplayMode::Flex` — which is
    /// why the whole `display: contents` test suite, written over flex
    /// containers, stayed green. That is narrower than "only plain blocks":
    /// `display: grid` maps to `DisplayMode::Block` (`style_resolution`), so
    /// grid containers were affected too.
    ///
    /// The IFC **measure-leaf canonicalization** (`ifc.rs`, #466 PR2) is a
    /// deliberate exception and not a fifth *caller*: it is selecting the
    /// *out-of-flow* children of one IFC root, which is a different question,
    /// and it reads the DOM rather than the attachment on purpose (#477). It
    /// does its own contents flattening through `collect_contents_out_of_flow`.
    ///
    /// It **is** a fifth whole-list `set_children`, and #477 counts it as one:
    /// it is the pass that heals a late-inserted out-of-flow child, which is
    /// why that issue's forecast about #466 PR2 came out inverted. "Four" here
    /// means four rebuilds that must take their **order** from this function —
    /// not four places that replace a Taffy child list.
    ///
    /// **"THE answer" is scoped to rebuilds, and one neighbour is deliberately
    /// outside that scope**: [`Self::collect_taffy_contribution`], immediately
    /// below, walks the same flattening with a **wider gate** (`Contents` **or**
    /// `contents_spliced`). It is not a rival authority and not a bug — it
    /// answers a different question, *"which ids might this node be occupying
    /// right now"*, for the incremental insert index at **mutation time**
    /// (#477), where this function's `Contents`-only gate is momentarily wrong:
    /// a wrapper restyled away from `contents` between syncs still has its
    /// children in the parent's list, and asking here would answer with the
    /// wrapper's own detached id instead. Read the two together before changing
    /// either gate.
    pub(crate) fn collect_effective_taffy_children(
        nodes: &slab::Slab<crate::node::Node>,
        node_id: usize,
    ) -> Vec<taffy::NodeId> {
        use crate::computed_style::values::DisplayValue;

        let mut result = Vec::new();
        if nodes.get(node_id).is_none() {
            return result;
        }
        // The **box** tree, not the element tree (#566): an inline run is
        // represented by the anonymous box that lays it out, at the position of
        // its first member, and the members themselves contribute nothing.
        //
        // Taking `children` here instead would emit both. The members keep a
        // `taffy_id` — `mark_inline_descendants` detaches them from their Taffy
        // *parent* and never clears the id — so `set_children` would re-attach
        // every one of them beside the box, and the detach a few lines later
        // would miss them (it only removes from the box's own Taffy children,
        // where they are not). The run would be laid out twice: once as Taffy
        // blocks and once as an IFC line.
        //
        // `get`, not indexing, on both hops. The anonymous-box rebuilds this
        // replaced skipped an id missing from the slab (`nodes.get(child_id)`),
        // and routing them here must not turn that into a panic — a DOM
        // `children` list holding a freed id is a bug, but a wrong picture beats
        // a crash and this function is not where it should be discovered.
        for &child_id in Self::box_tree_children(nodes, node_id).iter() {
            let Some(child) = nodes.get(child_id) else {
                continue;
            };
            if child.computed_style.display == DisplayValue::Contents {
                // Recursively flatten: add grandchildren directly
                result.extend(Self::collect_effective_taffy_children(nodes, child_id));
            } else if let Some(child_taffy) = child.taffy_id {
                result.push(child_taffy);
            }
        }
        result
    }

    /// The node whose effective Taffy child list holds `node_id`'s box — the
    /// **inverse** of [`Self::collect_effective_taffy_children`], and required
    /// to agree with it: `collect_effective_taffy_children(owner)` names
    /// `node_id`'s Taffy id exactly when this answers `owner` (#597).
    ///
    /// Two hops, for the two ways a node's box-tree parent is not its DOM
    /// parent. [`Self::box_tree_parent`] handles the first — a run member's box
    /// is held by the anonymous block box that lays the run out, not by the
    /// container (#566). The loop handles the second — a `display: contents`
    /// element generates no box, so it holds none of its children's either and
    /// they belong to the nearest ancestor that does, however many wrappers deep
    /// (#476).
    ///
    /// Returns `None` for a node with no such ancestor: the document root, and
    /// anything in a subtree that is not attached to it.
    ///
    /// **The first hop is currently defence, and that is measured**: replacing
    /// it with a plain `parent` walk survives `-p rinch-dom -p rinch`. Its only
    /// caller heals nodes that are no longer inline-level, and a run member is
    /// inline content by construction, so none of them carries a `run_box`.
    /// It is written as the general inverse rather than to that caller's shape,
    /// because a caller that asks about a run member would otherwise be handed
    /// the container — whose list names the anonymous box, not the member.
    ///
    /// **A hoisted out-of-flow box (#591) needs no hop at all**: its box-tree
    /// parent *is* its host — `box_tree_parent` answers
    /// `Node::hoisted_out_of_flow_to` first — and `collect_effective_taffy_children(host)`
    /// names it, because `collect_run_units` makes it a unit of the host. So the
    /// "exactly when" above holds for it in both directions, which it did not
    /// while the hoist was a canonicalization over this authority's answer
    /// (the review of PR 2 measured both directions broken).
    ///
    /// **The split-inline hop is defence too, measured the same way**, and it is
    /// kept for a reason the `Contents` hop does not need: after #513,
    /// `collect_effective_taffy_children` of a **split inline** still names its
    /// own children's boxes, because `restore_split_inlines` reads it that way to
    /// put them back — while the boxes are in fact held by that element's block
    /// container. So two nodes' lists name them, and the "exactly when" above is
    /// one-directional for that one node. This function answers about the *live*
    /// tree, which is the container, and that is why the hop stays even though no
    /// caller reaches it today: a caller that asks about a box inside a split
    /// inline and is handed the element would rebuild a list nothing lays out.
    pub(crate) fn effective_taffy_owner(
        nodes: &slab::Slab<crate::node::Node>,
        node_id: usize,
    ) -> Option<usize> {
        use crate::computed_style::values::DisplayValue;

        let mut ancestor = Self::box_tree_parent(nodes, node_id)?;
        loop {
            let node = nodes.get(ancestor)?;
            // Two kinds of ancestor hold none of their children's boxes, so
            // neither can be the answer: a `display: contents` element, which
            // generates no box (#476), and a **split inline** (#513), whose
            // pieces belong to the block container that minted the anonymous
            // boxes around them. Both are the same fact — the node contributes
            // its children's boxes rather than one of its own — which is why the
            // walk tests for both and `box_tree_parent` (a different question:
            // whose *coordinate space* is this box in) tests for neither.
            if node.computed_style.display != DisplayValue::Contents && !node.is_split_inline() {
                return Some(ancestor);
            }
            ancestor = Self::box_tree_parent(nodes, ancestor)?;
        }
    }

    /// Every Taffy id `node_id` may occupy in its **parent's** Taffy child
    /// list: its own, plus — when it is or was spliced away by
    /// `sync_display_contents` — its flattened descendants' (#477).
    ///
    /// **This is not a competing answer to
    /// [`Self::collect_effective_taffy_children`]'s question; it is a different
    /// question**, and the two live side by side so nobody reads one as a bug.
    /// That function answers *"which boxes belong in this node's child list"* —
    /// the authority for a whole-list **rebuild**, which runs inside
    /// `resolve_layout`'s `ifc_dirty` block. This one answers *"which Taffy ids
    /// might this node currently be occupying in its parent's list"*, at
    /// **mutation time**, between passes, for the incremental insert index
    /// ([`crate::RinchDocument::compute_taffy_child_index`]). A rebuild replaces
    /// the list, so it wants the exact set; an index searches the live list, so
    /// it wants a superset.
    ///
    /// **The gates differ, and the difference is load-bearing.**
    /// `collect_effective_taffy_children` gates on computed `display: contents`
    /// alone, which is right for a rebuild running inside the sync that
    /// maintains it. Here the gate is `Contents` **or**
    /// [`crate::node::Node::contents_spliced`], mirroring
    /// [`Self::taffy_detach_contribution`]'s (#517/#520) — because between
    /// syncs a wrapper restyled *away* from `contents` still has its children
    /// sitting in the parent's list, and the `Contents`-only gate would answer
    /// with the wrapper's own already-detached id and lose the sibling
    /// entirely. Pinned by
    /// `an_insert_after_a_wrapper_restyled_off_contents_still_clears_its_slots`
    /// (dropping the flag half is killed by it) — so the two are **not**
    /// interchangeable in this direction.
    ///
    /// The other half of the gate is not load-bearing here, measured: dropping
    /// the `Contents` test survives the whole workspace, because
    /// `sync_display_contents` sets the flag for *every* `Contents` node at the
    /// end of every pass, and a wrapper that computes `Contents` without yet
    /// being spliced still has its **own** id in the parent's list — which
    /// `out.push(taffy_id)` adds unconditionally, so the sibling is found
    /// without the recursion. Kept as belt-and-braces symmetry with the detach
    /// gate, not because a case here is known to need it.
    ///
    /// Erring wide is free — an id that is not actually attached contributes no
    /// position and is skipped, so a wide gate can only *miss* an absent id,
    /// never pick a wrong slot — while erring narrow silently loses the sibling
    /// and sends the search one step further back than it should go.
    pub(crate) fn collect_taffy_contribution(
        nodes: &slab::Slab<crate::node::Node>,
        node_id: usize,
        out: &mut Vec<taffy::NodeId>,
    ) {
        use crate::computed_style::values::DisplayValue;

        let Some(node) = nodes.get(node_id) else {
            return;
        };
        if let Some(taffy_id) = node.taffy_id {
            out.push(taffy_id);
        }
        if node.computed_style.display == DisplayValue::Contents || node.contents_spliced {
            for &child_id in &node.children {
                Self::collect_taffy_contribution(nodes, child_id, out);
            }
        }
        // A non-atomic inline element that hosts hoisted out-of-flow boxes
        // (#591) occupies its parent's list through *them*: its own node is
        // detached into the IFC, the boxes sit in the parent's list right after
        // where it would be. Erring wide is free here (see above), so every
        // hoisted descendant is offered, whatever its host.
        if node.hosts_hoisted_out_of_flow {
            let mut stack: Vec<usize> = node.children.clone();
            while let Some(id) = stack.pop() {
                let Some(n) = nodes.get(id) else {
                    continue;
                };
                if n.hoisted_out_of_flow_to.is_some()
                    && let Some(t) = n.taffy_id
                {
                    out.push(t);
                }
                if n.hosts_hoisted_out_of_flow {
                    stack.extend(n.children.iter().copied());
                }
            }
        }
    }

    /// Every node whose Taffy child list has to be rebuilt when `node_id`'s
    /// **DOM** children change: `node_id` itself, then — if it is a
    /// `display: contents` element, which generates no box — each contents
    /// ancestor in turn and finally the nearest non-contents one, which is
    /// where [`Self::collect_effective_taffy_children`] actually puts those
    /// boxes.
    ///
    /// Returned **bottom-up**, and it must be consumed in that order: Taffy's
    /// `set_children` removes each adopted child from its previous parent's
    /// list, so rebuilding the deepest owner first and the flattening one last
    /// leaves the boxes where the flattening says they belong.
    ///
    /// **What reaches the walk, since #568: only
    /// `cleanup_anonymous_block_boxes`** (#585). It takes each affected parent
    /// from the anonymous box's `parent`, recorded on the pass that *minted*
    /// the box, while #568's phase-1 guard applies at classification time on
    /// the current pass. So a container that was `display: block` when it
    /// minted a box and has since been restyled to `display: contents` — by an
    /// inline `style` write or through the cascade, both measured — arrives
    /// here as a `Contents` node and the walk runs.
    /// `create_anonymous_block_boxes`' two call sites cannot reach it: its
    /// `parent_id` passed that guard, and a box it mints is always `Block`,
    /// because `ComputedStyle::for_anonymous_box` copies `Contents` only from a
    /// `Contents` parent.
    ///
    /// Without the walk that restyle strands the container's boxes. Rebuilding
    /// only the now-boxless container hands them to a Taffy node nothing lays
    /// out, and `set_children` steals them out of the list that should hold
    /// them on the way. It shows only where the flattening ancestor is one
    /// phase 1 skips — anything that is not a block container, i.e. `Inline`,
    /// `InlineBlock`, `InlineFlex`, `InlineGrid` or `Flex`; the flex column is
    /// the one measured. Against a **block** ancestor the flattened
    /// `text + block` makes that ancestor mixed content in its own right, so
    /// phase 2 rebuilds its list anyway, and the walk changes nothing (also
    /// measured, as this section's control).
    /// `anon_box_contents_flatten_tests`' "owners walk, after #568" section is
    /// the witness, on a flex column, with a chain of two so it discriminates
    /// the loop and not merely the walk.
    ///
    /// **What used to reach it, and no longer can.** #567 wrote this for a
    /// wrapper classified as mixed content in its own right: `display: contents`
    /// computes to `DisplayMode::Block` (`style_resolution`), so a wrapper
    /// holding `text + block` minted the anonymous box inside a boxless
    /// element, and the ancestor's list — built by `sync_display_contents`
    /// before that box existed — had to be rebuilt after it. #568's guard
    /// removed that shape and with it every fixture that discriminated this
    /// function at all: on `4fe65ed` both `owners = vec![node_id]` and a
    /// loop-free single step survived `cargo test -p rinch-dom -p rinch`
    /// (41 binaries) with nothing failing. They all still pass. That is #585.
    /// `ComputedStyle::for_anonymous_box`'s `Contents` branch (#319) went with
    /// it — a minted box could be `Contents` only inside such a wrapper — so
    /// this walk no longer covers an anonymous box of its own.
    ///
    /// **A split inline (#513) is deliberately NOT in this walk**, even though it
    /// holds none of its children's boxes either and so satisfies the same
    /// description. It was, briefly, and the mutant that removed it survived the
    /// suite — because the one caller that passes a split node,
    /// `restore_split_inlines`, rebuilds the block container **explicitly** beside
    /// it. Two mechanisms that happen to agree about one list is the shape #476
    /// came from, so the explicit one is kept and this arm was taken out rather
    /// than documented as defence. Anything new that calls
    /// [`crate::RinchDocument::rebuild_effective_taffy_children`] with a split
    /// inline must rebuild that element's container itself, as that function's one
    /// such caller does.
    pub(crate) fn taffy_child_list_owners(
        nodes: &slab::Slab<crate::node::Node>,
        node_id: usize,
    ) -> Vec<usize> {
        use crate::computed_style::values::DisplayValue;

        let mut owners = vec![node_id];
        let mut current = node_id;
        while nodes
            .get(current)
            .is_some_and(|n| n.computed_style.display == DisplayValue::Contents)
        {
            let Some(parent) = nodes[current].parent else {
                break;
            };
            owners.push(parent);
            current = parent;
        }
        owners
    }

    /// Everything a node's **typography** change owes the layout already taken
    /// from it (issues #654, #661, #678).
    ///
    /// Three derived things are baked from a node's font, and each is dropped by
    /// a different mechanism, which is why this is one function and not three
    /// call sites that drift:
    ///
    /// 1. the Parley layout of the IFC that holds the text —
    ///    [`Self::invalidate_ifc_for_node`], plus the IFC of each text child,
    ///    which is an **anonymous box** outside the element tree when the
    ///    run sits beside a block-level sibling or inside a split inline
    ///    (#513) and so is reached by no ancestor walk;
    /// 2. the box of any atomic inline above it, which no Taffy compute reaches
    ///    — [`Self::mark_atomic_inline_dirty`];
    /// 3. the `NodeContext::Text` a text child is measured through when it is a
    ///    flex or grid item, which is a *copy* of this node's typography and
    ///    which the incremental sync would not refresh, because the set it reads
    ///    records DOM mutations and a restyle is not one. Taffy caches a leaf
    ///    measure per available space, so the leaf is marked as well as the
    ///    context — refreshing one without the other changes nothing.
    ///
    /// Called from the cascade (`apply_stylo_styles_to_taffy`, gated on
    /// `ComputedStyle::same_text_layout_inputs`) and from the transition and
    /// animation ticks, which write `computed_style` **without** going through
    /// the cascade and so reach none of that gating by themselves.
    ///
    /// The cascade's gate is deliberately the **wider** of its two predicates —
    /// `same_text_layout_inputs` rather than `same_measured_text_inputs`, which
    /// is what decides `layout_dirty`. Step 2 is an O(depth) walk that does
    /// nothing unless a compute follows, and steps 1 and 3 are invalidations
    /// rather than work; so a property listed one predicate too wide costs a
    /// spare re-shape, and one listed too narrow leaves a box frozen.
    pub(crate) fn invalidate_text_measure_for_node(&mut self, node_id: usize) {
        if !self.tree.nodes.contains(node_id) {
            return;
        }
        self.invalidate_ifc_for_node(node_id);
        self.mark_atomic_inline_dirty(node_id);
        for child in self.tree.nodes[node_id].children.clone() {
            let Some(child_node) = self.tree.nodes.get(child) else {
                continue;
            };
            if !matches!(child_node.kind, NodeKind::Text(_)) {
                continue;
            }
            let taffy_id = child_node.taffy_id;
            // A text run beside a block-level sibling is laid out by an
            // anonymous block box, and one inside a split inline (#513) by the
            // box around its fragment — neither in the element tree, so the
            // walk above, which asks this node and its ancestors, never finds
            // it. The text node's own `ifc_root` names that box. A text node is
            // never cascaded on its own, so this is the only call that can
            // reach it.
            if let Some(root) = child_node.ifc_root
                && root != node_id
            {
                self.invalidate_ifc_root(root);
            }
            self.tree.dirty_text_contexts.insert(child);
            if let Some(t) = taffy_id {
                let _ = self.tree.taffy.mark_dirty(t);
            }
        }
    }

    /// Invalidate the IFC that owns a node (if any).
    ///
    /// Clears the IFC root's cached text_layout so it rebuilds on next layout pass.
    /// Also checks the parent's text_layout as a fallback when ifc_root hasn't been
    /// set yet (before the first layout pass).
    pub(crate) fn invalidate_ifc_for_node(&mut self, node_id: usize) {
        if let Some(ifc_root_id) = self.tree.nodes.get(node_id).and_then(|n| n.ifc_root) {
            self.invalidate_ifc_root(ifc_root_id);
            // An atomic inline (`inline-block`/`-flex`/`-grid`) is a member of
            // the IFC it sits in *and* the root of its own text, so its restyle
            // owes both: the outer layout (its box moved) and its own (its
            // glyphs changed). Reaching only the outer one left a `<button>`'s
            // label in its old colour and size after a class change.
            if self.holds_ifc_layout(node_id) {
                self.invalidate_ifc_root(node_id);
            }
        } else if self.holds_ifc_layout(node_id) {
            // The node itself IS the IFC root (block element containing inline text)
            self.invalidate_ifc_root(node_id);
        } else {
            self.invalidate_nearest_ifc_ancestor(node_id);
        }
    }

    /// Fallback for a node with no `ifc_root` yet (before the first layout
    /// pass): walk ancestors to the nearest one holding an IFC layout.
    fn invalidate_nearest_ifc_ancestor(&mut self, node_id: usize) {
        let mut cur = self.tree.nodes.get(node_id).and_then(|n| n.parent);
        while let Some(pid) = cur {
            if self.holds_ifc_layout(pid) {
                self.invalidate_ifc_root(pid);
                break;
            }
            cur = self.tree.nodes.get(pid).and_then(|n| n.parent);
        }
    }

    /// What a node being **moved** (or inserted) leaves behind: the IFC it
    /// was a *member* of, which lost it — never the node's **own** IFC, if it
    /// is a root (issue #914).
    ///
    /// A move does not change what a root is built from: its members, their
    /// text and their order all travel with it. What its new position *can*
    /// change reaches it by the two routes every other change takes. Its
    /// typography, inherited from a new parent or matched by a positional
    /// selector (`:nth-child`), arrives through the cascade, which compares the
    /// old and new text inputs and drops the layout itself when they differ
    /// (`ComputedStyle::same_text_layout_inputs`); its available width is a
    /// key of the measure cache and of `build_ifc_layouts`' rebuild test. And
    /// the structural pass re-signs it (the verb seeds its subtree), so a
    /// content change it somehow missed still drops it
    /// (`refresh_ifc_signatures`).
    ///
    /// [`Self::invalidate_ifc_for_node`] drops both for an atomic inline and
    /// the node's own for a block root; that is right for a restyle of the
    /// node, and for a move it re-shaped the moved row of every keyed `for`
    /// reorder — twice per row on a reversed list — with nothing changed.
    pub(crate) fn invalidate_ifc_left_by(&mut self, node_id: usize) {
        if let Some(ifc_root_id) = self.tree.nodes.get(node_id).and_then(|n| n.ifc_root) {
            self.invalidate_ifc_root(ifc_root_id);
        } else if !self.holds_ifc_layout(node_id) {
            self.invalidate_nearest_ifc_ancestor(node_id);
        }
    }

    /// Whether `node_id` holds a layout derived from an inline formatting
    /// context of its own: a paint layout, or sizes the measure function
    /// cached for it.
    fn holds_ifc_layout(&self, node_id: usize) -> bool {
        self.tree
            .nodes
            .get(node_id)
            .is_some_and(|n| n.text_layout.is_some())
            || self
                .tree
                .ifc_measure_cache
                .get(&node_id)
                .is_some_and(|e| !e.sizes.is_empty())
    }

    /// Drop everything derived from IFC root `root_id`'s content: the paint
    /// layout (`text_layout`), the measure function's cached sizes, and
    /// Taffy's own cached size for it — on the root and on its #466 measure
    /// leaf, since `mark_dirty` propagates up, not down.
    ///
    /// The Taffy marks are what make the cache drop reachable. A restyle that
    /// changes only an inline *member's* typography (a `span` inside the
    /// paragraph) marks no Taffy node on its way — the member is detached from
    /// Taffy — so without the mark on the root the compute serves the root's
    /// cached size and never calls the measure function at all. The member
    /// branch of [`Self::invalidate_ifc_for_node`] used to drop the paint
    /// layout and the cache and mark nothing, which was harmless only while an
    /// eager subtree drop on every attribute write marked the root by another
    /// route.
    pub(crate) fn invalidate_ifc_root(&mut self, root_id: usize) {
        if let Some(root) = self.tree.nodes.get_mut(root_id) {
            root.text_layout = None;
        }
        self.tree.dirty_ifc_text_roots.insert(root_id);
        self.tree.forget_ifc_measures(root_id);
        if let Some(taffy_id) = self.tree.nodes.get(root_id).and_then(|n| n.taffy_id) {
            let _ = self.tree.taffy.mark_dirty(taffy_id);
        }
        self.mark_ifc_measure_dirty(root_id);
    }

    /// Append `child_taffy` under `parent_taffy`, asserting in debug builds that
    /// no Taffy list still holds it.
    ///
    /// Taffy's `add_child` does not remove a child from a previous parent (only
    /// `set_children` does), so attaching a node that some list still names
    /// makes two lists claim one node — `A double-claim`, which the next
    /// canonicalization may or may not happen to collapse. Since #591 and #513 a
    /// node's Taffy parent is not always its DOM parent's Taffy node, which is
    /// exactly how a DOM move could arrive here with an edge still standing; the
    /// mutation-time detach (`taffy_detach_contribution`) is what keeps this
    /// quiet, and this is what says so if it ever stops.
    pub(crate) fn taffy_add_child_checked(
        &mut self,
        parent_taffy: taffy::NodeId,
        child_taffy: taffy::NodeId,
    ) {
        debug_assert!(
            self.tree.taffy.parent(child_taffy).is_none(),
            "rinch-dom: attaching a Taffy node while another list still holds it — \
             detach from `taffy.parent()` first, or the tree carries a double-claim (#591)"
        );
        let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
    }

    /// Safely remove a child from a Taffy parent, checking membership first.
    /// Taffy's `remove_child` panics if the child isn't actually a child of the parent,
    /// which can happen when inline children were detached by `setup_inline_formatting_contexts`.
    pub(crate) fn taffy_remove_child_safe(
        &mut self,
        parent_taffy: taffy::NodeId,
        child_taffy: taffy::NodeId,
    ) {
        if let Ok(children) = self.tree.taffy.children(parent_taffy)
            && children.contains(&child_taffy)
        {
            let _ = self.tree.taffy.remove_child(parent_taffy, child_taffy);
        }
    }

    /// Remove `node_id`'s *contribution* to `parent_taffy`'s Taffy children —
    /// which is not always the node's own `taffy_id` (#517).
    ///
    /// A `display: contents` node owns no box: after a layout pass,
    /// `sync_display_contents` has spliced the node's effective children
    /// (recursively flattened through nested contents nodes) directly into
    /// the parent's Taffy child list and detached the node's own Taffy node.
    /// Detaching only the node's own id there is a silent no-op
    /// (`taffy_remove_child_safe` swallows it), and the spliced-in children
    /// stay behind as invisible siblings claiming layout space forever.
    ///
    /// The gate is `computed_style.display == Contents` **or**
    /// `Node::contents_spliced` (#520). Computed display alone is not enough:
    /// it describes the node *now*, while the splice describes what a past
    /// `sync_display_contents` pass did. A wrapper restyled away from
    /// `contents` (eagerly — e.g. `append_child`'s
    /// `recompute_node_styles_recursive` flushes every pending style root)
    /// and then detached before the next layout pass computes a box display
    /// while its children still sit spliced in the parent's list; only the
    /// flag records that. Conversely a freshly attached wrapper computes
    /// `Contents` while its own id is what the parent holds (the splice
    /// happens only in `resolve_layout`), so the node's own id is removed
    /// unconditionally as well.
    ///
    /// When the gate fires, the removal set is every descendant's Taffy id,
    /// flattening through children that are or were spliced
    /// (`collect_taffy_detach_candidates`) — not just the *current* effective
    /// children, which would miss the grandchildren of a nested wrapper whose
    /// own toggle hasn't been synced yet. Nothing is over-removed: every
    /// candidate is a proper descendant of `node_id`, and a descendant's id
    /// found directly in `parent_taffy`'s list can only be (possibly stale)
    /// splice attachment — the whole subtree is leaving, so it must go
    /// either way. Whatever is absent, `taffy_remove_child_safe` swallows.
    /// Every detach path (`remove_child`, `replace_node`,
    /// `set_text_content`/`set_inner_html` child-clearing, and the reparent
    /// legs of `append_child`/`insert_before`/`insert_child`) goes through
    /// here.
    ///
    /// `remove_node` goes through here too (#515). It was written with an
    /// either/or — flattened set for a `Contents`-computing node, own id
    /// otherwise — and that was changed to this deliberately, not folded in as
    /// a deduplication. Two reasons. The either/or misses the window where a
    /// node computes `Contents` while its own Taffy node is still attached
    /// (between a display toggle and the sync that heals it), and it cannot
    /// see `contents_spliced` at all. And it made a whole mutant class
    /// invisible: forcing its `is_contents` true — so a *plain* node's own id
    /// is never removed — passed the entire rinch-dom suite. Removing both
    /// sets unconditionally cannot express that bug.
    ///
    /// And the node's own id is removed from **whichever Taffy list holds it**,
    /// which since #591 and #513 is not always `parent_taffy` — see the comment
    /// at the bottom.
    pub(crate) fn taffy_detach_contribution(
        &mut self,
        parent_taffy: taffy::NodeId,
        node_id: usize,
    ) {
        use crate::computed_style::values::DisplayValue;

        let node = &self.tree.nodes[node_id];
        if node.computed_style.display == DisplayValue::Contents || node.contents_spliced {
            let mut candidates = Vec::new();
            Self::collect_taffy_detach_candidates(&self.tree.nodes, node_id, &mut candidates);
            for candidate_taffy in candidates {
                self.taffy_remove_child_safe(parent_taffy, candidate_taffy);
            }
        }
        if let Some(node_taffy) = self.tree.nodes[node_id].taffy_id {
            self.taffy_remove_child_safe(parent_taffy, node_taffy);
            // **And from the list that actually holds it** (#591). The DOM
            // parent's Taffy node is not always the node's Taffy parent: an
            // out-of-flow box beneath an inline element is a Taffy child of the
            // IFC root that lays the line out (`setup_inline_formatting_contexts`'
            // canonicalization), and a block inside a split inline is a child of
            // its block container (#513). Detaching from the DOM parent alone was
            // a silent no-op for those — `taffy_remove_child_safe` swallows a
            // non-member — and left the removed node's Taffy id in a list the
            // next pass then found one child too long. Measured for the
            // out-of-flow case: the root had no out-of-flow DOM child left to
            // canonicalize, carried `InlineRoot` on a non-leaf, and the #466 leaf
            // invariant fired
            // (`out_of_flow_in_inline_tests::removing_the_hoisted_absolute_leaves_a_clean_tree`).
            // Taffy knows the answer, so it is asked rather than re-derived.
            if let Some(actual) = self.tree.taffy.parent(node_taffy) {
                self.taffy_remove_child_safe(actual, node_taffy);
            }
        }
        // **And every edge that leaves the subtree** (#591). A hoisted
        // out-of-flow box beneath this node — or a block inside a split inline
        // (#513) — is a Taffy child of a list *outside* the subtree being
        // removed or moved, so detaching the node's own edge leaves that list
        // one stale id long. Measured by the review of PR 2: removing the span
        // while its container kept other inline text made the container a
        // non-leaf `InlineRoot` carrier and the #466 leaf invariant panicked;
        // removing it as the only child left the box silently reachable. Walk
        // the subtree, and cut every edge whose Taffy parent is not a subtree
        // member. O(subtree), like the `ifc_root` clear the same DOM ops run.
        let mut subtree: Vec<usize> = vec![node_id];
        let mut i = 0;
        while i < subtree.len() {
            if let Some(n) = self.tree.nodes.get(subtree[i]) {
                subtree.extend(n.children.iter().copied());
            }
            i += 1;
        }
        let members: std::collections::HashSet<taffy::NodeId> = subtree
            .iter()
            .filter_map(|&id| self.tree.nodes.get(id).and_then(|n| n.taffy_id))
            .collect();
        for &id in &subtree[1..] {
            let Some(t) = self.tree.nodes.get(id).and_then(|n| n.taffy_id) else {
                continue;
            };
            if let Some(p) = self.tree.taffy.parent(t)
                && !members.contains(&p)
            {
                self.taffy_remove_child_safe(p, t);
            }
        }
    }

    /// Every Taffy id under `node_id` that a splice may have left in an
    /// ancestor's Taffy child list (#517, #520): each child's own id, recursing
    /// through children that either compute `Contents` now or were spliced by
    /// a past pass and not yet healed (`Node::contents_spliced`).
    ///
    /// This is deliberately a superset of `collect_effective_taffy_children`:
    /// it exists for *removal* via `taffy_remove_child_safe`, where an id that
    /// was never attached is swallowed, so erring wide is free — while erring
    /// narrow (flattening only through *current* contents children) leaves a
    /// nested wrapper's grandchildren stranded when both wrappers were toggled
    /// away from `contents` in the same flush.
    fn collect_taffy_detach_candidates(
        nodes: &slab::Slab<crate::node::Node>,
        node_id: usize,
        out: &mut Vec<taffy::NodeId>,
    ) {
        use crate::computed_style::values::DisplayValue;

        for &child_id in &nodes[node_id].children {
            let Some(child) = nodes.get(child_id) else {
                continue;
            };
            if let Some(child_taffy) = child.taffy_id {
                out.push(child_taffy);
            }
            if child.computed_style.display == DisplayValue::Contents || child.contents_spliced {
                Self::collect_taffy_detach_candidates(nodes, child_id, out);
            }
        }
    }

    /// A subtree that has just left the document has **no before-change
    /// style** (issue #699).
    ///
    /// `has_been_styled` is the one thing a transition waits for: the cascade
    /// starts one only when the node it is restyling has been styled before
    /// (`apply_stylo_styles_to_taffy`). A node styled while it was *connected*,
    /// then detached, keeps that flag and keeps the `computed_style` it had in
    /// the document — so if an ancestor's class changes while it is out, its
    /// re-insertion resolves to a different value, the cascade reads old ≠ new
    /// on an already-styled node, and the box animates in from a style the user
    /// never saw. A browser does not: a removed element is not rendered, it has
    /// no before-change style, and re-insertion is a first style.
    ///
    /// #696 gave a node whose *first* resolution happened detached the same
    /// property, by never styling it at all. This is the other half — a node
    /// whose first life was connected — and it has to be answered where the
    /// node leaves, because nothing at the re-insertion can tell a returning
    /// subtree from one that never left.
    ///
    /// Three things are reset and a fourth deliberately is not. The three are
    /// exactly what [`NodeTree::remove_subtree`] drops when it *frees* a
    /// subtree, minus the freeing — which is the point: a detached node is not
    /// a dead one, and rinch has to keep it readable.
    ///
    /// - **`has_been_styled`**, for the whole removed subtree. Not just its
    ///   root: a descendant carries its own flag and its own `transition`
    ///   declaration, and resolution reaches it by its own recursion.
    /// - **Any running `ActiveTransition`**, for the same nodes. Without this
    ///   the fix would not hold: `tick_transitions` walks
    ///   `tree.active_transitions`, not the document, so a transition left
    ///   behind by a detach keeps writing interpolated values into
    ///   `computed_style` — and would go on doing so after the re-insertion,
    ///   reinstating the very animation the flag reset removes. Cancelling on
    ///   removal is also what CSS asks for, and it stops a detached node that is
    ///   never re-inserted from marking the tree layout-dirty for 150ms.
    /// - **Any running animation**, for the same nodes. Not needed by #699's
    ///   own symptom — an animation writes `computed_style` without consulting
    ///   `has_been_styled` either way — but a `@keyframes` animation has no
    ///   150ms bound to self-limit against, and the desktop shell decides
    ///   whether to keep asking for frames from
    ///   whether `tree.active_animations` holds a running (not paused, #763)
    ///   animation (`rinch/src/app/event_dispatch.rs`).
    ///   A removed `Loader` kept a desktop app rendering forever, and on **every
    ///   removal route that leaves the subtree alive** this is now what stops
    ///   it. Not every route: `set_inner_html` stops it by destruction (below),
    ///   and a blanket restyle clears every entry
    ///   (`recompute_all_styles_full`) whether the node is in the document or
    ///   not, so a theme change happens to stop a removed `Loader` with this
    ///   helper uninvolved.
    ///   It used not to be that either: `NodeHandle::clear_animations` stamped
    ///   an inline `animation: none` over a subtree on its way out at **five
    ///   call sites** — `show_dom`, `match_dom`, two of `for_each_dom_typed`'s
    ///   three remove sites, and the component re-render effect — which both
    ///   stopped the frames and permanently disarmed the subtree, at all five
    ///   (#704). That method is gone; every reactive removal reaches this
    ///   helper instead.
    ///   `a_detached_animation_stops_asking_for_frames` is the pin.
    /// - **`computed_style` is left exactly as it was**, and so is
    ///   `text_layout`. Clearing either would be wrong twice over. #696 pinned
    ///   a detached node as still readable —
    ///   `detached_style_roots_tests::a_detached_node_is_still_readable` asserts
    ///   `dom_tree(root_id: <detached id>)` reports the style the node last had
    ///   *in* the document — and `a_detached_subtree_keeps_its_text_layout`
    ///   pins the glyphs. They are also what the re-insertion's own staleness
    ///   gates compare against: `same_text_layout_inputs` and
    ///   `same_measured_text_inputs` read the old `computed_style` to decide
    ///   whether to re-shape (#654, #661, #678), and against a cleared one they
    ///   would answer "stale" for every re-inserted node forever. The flag is
    ///   what the transition reads; the value is what everything else reads.
    ///   Only the flag has to go.
    ///
    /// # Where this is called from
    ///
    /// **Five places in `dom_impl/dom_document_impl.rs` write `parent = None`.**
    /// Four of them call this; the fifth does not need to. That count is the
    /// claim to check against `grep -n '\.parent = None'` if this file ever
    /// grows a sixth — an unhooked one is silent, which is how the fourth row
    /// below was missed on the first pass.
    ///
    /// | route | who reaches it |
    /// |---|---|
    /// | `remove_node` | every reactive removal — `show_dom`, `match_dom`, `for_each_dom_typed`'s `Remove`, its re-render swap and `reclaim_displaced`, `virtual_list`, the component re-render effect, the editor's `ViewDesc` diff. They reach it by **two** verbs since #719: `NodeHandle::remove` where the same subtree may be shown again (`show_dom`, `match_dom`) and `NodeHandle::discard` where it may not (all the rest). On this backend `discard_node` **is** `remove_node` — the trait default — so both land here; only `rinch-web` tells them apart, by pruning its node maps on the second |
    /// | `remove_child` | `NodeHandle::remove_child` and `RenderScope`'s batched `DomUpdate::RemoveChild` |
    /// | `replace_node` | the displaced `old` subtree |
    /// | `set_text_content` | `NodeHandle::set_text_content` and `RenderScope`'s batched `DomUpdate::SetTextContent`, **when the target is an element with children** — it orphans every one of them. Reactive text in `rsx!` targets a text node and takes the other branch, so this is app code writing over an element's children |
    ///
    /// The fifth is `set_inner_html`, and it is safe by **destruction** rather
    /// than by reset: it calls `NodeTree::remove_subtree`, which frees the slab
    /// entries and drops `active_transitions` and `active_animations` with them.
    /// Nothing survives to carry a stale flag, and the handle is retired.
    ///
    /// **A reparenting `append_child` / `insert_before` / `insert_child` is
    /// deliberately not on that list.** Those three are the *move* routes — a
    /// keyed `for` reorder is `insert_after`, which is one of them — and a move
    /// must not reset anything: the node is back in the document before the
    /// call returns, so it never stopped being rendered, and a row that was
    /// mid-transition when the list reordered goes on transitioning.
    /// `a_reparenting_move_does_not_restart_a_running_transition` and
    /// `a_keyed_for_reorder_does_not_restart_a_running_transition` are the
    /// pins, and they are exactly the two fixtures that kill the mutant which
    /// adds the reset there.
    ///
    /// The one shape that *does* leave the document under a move — a **mounted**
    /// node moved into a **detached** parent — is not this helper's business
    /// either, and has its own: [`Self::detach_subtree_styles_if_moved_out`]
    /// (#702). It is not a `parent = None` detach, but it is disconnected,
    /// which #696 established is the question that matters.
    pub(crate) fn detach_subtree_styles(&mut self, node_id: usize) {
        // Iterative, like `clear_ifc_root_recursive` — a deep subtree must not
        // overflow the stack on its way out of the document.
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            let Some(node) = self.tree.nodes.get_mut(id) else {
                continue;
            };
            node.has_been_styled = false;
            stack.extend(node.children.iter().copied());
            self.tree.active_transitions.remove(&id);
            self.tree.active_animations.remove(&id);
        }
    }

    /// A **move** that takes `child` out of the document is a detach too
    /// (issue #702).
    ///
    /// [`Self::detach_subtree_styles`]'s own doc says a move is not a detach,
    /// and that is true of every move whose destination is in the document —
    /// which, before this, was assumed to be all of them. It is not: a mounted
    /// node appended into a parent that is *not* connected to `tree.root_id`
    /// has left the document while keeping a parent, so it fails the
    /// `parent = None` test the four detach routes share. #696 established that
    /// **connectivity** is the question that matters, not the parent field: a
    /// node under a detached parent is not styled at all, so it keeps
    /// `has_been_styled` and the `computed_style` it had where it was mounted,
    /// and animates in from that style when its new parent is spliced in
    /// somewhere else.
    ///
    /// # The two guards, and which one is for cost
    ///
    /// Both callers' conditions are here rather than at the four call sites, so
    /// that the reasoning is in one place and the sites are one line.
    ///
    /// - **`old_parent != new_parent`** is a **cost** guard, not a correctness
    ///   one. A move within one container cannot change whether the child is
    ///   connected, because the child's reachability *is* its parent's and the
    ///   parent has not changed — so walking would give the same answer more
    ///   slowly. It matters because that move is the keyed `for` reorder, which
    ///   is the hottest shape this code has: a reorder pays one integer
    ///   comparison per row and never walks.
    /// - **`depth_if_connected(new_parent).is_none()`** is the correctness one,
    ///   and it is [`RinchDocument::depth_if_connected`] — the same walk #696
    ///   filters `style_roots` with, so the two cannot disagree about what
    ///   "connected" means.
    ///
    /// The callers supply a third guard by construction: they only reach this
    /// when the child **already had a parent**, since a node created moments ago
    /// cannot be a move.
    ///
    /// # What that does and does not make free
    ///
    /// A node appended **straight into its final parent** never reaches this at
    /// all. An `rsx!` **component site** does reach it, once per child, and this
    /// is the non-obvious part: `component_codegen` builds a site's children
    /// into a `<template>` scratch container attached to nothing (#719), and
    /// `Component::render` then adopts each one into the component's own root,
    /// which is *also* still detached at that moment. So the adoption is a move
    /// into a detached parent by this rule — it walks, and it takes the full
    /// [`Self::detach_subtree_styles`] subtree walk.
    ///
    /// Counted on 500 component sites carrying a 20-node subtree each: **500
    /// entries, 500 walks, 500 resets over 10,000 nodes**, against 0/0/0/0 for
    /// the same nodes appended straight into their final parent. The reset is
    /// semantically a no-op there — a node created moments ago is already
    /// unstyled with empty transition and animation maps — and the cost does not
    /// show: best of 40, release, three alternated rounds, the build *with* this
    /// helper was the faster of the two every time (1801–1825ms against
    /// 1809–1830ms), i.e. inside build-to-build noise. It is recorded because
    /// "building a tree pays nothing" would otherwise read as covering the
    /// framework's own render path, which it does not.
    ///
    /// # One behaviour change that follows, and is narrower than it looks
    ///
    /// A node that is mounted and **still connected** when it is moved into a
    /// detached parent, and adopted straight back out in the same pass, loses
    /// its running transitions and restarts its animations. A browser never
    /// observes that intermediate state, because its style recalc is batched to
    /// the end of the task; rinch's cascade is not, so the round trip is two
    /// events here and one there.
    ///
    /// The component **re-render** path does not reach it:
    /// `reactive_component_dom` removes the previous output *before* rendering
    /// fresh, so #699 has already reset that subtree by the time the new
    /// `<template>` sees it. What remains is handing a component a handle that
    /// is mounted **elsewhere and still connected** — the #719 shape,
    /// `Card { {captured.clone()} }` — at a render where the old subtree was not
    /// the doomed one.
    ///
    /// # Where this is called from
    ///
    /// **Four places in `dom_impl/dom_document_impl.rs` write
    /// `nodes[..].parent = Some(..)` for a node that may already be mounted**,
    /// and all four call this: `append_child`, `insert_before`, `insert_child`,
    /// and `replace_node` for its incoming `new`. `grep -n '\.parent = Some('`
    /// is the check if a fifth ever appears; the other matches in that file and
    /// in `pseudo.rs` / `ifc.rs` are nodes created moments earlier, which cannot
    /// be moves. An unhooked route here is silent, which is how `replace_node`
    /// was nearly missed — its own comment asserted "`new` has not [left the
    /// document] — it was spliced in, which is a move, and a move resets
    /// nothing", true of every destination but a detached one.
    pub(crate) fn detach_subtree_styles_if_moved_out(
        &mut self,
        child: usize,
        old_parent: usize,
        new_parent: usize,
    ) {
        if old_parent != new_parent && self.depth_if_connected(new_parent).is_none() {
            self.detach_subtree_styles(child);
        }
    }

    /// Whether moving `child` to `new_parent` may keep the style it has, and
    /// skip the re-cascade an insertion gives a subtree (issue #914).
    ///
    /// Yes when the move stays **within one parent** and the child already
    /// carries a style. Its ancestor chain is then the one it was cascaded
    /// against, so every descendant, child and ancestor-attribute selector
    /// answers as before, and so does everything it inherits. What its
    /// position can change — `:nth-child`, `:first-/:last-child`, `+`, `~`,
    /// an `<ol>`'s numbering — is exactly what the selector flags on the
    /// parent say, and the verb's two `note_child_list_changed` calls (at the
    /// old index and the new) restyle the moved node itself whenever they do:
    /// `HAS_SLOW_SELECTOR*` and an `<ol>` mark every child from the insertion
    /// index on, which includes it, and `HAS_EDGE_CHILD_SELECTOR` marks the
    /// child at that index, which is it.
    ///
    /// This is **not** what a browser does for `insertBefore`: Chrome removes
    /// and re-inserts the node and discards its computed style. It is what
    /// `moveBefore()` does — a move that keeps the node's state — and rinch
    /// already treats a connected move that way on purpose (a move is not a
    /// detach: a running transition survives a keyed reorder).
    ///
    /// A move to **another** parent re-cascades, as an insertion always did:
    /// both the inherited values and the ancestor chain may differ, and
    /// comparing them to prove otherwise costs about what the cascade does.
    /// Neither the keyed `for` reorder nor the editor's view diff moves a node
    /// between parents.
    ///
    /// A child with no style yet — never styled, or its data dropped by an
    /// earlier insertion this frame — is cascaded as before.
    pub(crate) fn keeps_style_across_move(&self, child: usize, new_parent: usize) -> bool {
        let Some(node) = self.tree.nodes.get(child) else {
            return false;
        };
        node.parent == Some(new_parent)
            && node.is_element()
            && node
                .stylo_element_data
                .borrow()
                .as_ref()
                .is_some_and(|d| d.styles.primary.is_some())
    }

    /// Clear ifc_root on a node and all its descendants — and take each out
    /// of the anonymous block box whose run it was in.
    ///
    /// Every verb that moves or removes a subtree calls this. The run is its
    /// old container's: a scoped structural pass sets the *new* container up
    /// but reaches the old one only while it is still in the document, and a
    /// member that kept its `run_box` reads as "claimed by a box" to the new
    /// container's root detection, which then withheld roothood from a grid
    /// holding nothing but that text (found by the scoped pass's random
    /// differential: a text moved out of a mixed container that was removed
    /// in the same frame). The whole-document pass cleaned every box every
    /// time, which hid it.
    pub(crate) fn clear_ifc_root_recursive(&mut self, node_id: usize) {
        // Use iterative approach to avoid stack overflow
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            let Some(node) = self.tree.nodes.get_mut(id) else {
                continue;
            };
            node.ifc_root = None;
            stack.extend(node.children.iter().copied());
            if let Some(b) = node.run_box.take()
                && let Some(boxx) = self.tree.nodes.get_mut(b)
            {
                boxx.run_members.retain(|&m| m != id);
            }
        }
    }

    /// Invalidate IFC state for a parent element.
    /// Clears text_layout on the parent and ifc_root on all its inline children.
    /// Also marks the Taffy node dirty so the measure callback re-fires.
    pub(crate) fn invalidate_parent_ifc(&mut self, parent_id: usize) {
        // The mark on the #466 measure leaf inside matters here: without it a
        // text edit in a `text + absolute` container serves the leaf's cached
        // measure and the container's height never changes.
        self.invalidate_ifc_root(parent_id);
        // NOTE: Do NOT clear ifc_root on children here. This function handles
        // text/style invalidation where the IFC structure is unchanged. Clearing
        // ifc_root would prevent build_ifc_layouts() from finding this IFC root
        // (it discovers roots by checking child.ifc_root == Some(parent_id)),
        // and setup_inline_formatting_contexts() won't re-assign them because
        // ifc_dirty is not set for text-only changes.
    }
}
