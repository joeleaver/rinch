//! Component CSS styles.
//!
//! Generates CSS classes for all components that integrate with the theme.
//! Each component has its own style module for maintainability.

mod accordion;
mod action_icon;
mod alert;
mod anchor;
mod app_menu_bar;
mod avatar;
mod badge;
mod blockquote;
mod borderless_window;
mod breadcrumbs;
mod button;
mod card;
mod center;
mod checkbox;
mod close_button;
mod code;
mod container;
mod divider;
mod drawer;
mod dropdown_menu;
mod fieldset;
mod floating_panel;
mod group;
mod highlight;
mod hover_card;
mod image;
mod kbd;
mod list;
mod loader;
mod loading_overlay;
mod mark;
mod modal;
mod navlink;
mod notification;
mod number_input;
mod pagination;
mod paper;
mod password_input;
mod popover;
mod progress;
mod radio;
mod select;
mod simple_grid;
mod skeleton;
mod slider;
mod space;
mod stack;
mod stepper;
mod switch;
mod tabs;
mod text;
mod text_input;
mod textarea;
mod title;
mod tooltip;
mod tree;

/// Generate all component CSS styles.
pub fn generate_all_component_styles() -> String {
    let mut css = String::with_capacity(32768);

    css.push_str(&button::styles());
    css.push_str(&text_input::styles());
    css.push_str(&text::styles());
    css.push_str(&paper::styles());
    css.push_str(&stack::styles());
    css.push_str(&group::styles());
    css.push_str(&simple_grid::styles());
    css.push_str(&badge::styles());
    css.push_str(&divider::styles());
    css.push_str(&container::styles());
    css.push_str(&center::styles());
    css.push_str(&space::styles());
    css.push_str(&title::styles());
    css.push_str(&anchor::styles());
    css.push_str(&code::styles());
    css.push_str(&kbd::styles());
    css.push_str(&textarea::styles());
    css.push_str(&checkbox::styles());
    css.push_str(&switch::styles());
    css.push_str(&select::styles());
    css.push_str(&fieldset::styles());
    css.push_str(&alert::styles());
    css.push_str(&loader::styles());
    css.push_str(&progress::styles());
    css.push_str(&action_icon::styles());
    css.push_str(&close_button::styles());
    css.push_str(&radio::styles());
    css.push_str(&number_input::styles());
    css.push_str(&password_input::styles());

    // Tier 2 components
    css.push_str(&avatar::styles());
    css.push_str(&card::styles());
    css.push_str(&tooltip::styles());
    css.push_str(&skeleton::styles());
    css.push_str(&image::styles());
    css.push_str(&list::styles());
    css.push_str(&slider::styles());
    css.push_str(&blockquote::styles());
    css.push_str(&mark::styles());
    css.push_str(&highlight::styles());

    // Tier 3 components
    css.push_str(&tabs::styles());
    css.push_str(&accordion::styles());
    css.push_str(&breadcrumbs::styles());
    css.push_str(&pagination::styles());
    css.push_str(&navlink::styles());
    css.push_str(&stepper::styles());
    css.push_str(&tree::styles());

    // Tier 4 components (overlays)
    css.push_str(&modal::styles());
    css.push_str(&drawer::styles());
    css.push_str(&notification::styles());
    css.push_str(&popover::styles());
    css.push_str(&dropdown_menu::styles());
    css.push_str(&hover_card::styles());
    css.push_str(&loading_overlay::styles());

    // Window components
    css.push_str(&borderless_window::styles());
    css.push_str(&floating_panel::styles());

    // App menu bar (in-app menu for Linux)
    css.push_str(&app_menu_bar::styles());

    css
}
