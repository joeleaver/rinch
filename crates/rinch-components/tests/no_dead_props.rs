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
//! impls. So: source text, today, with the debt visible in `ALLOWLIST`.
//!
//! ## Known limits (this is a floor, not a ceiling)
//!
//! It finds props nothing reads. It does **not** find a prop read only inside a
//! helper that is itself never called, one read into a value that is then
//! discarded, or one reached through a binding rather than `self.` (no
//! component does that today). A field read by *any* impl of its own struct
//! counts as read, including one only a `class_string()` helper touches — which
//! is right, since that helper's output is what `render` applies.

use std::collections::BTreeSet;
use std::path::Path;

/// Props that are declared, documented, and still not wired — the debt #474
/// catalogued, with the sub-cluster each belongs to. Every entry must name a
/// live reason: this list may only shrink. It started at 31 and is 20; the
/// eleven that left were #474's whole `z_index` sub-cluster, its five dead
/// `radius` props and `CloseButton::icon_size`.
///
/// The overlay-behaviour cluster (#474 category A) is deliberately not wired:
/// `close_on_escape`, `trap_focus` and `lock_scroll` are new interaction work
/// against the focus arbiter (CLAUDE.md lists backdrop modality as *not yet*
/// implemented — these props are that gap, declared as if it were closed),
/// `overlay_opacity` needs a `Drawer` overlay to apply it to, and `auto_close`
/// is a timer.
const ALLOWLIST: &[(&str, &str)] = &[
    // #474 category A — overlay behaviour that does not exist yet.
    ("Modal::close_on_escape", "#474 A: focus/key behaviour"),
    ("Modal::lock_scroll", "#474 A: scroll locking"),
    ("Modal::trap_focus", "#474 A: focus arbiter work"),
    ("Drawer::close_on_escape", "#474 A: focus/key behaviour"),
    ("Drawer::lock_scroll", "#474 A: scroll locking"),
    ("Drawer::trap_focus", "#474 A: focus arbiter work"),
    ("Drawer::overlay_opacity", "#474 A: unapplied overlay var"),
    ("Popover::close_on_click_outside", "#474 A: no backdrop yet"),
    ("Popover::close_on_escape", "#474 A: focus/key behaviour"),
    ("Popover::trap_focus", "#474 A: focus arbiter work"),
    ("Notification::auto_close", "#474 A: dismiss timer"),
    // #474 category C — a parent prop whose child's twin works.
    ("List::icon", "#474 C: not plumbed to ListItem"),
    (
        "Stepper::completed_icon",
        "#474 C: not plumbed to StepperStep",
    ),
    (
        "Stepper::progress_icon",
        "#474 C: not plumbed to StepperStep",
    ),
    (
        "Stepper::allow_next_steps_select",
        "#474 C: selection gating",
    ),
    ("RadioGroup::size", "#474 C: not plumbed to Radio"),
    (
        "Accordion::disable_chevron_rotation",
        "#474 C: chevron is AccordionControl's",
    ),
    // #474 category D — content and input props.
    ("Checkbox::description", "#474 D: needs markup + CSS"),
    ("Switch::description", "#474 D: needs markup + CSS"),
    ("Textarea::max_rows", "#474 D: autosize bound"),
];

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

/// Does `text` mention `self.<field>` as a whole field name?
fn reads_field(text: &str, field: &str) -> bool {
    let needle = format!("self.{field}");
    let mut from = 0;
    while let Some(at) = text[from..].find(&needle) {
        let end = from + at + needle.len();
        let next = text[end..].chars().next();
        if !matches!(next, Some(c) if c.is_alphanumeric() || c == '_') {
            return true;
        }
        from = end;
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

/// Every `<Struct>::<field>` in this crate that no impl of `<Struct>` reads.
fn unread_props() -> BTreeSet<String> {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut unread = BTreeSet::new();
    let mut components_seen = 0usize;

    let mut files: Vec<_> = std::fs::read_dir(&src_dir)
        .expect("the crate has a src/ directory")
        .map(|e| e.expect("a readable dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.sort();

    for path in files {
        let src = std::fs::read_to_string(&path).expect("a readable source file");
        let items = top_level_items(&src);

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
