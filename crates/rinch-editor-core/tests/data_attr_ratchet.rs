//! The ratchet behind `RESERVED_DATA_ATTRS` (review of #1518): every
//! `"data-…"` string literal in the workspace's non-test sources is a name
//! rinch's code writes or reads on elements. An image keeps an app's data
//! attributes and the view stamps them on its host element, so a name the
//! runtime acts on that is **not** reserved can be pasted into a document and
//! change what rinch does (a pasted `data-tcm-item` made a press on the
//! picture run a text-menu item).
//!
//! Each literal must be reserved (`is_reserved_data_attr`) or on [`SAFE`], with
//! a reason. A new runtime attribute fails here until it is one or the other.

use std::path::{Path, PathBuf};

use rinch_editor_core::schema::is_reserved_data_attr;

/// Names rinch's sources use that an app may still carry, each with why it is
/// safe on an editor's element.
const SAFE: &[(&str, &str)] = &[
    // Not names: prefixes the code tests names against.
    (
        "data-on",
        "a prefix the debug HTML serializer skips; the runtime reads only the whole `data-on…` names, all reserved",
    ),
    // Component markers: written and read by one component on its own parts,
    // found under the component's own class, never by a walk from an
    // arbitrary element.
    ("data-active", "Stepper/Tabs markers on their own parts"),
    ("data-active-item", "Accordion's root, read by Accordion"),
    ("data-color", "Badge's root"),
    ("data-disable-chevron-rotation", "Accordion's root"),
    ("data-first", "Pagination's own buttons"),
    ("data-last", "Pagination's own buttons"),
    ("data-next", "Pagination's own buttons"),
    ("data-prev", "Pagination's own buttons"),
    ("data-page", "Pagination's own buttons"),
    ("data-icon-for", "Stepper's step icon box"),
    ("data-icon-has", "Stepper's step icon box"),
    ("data-icon-live", "Stepper's step icon box"),
    ("data-item-value", "Accordion's items"),
    ("data-list-icon", "List's items (`.rinch-list__item`)"),
    ("data-multiple", "Accordion's root"),
    ("data-panel-value", "Tabs' panels"),
    ("data-tab-value", "Tabs' tabs"),
    ("data-separator", "Breadcrumbs' separators"),
    ("data-size", "RadioGroup's default, on its radios"),
    ("data-state", "Stepper's steps (`.rinch-stepper__step`)"),
    ("data-step", "Stepper's steps"),
    ("data-step-derived", "Stepper's steps"),
    ("data-step-own-clickable", "Stepper's steps"),
    ("data-step-position", "Stepper's steps"),
    ("data-stepper-click-handler", "Stepper's steps"),
    ("data-stepper-settled", "Stepper's steps"),
    ("data-value", "Tree's rows"),
];

fn workspace_crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name != "tests" {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") && !name.ends_with("_tests.rs") && name != "tests.rs" {
            out.push(path);
        }
    }
}

/// The data attribute names in `source` outside comments and `#[cfg(test)]`
/// modules, with their line numbers:
///
/// - every `"data-…"` string literal: a whole name (`"data-rid"`), a prefix
///   (`"data-pm-"`), the bare `"data-"` fragment a `concat!` builds a name
///   from, or a `format!` template (`"data-{x}"`), reported as written so a
///   built name never passes as a plain one;
/// - every `[data-…` in a CSS selector (`closest("[data-rid]")`), the shape
///   the web backend reads names with.
///
/// A test module is skipped from its `mod … {` line to the `}` at the same
/// indentation (a brace inside a string in it does not end the skip early or
/// late).
fn literals(source: &str) -> Vec<(String, usize)> {
    fn name_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '-' || c == '_'
    }
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let module = lines.get(i + 1).filter(|l| {
            let l = l.trim_start();
            let l = l
                .strip_prefix("pub(crate) ")
                .or(l.strip_prefix("pub "))
                .unwrap_or(l);
            l.starts_with("mod ") && l.trim_end().ends_with('{')
        });
        if line.trim() == "#[cfg(test)]"
            && let Some(module) = module
        {
            let indent = &module[..module.len() - module.trim_start().len()];
            let close = format!("{indent}}}");
            let mut j = i + 2;
            while j < lines.len() && lines[j].trim_end() != close {
                j += 1;
            }
            i = j + 1;
            continue;
        }
        if !line.trim_start().starts_with("//") {
            // String literals that start with `data-`.
            let mut rest = line;
            while let Some(at) = rest.find("\"data-") {
                let after = &rest[at + 1..];
                let Some(end) = after[1..].find('"').map(|e| e + 1) else {
                    break;
                };
                let lit = &after[..end];
                if lit.chars().all(name_char) || lit.contains('{') {
                    out.push((lit.to_string(), i + 1));
                }
                rest = &after[end..];
            }
            // Attribute selectors.
            let mut rest = line;
            while let Some(at) = rest.find("[data-") {
                let after = &rest[at + 1..];
                let end = after.find(|c: char| !name_char(c)).unwrap_or(after.len());
                out.push((after[..end].to_string(), i + 1));
                rest = &after[end..];
            }
        }
        i += 1;
    }
    out
}

/// The one file that may hold the bare `"data-"` fragment: the name rule
/// itself. Anywhere else it builds a name the ratchet cannot read.
const NAME_RULE: &str = "rinch-editor-core/src/schema/data_attrs.rs";

#[test]
fn every_data_attribute_rinch_uses_is_reserved_or_known_safe() {
    let crates = workspace_crates();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap().flatten() {
        rust_files(&entry.path().join("src"), &mut files);
    }
    files.sort();
    let mut seen = 0usize;
    let mut unknown = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for (name, line) in literals(&source) {
            seen += 1;
            let reserved = name != "data-" && !name.contains('{') && is_reserved_data_attr(&name);
            let safe = SAFE.iter().any(|(s, _)| *s == name)
                || (name == "data-" && file.ends_with(NAME_RULE));
            if !reserved && !safe {
                unknown.push(format!(
                    "{name} at {}:{line}",
                    file.strip_prefix(&crates).unwrap().display()
                ));
            }
        }
    }
    // A positive control: the scan reaches the runtime's own sources.
    assert!(files.iter().any(|f| f.ends_with("text_context_menu.rs")));
    assert!(
        seen > 100,
        "only {seen} literals found: the scan read nothing"
    );
    assert!(
        unknown.is_empty(),
        "data attributes rinch's code uses that are neither reserved \
         (rinch_editor_core::schema::RESERVED_DATA_ATTRS / _PREFIXES) nor in this \
         test's SAFE list:\n{}",
        unknown.join("\n")
    );
}

/// Every [`SAFE`] entry is still used and not reserved: the list does not keep
/// names that no longer need it.
#[test]
fn the_safe_list_holds_only_names_that_need_it() {
    for (name, _) in SAFE {
        assert!(!is_reserved_data_attr(name), "{name} is reserved already");
    }
}

/// The scanner itself: test modules and comments are skipped, literals in code
/// are found.
#[test]
fn the_scanner_finds_code_literals_only() {
    let src = r#"
fn f() { el.set_attribute("data-found", "1"); }
// "data-in-comment"
#[cfg(test)]
mod tests {
    fn g() { x("data-in-test"); }
}
fn h() { y("data-after"); z(format!("data-{x}")); }
"#;
    let names: Vec<String> = literals(src).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["data-found", "data-after", "data-{x}"]);
}

/// Review 2: the scanner's former blind spots — a selector, a `format!` and a
/// `concat!`-built name, and a literal after a test module holding an
/// unbalanced `"{"` — are all seen.
#[test]
fn review2_the_scanner_sees_selectors_and_built_names() {
    let src = r#"
fn a(el: &E) {
    el.get_attribute("data-zap-plain");
    el.closest("[data-zap-sel]");
    el.get_attribute(&format!("data-{}", "zap-fmt"));
    el.get_attribute(concat!("data-", "zap-concat"));
}
#[cfg(test)]
mod t {
    const S: &str = "{";
}
fn b(el: &E) {
    el.get_attribute("data-zap-after-test-mod");
}
"#;
    let names: Vec<String> = literals(src).into_iter().map(|(n, _)| n).collect();
    for want in [
        "data-zap-plain",
        "data-zap-sel",
        "data-{}",
        "data-",
        "data-zap-after-test-mod",
    ] {
        assert!(
            names.iter().any(|n| n == want),
            "{want} not seen: {names:?}"
        );
    }
}
