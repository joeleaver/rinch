//! Which owner an app's handler runs under (issues #3027, #3030).
//!
//! The rule: **a handler runs under the owner of the code that wrote it**, so a
//! task, timer or signal it creates lives as long as that code's component and
//! ends with it. Two places carry it out:
//!
//! - [`CreatorOwner`]: every app-callback kind captures the owner ambient where
//!   it was created, and runs under it.
//! - [`CallerHandler`]: `rsx!` code that the *caller* wrote but that runs
//!   inside one render of a re-rendering component (that component's `on*`
//!   props, and the handlers among its children) is created under the
//!   caller's owner rather than under that render's, unless it holds what that
//!   render made.

use crate::element::IntoEventHandler;
use crate::events::{KeyEventData, ScrollEvent, WheelEvent};
use crate::file::LocalFile;

use super::scope::{Footprint, Owner, current_owner};

/// The owner an app callback runs under: the one ambient where the callback
/// was **created** (issue #3027).
///
/// Every app-callback kind (`Callback`, `ValueCallback`, `InputCallback`,
/// `FileDropCallback`, `ScrollCallback`) captures one in its `new` and runs its
/// function under it. So a task, timer or signal a handler creates belongs to
/// the component that wrote the handler, whose signals the handler captured —
/// not to the scope of whichever component the callback was handed to, which
/// for a component re-rendered by a reactive prop is one render of it, gone
/// when the handler's own write re-renders it.
///
/// No live owner at creation (app lifetime, or [`unowned`](super::unowned))
/// captures nothing, and the callback runs under whatever is ambient when it is
/// invoked: the owner of the handler that dispatched it. A creator that has
/// unmounted since is treated the same way, rather than pushed as "no owner",
/// so what the callback creates still ends with a live component instead of
/// living for the rest of the program.
///
/// Holds an [`Owner`], a `Weak`: a callback stored anywhere never keeps its
/// creator's scope alive.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct CreatorOwner(Option<Owner>);

impl CreatorOwner {
    /// The live ambient owner now, if there is one.
    pub fn capture() -> Self {
        Self(current_owner())
    }

    /// Run `f` under the captured owner while it is alive, otherwise under
    /// whatever owner is ambient.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        match &self.0 {
            Some(owner) if owner.is_alive() => owner.run(f),
            _ => f(),
        }
    }
}

impl std::fmt::Debug for CreatorOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("CreatorOwner").field(&self.0).finish()
    }
}

/// A handler the **caller** wrote that is built inside one render of a
/// re-rendering component (issue #3030): what `rsx!` wraps such a handler in.
///
/// A component with a reactive prop is rebuilt, struct and children, inside a
/// scope of its own for each render, disposed by the next render. The `on*`
/// props on it, and the element handlers and component props among its
/// children up to the next `for`/`if`/`match`, are not that render's code: the
/// caller wrote them, so they run under the caller's owner
/// ([`RenderScope::caller_owner`](crate::dom::RenderScope::caller_owner)), and
/// a task one spawns is not dropped by the re-render its own write causes.
///
/// Unless the handler holds what the render made. Building the handler's
/// expression is the render's code, and what it creates (`{ let s =
/// Signal::new(0); move || .. }`) belongs to the render and is freed by the
/// next one; a handler running under the caller's owner could start a task
/// that outlives the signal it reads. So [`begin`](Self::begin) takes the
/// ambient owner's footprint before the expression is evaluated (a method
/// receiver is evaluated before its arguments), and when evaluating it changed
/// the footprint, the handler stays the render's.
///
/// The handler is still **registered** in the render's scope either way, so
/// its registry entry is freed with that render.
#[doc(hidden)]
#[must_use]
pub struct CallerHandler {
    caller: CreatorOwner,
    /// The ambient owner when the handler's expression began, and its
    /// footprint then.
    render: Option<(Owner, Footprint)>,
}

impl CallerHandler {
    /// Begin building a handler the caller wrote, to run under `caller`.
    pub fn begin(caller: &CreatorOwner) -> Self {
        let render = current_owner().and_then(|owner| {
            let footprint = owner.footprint()?;
            Some((owner, footprint))
        });
        Self {
            caller: caller.clone(),
            render,
        }
    }

    /// The owner the finished handler runs under: the caller's, or nothing
    /// pushed (the registration's owner, the render's) when building the
    /// expression created something under the render.
    fn owner(self) -> CreatorOwner {
        match self.render {
            Some((render, before)) if render.footprint() != Some(before) => CreatorOwner::default(),
            _ => self.caller,
        }
    }

    /// A component's `on*` prop: convert the value to the field's callback
    /// type under the owner (the conversion is where the callback captures
    /// it).
    pub fn prop<T, H: IntoEventHandler<T>>(self, handler: H) -> T {
        self.owner().run(|| handler.into_event_handler())
    }

    /// An element's `Fn()` handler (`onclick` and the other plain events).
    pub fn plain<F: Fn() + 'static>(self, f: F) -> impl Fn() + 'static {
        let owner = self.owner();
        move || owner.run(&f)
    }

    /// An element's `oninput` / `onchange` handler.
    pub fn input<F: Fn(String) + 'static>(self, f: F) -> impl Fn(String) + 'static {
        let owner = self.owner();
        move |value: String| owner.run(|| f(value))
    }

    /// An element's `onscroll` handler.
    pub fn scroll<F: Fn(ScrollEvent) + 'static>(self, f: F) -> impl Fn(ScrollEvent) + 'static {
        let owner = self.owner();
        move |event: ScrollEvent| owner.run(|| f(event))
    }

    /// An element's `onkeydown` / `onkeyup` handler.
    pub fn key<F: Fn(&KeyEventData) -> bool + 'static>(
        self,
        f: F,
    ) -> impl Fn(&KeyEventData) -> bool + 'static {
        let owner = self.owner();
        move |key: &KeyEventData| owner.run(|| f(key))
    }

    /// An element's `onwheel` handler.
    pub fn wheel<F: Fn(&WheelEvent) -> bool + 'static>(
        self,
        f: F,
    ) -> impl Fn(&WheelEvent) -> bool + 'static {
        let owner = self.owner();
        move |wheel: &WheelEvent| owner.run(|| f(wheel))
    }

    /// An element's `onfiledrop` handler.
    pub fn file_drop<F: Fn(Vec<std::path::PathBuf>) + 'static>(
        self,
        f: F,
    ) -> impl Fn(Vec<std::path::PathBuf>) + 'static {
        let owner = self.owner();
        move |paths: Vec<std::path::PathBuf>| owner.run(|| f(paths))
    }

    /// An element's `ondropfiles` handler.
    pub fn files_drop<F: Fn(Vec<LocalFile>) + 'static>(
        self,
        f: F,
    ) -> impl Fn(Vec<LocalFile>) + 'static {
        let owner = self.owner();
        move |files: Vec<LocalFile>| owner.run(|| f(files))
    }
}
