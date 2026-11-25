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
/// mapping the kernel already gives as zeros, so nothing is written to it
/// here, and a size it will not map is the allocator's null: `None`. A
/// buffer reserved with `Vec::try_reserve_exact` and then filled with
/// `resize(len, 0)` wrote every page; with Linux overcommit the reservation
/// succeeds past what the machine has, and the fill is what the OOM killer
/// answers, killing the process rather than failing.
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

    /// The buffer is had as zeroed memory, not written: 512 MiB of it adds
    /// next to nothing to what the process holds resident. Filled after a
    /// reservation, every page of it was resident, and past what the machine
    /// has that fill is an OOM kill rather than a `None`.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_buffer_is_not_written_to_be_zeroed() {
        /// Resident memory, bytes (`/proc/self/status`'s `VmRSS`, in kB).
        fn resident() -> usize {
            let status = std::fs::read_to_string("/proc/self/status").expect("status");
            let line = status
                .lines()
                .find(|l| l.starts_with("VmRSS:"))
                .expect("VmRSS");
            let kb: usize = line.split_whitespace().nth(1).unwrap().parse().unwrap();
            kb * 1024
        }
        const LEN: usize = 512 << 20;
        let before = resident();
        let buf = try_zeroed(LEN).expect("512 MiB of address space");
        let grew = resident().saturating_sub(before);
        assert_eq!(buf.len(), LEN);
        // Other tests in this process allocate meanwhile: well under LEN.
        assert!(grew < LEN / 4, "{grew} bytes became resident");
        // Spot-read a few pages: zeros (reading maps the zero page).
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
