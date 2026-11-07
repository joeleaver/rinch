# Dependencies: Taffy, Parley, skrifa, Stylo (pins and their traps)

Read this page when your task touches: a Taffy, Parley, skrifa, Stylo, wgpu or Vello bump, text shaping/line breaking, white space, tabs, NBSP, justify.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

## Project Overview

Rinch is a lightweight cross-platform GUI library for Rust, built on rinch-dom, Taffy, Parley, and dual rendering backends (Vello for GPU, tiny-skia for software). The goal is to provide a reactive GUI framework using HTML/CSS for layout.

**Key dependencies:**
- **rinch-dom** - HTML/CSS DOM implementation (Taffy for layout, Parley for text, Painter trait for rendering). Taffy is pinned at **0.14** in one place, `crates/rinch-dom/Cargo.toml`, and comes from rinch's fork (see **Taffy Fork** below) (its `[dependencies]` and `[dev-dependencies]` lines must agree, **features included**: rinch builds Taffy without the four default features it never reads — `content_size`, `calc`, `flexbox_balance`, `detailed_layout_info`; `float_layout` was the fifth until floats came in — which cost about 4% of a structural edit's instructions whether read or not (#1260), and a dev-dependency left on the defaults would unify them back into every test build; the vendored `crates/stylo-taffy` that used to pin it too was unused and was deleted with the 0.14 bump, #1236). 0.9 could not resolve a percentage `min-height`/`max-height` against a block containing block (its block algorithm hard-coded that basis as indefinite), so `min-height: 100%` silently collapsed to the content height. 0.12 numbered grid lines as `i16` and **panicked** on a grid past 32,767 lines (40,000 auto-placed rows, stacked `span`s, #1210) and on `repeat(auto-fill, 0px)` or `minmax(0px, 1fr)` at any width (#1231); 0.14 clamps each grid axis at **10,000 tracks** instead (css-grid-1 §7.2.4), which is not Chrome's geometry: every item past the 10,000th track lands in the last one and they overlap, so a grid of 10,001–32,767 lines that 0.12 laid out correctly now overlaps too (`crates/rinch-dom/tests/grid_line_overflow_tests.rs`). Both measure closures (`layout_engine.rs`, `ifc.rs`) return a `LayoutOutput` through `taffy::compute_leaf_layout` with the same `0.0` calc resolver 0.12 used, and set its first baseline (`ifc::with_first_baseline`, #1013): an IFC root's or text leaf's first Parley line's baseline plus the box's top padding and border, since Taffy measures from the border-box top. `ifc_measure_cache` keeps the baseline beside each size (`IfcMeasure`), and Taffy's block algorithm hands a block item its first in-flow child's, so flex/grid `align-items: baseline` lines items up by their first lines (`tests/flex_baseline_tests.rs`). It used to synthesize every item's from its bottom border edge. A **form control** still reports none and is aligned by its bottom edge (an `<input>` beside a 32px line sits 6px higher than Chrome's, #1273), and so is an image, which is right. An item whose first line holds an atomic inline (an `inline-block`, a `Button`) is aligned by that box's baseline since parley 0.12 (#663): beside a 32px line Chrome 156 puts the `A` at 0 and the item at 1, as rinch does (it put the `A` at 18 before, 9 before #1013), pinned by `an_items_first_line_holding_an_inline_block_is_aligned_by_its_baseline_issue_663`. `min_size`/`max_size` are `LengthPercentageAuto` in 0.14 (`DimensionValue::to_taffy_lpa`), and `size`/`flex_basis` carry the intrinsic keywords, which rinch hands over since #691 (see **Common Issues**). The bump itself moved layout in a few shapes, every one to Chrome 153's answer (`crates/rinch-dom/tests/taffy_014_layout_changes_tests.rs`): flex auto margins absorb free space **before** `justify-content` (0.12 justified first and pushed an auto-margin item off the end), a `min-height` parent keeps its last child's bottom margin inside it, and an empty scroll container's margins collapse through it — so a horizontal `Tabs` nested in a vertical `Tabs` now gets an inner panel 44px wide where 0.12 gave 85 (Chrome: 42.9). An editor table of more than 10,000 rows (which #1209 places by auto-placement past 9,999 tracks) now overlaps its rows from the 10,000th on, on desktop.
- **parley** - Text shaping and line breaking, at **`0.12.0` from rinch's fork** (see **Parley Fork** below; from 0.11.1 by
  the parley 0.12 upgrade, Refs #624; Joe's decision of 2026-10-29). It was a git `rev` on an
  unmerged 22-commit "Floats WIP" branch off `v0.7.0` until #659; upstream landed that work in
  0.9.0, so nothing was lost by moving to the release line. The workspace line is
  `features = ["complex-scripts", "bytemuck"]` (`bytemuck` casts `Run::normalized_coords`, a
  `&[NormalizedCoord]` since 0.12, to the painters' `&[i16]` for free). Two things about the
  dependency line are load-bearing and neither is enforced by the compiler:
  - **`features = ["complex-scripts"]` is on, and must stay on.** It is not in parley's `default`.
    Without it icu4x has no Thai/Lao/Khmer/Burmese segmentation dictionary and a Thai paragraph
    finds **no** line-break opportunity at all — measured, 4 lines become 1 and the text runs off
    the container — plus an `ICU4X data error: No segmentation model for language: th` on stderr
    **twice per layout**. CJK is *not* affected (its breaking comes from Unicode line-break
    classes), so a CJK fixture cannot stand in for a Thai one.
    `crates/rinch-dom/tests/complex_scripts_tests.rs` is the pin.
    It is not free: the dictionaries cost about **+3.80 MB of stripped release binary**
    (measured on this host by #659's review, on a one-crate probe differing only in the parley
    dependency line — no `icu_*` crate is *added*, they were already in stylo's graph). It stays
    on by default because CJK and Thai line breaking should be correct out of the box; **#660**
    is where exposing it as a rinch cargo feature for size-sensitive embed/wasm builds is being
    considered.
  - **`parley::Glyph::y` is Y-down** since parley #528 (0.8.0). The struct did not change, only the
    sign of what parley puts in it, so a consumer that subtracts it still compiles and still looks
    right in English: `y` is 0 for ordinary Latin shaping and non-zero only for mark positioning.
    `crates/rinch-dom/src/paint/text.rs` **adds** it, in the main pass and the shadow pass alike;
    `crates/rinch-dom/tests/glyph_y_ydown_tests.rs` is the pin, one fixture per site.

  What 0.12 changed, and what rinch does about it:
  - **Line boxes are CSS line boxes** (linebender/parley#697, #863, #864). A line holds the root
    style's box — the **strut**, the block's own font and `line-height` — and every span's box
    with its own line height, and an inline box sits by its baseline (`InlineBox::baseline`,
    `None` = its bottom edge) and `vertical_align`. So #577 (a span's larger line height or font
    raises the line it shares) and #624 (a line holding only an atomic inline keeps the block's
    line height) are parley's now: `tests/line_box_metrics_tests.rs`, ignored until 0.12, is live.
    With the strut, a box lined up by its bottom grows its line by the strut's descent, so rinch
    hands parley each atomic inline's **baseline** (#663): `ifc::atomic_inline_baseline`, read
    when its detached compute is read back (`Node::atomic_baseline`) by walking the box's Taffy
    subtree to its last (an `inline-flex`/`inline-grid`: first) line, whose baseline the compute's
    IFC and text-leaf measures record by content size (`NodeTree::atomic_line_baselines`); none
    for a replaced element, a box with no line or `overflow` other than `visible`/`clip`; an
    `<input>` by its centred text (`form_control::input_baseline`, from the primary font's
    ascent and descent in `CharMetrics`). The baseline is in the IFC content signature, a box that
    became an atomic inline by a restyle is measured before the IFC around it
    (`Node::atomic_baseline_known`), and an atomic inline's `vertical-align` is parley's
    (`ifc::atomic_vertical_align`; a text span's is still rinch's glyph shift).
    `tests/atomic_inline_baseline_663_tests.rs` pins Chrome 156. Not done: a zero-height atomic
    inline alone on a line still makes a 0px line (Chrome: the strut's; parley counts a box with
    no extent as no content), and the **horizontal** margins of an atomic inline take no room on
    its line (#1405). Its top and bottom margins do: the `InlineBox` parley lines up is the box's
    margin box on the block axis (`ifc::atomic_vertical_margins`, its length margins; a
    percentage counts 0), its baseline moved down by the top margin, and
    `write_inline_positions` puts the border box back inside it, so a box with no baseline (an
    `<img>`) sits its bottom margin edge on the baseline, as CSS 2.1 §10.8 says. Before, an
    `<img style="margin: 20px 0 35px">` in a line of its own took 55px less than in Chrome. No
    other reader of a positioned inline box reads its `y` or `height` (they read `x`/`width`,
    or the node's own layout). A change of those margins is a change of the line: the text-layout staleness predicate compares them for an inline-level box (`ComputedStyle::same_atomic_line_margins`), the IFC content signature hashes them beside the box's size, and a `margin-top`/`margin-bottom` transition frame on an atomic inline re-measures its line (`tick_transitions`), since the box's own size does not move and the re-measure of a dirty atomic inline compares only that. An inline-block whose `height` or `min-height` is declared is never measured at its own width (Taffy needs no measure for a leaf of known size), so its last-line baseline is read from the top of its content box, not counted up from its bottom (`taffy_baseline`): counted from the bottom, a 40px box's one line sat 20px low, which a zero top margin hid. `tests/atomic_inline_vertical_margin_tests.rs` pins Chrome 156, and the restyle and transition cases against fresh layouts.
  - **A layout with no item keeps one line, the strut's**, for a caret. CSS makes such a line box
    zero-height, so an IFC with no text and no inline box is handed one zero-size out-of-flow box
    (`ifc::EMPTY_IFC_BOX`, an edge-box id no node owns): parley then makes the line invisible.
  - **An anonymous block box now matters to its text's line height** (its style is the strut), so
    a container's restyle refreshes its boxes' inherited style
    (`invalidate_text_layout_for_node`); they kept the style they were minted with until the
    next structural pass.
  - **`LineMetrics` lost `ascent`, `descent`, `leading`**; `block_min/max_coord` are the line box
    and `content_block_min/max_coord` the content (strut included). `ifc::line_ascent` /
    `line_descent` read the content extent; caret tops, selection rects and line hit tests use
    `ifc::line_top` / `line_bottom` — the line box or the line's **runs'** glyph extent, rounded,
    whichever reaches further (0.11.1's `block_min_coord`; not the content box, whose strut put a
    16px run's caret 8px above its line in a 32px block). `LineBreakData::advance` is
    `metrics.advance`; `trailing_whitespace` is `hanging_advance` and holds what actually hangs.
  - **Clusters are grapheme clusters** (#715): `ง่` is one cluster of two characters, and
    `BreakLines::break_next_with_length` counts characters, so the hang pass's `LineUnit::counted`
    is a cluster's character count. A glyph's style is its cluster's (`Cluster::style_index`).
  - **White space hangs as CSS Text 4 says** (#762, #784, #785, #790): preserved spaces hang at a
    soft wrap and, before a forced break or at the end, only as far as they overflow; an NBSP
    never hangs; a justified line's opportunities are CSS's. rinch hands parley every element's
    `white-space-collapse` (`ifc::white_space_hanging`): `Preserve` — rinch has already collapsed
    the text — or `BreakSpaces`, whose spaces never hang. The 0.11.1 fix-ups for all of that are
    gone (below).
  - **A `direction: rtl` block's paragraph is right to left** (`TreeBuilder::set_base_direction`);
    an `ltr` one is still parley's first-strong guess. `text-align: left`/`right` are
    `TextAlignValue::Left`/`Right`, no longer folded onto `Start`/`End`.
  - **Known differences from Chrome that 0.12 brought** (each pinned where it is measured): a
    `nowrap` root's spaces after a wrapping span's text no longer offer a break (`g2_55`,
    `g2_80` of `tests/soft_wrap_1238/chrome155.tsv`), parley misses the opportunity before an
    image after `pre` text and stops at an earlier one (`p704`), a glued word followed by
    ` \u200b ` takes the zero-width space onto its overflowing line
    (`nbsp_glue_line_break_tests`), and a collapsible space then `break-spaces` spaces break
    after the first `break-spaces` space (`pre_space_seams_1243_tests`). Six cases of the same
    corpus moved to Chrome's.
  - **An inline box too wide for any line leaves an empty line after it** when it is the
    paragraph's last item (its emergency break returns without marking the breaker done), and
    with text before the box that line is counted at the box's height. `ifc::phantom_last_line`
    finds it and `break_lines_hanging_spaces` breaks the paragraph again without it (#1050),
    counted as `ifc_phantom_rebreaks`;
    `crates/rinch-dom/tests/atomic_inline_line_box_tests.rs` is the pin. And that branch commits
    the box's line on the spot, so a `<br>`, a preserved newline or spaces right after the box
    made a line of their own (`x <ib><br>` was three lines where Chrome 155 makes two):
    `ifc::fold_after_overflowing_box`, in the hang pass, commits the box's line again by length
    with the run of hangable spaces after it, or a newline right after it (spaces a newline
    follows keep a line of their own, as in Chrome 155) (#1081);
    `crates/rinch-dom/tests/overflowing_box_line_end_1081_tests.rs` is the pin. A line holding
    only inline boxes still has an inverted text range (`usize::MAX..0`), which breaks parley's
    byte-to-line binary search for the text after it (#1082).
  - **A text-final newline gets an empty line after it**, and it is counted: a `<br>` is pushed
    as `"\n"`, so `<div>a<br></div>` and `<pre>a\n</pre>` were two lines where CSS makes one (a
    line after the last forced break exists only when something follows it). `phantom_last_line`
    drops that line too — last line, empty text range, no inline box, after an `Explicit`
    break — for an IFC and for a text leaf (`ifc::shape_text_leaf`, both leaf measure sites)
    (#1172); `crates/rinch-dom/tests/trailing_forced_break_tests.rs` is the pin. **Not** a
    `<textarea>`, which is laid out by the form-control path and keeps its empty last line, as
    Chrome does (that is where its caret goes). The rich-text editor needs the line after a
    textblock's trailing hard break **or** a code block's text ending in `"\n"`, so its view renders
    an unmodelled `<br data-pm-trailing-break>` there (`ViewDesc::sync_trailing_break`,
    ProseMirror's rule for `ProseMirror-trailingBreak`) on both backends. The web draws the caret
    after a `<br>` followed by text or another `<br>` from what follows it
    (`WebDocument::br_caret_viewport_rect`, its `query_inline_box_caret*`; a `<br>` before an
    image is still answered by byte), which also put the caret after the first of two breaks on
    the empty line between them, not the line after. A caret Chrome gives no collapsed-range
    rect — measured in Chrome 153: right before a preserved segment break (`"\n"` or `"\r"`: an
    empty line inside a code block, the start of one that begins with a newline), after a
    text-final `"\n"`, and in an empty text node, though not at every such position — is drawn
    where what follows it starts: the next character's own box (a newline's is zero-width), else
    the same "what follows" walk (`caret_where_next_starts_at`, #1202; it used to fall back to the block's whole box).
  - **An overflowing NBSP was hung like a space** until 0.12 and the line committed right after
    it, where UAX #14 class GL allows no break (`x yyx&nbsp;Wkxxpq` at 40px broke as
    `x yyx&nbsp;` / `Wkxxpq`). `ifc::unglue` breaks a line parley ends after an overflowing NBSP
    again where Chrome 153 does (#1218), by a galloping search over the line's units. Since 0.12
    no shape in the suite reaches it (an instrumented run over the NBSP, line-separator and #1238
    suites never did); it stays for a line parley could still end there.
    `crates/rinch-dom/tests/nbsp_glue_line_break_tests.rs` (Chrome 153) and
    `nbsp_glue_oracle_tests.rs` (a greedy token oracle) are the pins.
  - **Preserved spaces did not hang in 0.11.1**: parley hung the first overflowing one and
    committed the line, so the rest made one more line, and rinch's hang pass put them back
    (#1018). 0.12 hangs them itself. A `break-spaces` space or tab never hangs (#1043; parley's
    `BreakSpaces`); the hang pass still breaks a line parley ended past the box in such spaces
    (`ifc::break_overflowing_spaces`) after the last one that fits, else before the word they
    follow, else after their first space (Chrome 155, `tests/white_space_break_spaces_1043_tests.rs`).
    The hang pass (`ifc::hang_pass`) is **one** extra break of the paragraph, line by line,
    however many lines it fixes, and runs only for a paragraph with edge boxes at a soft wrap,
    seams, or a box parley placed by its emergency branch; `ifc_hang_passes` counts it.
Likewise an IFC's measure is `InlineLayout::measured_width`, not
    `Layout::width`: the latter drops a preserved trailing space that Chrome counts, and a box
    sized without it wraps the space onto a line of its own (the editor's list bullet dropping a
    line after "text "). Only when the IFC holds preserved text (`pre`, `pre-wrap`, `break-spaces`, a
    contenteditable root): collapsible spaces, `pre-line`'s included, are gone at a line's end
    before parley sees them.
    `crates/rinch-dom/tests/list_item_trailing_space_tests.rs` is the pin.
  - **Trailing white space and justification** were rinch's until 0.12: parley 0.11.1's `align`
    hung every line's trailing white space, NBSP included, so a right-aligned or centred line
    ending in spaces that fit was narrowed by the hang pass (#1042, `align_unhung_trailing`),
    and its justification counted every space that fit, so a justified line was committed again
    by length (#1056, `justify_line`) — one extra break of most justified paragraphs. 0.12 does
    both as CSS does; the code is gone, and `perf_regression_scenarios::*_pays_no_pass` pin that
    such paragraphs no longer pay the pass. `text_align_trailing_spaces_1042_tests.rs` and
    `justify_hanging_spaces_1056_tests.rs` (Chrome 155) are the pins, now with the carets in
    hanging spaces, a text leaf's trailing spaces and a right-to-left line at Chrome's numbers too.
  - **rinch collapses an IFC's white space itself; the IFC's tree builder always hands parley
    `Preserve`.** (The ranged builders — form controls, text leaves — are separate and unchanged.)
    `WhiteSpaceCollapse::Collapse` collapses and trims per *style span*, and rinch pushes one span
    per inline element: it trimmed every inline element's edges, so `a<span> b</span>` laid out
    `ab` and `rsx! { p { "Hello" b { " world" } } }` `Helloworld` (#1180); and its trims take every
    Unicode `White_Space` char, so `<div>&nbsp;</div>` had no line box (#1154). CSS collapses
    across the whole inline formatting context and removes spaces only at a *line's* start and
    end. `ifc::IfcText` does that: `walk_inline_children` records its builder ops into it, it
    collapses each run of spaces, tabs and segment breaks (not U+000C FORM FEED, which Chrome
    draws — #1181) to one space across element boundaries, removes a
    space at the IFC's start or after a forced break, holds the last kept space until content (a
    character, an atomic inline) confirms it or a forced break / the IFC's end removes it, and replays
    the ops under `Preserve`. **Each text node is collapsed by its own element's `white-space`**
    (its DOM parent's, `ifc::SpaceCollapse`; #1192), not the root's: `normal`/`nowrap` collapse,
    `pre`/`pre-wrap` keep everything, `pre-line` collapses spaces and tabs and keeps a newline as a
    forced break. A preserved space is content, so a collapsible space after one is kept and one
    before it is not at a line's end; a preserved newline is a forced break like a `<br>`. A
    `contenteditable` root keeps every node's text verbatim. (Chrome 153 counts a collapsible
    space before a preserved newline in the max-content width while laying it out removed; rinch
    removes it from both.) A space at a soft wrap is still parley's to hang. U+2028/U+2029,
    which parley reads as forced newlines, are handed to parley as NBSP + ZERO WIDTH SPACE under
    every `white-space` (Chrome 153 draws a space-wide character that never collapses, trims or
    hangs, with a break opportunity after it), U+0085 as a ZERO WIDTH SPACE, and a preserved
    U+000C as nothing (`ifc::laid_out_as`, #1181; the offset map records the length
    change; `letter-spacing`/`word-spacing` on the substitutes still differ from Chrome's).
    The rich-text editor's caret map counts the same bytes for each through
    `DomDocument::substituted_char_flat_bytes`, answered from the table `laid_out_as` reads
    (`ifc::PRESERVED_SUBSTITUTES`) — as it counts a tab through `tab_flat_bytes` (#1109; 1 since #1122).
    **A flex or grid item's own text (a text leaf) goes through the same functions** (#1279,
    #1672): `ifc::LeafText` collapses it with `IfcText` by its element's `white-space`
    (substitutes included, a preserved tab kept as a tab — #1585), `ifc::shape_text_leaf` breaks
    it with `break_lines_hanging_spaces` (hung preserved spaces, `break-spaces`, the final
    forced break) and measures it with `node::measured_width_of`, `InlineLayout::measured_width`'s
    rule; it used to hand parley its text raw, so `<div style="display:flex">Use </div>` was two
    lines tall. A leaf that is all spaces, tabs and newlines measures 0x0 and reports
    no first baseline (`LeafShape::blank`), under every `white-space`; Chrome makes no item of it at
    all (css-flexbox-1 §4), rinch's is still an item and takes a `gap` (#1760). Offsets into a leaf's layout are offsets into its collapsed text,
    as an IFC root's are into its flat text; the leaf keeps no DOM↔flat map.
    `crates/rinch-dom/tests/text_leaf_white_space_1279_tests.rs` is the pin.
    **The flat offsets index that collapsed text** — the caret maps,
    inline backgrounds and decorations, the visibility mask; they used to count the pushed text,
    one byte late per collapsed byte — and `IfcTextRange::offset_map` is each text node's
    DOM↔flat correspondence. `measured_width` keeps a trailing NBSP (`width_keeping_nbsp`),
    which parley 0.11.1 counted as trailing white space; since 0.12 it never hangs.
    `crates/rinch-dom/tests/inline_edge_space_tests.rs` and `nbsp_line_box_tests.rs` are the pins.
  - **Parley has no tab stops; rinch sets them** (#1122). A preserved tab
    (`pre`, `pre-wrap`, `break-spaces`, a contenteditable root) is pushed as
    one space, one flat byte, recorded in `IfcText::tabs`; `IfcText::replay`
    gives each its own span of `word-spacing: 0` and a `letter-spacing` that
    widens it to its stop, and `build_inline_layout` builds and breaks again
    while a tab is off its stop (`ifc::TabStops::deltas`), at most
    `MAX_TAB_BUILDS` = 4 builds; `ifc_tab_rebuilds` counts the extra ones: 1 for
    lines that do not wrap. Each build settles one more moved soft wrap, so a
    long soft-wrapped `pre-wrap` paragraph can stop at the cap with some tabs
    off their stops, 3.8–5px from Chrome's, from about its fifth line on (Refs
    #1593; `review_1587_tests.rs` pins a paragraph that needs all four). The rule is Chrome 155's: stops every `TAB_SIZE`
    (8) spaces of the **block's** font, its letter- and word-spacing included,
    from the line's start edge in logical order before `text-align`; the first
    stop strictly past the tab's start, or the next when that is under half a
    space away; an overflowing tab hangs. `tab-size` is Gecko-only in Stylo and
    not read (#1584). Since the tab is one byte, `tab_flat_bytes` is 1 on
    rinch-dom as in a browser. Not done: a text leaf's and a form control's tab
    (#1585). A `text-overflow: ellipsis` cut keeps every tab at its stop:
    the cut is made at paint over the lines as laid out (#1100; until then a
    flat rebuild drew a tab one space wide, #1586).
    `tests/tab_stop_1122_tests.rs` is the pin.
  - **parley has no per-span padding; rinch pushes edge boxes** (#1426). A
    `display: inline` element's inline-start margin + border + padding is a
    zero-height in-flow `InlineBox` before its content and its inline-end ones
    one after (`IfcText::push_edge_box`, ids `ifc::EDGE_BOX` | owner, inside 32
    bits so a 32-bit target cannot read one as a node id), so what follows
    moves by them, the start ones stay on the first line and the end ones on
    the last (`box-decoration-break: slice`), and right to left takes the
    right side first. They hold no text bytes: flat offsets, the DOM↔flat maps
    and the editor's caret maps are unchanged, and a byte's caret is drawn
    before the box at it, except at a line's start (`|x` inside a span
    mid-line outside its start padding,
    `|tail` after it inside its end padding; the editor draws its caret on
    the side its marks say, `docs/internals/editor.md`). **Every walker of a layout's inline boxes that maps a box to a
    node asks `ifc::is_edge_box` first** (positions, paint, layer bounds,
    span fragments, text queries). parley allows a break after every
    in-flow box and before one that does not fit, which is wrong for an edge
    box, so a paragraph with a soft wrap next to one takes the hang pass
    (`any_line_split_at_edge`), and a line parley ended where CSS does not
    break (`edge_break_allowed`: after the run's end boxes and before its
    start boxes, at a UAX #14 opportunity of the text around them, where
    `white-space` allows one: `ifc::wrap_allowed_after`) is committed again by
    length at the last allowed break that fits, else the first (`css_line`,
    `commit_css_line`; generalised to atomic inlines by #1238) — one extra
    break of such a paragraph per layout. A percentage margin or padding counts 0 and a
    negative total is clamped to 0 (Chrome lets a negative margin pull the
    content back). A split inline (#513) gets no edge boxes.
    `tests/inline_edge_room_1426_tests.rs` is the pin.
- **skrifa** - read directly by rinch-dom only to size a text control from its font's OS/2 and `head` tables (#1177). Pinned at **0.44**, the version parley 0.12.0 (and 0.11.1 before it) depends on — and Vello 0.11 too (`vello`/`vello_encoding` ^0.44; the Vello 0.7 rinch had before #1955 built a second skrifa, 0.40). parley does not re-export it, so a parley (or Vello) bump that moves skrifa silently builds a second copy unless this pin moves with it. (swash 0.2, the software painter's glyph rasteriser, still brings its own skrifa 0.37.)
- **stylo** - the CSS engine, at **0.22** (crates.io `stylo`, servo feature), with the family at the
  versions it builds with: `stylo_traits`, `stylo_dom`, `stylo_atoms`, `stylo_static_prefs` 0.22,
  `servo_arc` 0.5, and in `crates/rinch-dom/Cargo.toml` `cssparser` 0.38, `selectors` 0.41,
  `web_atoms` 0.3. Move them together: a second `cssparser` or `selectors` is a second copy of every
  type that crosses the boundary. Upgraded from 0.11 (with selectors 0.35, cssparser 0.36) for
  `:has()` (#680), as a dependency-only change. What a bump has to know:
  - **Preferences are process-global atomics** (`stylo_static_prefs::set_pref!`, defaults in that
    crate's `preferences.toml`); 0.11 split them between a compile-time macro and the `stylo_config`
    store, which 0.21 dropped. `dom_impl::set_stylo_prefs`, run by every `RinchDocument::new` before
    any parsing, turns on `layout.grid.enabled` and `layout.unimplemented`, and holds the rest at
    0.11's answers: `layout.threads` 0 (0.22 defaults to −1, a style thread pool of up to six), and
    off `@property` (`properties-and-values`, off in 0.11) and five gates of syntax 0.11 did not
    parse that upstream turns on (`alpha()`, typed `calc()` arithmetic, multi-colour `color-mix()`,
    `revert-rule`, `@supports at-rule()`). Turning any on is its own change. **`:has()` is
    `layout.css.has-selector.enabled`**, off; the follow-up to #680 sets it there.
  - **Every computed `font-size` is rounded to 11 significant bits**
    (`specified::FontSize::quantize_font_size`, Gecko's rule, no pref): the form controls' and
    `smaller`'s 13.3333px is 13.328125, 16.6667 is 16.65625, 19.2 is 19.1875 (at most 2⁻¹¹ of the
    size; an integer size up to 2048px is exact). Chrome does not round. The four
    `ua_*_defaults` pins assert the rounded values and name Chrome's.
  - **`attr()` is a substitution function** (css-values-5), in any property, resolved in the
    cascade through `TElement::get_attr`; `ContentItem::Attr` is gone, and the value reaches
    `content` as a string. `get_attr` sets `Node::content_reads_attrs`, which
    `note_attribute_change` reads.
  - `vertical-align` is a shorthand of `alignment-baseline`, `baseline-shift` and
    `baseline-source` (`from_stylo::box_model::vertical_align_from_stylo` reads the first two);
    `list-style-type` is a `<counter-style>` (a name, not a keyword enum); `flex-wrap` is a bitflag
    set (with `balance`, behind `layout.flexbox.balance`, off); a specified `Number`, `Angle` or
    `Length` can hold an unresolved `calc()` (`resolve()`/`degrees()` answer `None` for one);
    `NoCalcLength` is a unit and a value, not an enum of enums.
  - `TElement::borrow_data`/`mutate_data` return `ElementDataRef`/`ElementDataMut`, which only an
    `ElementDataWrapper` hands out, so `Node::stylo_element_data` is a `node::StyloData` (the
    wrapper plus a present flag) where it was `AtomicRefCell<Option<ElementData>>`. The wrapper's
    borrow checks run in debug builds only.
  - Newly parsed, and laid out by nothing in rinch: `text-underline-offset` and `scrollbar-color`
    (servo-enabled behind `layout.unimplemented`; rinch does not read either, #580). Newly
    evaluated media features: `height`, `aspect-ratio`, `orientation`, `device-width`/`-height`
    (the window's size, `build_device`), `hover`/`any-hover`, `pointer`/`any-pointer` (stylo's
    default: fine with hover on desktop, coarse on Android); with 0.11 a query on any of them never
    matched. The `jump-*` step positions parse (#1581).
  - The `cssparser` API lost its lifetimes: `Parser::new(&str)` replaces `ParserInput`, and
    `ParseError<E>` carries no location (`ParseError::custom(e)`).
- **vello** - 2D GPU rendering via wgpu (GPU mode, enabled with `features = ["gpu"]`). **Vello 0.11 on wgpu 30** (#1955; workspace `[workspace.dependencies]`, every crate takes `workspace = true`), no wgpu patch (`app-and-windows.md` "wgpu Fork"). Vello pins wgpu's major (0.8 → 28, 0.9/0.10 → 29, 0.11 → 30), so the two move together; peniko stays 0.6 (kurbo 0.13). Check `cargo tree -d` for a second wgpu/vello/skrifa after a bump. wgpu 30 needs wasm-bindgen ≥ 0.2.127 (CI's `wasm-bindgen-test-runner` follows `Cargo.lock`). Rendering changes met in 0.7 → 0.11: images are sampled at the pixel centre since Vello 0.9 (rinch dropped its own half-pixel shift for images; `platform.md` item 6); `ImageQuality::High` became bicubic (rinch uses Low/Medium only); gradients are still evaluated at the pixel corner and the ramp is unchanged for the default `InterpolationAlphaSpace::Premultiplied`. `anyrender`/`anyrender_vello` were removed in #1955: no rinch code used them, and they pulled a second (crates.io) Vello. **Vello comes from `joeleaver/vello-fork` at a pinned rev** (workspace `vello = { git, rev }`; its `vello` takes the patched `vello_encoding` by path): Vello 0.9+ keeps its image atlas across renders, and the fork fixes the atlas being wiped by a render with no images, gradients or glyphs and images staying resident two renders after their last use (`app-and-windows.md` "Vello Fork"). A dependency, not a `[patch]`, so apps get it with nothing to copy; they use `rinch::vello`. Nothing else may pull crates.io `vello` (check `cargo tree -d`), or the graph has two Vellos. Images reach Vello as one blob per image (`DecodedImage::blob`, `frame_blob`), so a repainted image stays resident.
- **tiny-skia** - 2D software rendering (default mode, no GPU required). **0.12** since #2111 (was 0.11): resvg 0.49 needs 0.12, and winit's `sctk-adwaita` already built 0.12 on Linux, so the bump removed a duplicate rather than adding one. The one API change met: `RadialGradient::new` takes a start radius (`skia_painter`, `0.0`, the 0.11 behaviour). rinch-dom's tests passed unchanged on 0.12.
- **resvg / usvg 0.49** - SVG files as images (#2111, `svg_image.rs`), a plain dependency of rinch-dom (no feature gate: the cost below is small, and a build without it would quietly draw nothing for an `.svg`). **Default features off**: no `text` (fontdb + harfrust + skrifa shaping; SVG `<text>` is not drawn — usvg can take a `fontdb::Database`, not rinch's fontique collection, so supporting text means a second font database), no `system-fonts`/`memmap-fonts` (a fontconfig scan at startup), no `raster-images` (an `<image>` inside a file is refused anyway), no `svgz`. 0.49 is the version whose graph lines up: tiny-skia 0.12, kurbo 0.13 (peniko's), roxmltree 0.21 (already built), base64 0.23 only behind `writer`, which is off. **New crates**: resvg, usvg, svgtypes, simplecss, data-url, imagesize, pico-args, float-cmp (eight), no second copy of anything: `cargo tree -d -e normal --target all` lists the same set as main minus tiny-skia/tiny-skia-path. 0.45 (tiny-skia 0.11) would have added kurbo 0.11, roxmltree 0.20 and svgtypes 0.15 as second copies. `roxmltree` is locked at **0.21.2**: 0.21.1 recursed once per nesting level and a 28 KB SVG overflowed a load thread's stack in release (review of #2125; rinch's own `svg_image::prescan` also caps nesting before it parses). rinch-dom also names `svgtypes = "0.16"` (usvg's own) for the root's `viewBox`/`preserveAspectRatio`. An `embed` build (no software painter) gets tiny-skia through resvg; that is its rasteriser for SVG files. Measured on #2111 (release `hello-rinch-dom`, desktop features, against the same main): the binary grows 37,006,952 → 37,630,680 bytes (+624 KB, +1.7%), stripped 26,218,680 → 26,707,112 (+488 KB, +1.9%). Build: the eight new crates take 20 s of compile time between them (usvg 10.6 s, the rest under 2.5 s each; `--timings`, `-j 4`, a loaded host), and dropping tiny-skia 0.11 saves 16 s, so a clean build is about even (wall clock 408 s on the branch, 466 s on main, inside the noise).
- **softbuffer** - Software window presentation (default mode)
- **winit** - Cross-platform windowing and input. **0.31.0-beta.4 from `joeleaver/winit-fork` (branch `rinch-patch-beta4`) at a pinned rev**, a workspace dependency, not a `[patch]` (#1962): upstream's tag plus the `wlr-layer-shell` window role. Apps copy no patch lines. Wayland drag and drop is upstream's data transfer API (`shell::os_file_drag`). The fork takes crates.io `dpi`, so there is one `dpi` in the graph (muda's too). Detail: `app-and-windows.md` "winit Fork".
- **muda** - Native menu support

**Design philosophy:** Declarative UI with fine-grained reactive programming. The `rsx!` macro generates DOM construction code with Effects for reactive updates. Only changed DOM nodes are updated - no full re-renders.

## Taffy Fork

**Carried since #2080** (floats). rinch-dom depends on `taffy` from
https://github.com/joeleaver/taffy-fork, pinned by rev
(`f1d5882c981319c11cd09d8867df1c9ca15858a7`, branch `rinch-patch`): nineteen commits on the taffy `v0.14.0` tag
(item 10 is two of them, item 15 three). It is a dependency, not a
`[patch.crates-io]` entry, so an app on rinch gets it with nothing to copy, and
Taffy's types never reach an app (nothing re-exports them). The rev is pinned
so a push to the fork's branch never changes rinch; moving it is a commit here,
and a Taffy bump rebases the nineteen commits onto the new tag. All are upstream
candidates.

1. **`TaffyTree::compute_layout_with_block_measure`.** Taffy's block algorithm
   hands each in-flow child in the same block formatting context a
   `BlockContext` (the floats placed so far, `find_content_slot`,
   `place_floated_box`), but `TaffyTree`'s measure function was never given
   it, and the trait route around that (`LayoutBlockContainer`) needs
   `TaffyTree` internals that are private. The new method takes a measure
   function with a fifth argument, `Option<&mut BlockContext>`; the two
   compute sites (`layout_engine.rs`'s root compute, `ifc.rs`'s atomic-inline
   compute) use it so an IFC root wraps its lines around floats
   (`float_wrap.rs`, `common-issues.md` "Floats"). A box laid out while floats
   of its context are active (`has_active_floats(0.0)`) is computed afresh
   rather than answered from Taffy's layout cache, whose key does not hold
   where the floats are: a paragraph would otherwise keep lines broken around a
   float that has since moved. A document with no floats takes exactly the old
   path (the perf counter baselines are unchanged).
2. **`ContentSlot::height`.** `FloatContext::find_content_slot` always
   reported `f32::INFINITY`, so a caller could not tell where the room beside
   the floats next changes. It is now the distance to the end of the slot's
   segment (still infinity with no float beside it), which is how
   `FloatBands::of` walks a paragraph's height.
3. **Pending floats.** The block-measure method takes a second callback,
   `places_floats(leaf, context)`: a float right before a childless in-flow
   leaf that it accepts is sized but not placed. The leaf's measure finds it in
   `BlockContext::pending_floats()` and places it with
   `place_pending_float(index, min_y)` at the height it chooses; one it leaves
   unplaced is placed after it, where it stood. rinch accepts measure leaves
   only (`float_wrap::is_measure_leaf`): the floats before an IFC root's
   measure leaf are that root's own, placed at the top of the line each stands
   in (CSS 2.2 §9.5.1). A leaf with pending floats bypasses the layout cache too.
4. **A formatting-context root's bottom inset below its floats.** A box that
   establishes a block formatting context grew only to its floats' bottom
   margin edge, losing its bottom padding, border and scrollbar gutter: a
   padded `overflow: hidden` bar holding only floats was 30px tall where
   Chrome makes it 38. CSS 2.2 §10.6.7 grows the content box to include them.
5. **Dirty marking past uncached boxes.** A box laid out beside active floats
   keeps no cache entry, so `Cache::clear` answered `AlreadyEmpty` and
   `mark_dirty` stopped at it: text edited in a paragraph beside a float left
   its parent's cached layout in place. And an entry stored before the floats
   came was served again once they went, though the box's children had been
   laid out since. The bypass now clears the cache and counts the node
   computed (`Cache::clear_for_uncached_compute`), so marking climbs on.

6. **Sixteen measure entries per node, not nine** (#2136). Taffy 0.14's cache
   keeps nine exact-key size entries per node with clock eviction, so a node
   asked more distinct sizes per layout than that misses on every one of them
   on every layout: the cycle evicts each entry just before it is asked again.
   A grid item under a few intrinsic probes reaches ten — its min- and
   max-content widths, then its height under a min- and a max-content
   constraint at each of four column widths (Playweft's asset grid, in a
   CSS-grid panel in a flex column, is asked at 108, 115.85, 131 and 275.57) —
   and with nine a grid of 430 such cards re-measured every card whenever any
   one changed: 15,242 text measures for one changed label, 27 with sixteen.
   Sixteen is the width of the `recently_used_entries` bitmask, so going past
   it is a change of that field too. Pin:
   `perf_regression_scenarios::a_label_edit_in_a_grid_probed_at_four_widths_measures_one_card`.

7. **An auto-height grid's rows have indefinite available space** (#2139).
   A grid whose height is neither known (stretched, flexed) nor styled sizes
   its rows with `AvailableSpace::MaxContent`, whatever its parent passed
   (`compute/grid/mod.rs`, `available_grid_space`). Browsers lay a box's block
   size out from its content: Chrome 156 gives an auto-height grid of
   `overflow: hidden` cards holding 100px boxes rows of 100px in a 300px column
   flex container, a 300px grid parent and an `align-items: start` row flex
   container. Taffy shared the parent's definite height among the `auto` rows
   (69px each), and under a min-content height (the probe that gives a column
   flex item its automatic minimum size, css-flexbox-1 §4.5) sized them from
   the items' minimum contributions, zero for a scroll container, so a grid
   flex item was shrunk below its content and its rows collapsed. The spec's
   "available grid space" (css-grid-1 §11.1) does take the parent's space;
   rinch follows Chrome. A stretched or flexed height (`min-height: 0`), a
   styled `height` and a `min-height` above the content still share their
   free space among the rows; a `max-height` clamps the grid, not its rows;
   `repeat(auto-fill, …)` rows count from the grid's own height and
   `max-height` (Chrome's numbers for each). Pins:
   `crates/rinch-dom/tests/grid_contents_and_min_size_2138_2139_tests.rs`.
8. **`TaffyTree::compute_sizes_with_block_measure`** (#2141). Asks given nodes
   their size the way a parent asks a child for its contribution
   (`RunMode::ComputeSize`, no known dimensions), through the same measure
   functions as `compute_layout_with_block_measure`, and writes no final
   layout (`src/tree/taffy_tree.rs`). rinch's table column measure uses it to
   ask each changed cell its max-content width between layouts without
   evicting the final-layout cache entries the next layout reuses; the old
   probe laid the whole table out detached and so asked every cell twice per
   edit. Pins: `crates/rinch-dom/tests/table_column_weights_2141_tests.rs` and
   the `a_cell_edit_in_a_table_with_a_width_asks_that_cell_alone` scenario.

9. **`Style::stretch_past_max_width`** (#2264). A stretched grid item with the flag fills its grid
   area's width past its `max_size.width`, which still caps the contributions the grid measures
   (`compute/grid/alignment.rs` for the final size, `GridItem::known_dimensions` for the width its
   row height is measured at). rinch sets it on every `display: table-cell` box (not a `<td>`
   displayed as a block or flex box, which is a block in an anonymous cell): a cell's `max-width` caps what
   it asks of its column and the cell is laid out at its column's width (Blink's
   `CreateCellInlineConstraint`, `SetupTableCellConstraintSpaceBuilder`); css-grid-1 §6.2 clamps
   a grid item proper, the default. With it, `compute_leaf_layout` no longer clamps a **known**
   dimension by the leaf's own min/max size (the block algorithm never did), so a cell that is
   itself an IFC leaf wraps its lines at the column's width. Most parents hand a leaf a known
   size they have already clamped, so those don't move. Two hand it an unclamped one: the
   flagged grid stretch, and flexbox's intrinsic main-size path (`compute/flexbox.rs`, about
   line 1435), which measures an item at a known cross size it doesn't clamp by the item's
   min/max. A leaf measured there now sees that size as given. The review of #2268 found no
   case among 36 non-table and 8 aspect-ratio probes where it changes Chrome-matching output. Pins: the fork's `tests/stretch_past_max_width.rs` and
   `crates/rinch-dom/tests/table_cell_max_width_2264_tests.rs`.
10. **`Style::proportional_column_sizing`** (#2280; commits `9a7ae9e1` and `25caf91e` on
   `rinch-patch`). The inline axis shares space between its tracks as CSS tables share it
   between columns. The Maximise Tracks step (`maximise_tracks`) shares the free space in
   stages, as a table distributes its width between the guesses of css-tables-3 (Blink's
   `DistributeInlineSizeToComputedInlineSizeAuto`): first to the tracks whose max track sizing
   function is a percentage, then to those whose max is a fixed length (a `<td width=N>`
   column, `minmax(min-content, Npx)`), then to every track; within a stage in proportion to
   each track's room to grow (growth limit minus base size), so the stage's tracks reach their
   limits at once. An `auto` column is then its min-content width plus the same fraction of its
   max-content minus min-content (CSS 2.1 §17.5.2.2). A grid gives equal shares in one pass. The
   first commit shared a fixed column's room with the auto ones' in one pass, which squeezed
   mail's spacer columns (20/560/20 came out 6/588/6); the second adds the stages. And a
   spanning item's space beyond the growth limits of the tracks it spans goes to them in
   proportion to those limits (`distribute_item_space_to_growth_limit`), as Blink shares a
   colspan cell's max-content width (`DistributeColspanCellToColumnsAuto`). The block axis is
   unchanged. rinch sets it on every automatic-layout table since #2306 (`apply_table_style`;
   before, only on a table with no width of its own, and a table with a width used `fr`
   weights): `auto` columns are `auto` tracks (`minmax(min-content, auto)` in a table with a
   width), declared ones `minmax(min-content, Npx)`; a percentage column in a table with no
   width is still an `auto` track (a percentage track alone squeezes the other columns when the
   table should widen, #2284). Pins: the fork's `tests/proportional_column_sizing.rs` and
   `crates/rinch-dom/tests/table_auto_column_shares_2280_tests.rs`.

12. **A table's excess, a spanning cell's minimum and a table's width floor** (#2306, #2281,
   #2266; commits `1566e27e` and `54fac8c0` on `rinch-patch`). With `proportional_column_sizing`:
   the Stretch auto Tracks step gives the width past every column's limit (Blink's "max
   guess") to the tracks with an intrinsic max and a non-zero size in proportion to their
   sizes; when every such track is empty (size 0, an email's filler cell beside its logo), to
   those in equal shares (Chrome 156: `<td width=100>` and an empty `<td>` in 400px are
   108 / 280; the first commit gave 388 / 0, review of #2329); with none to the fixed-length ones in proportion to theirs, then the percentage
   ones, then equally (`distribute_table_excess`; a grid stretches its `auto` tracks
   equally); a spanning item's base-size contribution goes to its tracks in proportion to
   their room up to their growth limits and past them in proportion to the limits
   (`distribute_item_space_to_base_size`), as Blink shares a colspan cell's min-content:
   columns of max 124 and 44 under a 295px colspan are 217.7 / 77.3 in Chrome 156, 186 / 106
   with equal shares; and a grid that resolves its own width (no known width from its parent)
   widens to its tracks (`compute/grid/mod.rs`, CSS 2.1 §17.5.2.2). rinch sets
   `item_is_table` on a table with a width of its own and automatic layout, so Taffy's block
   algorithm hands it no width and it resolves its own. Pins: the fork's
   `tests/proportional_column_sizing.rs` (seven cases) and
   `crates/rinch-dom/tests/table_width_distribution_2306_tests.rs`.

11. **A float is laid out in its container's run mode** (commit `6e6feeeb`, review of
   #2313/#2319). The block algorithm laid every float out with `perform_child_layout`,
   also when the container was only asked its size (`RunMode::ComputeSize`: a table's
   column measure, an auto table's or a shrink-to-fit box's intrinsic pass). That pass
   wrote the float's descendants' layouts for the room of its question, a min-content
   width, and the final layout of the float, answered from its own cache, never wrote them
   back: in a Postrinch mail the one cell of an `align=left` table came out at its
   min-content width (147 of 420) and the table's height left the wrapped cell out, so the
   next block overlapped it. It took a second root compute to show (a sibling table whose
   columns moved). A float is now laid out with the container's run mode, as an in-flow
   child is. Pins: `crates/rinch-dom/tests/float_size_pass_2313_tests.rs` (Chrome 156's
   sizes; the cell fills its table, the next block is below it).

13. **`Style::fill_max_content`** (#2322, commit `82428ec2`). A grid whose
   percentage width has no basis, asked its size (an inherent-size request with
   no parent width: its contribution to an auto table cell, a float, an
   inline-block), answers a max-content width of `FILL_MAX_CONTENT_WIDTH`
   (1,000,000), or its definite available width when that is smaller, clamped
   by its min and max widths; its min-content width is its content's
   (`compute/grid/mod.rs`, where the preferred size is read). Blink's quirk for a
   `table-layout: fixed` table with a percentage width (`kTableMaxInlineSize`):
   the box around it fills its room and the percentage resolves against that.
   Chrome 156 caps it at 1,000,000 (an inline-block in a 2,000,000px column).
   rinch sets it on a fixed table whose `width` is a percentage
   (`apply_table_style`; a mixed `calc()` reaches Taffy as a resolved length
   and does not fill, where Chrome fills). Pins: the fork's
   `tests/fill_max_content.rs` and
   `crates/rinch-dom/tests/float_intrinsic_width_2322_tests.rs`.
14. **A content-sized block resolves its children's percentages; a leaf is
   measured beside the floats it places** (#2322, #2326, commit `d82d505f`).
   Two rules of `compute/block.rs`. A block whose width came from its content
   (a float, or another box laid out with no known width) reads its children's
   styles again once it has that width (`generate_item_list`), so a
   `width: 70%` child of an auto-width float is 70% of it (it was stretched to
   the whole float). And the floats right before a leaf that places them
   (item 3) share that leaf's lines: `determine_content_based_container_width`
   measures the leaf with a block context holding those floats, sized, and
   nothing placed (`BlockContext::is_intrinsic`), except at min-content, where
   each float is its own. rinch's measure lays the paragraph out with no float
   placed and adds each float to the line it stands in
   (`float_wrap::intrinsic_width_beside_floats`), as Blink's
   `NGInlineNode::ComputeMinMaxSizes` does: a cell holding `<float 100px>` and
   83px of text is 183 wide, where it was the wider of the two and the text
   wrapped below the float. Such a leaf takes the uncached path (item 1), as
   it does in a layout. Pins: the fork's `tests/rinch_intrinsic_widths.rs` and
   `crates/rinch-dom/tests/float_intrinsic_width_2322_tests.rs`.
15. **`Style::table_rows`** (#2312, commits `4be20203`, `627ea72d` and `f1d5882c`). Sizes a
   grid's rows as Blink sizes a table's (`compute/grid/table_rows.rs`, in place
   of the block-axis track sizing algorithm, from LayoutNG's
   `table_layout_utils.cc`). A row is as tall as its tallest cell of span 1 and
   at least its track's minimum (rinch writes a row's `height` as
   `minmax(<height>, auto)`); a row with such a minimum, or a span-1 cell with
   a length `min_size.height` (a cell's `height`), is *constrained*. A
   spanning cell taller than its rows grows, in Blink's order, the rows past
   its first where another spanning cell starts (equally), else the
   unconstrained rows that hold something (in proportion to their heights),
   else the last row when all are empty, else every non-empty row in
   proportion to its height. With `TableRows::sized`, a table taller than its
   rows gives the excess to its body sections (`TableRow::body`; all sections
   when there is none) in proportion to their heights as they were before any
   spanning cell grew their rows (Blink records a section's height before its
   rowspan step; that sum is also what the table's height is compared with), and each section to its
   rows the same way less the rowspan step, the empty rows growing when every
   row that holds something is constrained; a caption row
   (`TableRow::caption`) takes none of it; and a table shorter than its rows
   and captions grows to them unless its parent imposed its height (block
   layout hands it none, `BlockItemStyle::resolves_own_height`, and a size
   request does not stop at its `height`; a flex item's, an absolutely
   positioned table's or a grid item's height reaches it as `known_dimensions`
   and it does not grow, where Chrome does: #2358). Not modelled: percentage row
   heights and row baselines. `Style` is 32 bytes bigger. rinch sets it on
   every table, `sized` off for a block standing for an anonymous table
   (`apply_table_style`). Pins: the fork's `tests/table_rows.rs` and
   `crates/rinch-dom/tests/table_row_heights_2312_tests.rs`.

The fork's own tests (`tests/block_measure.rs`) cover 1–5; `tree::cache` tests cover 6; rinch's
fixtures named in 7 and 8 cover those; Taffy's suite (6,021 generated tests) passes unchanged with 7
and 9, (6,286 tests) with 10, (6,021 generated tests, 6,293 in all) with 12, (6,298 in all)
with 13 and 14, and (6,021 generated, plus `tests/table_rows.rs`) with 15.


## Parley Fork

**Carried since the parley 0.12 upgrade.** The workspace depends on `parley` from
https://github.com/joeleaver/parley-fork, pinned by rev
(`96699deb74d2b494af6aa27c1cccde2bd73028f2`, branch `rinch-patch`): commits on parley's
`v0.12.0` tag. Like the Taffy and Vello forks it is a dependency, not a `[patch]` entry. The git
dependency brings the fork's whole workspace (`parley_engine`, `parley_data`, `parley_emoji`,
`fontique`, `parlance`), so `rinch`'s own `parlance` line names the same repository and rev:
crates.io's `parlance` beside it would be a second copy of `GenericFamily`. Moving the rev is a
commit here; a parley bump rebases these commits onto the new tag. Nothing is filed upstream.

1. **A box that does not fit breaks before it when its parent wraps** (`f25dc8f3`,
   `parley/src/layout/line_break.rs`). The breaker allowed a break before an in-flow inline box
   only by the wrap mode of the text before it (lagged by an atom), so a box after `pre` text
   inside a `pre-wrap` span was appended past the line's end and the line went back to an
   earlier opportunity. Now the break is allowed when the text before the box wraps **or** the
   box's parent style wraps. That is wider than CSS Text 3 §5.1, which gives the opportunity to
   the nearest common ancestor of the two (`aa<span normal>[B]</span>` in a `nowrap` root has
   none between `aa` and `[B]`, and the fork's rule allows one); the fork's commit message and
   its code comment call the rule "nearest common ancestor", which is not what the code does
   (#2238). rinch matches Chrome anyway: a paragraph that holds an atomic inline and an element
   that does not wrap is broken where CSS breaks next to its boxes (`ifc::WrapSeams`), not where
   parley does, and without such an element the two rules agree. #1238 corpus `p704`
   (`[B2]q-refg` / `[I1]xyz`, Chrome 155), and with it `m420` and `p660`, known differences since
   before 0.12.

2. **fontconfig: a font is in every family it lists** (`0601dc37`,
   `fontique/src/backend/fontconfig.rs`, #2294; fork PR joeleaver/parley-fork#1). A font's
   second `FC_FAMILY` name was an alias of its first name's family, and `family_uncached`
   collected only fonts whose *first* name was the one asked for, so `'Arial Black'` was Arial
   Regular (or nothing) where a font lists "Arial, Arial Black". Every listed name is now a family
   of its own, and a family is the sorted fonts that list its name at any index
   (case-insensitively). A second name that is an RBIZ name ("Roboto Light") is now the faces that
   list it rather than the first name's whole family. Pinned by
   `crates/rinch-dom/tests/fontconfig_second_family_name_2294.rs` (a private `FONTCONFIG_FILE`).

3. **Blink's two synthetic-bold rules, and `Collection::system_family_match`** (`96699deb`,
   `fontique/src/font.rs`, `fontique/src/collection/mod.rs`, `fontique/src/backend/fontconfig.rs`;
   #2343, #2336; fork PR joeleaver/parley-fork#2). A static face registered with the
   collection (`register_fonts`, `load_fonts_from_paths`: rinch's app and test fonts, the
   counterpart of `@font-face`) is emboldened when the request is 600 or more and the face is
   under 600 (Blink's `CSSSegmentedFontFace`); a system face keeps the old rule, more than 200
   over the face (Blink's `FontCache::CreateFontPlatformData`). `FontInfo::is_registered` says
   which a face is. `system_family_match(name)` is fontconfig's answer for a family name as
   Chrome's Linux font manager asks it: the first family after `FcConfigSubstitute` and the names
   of `FcFontMatch`'s font. rinch judges it (`rinch_dom::fonts::system_substitute`); other
   backends return `None`. Pinned by fontique's own tests (the weight tables measured in Chrome
   156), `crates/rinch-dom/tests/fontconfig_metric_aliases_2336.rs` and
   `crates/rinch-dom/tests/faux_bold_threshold_2343.rs`.

The other parley 0.12 line-break differences are rinch's to fix, in how it hands text to parley
(`ifc::IfcText::wrap_spaces`, `ifc::SPACE_RUN_BREAKS`) and in the seams pass
(`ifc::pre_seams_needed`, `ifc::break_overflowing_spaces`). `wrap_spaces` hands parley a `nowrap`
or `pre` element's collapsible space as wrapping where Chrome has an opportunity before it, which
Chrome decides item by item from the content before the space (`ifc::IfcText::last_wraps`,
#2238): text or a box whose element wraps allows it and one whose element does not takes it
away, an element's end passes it on (and allows it where an element that does not wrap closes
into one that does), and an element's start allows it when that element wraps and takes it away
when it does not, even an empty one. So `aa<span normal>hello</span> ccc` in a `nowrap` root
breaks after `hello` with end padding, a border or a margin on the span, or a padded child at
its end, and so do `aa<span><span normal>hello</span></span> ccc` and `aa<span normal><span
nowrap>hello</span></span> ccc`; `…hello</span><span></span> ccc` and `…hello</span><span
nowrap> ccc</span>` do not (Chrome 156). An element's end inside a `display: contents` element
closes into the nearest ancestor with a box, text that does not wrap in a contents element whose
boxed ancestor wraps takes the opportunity away, and a `<wbr>` gives one that empty elements after
it do not take away (review of #2258); a `<wbr>` right before text that does not wrap still makes
no opportunity of its own. The seams and edge-box passes ask the same answer
(`ifc::UnhungSpaces::space_breaks`), and a paragraph with such a space is broken at its width
even when none of its text wraps.
