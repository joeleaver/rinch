//! Seeded round-trip fuzz for the Markdown writer and strict reader (from the
//! review of #1242): a random starter-kit document written with
//! `doc_to_markdown` must read back with `doc_from_markdown_strict` as the same
//! document, and writing it again must change nothing.
//!
//! The generator leaves out what the format is known not to keep, so a failure
//! here is a new loss, not a known one:
//! - whitespace at a textblock's edge (CommonMark strips it), and at the edge
//!   of a bold, italic, strike or link run (written outside the run, by design);
//! - two adjacent ordered lists or blockquotes (they merge, #1366; bullet and
//!   task lists are written with alternating bullets and stay apart);
//! - empty paragraphs (dropped, #1366);
//! - a code block's language in a table cell written as HTML (#1366);
//! - a line break inside code (a code span is literal).
//!
//! `strict_reads_everything_the_writer_writes` generates those too, and asks
//! only that the strict reader accept what the writer wrote.
//!
//! `RINCH_MD_FUZZ_SEEDS` raises the seed count (default 1000 per mode).
#![cfg(feature = "markdown")]

use rinch_editor_core::serialize::{doc_from_markdown, doc_from_markdown_strict, doc_to_markdown};
use rinch_editor_core::{AttrValue, Attrs, Fragment, Mark, Node, Schema};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// Letters, digits and single spaces, with marks.
    Tame,
    /// Markdown's special characters, no marks.
    WildText,
    /// Special characters and marks.
    Wild,
}

struct Gen<'s> {
    schema: &'s Schema,
    rng: Rng,
    mode: Mode,
    pool: Vec<Mark>,
    /// Generate the losses too (edge whitespace, empty paragraphs, adjacent
    /// lists, languages in cells, oversized spans): for the
    /// property that strict reads whatever the writer writes.
    unruly: bool,
}

fn mark(schema: &Schema, name: &str, attrs: &[(&str, &str)]) -> Mark {
    let mt = schema.mark_type(name).unwrap();
    let a = Attrs::from_iter(
        attrs
            .iter()
            .map(|(k, v)| (*k, AttrValue::from(v.to_string()))),
    );
    Mark::new(mt.clone(), mt.compute_attrs(&a).unwrap())
}

/// The links come last: an image takes its mark from `pool[LINKS..]`.
const LINKS: usize = 11;

fn mark_pool(schema: &Schema) -> Vec<Mark> {
    vec![
        mark(schema, "bold", &[]),
        mark(schema, "italic", &[]),
        mark(schema, "strike", &[]),
        mark(schema, "underline", &[]),
        mark(schema, "highlight", &[]),
        mark(schema, "highlight", &[("color", "#ffee00")]),
        mark(schema, "text_color", &[("color", "#c0392b")]),
        mark(schema, "text_color", &[("color", "red")]),
        mark(schema, "subscript", &[]),
        mark(schema, "superscript", &[]),
        mark(schema, "code", &[]),
        mark(schema, "link", &[("href", "https://a.com/x")]),
        mark(schema, "link", &[("href", "https://a.com/p(1)")]),
        mark(
            schema,
            "link",
            &[("href", "/rel?q=1&r=2"), ("title", "a \"t\"\n2")],
        ),
        mark(schema, "link", &[("href", "mailto:a@b.c")]),
    ]
}

impl Gen<'_> {
    fn text(&mut self, line_breaks: bool) -> String {
        let n = 1 + self.rng.below(8);
        let mut s = String::new();
        if self.mode == Mode::Tame {
            for i in 0..n {
                if i > 0 && self.rng.chance(40) {
                    s.push(' ');
                }
                s.push(*self.rng.pick(&['a', 'b', 'c', 'x', 'y', 'Z', '1', 'é']));
            }
            return s;
        }
        const CH: &[char] = &[
            'a', 'b', 'Z', '1', '2', ' ', ' ', '*', '_', '`', '[', ']', '(', ')', '~', '\\', '#',
            '>', '-', '+', '=', '!', '<', '>', '&', ';', '|', ':', '.', '"', '\'', 'é', '漢', '/',
            '?', '{', '}', '$', '%', '^', '@', '\n',
        ];
        for _ in 0..n {
            let c = *self.rng.pick(CH);
            if c != '\n' || line_breaks {
                s.push(c);
            }
        }
        s
    }

    fn marks(&mut self) -> Vec<Mark> {
        if self.mode == Mode::WildText {
            return vec![];
        }
        let mut set: Vec<Mark> = Vec::new();
        for _ in 0..self.rng.below(4) {
            let m = self.rng.pick(&self.pool).clone();
            if !set.iter().any(|x| x.type_name() == m.type_name()) {
                set = m.add_to_set(&set);
            }
        }
        set
    }

    fn node(&mut self, name: &str, attrs: Attrs, kids: Vec<Node>) -> Node {
        self.schema
            .create_node(name, attrs, Fragment::from_children(kids))
            .unwrap()
    }

    /// Inline content: text merged where marks repeat, edge whitespace
    /// trimmed, never empty.
    fn inline(&mut self, allow_break: bool) -> Vec<Node> {
        let n = 1 + self.rng.below(5);
        let mut out: Vec<Node> = Vec::new();
        for i in 0..n {
            let roll = self.rng.below(100);
            let node = if roll < 8 && allow_break && i > 0 && i + 1 < n {
                self.node("hard_break", Attrs::new(), vec![])
            } else if roll < 14 {
                let src = *self.rng.pick(&["a.png", "https://x.y/i.png", "/p/a b.png"]);
                let mut pairs = vec![("src", AttrValue::from(src.to_string()))];
                if self.rng.chance(50) {
                    let alt = if self.mode == Mode::Tame {
                        "alt t".to_string()
                    } else {
                        self.text(true).trim().to_string()
                    };
                    if !alt.is_empty() {
                        pairs.push(("alt", AttrValue::from(alt)));
                    }
                }
                let img = self
                    .schema
                    .create_node("image", Attrs::from_iter(pairs), Fragment::empty())
                    .unwrap();
                if self.mode != Mode::WildText && self.rng.chance(30) {
                    let link = self.rng.pick(&self.pool[LINKS..]).clone();
                    img.with_marks(vec![link])
                } else {
                    img
                }
            } else {
                let marks = self.marks();
                let code = marks.iter().any(|m| m.type_name() == "code");
                let mut t = self.text(!code);
                let runs = marks
                    .iter()
                    .any(|m| matches!(m.type_name(), "bold" | "italic" | "strike" | "link"));
                if runs {
                    t = t.trim().to_string();
                }
                if t.is_empty() {
                    t = "q".to_string();
                }
                self.schema.text_with_marks(&t, marks).unwrap()
            };
            if let Some(last) = out.last_mut()
                && last.is_text()
                && node.is_text()
                && last.same_markup(&node)
            {
                let merged = format!("{}{}", last.text().unwrap(), node.text().unwrap());
                *last = self
                    .schema
                    .text_with_marks(&merged, last.marks().to_vec())
                    .unwrap();
                continue;
            }
            out.push(node);
        }
        if self.rng.chance(3) {
            // Shift+Enter on an empty line.
            let n = 1 + self.rng.below(2);
            return (0..n)
                .map(|_| self.node("hard_break", Attrs::new(), vec![]))
                .collect();
        }
        if self.unruly {
            return out;
        }
        // Whitespace at a textblock's or a line's edge is CommonMark's to
        // strip, until none is left there.
        let mut res = out;
        loop {
            let len = res.len();
            let mut next: Vec<Node> = Vec::new();
            for (i, n) in res.iter().enumerate() {
                let Some(text) = n.text() else {
                    next.push(n.clone());
                    continue;
                };
                let mut t = text;
                if i == 0 || res[i - 1].type_name() == "hard_break" {
                    t = t.trim_start_matches([' ', '\t']);
                }
                if i + 1 == len || res[i + 1].type_name() == "hard_break" {
                    t = t.trim_end_matches([' ', '\t']);
                }
                if t == text {
                    next.push(n.clone());
                } else if !t.is_empty() {
                    next.push(self.schema.text_with_marks(t, n.marks().to_vec()).unwrap());
                }
            }
            let done =
                next.len() == res.len() && next.iter().zip(&res).all(|(a, b)| a.text() == b.text());
            res = next;
            if done {
                break;
            }
        }
        if res.is_empty() {
            res = vec![self.schema.text("p").unwrap()];
        }
        res
    }

    fn para(&mut self, attrs: Attrs) -> Node {
        let inline = if self.unruly && self.rng.chance(5) {
            vec![]
        } else {
            self.inline(true)
        };
        self.node("paragraph", attrs, inline)
    }

    /// Blocks, with a paragraph between two lists or quotes of one type.
    fn blocks(&mut self, n: usize, depth: usize, in_cell: bool) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        for _ in 0..n {
            let b = self.block(depth, in_cell);
            if let Some(prev) = out.last()
                && !self.unruly
                && prev.type_name() == b.type_name()
                && matches!(b.type_name(), "ordered_list" | "blockquote")
            {
                let sep = self.schema.text("sep").unwrap();
                out.push(self.node("paragraph", Attrs::new(), vec![sep]));
            }
            out.push(b);
        }
        out
    }

    fn block(&mut self, depth: usize, in_cell: bool) -> Node {
        let roll = self.rng.below(if depth > 1 { 50 } else { 100 });
        match roll {
            0..=29 => self.para(Attrs::new()),
            30..=39 => {
                let level = 1 + self.rng.below(6) as i64;
                let inline = self.inline(true);
                self.node(
                    "heading",
                    Attrs::from_iter([("level", AttrValue::Int(level))]),
                    inline,
                )
            }
            40..=44 => {
                let mode = self.mode;
                self.mode = Mode::WildText;
                let mut text = self.text(false);
                if self.rng.chance(50) {
                    text.push('\n');
                    text.push_str(&self.text(false));
                }
                self.mode = mode;
                if text.is_empty() {
                    text.push('c');
                }
                let lang = if in_cell && !self.unruly {
                    ""
                } else {
                    *self.rng.pick(&["", "rust", "c++"])
                };
                let attrs = if lang.is_empty() {
                    Attrs::new()
                } else {
                    Attrs::from_iter([("language", AttrValue::from(lang.to_string()))])
                };
                let text = self.schema.text(&text).unwrap();
                self.node("code_block", attrs, vec![text])
            }
            45..=49 => self.node("horizontal_rule", Attrs::new(), vec![]),
            50..=59 => {
                let n = 1 + self.rng.below(2);
                let kids = self.blocks(n, depth + 1, in_cell);
                self.node("blockquote", Attrs::new(), kids)
            }
            60..=79 => {
                let ordered = self.rng.chance(50);
                let mut items = Vec::new();
                for _ in 0..1 + self.rng.below(3) {
                    let mut kids = vec![self.para(Attrs::new())];
                    if self.rng.chance(30) {
                        kids.push(self.block(depth + 1, in_cell));
                    }
                    items.push(self.node("list_item", Attrs::new(), kids));
                }
                if ordered {
                    let start = *self.rng.pick(&[1i64, 1, 3, 10]);
                    let attrs = Attrs::from_iter([("start", AttrValue::Int(start))]);
                    self.node("ordered_list", attrs, items)
                } else {
                    self.node("bullet_list", Attrs::new(), items)
                }
            }
            80..=89 => {
                // Task lists (#1365): an item starts with a paragraph or,
                // sometimes, any other block, and may hold more blocks.
                let mut items = Vec::new();
                for _ in 0..1 + self.rng.below(3) {
                    let checked = AttrValue::Bool(self.rng.chance(50));
                    let first = if self.rng.chance(75) {
                        self.para(Attrs::new())
                    } else {
                        self.block(depth + 1, in_cell)
                    };
                    let mut kids = vec![first];
                    if self.rng.chance(30) {
                        let next = self.block(depth + 1, in_cell);
                        if !self.unruly
                            && next.type_name() == kids[0].type_name()
                            && matches!(next.type_name(), "ordered_list" | "blockquote")
                        {
                            let sep = self.schema.text("sep").unwrap();
                            kids.push(self.node("paragraph", Attrs::new(), vec![sep]));
                        }
                        kids.push(next);
                    }
                    items.push(self.node(
                        "task_item",
                        Attrs::from_iter([("checked", checked)]),
                        kids,
                    ));
                }
                self.node("task_list", Attrs::new(), items)
            }
            _ => self.table(depth),
        }
    }

    fn table(&mut self, depth: usize) -> Node {
        let cols = 1 + self.rng.below(3);
        let rows = 1 + self.rng.below(3);
        let header = self.rng.chance(70);
        let aligns: Vec<&str> = (0..cols)
            .map(|_| *self.rng.pick(&["left", "left", "center", "right"]))
            .collect();
        let mut row_nodes = Vec::new();
        for r in 0..rows {
            let mut cells = Vec::new();
            let mut c = 0;
            while c < cols {
                let mut attrs = Attrs::new();
                if c + 1 < cols && self.rng.chance(10) {
                    attrs = attrs.with("colspan", AttrValue::Int(2));
                    c += 1;
                }
                if self.unruly && self.rng.chance(5) {
                    let span = *self.rng.pick(&[0i64, 1, 1001, 70_000, i64::MAX]);
                    let name = *self.rng.pick(&["colspan", "rowspan"]);
                    attrs = attrs.with(name, AttrValue::Int(span));
                }
                let pattrs = if aligns[c] == "left" {
                    Attrs::new()
                } else {
                    Attrs::from_iter([("text_align", AttrValue::from(aligns[c].to_string()))])
                };
                let mut blocks = vec![self.para(pattrs)];
                if depth < 2 && self.rng.chance(8) {
                    blocks.push(self.block(depth + 2, true));
                }
                let name = if header && r == 0 {
                    "table_header_cell"
                } else {
                    "table_cell"
                };
                cells.push(self.node(name, attrs, blocks));
                c += 1;
            }
            row_nodes.push(self.node("table_row", Attrs::new(), cells));
        }
        self.node("table", Attrs::new(), row_nodes)
    }
}

fn seeds() -> u64 {
    std::env::var("RINCH_MD_FUZZ_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000)
}

fn gen_doc(schema: &Schema, seed: u64, mode: Mode, unruly: bool) -> Node {
    let mut g = Gen {
        schema,
        rng: Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
        mode,
        pool: mark_pool(schema),
        unruly,
    };
    let n = 1 + g.rng.below(4);
    let blocks = g.blocks(n, 0, false);
    g.node("doc", Attrs::new(), blocks)
}

fn run(mode: Mode) {
    let schema = Schema::starter_kit();
    let seeds = seeds();
    let mut failures = Vec::new();
    for seed in 1..=seeds {
        let d = gen_doc(&schema, seed, mode, false);
        let md = doc_to_markdown(&d);
        let why = match doc_from_markdown_strict(&schema, &md) {
            Err(e) => format!("strict refused it: {e}"),
            Ok(back) if back != d => format!("read back as\n  {back:?}\nwant\n  {d:?}"),
            Ok(back) => {
                let again = doc_to_markdown(&back);
                if again == md {
                    continue;
                }
                format!("second write {again:?}")
            }
        };
        failures.push(format!("seed {seed}: {md:?}\n{why}"));
    }
    assert!(
        failures.is_empty(),
        "{mode:?}: {} of {seeds} documents did not round-trip; the first:\n{}",
        failures.len(),
        failures
            .iter()
            .take(show())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    );
}

#[test]
fn letters_and_marks_round_trip() {
    run(Mode::Tame);
}

#[test]
fn special_characters_round_trip() {
    run(Mode::WildText);
}

#[test]
fn special_characters_and_marks_round_trip() {
    run(Mode::Wild);
}

/// Whatever the writer writes, the strict reader accepts — the known losses
/// included: what a document loses on the way out must not make the note
/// unreadable.
#[test]
fn strict_reads_everything_the_writer_writes() {
    let schema = Schema::starter_kit();
    let seeds = seeds();
    let mut failures = Vec::new();
    for mode in [Mode::Tame, Mode::Wild] {
        for seed in 1..=seeds {
            let d = gen_doc(&schema, seed, mode, true);
            let md = doc_to_markdown(&d);
            if let Err(e) = doc_from_markdown_strict(&schema, &md) {
                failures.push(format!("{mode:?} seed {seed}: {md:?}\n{e}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} documents' Markdown was refused; the first:\n{}",
        failures.len(),
        failures
            .iter()
            .take(show())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    );
}

/// Random Markdown built from list, task-marker and block fragments (the
/// review of #1374): neither reader panics, strict reads what it accepts
/// exactly as lenient does, and a document that came out of it writes and
/// reads back with its task lists.
#[test]
fn random_markdown_reads_without_panic_and_strict_agrees_with_lenient() {
    const PIECES: &[&str] = &[
        "- ", "* ", "+ ", " ", "  ", "\t", "[ ] ", "[x] ", "[X] ", "[ ]", "[x]", "[", "]", "x",
        "\n", "\n", "\n\n", "> ", "# ", "1. ", "a", "b", "`", "\\", "\x0b", "\x0c", "---", "***",
        "```\n", "[\t] ", "[\x0b]", "<br>", "| a |\n", "===\n", "\r\n", "- [ ]\n", "- \t[x] ",
    ];
    let schema = Schema::starter_kit();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut failures = Vec::new();
    for _ in 0..seeds() * 20 {
        let len = 2 + rng.below(10);
        let md: String = (0..len).map(|_| *rng.pick(PIECES)).collect();
        let lenient = doc_from_markdown(&schema, &md).unwrap_or_else(|e| panic!("{md:?}: {e}"));
        let Ok(strict) = doc_from_markdown_strict(&schema, &md) else {
            continue;
        };
        if strict != lenient {
            failures.push(format!("{md:?}: strict {strict:?}\nlenient {lenient:?}"));
            continue;
        }
        // What the writer makes of it, strict accepts; and a task list in it
        // is still there (other content has the known losses).
        let written = doc_to_markdown(&strict);
        match doc_from_markdown_strict(&schema, &written) {
            Err(e) => failures.push(format!("{md:?} wrote {written:?}, refused: {e}")),
            Ok(back) => {
                if task_shape(&back) != task_shape(&strict) {
                    failures.push(format!(
                        "{md:?} wrote {written:?}: had {strict:?}\nread {back:?}"
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} strings; the first:\n{}",
        failures.len(),
        failures
            .iter()
            .take(show())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    );
}

/// Every task item of `node` in document order, as its checked state.
fn task_shape(node: &Node) -> Vec<bool> {
    let mut out = Vec::new();
    fn walk(n: &Node, out: &mut Vec<bool>) {
        if n.type_name() == "task_item" {
            out.push(n.attrs().get_bool("checked").unwrap_or(false));
        }
        for c in n.content().children() {
            walk(c, out);
        }
    }
    walk(node, &mut out);
    out
}

/// How many failures to print (`RINCH_MD_FUZZ_SHOW`, default 3).
fn show() -> usize {
    std::env::var("RINCH_MD_FUZZ_SHOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
}
