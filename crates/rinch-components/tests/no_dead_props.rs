//! Every `pub` field of a component struct must be read by that component
//! (issue #474).
//!
//! A prop that `render` never reads compiles, documents, appears in `Debug`,
//! and does nothing. There is no warning for it and no type that forbids it:
//! #474 found **36** such props across 21 of this crate's 72 components, 34 of
//! them documented in `docs/src/guide/component-props.md` as if they worked.
//! This test is the ratchet the issue asked for — it reads the crate's own
//! sources and fails when a component declares a field no `impl` of that same
//! component mentions.
//!
//! ## What it checks
//!
//! For every struct with an `impl Component for S`, every `pub` field `f` of
//! `S` must appear as `self.f` somewhere in an `impl` **of `S`** other than its
//! `Debug` and `Default` impls. Reading a field only in `Default` is exactly
//! the trap that hid `TextInput::radius`: `close_on_escape: true` in a `Default`
//! impl reads like working code.
//!
//! ## Why source text rather than something stronger
//!
//! Rust cannot enumerate a struct's fields at runtime, and no derive can
//! observe whether `render` *read* one. The two mechanical alternatives are a
//! `#[component]` macro lint — which would catch none of these, since every
//! component here is a hand-written `struct` + `impl Component` and the macro is
//! for *user* components — and destructuring `self` in `render` without `..`,
//! which is compiler-enforced but is 59 files of churn and fights the `Debug`
//! impls. So: source text, today, with the debt visible in `ALLOWLIST` — which
//! #707 took from nine entries to one.
//!
//! ## Known limits (this is a floor, not a ceiling)
//!
//! It finds props nothing reads. It does **not** find:
//!
//! - a prop read only inside a helper that is itself never called, or one read
//!   into a value that is then discarded;
//! - a prop **read into a custom property no stylesheet spends** — the read is
//!   real, the effect is nil. `Modal::overlay_opacity` was exactly that until
//!   #646 wired the two sheets, and #474's hand sweep caught it where this scan
//!   could not;
//! - a prop reached through a binding rather than `self` (`let me = self; …
//!   me.radius`) — that reports a false *positive*, which errs safe;
//! - a `self.field` inside a **comment or a string literal**, which counts as a
//!   read. Measured; no component does it today.
//!
//! A field read by *any* impl of its own struct counts as read, including one
//! only a `class_string()` helper touches — which is right, since that helper's
//! output is what `render` applies.
//!
//! One latent hazard in the parser rather than the rule: [`top_level_items`]
//! ends an item at a column-0 `}` or `;`, so a raw string containing column-0
//! CSS in a scanned file (any `.rs` under `src/` outside `styles/`, child
//! module directories included) would corrupt the item ranges. None does
//! today — the CSS all lives in `src/styles/`, which this scan does not read.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Props that are declared, documented, and still not wired — the debt #474
/// catalogued, with the sub-cluster each belongs to. Every entry must name a
/// live reason: **this list may only shrink**. It started at 31, stood at 9
/// after #474's own three PRs, and #707 took eight of those nine.
///
/// Three of #474's four categories are closed. **A** (overlay behaviour) landed
/// with the dismiss stack, `data-trap-focus` and `DomScroll::set_scroll_locked`;
/// **B** (styling) with the five `radius` props, `CloseButton::icon_size` and
/// the two `overlay_opacity`; **C** (a parent prop whose child's twin works)
/// with `List::icon`, the two `Stepper` icons, `Stepper::allow_next_steps_select`,
/// `RadioGroup::size` and `Accordion::disable_chevron_rotation`. **D** (content
/// and input props) is all but one: the two `description`s landed.
///
/// `Textarea::max_rows` was the one exception to all of this — wired during
/// #707, then **reverted**, because a `max-height` could not yet do what the
/// prop means on a rinch `<textarea>`: before #297/#1152 the control's
/// content height was a `min-height` rinch wrote, and one `min-height` always
/// beats another `max-height` in CSS. #715 closed that gap (the control's
/// `rows` is now a Taffy *measure*, so `max-height` binds), and `max_rows` is
/// wired for real — the list is empty again.
const ALLOWLIST: &[(&str, &str)] = &[];

/// One top-level item: the header line that opens it plus its whole text.
struct Item {
    header: String,
    text: String,
}

/// Split a source file into top-level items.
///
/// Ranges come from column-0 headers and column-0 terminators rather than brace
/// matching, which would be wrong anyway — these render bodies are full of
/// braces inside string literals and `rsx!` blocks. A terminator is a column-0
/// `}` (a braced item) or a column-0 line ending in `;` (a `use`, a unit
/// struct, a `type`, the `];` closing a const array). Statements inside an item
/// are always indented, this crate being rustfmt-clean — so a column-0 `;`
/// cannot be one.
fn top_level_items(src: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut current: Option<Vec<&str>> = None;

    for line in src.lines() {
        let starts_at_column_0 = !line.is_empty() && !line.starts_with(char::is_whitespace);

        match current.as_mut() {
            Some(buf) => buf.push(line),
            None if starts_at_column_0 => current = Some(vec![line]),
            None => continue,
        }

        let trimmed = line.trim_end();
        if starts_at_column_0 && (trimmed == "}" || trimmed.ends_with(';')) {
            let lines = current.take().expect("an item is open");
            let header = lines
                .iter()
                .find(|l| {
                    let t = l.trim_start();
                    !t.starts_with("//") && !t.starts_with("#[") && !t.is_empty()
                })
                .unwrap_or(&"")
                .to_string();
            items.push(Item {
                header,
                text: lines.join("\n"),
            });
        }
    }

    items
}

/// The name a `struct`/`impl` header declares or implements for, i.e. the last
/// path segment before a `{`, `<`, or `(`.
fn type_name_after(header: &str, keyword: &str) -> Option<String> {
    let rest = header.split_once(keyword)?.1.trim_start();
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Is `impl … for <name>` (or `impl <name>`) an impl of `name` whose body may
/// legitimately read fields for display purposes only?
fn is_boilerplate_impl(header: &str) -> bool {
    header.contains("fmt::Debug for")
        || header.contains(" Debug for")
        || header.contains("Default for")
}

/// Does `text` mention `self . <field>` as a whole field name?
///
/// **Whitespace between `self`, the dot and the field is tolerated, and that is
/// load-bearing.** `cargo fmt` wraps a long expression as `self`⏎`.overlay_opacity`,
/// and a literal `self.<field>` search calls that field unread — so a reformat
/// with no behaviour change whatsoever flips a prop's status and turns this test
/// red or green for nothing. Measured on `Drawer::overlay_opacity`, which sat in
/// the allowlist for exactly that reason until #646.
fn reads_field(text: &str, field: &str) -> bool {
    let mut from = 0;
    while let Some(at) = text[from..].find("self") {
        let start = from + at;
        from = start + "self".len();

        // `myself.field` is not a read of `self`.
        if text[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }

        let Some(after_dot) = text[from..].trim_start().strip_prefix('.') else {
            continue;
        };
        let Some(tail) = after_dot.trim_start().strip_prefix(field) else {
            continue;
        };
        if !tail.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
            return true;
        }
    }
    false
}

/// `pub <name>:` field names declared inside a struct item.
fn pub_fields(item: &Item) -> Vec<String> {
    item.text
        .lines()
        .skip(1)
        .filter_map(|l| {
            let t = l.trim_start();
            let rest = t.strip_prefix("pub ")?;
            let (name, _) = rest.split_once(':')?;
            let name = name.trim();
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                .then(|| name.to_string())
        })
        .collect()
}

/// The `.rs` files under `dir`, recursively, sorted.
fn rs_files_under(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("a readable directory")
        .map(|e| e.expect("a readable dir entry").path())
        .collect();
    entries.sort();
    let mut out = Vec::new();
    for path in entries {
        if path.is_dir() {
            out.extend(rs_files_under(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// The crate's top-level modules, each as the files that make it up: `foo.rs`
/// together with everything under `foo/` (its child modules, such as
/// `color_input/classes.rs`), or a `foo/` directory module on its own
/// (`stepper/mod.rs` and its children). A component's impls may sit in any
/// file of its module. `styles/` is not read: the CSS lives there (see the
/// parser hazard in the module docs), and it declares no component.
fn module_sources(src_dir: &Path) -> Vec<Vec<PathBuf>> {
    let mut entries: Vec<_> = std::fs::read_dir(src_dir)
        .expect("the crate has a src/ directory")
        .map(|e| e.expect("a readable dir entry").path())
        .collect();
    entries.sort();
    let mut modules = Vec::new();
    for path in &entries {
        if path.file_name().is_some_and(|n| n == "styles") {
            continue;
        }
        if path.is_dir() {
            // A directory module, unless the `foo.rs` beside it owns it.
            if !path.with_extension("rs").exists() {
                modules.push(rs_files_under(path));
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            let mut files = vec![path.clone()];
            let children = path.with_extension("");
            if children.is_dir() {
                files.extend(rs_files_under(&children));
            }
            modules.push(files);
        }
    }
    modules
}

/// Every `<Struct>::<field>` in this crate that no impl of `<Struct>` reads.
fn unread_props() -> BTreeSet<String> {
    // The run-time `CARGO_MANIFEST_DIR`, which `cargo test` sets for the test
    // process, before the one baked in at compile time: a shared target dir can
    // hand this test the binary a since-deleted git worktree built, and the
    // baked path would still point into that worktree (#275). The baked one is
    // only the fallback for running this binary by hand.
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let src_dir = manifest_dir.join("src");
    let mut unread = BTreeSet::new();
    let mut components_seen = 0usize;

    for files in module_sources(&src_dir) {
        // Each file is split on its own, since an item cannot span two files;
        // the items of a module and its child modules are then scanned as one,
        // so a field read in `color_input/classes.rs` counts as read.
        let items: Vec<Item> = files
            .iter()
            .flat_map(|path| {
                let src = std::fs::read_to_string(path).expect("a readable source file");
                top_level_items(&src)
            })
            .collect();

        // Structs with an `impl Component for S`.
        let component_names: BTreeSet<String> = items
            .iter()
            .filter_map(|i| type_name_after(&i.header, "impl Component for "))
            .collect();

        for name in &component_names {
            let Some(decl) = items
                .iter()
                .find(|i| type_name_after(&i.header, "struct ").as_deref() == Some(name.as_str()))
            else {
                continue;
            };
            components_seen += 1;

            // Every impl of this struct that is not Debug/Default.
            let reading_text: String = items
                .iter()
                .filter(|i| i.header.starts_with("impl"))
                .filter(|i| !is_boilerplate_impl(&i.header))
                .filter(|i| {
                    let target = match i.header.split_once(" for ") {
                        Some((_, tail)) => type_name_after(&format!("for {tail}"), "for "),
                        None => type_name_after(&i.header, "impl "),
                    };
                    target.as_deref() == Some(name.as_str())
                })
                .map(|i| i.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            for field in pub_fields(decl) {
                if field.starts_with('_') {
                    continue; // an internal channel, not a prop
                }
                if !reads_field(&reading_text, &field) {
                    unread.insert(format!("{name}::{field}"));
                }
            }
        }
    }

    // Positive control: the scan found components at all. A parser that matched
    // nothing would otherwise report a clean tree (see "the silences").
    assert!(
        components_seen >= 60,
        "the scan found only {components_seen} component structs — the parser, \
         not the crate, is what changed"
    );

    unread
}

#[test]
fn every_component_prop_is_read_or_allowlisted() {
    let unread = unread_props();
    let allowed: BTreeSet<&str> = ALLOWLIST.iter().map(|(p, _)| *p).collect();

    let undeclared: Vec<&String> = unread
        .iter()
        .filter(|p| !allowed.contains(p.as_str()))
        .collect();

    assert!(
        undeclared.is_empty(),
        "these component props are declared and never read — wire them, or add \
         them to ALLOWLIST with the issue that tracks the behaviour:\n{}",
        undeclared
            .iter()
            .map(|p| format!("  {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_allowlist_does_not_outlive_its_entries() {
    let unread = unread_props();

    let stale: Vec<&str> = ALLOWLIST
        .iter()
        .map(|(p, _)| *p)
        .filter(|p| !unread.contains(*p))
        .collect();

    assert!(
        stale.is_empty(),
        "these ALLOWLIST entries are now read — delete them, so the list keeps \
         meaning what it says:\n{}",
        stale
            .iter()
            .map(|p| format!("  {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
