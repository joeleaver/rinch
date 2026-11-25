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

/// A transparent pixmap, its buffer reserved fallibly
/// ([`Vec::try_reserve_exact`]) rather than through [`Pixmap::new`], which
/// aborts when the allocation fails.
fn try_pixmap(width: u32, height: u32) -> Option<Pixmap> {
    let size = IntSize::from_wh(width, height)?;
    // tiny-skia's own row limit, checked before anything is reserved.
    let stride = width.checked_mul(4).filter(|s| *s <= i32::MAX as u32)? as usize;
    let len = stride.checked_mul(height as usize)?;
    let mut data = Vec::new();
    data.try_reserve_exact(len).ok()?;
    data.resize(len, 0);
    Pixmap::from_vec(data, size)
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

    /// An ordinary size is a transparent surface of that size.
    #[test]
    fn an_ordinary_surface_is_transparent() {
        let p = TinySkiaPainter::try_new(7, 3).expect("a 7 x 3 surface");
        assert_eq!((p.width(), p.height()), (7, 3));
        assert_eq!(p.pixels().len(), 7 * 3 * 4);
        assert!(p.pixels().iter().all(|b| *b == 0));
    }
}
