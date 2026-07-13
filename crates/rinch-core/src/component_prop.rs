//! `DefaultProp` — the default used by `#[component]`-generated props structs.
//!
//! A PascalCase `#[component]` fills any prop the caller omits via
//! `..Default::default()`, so every prop type needs a default. For prop types
//! the macro doesn't special-case, its generated `Default` impl defaults the
//! field through this trait, so a prop type that forgets `Default` produces a
//! rinch-specific, actionable message (via `#[diagnostic::on_unimplemented]`)
//! pointing at the offending field — instead of a bare `E0277` deep inside
//! generated code.

/// Blanket-implemented for every `T: Default`. The `#[component]` macro calls
/// `<T as DefaultProp>::default_prop()` for prop types it doesn't recognize, so
/// a non-`Default` prop type fails with a clear, framework-specific error.
#[diagnostic::on_unimplemented(
    message = "`#[component]` prop type `{Self}` must implement `Default`",
    label = "this prop's type needs `Default`",
    note = "a `#[component]` fills omitted props via `..Default::default()`, so every prop type needs a default",
    note = "add `#[derive(Default)]` to the type — for an enum, put `#[default]` on one unit variant"
)]
pub trait DefaultProp: Sized {
    /// Produce the default value for a component prop of this type.
    fn default_prop() -> Self;
}

impl<T: Default> DefaultProp for T {
    #[inline]
    fn default_prop() -> Self {
        T::default()
    }
}
