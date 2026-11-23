//! The generic families' warm-up (#2552): `warm_generic_families` resolves
//! every family the generic lists name, in the scan every font context
//! shares, and never makes that scan itself. Its own process: the scan is
//! process-wide, so nothing else here may have made one first.

use parley::fontique::GenericFamily;
use rinch_dom::fonts::{FontDefaults, new_font_context, system_font_scans, warm_generic_families};
use std::collections::HashSet;

#[test]
fn the_warm_up_resolves_every_generic_family_and_never_scans() {
    let defaults = FontDefaults::default();
    // Before any context: nothing to warm, and no scan made for it.
    assert_eq!(warm_generic_families(&defaults).families, 0);
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

    let first = warm_generic_families(&defaults);
    assert!(
        first.families >= listed.len(),
        "{} looked up, {} listed",
        first.families,
        listed.len()
    );
    assert_eq!(system_font_scans(), 1, "the warm-up scanned again");

    // The look-ups did the work: a second warm-up finds every family
    // resolved already, in the scan all contexts share, and costs a
    // fraction of the first (a resolve is an `FcFontSort` over every
    // installed font; a resolved family is a map look-up).
    let second = warm_generic_families(&defaults);
    assert_eq!(second.families, first.families);
    assert!(
        first.lookups > second.lookups * 10,
        "first {:?}, second {:?}",
        first.lookups,
        second.lookups
    );
}
