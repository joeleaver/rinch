//! Stepper component.
//!
//! Step-by-step progress indicator.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::rc::Rc;

use rinch_core::Component;
use rinch_core::ValueCallback;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

use crate::borderless_window::SectionRenderer;

mod content;
mod derive;
mod icon;
mod step;
mod tree;

use content::*;
use derive::*;
use icon::*;
use tree::*;

pub use step::{StepperCompleted, StepperStep};

#[cfg(test)]
mod differential_tests;
#[cfg(test)]
mod extended_differential_tests;

/// Stepper size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepperSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl StepperSize {
    pub fn class_name(&self) -> &'static str {
        match self {
            StepperSize::Xs => "rinch-stepper--xs",
            StepperSize::Sm => "rinch-stepper--sm",
            StepperSize::Md => "rinch-stepper--md",
            StepperSize::Lg => "rinch-stepper--lg",
            StepperSize::Xl => "rinch-stepper--xl",
        }
    }
}

impl std::str::FromStr for StepperSize {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(StepperSize::Xs),
            "sm" => Ok(StepperSize::Sm),
            "md" => Ok(StepperSize::Md),
            "lg" => Ok(StepperSize::Lg),
            "xl" => Ok(StepperSize::Xl),
            _ => Err(()),
        }
    }
}

/// Stepper orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepperOrientation {
    #[default]
    Horizontal,
    Vertical,
}

impl StepperOrientation {
    pub fn class_name(&self) -> &'static str {
        match self {
            StepperOrientation::Horizontal => "rinch-stepper--horizontal",
            StepperOrientation::Vertical => "rinch-stepper--vertical",
        }
    }
}

impl std::str::FromStr for StepperOrientation {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "horizontal" => Ok(StepperOrientation::Horizontal),
            "vertical" => Ok(StepperOrientation::Vertical),
            _ => Err(()),
        }
    }
}

/// Stepper component for multi-step processes.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Stepper { active: 1, completed_icon: TablerIcon::CircleCheck,
///         StepperStep { label: "Step 1", description: "Create account" }
///         StepperStep { label: "Step 2", description: "Verify email" }
///         StepperStep { label: "Step 3", description: "Complete profile" }
///     }
/// }
/// ```
///
/// Each step's state and index come from [`Stepper::active`] and the step's own
/// position: the steps before `active` are completed, the one at it is in
/// progress, and the ones after it are inactive (issue #709). So the example
/// above draws a `CircleCheck` on step 1, the number `2` on step 2, and `3` on
/// step 3, with no `state:` or `step:` written by hand.
///
/// A step may still name its own [`state`](StepperStep::state) or
/// [`step`](StepperStep::step), and what it names it keeps — that is the escape
/// hatch for a stepper whose states are not a simple prefix of the list.
///
/// # Step content
///
/// The children of a [`StepperStep`] are that step's content, and the stepper
/// shows the content of the step at [`Stepper::active`] in a
/// `.rinch-stepper__content` region **below** the row of steps, as Mantine
/// does (issue #2195). The other steps' content stays inside their steps,
/// hidden. Once `active` is past the last step, the
/// [`StepperCompleted`] block is shown there instead; before that it is hidden.
/// Content is moved, not copied, so its effects and handlers are the ones it
/// rendered with, and it goes back into its step when another step becomes the
/// active one (or when the step leaves the stepper, and is discarded with the
/// step when the step is). Set [`Stepper::content_for_active_only`] to `false`
/// to keep every step's content inside its step instead.
#[derive(Debug)]
pub struct Stepper {
    /// Currently active step (0-indexed).
    ///
    /// Each step takes its state from its position against this: before it
    /// completed, at it in progress, after it inactive. A step that named a
    /// `state` of its own keeps it. A step that arrives **later**, by a `for`
    /// reconcile or a `show_dom` branch, is stated as it lands — and so are the
    /// steps it displaced, since an insertion moves everything after it (issue
    /// #716). A step *removed*, or moved out to another stepper, renumbers and
    /// restates the steps behind it the same way (issue #745).
    ///
    /// A closure — `active: {|| signal.get()}` — re-renders the whole stepper,
    /// children included, so the derivation runs again on every change.
    pub active: u32,
    /// Size variant (xs, sm, md, lg, xl).
    pub size: String,
    /// Orientation (horizontal, vertical).
    pub orientation: String,
    /// Color for completed/active steps.
    pub color: String,
    /// Border radius for step icons.
    pub radius: String,
    /// Icon size.
    pub icon_size: String,
    /// Whether a step *after* the active one may be selected.
    ///
    /// When true, each step past [`Stepper::active`] that did not ask to be
    /// clickable itself is made clickable, a step that arrives later included
    /// (issue #716). A step that set
    /// `allow_step_click` or `allow_step_select` of its own is already clickable
    /// and is left alone, so this only ever grants — turning it off does not
    /// take a step's own ask away.
    ///
    /// This prop alone is still decorative — the class carries a cursor and a
    /// hover state, and granting it adds no handler. What makes a grant *act*
    /// is [`Stepper::on_step_click`] (issue #737): with no callback set, every
    /// behaviour this prop had before #737 is unchanged (the class, and only
    /// the class).
    pub allow_next_steps_select: bool,
    /// Fired with a step's 0-based position when the user clicks or activates
    /// (Enter/Space) a step the stepper considers clickable (issue #737).
    ///
    /// Mantine's default: a strictly **completed** step — `position <
    /// Stepper::active` — is clickable without `allow_next_steps_select`;
    /// the active step itself and every step past it need that flag, or the
    /// step's own
    /// [`allow_step_click`](StepperStep::allow_step_click) /
    /// [`allow_step_select`](StepperStep::allow_step_select) (Mantine's
    /// `shouldAllowSelect`: `state === 'stepCompleted' || allowNextStepsSelect`,
    /// and `state` for the active step is `'stepProgress'`, not
    /// `'stepCompleted'`). With no callback set, nothing is wired — a step's
    /// `cursor: pointer` from its own ask or from `allow_next_steps_select` is
    /// unchanged, decorative, as it always was.
    ///
    /// `Stepper` does not move `active` itself: the caller does, from this
    /// callback, exactly as [`Tabs`](crate::tabs::Tabs) moves its own selection
    /// from a tab click.
    ///
    /// The index is read from the clicked step's own position **at click
    /// time**, not baked into the handler at the render pass that wired it
    /// (the #714 pattern): a step's position can move after it registers its
    /// handler, through a later insertion or removal (issues #716, #745), and
    /// a captured index would then fire the position the step *used to* be at.
    pub on_step_click: Option<ValueCallback<u32>>,
    /// Default completed-step icon.
    ///
    /// Used by the [`StepperStep`]s in the completed state that set no
    /// `completed_icon` of their own — the step's icon wins, whether the step
    /// named its own state or this stepper derived it. Steps are patched after
    /// they have rendered, since a parent component renders *after* its
    /// children, and a step that arrives later is patched as it lands (issue
    /// #716).
    ///
    /// A step **moved in from another stepper** keeps the icon that stepper gave
    /// it: the box records that it holds a real glyph for the state, not which
    /// stepper put it there (issue #745).
    pub completed_icon: Option<TablerIcon>,
    /// Default in-progress-step icon.
    ///
    /// Used by the [`StepperStep`]s in the progress state that set no
    /// `progress_icon` of their own, in place of that step's `icon` or its
    /// number. The step's own `progress_icon` wins, and a step that arrives
    /// later takes this one as it lands (issue #716).
    pub progress_icon: Option<TablerIcon>,
    /// Whether only the active step's content is shown, in the
    /// `.rinch-stepper__content` region below the steps (default `true`, the
    /// Mantine layout; issue #2195).
    ///
    /// `false` is the escape hatch: the stepper moves no step's content, and
    /// every step shows its own content **inside the step**, at the end of its
    /// body, under the label and description (the stepper carries
    /// `rinch-stepper--content-in-steps`). In a vertical stepper that is a
    /// timeline: each step's content beside its icon and connector, indented
    /// with the label. In a horizontal one it sits under the label in the
    /// step's centred column, which is then at least as wide as the content.
    /// An app that wants only some steps' content seen there hides the rest
    /// with its own rule, such as
    /// `.rinch-stepper__step--inactive .rinch-stepper__step-content { display:
    /// none }`. The [`StepperCompleted`] block is shown below the steps past the
    /// last one either way.
    pub content_for_active_only: bool,
}

impl Default for Stepper {
    fn default() -> Self {
        Self {
            active: 0,
            size: String::new(),
            orientation: String::new(),
            color: String::new(),
            radius: String::new(),
            icon_size: String::new(),
            allow_next_steps_select: false,
            on_step_click: None,
            completed_icon: None,
            progress_icon: None,
            content_for_active_only: true,
        }
    }
}

/// Which of the three states a step is in.
///
/// A step is in exactly one, and carries exactly one of the three modifier
/// classes for it. `StepperStep` reads the caller's string into this and
/// `Stepper` derives it from a position, so the two agree by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepState {
    Completed,
    Progress,
    Inactive,
}

impl StepState {
    /// The caller's spelling. Anything but the two named states is inactive,
    /// **including the empty string** — which is why an unset `state` cannot be
    /// told from an explicit `"inactive"` by the outcome alone, and why the step
    /// records the ask separately as `data-state`.
    fn parse(s: &str) -> Self {
        match s {
            "completed" => StepState::Completed,
            "progress" => StepState::Progress,
            _ => StepState::Inactive,
        }
    }

    /// The state of the step at `index` in a stepper on `active`.
    fn derive(index: u32, active: u32) -> Self {
        match index.cmp(&active) {
            Ordering::Less => StepState::Completed,
            Ordering::Equal => StepState::Progress,
            Ordering::Greater => StepState::Inactive,
        }
    }

    fn class_name(self) -> &'static str {
        match self {
            StepState::Completed => "rinch-stepper__step--completed",
            StepState::Progress => "rinch-stepper__step--progress",
            StepState::Inactive => "rinch-stepper__step--inactive",
        }
    }
}

impl Stepper {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-stepper"];

        if !self.size.is_empty()
            && let Ok(size) = self.size.parse::<StepperSize>()
        {
            classes.push(size.class_name());
        }

        if !self.orientation.is_empty() {
            if let Ok(orientation) = self.orientation.parse::<StepperOrientation>() {
                classes.push(orientation.class_name());
            }
        } else {
            classes.push(StepperOrientation::Horizontal.class_name());
        }

        let radius_cls = crate::class_utils::radius_class("rinch-stepper", &self.radius);
        classes.extend(radius_cls.as_deref());

        if !self.content_for_active_only {
            classes.push(CONTENT_IN_STEPS_CLASS);
        }

        classes.join(" ")
    }
}

impl Component for Stepper {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut style_parts = Vec::new();
        if !self.color.is_empty() {
            style_parts.push(crate::color_utils::accent_vars(
                "--rinch-stepper",
                &self.color,
            ));
        }
        if !self.icon_size.is_empty() {
            style_parts.push(format!("--rinch-stepper-icon-size: {}", self.icon_size));
        }

        let container = rinch_macros::rsx! { div { class: "rinch-stepper" } };
        container.set_attribute("class", &self.class_string());
        container.set_attribute("data-active", &self.active.to_string());

        if !style_parts.is_empty() {
            container.set_attribute("style", &style_parts.join("; "));
        }

        let steps_container = rinch_macros::rsx! { div { class: "rinch-stepper__steps" } };

        for child in children {
            steps_container.append_child(child);
        }

        // Where the active step's content is shown, below the steps (issue
        // #2195). Created before the first pass so that pass can fill it.
        let content_region = rinch_macros::rsx! { div { class: "rinch-stepper__content" } };

        let derivation = Derivation {
            active: self.active,
            allow_next_steps_select: self.allow_next_steps_select,
            completed_icon: self.completed_icon,
            progress_icon: self.progress_icon,
            on_step_click: self.on_step_click.clone(),
            settler: next_settler(),
            content: ContentRegion {
                region: content_region.clone(),
                for_active_only: self.content_for_active_only,
                shown: Rc::new(RefCell::new(Vec::new())),
                completed: Rc::new(Cell::new(false)),
            },
        };
        settle_steps(__scope, &steps_container, &derivation);

        // A step that arrives *after* this render — a `for` reconcile, a
        // `show_dom` branch, a hand-rolled `append_child` — needs the same pass,
        // and so do the steps it displaces: an insertion in front of a step
        // moves it, which changes its number and can change its state (issue
        // #716). So the pass runs again over the whole list, and it is written
        // to be idempotent for exactly that reason. It derives only the steps
        // the change moved (issue #748): one this stepper settled at the
        // position it still holds is passed over.
        // A later pass registers click handlers too, and it runs under
        // whoever made the insertion or removal. They are this render's: owned
        // by its scope and freed with it, not by the code that moved a node.
        // (Rendered with no owner, they keep app lifetime, as the render's own.)
        let owner = rinch_core::reactive::current_owner();
        let watched = steps_container.clone();
        let d = derivation.clone();
        let o = owner.clone();
        crate::late_children::adopt_late_children(
            __scope,
            &steps_container,
            STEP_BOUNDARY,
            move |_inserted, scope| {
                // The subtree that landed is deliberately ignored. What has to
                // be found is every step's *position*, and only the whole list
                // gives that — an insertion in front of a step renumbers it and
                // can restate it, so patching the newcomer alone would leave the
                // stepper saying two different things.
                as_owned_by(&o, || settle_steps(scope, &watched, &d));
            },
        );

        // And a step that *leaves* (issue #745). A removal is the same event
        // read the other way: every step behind the one that went moves
        // backwards, which renumbers it and can restate it. `List` and
        // `RadioGroup` register no such observer, because neither of their
        // defaults depends on a sibling's position; this stepper's whole
        // derivation does.
        let watched = steps_container.clone();
        let d = derivation.clone();
        crate::late_children::adopt_child_removals(
            __scope,
            &steps_container,
            STEP_BOUNDARY,
            move |scope| {
                as_owned_by(&owner, || settle_steps(scope, &watched, &d));
            },
        );

        container.append_child(&steps_container);
        container.append_child(&content_region);

        container
    }
}
