# Dependencies: Taffy, Parley, skrifa, Stylo (pins and their traps)

Read this page when your task touches: a Taffy, Parley, skrifa, Stylo, wgpu or Vello bump, text shaping/line breaking, white space, tabs, NBSP, justify.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

## Project Overview

Rinch is a lightweight cross-platform GUI library for Rust, built on rinch-dom, Taffy, Parley, and dual rendering backends (Vello for GPU, tiny-skia for software). The goal is to provide a reactive GUI framework using HTML/CSS for layout.

**Key dependencies:**
- **rinch-dom** - HTML/CSS DOM implementation (Taffy for layout, Parley for text, Painter trait for rendering). Taffy is pinned at **0.14** in one place, `crates/rinch-dom/Cargo.toml` (its `[dependencies]` and `[dev-dependencies]` lines must agree, **features included**: rinch builds Taffy without the five default features it never reads — `content_size`, `calc`, `float_layout`, `flexbox_balance`, `detailed_layout_info` — which cost about 4% of a structural edit's instructions whether read or not (#1260), and a dev-dependency left on the defaults would unify them back into every test build; the vendored `crates/stylo-taffy` that used to pin it too was unused and was deleted with the 0.14 bump, #1236). 0.9 could not resolve a percentage `min-height`/`max-height` against a block containing block (its block algorithm hard-coded that basis as indefinite), so `min-height: 100%` silently collapsed to the content height. 0.12 numbered grid lines as `i16` and **panicked** on a grid past 32,767 lines (40,000 auto-placed rows, stacked `span`s, #1210) and on `repeat(auto-fill, 0px)` or `minmax(0px, 1fr)` at any width (#1231); 0.14 clamps each grid axis at **10,000 tracks** instead (css-grid-1 §7.2.4), which is not Chrome's geometry: every item past the 10,000th track lands in the last one and they overlap, so a grid of 10,001–32,767 lines that 0.12 laid out correctly now overlaps too (`crates/rinch-dom/tests/grid_line_overflow_tests.rs`). Both measure closures (`layout_engine.rs`, `ifc.rs`) return a `LayoutOutput` through `taffy::compute_leaf_layout` with the same `0.0` calc resolver 0.12 used, and set its first baseline (`ifc::with_first_baseline`, #1013): an IFC root's or text leaf's first Parley line's baseline plus the box's top padding and border, since Taffy measures from the border-box top. `ifc_measure_cache` keeps the baseline beside each size (`IfcMeasure`), and Taffy's block algorithm hands a block item its first in-flow child's, so flex/grid `align-items: baseline` lines items up by their first lines (`tests/flex_baseline_tests.rs`). It used to synthesize every item's from its bottom border edge. A **form control** still reports none and is aligned by its bottom edge (an `<input>` beside a 32px line sits 6px higher than Chrome's, #1273), and so is an image, which is right. And an item whose first line holds an atomic inline (an `inline-block`, a `Button`) is aligned by that box's **bottom**, because rinch's IFC puts such a line's baseline there (#663): beside a 32px line Chrome puts the other item at 0 and rinch at 18 (9 before #1013), pinned by `an_items_first_line_holding_an_inline_block_is_aligned_by_its_bottom_issue_663`. `min_size`/`max_size` are `LengthPercentageAuto` in 0.14 (`DimensionValue::to_taffy_lpa`), and `size`/`flex_basis` carry the intrinsic keywords, which rinch hands over since #691 (see **Common Issues**). The bump itself moved layout in a few shapes, every one to Chrome 153's answer (`crates/rinch-dom/tests/taffy_014_layout_changes_tests.rs`): flex auto margins absorb free space **before** `justify-content` (0.12 justified first and pushed an auto-margin item off the end), a `min-height` parent keeps its last child's bottom margin inside it, and an empty scroll container's margins collapse through it — so a horizontal `Tabs` nested in a vertical `Tabs` now gets an inner panel 44px wide where 0.12 gave 85 (Chrome: 42.9). An editor table of more than 10,000 rows (which #1209 places by auto-placement past 9,999 tracks) now overlaps its rows from the 10,000th on, on desktop.
- **parley** - Text shaping and line breaking, at the **published `0.11.1`**. It was a git `rev`
  on an unmerged 22-commit "Floats WIP" branch off `v0.7.0` until #659; upstream landed that work
  in 0.9.0, so nothing was lost by moving to the release line. Two things about the dependency line
  are load-bearing and neither is enforced by the compiler:
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
  - **An overflowing NBSP is hung like a space** (`is_space_or_nbsp` in the breaker's hang
    branch) and the line committed right after it, where UAX #14 class GL allows no break:
    `x yyx&nbsp;Wkxxpq` at 40px broke as `x yyx&nbsp;` / `Wkxxpq`. `ifc::unglue` breaks such a
    line again inside `break_lines_hanging_spaces`' pass (and `shape_text_leaf`, a flex item's
    own text), where Chrome 153 does: at the last opportunity before the NBSP; right before it
    after an atomic inline or a hyphen (LB12a), or for `overflow-wrap` with nothing else; and
    with no opportunity before it, the glued word overflows to the next one (#1218). That last
    one is found by a galloping search over the line's units, a few breaks of the line each
    (`ifc_unglue_rebreaks`, 15 for a 1600-word chain) — one re-break per glued word was
    quadratic in an NBSP chain (review of #1257).
    `crates/rinch-dom/tests/nbsp_glue_line_break_tests.rs` (Chrome 153) and
    `nbsp_glue_oracle_tests.rs` (a greedy token oracle) are the pins;
    `perf_regression_scenarios::an_nbsp_glued_chain_*` pins the cost. Upstream fixed it
    after 0.11.1 (linebender/parley#762, merged 2026-09-07, unreleased as of 2026-10-01:
    `whitespace_hangs` there hangs no NBSP); `unglue` can go when rinch takes that release.
  - **Preserved spaces do not hang in 0.11.1.** At a width a `pre-wrap` space overflows, parley
    hangs the first such space and commits the line, so the rest of the spaces (or, when nothing
    is left, an empty line) make one more line. `ifc::break_lines_hanging_spaces` is how an IFC
    breaks its lines, and puts the spaces back on the line they hang from; upstream's rework
    (#790, #785) is unreleased. Spaces and tabs hang, NBSP does not (it glues, and after a hung run starts the next line).
    A `break-spaces` space or tab in an IFC never hangs (#1043, `WhiteSpaceValue::BreakSpaces`; it read as
    `pre-wrap` before; and, since #1672, a flex or grid item's own text too): the same pass breaks a line parley ended past the box in such spaces
    (`ifc::break_overflowing_spaces`) after the last one that fits, else before the word they follow,
    else after their first space, committed by length; the line is aligned with them as content and
    `measured_width` counts them (Chrome 155, `tests/white_space_break_spaces_1043_tests.rs`). It is **one**
    extra break of the paragraph, line by line, however many lines it fixes: the first version
    restarted from line 0 per fixed line, which is quadratic (0.7–1.0 s for 1,600 double-spaced
    lines). `ifc_hang_passes` counts those breaks, at most one per layout, and
    `perf_regression_scenarios::a_double_spaced_pre_wrap_paragraph_hangs_in_one_pass` pins it;
    `ifc::hang_differential_tests` holds the old loop as the oracle. Likewise an IFC's measure is `InlineLayout::measured_width`, not
    `Layout::width`: the latter drops a preserved trailing space that Chrome counts, and a box
    sized without it wraps the space onto a line of its own (the editor's list bullet dropping a
    line after "text "). Only when the IFC holds preserved text (`pre`, `pre-wrap`, `break-spaces`, a
    contenteditable root): collapsible spaces, `pre-line`'s included, are gone at a line's end
    before parley sees them.
    `crates/rinch-dom/tests/list_item_trailing_space_tests.rs` is the pin.
  - **parley's `align` hangs every line's trailing white space** (`free = width - advance +
    trailing_whitespace`), NBSP included. CSS hangs a preserved space unconditionally only at a
    soft wrap; at the end of the paragraph or before a forced break only the part that does not
    fit hangs, and an NBSP never hangs. So a line aligned `right`, `center` or `end` that ends in
    such white space is sent through the hang pass and its alignment width narrowed by what
    counts (`ifc::align_unhung_trailing` with `count_trailing`; the cheap trigger is
    `any_counted_trailing_line`), #1042: `abc` and three `pre-wrap` spaces right-aligned in 200px
    start at 158.58, as in Chrome 155 (they started at 172.08), and a shrink-to-fit box no
    longer draws its line past its own edge. A start-aligned paragraph is not re-broken for it.
    The width is narrowed by what parley adds back — the trailing white space of the line's
    **last shaped run** only (a tab, or a span with its own font size or spacing, is a run of its
    own) — less what CSS hangs. Such a paragraph pays one extra break of the whole paragraph per
    layout (`ifc_hang_passes`; `perf_regression_scenarios::a_right_aligned_paragraph_ending_in_spaces_pays_one_pass`).
    Not done: a right-to-left line, by parley's inferred direction or CSS `direction: rtl`
    (parley offsets it by its trailing white space, so narrowing would move it the wrong way;
    #1666), a flex or grid item's own text (a text leaf, `shape_text_leaf`). A `nowrap`/`pre`
    root with no wrapping text gets the same narrowing on its unconstrained lines since #1237
    (`ifc::break_unwrapped_aligned`).
    `crates/rinch-dom/tests/text_align_trailing_spaces_1042_tests.rs` is the pin.
  - **parley counts a justified line's opportunities as it breaks it** (`num_spaces`, not
    public): every space or NBSP that fits, less one when the line ends in one. So two or more
    spaces hanging at a wrap took shares of the free space, and a line parley ended by hanging
    the one space that did not fit lost a word gap (the line was not flush; on `main` that
    second case was any justified line whose last word ended within a space of the edge, in
    `normal` and `pre-wrap` alike). Chrome 155's opportunities are the spaces and NBSPs of the
    line's content, hanging spaces excluded, less the last when the content ends in one. With
    `text-align: justify` in a left-to-right block, a paragraph with such a line
    (`any_miscounted_justified_line`) takes the hang pass, and `ifc::justify_line` commits the
    line again **by length** — parley's count is then every space on it less one, never fewer
    than CSS's — and sets its alignment width so each opportunity gets `free / opportunities`
    (#1056). Most multi-line justified paragraphs pay that pass, one extra break per layout
    built, since some line's last word usually ends within a space of the edge (35 of 41 widths
    of a four-sentence paragraph; `perf_regression_scenarios::a_justified_paragraph_whose_word_ends_at_the_edge_pays_one_pass`).
    The surplus shares land on the spaces after the opportunities: hanging spaces
    (past the edge; a caret between two of them is right of Chrome's) or, when the content
    ends in an NBSP before hanging spaces, that NBSP (a caret after it is one share right of
    Chrome's). `LineUnit::run` is the shaped run's identity (its `RunMetrics` address), not
    `Run::index`, which is a place on a line: #1042's trailing white space read hanging spaces
    of one run that the table's layout had on two lines as two runs.
    `crates/rinch-dom/tests/justify_hanging_spaces_1056_tests.rs` is the pin.
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
    DOM↔flat correspondence. Since parley counts a trailing NBSP in `trailing_whitespace`,
    `measured_width` keeps it (`width_keeping_nbsp`), whatever else the IFC holds.
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
    span fragments, text queries). parley 0.11.1 allows a break after every
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
- **skrifa** - read directly by rinch-dom only to size a text control from its font's OS/2 and `head` tables (#1177). Pinned at **0.44**, the version parley 0.11.1 depends on — and Vello 0.11 too (`vello`/`vello_encoding` ^0.44; the Vello 0.7 rinch had before #1955 built a second skrifa, 0.40). parley does not re-export it, so a parley (or Vello) bump that moves skrifa silently builds a second copy unless this pin moves with it. (swash 0.2, the software painter's glyph rasteriser, still brings its own skrifa 0.37.)
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
- **tiny-skia** - 2D software rendering (default mode, no GPU required)
- **softbuffer** - Software window presentation (default mode)
- **winit** - Cross-platform windowing and input. **0.31.0-beta.4 from `joeleaver/winit-fork` (branch `rinch-patch-beta4`) at a pinned rev**, a workspace dependency, not a `[patch]` (#1962): upstream's tag plus the `wlr-layer-shell` window role. Apps copy no patch lines. Wayland drag and drop is upstream's data transfer API (`shell::os_file_drag`). The fork takes crates.io `dpi`, so there is one `dpi` in the graph (muda's too). Detail: `app-and-windows.md` "winit Fork".
- **muda** - Native menu support

**Design philosophy:** Declarative UI with fine-grained reactive programming. The `rsx!` macro generates DOM construction code with Effects for reactive updates. Only changed DOM nodes are updated - no full re-renders.
