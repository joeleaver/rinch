//! Font files for tests of what a registered face answers to (#2101, #2102):
//! one real face, Inter Regular, with its family name and the class it
//! declares rewritten, so a test has several distinct families of chosen
//! classes without vendoring more fonts.
//!
//! The rewrite touches three places and nothing else: every `Inter` in the
//! `name` table's string storage (both the Mac Roman and the UTF-16BE
//! records) becomes a five-letter name of the test's choosing, so the face
//! registers as a family of that name; the OS/2 table's IBM family class
//! (`sFamilyClass`) and PANOSE digits; and `post`'s `isFixedPitch`. Table
//! checksums are left stale, which neither fontique nor skrifa verifies.
//!
//! Shared by the `rinch` unit tests (`#[path]`), the desktop embed test and
//! the wasm embed test.

#![allow(dead_code)]

/// The face every variant is cut from.
pub const BASE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// The class a variant declares.
#[derive(Clone, Copy, Debug)]
pub enum Declared {
    /// IBM class 8 (sans serif), PANOSE Latin text with a normal sans serif.
    SansSerif,
    /// IBM class 1 (oldstyle serif), PANOSE Latin text with a cove serif.
    Serif,
    /// IBM class 10 (script), PANOSE Latin hand written.
    Script,
    /// `isFixedPitch` set; the IBM class and PANOSE say sans serif.
    Monospace,
    /// Everything zero: no classification.
    Unclassified,
}

/// `BASE` registered as the family `name` (exactly five ASCII letters),
/// declaring `class`.
pub fn variant(name: &str, class: Declared) -> Vec<u8> {
    assert_eq!(name.len(), 5, "the family name replaces `Inter` in place");
    assert!(name.is_ascii());
    let mut data = BASE.to_vec();

    let (name_off, name_len) = table(&data, b"name");
    let mac: &[u8] = b"Inter";
    let utf16: Vec<u8> = "Inter".encode_utf16().flat_map(u16::to_be_bytes).collect();
    let new_utf16: Vec<u8> = name.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let region = &mut data[name_off..name_off + name_len];
    let replaced =
        replace_all(region, &utf16, &new_utf16) + replace_all(region, mac, name.as_bytes());
    assert!(replaced > 0, "Inter's name table must name it Inter");

    let (os2, _) = table(&data, b"OS/2");
    let (ibm, panose): (u8, [u8; 4]) = match class {
        Declared::SansSerif => (8, [2, 11, 0, 3]),
        Declared::Serif => (1, [2, 2, 0, 3]),
        Declared::Script => (10, [3, 0, 0, 0]),
        Declared::Monospace => (8, [2, 11, 0, 3]),
        Declared::Unclassified => (0, [0, 0, 0, 0]),
    };
    data[os2 + 30] = ibm;
    data[os2 + 31] = 0;
    data[os2 + 32..os2 + 42].fill(0);
    data[os2 + 32..os2 + 36].copy_from_slice(&panose);

    let (post, _) = table(&data, b"post");
    let fixed: u32 = matches!(class, Declared::Monospace).into();
    data[post + 12..post + 16].copy_from_slice(&fixed.to_be_bytes());
    data
}

/// The offset and length of table `tag` in a single-face sfnt.
fn table(data: &[u8], tag: &[u8; 4]) -> (usize, usize) {
    let be32 = |at: usize| u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
    let count = u16::from_be_bytes([data[4], data[5]]) as usize;
    (0..count)
        .map(|i| 12 + 16 * i)
        .find(|record| &data[*record..*record + 4] == tag)
        .map(|record| (be32(record + 8), be32(record + 12)))
        .unwrap_or_else(|| panic!("no {} table", String::from_utf8_lossy(tag)))
}

/// Replace every occurrence of `from` in `region` with `to` (same length).
fn replace_all(region: &mut [u8], from: &[u8], to: &[u8]) -> usize {
    assert_eq!(from.len(), to.len());
    let mut count = 0;
    let mut at = 0;
    while at + from.len() <= region.len() {
        if &region[at..at + from.len()] == from {
            region[at..at + from.len()].copy_from_slice(to);
            count += 1;
            at += from.len();
        } else {
            at += 1;
        }
    }
    count
}
