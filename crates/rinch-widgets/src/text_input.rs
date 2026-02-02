//! TextInput widget.
//!
//! A single-line text input field with label and error support.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;

/// TextInput size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextInputSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl TextInputSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            TextInputSize::Xs => "rinch-text-input--xs",
            TextInputSize::Sm => "rinch-text-input--sm",
            TextInputSize::Md => "rinch-text-input--md",
            TextInputSize::Lg => "rinch-text-input--lg",
            TextInputSize::Xl => "rinch-text-input--xl",
        }
    }
}

impl std::str::FromStr for TextInputSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(TextInputSize::Xs),
            "sm" => Ok(TextInputSize::Sm),
            "md" => Ok(TextInputSize::Md),
            "lg" => Ok(TextInputSize::Lg),
            "xl" => Ok(TextInputSize::Xl),
            _ => Err(()),
        }
    }
}

/// A single-line text input field.
#[derive(Debug, Default)]
pub struct TextInput {
    /// Input label.
    pub label: Option<String>,
    /// Placeholder text.
    pub placeholder: Option<String>,
    /// Description text below the input.
    pub description: Option<String>,
    /// Error message (shows error styling when present).
    pub error: Option<String>,
    /// Input size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Whether the input is disabled.
    pub disabled: bool,
    /// Whether the input is required.
    pub required: bool,
    /// Override border radius.
    pub radius: Option<String>,
    /// Input type (text, password, email, etc.).
    pub input_type: Option<String>,
}

impl TextInput {
    /// Generate the CSS class string for this text input.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-text-input"];

        // Size class
        let size: TextInputSize = self
            .size
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        classes.push(size.class_name());

        // Error state
        if self.error.is_some() {
            classes.push("rinch-text-input--error");
        }

        classes.join(" ")
    }
}

impl Widget for TextInput {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-text-input" } };
        container.set_attribute("class", &self.class_string());

        // Label element
        if let Some(label_text) = &self.label {
            let label = rinch_macros::rsx! { label { class: "rinch-text-input__label" } };
            let label_text_node = __scope.create_text(label_text);
            label.append_child(&label_text_node);

            if self.required {
                let required_span =
                    rinch_macros::rsx! { span { class: "rinch-text-input__required", "*" } };
                label.append_child(&required_span);
            }

            container.append_child(&label);
        }

        // Input element
        let input_type = self.input_type.as_deref().unwrap_or("text");
        let input = rinch_macros::rsx! { input { class: "rinch-text-input__input" } };
        input.set_attribute("type", input_type);

        if let Some(placeholder) = &self.placeholder {
            input.set_attribute("placeholder", placeholder);
        }
        if self.disabled {
            input.set_attribute("disabled", "");
        }
        if self.required {
            input.set_attribute("required", "");
        }

        container.append_child(&input);

        // Description element
        if let Some(desc) = &self.description {
            let desc_div = rinch_macros::rsx! { div { class: "rinch-text-input__description" } };
            let desc_text = __scope.create_text(desc);
            desc_div.append_child(&desc_text);
            container.append_child(&desc_div);
        }

        // Error element
        if let Some(err) = &self.error {
            let err_div = rinch_macros::rsx! { div { class: "rinch-text-input__error" } };
            let err_text = __scope.create_text(err);
            err_div.append_child(&err_text);
            container.append_child(&err_div);
        }

        container
    }
}
