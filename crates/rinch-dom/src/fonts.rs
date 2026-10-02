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

/// The `font-family` to hand parley for a computed stack. **Every** parley
/// layout rinch builds from a computed `font-family` takes its family from
/// here, so no two sites can resolve a stack differently.
///
/// An empty stack is `sans-serif`. A stack that resolves to **no family** in
/// this font context — every name missing (`font-family: Helvetica` on Linux,
/// a typo) and no generic, or only generics this platform left empty — is
/// finished with the primary `sans-serif` family, by name (#1198).
///
/// Why: CSS falls back to the user agent's default font once the list is
/// exhausted, and parley has none. An ordinary character goes to the script
/// fallback, but a character with the Unicode `Emoji` property — which the
/// ASCII digits, `#` and `*` have, as keycap bases — is queried against the
/// stack and then the `emoji` generic, **ahead of** the script fallback. In a
/// stack that resolved to nothing, every digit was drawn from the colour-emoji
/// face at 1.245em while the letters beside it were not.
///
/// The primary face only, not the `sans-serif` generic: the generic expands to
/// the platform's whole list, whose text faces (DejaVu Sans, FreeSans) cover
/// many emoji and would win them ahead of the `emoji` generic. One face covers
/// the digits and letters and, where it has no emoji (Noto Sans, Roboto),
/// leaves an emoji to the `emoji` generic as before. **Accepted consequence:**
/// on a host whose primary `sans-serif` face is DejaVu Sans, an emoji in such
/// a stack is drawn in DejaVu's monochrome glyph: the appended face is named,
/// and a named family that covers an emoji draws it ([`TextFamily`], #1204),
/// as Chrome draws U+1F600 under `'DejaVu Sans'`.
///
/// A stack that resolves to anything is left alone, emoji included. So is
/// every stack in a context with no `sans-serif` face at all (wasm or embed
/// with no fonts registered), where there is nothing to append.
///
/// Not Chrome's answer for every name: Chrome 153 on Linux resolves
/// `Helvetica`, `Times` and `Courier` through fontconfig aliases (Nimbus,
/// Liberation) and draws an unknown family in its serif default. fontique
/// knows only installed names. Where no app font claims `sans-serif`, its
/// first face is where rinch's letters already fell back to (the platform's
/// script fallback), so this moves the digits and emoji-property punctuation
/// and leaves the letters where they were. With a claim
/// (`AppFont::sans_serif`), the letters of such a stack move to the claimed
/// face as well — the app's default, as CSS's UA default would be.
///
/// Cached per thread, keyed by the context's primary `sans-serif` family id
/// (unique per loaded collection, and changed by a claim on the slot) and the
/// stack, so a layout pays one lookup. A family registered after a stack was
/// cached as resolving to nothing leaves the appended face behind the newly
/// found one, which changes nothing it draws; registration cannot make a
/// resolving stack stop resolving.
pub fn parley_font_family(
    font_cx: &mut parley::FontContext,
    stack: &str,
) -> parley::style::FontFamily<'static> {
    use parley::style::FontFamily;
    use std::borrow::Cow;
    if stack.is_empty() {
        return FontFamily::Source(Cow::Borrowed(STACK_FALLBACK_GENERIC));
    }
    let collection = &mut font_cx.collection;
    let Some(primary) = collection.generic_families(GenericFamily::SansSerif).next() else {
        return FontFamily::Source(Cow::Owned(stack.to_owned()));
    };
    let cached = STACK_CACHE.with(|cache| {
        cache
            .borrow()
            .get(&primary)
            .and_then(|stacks| stacks.get(stack).cloned())
    });
    let finished = match cached {
        Some(finished) => finished,
        None => {
            let finished = if stack_resolves(collection, stack) {
                None
            } else {
                collection
                    .family_name(primary)
                    .and_then(quote_family)
                    .map(|name| format!("{stack}, {name}"))
            };
            STACK_CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                if cache.values().map(|m| m.len()).sum::<usize>() >= STACK_CACHE_LIMIT {
                    cache.clear();
                }
                cache
                    .entry(primary)
                    .or_default()
                    .insert(stack.to_owned(), finished.clone());
            });
            finished
        }
    };
    FontFamily::Source(Cow::Owned(finished.unwrap_or_else(|| stack.to_owned())))
}

/// The families a text is shaped with: [`parley_font_family`]'s for the whole
/// text, and for each emoji-presentation cluster in it the same stack with
/// every generic replaced by its primary face (#1204).
///
/// Why: parley shapes a cluster with the Unicode `Emoji` property against the
/// stack and then the `emoji` generic, and a generic such as `sans-serif`
/// expands to the platform's whole list (fontconfig's trimmed sort list:
/// about 180 families on a Linux desktop). Its text faces cover many emoji —
/// DejaVu Sans has U+1F600, FreeSans U+2B1C — so they won the emoji ahead of
/// the `emoji` generic, and U+1F600 under `sans-serif` (and under the theme's
/// `DEFAULT_FONT_FAMILY`) was drawn as a monochrome text glyph.
///
/// Chrome 153 on the same host treats a generic as its one primary face and
/// keeps a named family in its place: U+1F600 is Noto Color Emoji under
/// `sans-serif`, DejaVu Sans under `'DejaVu Sans'`, and Segoe UI Emoji under
/// the theme's stack, which names it after `sans-serif`. That is what the
/// rewritten stack gives an emoji cluster: named families in order, each
/// generic as its primary face, then parley's `emoji` generic. An `emoji`
/// generic written in the stack is kept as it is.
///
/// Only an emoji-presentation cluster gets it ([`emoji_presentation_ranges`]).
/// A text-default `Emoji` character — the digits, `#`, `*`, ©, U+2764 alone —
/// keeps the stack as written, so #1198's digits stay in the text face, and
/// U+2764 is a text glyph as it is in Chrome.
pub struct TextFamily {
    family: parley::style::FontFamily<'static>,
    emoji: Option<(
        parley::style::FontFamily<'static>,
        Vec<std::ops::Range<usize>>,
    )>,
}

impl TextFamily {
    /// Push the families onto a ranged builder of `text` (the text this was
    /// made for): the stack as the default, the emoji stack over each
    /// emoji-presentation cluster.
    pub fn push_to<B: parley::style::Brush>(self, builder: &mut parley::RangedBuilder<'_, B>) {
        use parley::style::StyleProperty;
        builder.push_default(StyleProperty::FontFamily(self.family));
        if let Some((family, ranges)) = self.emoji {
            for range in ranges {
                builder.push(StyleProperty::FontFamily(family.clone()), range);
            }
        }
    }
}

/// [`TextFamily`] for `text` under the computed `stack`. Every ranged parley
/// layout rinch builds from a computed `font-family` takes its families from
/// here.
pub fn parley_text_family(
    font_cx: &mut parley::FontContext,
    stack: &str,
    text: &str,
) -> TextFamily {
    let family = parley_font_family(font_cx, stack);
    let ranges = emoji_presentation_ranges(text);
    let emoji = if ranges.is_empty() {
        None
    } else {
        emoji_stack(&mut font_cx.collection, &family).map(|f| (f, ranges))
    };
    TextFamily { family, emoji }
}

/// The family an emoji-presentation cluster under `stack` is shaped with, or
/// `None` when it is the stack itself (no generic other than `emoji` in it).
/// For a builder that pushes its text piece by piece (the IFC's tree
/// builder); see [`TextFamily`].
pub fn parley_emoji_font_family(
    font_cx: &mut parley::FontContext,
    stack: &str,
) -> Option<parley::style::FontFamily<'static>> {
    let family = parley_font_family(font_cx, stack);
    emoji_stack(&mut font_cx.collection, &family)
}

/// `family` with each generic but `emoji` replaced by the first family of its
/// slot (dropped when the slot is empty), then the `emoji` generic, then those
/// generics as written; `None` if it holds no generic but `emoji`.
fn emoji_stack(
    collection: &mut Collection,
    family: &parley::style::FontFamily<'_>,
) -> Option<parley::style::FontFamily<'static>> {
    use parley::style::{FontFamily, FontFamilyName};
    use std::borrow::Cow;
    let FontFamily::Source(source) = family else {
        return None;
    };
    let mut replaced: Vec<GenericFamily> = Vec::new();
    let mut out: Vec<FontFamilyName<'static>> = Vec::new();
    for name in FontFamilyName::parse_css_list(source).map_while(Result::ok) {
        match name {
            FontFamilyName::Generic(GenericFamily::Emoji) => {
                out.push(FontFamilyName::Generic(GenericFamily::Emoji));
            }
            FontFamilyName::Generic(generic) => {
                replaced.push(generic);
                let primary = collection.generic_families(generic).next();
                if let Some(name) = primary.and_then(|id| collection.family_name(id)) {
                    out.push(FontFamilyName::Named(Cow::Owned(name.to_owned())));
                }
            }
            FontFamilyName::Named(name) => {
                out.push(FontFamilyName::Named(Cow::Owned(name.into_owned())));
            }
        }
    }
    if replaced.is_empty() {
        return None;
    }
    // Then the `emoji` generic, then each replaced generic whole: the emoji
    // face wins over a generic's later text faces, and those faces stay
    // behind it as coverage. Without the tail, a cluster the first face and
    // the emoji face both lack fell to the script fallback, or to `.notdef`
    // where there is none (embed and wasm app fonts) — review of #1270.
    // parley appends `emoji` once more, which changes nothing.
    out.push(FontFamilyName::Generic(GenericFamily::Emoji));
    out.extend(replaced.into_iter().map(FontFamilyName::Generic));
    Some(FontFamily::List(Cow::Owned(out)))
}

/// The byte ranges of `text`'s emoji-presentation grapheme clusters, adjacent
/// ones merged: a cluster whose base has the `Emoji` property and that holds
/// U+FE0F (VARIATION SELECTOR-16), or one holding an `Emoji_Presentation`
/// character and no U+FE0E (VARIATION SELECTOR-15). U+FE0F after any other
/// base (`𝄞`, Thai `ก`) is no emoji variation sequence (UTS #51) and is text. That takes in a modifier sequence (`☝🏽`, whose modifier has
/// the property), a flag (regional indicators have it) and a keycap written
/// with U+FE0F; a keycap without it (`1⃣`) is text, as in Chrome.
///
/// Every `Emoji_Presentation` character and U+FE0F is at or above U+231A,
/// whose UTF-8 lead byte is 0xE2, so a text with no byte that high is answered
/// without segmenting it.
pub fn emoji_presentation_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    use icu_properties::CodePointSetData;
    use icu_properties::props::{Emoji, EmojiPresentation};
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    if text.bytes().all(|b| b < 0xE2) {
        return ranges;
    }
    let presentation = CodePointSetData::new::<EmojiPresentation>();
    let emoji_property = CodePointSetData::new::<Emoji>();
    if !text
        .chars()
        .any(|c| c == '\u{fe0f}' || presentation.contains(c))
    {
        return ranges;
    }
    let segmenter = icu_segmenter::GraphemeClusterSegmenter::new();
    let mut start = 0;
    for end in segmenter.segment_str(text).skip(1) {
        let cluster = &text[start..end];
        let base_is_emoji = cluster
            .chars()
            .next()
            .is_some_and(|c| emoji_property.contains(c));
        let emoji = (base_is_emoji && cluster.contains('\u{fe0f}'))
            || (!cluster.contains('\u{fe0e}') && cluster.chars().any(|c| presentation.contains(c)));
        if emoji {
            match ranges.last_mut() {
                Some(last) if last.end == start => last.end = end,
                _ => ranges.push(start..end),
            }
        }
        start = end;
    }
    ranges
}

/// The generic an empty stack means, and whose primary face finishes a stack
/// that resolves to nothing.
pub const STACK_FALLBACK_GENERIC: &str = "sans-serif";

/// Distinct stacks kept per thread before the cache is dropped and rebuilt.
const STACK_CACHE_LIMIT: usize = 4096;

type StackCache =
    std::collections::HashMap<FamilyId, std::collections::HashMap<String, Option<String>>>;

thread_local! {
    /// primary `sans-serif` family → stack → `Some(finished stack)` or `None`
    /// (the stack resolves; use it as written).
    static STACK_CACHE: std::cell::RefCell<StackCache> =
        std::cell::RefCell::new(StackCache::new());
}

/// Whether `stack` names at least one family this collection has, the way
/// parley resolves it (`parse_css_list`, stopping at the first parse error).
fn stack_resolves(collection: &mut Collection, stack: &str) -> bool {
    use parley::style::FontFamilyName;
    for family in FontFamilyName::parse_css_list(stack).map_while(Result::ok) {
        let found = match family {
            FontFamilyName::Named(name) => collection.family_by_name(&name).is_some(),
            FontFamilyName::Generic(generic) => {
                collection.generic_families(generic).next().is_some()
            }
        };
        if found {
            return true;
        }
    }
    false
}

/// `name` as a CSS string that parley's `parse_css_list` reads back as that
/// name, so a family name with spaces or commas survives. parley's parser has
/// no escapes, so the quote is one the name does not contain; a name holding
/// both quote characters cannot be written. Used for the face appended here
/// and for every name in a computed `font-family` (#1223).
pub(crate) fn quote_family(name: &str) -> Option<String> {
    let quote = ['"', '\''].into_iter().find(|q| !name.contains(*q))?;
    Some(format!("{quote}{name}{quote}"))
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

    /// `parley_font_family` (#1198) on a collection with no system fonts, so
    /// what resolves is exactly what the test registered.
    mod finish_stack {
        use super::super::parley_font_family;
        use parley::fontique::{
            Blob, Collection, CollectionOptions, FontInfoOverride, GenericFamily,
        };
        use parley::style::FontFamily;

        const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

        fn context() -> parley::FontContext {
            parley::FontContext {
                collection: Collection::new(CollectionOptions {
                    shared: false,
                    system_fonts: false,
                }),
                ..Default::default()
            }
        }

        fn register(font_cx: &mut parley::FontContext, name: &str) -> parley::fontique::FamilyId {
            font_cx.collection.register_fonts(
                Blob::new(std::sync::Arc::new(FACE)),
                Some(FontInfoOverride {
                    family_name: Some(name),
                    ..Default::default()
                }),
            )[0]
            .0
        }

        fn source(family: FontFamily<'_>) -> String {
            match family {
                FontFamily::Source(s) => s.into_owned(),
                other => panic!("not a source list: {other:?}"),
            }
        }

        #[test]
        fn with_no_sans_serif_face_nothing_is_appended() {
            // wasm / embed before any font is registered: nothing to append,
            // and no panic.
            let mut cx = context();
            assert_eq!(
                source(parley_font_family(&mut cx, "Helvetica")),
                "Helvetica"
            );
            assert_eq!(source(parley_font_family(&mut cx, "")), "sans-serif");
        }

        #[test]
        fn a_stack_that_resolves_to_nothing_gets_the_primary_face_quoted() {
            let mut cx = context();
            let id = register(&mut cx, "Primary, Face \"1198\"");
            cx.collection
                .set_generic_families(GenericFamily::SansSerif, core::iter::once(id));
            let finished = source(parley_font_family(&mut cx, "NoSuchFamily1198"));
            assert_eq!(finished, r#"NoSuchFamily1198, 'Primary, Face "1198"'"#);
            // parley parses the quoted name back to the registered family.
            let names: Vec<_> = parley::style::FontFamilyName::parse_css_list(&finished)
                .map_while(Result::ok)
                .collect();
            assert_eq!(names.len(), 2);
            assert!(matches!(&names[1],
                parley::style::FontFamilyName::Named(n) if n == "Primary, Face \"1198\""));
            // A generic this collection left empty resolves to nothing too.
            assert_eq!(
                source(parley_font_family(&mut cx, "monospace")),
                r#"monospace, 'Primary, Face "1198"'"#
            );
        }

        #[test]
        fn a_stack_that_resolves_is_left_alone() {
            let mut cx = context();
            let id = register(&mut cx, "Primary1198");
            register(&mut cx, "Other1198");
            cx.collection
                .set_generic_families(GenericFamily::SansSerif, core::iter::once(id));
            for stack in [
                "Other1198",
                "NoSuchFamily1198, Other1198",
                "sans-serif",
                "x, sans-serif",
            ] {
                assert_eq!(source(parley_font_family(&mut cx, stack)), stack);
            }
        }

        #[test]
        fn a_claim_on_the_slot_changes_the_appended_face() {
            // The cache is keyed by the primary face, so a later claim is seen.
            let mut cx = context();
            let a = register(&mut cx, "FirstPrimary1198");
            cx.collection
                .set_generic_families(GenericFamily::SansSerif, core::iter::once(a));
            assert_eq!(
                source(parley_font_family(&mut cx, "Gone1198")),
                r#"Gone1198, "FirstPrimary1198""#
            );
            let b = register(&mut cx, "SecondPrimary1198");
            crate::fonts::claim_generic_families(
                &mut cx.collection,
                GenericFamily::SansSerif,
                core::iter::once(b),
            );
            assert_eq!(
                source(parley_font_family(&mut cx, "Gone1198")),
                r#"Gone1198, "SecondPrimary1198""#
            );
        }
    }

    /// Which clusters count as emoji presentation (#1204).
    mod emoji_presentation {
        use super::super::emoji_presentation_ranges as ranges;

        #[test]
        fn text_with_no_emoji_presentation_has_no_ranges() {
            for text in [
                "",
                "abc 123 #*",
                "\u{a9}\u{ae}\u{2122}\u{2194}",
                // U+2764 is `Emoji` but text-default; U+2026 is neither.
                "\u{2764}\u{2026}",
                // A keycap without U+FE0F is text.
                "1\u{20e3}",
                // CJK takes the slow path (lead bytes >= 0xE2) and finds nothing.
                "\u{6f22}\u{5b57}",
            ] {
                assert_eq!(
                    ranges(text),
                    Vec::<std::ops::Range<usize>>::new(),
                    "{text:?}"
                );
            }
        }

        #[test]
        fn emoji_presentation_clusters_are_found_whole() {
            // U+2B1C (3 bytes), U+1F600 (4), with text around.
            assert_eq!(ranges("a\u{2b1c}b\u{1f600}"), vec![1..4, 5..9]);
            // U+2764 U+FE0F: emoji by its selector.
            assert_eq!(ranges("x\u{2764}\u{fe0f}"), vec![1..7]);
            // A keycap written with U+FE0F.
            assert_eq!(ranges("1\u{fe0f}\u{20e3}"), vec![0..7]);
            // A modifier sequence on a text-default base (U+261D, U+1F3FD).
            assert_eq!(ranges("\u{261d}\u{1f3fd}"), vec![0..7]);
            // A flag: two regional indicators, one cluster.
            assert_eq!(ranges("\u{1f1ef}\u{1f1f5}"), vec![0..8]);
            // Adjacent clusters merge into one range.
            assert_eq!(ranges("\u{2b1c}\u{2b1c}a"), vec![0..6]);
        }

        /// U+FE0F counts only after a base with the `Emoji` property.
        #[test]
        fn a_selector_after_a_non_emoji_base_is_text() {
            for text in [
                "\u{1d11e}\u{fe0f}",
                "\u{e01}\u{fe0f}",
                "A\u{fe0f}",
                "\u{416}\u{fe0f}",
            ] {
                assert_eq!(
                    ranges(text),
                    Vec::<std::ops::Range<usize>>::new(),
                    "{text:?}"
                );
            }
            // (c) and U+2600 have the property: their VS16 sequences are emoji.
            assert_eq!(ranges("\u{a9}\u{fe0f}"), vec![0..5]);
            assert_eq!(ranges("\u{2600}\u{fe0f}"), vec![0..6]);
        }

        #[test]
        fn a_text_variation_selector_makes_a_cluster_text() {
            assert_eq!(ranges("\u{2b1c}\u{fe0e}\u{2b1c}"), vec![6..9]);
        }
    }

    /// The stack an emoji cluster is shaped with (#1204).
    mod emoji_stack {
        use super::super::{emoji_stack, quote_family};
        use parley::fontique::{
            Blob, Collection, CollectionOptions, FontInfoOverride, GenericFamily,
        };
        use parley::style::{FontFamily, FontFamilyName};

        const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

        fn collection() -> Collection {
            let mut c = Collection::new(CollectionOptions {
                shared: false,
                system_fonts: false,
            });
            let mut reg = |name: &str| {
                c.register_fonts(
                    Blob::new(std::sync::Arc::new(FACE)),
                    Some(FontInfoOverride {
                        family_name: Some(name),
                        ..Default::default()
                    }),
                )[0]
                .0
            };
            let (a, b, m) = (reg("SansA1204"), reg("Sans B 1204"), reg("MonoA1204"));
            c.set_generic_families(GenericFamily::SansSerif, [a, b].into_iter());
            c.set_generic_families(GenericFamily::Monospace, core::iter::once(m));
            c
        }

        fn list(stack: &str) -> Option<Vec<String>> {
            let family = FontFamily::Source(stack.to_owned().into());
            let out = emoji_stack(&mut collection(), &family)?;
            let FontFamily::List(names) = out else {
                panic!("a list: {out:?}");
            };
            Some(
                names
                    .iter()
                    .map(|n| match n {
                        FontFamilyName::Named(n) => quote_family(n).unwrap(),
                        FontFamilyName::Generic(g) => format!("{g:?}"),
                    })
                    .collect(),
            )
        }

        #[test]
        fn each_generic_is_its_primary_face_and_names_keep_their_place() {
            assert_eq!(
                list("X1204, sans-serif, 'Y 1204', monospace").unwrap(),
                vec![
                    "\"X1204\"",
                    "\"SansA1204\"",
                    "\"Y 1204\"",
                    "\"MonoA1204\"",
                    // The generics stay behind the emoji face as coverage.
                    "Emoji",
                    "SansSerif",
                    "Monospace",
                ]
            );
        }

        #[test]
        fn an_empty_generic_is_dropped_and_emoji_is_kept() {
            assert_eq!(
                list("cursive, emoji, sans-serif").unwrap(),
                vec!["Emoji", "\"SansA1204\"", "Emoji", "Cursive", "SansSerif"]
            );
            assert_eq!(list("cursive").unwrap(), vec!["Emoji", "Cursive"]);
        }

        #[test]
        fn a_stack_with_no_generic_but_emoji_is_left_alone() {
            assert_eq!(list("X1204, 'Y 1204'"), None);
            assert_eq!(list("X1204, emoji"), None);
        }
    }
}
