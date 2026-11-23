//! The generic families' warm-up (#2552): `warm_generic_families` resolves
//! every family the generic lists name, in the scan every font context
//! shares, and never makes that scan itself. Its own process: the scan is
//! process-wide, so nothing else here may have made one first.

use parley::fontique::GenericFamily;
use rinch_dom::fonts::{new_font_context, system_font_scans, warm_generic_families};
use std::collections::HashSet;

#[test]
fn the_warm_up_resolves_every_generic_family_and_never_scans() {
    // Before any context: nothing to warm, and no scan made for it.
    assert_eq!(warm_generic_families(), 0);
    assert_eq!(system_font_scans(), 0, "the warm-up scanned");

    // The families a document's context lists for the four generics.
    let mut cx = new_font_context();
    let mut listed = HashSet::new();
    for generic in [
        GenericFamily::SansSerif,
        GenericFamily::SystemUi,
        GenericFamily::Serif,
        GenericFamily::Monospace,
    ] {
        listed.extend(cx.collection.generic_families(generic));
    }
    assert!(!listed.is_empty(), "this host lists no generic family");

    assert_eq!(warm_generic_families(), listed.len());
    assert_eq!(system_font_scans(), 1, "the warm-up scanned again");
}
