//! Shared class-name helpers for components (issue #474).
//!
//! One helper so far, [`radius_class`].
//!
//! Twenty-one components take a `radius: String` prop naming a step of the
//! theme's radius scale, and every one of them used to hand-roll the identical
//! five-arm `match self.radius.as_str()` over its own class prefix. Five of them
//! — `Badge`, `NumberInput`, `PasswordInput`, `Tabs` and `TextInput` (#457) —
//! forgot to write the match at all, so the prop was declared, documented, and
//! read by nothing. Five copies of a thing is a habit; twenty is a helper that
//! was missing.
//!
//! The CSS half stays per component, because the class has to *land* somewhere
//! different in each one: on the box itself (`Badge`, `CloseButton`), on the
//! field inside a wrapper (`TextInput`), on every tab button (`Tabs`).
//!
//! **The module is `class_utils` and not `radius` on purpose.** `rinch-theme`
//! declares its own `pub mod radius`, and the `rinch` facade globs both
//! `rinch_theme::*` and `rinch_components::*` into one prelude — two modules of
//! the same name there is an `ambiguous_glob_reexports` warning, which is fatal
//! only under CI's `-D warnings` and invisible to a single-crate clippy run.
//! Renaming this module back would rediscover that in CI.

/// The five steps of the theme's radius scale, in ascending order.
///
/// These are the values a `radius` prop accepts; `--rinch-radius-{step}` is the
/// CSS variable each resolves to.
pub const RADIUS_SCALE: [&str; 5] = ["xs", "sm", "md", "lg", "xl"];

/// The radius modifier class for a component whose base class is `prefix`, or
/// `None` when `radius` names no step of [`RADIUS_SCALE`].
///
/// The empty string means "unset" throughout this crate's string props, and so
/// yields `None` here — an unset radius must leave the stylesheet's own
/// `border-radius` in place rather than override it with a default.
///
/// Matching is exact and case-sensitive, which is what the twenty hand-rolled
/// matches this replaces all did; `size` props parse case-insensitively through
/// `FromStr`, and reconciling the two is a behaviour change rather than this
/// consolidation.
///
/// ```
/// use rinch_components::class_utils::radius_class;
/// assert_eq!(
///     radius_class("rinch-badge", "md").as_deref(),
///     Some("rinch-badge--radius-md")
/// );
/// assert_eq!(radius_class("rinch-badge", ""), None);
/// assert_eq!(radius_class("rinch-badge", "enormous"), None);
/// ```
pub fn radius_class(prefix: &str, radius: &str) -> Option<String> {
    RADIUS_SCALE
        .contains(&radius)
        .then(|| format!("{prefix}--radius-{radius}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scale_step_yields_its_own_class() {
        for step in RADIUS_SCALE {
            assert_eq!(
                radius_class("rinch-thing", step).as_deref(),
                Some(format!("rinch-thing--radius-{step}").as_str())
            );
        }
    }

    #[test]
    fn an_unset_or_unknown_radius_yields_nothing() {
        for value in ["", " ", "MD", "md ", "none", "2rem", "xxl"] {
            assert_eq!(
                radius_class("rinch-thing", value),
                None,
                "{value:?} is not a step of the scale"
            );
        }
    }
}
