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
    ("data-", "the `data-` prefix itself (the name rule)"),
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

/// The `"data-…"` literals in `source` outside comments and `#[cfg(test)]`
/// modules, with their line numbers.
fn literals(source: &str) -> Vec<(String, usize)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let next_is_mod = lines.get(i + 1).is_some_and(|l| {
            let l = l.trim_start();
            let l = l
                .strip_prefix("pub(crate) ")
                .or(l.strip_prefix("pub "))
                .unwrap_or(l);
            l.starts_with("mod ") && l.trim_end().ends_with('{')
        });
        if line.trim() == "#[cfg(test)]" && next_is_mod {
            // Skip the module: from its `{` to the brace that closes it.
            let mut depth = 0i64;
            let mut j = i + 1;
            while j < lines.len() {
                depth += lines[j].matches('{').count() as i64;
                depth -= lines[j].matches('}').count() as i64;
                if depth <= 0 {
                    break;
                }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        if !line.trim_start().starts_with("//") {
            let mut rest = line;
            while let Some(at) = rest.find("\"data-") {
                let after = &rest[at + 1..];
                let end = after[1..].find('"').map(|e| e + 1);
                if let Some(end) = end {
                    let lit = &after[..end];
                    if lit
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    {
                        out.push((lit.to_string(), i + 1));
                    }
                    rest = &after[end..];
                } else {
                    break;
                }
            }
        }
        i += 1;
    }
    out
}

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
            let reserved = is_reserved_data_attr(&name);
            let safe = SAFE.iter().any(|(s, _)| *s == name);
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
    assert_eq!(names, ["data-found", "data-after"]);
}
