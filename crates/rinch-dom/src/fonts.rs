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
/// repaired, and on a desktop the `serif` and `sans-serif` slots headed by
/// this thread's [`FontDefaults`] (#2255).
///
/// Construct font contexts through here rather than `FontContext::new()`, so
/// no context can exist that resolves `monospace` differently from its peers.
pub fn new_font_context() -> parley::FontContext {
    let mut font_cx = parley::FontContext::new();
    repair_generic_families(&mut font_cx.collection);
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    FONT_DEFAULTS
        .with(|defaults| apply_generic_defaults(&mut font_cx.collection, &defaults.borrow()));
    font_cx
}

pub use rinch_core::font_defaults::FontDefaults;

thread_local! {
    /// This thread's [`FontDefaults`]. Per thread, like the stack cache: the
    /// thread that lays out is the one that reads it (the UI thread, which
    /// `App::run` sets it on).
    static FONT_DEFAULTS: std::cell::RefCell<FontDefaults> =
        std::cell::RefCell::new(FontDefaults::default());
}

/// Set this thread's [`FontDefaults`]. `standard` applies to every layout
/// from here on; `serif` and `sans_serif` to the font contexts
/// [`new_font_context`] builds from here on, so set them before the first
/// document (the `App` builder does).
pub fn set_font_defaults(defaults: FontDefaults) {
    FONT_DEFAULTS.with(|d| *d.borrow_mut() = defaults);
    STACK_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// This thread's [`FontDefaults`].
pub fn font_defaults() -> FontDefaults {
    FONT_DEFAULTS.with(|d| d.borrow().clone())
}

/// Put `defaults`' `serif` and `sans_serif` families at the head of their
/// slots in `collection`, each only where it is drawn in an installed face
/// (itself, a system substitute (`system_substitute`) or its
/// [alternate](alternate_family_name)); the slot's other families stay
/// behind it. [`new_font_context`] does this on a desktop.
///
/// Why: Chrome's `serif` is its serif *setting*, Times New Roman on Linux,
/// not fontconfig's `serif` alias (Noto Serif or DejaVu Serif): 74 characters
/// at 16px were 501.66px in Chrome and 593px in rinch on one host (#2255).
/// A host without the family keeps the platform's slot, as Chrome then falls
/// back to the platform too. A host with Liberation Serif and no Times New
/// Roman gets Liberation Serif, as in Chrome (#2336).
pub fn apply_generic_defaults(collection: &mut Collection, defaults: &FontDefaults) {
    apply_generic_defaults_with(collection, defaults, &mut system_matcher(defaults));
}

fn apply_generic_defaults_with(
    collection: &mut Collection,
    defaults: &FontDefaults,
    system: &mut SystemMatcher<'_>,
) {
    for (generic, name) in [
        (GenericFamily::Serif, &defaults.serif),
        (GenericFamily::SansSerif, &defaults.sans_serif),
    ] {
        let Some(id) = name
            .as_deref()
            .and_then(|n| installed_family(collection, n, system))
        else {
            continue;
        };
        let rest: Vec<FamilyId> = collection
            .generic_families(generic)
            .filter(|f| *f != id)
            .collect();
        collection.set_generic_families(generic, core::iter::once(id).chain(rest));
    }
}

/// Blink's alternate family for `name`, tried when `name` itself is not
/// installed (`AlternateFamilyName`, `alternate_font_family.h`): Courier and
/// Courier New, Times and Times New Roman, Arial and Helvetica, each way
/// (Courier New to Courier except on Windows). ASCII case-insensitive.
///
/// Why (#2250): a mail stack such as `Helvetica Neue, Helvetica, Lucida
/// Grande, tahoma, verdana, arial, sans-serif` on a Linux host without
/// Helvetica is drawn in Arial by Chrome (166.53px, Arial's width), where
/// rinch fell through to Tahoma. Not fontconfig's best match: `fc-match
/// Helvetica` there is Nimbus Sans (165.77px), which Chrome does not use
/// (not a replacement it accepts, `system_substitute`).
pub fn alternate_family_name(name: &str) -> Option<&'static str> {
    let is = |s: &str| name.eq_ignore_ascii_case(s);
    if is("Courier") {
        Some("Courier New")
    } else if is("Courier New") && !cfg!(target_os = "windows") {
        Some("Courier")
    } else if is("Times") {
        Some("Times New Roman")
    } else if is("Times New Roman") {
        Some("Times")
    } else if is("Arial") {
        Some("Helvetica")
    } else if is("Helvetica") {
        Some("Arial")
    } else {
        None
    }
}

/// The family `name` is drawn in here, as [`resolve_family`] finds it.
fn installed_family(
    collection: &mut Collection,
    name: &str,
    system: &mut SystemMatcher<'_>,
) -> Option<FamilyId> {
    resolve_family(collection, name, system).map(|(id, _)| id)
}

/// The family a named family `name` is drawn in, in Chrome's order, and the
/// name to write in its place (`None` when it is `name` itself):
///
/// 1. `name`, if this collection has it;
/// 2. else the family the system substitutes for `name`, where Chrome
///    accepts it ([`system_substitute`], #2336);
/// 3. else `name`'s [alternate](alternate_family_name), if the collection has
///    it, or else the system's substitute for the alternate.
///
/// That is Blink's order: `FontCache` asks the font manager for `name`, which
/// on Linux takes fontconfig's match when it is acceptable, and only when
/// that fails asks again for the alternate. So with Liberation Sans and
/// Nimbus Sans installed and no Arial, `Helvetica` is Liberation Sans:
/// fontconfig's Nimbus Sans is not acceptable for Helvetica, and Arial's
/// match, Liberation Sans, is (Chrome 156).
fn resolve_family(
    collection: &mut Collection,
    name: &str,
    system: &mut SystemMatcher<'_>,
) -> Option<(FamilyId, Option<String>)> {
    if let Some(family) = collection.family_by_name(name) {
        return Some((family.id(), None));
    }
    if let Some(found) = substitute_family(collection, name, system) {
        return Some(found);
    }
    let alternate = alternate_family_name(name)?;
    if let Some(family) = collection.family_by_name(alternate) {
        return Some((family.id(), Some(alternate.to_owned())));
    }
    substitute_family(collection, alternate, system)
}

/// The system's substitute for a missing `name`, where Chrome accepts it,
/// with the family the collection has under that name.
fn substitute_family(
    collection: &mut Collection,
    name: &str,
    system: &mut SystemMatcher<'_>,
) -> Option<(FamilyId, Option<String>)> {
    let answer = system(collection, name)?;
    let substitute = system_substitute(name, &answer)?;
    let family = collection.family_by_name(substitute)?;
    Some((family.id(), Some(substitute.to_owned())))
}

/// What the system's font configuration answers for a family name: the
/// query's first family after the configuration's substitutions, and the
/// family names of the font it matches (fontique's
/// `Collection::system_family_match`, fontconfig's `FcConfigSubstitute` and
/// `FcFontMatch`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SystemMatch {
    pub(crate) configured: String,
    pub(crate) matched: Vec<String>,
}

/// Asks the system for a family name's [`SystemMatch`].
pub(crate) type SystemMatcher<'a> = dyn FnMut(&mut Collection, &str) -> Option<SystemMatch> + 'a;

/// The system matcher a layout uses: fontique's fontconfig query where
/// `defaults` follow the system's substitutes, else none. fontique answers
/// on Linux and the BSDs only, so elsewhere this finds nothing.
fn system_matcher(
    defaults: &FontDefaults,
) -> impl FnMut(&mut Collection, &str) -> Option<SystemMatch> + use<> {
    gated(defaults, |collection: &mut Collection, name: &str| {
        let answer = collection.system_family_match(name)?;
        Some(SystemMatch {
            configured: answer.configured,
            matched: answer.matched,
        })
    })
}

/// `query`, or nothing where `defaults` turn system substitutes off.
fn gated<Q>(
    defaults: &FontDefaults,
    mut query: Q,
) -> impl FnMut(&mut Collection, &str) -> Option<SystemMatch> + use<Q>
where
    Q: FnMut(&mut Collection, &str) -> Option<SystemMatch>,
{
    let on = defaults.system_substitutes;
    move |collection: &mut Collection, name: &str| if on { query(collection, name) } else { None }
}

/// The name of the font the system matched for the family `name`, where
/// Chrome draws `name` in it, else `None` (Skia's
/// `SkFontConfigInterfaceDirect::isAcceptableMatch`, which Chrome on Linux
/// uses for every family it looks up).
///
/// fontconfig always matches *some* font, so Chrome accepts the match only
/// when one of its family names is
///
/// - the family the configuration put first (`configured`): an alias the
///   configuration prefers, such as a user's rule mapping Arial to another
///   family;
/// - `name` itself;
/// - or a metric-compatible replacement of `name` ([`metric_compatible`]):
///   Liberation Sans or Arimo for Arial, Liberation Serif or Tinos for Times
///   New Roman, and so on.
///
/// `sans`, `serif` and `monospace` written as family names (and an empty
/// name) accept any match, as Chrome's fixed-width default `Monospace` does.
///
/// Measured in Chrome 156 on a host with Liberation and no msttcorefonts
/// (#2336): `Arial` is drawn in Liberation Sans, `Times New Roman` in
/// Liberation Serif, `Courier New` in Liberation Mono, `'MS PGothic'` in
/// Noto Sans CJK JP, and a family a configuration rule *prepends* in front
/// of a name is drawn for it; but `Calibri` without Carlito, `Georgia`, and a
/// family a rule only *appends* (fontconfig's `fc-match` answer) are not:
/// the list moves on to its next family.
pub(crate) fn system_substitute<'m>(name: &str, answer: &'m SystemMatch) -> Option<&'m str> {
    let any = name.is_empty()
        || ["sans", "serif", "monospace"]
            .iter()
            .any(|g| name.eq_ignore_ascii_case(g));
    if any {
        return answer.matched.first().map(String::as_str);
    }
    answer
        .matched
        .iter()
        .find(|m| {
            m.eq_ignore_ascii_case(&answer.configured)
                || m.eq_ignore_ascii_case(name)
                || metric_compatible(name, m)
        })
        .map(String::as_str)
}

/// Whether `a` and `b` are metric-compatible replacements for each other in
/// Chrome's table (Skia's `GetFontEquivClass`, `SkFontConfigInterface_direct`):
/// both in one of its classes. ASCII case-insensitive, as Skia's `strcasecmp`
/// is.
pub fn metric_compatible(a: &str, b: &str) -> bool {
    let class = |name: &str| {
        METRIC_COMPATIBLE
            .iter()
            .position(|class| class.iter().any(|n| n.eq_ignore_ascii_case(name)))
    };
    let a = class(a);
    a.is_some() && a == class(b)
}

/// Chrome's metric-compatible classes (Skia's `kFontEquivMap`): the
/// families each class may draw in each other's place. A name may be in more
/// than one class (Noto Serif CJK JP is both MS PMincho's and MS Mincho's);
/// as in Skia, its first class is the one it is judged by.
const METRIC_COMPATIBLE: &[&[&str]] = &[
    &["Arial", "Arimo", "Liberation Sans"],
    &["Times New Roman", "Tinos", "Liberation Serif"],
    &["Courier New", "Cousine", "Liberation Mono"],
    &["Symbol", "Symbol Neu"],
    // MS PGothic
    &[
        "MS PGothic",
        "\u{ff2d}\u{ff33} \u{ff30}\u{30b4}\u{30b7}\u{30c3}\u{30af}",
        "Noto Sans CJK JP",
        "IPAPGothic",
        "MotoyaG04Gothic",
    ],
    // MS Gothic
    &[
        "MS Gothic",
        "\u{ff2d}\u{ff33} \u{30b4}\u{30b7}\u{30c3}\u{30af}",
        "Noto Sans Mono CJK JP",
        "IPAGothic",
        "MotoyaG04GothicMono",
    ],
    // MS PMincho
    &[
        "MS PMincho",
        "\u{ff2d}\u{ff33} \u{ff30}\u{660e}\u{671d}",
        "Noto Serif CJK JP",
        "IPAPMincho",
        "MotoyaG04Mincho",
    ],
    // MS Mincho
    &[
        "MS Mincho",
        "\u{ff2d}\u{ff33} \u{660e}\u{671d}",
        "Noto Serif CJK JP",
        "IPAMincho",
        "MotoyaG04MinchoMono",
    ],
    // SimSun
    &[
        "Simsun",
        "\u{5b8b}\u{4f53}",
        "Noto Serif CJK SC",
        "MSung GB18030",
        "Song ASC",
    ],
    // NSimSun
    &[
        "NSimsun",
        "\u{65b0}\u{5b8b}\u{4f53}",
        "Noto Serif CJK SC",
        "MSung GB18030",
        "N Song ASC",
    ],
    // SimHei
    &[
        "Simhei",
        "\u{9ed1}\u{4f53}",
        "Noto Sans CJK SC",
        "MYingHeiGB18030",
        "MYingHeiB5HK",
    ],
    // PMingLiU
    &[
        "PMingLiU",
        "\u{65b0}\u{7d30}\u{660e}\u{9ad4}",
        "Noto Serif CJK TC",
        "MSung B5HK",
    ],
    // MingLiU
    &[
        "MingLiU",
        "\u{7d30}\u{660e}\u{9ad4}",
        "Noto Serif CJK TC",
        "MSung B5HK",
    ],
    // PMingLiU_HKSCS
    &[
        "PMingLiU_HKSCS",
        "\u{65b0}\u{7d30}\u{660e}\u{9ad4}_HKSCS",
        "Noto Serif CJK TC",
        "MSung B5HK",
    ],
    // MingLiU_HKSCS
    &[
        "MingLiU_HKSCS",
        "\u{7d30}\u{660e}\u{9ad4}_HKSCS",
        "Noto Serif CJK TC",
        "MSung B5HK",
    ],
    &["Cambria", "Caladea"],
    &["Calibri", "Carlito"],
];

/// The `font-family` to hand parley for a computed stack. **Every** parley
/// layout rinch builds from a computed `font-family` takes its family from
/// here, so no two sites can resolve a stack differently.
///
/// An empty stack is `sans-serif`. Otherwise two things are done to it, both
/// what Chrome does:
///
/// - **A missing family is replaced by the family it is drawn in**, in its
///   place in the list: fontconfig's substitute where Chrome accepts it
///   (#2336), else its alternate where that is installed (#2250): Helvetica
///   by Arial, Times by Times New Roman, Courier by Courier New and back
///   ([`alternate_family_name`], Blink's table). See `resolve_family`.
/// - **A stack that resolves to no family** in this font context — every name
///   missing (a typo, a web font the host lacks) and no generic, or only
///   generics this platform left empty — **ends in the default font**, by
///   name (#2255): the first family of this thread's
///   [`FontDefaults::standard`] the context has, Times New Roman (or
///   Times) by default as in Chrome; else the `sans-serif` slot's first
///   face. Chrome on a host with no Times New Roman draws such a stack in
///   fontconfig's "Sans" (DejaVu Sans, 609.94px on a DejaVu-only host), not
///   in its serif setting, so the default `standard` names Times alone.
///
/// Why the default font is appended at all (#1198): CSS falls back to the user
/// agent's default font once the list is exhausted, and parley has none. An
/// ordinary character goes to the script fallback, but a character with the
/// Unicode `Emoji` property — which the ASCII digits, `#` and `*` have, as
/// keycap bases — is queried against the stack and then the `emoji` generic,
/// **ahead of** the script fallback. In a stack that resolved to nothing,
/// every digit was drawn from the colour-emoji face at 1.245em while the
/// letters beside it were not. And Chrome draws such a stack in its standard
/// font: `font-family: Oswald` measured 501.66px in Chrome (Times New Roman)
/// and 547px in rinch (the script fallback) for one 74-character line.
///
/// One face by name, not a generic: a generic expands to the platform's whole
/// list, whose text faces (DejaVu Sans, FreeSans) cover many emoji and would
/// win them ahead of the `emoji` generic. One face covers the digits and
/// letters and, where it has no emoji (Times New Roman, Noto Sans), leaves an
/// emoji to the `emoji` generic as before. **Accepted consequence:** where
/// that face is DejaVu, an emoji in such a stack is drawn in DejaVu's
/// monochrome glyph: the appended face is named, and a named family that
/// covers an emoji draws it ([`TextFamily`], #1204), as Chrome draws U+1F600
/// under `'DejaVu Sans'`.
///
/// A stack that resolves to anything gets no default, emoji included, so a
/// theme's stack (which ends in a generic) is never changed by it. A context
/// with no face to append appends nothing, and one with no generic family at
/// all (wasm or an embed before any font is registered) gets its stack back
/// as written (no alternates either), without a lookup.
///
/// fontconfig's substitutes are followed where Chrome follows them (#2336):
/// a missing family is drawn in fontconfig's match when that is a
/// metric-compatible replacement (Liberation Sans for a missing Arial) or the
/// family the configuration puts first (`resolve_family`,
/// `system_substitute`; [`FontDefaults::system_substitutes`] turns it off).
///
/// Cached per thread, keyed by the context's first `sans-serif` and `serif`
/// faces (unique per loaded collection, and changed by a claim on either
/// slot) and the stack, so a layout pays one lookup; [`set_font_defaults`]
/// drops the cache. A family
/// registered after a stack was cached as resolving to nothing leaves the
/// appended face behind the newly found one, which changes nothing it draws;
/// registration cannot make a resolving stack stop resolving.
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
    let sans = collection.generic_families(GenericFamily::SansSerif).next();
    let serif = collection.generic_families(GenericFamily::Serif).next();
    let key = (sans, serif);
    if key == (None, None) {
        // No generic at all (wasm or an embed before any font is registered):
        // used as written, as before #2255, with no scan per call. Not
        // cached, because every such context would share the key.
        return FontFamily::Source(Cow::Owned(stack.to_owned()));
    }
    let cached = STACK_CACHE.with(|cache| {
        cache
            .borrow()
            .get(&key)
            .and_then(|stacks| stacks.get(stack).cloned())
    });
    let finished = match cached {
        Some(finished) => finished,
        None => {
            let finished = finish_stack(collection, stack);
            STACK_CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                if cache.values().map(|m| m.len()).sum::<usize>() >= STACK_CACHE_LIMIT {
                    cache.clear();
                }
                cache
                    .entry(key)
                    .or_default()
                    .insert(stack.to_owned(), finished.clone());
            });
            finished
        }
    };
    FontFamily::Source(Cow::Owned(finished.unwrap_or_else(|| stack.to_owned())))
}

/// [`parley_font_family`]'s answer for a non-empty `stack`, uncached:
/// `Some(rewritten stack)`, or `None` to use it as written.
fn finish_stack(collection: &mut Collection, stack: &str) -> Option<String> {
    let defaults = font_defaults();
    finish_stack_with(collection, stack, &defaults, &mut system_matcher(&defaults))
}

/// [`finish_stack`] with these defaults and this system matcher.
fn finish_stack_with(
    collection: &mut Collection,
    stack: &str,
    defaults: &FontDefaults,
    system: &mut SystemMatcher<'_>,
) -> Option<String> {
    let (with_alternates, resolves) = with_alternates(collection, stack, system);
    if resolves {
        return with_alternates;
    }
    let base = with_alternates.as_deref().unwrap_or(stack);
    default_face(collection, defaults, system).map(|face| format!("{base}, {face}"))
}

/// `stack` with each missing named family that is drawn in another family
/// ([`resolve_family`]: a system substitute or an alternate) replaced by it
/// in its place (`Some` only when one was), and whether the result names a
/// family this collection has. Read the way parley reads it
/// (`parse_css_list`, stopping at the first parse error).
///
/// Past the first family that resolves, a name is checked by the name map
/// alone (`family_id`, which loads no faces) and only a missing one is
/// resolved: the rest cannot change, but a missing one is what a character
/// the first family lacks reaches, and Chrome looks it up the same way.
fn with_alternates(
    collection: &mut Collection,
    stack: &str,
    system: &mut SystemMatcher<'_>,
) -> (Option<String>, bool) {
    use parley::style::FontFamilyName;
    let mut items: Vec<String> = Vec::new();
    let mut changed = false;
    let mut resolves = false;
    for family in FontFamilyName::parse_css_list(stack).map_while(Result::ok) {
        match family {
            FontFamilyName::Named(name) => {
                let mut item = name.to_string();
                if !resolves || collection.family_id(&name).is_none() {
                    if let Some((_, replacement)) = resolve_family(collection, &name, system) {
                        resolves = true;
                        if let Some(replacement) = replacement {
                            changed = true;
                            item = replacement;
                        }
                    }
                }
                items.extend(quote_family(&item));
            }
            FontFamilyName::Generic(generic) => {
                resolves |= collection.generic_families(generic).next().is_some();
                items.push(generic.to_string());
            }
        }
    }
    (changed.then(|| items.join(", ")), resolves)
}

/// The default font a stack that resolves to nothing ends in, as a quoted
/// family name: the first family of this thread's [`FontDefaults::standard`]
/// this collection has (a named family, its system substitute or its
/// alternate, or a generic's first face), else the `sans-serif` slot's first face; `None` if neither exists.
fn default_face(
    collection: &mut Collection,
    defaults: &FontDefaults,
    system: &mut SystemMatcher<'_>,
) -> Option<String> {
    use parley::style::FontFamilyName;
    let mut id = None;
    for family in FontFamilyName::parse_css_list(&defaults.standard).map_while(Result::ok) {
        id = match family {
            FontFamilyName::Named(name) => installed_family(collection, &name, system),
            FontFamilyName::Generic(generic) => collection.generic_families(generic).next(),
        };
        if id.is_some() {
            break;
        }
    }
    let id = id.or_else(|| collection.generic_families(GenericFamily::SansSerif).next())?;
    collection.family_name(id).and_then(quote_family)
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
/// that resolves to nothing when [`FontDefaults::standard`] resolves to
/// nothing too.
pub const STACK_FALLBACK_GENERIC: &str = "sans-serif";

/// Distinct stacks kept per thread before the cache is dropped and rebuilt.
const STACK_CACHE_LIMIT: usize = 4096;

/// The first `sans-serif` and `serif` faces of a context.
type StackCacheKey = (Option<FamilyId>, Option<FamilyId>);

type StackCache =
    std::collections::HashMap<StackCacheKey, std::collections::HashMap<String, Option<String>>>;

thread_local! {
    /// first `sans-serif` and `serif` faces → stack → `Some(finished stack)`
    /// or `None` (use the stack as written).
    static STACK_CACHE: std::cell::RefCell<StackCache> =
        std::cell::RefCell::new(StackCache::new());
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

/// The CSS generic class a face declares for itself, read from its own tables
/// (#2102). What a browser's generic matching keys on, and the only basis
/// [`fill_empty_generics`] puts a face in a slot on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceClass {
    /// A sans-serif text face, or one that declares no class at all.
    SansSerif,
    /// A serif text face.
    Serif,
    /// A fixed-pitch face.
    Monospace,
    /// A face that declares itself something no text generic means: script,
    /// handwritten, decorative, ornamental or symbol.
    Display,
}

impl FaceClass {
    /// The generic slots a face of this class can fill when they are empty:
    /// the plain generic first, then its companions, which are filled only
    /// together with it (see [`fill_empty_generics`]). `Display` fills none.
    pub fn generics(self) -> &'static [GenericFamily] {
        match self {
            // `system-ui` with it: a theme's stack that names no family ends
            // in one of the two, and on wasm there is nothing else for either
            // to mean (what `register_font_data` always claimed).
            FaceClass::SansSerif => &[
                GenericFamily::SansSerif,
                GenericFamily::SystemUi,
                GenericFamily::UiSansSerif,
            ],
            FaceClass::Serif => &[GenericFamily::Serif, GenericFamily::UiSerif],
            FaceClass::Monospace => &[GenericFamily::Monospace, GenericFamily::UiMonospace],
            FaceClass::Display => &[],
        }
    }
}

/// The class face `index` of the font file `data` declares; see the match
/// below for the order. [`FaceClass::SansSerif`] when it declares none (or
/// cannot be read).
///
/// Not a text face at all, first (review of PR #2122): a face with color
/// glyph tables (`COLR`, `CBDT`, `sbix`, `SVG `) is an emoji face, which
/// Noto Color Emoji shows also *declares* itself fixed pitch (PANOSE
/// proportion 9, `isFixedPitch` 1); and a face whose cmap has neither `A`
/// nor `a` is an icon or symbol face (FontAwesome, Noto Sans Symbols), which
/// declares no class at all. Both are [`FaceClass::Display`].
///
/// Then fixed pitch, so a monospaced slab serif is `Monospace`. Then the IBM
/// class: 1–5 and 7 are serif classes, 8 sans serif, 9 ornamental, 10
/// script and 12 symbolic. Then PANOSE, whose family kind is 2 for Latin text
/// (its serif-style digit 11–15 is a sans, 2–10 a serif; its proportion 9 is
/// monospaced) and 3, 4 and 5 for handwritten, decorative and symbol faces.
/// Many text fonts leave both at 0 ("no classification"), and those are
/// treated as sans serif, the class every face registered as data had before
/// #2102.
pub fn classify_face(data: &[u8], index: u32) -> FaceClass {
    use skrifa::MetadataProvider;
    use skrifa::raw::TableProvider;
    use skrifa::raw::types::Tag;
    let Ok(face) = skrifa::FontRef::from_index(data, index) else {
        return FaceClass::SansSerif;
    };
    let color = [b"COLR", b"CBDT", b"sbix", b"SVG "]
        .into_iter()
        .any(|tag| face.table_data(Tag::new(tag)).is_some());
    let charmap = face.charmap();
    if color || (charmap.map('A').is_none() && charmap.map('a').is_none()) {
        return FaceClass::Display;
    }
    if face.post().is_ok_and(|post| post.is_fixed_pitch() != 0) {
        return FaceClass::Monospace;
    }
    let Ok(os2) = face.os2() else {
        return FaceClass::SansSerif;
    };
    match (os2.s_family_class() >> 8) & 0xff {
        1..=5 | 7 => return FaceClass::Serif,
        8 => return FaceClass::SansSerif,
        9 | 10 | 12 => return FaceClass::Display,
        _ => {}
    }
    let panose = os2.panose_10();
    match panose.first().copied() {
        Some(2) => match (panose.get(3).copied(), panose.get(1).copied()) {
            (Some(9), _) => FaceClass::Monospace,
            (_, Some(2..=10)) => FaceClass::Serif,
            _ => FaceClass::SansSerif,
        },
        Some(3..=5) => FaceClass::Display,
        _ => FaceClass::SansSerif,
    }
}

/// Put each family registered from `data` into the generic slots its class
/// can fill ([`classify_face`], [`FaceClass::generics`]), **only where the
/// slot is empty** (#2102).
///
/// For font data an app hands over without saying what it is
/// (`RinchApp::register_font_data`, the embed's fonts): on a platform with no
/// fonts of its own (wasm, an embed with none), `font-family: sans-serif` has
/// to mean something, and the first face of the right class is it. A slot
/// that is already filled — the platform's mapping on a desktop, or an
/// earlier face — is left as it is, so a later registration never changes
/// what a generic means. It used to prepend every face to `sans-serif`, so
/// the last one registered, a script face as easily as a text face, became
/// what every `sans-serif` element drew in. A face meant for a slot that is
/// already filled is claimed explicitly (`AppFont::sans_serif`,
/// [`claim_generic_families`]).
///
/// The companions of a plain generic (`system-ui` and `ui-sans-serif` for
/// `sans-serif`, `ui-serif`, `ui-monospace`) are filled only together with
/// it — when the plain slot was empty too. fontconfig maps the plain
/// generics and `system-ui` but no `ui-*` one, so filling an empty `ui-*`
/// slot on a desktop changed what the theme's monospace stack, which starts
/// with `ui-monospace`, drew in (review of PR #2122). So on a desktop data
/// fills nothing.
pub fn fill_empty_generics(
    collection: &mut Collection,
    data: &[u8],
    families: &[(FamilyId, Vec<parley::fontique::FontInfo>)],
) {
    for (family, fonts) in families {
        let index = fonts.first().map_or(0, |font| font.index());
        let generics = classify_face(data, index).generics();
        let Some(plain) = generics.first() else {
            continue;
        };
        if collection.generic_families(*plain).next().is_some() {
            continue;
        }
        for generic in generics {
            if collection.generic_families(*generic).next().is_none() {
                collection.append_generic_families(*generic, core::iter::once(*family));
            }
        }
    }
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
    /// What font data fills (#2102, review of PR #2122): a color-emoji face
    /// and a face with no basic Latin are not text faces, and on a platform
    /// that maps a plain generic its `ui-*` and `system-ui` companions are
    /// left alone even where the platform leaves them empty.
    mod data_fill {
        use super::super::{FaceClass, classify_face, fill_empty_generics};
        use parley::fontique::{
            Blob, Collection, CollectionOptions, FamilyId, FontInfoOverride, GenericFamily,
        };

        const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
        /// Noto Color Emoji cut to U+1F600: `CBDT`/`CBLC` bitmaps,
        /// `post.isFixedPitch` 1 and PANOSE proportion 9 — it *declares*
        /// itself monospaced.
        const EMOJI: &[u8] = include_bytes!("../assets/fonts/NotoColorEmoji-U1F600.ttf");

        /// The offset of table `tag`'s record in the table directory.
        fn record(data: &[u8], tag: &[u8; 4]) -> usize {
            let count = u16::from_be_bytes([data[4], data[5]]) as usize;
            (0..count)
                .map(|i| 12 + 16 * i)
                .find(|r| &data[*r..*r + 4] == tag)
                .expect("table present")
        }

        fn table(data: &[u8], tag: &[u8; 4]) -> usize {
            let r = record(data, tag);
            u32::from_be_bytes(data[r + 8..r + 12].try_into().unwrap()) as usize
        }

        /// `post.isFixedPitch` set to `fixed`.
        fn fixed_pitch(mut data: Vec<u8>, fixed: bool) -> Vec<u8> {
            let post = table(&data, b"post");
            data[post + 12..post + 16].copy_from_slice(&u32::from(fixed).to_be_bytes());
            data
        }

        /// The emoji face with nothing color about it and no declared class:
        /// `CBDT` renamed `CBDA` (still sorted ahead of `CBLC`), not fixed
        /// pitch, IBM class and PANOSE zero. What is left is a face whose
        /// cmap has no Latin — an icon or symbol font's shape.
        fn symbol_face() -> Vec<u8> {
            let mut data = fixed_pitch(EMOJI.to_vec(), false);
            let r = record(&data, b"CBDT");
            data[r..r + 4].copy_from_slice(b"CBDA");
            let os2 = table(&data, b"OS/2");
            data[os2 + 30..os2 + 42].fill(0);
            data
        }

        fn bare() -> Collection {
            Collection::new(CollectionOptions {
                shared: false,
                system_fonts: false,
            })
        }

        /// Register `data` as family `name` and fill as `register_font_data`
        /// does; the family's id.
        fn register(collection: &mut Collection, data: Vec<u8>, name: &str) -> FamilyId {
            let families = collection.register_fonts(
                Blob::new(std::sync::Arc::new(data.clone())),
                Some(FontInfoOverride {
                    family_name: Some(name),
                    ..Default::default()
                }),
            );
            fill_empty_generics(collection, &data, &families);
            families[0].0
        }

        fn slot(collection: &mut Collection, generic: GenericFamily) -> Vec<FamilyId> {
            collection.generic_families(generic).collect()
        }

        #[test]
        fn a_color_emoji_face_is_not_monospace() {
            assert_eq!(classify_face(EMOJI, 0), FaceClass::Display);
        }

        /// A color table alone makes a face an emoji face, Latin or not
        /// (color emoji fonts commonly map the ASCII digits and `#`): Inter
        /// with its `GDEF` table renamed `COLR`, which keeps the directory
        /// sorted.
        #[test]
        fn a_face_with_a_color_table_is_display_even_with_latin() {
            let mut data = INTER.to_vec();
            let r = record(&data, b"GDEF");
            data[r..r + 4].copy_from_slice(b"COLR");
            assert_eq!(classify_face(&data, 0), FaceClass::Display);
            assert_eq!(classify_face(INTER, 0), FaceClass::SansSerif, "control");
        }

        #[test]
        fn a_face_with_no_basic_latin_is_not_a_text_face() {
            assert_eq!(classify_face(&symbol_face(), 0), FaceClass::Display);
            // Positive control: the same rewrite on a Latin face is sans.
            assert_eq!(
                classify_face(&fixed_pitch(INTER.to_vec(), false), 0),
                FaceClass::SansSerif
            );
        }

        /// wasm shape: an emoji face registered before a monospaced text face
        /// leaves `monospace` to the text face; an icon face registered
        /// before a sans face leaves `sans-serif` to it.
        #[test]
        fn emoji_and_symbol_faces_leave_empty_slots_to_text_faces() {
            let mut collection = bare();
            register(&mut collection, EMOJI.to_vec(), "Emoji");
            register(&mut collection, symbol_face(), "Icons");
            let mono = register(&mut collection, fixed_pitch(INTER.to_vec(), true), "Mono");
            let sans = register(&mut collection, INTER.to_vec(), "Sans");
            assert_eq!(slot(&mut collection, GenericFamily::Monospace), [mono]);
            assert_eq!(slot(&mut collection, GenericFamily::UiMonospace), [mono]);
            assert_eq!(slot(&mut collection, GenericFamily::SansSerif), [sans]);
        }

        /// Desktop shape: the platform maps `sans-serif`, `serif` and
        /// `monospace` (fontconfig does) but not their `ui-*` forms. Data
        /// fills none of them: the theme's monospace stack starts with
        /// `ui-monospace`, so filling it switched every code element's face.
        #[test]
        fn companions_stay_empty_where_the_platform_maps_the_plain_generic() {
            let mut collection = bare();
            // The platform's face: registered, not filled.
            let platform = collection.register_fonts(
                Blob::new(std::sync::Arc::new(INTER)),
                Some(FontInfoOverride {
                    family_name: Some("Platform"),
                    ..Default::default()
                }),
            )[0]
            .0;
            for generic in [
                GenericFamily::SansSerif,
                GenericFamily::Serif,
                GenericFamily::Monospace,
            ] {
                collection.set_generic_families(generic, core::iter::once(platform));
            }
            register(&mut collection, fixed_pitch(INTER.to_vec(), true), "Mono");
            register(&mut collection, INTER.to_vec(), "Sans");
            for generic in [
                GenericFamily::UiMonospace,
                GenericFamily::UiSansSerif,
                GenericFamily::SystemUi,
            ] {
                assert!(slot(&mut collection, generic).is_empty(), "{generic:?}");
            }
            assert_eq!(slot(&mut collection, GenericFamily::Monospace), [platform]);
        }
    }

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

        /// A context whose `sans-serif` is `Sans2255` and `serif` is
        /// `Serif2255`, so the default font is visible against both.
        fn context_with_generics() -> parley::FontContext {
            let mut cx = context();
            let sans = register(&mut cx, "Sans2255");
            let serif = register(&mut cx, "Serif2255");
            cx.collection
                .set_generic_families(GenericFamily::SansSerif, core::iter::once(sans));
            cx.collection
                .set_generic_families(GenericFamily::Serif, core::iter::once(serif));
            cx
        }

        #[test]
        fn an_exhausted_stack_ends_in_times_new_roman_as_in_chrome() {
            // #2255: Chrome draws `font-family: Oswald` (not installed) in its
            // standard font, Times New Roman; rinch drew the sans face.
            let mut cx = context_with_generics();
            register(&mut cx, "Times New Roman");
            assert_eq!(
                source(parley_font_family(&mut cx, "Oswald")),
                r#"Oswald, "Times New Roman""#
            );
        }

        #[test]
        fn without_times_an_exhausted_stack_ends_in_the_sans_serif_face() {
            // Not the serif face: on a host with no Times New Roman, Chrome
            // draws such a stack in fontconfig's "Sans" (DejaVu Sans,
            // 609.94px), never in its serif setting (review of PR #2337).
            let mut cx = context_with_generics();
            assert_eq!(
                source(parley_font_family(&mut cx, "Oswald, NoSuchFamilyX")),
                r#"Oswald, NoSuchFamilyX, "Sans2255""#
            );
            // The standard font's alternate counts: Times stands for Times
            // New Roman.
            let mut cx = context_with_generics();
            register(&mut cx, "Times");
            assert_eq!(
                source(parley_font_family(&mut cx, "Oswald")),
                r#"Oswald, "Times""#
            );
        }

        #[test]
        fn the_standard_font_is_the_apps_to_set() {
            let mut cx = context_with_generics();
            register(&mut cx, "Times New Roman");
            register(&mut cx, "AppFace2255");
            crate::fonts::set_font_defaults(
                crate::fonts::FontDefaults::default()
                    .with_standard("NotHere2255, AppFace2255, serif"),
            );
            assert_eq!(
                source(parley_font_family(&mut cx, "Oswald")),
                r#"Oswald, "AppFace2255""#
            );
            // A generic in it means that slot's first face.
            crate::fonts::set_font_defaults(
                crate::fonts::FontDefaults::default().with_standard("sans-serif"),
            );
            assert_eq!(
                source(parley_font_family(&mut cx, "Oswald")),
                r#"Oswald, "Sans2255""#
            );
            // One that resolves to nothing falls to the `sans-serif` face.
            crate::fonts::set_font_defaults(
                crate::fonts::FontDefaults::default().with_standard("NotHere2255"),
            );
            assert_eq!(
                source(parley_font_family(&mut cx, "Oswald")),
                r#"Oswald, "Sans2255""#
            );
            crate::fonts::set_font_defaults(Default::default());
        }

        #[test]
        fn a_missing_family_is_replaced_by_its_alternate_in_place() {
            // #2250: Chrome draws a missing Helvetica in Arial, ahead of the
            // next family in the list.
            let mut cx = context_with_generics();
            register(&mut cx, "Arial");
            register(&mut cx, "Tahoma2250");
            assert_eq!(
                source(parley_font_family(
                    &mut cx,
                    "Helvetica Neue, Helvetica, Tahoma2250, sans-serif"
                )),
                r#""Helvetica Neue", "Arial", "Tahoma2250", sans-serif"#
            );
            // ASCII case-insensitive, as Blink's table is.
            assert_eq!(
                source(parley_font_family(&mut cx, "helvetica")),
                r#""Arial""#
            );
        }

        #[test]
        fn each_alternate_pair_goes_both_ways() {
            for (asked, installed) in [
                ("Helvetica", "Arial"),
                ("Arial", "Helvetica"),
                ("Times", "Times New Roman"),
                ("Times New Roman", "Times"),
                ("Courier", "Courier New"),
                #[cfg(not(target_os = "windows"))]
                ("Courier New", "Courier"),
            ] {
                let mut cx = context_with_generics();
                register(&mut cx, installed);
                assert_eq!(
                    source(parley_font_family(&mut cx, &format!("'{asked}', Gone2250"))),
                    format!("\"{installed}\", \"Gone2250\""),
                    "{asked}"
                );
            }
        }

        #[test]
        fn an_installed_family_or_a_missing_alternate_is_left_alone() {
            let mut cx = context_with_generics();
            register(&mut cx, "Helvetica");
            register(&mut cx, "Arial");
            // Both installed: Helvetica is drawn as Helvetica.
            assert_eq!(
                source(parley_font_family(&mut cx, "Helvetica, x")),
                "Helvetica, x"
            );
            // A missing family whose alternate is missing too is written
            // as it was, ahead of the default font.
            let mut cx = context_with_generics();
            assert_eq!(
                source(parley_font_family(&mut cx, "Courier, sans-serif")),
                "Courier, sans-serif"
            );
            // An alternate after the first family that resolves is still
            // substituted: it is what a character the first lacks reaches.
            let mut cx = context_with_generics();
            register(&mut cx, "Arial");
            assert_eq!(
                source(parley_font_family(&mut cx, "Sans2255, Helvetica")),
                r#""Sans2255", "Arial""#
            );
        }

        #[test]
        fn the_generic_defaults_head_their_slots_where_installed() {
            use crate::fonts::{FontDefaults, apply_generic_defaults};
            let mut cx = context_with_generics();
            let sans = cx
                .collection
                .generic_families(GenericFamily::SansSerif)
                .next();
            let serif = cx.collection.generic_families(GenericFamily::Serif).next();
            let tnr = register(&mut cx, "Times New Roman");
            apply_generic_defaults(&mut cx.collection, &FontDefaults::default());
            let slot = |cx: &mut parley::FontContext, g| -> Vec<_> {
                cx.collection.generic_families(g).collect()
            };
            // Chrome's `serif` is Times New Roman, ahead of the platform's;
            // no Arial here, so `sans-serif` keeps the platform's.
            assert_eq!(
                slot(&mut cx, GenericFamily::Serif),
                vec![tnr, serif.unwrap()]
            );
            assert_eq!(slot(&mut cx, GenericFamily::SansSerif), vec![sans.unwrap()]);
            // Idempotent: the head is not added twice.
            apply_generic_defaults(&mut cx.collection, &FontDefaults::default());
            assert_eq!(
                slot(&mut cx, GenericFamily::Serif),
                vec![tnr, serif.unwrap()]
            );
            // The alternate counts, and `None` leaves a slot alone.
            let mut cx = context_with_generics();
            let sans = cx
                .collection
                .generic_families(GenericFamily::SansSerif)
                .next();
            let helvetica = register(&mut cx, "Helvetica");
            register(&mut cx, "Times New Roman");
            let serif = cx.collection.generic_families(GenericFamily::Serif).next();
            apply_generic_defaults(
                &mut cx.collection,
                &FontDefaults::default().with_serif(None),
            );
            assert_eq!(
                slot(&mut cx, GenericFamily::SansSerif),
                vec![helvetica, sans.unwrap()]
            );
            assert_eq!(slot(&mut cx, GenericFamily::Serif), vec![serif.unwrap()]);
        }
    }

    /// Chrome's fontconfig substitutes (#2336), on bare collections with a
    /// stand-in for fontconfig: host-independent. The fontconfig query itself
    /// is pinned by `tests/fontconfig_metric_aliases_2336.rs`.
    mod system_substitutes {
        use super::super::{
            FontDefaults, SystemMatch, apply_generic_defaults_with, finish_stack_with, gated,
            metric_compatible, system_substitute,
        };
        use parley::fontique::{
            Blob, Collection, CollectionOptions, FamilyId, FontInfoOverride, GenericFamily,
        };

        const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

        fn answer(configured: &str, matched: &[&str]) -> SystemMatch {
            SystemMatch {
                configured: configured.to_owned(),
                matched: matched.iter().map(|m| (*m).to_owned()).collect(),
            }
        }

        #[test]
        fn chromes_metric_classes() {
            for (a, b) in [
                ("Arial", "Liberation Sans"),
                ("Arial", "Arimo"),
                ("liberation sans", "ARIAL"),
                ("Times New Roman", "Tinos"),
                ("Times New Roman", "Liberation Serif"),
                ("Courier New", "Cousine"),
                ("Liberation Mono", "Courier New"),
                ("Calibri", "Carlito"),
                ("Cambria", "Caladea"),
                ("Symbol", "Symbol Neu"),
                ("MS PGothic", "Noto Sans CJK JP"),
                (
                    "\u{ff2d}\u{ff33} \u{ff30}\u{30b4}\u{30b7}\u{30c3}\u{30af}",
                    "MS PGothic",
                ),
                ("MS PMincho", "Noto Serif CJK JP"),
                ("Simhei", "Noto Sans CJK SC"),
            ] {
                assert!(metric_compatible(a, b), "{a} ~ {b}");
            }
            for (a, b) in [
                // Not in the same class.
                ("Arial", "Liberation Serif"),
                ("Times New Roman", "Arimo"),
                ("Courier New", "Liberation Sans"),
                // Not in Chrome's table, though fontconfig aliases them.
                ("Helvetica", "Liberation Sans"),
                ("Times", "Liberation Serif"),
                ("Georgia", "Gelasio"),
                ("Tahoma", "DejaVu Sans"),
                // A family in no class is no one's replacement, even its own.
                ("Tahoma", "Tahoma"),
                // As in Skia, a name in two classes is judged by its first:
                // Noto Serif CJK JP is MS PMincho's, so not MS Mincho's.
                ("MS Mincho", "Noto Serif CJK JP"),
            ] {
                assert!(!metric_compatible(a, b), "{a} !~ {b}");
            }
        }

        #[test]
        fn chrome_accepts_a_match_only_for_these_three_reasons() {
            // A metric-compatible replacement.
            let arial = answer("Arial", &["Liberation Sans"]);
            assert_eq!(system_substitute("Arial", &arial), Some("Liberation Sans"));
            // Any of the match's names counts.
            let two = answer("Arial", &["Something", "Arimo"]);
            assert_eq!(system_substitute("Arial", &two), Some("Arimo"));
            // The family the configuration put first (a prepended alias).
            let prepended = answer("DejaVu Serif", &["DejaVu Serif"]);
            assert_eq!(system_substitute("Probe", &prepended), Some("DejaVu Serif"));
            // The name itself.
            let itself = answer("Other", &["probe"]);
            assert_eq!(system_substitute("Probe", &itself), Some("probe"));
            // Not fontconfig's best guess: an appended alias, a fontconfig
            // alias outside Chrome's table, a replacement that is missing.
            let appended = answer("Probe", &["DejaVu Serif"]);
            assert_eq!(system_substitute("Probe", &appended), None);
            let helvetica = answer("Helvetica", &["Nimbus Sans"]);
            assert_eq!(system_substitute("Helvetica", &helvetica), None);
            let calibri = answer("Calibri", &["Noto Sans"]);
            assert_eq!(system_substitute("Calibri", &calibri), None);
            // `sans`, `serif` and `monospace` as names accept anything.
            let mono = answer("monospace", &["DejaVu Sans Mono"]);
            for name in ["Monospace", "sans", "SERIF", ""] {
                assert_eq!(system_substitute(name, &mono), Some("DejaVu Sans Mono"));
            }
            assert_eq!(system_substitute("Arial", &answer("Arial", &[])), None);
        }

        fn register(collection: &mut Collection, name: &str) -> FamilyId {
            collection.register_fonts(
                Blob::new(std::sync::Arc::new(FACE)),
                Some(FontInfoOverride {
                    family_name: Some(name),
                    ..Default::default()
                }),
            )[0]
            .0
        }

        /// A host with Liberation and Nimbus but no msttcorefonts, and a
        /// stand-in for its fontconfig: the answers fontconfig gave for these
        /// names on such a host (`FcFontMatch`, Chrome's query).
        fn liberation_host() -> Collection {
            let mut collection = Collection::new(CollectionOptions {
                shared: false,
                system_fonts: false,
            });
            let sans = register(&mut collection, "Sans2336");
            let serif = register(&mut collection, "Serif2336");
            for name in [
                "Liberation Sans",
                "Liberation Serif",
                "Liberation Mono",
                "Nimbus Sans",
                "Nimbus Roman",
                "Noto Sans CJK JP",
            ] {
                register(&mut collection, name);
            }
            collection.set_generic_families(GenericFamily::SansSerif, core::iter::once(sans));
            collection.set_generic_families(GenericFamily::Serif, core::iter::once(serif));
            collection
        }

        fn fontconfig(_: &mut Collection, name: &str) -> Option<SystemMatch> {
            let (configured, matched) = match name.to_ascii_lowercase().as_str() {
                "arial" | "arimo" => ("Arial", "Liberation Sans"),
                "times new roman" => ("Times New Roman", "Liberation Serif"),
                "courier new" => ("Courier New", "Liberation Mono"),
                "helvetica" => ("Helvetica", "Nimbus Sans"),
                "times" => ("Times", "Nimbus Roman"),
                "ms pgothic" => ("MS PGothic", "Noto Sans CJK JP"),
                // Everything else falls to fontconfig's sans face.
                _ => return Some(answer(name, &["Sans2336"])),
            };
            Some(answer(configured, &[matched]))
        }

        fn finish(collection: &mut Collection, stack: &str, defaults: &FontDefaults) -> String {
            finish_stack_with(collection, stack, defaults, &mut fontconfig)
                .unwrap_or_else(|| stack.to_owned())
        }

        #[test]
        fn a_missing_family_is_drawn_in_its_metric_replacement_in_place() {
            let mut host = liberation_host();
            let chrome = FontDefaults::default();
            for (stack, finished) in [
                ("Arial, Tahoma2336", r#""Liberation Sans", "Tahoma2336""#),
                ("Arimo, Tahoma2336", r#""Liberation Sans", "Tahoma2336""#),
                ("'Times New Roman', x", r#""Liberation Serif", "x""#),
                ("'Courier New', x", r#""Liberation Mono", "x""#),
                ("'MS PGothic', x", r#""Noto Sans CJK JP", "x""#),
                // Helvetica: fontconfig's Nimbus Sans is not acceptable, so
                // Chrome tries the alternate, Arial, and takes its match.
                (
                    "Helvetica, Tahoma2336",
                    r#""Liberation Sans", "Tahoma2336""#,
                ),
                ("Times, x", r#""Liberation Serif", "x""#),
                // Past the first family that resolves, a missing family is
                // replaced too: a character the first lacks reaches it.
                ("Sans2336, Arial", r#""Sans2336", "Liberation Sans""#),
                // An installed family is left alone.
                (
                    "'Liberation Sans', Arial",
                    r#""Liberation Sans", "Liberation Sans""#,
                ),
                ("Sans2336, x", "Sans2336, x"),
            ] {
                assert_eq!(finish(&mut host, stack, &chrome), finished, "{stack}");
            }
        }

        #[test]
        fn the_substitute_comes_before_the_alternate() {
            // Blink asks for the alternate only when the name itself finds
            // nothing, and on Linux finding the name includes fontconfig's
            // acceptable match: with Helvetica and Liberation Sans installed
            // and no Arial, Arial is Liberation Sans, not Helvetica.
            let mut host = liberation_host();
            register(&mut host, "Helvetica");
            assert_eq!(
                finish(&mut host, "Arial, x", &FontDefaults::default()),
                r#""Liberation Sans", "x""#
            );
        }

        #[test]
        fn an_unacceptable_match_falls_through_to_the_next_family() {
            let mut host = liberation_host();
            let chrome = FontDefaults::default();
            // fontconfig answers Sans2336 for Calibri; Chrome moves on.
            assert_eq!(
                finish(&mut host, "Calibri, sans-serif", &chrome),
                "Calibri, sans-serif"
            );
            // A stack of such names ends in the default font, Times New
            // Roman, which is Liberation Serif here (Chrome 156).
            assert_eq!(
                finish(&mut host, "Oswald", &chrome),
                r#"Oswald, "Liberation Serif""#
            );
        }

        #[test]
        fn the_app_can_turn_system_substitutes_off() {
            let mut host = liberation_host();
            for (defaults, arial, oswald) in [
                (
                    FontDefaults::default(),
                    r#""Liberation Sans", "x""#,
                    r#"Oswald, "Liberation Serif""#,
                ),
                // Off: Arial is missing, so the list ends in the default
                // font, and with no Times New Roman that is the sans face.
                (
                    FontDefaults::default().with_system_substitutes(false),
                    r#"Arial, x, "Sans2336""#,
                    r#"Oswald, "Sans2336""#,
                ),
            ] {
                let mut matcher = gated(&defaults, fontconfig);
                for (stack, finished) in [("Arial, x", arial), ("Oswald", oswald)] {
                    let got = finish_stack_with(&mut host, stack, &defaults, &mut matcher)
                        .unwrap_or_else(|| stack.to_owned());
                    assert_eq!(got, finished, "{stack}, {}", defaults.system_substitutes);
                }
            }
        }

        #[test]
        fn the_generic_defaults_take_their_substitutes() {
            let mut host = liberation_host();
            let sans = host.generic_families(GenericFamily::SansSerif).next();
            let serif = host.generic_families(GenericFamily::Serif).next();
            apply_generic_defaults_with(&mut host, &FontDefaults::default(), &mut fontconfig);
            let head = |host: &mut Collection, g| {
                let id = host.generic_families(g).next().unwrap();
                host.family_name(id).unwrap().to_owned()
            };
            // Chrome's `serif` (Times New Roman) and `sans-serif` (Arial) on
            // this host: Liberation Serif and Liberation Sans (Chrome 156).
            assert_eq!(head(&mut host, GenericFamily::Serif), "Liberation Serif");
            assert_eq!(head(&mut host, GenericFamily::SansSerif), "Liberation Sans");
            assert_eq!(host.generic_families(GenericFamily::Serif).nth(1), serif);
            assert_eq!(host.generic_families(GenericFamily::SansSerif).nth(1), sans);
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
