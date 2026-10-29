//! Instruction counts for rinch's hot paths, under Callgrind (Gungraun).
//!
//! Each benchmark runs its `setup` (not counted) and then one operation
//! (counted); the scenarios and why each is shaped as it is live in
//! `src/lib.rs`. The operation returns the fixture so that dropping the
//! document happens outside the measured function.
//!
//! ```text
//! cargo install gungraun-runner --version <the gungraun version in Cargo.lock>
//! cargo bench -p rinch-bench
//! ```
//!
//! The `Perf` CI workflow runs this on a PR's merge commit and on that commit's
//! first parent, and compares the `Ir` (instructions executed) of every
//! benchmark;
//! `docs/src/guide/performance.md#ci-regression-job`.

use std::hint::black_box;

use gungraun::prelude::*;
use rinch::app::RinchApp;
use rinch_bench::alloc::BenchAlloc;
use rinch_bench::*;
use rinch_core::reactive::Signal;

/// Bump-allocates inside the measured region so glibc's heap history cannot
/// move the count (`rinch_bench::alloc`).
#[global_allocator]
static ALLOC: BenchAlloc = BenchAlloc;

/// One `#[library_benchmark]` per list operation, all over the same fixture.
macro_rules! list_bench {
    ($name:ident, $setup:ident, $op:ident) => {
        #[library_benchmark]
        #[bench::list_500(setup = $setup)]
        fn $name(f: ListFixture) -> ListFixture {
            black_box(measure(black_box(f), $op))
        }
    };
}

list_bench!(hover, setup_hover, op_hover);
list_bench!(class_toggle, setup_class_toggle, op_class_toggle);
list_bench!(append_row, setup_append_row, op_append_row);
list_bench!(remove_row, setup_remove_row, op_remove_row);
list_bench!(resize_1px, setup_resize, op_resize);
list_bench!(set_text_content, setup_set_text, op_set_text);
list_bench!(
    flex_label_hover,
    setup_flex_label_hover,
    op_flex_label_hover
);
list_bench!(
    animated_list_color,
    setup_animated_list_color,
    op_animated_list_color
);

#[library_benchmark]
#[bench::list_500_closed(setup = setup_drawer_toggle)]
fn drawer_toggle(f: DrawerFixture) -> DrawerFixture {
    black_box(measure(black_box(f), op_drawer_toggle))
}

#[library_benchmark]
#[bench::list_500(setup = setup_inset_move)]
fn inset_move(f: InsetFixture) -> InsetFixture {
    black_box(measure(black_box(f), op_inset_move))
}

// The scenarios that must cost what they cost on a tree with no out-of-flow
// correction at all: a box Taffy already resolves, a box whose containing
// block needs no size check, a scroller that is between no box and its
// containing block. The two that pay for the correction itself — a relayout
// of ancestor-resolved boxes, a scroll that carries boxes it is not the
// containing block of — are pinned as exact counters in
// `rinch-dom/tests/perf_regression_scenarios.rs` (`abs_boxes_visited`).
#[library_benchmark]
#[bench::direct_500(setup = setup_badges_direct)]
#[bench::icb_500(setup = setup_badges_icb)]
fn abs_badge_relayout(f: BadgeFixture) -> BadgeFixture {
    black_box(measure(black_box(f), op_badge_relayout))
}

#[library_benchmark]
#[bench::direct_500(setup = setup_badges_direct)]
#[bench::ancestor_500(setup = setup_badges_ancestor)]
fn abs_badge_scroll(f: BadgeFixture) -> BadgeFixture {
    black_box(measure(black_box(f), op_badge_scroll))
}

#[library_benchmark]
#[bench::text_page_warm(setup = setup_full_paint)]
fn full_paint(f: PaintFixture) -> PaintFixture {
    black_box(measure(black_box(f), op_full_paint))
}

#[library_benchmark]
#[bench::text_page(setup = setup_vello_paint)]
#[bench::split_surface_140_boxes(setup = setup_vello_paint_split)]
#[bench::text_page_unsliced(setup = setup_vello_paint_unsliced)]
#[bench::split_140_unsliced(setup = setup_vello_paint_split_unsliced)]
fn vello_paint(f: VelloPaintFixture) -> VelloPaintFixture {
    black_box(measure(black_box(f), op_vello_paint))
}

#[library_benchmark]
#[bench::fields_40(setup = setup_shadow_paint)]
fn shadow_paint(f: PaintFixture) -> PaintFixture {
    black_box(measure(black_box(f), op_shadow_paint))
}

#[library_benchmark]
#[bench::paragraphs_40(setup = setup_text_shadow_paint)]
#[bench::paragraphs_40_cold(setup = setup_text_shadow_paint_cold)]
#[bench::spans_30x12(setup = setup_text_shadow_spans)]
#[bench::coloured_30x12(setup = setup_text_shadow_coloured)]
#[bench::count_30(setup = setup_text_shadow_count)]
#[bench::distinct_lists_200(setup = setup_text_shadow_distinct)]
fn text_shadow_paint(f: PaintFixture) -> PaintFixture {
    black_box(measure(black_box(f), op_text_shadow_paint))
}

#[library_benchmark]
#[bench::warm_x50(setup = setup_pointer_move_warm)]
fn pointer_move_warm(app: RinchApp) -> RinchApp {
    black_box(measure(black_box(app), op_pointer_move_warm))
}

#[library_benchmark]
#[bench::cold(setup = setup_pointer_move_cold)]
fn pointer_move_cold(app: RinchApp) -> RinchApp {
    black_box(measure(black_box(app), op_pointer_move_cold))
}

#[library_benchmark]
#[bench::partial_repaint(setup = setup_hover_frame)]
fn hover_frame(app: RinchApp) -> RinchApp {
    black_box(measure(black_box(app), op_hover_frame))
}

#[library_benchmark]
#[bench::reverse_200(setup = setup_keyed_reorder)]
fn keyed_for(f: ShellFixture<Signal<Vec<u32>>>) -> ShellFixture<Signal<Vec<u32>>> {
    black_box(measure(black_box(f), op_keyed_reorder))
}

#[library_benchmark]
#[bench::selection_40(setup = setup_memo_selection)]
fn memo_flush(f: ShellFixture<Signal<usize>>) -> ShellFixture<Signal<usize>> {
    black_box(measure(black_box(f), op_memo_selection))
}

library_benchmark_group!(
    name = dom,
    benchmarks = [
        hover,
        class_toggle,
        append_row,
        remove_row,
        resize_1px,
        set_text_content,
        flex_label_hover,
        animated_list_color,
        drawer_toggle,
        inset_move,
        abs_badge_relayout,
        abs_badge_scroll,
        full_paint,
        vello_paint,
        shadow_paint,
        text_shadow_paint
    ]
);

#[library_benchmark]
#[bench::paragraphs_2000(setup = setup_collab_keystroke)]
fn collab_keystroke(f: CollabFixture) -> CollabFixture {
    black_box(measure(black_box(f), op_collab_keystroke))
}

// The same keystroke in a table of 100 rows and of 1000: equal counts mean a
// keystroke's cost does not grow with the table (#1241).
#[library_benchmark]
#[bench::rows_100(args = (100), setup = setup_collab_table_keystroke)]
#[bench::rows_1000(args = (1000), setup = setup_collab_table_keystroke)]
fn collab_table_keystroke(f: CollabFixture) -> CollabFixture {
    black_box(measure(black_box(f), op_collab_keystroke))
}

// Typing over a two-character selection inside one cell of a table of 100
// rows and of 1000, and at a caret there: the check for a table the edit left
// ragged (#1847) reads the range's start, resolved once with the inherited
// marks, and nothing of the table (its first round walked it: 1.0 ms at 1,024
// rows).
#[library_benchmark]
#[bench::rows_100(args = ((100, 2)), setup = setup_range_type_in_table)]
#[bench::rows_1000(args = ((1000, 2)), setup = setup_range_type_in_table)]
#[bench::caret_rows_100(args = ((100, 0)), setup = setup_range_type_in_table)]
#[bench::caret_rows_1000(args = ((1000, 0)), setup = setup_range_type_in_table)]
fn range_type_in_table(f: EditorFixture) -> EditorFixture {
    black_box(measure(black_box(f), op_range_type_in_table))
}

// Undoing a keystroke in a cell of a table of 100 and of 1000 rows, alone and
// after a peer's keystroke in another cell: the check for a table the undo
// left ragged (#1937) reads no table.
#[library_benchmark]
#[bench::rows_100(args = ((100, false)), setup = setup_undo_in_table)]
#[bench::rows_1000(args = ((1000, false)), setup = setup_undo_in_table)]
#[bench::after_peer_rows_100(args = ((100, true)), setup = setup_undo_in_table)]
#[bench::after_peer_rows_1000(args = ((1000, true)), setup = setup_undo_in_table)]
fn undo_in_table(f: EditorFixture) -> EditorFixture {
    black_box(measure(black_box(f), op_undo))
}

// Typing over a range from one paragraph of a cell into the next, in a table
// of 100 and of 1000 rows: the range is inside one cell, so the check for a
// table the edit left ragged (#1847) reads no table.
#[library_benchmark]
#[bench::rows_100(args = ((100, 0)), setup = setup_cells_in_table)]
#[bench::rows_1000(args = ((1000, 0)), setup = setup_cells_in_table)]
fn range_across_paragraphs_in_cell(f: EditorFixture) -> EditorFixture {
    black_box(measure(black_box(f), op_range_type_in_table))
}

// Whether bold is active over a two-cell selection, in a table of 100 and of
// 1000 rows: equal counts mean asking costs in the selected cells, not the
// table (`tables::selected_cells`: the table's map is kept per table node, and
// in the same document the cells are found without a resolve). The fixture's
// setup asks once, as the toolbar's first mark does.
#[library_benchmark]
#[bench::rows_100(args = ((100, 1)), setup = setup_cells_in_table)]
#[bench::rows_1000(args = ((1000, 1)), setup = setup_cells_in_table)]
fn cell_selection_mark_active(f: EditorFixture) -> EditorFixture {
    black_box(measure(black_box(f), op_bold_active))
}

// `toggleBold` over a two-cell selection, in a table of 100 and of 1000 rows.
#[library_benchmark]
#[bench::rows_100(args = ((100, 1)), setup = setup_cells_in_table)]
#[bench::rows_1000(args = ((1000, 1)), setup = setup_cells_in_table)]
fn cell_selection_toggle_bold(f: EditorFixture) -> EditorFixture {
    black_box(measure(black_box(f), op_toggle_bold))
}

// A peer's keystroke in one cell, integrated, at 100 and 1000 rows (#1241).
#[library_benchmark]
#[bench::rows_100(args = (100), setup = setup_collab_table_integrate)]
#[bench::rows_1000(args = (1000), setup = setup_collab_table_integrate)]
fn collab_table_integrate(f: CollabIntegrateFixture) -> CollabIntegrateFixture {
    black_box(measure(black_box(f), op_collab_integrate))
}

// A local keystroke in a table cell right after integrating a peer's (#1241).
#[library_benchmark]
#[bench::rows_100(args = (100), setup = setup_collab_table_keystroke_after_remote)]
#[bench::rows_1000(args = (1000), setup = setup_collab_table_keystroke_after_remote)]
fn collab_table_keystroke_after_remote(f: CollabFixture) -> CollabFixture {
    black_box(measure(black_box(f), op_collab_keystroke))
}

#[library_benchmark]
#[bench::rows_1000(setup = setup_branch_hide)]
fn branch_hide(f: ShellFixture<Signal<bool>>) -> ShellFixture<Signal<bool>> {
    black_box(measure(black_box(f), op_branch_hide))
}

library_benchmark_group!(
    name = shell,
    benchmarks = [
        pointer_move_warm,
        pointer_move_cold,
        hover_frame,
        keyed_for,
        memo_flush,
        branch_hide
    ]
);

library_benchmark_group!(
    name = editor,
    benchmarks = [
        collab_keystroke,
        collab_table_keystroke,
        range_type_in_table,
        undo_in_table,
        range_across_paragraphs_in_cell,
        cell_selection_mark_active,
        cell_selection_toggle_bold,
        collab_table_integrate,
        collab_table_keystroke_after_remote
    ]
);

main!(library_benchmark_groups = dom, shell, editor);
