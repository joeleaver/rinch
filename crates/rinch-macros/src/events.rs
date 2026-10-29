//! The `on*` event props `rsx!` accepts on an HTML or SVG element.
//!
//! Each one is written as the `data-` attribute the runtime dispatches it from
//! (both backends read the same names). The list used to be open-ended: a name
//! the codegen did not match fell through to `data-rid`, the **click**
//! handler, so `input { onsubmit: … }` ran on a click and never on Enter, and
//! `onkeydown`, `onfocus` or a typo (`onclik`) were click handlers too
//! (issue #1590). A name outside this list is now a compile error, with a
//! "did you mean" for a near miss, like an unknown tag (#528).
//!
//! Components are not checked here: an `on*` prop on a component is one of
//! its struct fields, and the compiler already rejects a field it lacks.

/// Every supported event, with the attribute its handler id is written to.
///
/// The first nine take handlers of their own shape (`Fn(String)`,
/// `Fn(ScrollEvent)`, `Fn(Vec<PathBuf>)`, `Fn(Vec<LocalFile>)` (issue #1771),
/// `Fn(&KeyEventData) -> bool` for the two key events, issue #1678, and
/// `Fn(&WheelEvent) -> bool` for `onwheel`) and have their own arms in the
/// codegen;
/// the rest are `Fn()` handlers registered with `register_handler`.
pub const HTML_EVENTS: &[(&str, &str)] = &[
    ("onclick", "data-rid"),
    ("oninput", "data-oninput"),
    ("onchange", "data-onchange"),
    ("onscroll", "data-onscroll"),
    ("onfiledrop", "data-onfiledrop"),
    ("ondropfiles", "data-ondropfiles"),
    ("onkeydown", "data-onkeydown"),
    ("onkeyup", "data-onkeyup"),
    ("onwheel", "data-onwheel"),
    ("onsubmit", "data-onsubmit"),
    ("onmousedown", "data-onmousedown"),
    ("onmouseup", "data-onmouseup"),
    ("onmousemove", "data-onmousemove"),
    ("onmouseenter", "data-onenter"),
    ("onmouseleave", "data-onleave"),
    ("oncontextmenu", "data-oncontextmenu"),
    ("ondragstart", "data-ondragstart"),
    ("ondragmove", "data-ondragmove"),
    ("ondragenter", "data-ondragenter"),
    ("ondragover", "data-ondragover"),
    ("ondragleave", "data-ondragleave"),
    ("ondrop", "data-ondrop"),
    ("ondragend", "data-ondragend"),
    ("onfiledragenter", "data-onfiledragenter"),
    ("onfiledragleave", "data-onfiledragleave"),
];

/// The attribute a supported `Fn()` event is written to, or `None` for a name
/// `rsx!` does not support on an HTML element.
pub fn event_attribute(name: &str) -> Option<&'static str> {
    HTML_EVENTS
        .iter()
        .find(|(event, _)| *event == name)
        .map(|(_, attr)| *attr)
}

/// The diagnostic for an `on*` prop the macro does not support.
pub fn unknown_event_error(name: &str, span: proc_macro2::Span) -> syn::Error {
    let budget = if name.len() >= 5 { 2 } else { 1 };
    let suggestion = HTML_EVENTS
        .iter()
        .map(|(event, _)| (*event, crate::tags::edit_distance(name, event)))
        .filter(|(_, d)| *d <= budget)
        .min_by_key(|(_, d)| *d)
        .map(|(event, _)| format!("help: did you mean `{event}`?\n"))
        .unwrap_or_default();
    let supported: Vec<&str> = HTML_EVENTS.iter().map(|(event, _)| *event).collect();
    syn::Error::new(
        span,
        format!(
            "`{name}` is not an event `rsx!` supports on an HTML element\n\
             {suggestion}note: the supported events are {}\n\
             note: focus events have no element attribute; keys are \
             `onkeydown` / `onkeyup`",
            supported.join(", "),
        ),
    )
}

/// The diagnostic for a `*_fn` prop on an HTML element (issue #1591).
///
/// `value_fn`, `checked_fn`, `opened_fn` … are a **component** convention: a
/// prop that updates the component's DOM without re-rendering it. On an HTML
/// element every attribute already takes a closure and updates surgically, so
/// the `_fn` form has nothing to add — and it used to be written as a literal
/// `value_fn` attribute that nothing reads.
pub fn fn_prop_error(name: &str, span: proc_macro2::Span) -> syn::Error {
    let attr = name.strip_suffix("_fn").unwrap_or(name);
    syn::Error::new(
        span,
        format!(
            "`{name}` is a component prop; on an HTML element write \
             `{attr}: {{|| …}}`\n\
             note: every attribute of an HTML element takes a reactive closure \
             and updates only that attribute when the signals it reads change"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_has_a_data_attribute_and_names_are_unique() {
        let mut names: Vec<&str> = HTML_EVENTS.iter().map(|(e, _)| *e).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), HTML_EVENTS.len(), "duplicate event name");
        for (event, attr) in HTML_EVENTS {
            assert!(event.starts_with("on"), "{event}");
            assert!(*attr == "data-rid" || attr.starts_with("data-on"), "{attr}");
        }
    }
}
