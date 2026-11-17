//! Metadata a raster file carries beside its pixels, read from the encoded
//! bytes at decode (`image_cache::decode_raster`).
//!
//! **EXIF orientation** (#2785). Browsers draw an image as its EXIF
//! orientation says (`image-orientation: from-image`, the initial value), and
//! Chrome reads the orientation only where the EXIF comes before the image
//! data:
//!
//! - **JPEG**: an `APP1` segment starting `Exif\0\0`, always (every `APPn`
//!   segment comes before the scan). The first such segment whose first IFD
//!   has an orientation is the one used.
//! - **PNG**: an `eXIf` chunk before the first `IDAT` (Safari's rule, which
//!   Chrome follows); one after the image data is ignored.
//! - **WebP**: never. The container puts its `EXIF` chunk after the image
//!   data, and Chrome and Firefox ignore it there.
//! - GIF has no EXIF.
//!
//! The scan walks only the segment or chunk headers up to the image data, so a
//! file with no EXIF costs that walk and nothing else, and orientation 1 (or a
//! value outside 1..=8) leaves the decoded pixels as they are.

/// The EXIF orientation `bytes` (a file in `format`) asks to be drawn in,
/// `1..=8` as EXIF numbers them, or `None` when it carries none that applies
/// (no EXIF, no orientation tag, a value outside `1..=8`, or a WebP file).
pub(crate) fn exif_orientation(format: image::ImageFormat, bytes: &[u8]) -> Option<u8> {
    match format {
        image::ImageFormat::Jpeg => jpeg_exif_orientation(bytes),
        image::ImageFormat::Png => png_exif_orientation(bytes),
        _ => None,
    }
}

/// The orientation in the first `APP1` `Exif` segment that has one, walking
/// the marker segments up to the start of the scan (`SOS`).
fn jpeg_exif_orientation(bytes: &[u8]) -> Option<u8> {
    let mut found = None;
    jpeg_segments(bytes, |marker, data| {
        if marker == 0xE1 {
            found = data.strip_prefix(b"Exif\0\0").and_then(tiff_orientation);
        }
        found.is_none()
    });
    found
}

/// Walk a JPEG's marker segments up to the start of the scan (`SOS`, where
/// the image data begins), calling `f` with each segment's marker code and
/// its data (after the length) until `f` returns `false`. A malformed or
/// truncated file ends the walk where it stops making sense.
pub(crate) fn jpeg_segments<'a>(bytes: &'a [u8], mut f: impl FnMut(u8, &'a [u8]) -> bool) {
    if bytes.get(..2) != Some(&[0xFF, 0xD8][..]) {
        return;
    }
    let mut pos = 2;
    loop {
        // A marker is 0xFF and a code; any number of 0xFF fill bytes may
        // come before the code.
        if bytes.get(pos) != Some(&0xFF) {
            return;
        }
        while bytes.get(pos) == Some(&0xFF) {
            pos += 1;
        }
        let Some(&marker) = bytes.get(pos) else {
            return;
        };
        pos += 1;
        match marker {
            // Start of scan or end of image: the image data begins.
            0xDA | 0xD9 => return,
            // Markers with no length: TEM and RSTn.
            0x01 | 0xD0..=0xD7 => continue,
            _ => {}
        }
        let (Some(&hi), Some(&lo)) = (bytes.get(pos), bytes.get(pos + 1)) else {
            return;
        };
        let len = usize::from(u16::from_be_bytes([hi, lo]));
        let Some(data) = bytes.get(pos + 2..pos + len.max(2)) else {
            return;
        };
        if len < 2 || !f(marker, data) {
            return;
        }
        pos += len;
    }
}

/// The orientation in an `eXIf` chunk that comes before the first `IDAT`.
fn png_exif_orientation(bytes: &[u8]) -> Option<u8> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.get(..8)? != SIGNATURE {
        return None;
    }
    let mut pos = 8;
    loop {
        let len = u32::from_be_bytes(bytes.get(pos..pos + 4)?.try_into().ok()?) as usize;
        let kind = bytes.get(pos + 4..pos + 8)?;
        if kind == b"IDAT" || kind == b"IEND" {
            return None;
        }
        let data = bytes.get(pos + 8..(pos + 8).checked_add(len)?)?;
        if kind == b"eXIf" {
            // The chunk is the TIFF structure itself; some writers keep the
            // JPEG `Exif\0\0` header in front of it.
            let tiff = data.strip_prefix(b"Exif\0\0").unwrap_or(data);
            return tiff_orientation(tiff);
        }
        // Length, type, data, CRC.
        pos = pos.checked_add(12 + len)?;
    }
}

/// The orientation tag (`0x0112`, a single `SHORT`) of the first IFD of a
/// TIFF structure, if it is `1..=8`.
fn tiff_orientation(tiff: &[u8]) -> Option<u8> {
    let big_endian = match tiff.get(..4)? {
        [b'I', b'I', 42, 0] => false,
        [b'M', b'M', 0, 42] => true,
        _ => return None,
    };
    // Offsets are the file's own `u32`s, and every sum of them is checked:
    // an IFD offset near `u32::MAX` must end the scan, not overflow (on
    // wasm32 `usize` is 32 bits, and a debug overflow there aborts).
    let bytes_at = |at: u32, n: u32| -> Option<&[u8]> {
        let end = at.checked_add(n)?;
        tiff.get(usize::try_from(at).ok()?..usize::try_from(end).ok()?)
    };
    let u16_at = |at: u32| -> Option<u16> {
        let b: [u8; 2] = bytes_at(at, 2)?.try_into().ok()?;
        Some(if big_endian {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    };
    let u32_at = |at: u32| -> Option<u32> {
        let b: [u8; 4] = bytes_at(at, 4)?.try_into().ok()?;
        Some(if big_endian {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    };
    let ifd = u32_at(4)?;
    let entries = u32::from(u16_at(ifd)?);
    for i in 0..entries {
        let entry = ifd.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        let tag = u16_at(entry)?;
        if tag != 0x0112 {
            continue;
        }
        let kind = u16_at(entry.checked_add(2)?)?;
        let count = u32_at(entry.checked_add(4)?)?;
        if kind != 3 || count != 1 {
            return None;
        }
        let value = u16_at(entry.checked_add(8)?)?;
        return (1..=8).contains(&value).then_some(value as u8);
    }
    None
}

/// Turn decoded pixels the way EXIF orientation `o` (`2..=8`) says, so they
/// are drawn upright: the picture rinch keeps, sizes and paints is the
/// oriented one, so its natural size is the oriented size (width and height
/// swap for 5 to 8). Orientation 1 is the identity and changes nothing.
pub(crate) fn orient(img: &mut image::DynamicImage, o: u8) {
    use image::metadata::Orientation;
    if let Some(orientation) = Orientation::from_exif(o) {
        img.apply_orientation(orientation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TIFF structure whose first IFD holds `entries` (tag, type, count,
    /// value), in either byte order.
    fn tiff(big: bool, entries: &[(u16, u16, u32, u16)]) -> Vec<u8> {
        let u16b = |v: u16| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let u32b = |v: u32| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let mut t = Vec::new();
        t.extend_from_slice(if big { b"MM\0*" } else { b"II*\0" });
        t.extend_from_slice(&u32b(8));
        t.extend_from_slice(&u16b(entries.len() as u16));
        for &(tag, kind, count, value) in entries {
            t.extend_from_slice(&u16b(tag));
            t.extend_from_slice(&u16b(kind));
            t.extend_from_slice(&u32b(count));
            t.extend_from_slice(&u16b(value));
            t.extend_from_slice(&[0, 0]);
        }
        t.extend_from_slice(&u32b(0));
        t
    }

    #[test]
    fn the_orientation_tag_is_read_in_both_byte_orders() {
        for big in [false, true] {
            for o in 1..=8 {
                let t = tiff(big, &[(0x010F, 2, 1, 0), (0x0112, 3, 1, o)]);
                assert_eq!(tiff_orientation(&t), Some(o as u8), "big {big} o {o}");
            }
        }
    }

    #[test]
    fn a_value_outside_one_to_eight_or_the_wrong_type_is_no_orientation() {
        assert_eq!(tiff_orientation(&tiff(false, &[(0x0112, 3, 1, 0)])), None);
        assert_eq!(tiff_orientation(&tiff(false, &[(0x0112, 3, 1, 9)])), None);
        assert_eq!(tiff_orientation(&tiff(false, &[(0x0112, 4, 1, 6)])), None);
        assert_eq!(tiff_orientation(&tiff(false, &[(0x0112, 3, 2, 6)])), None);
        assert_eq!(tiff_orientation(&tiff(false, &[(0x010F, 3, 1, 6)])), None);
        assert_eq!(tiff_orientation(b"II*\0\xff\xff\xff\xff"), None);
        assert_eq!(tiff_orientation(b""), None);
    }

    /// IFD offsets at the top of the `u32` range: the sums of offsets
    /// overflow `u32` (and a 32-bit `usize`), which ends the scan.
    #[test]
    fn an_ifd_offset_near_the_top_of_the_range_is_no_orientation() {
        for big in [false, true] {
            for offset in [u32::MAX, u32::MAX - 1, u32::MAX - 3, u32::MAX - 13] {
                let mut t = tiff(big, &[(0x0112, 3, 1, 6)]);
                let b = if big {
                    offset.to_be_bytes()
                } else {
                    offset.to_le_bytes()
                };
                t[4..8].copy_from_slice(&b);
                assert_eq!(tiff_orientation(&t), None, "big {big} offset {offset:#x}");
            }
        }
    }

    #[test]
    fn truncated_files_are_no_orientation() {
        assert_eq!(jpeg_exif_orientation(&[0xFF, 0xD8, 0xFF, 0xE1, 0x00]), None);
        assert_eq!(
            jpeg_exif_orientation(&[0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x01]),
            None
        );
        assert_eq!(
            png_exif_orientation(b"\x89PNG\r\n\x1a\n\0\0\0\x10eXIf"),
            None
        );
        assert_eq!(
            png_exif_orientation(b"\x89PNG\r\n\x1a\n\xff\xff\xff\xffeXIf"),
            None
        );
    }
}
