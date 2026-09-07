//! Font-context construction, and the Android generic-family repair (#322).
//!
//! Every `parley::FontContext` in the process is built by [`new_font_context`]
//! so they all answer a font stack the same way. rinch keeps more than one —
//! the document's (layout, IFC, paint), the app's hit-test context, and a
//! throwaway one for input hit testing — and they have to agree glyph for
//! glyph or a tap lands on the wrong character.
//!
//! # The Android repair
//!
//! fontique's Android backend builds its generic-family map from a table of
//! literal family names *before* it reads `/system/etc/fonts.xml`, and the name
//! it uses for `monospace` is the CSS slot label `"monospace"` rather than a
//! family name that `/system/fonts` is actually indexed under (files are
//! indexed by the family name in their own `name` table — `Droid Sans Mono`,
//! `Roboto Mono`, …). The lookup misses, `filter_map` drops it, and the slot is
//! stored **empty**: `font-family: monospace` resolves to no face at all, so
//! every `<code>`, every `<pre>`, and rinch's own
//! `DEFAULT_FONT_FAMILY_MONOSPACE` stack — whose every name dead-ends on
//! Android — comes out proportional. `fonts.xml` cannot rescue it: its
//! `<family name=…>` arm interns the name and attaches nothing (the body is a
//! bare `TODO`), and it runs after the generic map is already built. The `ui-*`
//! and `fangsong` slots are never populated at all. Upstream `main` still
//! carries all of this, so a version bump is not a fix.
//!
//! [`repair_generic_families`] repairs the map afterwards through fontique's
//! public API: for each generic the platform left *empty*, resolve the first
//! candidate family the device actually has and append it. Probing for empty
//! first means a device whose map fontique populated correctly is left alone,
//! and a future upstream fix is not double-registered.

use parley::fontique::{Collection, FamilyId, GenericFamily};

/// Build a [`parley::FontContext`] with this platform's generic-family map
/// repaired.
///
/// Construct font contexts through here rather than `FontContext::new()`, so
/// no context can exist that resolves `monospace` differently from its peers.
pub fn new_font_context() -> parley::FontContext {
    let mut font_cx = parley::FontContext::new();
    repair_generic_families(&mut font_cx.collection);
    font_cx
}

/// Fill in the generic families this platform's fontique backend left empty.
///
/// A no-op on every platform but Android. Idempotent, and it never overrides a
/// generic the backend did populate.
pub fn repair_generic_families(collection: &mut Collection) {
    for (generic, family) in plan_repairs(GENERIC_REPAIRS, collection) {
        collection.append_generic_families(generic, core::iter::once(family));
    }
}

/// Put families an app *declared* for a generic at the head of that slot.
///
/// The claim goes to the front, not the tail, because of who else writes the
/// slot. fontique resolves a generic from the collection's own list first and
/// the platform backend's map second, so `append_generic_families` — which
/// extends the tail of the collection's own list — is already ahead of
/// fontconfig. But [`repair_generic_families`] fills an empty slot by
/// appending into that same list, at context construction, before any app
/// font has a chance to register: an appended claim would sit *behind* the
/// repair's platform face and silently lose, on Android only. A face the app
/// explicitly declared for a slot outranks any platform fallback, so a claim
/// is a prepend.
///
/// What was in the slot stays behind the claim rather than being replaced, so
/// a character the claimed face lacks can still fall through to the platform's
/// entry. When several claims name the same slot, the most recent goes first.
pub fn claim_generic_families(
    collection: &mut Collection,
    generic: GenericFamily,
    families: impl Iterator<Item = FamilyId>,
) {
    let incumbents: Vec<FamilyId> = collection.generic_families(generic).collect();
    collection.set_generic_families(generic, families.chain(incumbents));
}

/// Monospace candidates, in preference order.
///
/// `Roboto Mono` first because it is the face that matches the Roboto UI font
/// on a device that has it; `Droid Sans Mono` is the canonical AOSP monospace
/// (`/system/fonts/DroidSansMono.ttf` — the file `fonts.xml`'s own `monospace`
/// family names) and is present on essentially every device; `Noto Sans Mono`
/// covers newer Noto-only images; `Cutive Mono` is the last resort — a
/// typewriter serif, but a monospaced one, and `fonts.xml` already uses it for
/// `serif-monospace`.
#[cfg(any(target_os = "android", test))]
const MONOSPACE_CANDIDATES: &[&str] = &[
    "Roboto Mono",
    "Droid Sans Mono",
    "Noto Sans Mono",
    "Cutive Mono",
];

/// Sans-serif candidates — the names fontique's own Android backend uses for
/// [`GenericFamily::SansSerif`], which do resolve.
#[cfg(any(target_os = "android", test))]
const SANS_SERIF_CANDIDATES: &[&str] = &["Roboto Flex", "Roboto", "Noto Sans"];

/// Serif candidates — as fontique's own [`GenericFamily::Serif`].
#[cfg(any(target_os = "android", test))]
const SERIF_CANDIDATES: &[&str] = &["Noto Serif"];

/// Candidate family names for each generic family fontique's Android backend
/// leaves empty, in preference order.
///
/// The `ui-*` slots mirror their non-`ui-` counterparts — `ui-monospace` shares
/// [`MONOSPACE_CANDIDATES`] with `monospace` exactly so the two can never
/// disagree, which they otherwise could inside a single font stack.
///
/// Deliberately absent:
///
/// - `ui-rounded` — AOSP ships no rounded UI face at all. Mirroring sans-serif
///   would answer with a face that is not rounded; leaving the slot empty lets
///   a stack fall through to its next entry, which is what every other platform
///   does with this generic.
/// - `fangsong` — fangsong is a specific style, between Song and Kai. Android's
///   CJK faces are `Noto Sans/Serif CJK`, neither of which is one, so there is
///   no honest candidate and the slot stays empty rather than answering with
///   the wrong style.
/// - `cursive` — fontique already names a real family (`Dancing Script`). It is
///   not on every device, but AOSP has no second cursive face to fall back to,
///   so there is nothing to add.
#[cfg(any(target_os = "android", test))]
const ANDROID_GENERIC_REPAIRS: &[(GenericFamily, &[&str])] = &[
    (GenericFamily::Monospace, MONOSPACE_CANDIDATES),
    (GenericFamily::UiMonospace, MONOSPACE_CANDIDATES),
    (GenericFamily::UiSansSerif, SANS_SERIF_CANDIDATES),
    (GenericFamily::UiSerif, SERIF_CANDIDATES),
];

/// The repairs that apply to the platform being compiled for. Empty everywhere
/// but Android, so no other platform pays for this.
#[cfg(target_os = "android")]
const GENERIC_REPAIRS: &[(GenericFamily, &[&str])] = ANDROID_GENERIC_REPAIRS;
#[cfg(not(target_os = "android"))]
const GENERIC_REPAIRS: &[(GenericFamily, &[&str])] = &[];

/// What [`plan_repairs`] needs of a font collection.
///
/// A trait rather than a pair of closures so a host test can stand a fake in
/// for a `Collection` — none of this can be tested any other way, because its
/// only live call site is `#[cfg(target_os = "android")]` and CI has no Android
/// target.
trait FamilySource {
    /// How this source names a font family.
    type Family;

    /// Whether this generic currently resolves to no family at all.
    fn generic_is_empty(&mut self, generic: GenericFamily) -> bool;

    /// The family with this name, if the source has an actual face for it.
    fn family_with_faces(&mut self, name: &str) -> Option<Self::Family>;
}

impl FamilySource for Collection {
    type Family = FamilyId;

    fn generic_is_empty(&mut self, generic: GenericFamily) -> bool {
        self.generic_families(generic).next().is_none()
    }

    /// The has-a-face check is not redundant: parsing `fonts.xml` interns
    /// family names with nothing attached (`<family name="monospace">` is the
    /// reason this module exists), so a name resolving to an id is not by
    /// itself evidence that any font is behind it — appending such an id would
    /// leave the generic just as dead as before, while shadowing a candidate
    /// that would have worked.
    fn family_with_faces(&mut self, name: &str) -> Option<FamilyId> {
        let id = self.family_id(name)?;
        let family = self.family(id)?;
        (!family.fonts().is_empty()).then_some(id)
    }
}

/// The repairs to apply to `source`: for each generic in `table` that the
/// source reports empty, the first candidate family it actually has.
///
/// Generics the platform populated are skipped, so a device fontique got right
/// — or a future upstream fix — is left alone, and applying the plan twice
/// changes nothing the second time.
fn plan_repairs<S: FamilySource>(
    table: &[(GenericFamily, &[&str])],
    source: &mut S,
) -> Vec<(GenericFamily, S::Family)> {
    let mut plan = Vec::new();
    for (generic, candidates) in table {
        if !source.generic_is_empty(*generic) {
            continue;
        }
        if let Some(family) = first_available(candidates, |name| source.family_with_faces(name)) {
            plan.push((*generic, family));
        }
    }
    plan
}

/// The first candidate name that `lookup` resolves, in order.
fn first_available<T>(candidates: &[&str], mut lookup: impl FnMut(&str) -> Option<T>) -> Option<T> {
    candidates.iter().find_map(|name| lookup(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use parley::fontique::CollectionOptions;
    use std::cell::RefCell;

    /// A device with `installed` font families, standing in for a
    /// `Collection`. `populated` are the generics its platform backend already
    /// filled in. Records every family name looked up, in order.
    struct FakeDevice {
        installed: Vec<&'static str>,
        populated: Vec<GenericFamily>,
        asked: RefCell<Vec<String>>,
    }

    impl FakeDevice {
        fn new(installed: &[&'static str]) -> Self {
            Self {
                installed: installed.to_vec(),
                populated: Vec::new(),
                asked: RefCell::new(Vec::new()),
            }
        }

        fn with_populated(mut self, populated: &[GenericFamily]) -> Self {
            self.populated = populated.to_vec();
            self
        }

        fn asked(&self) -> Vec<String> {
            self.asked.borrow().clone()
        }
    }

    impl FamilySource for FakeDevice {
        /// Family names, so a plan says *which* face won.
        type Family = &'static str;

        fn generic_is_empty(&mut self, generic: GenericFamily) -> bool {
            !self.populated.contains(&generic)
        }

        /// Case-insensitive, as fontique's family-name map is.
        fn family_with_faces(&mut self, name: &str) -> Option<&'static str> {
            self.asked.borrow_mut().push(name.to_string());
            self.installed
                .iter()
                .find(|f| f.eq_ignore_ascii_case(name))
                .copied()
        }
    }

    /// The plan the real Android table produces on this device.
    fn plan(device: &mut FakeDevice) -> Vec<(GenericFamily, &'static str)> {
        plan_repairs(ANDROID_GENERIC_REPAIRS, device)
    }

    /// The face one generic would be repaired with, planning that row on its
    /// own so `asked()` reports only the lookups that row made.
    fn repaired(device: &mut FakeDevice, generic: GenericFamily) -> Option<&'static str> {
        let row = *ANDROID_GENERIC_REPAIRS
            .iter()
            .find(|(g, _)| *g == generic)
            .unwrap_or_else(|| panic!("{generic:?} is not in the repair table"));
        plan_repairs(&[row], device)
            .into_iter()
            .next()
            .map(|(_, family)| family)
    }

    /// The bug this module exists for: a stock device whose only monospaced
    /// face is `DroidSansMono.ttf` renders `<code>` proportional.
    #[test]
    fn a_stock_aosp_device_resolves_every_repaired_generic() {
        let mut device = FakeDevice::new(&["Roboto", "Noto Sans", "Noto Serif", "Droid Sans Mono"]);
        assert_eq!(
            plan(&mut device),
            vec![
                (GenericFamily::Monospace, "Droid Sans Mono"),
                (GenericFamily::UiMonospace, "Droid Sans Mono"),
                (GenericFamily::UiSansSerif, "Roboto"),
                (GenericFamily::UiSerif, "Noto Serif"),
            ]
        );
    }

    #[test]
    fn a_generic_the_platform_populated_is_left_alone() {
        let mut device = FakeDevice::new(&["Roboto Mono", "Noto Serif"])
            .with_populated(&[GenericFamily::Monospace]);
        assert_eq!(
            repaired(&mut device, GenericFamily::Monospace),
            None,
            "a populated generic must not be appended to"
        );
        assert!(
            device.asked().is_empty(),
            "a populated generic should not even be looked up"
        );
        // The rest of the table is still repaired.
        assert!(
            plan(&mut device)
                .iter()
                .any(|(g, _)| *g == GenericFamily::UiMonospace)
        );
    }

    #[test]
    fn applying_the_plan_twice_would_change_nothing() {
        // Second pass: everything the first pass filled in now reports
        // populated, so the planner has nothing left to do.
        let mut device = FakeDevice::new(&["Roboto", "Noto Serif", "Droid Sans Mono"]);
        let first = plan(&mut device);
        assert!(!first.is_empty());
        let mut device = device.with_populated(&first.iter().map(|(g, _)| *g).collect::<Vec<_>>());
        assert_eq!(plan(&mut device), vec![]);
    }

    #[test]
    fn the_earliest_candidate_wins_not_just_any_installed_one() {
        // Both are installed; the candidate order, not the install order,
        // decides.
        let mut device = FakeDevice::new(&["Cutive Mono", "Droid Sans Mono"]);
        assert_eq!(
            repaired(&mut device, GenericFamily::Monospace),
            Some("Droid Sans Mono")
        );
    }

    #[test]
    fn candidates_the_device_lacks_are_skipped() {
        let mut device = FakeDevice::new(&["Noto Sans Mono"]);
        assert_eq!(
            repaired(&mut device, GenericFamily::Monospace),
            Some("Noto Sans Mono")
        );
        assert_eq!(
            device.asked(),
            vec!["Roboto Mono", "Droid Sans Mono", "Noto Sans Mono"],
            "each earlier candidate should be tried exactly once"
        );
    }

    #[test]
    fn lookups_stop_at_the_first_hit() {
        let mut device = FakeDevice::new(&["Roboto Mono", "Droid Sans Mono"]);
        assert_eq!(
            repaired(&mut device, GenericFamily::Monospace),
            Some("Roboto Mono")
        );
        assert_eq!(
            device.asked(),
            vec!["Roboto Mono"],
            "later candidates should not be looked up once one resolves"
        );
    }

    #[test]
    fn a_generic_with_no_candidate_installed_is_dropped_from_the_plan() {
        // A device with no monospaced face at all: nothing to append, and
        // nothing bogus appended either.
        let mut device = FakeDevice::new(&["Roboto", "Noto Serif"]);
        assert_eq!(repaired(&mut device, GenericFamily::Monospace), None);
        assert_eq!(first_available(&[], |_: &str| Some(1)), None);
    }

    #[test]
    fn family_lookup_is_case_insensitive_like_fontique() {
        let mut device = FakeDevice::new(&["droid sans mono"]);
        assert_eq!(
            repaired(&mut device, GenericFamily::Monospace),
            Some("droid sans mono")
        );
    }

    #[test]
    fn the_two_monospace_slots_cannot_disagree() {
        // The same list, so no device can answer `monospace` and
        // `ui-monospace` with different faces.
        let mono: Vec<_> = ANDROID_GENERIC_REPAIRS
            .iter()
            .filter(|(g, _)| matches!(g, GenericFamily::Monospace | GenericFamily::UiMonospace))
            .map(|(_, candidates)| *candidates)
            .collect();
        assert_eq!(mono.len(), 2);
        assert_eq!(mono[0], mono[1]);
    }

    #[test]
    fn the_repair_table_is_well_formed() {
        for (generic, candidates) in ANDROID_GENERIC_REPAIRS {
            assert!(
                !candidates.is_empty(),
                "{generic:?} has no candidates, so its entry does nothing"
            );
            assert_eq!(
                ANDROID_GENERIC_REPAIRS
                    .iter()
                    .filter(|(g, _)| g == generic)
                    .count(),
                1,
                "{generic:?} is listed twice"
            );
            for name in *candidates {
                assert!(
                    GenericFamily::parse(name).is_none(),
                    "{name:?} is a CSS generic name, not a family name — naming a \
                     slot label instead of a family is exactly the upstream mistake \
                     this module repairs"
                );
            }
        }
    }

    /// The real `Collection` adapter — not the fake — on a collection holding
    /// no fonts at all: every generic reads empty, no candidate resolves, and
    /// the plan is therefore empty rather than appending an id with nothing
    /// behind it. (The positive path needs a device font index, so it is only
    /// reachable on a real Android device.)
    #[test]
    fn the_collection_adapter_finds_nothing_in_an_empty_collection() {
        let mut collection = Collection::new(CollectionOptions {
            system_fonts: false,
            shared: false,
        });
        assert!(collection.generic_is_empty(GenericFamily::Monospace));
        assert_eq!(collection.family_with_faces("Droid Sans Mono"), None);
        assert!(plan_repairs(ANDROID_GENERIC_REPAIRS, &mut collection).is_empty());

        repair_generic_families(&mut collection);
        assert!(collection.generic_is_empty(GenericFamily::Monospace));
    }

    /// Guards the decisions documented on `ANDROID_GENERIC_REPAIRS`: these
    /// slots are left empty on purpose, so filling one in should be a
    /// deliberate edit here too.
    #[test]
    fn the_slots_without_an_honest_candidate_are_left_alone() {
        for generic in [
            GenericFamily::UiRounded,
            GenericFamily::FangSong,
            GenericFamily::Cursive,
        ] {
            assert!(
                !ANDROID_GENERIC_REPAIRS.iter().any(|(g, _)| *g == generic),
                "{generic:?} has no AOSP face worth naming — see the module docs"
            );
        }
    }
    /// The `claim_generic_families` tests register a real font file — family
    /// ids only exist for registered fonts — under override names, so the
    /// families are distinct whatever the file's own `name` table says and
    /// nothing depends on the host having fonts installed.
    const CLAIM_FIXTURE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

    /// A no-system-fonts collection with one family per name in `names`, all
    /// from [`CLAIM_FIXTURE`], plus the families' ids in the same order.
    fn collection_with(names: &[&str]) -> (Collection, Vec<FamilyId>) {
        use parley::fontique::{Blob, CollectionOptions, FontInfoOverride};
        let mut collection = Collection::new(CollectionOptions {
            system_fonts: false,
            shared: false,
        });
        let ids = names
            .iter()
            .map(|name| {
                let registered = collection.register_fonts(
                    Blob::new(std::sync::Arc::new(CLAIM_FIXTURE)),
                    Some(FontInfoOverride {
                        family_name: Some(name),
                        ..Default::default()
                    }),
                );
                assert_eq!(registered.len(), 1, "one file, one family");
                registered[0].0
            })
            .collect();
        (collection, ids)
    }

    /// The order `font-family: monospace` would try families in.
    fn slot(collection: &mut Collection) -> Vec<FamilyId> {
        collection
            .generic_families(GenericFamily::Monospace)
            .collect()
    }

    /// A claim on a slot nobody filled is simply that face.
    #[test]
    fn a_claim_on_an_empty_generic_is_its_only_entry() {
        let (mut collection, ids) = collection_with(&["App Mono"]);
        claim_generic_families(
            &mut collection,
            GenericFamily::Monospace,
            ids.iter().copied(),
        );
        assert_eq!(slot(&mut collection), ids);
    }

    /// The reason this helper exists instead of `append_generic_families`:
    /// the #322 repair has already **appended** a platform family into the
    /// slot by the time an app font registers (it runs at context
    /// construction), so an appended claim would resolve second. A claim goes
    /// ahead of the incumbent — and keeps it, so a character the claimed face
    /// lacks can still fall through to the platform's.
    #[test]
    fn a_claim_goes_ahead_of_a_repair_filled_slot_and_keeps_it() {
        let (mut collection, ids) = collection_with(&["Platform Mono", "App Mono"]);
        let (platform, app) = (ids[0], ids[1]);
        // What `repair_generic_families` does to an empty slot.
        collection.append_generic_families(GenericFamily::Monospace, core::iter::once(platform));

        claim_generic_families(
            &mut collection,
            GenericFamily::Monospace,
            core::iter::once(app),
        );
        assert_eq!(
            slot(&mut collection),
            vec![app, platform],
            "the declared face resolves first; the repaired one stays as the fallthrough"
        );
    }

    /// Two claims on one slot: the most recent goes first. The case is
    /// degenerate — a real app declares one face per generic — but the order
    /// is documented, so it is pinned.
    #[test]
    fn the_most_recent_claim_on_a_slot_goes_first() {
        let (mut collection, ids) = collection_with(&["First Mono", "Second Mono"]);
        let (first, second) = (ids[0], ids[1]);
        claim_generic_families(
            &mut collection,
            GenericFamily::Monospace,
            core::iter::once(first),
        );
        claim_generic_families(
            &mut collection,
            GenericFamily::Monospace,
            core::iter::once(second),
        );
        assert_eq!(slot(&mut collection), vec![second, first]);
    }
}
