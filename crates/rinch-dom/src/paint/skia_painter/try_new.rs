//! [`TinySkiaPainter::try_new`]: a painter whose surface may not be had.
//!
//! [`TinySkiaPainter::new`] panics on a size tiny-skia refuses, and aborts
//! the process where the allocator cannot give the memory. That suits a
//! window's surface, whose size the platform gave. A surface sized by what a
//! document paints (the software drag ghost, which holds everything its
//! subtree draws, a `box-shadow` spread included; #3065) has to be able to
//! fail instead.

use tiny_skia::{IntSize, Pixmap};

use super::TinySkiaPainter;

impl TinySkiaPainter {
    /// [`Self::new`], or `None` where the surface cannot be had: a size
    /// tiny-skia refuses (a row of more than `i32::MAX / 4` pixels, or more
    /// bytes than it can address), or memory the allocator will not reserve.
    /// A zero width or height is 1, as in [`Self::new`].
    pub fn try_new(width: u32, height: u32) -> Option<Self> {
        let pixmap = try_pixmap(width.max(1), height.max(1))?;
        let mut painter = Self::new(1, 1);
        painter.pixmap = pixmap;
        Some(painter)
    }
}

/// A transparent pixmap, its buffer allocated fallibly ([`try_zeroed`])
/// rather than through [`Pixmap::new`], which aborts when the allocation
/// fails.
fn try_pixmap(width: u32, height: u32) -> Option<Pixmap> {
    let size = IntSize::from_wh(width, height)?;
    // tiny-skia's own row limit, checked before anything is allocated.
    let stride = width.checked_mul(4).filter(|s| *s <= i32::MAX as u32)? as usize;
    let len = stride.checked_mul(height as usize)?;
    Pixmap::from_vec(try_zeroed(len)?, size)
}

/// `len` zero bytes, or `None` where the allocator will not give them.
///
/// Asked of the allocator as zeroed memory ([`std::alloc::alloc_zeroed`]),
/// not reserved and then written. A large zeroed allocation is a fresh
/// mapping the kernel already gives as zeros, so allocating it commits none
/// of its pages, and a size it will not map is the allocator's null: `None`.
/// (A small one may come from the heap and be cleared there, which costs no
/// more than its size.) A buffer reserved with `Vec::try_reserve_exact` and
/// then filled with `resize(len, 0)` wrote every page at once, pages the
/// ghost may never paint.
///
/// What this does not do is make the pages safe to use. Each page becomes
/// resident when it is first written, so a ghost that paints its whole
/// pixmap (a shadow covering its reach) still commits all of it while it is
/// painted, and under Linux overcommit, where the allocation succeeded past
/// what the machine can back, that paint can still be OOM-killed rather than
/// fail. The bound on that is the pixmap's size (`ghost_pixmap_rect`: at most
/// twice the surface each way), not this function.
fn try_zeroed(len: usize) -> Option<Vec<u8>> {
    use std::alloc::{Layout, alloc_zeroed};
    if len == 0 {
        return Some(Vec::new());
    }
    let layout = Layout::array::<u8>(len).ok()?;
    // SAFETY: `layout` has a non-zero size (`len > 0`, checked above).
    let ptr = unsafe { alloc_zeroed(layout) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` was allocated just now by the global allocator with
    // `layout`, which is `u8`'s alignment and `len` bytes, the capacity
    // given here; all `len` bytes are initialised (zeroed), and nothing else
    // owns the allocation, so the `Vec` frees it with the same layout.
    Some(unsafe { Vec::from_raw_parts(ptr, len, len) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A size tiny-skia refuses is `None`, not a panic.
    #[test]
    fn a_surface_tiny_skia_refuses_is_none() {
        assert!(TinySkiaPainter::try_new(u32::MAX, 1).is_none());
        assert!(TinySkiaPainter::try_new(i32::MAX as u32 / 4 + 1, 1).is_none());
    }

    /// A size the allocator will not reserve is `None`, not an abort: a row
    /// tiny-skia accepts, by more rows than any address space holds.
    #[test]
    fn a_surface_the_allocator_refuses_is_none() {
        assert!(TinySkiaPainter::try_new(i32::MAX as u32 / 4, u32::MAX).is_none());
    }

    /// The buffer is had as zeroed memory, not written: allocating 512 MiB
    /// of it makes next to none of its own pages resident. Filled after a
    /// reservation, every page of it was.
    ///
    /// Counted with `mincore(2)` over the buffer's own pages, so what other
    /// tests in this process allocate meanwhile does not count; a page
    /// written afterwards is counted, which shows the count sees a page
    /// becoming resident at all.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_buffer_is_not_written_to_be_zeroed() {
        /// How many of `buf`'s pages are resident.
        fn resident_pages(buf: &[u8]) -> usize {
            // SAFETY: `sysconf` reads a constant; no memory is touched.
            let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
            // `mincore` wants a page-aligned start: the page holding the
            // first byte, through the page holding the last.
            let start = buf.as_ptr() as usize & !(page - 1);
            let len = buf.as_ptr() as usize + buf.len() - start;
            let mut vec = vec![0u8; len.div_ceil(page)];
            // SAFETY: `[start, start + len)` covers only pages `buf` lies in,
            // all of them mapped while `buf` lives, and `vec` has one byte
            // for each of those pages, which is what `mincore` writes.
            let rc = unsafe { libc::mincore(start as *mut libc::c_void, len, vec.as_mut_ptr()) };
            assert_eq!(rc, 0, "mincore: {}", std::io::Error::last_os_error());
            vec.iter().filter(|b| *b & 1 != 0).count()
        }
        const LEN: usize = 512 << 20;
        let mut buf = try_zeroed(LEN).expect("512 MiB of address space");
        assert_eq!(buf.len(), LEN);
        // At most the allocator's own header page(s) at the start, out of
        // 131,072 (4 KiB pages).
        let resident = resident_pages(&buf);
        assert!(resident <= 2, "{resident} pages became resident");
        // The count sees a page that is written.
        buf[LEN / 2] = 1;
        assert_eq!(resident_pages(&buf), resident + 1, "the written page");
        // Spot-read a few pages: zeros.
        buf[LEN / 2] = 0;
        assert!((0..LEN).step_by(LEN / 8).all(|i| buf[i] == 0));
    }

    /// An ordinary size is a transparent surface of that size.
    #[test]
    fn an_ordinary_surface_is_transparent() {
        let p = TinySkiaPainter::try_new(7, 3).expect("a 7 x 3 surface");
        assert_eq!((p.width(), p.height()), (7, 3));
        assert_eq!(p.pixels().len(), 7 * 3 * 4);
        assert!(p.pixels().iter().all(|b| *b == 0));
    }
}
