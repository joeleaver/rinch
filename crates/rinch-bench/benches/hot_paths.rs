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

#[library_benchmark]
#[bench::text_page_warm(setup = setup_full_paint)]
fn full_paint(f: PaintFixture) -> PaintFixture {
    black_box(measure(black_box(f), op_full_paint))
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
        full_paint
    ]
);

library_benchmark_group!(
    name = shell,
    benchmarks = [
        pointer_move_warm,
        pointer_move_cold,
        hover_frame,
        keyed_for,
        memo_flush
    ]
);

main!(library_benchmark_groups = dom, shell);
