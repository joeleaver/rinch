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
/// of its pages beyond the one the allocator writes its own header into (the
/// first; a whole huge page where transparent huge pages back the mapping),
/// and a size it will not map is the allocator's null: `None`.
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
    /// of it makes none of its pages resident past the allocator's header.
    /// Filled after a reservation, every page of it was.
    ///
    /// Counted with `mincore(2)` over the buffer's own pages, so what other
    /// tests in this process allocate meanwhile does not count; a page
    /// written afterwards is counted, which shows the count sees a page
    /// becoming resident at all.
    ///
    /// Transparent huge pages are turned off for the buffer first
    /// (`MADV_NOHUGEPAGE`): where they back it (`enabled` set to `always`, the
    /// Fedora/RHEL default, or glibc asked to advise them with
    /// `GLIBC_TUNABLES=glibc.malloc.hugetlb=1`), one write faults in a whole
    /// huge page, 512 resident pages at 2 MiB, and the written page was
    /// counted 512 times. The advice comes after the allocator wrote its
    /// header, which may already have faulted in the huge page holding it,
    /// so the count leaves out the buffer's head, up to the first huge-page
    /// boundary at or past its start: no header lies past that. The buffer
    /// is at least four huge pages long, so the body is most of it wherever
    /// it starts, also where a huge page is 512 MiB (aarch64 with 64 KiB
    /// pages).
    ///
    /// Not covered: `GLIBC_TUNABLES=glibc.malloc.hugetlb=2` with hugetlbfs
    /// pages reserved maps the buffer with `MAP_HUGETLB`, which the advice
    /// does not change; there the one write counts as a whole huge page.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_buffer_is_not_written_to_be_zeroed() {
        /// Where the kernel has transparent huge pages.
        const THP: &str = "/sys/kernel/mm/transparent_hugepage";
        /// The size of a page.
        fn page_size() -> usize {
            // SAFETY: `sysconf` reads a constant; no memory is touched.
            unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
        }
        /// The size of a transparent huge page, or `None` where the kernel
        /// has none.
        fn huge_page_size() -> Option<usize> {
            let s = std::fs::read_to_string(format!("{THP}/hpage_pmd_size")).ok()?;
            Some(s.trim().parse().expect("hpage_pmd_size is a number"))
        }
        /// How many pages of `[start, end)` are resident; `start` is
        /// page-aligned and the range lies in a live mapping.
        fn resident_pages(start: usize, end: usize) -> usize {
            let len = end - start;
            let mut vec = vec![0u8; len.div_ceil(page_size())];
            // SAFETY: the caller's range is mapped and page-aligned at its
            // start, and `vec` has one byte for each of its pages, which is
            // what `mincore` writes.
            let rc = unsafe { libc::mincore(start as *mut libc::c_void, len, vec.as_mut_ptr()) };
            assert_eq!(rc, 0, "mincore: {}", std::io::Error::last_os_error());
            vec.iter().filter(|b| *b & 1 != 0).count()
        }
        let page = page_size();
        let huge = huge_page_size().unwrap_or(page);
        // 512 MiB, or four huge pages where those are larger.
        let len: usize = (512 << 20).max(4 * huge);
        let mut buf = try_zeroed(len).expect("the buffer's address space");
        assert_eq!(buf.len(), len);
        let first = buf.as_ptr() as usize;
        let end = first + len;
        // The pages the buffer lies in: the one holding its first byte,
        // through the one holding its last.
        let start = first & !(page - 1);
        // SAFETY: `[start, end)` covers only pages the buffer lies in, all
        // mapped while it lives; the advice changes how later faults in
        // them are served, not their contents.
        let rc = unsafe {
            libc::madvise(
                start as *mut libc::c_void,
                end - start,
                libc::MADV_NOHUGEPAGE,
            )
        };
        // A kernel built without transparent huge pages refuses the advice;
        // it has nothing to turn off.
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            assert!(
                err.raw_os_error() == Some(libc::EINVAL) && !std::path::Path::new(THP).exists(),
                "madvise(MADV_NOHUGEPAGE): {err}"
            );
        }
        // Past the head: from the first huge-page boundary at or past the
        // buffer's start (its first page boundary, without huge pages).
        let body = first.next_multiple_of(huge);
        assert!(
            body < first + len / 2,
            "a huge page of {huge} bytes leaves no body"
        );
        let resident = resident_pages(body, end);
        assert_eq!(resident, 0, "pages past the head became resident");
        // The count sees a page that is written, and only that page.
        buf[len / 2] = 1;
        assert_eq!(resident_pages(body, end), 1, "the written page");
        // Spot-read a few pages: zeros.
        buf[len / 2] = 0;
        assert!((0..len).step_by(len / 8).all(|i| buf[i] == 0));
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
