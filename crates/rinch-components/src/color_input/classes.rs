//! `ColorInput`'s size step, swatch size and root class string: what `size`,
//! `radius`, `error` and `disabled` turn into.

use super::ColorInput;

impl ColorInput {
    /// The size step this input renders at: one of `xs`/`sm`/`md`/`lg`/`xl`.
    ///
    /// Empty and unrecognised spellings both fall back to `md`, matching
    /// `TextInput`, whose `size` parses with `unwrap_or_default()` onto a
    /// `#[default] Md`. Trimmed and lowercased for the same reason it is there.
    pub(super) fn size_step(&self) -> &'static str {
        match self.size.trim().to_lowercase().as_str() {
            "xs" => "xs",
            "sm" => "sm",
            "lg" => "lg",
            "xl" => "xl",
            _ => "md",
        }
    }

    /// The preview swatch's edge length for this size step.
    ///
    /// `md` is `22px`, the value the swatch was hard-coded to before `size` was
    /// wired, so the default rendering is unchanged.
    pub(super) fn swatch_size(&self) -> &'static str {
        match self.size_step() {
            "xs" => "16px",
            "sm" => "18px",
            "lg" => "26px",
            "xl" => "30px",
            _ => "22px",
        }
    }

    /// The CSS class string for the root element.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-color-input".to_string()];

        // Always emitted, `md` included: the size rules key off it, so the
        // default step has to name itself the way `TextInput`'s does.
        classes.push(format!("rinch-color-input--{}", self.size_step()));

        // Radius, unlike size, has no default class. An empty or unrecognised
        // value leaves the base rule's `--rinch-radius-sm` standing, which is
        // the idiom every other radius-taking component uses (`DropdownMenu`,
        // `Modal`, `Card`).
        if !self.radius.is_empty() {
            match self.radius.trim().to_lowercase().as_str() {
                "xs" => classes.push("rinch-color-input--radius-xs".to_string()),
                "sm" => classes.push("rinch-color-input--radius-sm".to_string()),
                "md" => classes.push("rinch-color-input--radius-md".to_string()),
                "lg" => classes.push("rinch-color-input--radius-lg".to_string()),
                "xl" => classes.push("rinch-color-input--radius-xl".to_string()),
                _ => {}
            }
        }

        if !self.error.is_empty() {
            classes.push("rinch-color-input--error".to_string());
        }
        if self.disabled {
            classes.push("rinch-color-input--disabled".to_string());
        }

        classes.join(" ")
    }
}
