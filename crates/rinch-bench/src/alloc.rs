//! A global allocator that makes the measured region's instruction count
//! independent of the heap's history.
//!
//! # Why
//!
//! Callgrind counts every instruction, glibc's `malloc`/`free` included, and
//! what a `free` costs depends on the free lists it lands in: whether the chunk
//! can merge with a neighbour, which bin it goes to, whether the top chunk
//! trims. Those lists are shaped by everything the process did before the
//! measured call, and that is **not** repeatable run to run — the order of a
//! `HashMap`'s iteration (std seeds `RandomState` from the OS) decides the
//! order a setup allocates and frees in. Measured on `set_text_content`, the
//! same binary gave 5,343,220, 5,707,601 and 5,833,040 instructions on three
//! runs; every instruction of the difference was inside `malloc.c`
//! (`_int_malloc`, `_int_free_merge_chunk`, `unlink_chunk`, ...), and outside
//! it the two profiles differed by under 250 instructions.
//!
//! # What
//!
//! Between [`begin`] and [`end`] — exactly the measured operation — every
//! allocation is a bump of a pointer into one region reserved beforehand, and
//! every free is a no-op (the memory is simply never reused). Both cost the
//! same handful of instructions whatever came before, so the count is the
//! rinch code's own. Outside that window the system allocator serves
//! everything, as usual; a block the bump arena handed out is never passed to
//! it.
//!
//! What this gives up: the benchmark counts an allocation as a few instructions
//! rather than glibc's 50–150, so a change that only adds allocations moves
//! the count less than it moves a real frame. It still moves it, and the code
//! that builds what is allocated is counted in full.
//!
//! Two limits. Only Rust allocations go through this allocator: C code that
//! calls `malloc` directly (fontconfig, say) still goes through glibc, so its
//! cost is not made repeatable. Keep such calls out of the measured operation.
//! And a pointer from the arena must never reach libc's `free`, which would
//! crash on a block glibc never handed out. Rust never hands one over, and
//! nothing in the measured paths passes Rust-allocated memory to C for C to
//! free.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Bytes reserved for one measured operation. Virtual: pages nothing touches
/// are never committed, and the kernel hands the region over zeroed.
const ARENA_BYTES: usize = 1 << 30;

static ON: AtomicBool = AtomicBool::new(false);
static EXHAUSTED: AtomicBool = AtomicBool::new(false);
static BASE: AtomicUsize = AtomicUsize::new(0);
static CURSOR: AtomicUsize = AtomicUsize::new(0);
static END: AtomicUsize = AtomicUsize::new(0);

/// The benchmark binary's `#[global_allocator]`.
pub struct BenchAlloc;

fn in_arena(ptr: *mut u8) -> bool {
    let p = ptr as usize;
    let base = BASE.load(Ordering::Relaxed);
    base != 0 && p >= base && p < END.load(Ordering::Relaxed)
}

fn bump(layout: Layout) -> *mut u8 {
    let align = layout.align();
    let size = layout.size().max(1);
    let mut cur = CURSOR.load(Ordering::Relaxed);
    loop {
        let Some(start) = cur.checked_add(align - 1).map(|a| a & !(align - 1)) else {
            EXHAUSTED.store(true, Ordering::Relaxed);
            return std::ptr::null_mut();
        };
        let next = match start.checked_add(size) {
            Some(n) if n <= END.load(Ordering::Relaxed) => n,
            _ => {
                EXHAUSTED.store(true, Ordering::Relaxed);
                return std::ptr::null_mut();
            }
        };
        match CURSOR.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return start as *mut u8,
            Err(seen) => cur = seen,
        }
    }
}

unsafe impl GlobalAlloc for BenchAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ON.load(Ordering::Relaxed) {
            let p = bump(layout);
            if !p.is_null() {
                return p;
            }
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ON.load(Ordering::Relaxed) {
            // The arena is never reused, and the kernel zeroed it.
            let p = bump(layout);
            if !p.is_null() {
                return p;
            }
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if in_arena(ptr) || ON.load(Ordering::Relaxed) {
            // Never reused: an arena block, or a system block freed inside the
            // measured window (leaked, so its cost does not depend on the
            // free lists).
            return;
        }
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if !in_arena(ptr) && !ON.load(Ordering::Relaxed) {
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        let Ok(new_layout) = Layout::from_size_align(new_size, layout.align()) else {
            return std::ptr::null_mut();
        };
        let new = unsafe { self.alloc(new_layout) };
        if !new.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
        }
        new
    }
}

/// Reserve the arena. Called from every setup, outside the measured region:
/// the reservation is a system allocation and must not be counted.
pub fn reserve() {
    if BASE.load(Ordering::Relaxed) != 0 {
        return;
    }
    let layout = Layout::from_size_align(ARENA_BYTES, 4096).expect("arena layout");
    // SAFETY: a non-zero-sized layout.
    let base = unsafe { System.alloc_zeroed(layout) } as usize;
    assert!(
        base != 0,
        "could not reserve the {ARENA_BYTES}-byte bench arena"
    );
    CURSOR.store(base, Ordering::Relaxed);
    END.store(base + ARENA_BYTES, Ordering::Relaxed);
    BASE.store(base, Ordering::Relaxed);
}

/// Start the measured region's allocation discipline.
#[inline(always)]
pub fn begin() {
    assert!(
        BASE.load(Ordering::Relaxed) != 0,
        "bench arena not reserved: call alloc::reserve() in the setup"
    );
    ON.store(true, Ordering::Relaxed);
}

/// End it. Panics if the arena ran out, since the allocations past that point
/// came from the system allocator and the count is no longer repeatable.
#[inline(always)]
pub fn end() {
    ON.store(false, Ordering::Relaxed);
    assert!(
        !EXHAUSTED.load(Ordering::Relaxed),
        "the measured operation outgrew the {ARENA_BYTES}-byte bench arena"
    );
}
