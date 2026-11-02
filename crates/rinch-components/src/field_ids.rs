//! The label/field association every labelled input component shares
//! (issue #2034).
//!
//! `TextInput`, `Textarea`, `NumberInput`, `PasswordInput`, `ColorInput` and
//! `Select` render their `<label>` *beside* the field, so the label labels
//! nothing unless it names the field by id. This mirrors Mantine's
//! `Input.Wrapper`: the field carries `id` (the app's `id` prop, else a
//! generated one), the label `id="{id}-label"` and `for="{id}"`, the
//! description and error `{id}-description` / `{id}-error`, and the field
//! `aria-describedby` naming the error then the description, plus
//! `aria-invalid="true"` while an error is shown.
//!
//! Generated ids come from one process-wide counter, so no two instances —
//! of one component or of different ones, in one window or several, or one
//! instance across its own re-renders — ever share an id. They are spelled
//! `rinch-field-{n}`: an app that writes its own ids in that spelling can
//! collide with them, as it could with any library's generated ids.

use rinch_core::dom::NodeHandle;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIELD: AtomicU64 = AtomicU64::new(1);

/// The ids of one field and the parts around it.
pub(crate) struct FieldIds {
    field: String,
}

impl FieldIds {
    /// `app_id` when it is non-empty, else a fresh generated id.
    pub(crate) fn new(app_id: &str) -> Self {
        let field = if app_id.is_empty() {
            format!("rinch-field-{}", NEXT_FIELD.fetch_add(1, Ordering::Relaxed))
        } else {
            app_id.to_string()
        };
        Self { field }
    }

    fn label(&self) -> String {
        format!("{}-label", self.field)
    }

    /// Write the association onto the rendered parts.
    ///
    /// `labelable` says whether `field` is an HTML labelable element (`input`,
    /// `textarea`, …), which a label's `for` can reach. When it is not — a
    /// `div role="combobox"` — the label gets no `for` (it would name nothing
    /// on either backend) and the field names the label in `aria-labelledby`
    /// instead; the caller then forwards the label's press itself.
    pub(crate) fn wire(
        &self,
        field: &NodeHandle,
        labelable: bool,
        label: Option<&NodeHandle>,
        description: Option<&NodeHandle>,
        error: Option<&NodeHandle>,
    ) {
        field.set_attribute("id", &self.field);
        if let Some(label) = label {
            label.set_attribute("id", &self.label());
            if labelable {
                label.set_attribute("for", &self.field);
            } else {
                field.set_attribute("aria-labelledby", &self.label());
            }
        }
        let mut described_by = Vec::new();
        if let Some(error) = error {
            let id = format!("{}-error", self.field);
            error.set_attribute("id", &id);
            described_by.push(id);
            field.set_attribute("aria-invalid", "true");
        }
        if let Some(description) = description {
            let id = format!("{}-description", self.field);
            description.set_attribute("id", &id);
            described_by.push(id);
        }
        if !described_by.is_empty() {
            field.set_attribute("aria-describedby", &described_by.join(" "));
        }
    }
}
