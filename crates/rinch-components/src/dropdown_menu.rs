//! DropdownMenu component.
//!
//! A dropdown menu component (distinct from native AppMenu/Menu).

use rinch_core::Callback;
use rinch_core::Component;
use rinch_core::Signal;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events::get_click_context;
use rinch_core::reactive::Effect;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use std::rc::Rc;

/// The one class the `opened_fn` effect owns.
///
/// `styles/dropdown_menu.rs` keeps `.rinch-dropdown-menu__dropdown` and
/// `.rinch-dropdown-menu__backdrop` at `display: none` and shows both from
/// `.rinch-dropdown-menu--opened`, so the class *is* the show/hide. It used to
/// be emitted by [`DropdownMenu::class_string`] and matched by nothing at all,
/// while the reveal was an inline `style` rewrite on each panel — issue #760.
const OPENED_CLASS: &str = "rinch-dropdown-menu--opened";

/// Registry of live menu-close signals, keyed by the id a menu's content root
/// carries for its whole lifetime via [`MENU_CLOSE_ID_ATTR`].
///
/// Issue #714: a `DropdownMenuItem`'s click handler used to resolve its menu's
/// close signal through a thread-local the container set just before its
/// children rendered and cleared right after — a snapshot of "what is
/// rendering right now". An item inside a reactive block
/// (`if clipboard.get().is_some() { DropdownMenuItem { .. } }`) renders again
/// whenever that block's own signal changes, long after the container's own
/// `render` has returned and the thread-local is back to `None`: the item's
/// own `onclick` ran, but the lookup found nothing, and the menu stayed open.
///
/// The fix moves the lookup from render time to **click** time, and keys it
/// off the live DOM rather than off whatever happened to be rendering when the
/// item was built: a menu's content root carries [`MENU_CLOSE_ID_ATTR`] for as
/// long as the menu exists, so [`find_menu_close_signal`] — called from inside the
/// click handler, not baked into it at render — finds it by walking up from
/// whichever button was actually clicked, however late that button arrived.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
thread_local! {
    static MENU_CLOSE_REGISTRY: RefCell<HashMap<u64, Signal<bool>>> =
        RefCell::new(HashMap::new());
    /// Monotonic id source for [`MENU_CLOSE_REGISTRY`]. Never reused, so a
    /// stale id left on a long-discarded node never resolves to some other
    /// menu's entry.
    static NEXT_MENU_CLOSE_ID: Cell<u64> = const { Cell::new(1) };
}

/// The attribute a menu's content root carries for its whole lifetime, naming
/// its entry in [`MENU_CLOSE_REGISTRY`]. See the registry's doc comment for
/// why this replaced a thread-local set only around the container's own
/// render.
pub(crate) const MENU_CLOSE_ID_ATTR: &str = "data-rinch-menu-close-id";

/// Register `signal` as a menu's close signal and return the id to stamp on
/// its content root via [`MENU_CLOSE_ID_ATTR`]. Pair with
/// [`unregister_menu_close_signal`] on the menu's own cleanup, or the registry
/// accumulates one dead entry per menu built over the session.
pub(crate) fn register_menu_close_signal(signal: Signal<bool>) -> u64 {
    let id = NEXT_MENU_CLOSE_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    });
    MENU_CLOSE_REGISTRY.with(|r| r.borrow_mut().insert(id, signal));
    id
}

/// Drop `id`'s registry entry. The `Signal` itself is freed by whatever scope
/// owns it regardless of this call; this only stops the registry holding a
/// copy of it once the menu that named it is gone.
pub(crate) fn unregister_menu_close_signal(id: u64) {
    MENU_CLOSE_REGISTRY.with(|r| {
        r.borrow_mut().remove(&id);
    });
}

/// The close signal of the menu whose content root is `node` or its nearest
/// ancestor carrying [`MENU_CLOSE_ID_ATTR`], if that menu is still registered.
///
/// An item resolves this **before** running its own callback: the callback may
/// queue a write that removes the clicked button itself (a Paste item inside an
/// `if` its own action flips), and the walk's first `NodeHandle` access flushes
/// that write — leaving a detached button with no menu above it.
pub(crate) fn find_menu_close_signal(node: &NodeHandle) -> Option<Signal<bool>> {
    let mut current = Some(node.clone());
    while let Some(here) = current {
        if let Some(raw) = here.get_attribute(MENU_CLOSE_ID_ATTR) {
            let id = raw.parse::<u64>().ok()?;
            return MENU_CLOSE_REGISTRY.with(|r| r.borrow().get(&id).copied());
        }
        current = here.parent_node();
    }
    None
}

/// Close the menu `signal` belongs to. A freed signal — the item's callback
/// disposed the scope that owns it — is a no-op: there is nothing left to close.
pub(crate) fn close_menu_signal(signal: Option<Signal<bool>>) {
    if let Some(signal) = signal
        && signal.is_alive()
    {
        signal.set(false);
    }
}

/// How many menus are registered on this thread (tests: a leak shows as growth).
#[cfg(test)]
pub(crate) fn registered_menu_count() -> usize {
    MENU_CLOSE_REGISTRY.with(|r| r.borrow().len())
}

/// Reactive callback type for opened state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// Dropdown menu position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DropdownMenuPosition {
    #[default]
    Bottom,
    BottomStart,
    BottomEnd,
    Top,
    TopStart,
    TopEnd,
    Left,
    LeftStart,
    LeftEnd,
    Right,
    RightStart,
    RightEnd,
}

impl DropdownMenuPosition {
    pub fn class_name(&self) -> &'static str {
        match self {
            DropdownMenuPosition::Bottom => "rinch-dropdown-menu--bottom",
            DropdownMenuPosition::BottomStart => "rinch-dropdown-menu--bottom-start",
            DropdownMenuPosition::BottomEnd => "rinch-dropdown-menu--bottom-end",
            DropdownMenuPosition::Top => "rinch-dropdown-menu--top",
            DropdownMenuPosition::TopStart => "rinch-dropdown-menu--top-start",
            DropdownMenuPosition::TopEnd => "rinch-dropdown-menu--top-end",
            DropdownMenuPosition::Left => "rinch-dropdown-menu--left",
            DropdownMenuPosition::LeftStart => "rinch-dropdown-menu--left-start",
            DropdownMenuPosition::LeftEnd => "rinch-dropdown-menu--left-end",
            DropdownMenuPosition::Right => "rinch-dropdown-menu--right",
            DropdownMenuPosition::RightStart => "rinch-dropdown-menu--right-start",
            DropdownMenuPosition::RightEnd => "rinch-dropdown-menu--right-end",
        }
    }
}

impl std::str::FromStr for DropdownMenuPosition {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "bottom" => Ok(DropdownMenuPosition::Bottom),
            "bottom_start" | "bottomstart" => Ok(DropdownMenuPosition::BottomStart),
            "bottom_end" | "bottomend" => Ok(DropdownMenuPosition::BottomEnd),
            "top" => Ok(DropdownMenuPosition::Top),
            "top_start" | "topstart" => Ok(DropdownMenuPosition::TopStart),
            "top_end" | "topend" => Ok(DropdownMenuPosition::TopEnd),
            "left" => Ok(DropdownMenuPosition::Left),
            "left_start" | "leftstart" => Ok(DropdownMenuPosition::LeftStart),
            "left_end" | "leftend" => Ok(DropdownMenuPosition::LeftEnd),
            "right" => Ok(DropdownMenuPosition::Right),
            "right_start" | "rightstart" => Ok(DropdownMenuPosition::RightStart),
            "right_end" | "rightend" => Ok(DropdownMenuPosition::RightEnd),
            _ => Err(()),
        }
    }
}

/// A dropdown menu component.
///
/// # Example
///
/// ```ignore
/// let show_menu = Signal::new(false);
///
/// rsx! {
///     DropdownMenu {
///         opened_fn: move || show_menu.get(),
///         on_close: move || show_menu.set(false),
///         DropdownMenuTarget {
///             Button { onclick: move || show_menu.update(|v| *v = !*v),
///                 "Options"
///             }
///         }
///         DropdownMenuDropdown {
///             DropdownMenuItem { onclick: || println!("Edit"), "Edit" }
///             DropdownMenuItem { onclick: || println!("Delete"), "Delete" }
///             DropdownMenuDivider {}
///             DropdownMenuItem { color: "red", "Delete All" }
///         }
///     }
/// }
/// ```
pub struct DropdownMenu {
    /// Whether the menu is open (static, for initial render or non-reactive use).
    pub opened: bool,
    /// Reactive opened getter - use this for fine-grained updates.
    pub opened_fn: Option<ReactiveBool>,
    /// Position relative to target.
    pub position: String,
    /// Offset from target.
    pub offset: Option<i32>,
    /// Border radius.
    pub radius: String,
    /// Shadow size.
    pub shadow: String,
    /// Whether clicking outside closes menu. Requires `on_close` to actually
    /// close — the menu's open state is owned by the caller via `opened_fn`,
    /// and the backdrop fires `on_close` so the caller can flip their signal.
    ///
    /// The backdrop is a `position: fixed` box covering the viewport (see the
    /// note on `.rinch-dropdown-menu__backdrop` in `styles::dropdown_menu`), so
    /// "outside" means the whole window — including the app's own fixed chrome
    /// — and not merely the panel's clipping ancestor. It was `absolute` and
    /// clipped to that ancestor between PR #317 and #324's stage C.
    pub close_on_click_outside: bool,
    /// Whether clicking an item closes the menu. Requires `on_close`; when
    /// true, item clicks fire `on_close` automatically so the caller doesn't
    /// need `set(false)` boilerplate inside every item's onclick.
    pub close_on_item_click: bool,
    /// Fired when the user clicks outside the menu or clicks a menu item
    /// (gated by `close_on_click_outside` and `close_on_item_click` flags).
    /// The caller should set their `opened_fn` source to false from here.
    pub on_close: Option<Callback>,
    /// Width of the dropdown.
    pub width: String,
    /// Z-index.
    pub z_index: Option<i32>,
    /// Internal signal `DropdownMenuItem` children close by setting it to
    /// `false` (issue #714: resolved at click time from the DOM, not at
    /// render time from a thread-local — see [`register_menu_close_signal`]).
    /// `DropdownMenu` observes the transition to fire `on_close`. Set
    /// automatically — not a user prop.
    #[doc(hidden)]
    pub _close_signal: Signal<bool>,
    /// This menu's entry in [`MENU_CLOSE_REGISTRY`], stamped on `root` via
    /// [`MENU_CLOSE_ID_ATTR`] so a late-rendered item can still find it. Set
    /// automatically — not a user prop.
    #[doc(hidden)]
    pub _close_signal_id: u64,
}

impl std::fmt::Debug for DropdownMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DropdownMenu")
            .field("opened", &self.opened)
            .field("opened_fn", &self.opened_fn.as_ref().map(|_| "<reactive>"))
            .field("position", &self.position)
            .finish()
    }
}

impl Default for DropdownMenu {
    fn default() -> Self {
        // Initialized to true; items publish a close request by setting it to
        // false. The registry id is minted here rather than in `render` only
        // because this is where `close_sig` is minted — `render` stamps it on
        // `root` once that exists (issue #714, same pattern as `ContextMenu`).
        let close_sig = Signal::new(true);
        let close_signal_id = register_menu_close_signal(close_sig);
        Self {
            opened: false,
            opened_fn: None,
            position: String::new(),
            offset: None,
            radius: String::new(),
            shadow: String::new(),
            close_on_click_outside: true,
            close_on_item_click: true,
            on_close: None,
            width: String::new(),
            z_index: None,
            _close_signal: close_sig,
            _close_signal_id: close_signal_id,
        }
    }
}

impl DropdownMenu {
    /// Resolved user position (or default `Bottom`).
    fn resolved_position(&self) -> DropdownMenuPosition {
        if self.position.is_empty() {
            DropdownMenuPosition::Bottom
        } else {
            self.position
                .parse::<DropdownMenuPosition>()
                .unwrap_or(DropdownMenuPosition::Bottom)
        }
    }

    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-dropdown-menu"];

        classes.push(self.resolved_position().class_name());

        let radius_cls = crate::class_utils::radius_class("rinch-dropdown-menu", &self.radius);
        classes.extend(radius_cls.as_deref());

        if !self.shadow.is_empty() {
            match self.shadow.as_str() {
                "xs" => classes.push("rinch-dropdown-menu--shadow-xs"),
                "sm" => classes.push("rinch-dropdown-menu--shadow-sm"),
                "md" => classes.push("rinch-dropdown-menu--shadow-md"),
                "lg" => classes.push("rinch-dropdown-menu--shadow-lg"),
                "xl" => classes.push("rinch-dropdown-menu--shadow-xl"),
                _ => {}
            }
        }

        if self.opened {
            classes.push(OPENED_CLASS);
        }

        classes.join(" ")
    }
}

impl Component for DropdownMenu {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut style_parts = Vec::new();
        if let Some(offset) = self.offset {
            style_parts.push(format!("--rinch-dropdown-menu-offset: {}px", offset));
        }
        if !self.width.is_empty() {
            style_parts.push(format!("--rinch-dropdown-menu-width: {}", self.width));
        }
        // z_index (#474): an inherited custom property, because the dropdown is a
        // separate component and because the backdrop's level is derived from
        // the panel's in CSS — one number moves both and keeps the backdrop
        // below the items.
        //
        // It had a third reason until #760: the panel's and the backdrop's
        // inline `style` were rewritten wholesale by the visibility effects, so
        // an inline `z-index` on either would have been dropped the first time
        // the menu opened. Those effects are gone — the reveal is a class on the
        // root now — so that hazard is gone with them, and the custom property
        // stays for the two reasons above.
        if let Some(z) = self.z_index {
            style_parts.push(format!("--rinch-dropdown-menu-z-index: {}", z));
        }

        let root = rinch_macros::rsx! { div { class: "rinch-dropdown-menu" } };
        root.set_attribute("class", &self.class_string());

        if !style_parts.is_empty() {
            root.set_attribute("style", &style_parts.join("; "));
        }

        // Stamped for this menu's whole lifetime (issue #714) — see
        // `register_menu_close_signal`'s doc comment — so a `DropdownMenuItem`
        // that renders late, inside a reactive block nested in `children`,
        // still finds its menu by walking up from the button actually
        // clicked rather than from whatever rendered when the item did.
        root.set_attribute(MENU_CLOSE_ID_ATTR, &self._close_signal_id.to_string());
        {
            let close_signal_id = self._close_signal_id;
            rinch_core::reactive::on_cleanup(move || {
                unregister_menu_close_signal(close_signal_id);
            });
        }

        for child in children {
            root.append_child(child);
        }

        // If reactive opened_fn is provided, create an Effect to toggle the
        // `--opened` class and reposition near the viewport edge.
        //
        // The class, not an inline `display` write per panel (issue #760):
        // `styles/dropdown_menu.rs` hides `.rinch-dropdown-menu__dropdown` and
        // `.rinch-dropdown-menu__backdrop` and shows both under
        // `.rinch-dropdown-menu--opened`. What it costs is that "the dropdown"
        // is now named by its class rather than by its position among the
        // children — a caller who passes some *other* element as a second child
        // gets an element that is always visible, where the positional write
        // hid it. That is the documented composition (`DropdownMenuTarget` +
        // `DropdownMenuDropdown`).
        //
        // The panel does not have to be a direct child of this root: `rsx!`
        // often wraps it (an `{Option}`, an `else if` branch, a helper whose
        // body is an `if`), and the sheet's panel rule reaches through any
        // wrapper while still keeping a closed menu nested in this one's panel
        // closed. Its one limit, and why it is spelled the way it is, are in
        // the note above that rule. (This is no longer the shape `Popover` has:
        // its rule is still a plain descendant one, wrapper-tolerant but opening
        // nested popovers too — issue #778.)
        //
        // The effect adds and removes the one class rather than rewriting
        // `class` (issue #717): the rsx `class:` prop is merged onto the
        // returned handle *after* `render` returns.
        if let Some(ref opened_fn) = self.opened_fn {
            let opened_fn_vis = opened_fn.clone();
            let root_vis = root.clone();
            Effect::new(move || {
                if opened_fn_vis() {
                    root_vis.add_class(OPENED_CLASS);
                } else {
                    root_vis.remove_class(OPENED_CLASS);
                }
            });

            // Auto-flip: when the menu opens within ~220px of the viewport's
            // bottom edge (and has room above), swap a Bottom*/Top* position
            // class so the dropdown opens upward instead of clipping. Decided
            // at open time using ClickContext — the click that triggered the
            // open populated the trigger's bounds and viewport size, which is
            // the best info available before layout has measured the dropdown.
            const DROPDOWN_RESERVE_PX: f32 = 220.0;
            let user_pos = self.resolved_position();
            let root_for_flip = root.clone();
            let opened_fn_flip = opened_fn.clone();
            let was_open = std::cell::Cell::new(false);
            let flipped = std::cell::Cell::new(false);
            Effect::new(move || {
                let is_open = opened_fn_flip();
                if is_open && !was_open.get() {
                    let ctx = get_click_context();
                    let space_below = ctx.viewport_height - ctx.element_y - ctx.element_height;
                    let space_above = ctx.element_y;
                    let want_flip = space_below < DROPDOWN_RESERVE_PX && space_above > space_below;
                    if want_flip != flipped.get() {
                        let old_class = if flipped.get() {
                            flip_position(user_pos)
                        } else {
                            user_pos
                        };
                        let new_class = if want_flip {
                            flip_position(user_pos)
                        } else {
                            user_pos
                        };
                        root_for_flip.remove_class(old_class.class_name());
                        root_for_flip.add_class(new_class.class_name());
                        flipped.set(want_flip);
                    }
                }
                was_open.set(is_open);
            });
        }

        // close_on_item_click: items publish close requests by setting
        // _close_signal to false (found via MENU_CLOSE_ID_ATTR on `root`,
        // stamped above). We observe true→false transitions and forward to
        // on_close. Don't
        // write back to close_sig inside this effect — that would borrow_mut
        // the running effect's own closure and panic. Instead, a separate
        // effect resets close_sig to true on the next opened_fn rising edge
        // (menu reopen), so the next item click is observable as a fresh
        // transition.
        if self.close_on_item_click && self.on_close.is_some() {
            let close_sig = self._close_signal;
            let cb = self.on_close.clone().unwrap();
            let last = std::rc::Rc::new(std::cell::Cell::new(true));
            let last_e = last.clone();
            Effect::new(move || {
                let is_true = close_sig.get();
                if last_e.get() && !is_true {
                    cb.invoke();
                }
                last_e.set(is_true);
            });

            // Reset close_sig to true on opened_fn rising edge so the next
            // item click registers as a transition. Separate effect = no
            // re-entrancy with the transition watcher above.
            if let Some(ref opened_fn) = self.opened_fn {
                let f = opened_fn.clone();
                let last_open = std::rc::Rc::new(std::cell::Cell::new(false));
                Effect::new(move || {
                    let open = f();
                    if open && !last_open.get() {
                        close_sig.set(true);
                    }
                    last_open.set(open);
                });
            }
        }

        // close_on_click_outside: render an invisible backdrop that fires
        // on_close when clicked. Sibling of the dropdown content inside the
        // position:relative root, `position: fixed` so it covers the window
        // whatever clips that root, and a z-index below the panel's so option
        // clicks still hit the items. Mirrors the Select pattern.
        //
        // Fixed rather than absolute is load-bearing, not incidental — the
        // long note above `.rinch-dropdown-menu__backdrop` says why.
        if self.close_on_click_outside && self.on_close.is_some() {
            let backdrop = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__backdrop" } };
            // No inline `display` here and no effect of its own: the sheet's
            // `.rinch-dropdown-menu--opened > .rinch-dropdown-menu__backdrop`
            // rule shows it from the root's class, which the effect above keeps
            // in step with `opened_fn` (issue #760).

            let cb = self.on_close.clone().unwrap();
            let handler_id = __scope.register_handler(move || cb.invoke());
            backdrop.set_attribute("data-rid", &handler_id.0.to_string());
            // A press of any button outside dismisses, not only a left one (#1093).
            backdrop.set_attribute(rinch_core::events::BACKDROP_ATTRIBUTE, "");

            root.append_child(&backdrop);
        }

        root
    }
}

/// Flip a position from Bottom* ↔ Top* (other positions return unchanged).
fn flip_position(pos: DropdownMenuPosition) -> DropdownMenuPosition {
    match pos {
        DropdownMenuPosition::Bottom => DropdownMenuPosition::Top,
        DropdownMenuPosition::BottomStart => DropdownMenuPosition::TopStart,
        DropdownMenuPosition::BottomEnd => DropdownMenuPosition::TopEnd,
        DropdownMenuPosition::Top => DropdownMenuPosition::Bottom,
        DropdownMenuPosition::TopStart => DropdownMenuPosition::BottomStart,
        DropdownMenuPosition::TopEnd => DropdownMenuPosition::BottomEnd,
        other => other,
    }
}

/// Target element for the dropdown menu.
#[derive(Debug, Default)]
pub struct DropdownMenuTarget;

impl Component for DropdownMenuTarget {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let target = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__target" } };

        for child in children {
            target.append_child(child);
        }

        target
    }
}

/// Dropdown content container.
#[derive(Debug, Default)]
pub struct DropdownMenuDropdown;

impl Component for DropdownMenuDropdown {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let dropdown = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__dropdown" } };

        for child in children {
            dropdown.append_child(child);
        }

        dropdown
    }
}

/// A menu item in the dropdown.
#[derive(Debug, Default)]
pub struct DropdownMenuItem {
    /// Left section icon.
    pub left_section: Option<TablerIcon>,
    /// Right section icon.
    pub right_section: Option<TablerIcon>,
    /// Color variant.
    pub color: String,
    /// Whether item is disabled.
    pub disabled: bool,
    /// Click callback.
    pub onclick: Option<rinch_core::Callback>,
}

impl Component for DropdownMenuItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut classes = vec!["rinch-dropdown-menu__item"];

        if self.disabled {
            classes.push("rinch-dropdown-menu__item--disabled");
        }

        let color_style = if self.color.is_empty() {
            None
        } else {
            let c = &self.color;
            if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                Some(format!("--rinch-dropdown-menu-item-color: {}", c))
            } else {
                Some(format!(
                    "--rinch-dropdown-menu-item-color: var(--rinch-color-{}-6)",
                    c
                ))
            }
        };

        let btn = rinch_macros::rsx! { button { class: "rinch-dropdown-menu__item" } };
        btn.set_attribute("class", &classes.join(" "));

        if let Some(ref style) = color_style {
            btn.set_attribute("style", style);
        }

        if self.disabled {
            btn.set_attribute("disabled", "");
        }

        // Click handler — also closes the parent menu if one exists.
        //
        // The lookup is `find_menu_close_signal(&btn_for_close)`, run at click
        // time rather than resolved once here at render time (issue #714), and
        // before the item's own callback, which may detach the button. An item
        // built inside a reactive block nested in the dropdown renders again on
        // every change to that block's own condition, long after the
        // container's own render — and a lookup
        // resolved at render time would see the container's render as already
        // finished. Walking the DOM from the clicked button instead finds the
        // menu's content root — which carries `MENU_CLOSE_ID_ATTR` for the
        // menu's whole lifetime — however late `btn` itself was built.
        //
        // A freed signal (the callback above disposed the scope that owns it
        // — an item whose action removes the very row the menu hangs off) is a
        // no-op inside `close_menu_signal`, same as it was here: there is
        // nothing left to close.
        if let Some(ref cb) = self.onclick {
            let btn_for_close = btn.clone();
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || {
                    let close = find_menu_close_signal(&btn_for_close);
                    cb.invoke();
                    close_menu_signal(close);
                }
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }

        // Left section
        if let Some(icon) = self.left_section {
            let left_span = rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-left" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            left_span.append_child(&icon_el);
            btn.append_child(&left_span);
        }

        // Label
        let label_span = rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-label" } };
        for child in children {
            label_span.append_child(child);
        }
        btn.append_child(&label_span);

        // Right section
        if let Some(icon) = self.right_section {
            let right_span =
                rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-right" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            right_span.append_child(&icon_el);
            btn.append_child(&right_span);
        }

        btn
    }
}

/// A label/header in the dropdown.
#[derive(Debug, Default)]
pub struct DropdownMenuLabel;

impl Component for DropdownMenuLabel {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let label = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__label" } };

        for child in children {
            label.append_child(child);
        }

        label
    }
}

/// A divider in the dropdown.
#[derive(Debug, Default)]
pub struct DropdownMenuDivider;

impl Component for DropdownMenuDivider {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        rinch_macros::rsx! { div { class: "rinch-dropdown-menu__divider" } }
    }
}

#[cfg(test)]
mod close_signal_tests {
    use super::*;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_core::dom::{DomDocument, RenderScope};
    use rinch_core::events::{EventHandlerId, dispatch_event};
    use rinch_core::reactive::Scope;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// An item whose action removes the very row the menu hangs off.
    ///
    /// That is an ordinary thing to write — "Delete", "Close tab", anything that
    /// rebuilds the list — and the menu goes with the row: the scope that owns
    /// the close signal is disposed inside the callback, so the close write that
    /// follows has nothing to write to.
    ///
    /// This characterises the shape rather than catching a crash, and it is
    /// worth being exact about which: `Signal::set` on a freed signal **drops
    /// the write and warns**, it does not panic, so this test passes with or
    /// without the `is_alive` guard. What the guard removes is that warning,
    /// which names the menu's own line and reads like a defect in it. The
    /// regression risk the guard introduces — that it stops closing menus that
    /// are still there — is what the test below pins.
    #[test]
    fn an_item_whose_action_disposes_the_menus_scope_leaves_nothing_to_close() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        // `RenderScope` holds only a `Weak`, so the strong handle has to outlive
        // it or every `create_element` panics with "Document dropped".
        let mut scope = RenderScope::new(doc.clone(), body);
        let _doc = doc;

        // The signal belongs to the menu's own scope, as it does in a real tree.
        let menu_scope = Scope::new();
        let opened = menu_scope.run(|| Signal::new(true));
        let close_id = register_menu_close_signal(opened);
        let menu_root = scope.create_element("div");
        menu_root.set_attribute(MENU_CLOSE_ID_ATTR, &close_id.to_string());

        let ran = Rc::new(Cell::new(false));
        let fired = ran.clone();
        // `Scope` is not `Clone`, and the callback must outlive this frame, so
        // the scope is shared rather than copied.
        let doomed = Rc::new(menu_scope);
        let doomed_in_callback = doomed.clone();
        let item = DropdownMenuItem {
            onclick: Some(rinch_core::Callback::new(move || {
                fired.set(true);
                // The action takes the row, and the menu with it.
                doomed_in_callback.dispose();
            })),
            ..Default::default()
        };
        let node = item.render(&mut scope, &[]);
        menu_root.append_child(&node);

        let rid: usize = node
            .get_attribute("data-rid")
            .expect("an item with an onclick carries a handler")
            .parse()
            .unwrap();

        dispatch_event(EventHandlerId(rid));

        assert!(ran.get(), "the item's own action runs to completion");
        assert!(
            !opened.is_alive(),
            "and the menu it belonged to is gone, so there is nothing to close"
        );
    }

    /// The ordinary case, and the one that actually bites: the guard must not
    /// quietly stop closing menus that are still there.
    #[test]
    fn an_ordinary_item_still_closes_its_menu() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        // `RenderScope` holds only a `Weak`, so the strong handle has to outlive
        // it or every `create_element` panics with "Document dropped".
        let mut scope = RenderScope::new(doc.clone(), body);
        let _doc = doc;

        let menu_scope = Scope::new();
        let opened = menu_scope.run(|| Signal::new(true));
        let close_id = register_menu_close_signal(opened);
        let menu_root = scope.create_element("div");
        menu_root.set_attribute(MENU_CLOSE_ID_ATTR, &close_id.to_string());

        let item = DropdownMenuItem {
            onclick: Some(rinch_core::Callback::new(|| {})),
            ..Default::default()
        };
        let node = item.render(&mut scope, &[]);
        menu_root.append_child(&node);

        let rid: usize = node.get_attribute("data-rid").unwrap().parse().unwrap();
        dispatch_event(EventHandlerId(rid));

        assert_eq!(opened.try_get(), Some(false), "the menu closed");
        menu_scope.dispose();
        unregister_menu_close_signal(close_id);
    }

    /// Issue #714: an item built *after* the menu's own render — not merely
    /// appended, but rendered late, the way a `show_dom`/`if` branch nested in
    /// the dropdown renders its content — must still close the menu it was
    /// built inside of.
    ///
    /// The old thread-local lookup resolved `get_menu_close_signal()` at
    /// `DropdownMenuItem::render` time. The menu's own `render` (simulated
    /// here by stamping `MENU_CLOSE_ID_ATTR` on `menu_root` and then, as the
    /// old code did, leaving no thread-local set) has already "returned" by
    /// the time this item renders, so the old lookup found `None` and the
    /// click ran the item's `onclick` without ever touching `opened`. The new
    /// lookup is resolved from inside the click handler against the live DOM,
    /// so it does not matter that `item` was built after `menu_root`.
    #[test]
    fn an_item_rendered_after_the_menu_still_closes_it() {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let _doc = doc;

        // Stand in for the menu container's own render, complete and
        // returned, exactly as the old thread-local would have been cleared
        // by then.
        let opened = Signal::new(true);
        let close_id = register_menu_close_signal(opened);
        let menu_root = scope.create_element("div");
        menu_root.set_attribute(MENU_CLOSE_ID_ATTR, &close_id.to_string());
        scope.body_handle().append_child(&menu_root);

        // The item renders strictly afterwards — the `show_dom`/`if` shape —
        // and is only now appended under the already-rendered menu root.
        let item = DropdownMenuItem {
            onclick: Some(rinch_core::Callback::new(|| {})),
            ..Default::default()
        };
        let node = item.render(&mut scope, &[]);
        menu_root.append_child(&node);

        let rid: usize = node.get_attribute("data-rid").unwrap().parse().unwrap();
        dispatch_event(EventHandlerId(rid));

        assert_eq!(
            opened.try_get(),
            Some(false),
            "an item rendered after the menu must still be able to close it"
        );
        unregister_menu_close_signal(close_id);
    }
}
