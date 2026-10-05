//! The HTML reader is total about text (#1397, #1392, #1401).
//!
//! For any HTML, every character of text a browser would show for that
//! fragment is in what `slice_from_html` returns, nothing a browser hides
//! (comments, conditional comments, `<style>`, `<script>`, `<head>`,
//! `<title>`) is, the result is valid content for a document, and reading back
//! the HTML written from it gives the same document.
//!
//! The generator below writes tag soup and knows, for each piece of text it
//! writes, whether a browser shows it. Each piece carries a token (`t17q`) no
//! other piece holds, so "shown exactly once" and "not shown" are checked
//! whatever the reader does with the structure around it. Where the generator
//! also knows the order (everything but markup closed early inside a table),
//! the document's text must equal the expected text character for character,
//! whitespace aside.
//!
//! `RINCH_HTML_TOTAL_SEEDS` raises the seed count (default 3000).
//! `RINCH_HTML_TOTAL_TALLY=1` counts the failures of each kind instead of
//! stopping at the first.

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{Node, Schema};
use std::collections::BTreeMap;

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
    fn pick<'a>(&mut self, of: &[&'a str]) -> &'a str {
        of[self.below(of.len())]
    }
}

/// How an element is closed.
#[derive(Clone, Copy, PartialEq)]
enum Close {
    Tag,
    Omitted,
}

struct Soup {
    rng: Rng,
    tokens: usize,
    /// Tokens a browser does not show.
    hidden: Vec<String>,
    /// Tokens a browser shows.
    shown: Vec<String>,
    /// Open `<table>`s (and runs of bare table parts) around what is being written.
    in_table: usize,
    /// False once something was written whose effect on the order of the text
    /// the generator does not model.
    order_known: bool,
    /// The cell written last has no end tag: what follows is inside it.
    cell_open: bool,
    /// End tags left out so far.
    omitted: usize,
}

const INLINE_TAGS: &[&str] = &[
    "span",
    "span style='mso-list:Ignore'",
    "span style=\"color:#ff0000;font-weight:700\"",
    "span lang=EN-US style='font-size:11.0pt;mso-bidi-font-size:10.0pt'",
    "SPAN CLASS=MsoNormal",
    "b",
    "B",
    "strong",
    "i",
    "em",
    "u",
    "s",
    "a href=\"https://e.x/?a=1&amp;b=2\"",
    "a name=\"_Toc1\"",
    "code",
    "sub",
    "sup",
    "font face=Arial",
    "label",
    "button type=button",
    "small",
    "abbr title=\"x > y\"",
    "o:p",
    "st1:place w:st=\"on\"",
    "st1:City",
    "w:sdt id=\"1\"",
    "my-widget data-x='a>b'",
    "google-sheets-html-origin",
    "math",
    "time",
    "q",
];

const BLOCK_TEXT_TAGS: &[&str] = &[
    "p",
    "p class=MsoNormal style='margin:0in'",
    "p class=MsoListParagraph style='text-indent:-.25in;mso-list:l0 level1 lfo1'",
    "P",
    "div",
    "div dir=\"auto\"",
    "h1",
    "h2",
    "h3",
    "h6",
    "center",
    "address",
    "summary",
    "figcaption",
    "dt",
    "dd",
    "legend",
];

const BLOCK_BOX_TAGS: &[&str] = &[
    "div",
    "div class=\"WordSection1\"",
    "section",
    "article",
    "blockquote",
    "form action=\"/x\"",
    "fieldset",
    "details open",
    "main",
    "x-card",
    "dl",
    "figure",
    "body",
];

/// Inline elements that are also written around blocks.
const WRAPPER_TAGS: &[&str] = &[
    "span",
    "b style=\"font-weight:normal\" id=\"docs-internal-guid-1\"",
    "strong",
    "em",
    "a href=\"https://e.x/card\"",
    "google-sheets-html-origin",
    "w:sdt",
    "o:p",
    "my-widget",
    "font size=2",
];

fn tag_name(open: &str) -> &str {
    open.split(' ').next().unwrap_or(open)
}

impl Soup {
    fn new(seed: u64) -> Self {
        Self {
            rng: Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
            tokens: 0,
            hidden: Vec::new(),
            shown: Vec::new(),
            in_table: 0,
            order_known: true,
            cell_open: false,
            omitted: 0,
        }
    }

    fn token(&mut self) -> String {
        self.tokens += 1;
        format!("t{}q", self.tokens)
    }

    fn shown_token(&mut self) -> String {
        let t = self.token();
        self.shown.push(t.clone());
        t
    }

    fn hidden_token(&mut self) -> String {
        let t = self.token();
        self.hidden.push(t.clone());
        t
    }

    /// Something whose effect on nesting the generator does not follow.
    fn risky(&mut self) {
        if self.in_table > 0 {
            self.order_known = false;
        }
    }

    fn close(&mut self, pct_omitted: usize) -> Close {
        if self.rng.chance(pct_omitted) {
            self.risky();
            self.omitted += 1;
            Close::Omitted
        } else {
            Close::Tag
        }
    }

    /// The end tag of an element the next start tag closes anyway (`<li>`,
    /// `<td>`, `<tr>`, a row group).
    fn close_implied(&mut self, pct_omitted: usize) -> Close {
        if self.rng.chance(pct_omitted) {
            self.omitted += 1;
            Close::Omitted
        } else {
            Close::Tag
        }
    }

    /// A piece of text; returns what a browser shows for it.
    fn text(&mut self, out: &mut String) -> String {
        let t = self.shown_token();
        let (src, shown) = match self.rng.below(14) {
            0 => (format!(" {t} "), t.clone()),
            1 => (format!("{t} &amp; "), format!("{t}&")),
            2 => (format!("&lt;{t}&gt;"), format!("<{t}>")),
            3 => (format!("{t}&nbsp;"), t.clone()),
            4 => (format!("{t} < "), format!("{t}<")),
            5 => (
                format!("{t}&#233;&eacute;&#x263A;&mdash;&rsquo; "),
                format!("{t}\u{e9}\u{e9}\u{263a}\u{2014}\u{2019}"),
            ),
            6 => (format!("\n  {t}\n"), t.clone()),
            7 => (format!("{t} > "), format!("{t}>")),
            8 => (format!("{t}&nosuchname; "), format!("{t}&nosuchname;")),
            9 => (format!("{t}<3 "), format!("{t}<3")),
            10 => (format!("{t} R&D "), format!("{t}R&D")),
            11 => (format!("{t}\u{e9}\u{4e2d}\u{1f600} "), format!("{t}\u{e9}\u{4e2d}\u{1f600}")),
            _ => (t.clone(), t.clone()),
        };
        out.push_str(&src);
        shown
    }

    /// Markup a browser shows nothing for (one kind shows what it wraps).
    fn hidden(&mut self, out: &mut String) -> String {
        match self.rng.below(15) {
            0 => {
                let t = self.hidden_token();
                out.push_str(&format!("<!-- {t} -->"));
            }
            1 => {
                let t = self.hidden_token();
                out.push_str(&format!(
                    "<!--[if gte mso 9]><xml><o:OfficeDocumentSettings><o:AllowPNG/>{t}\
                     </o:OfficeDocumentSettings></xml><![endif]-->"
                ));
            }
            2 => {
                let t = self.hidden_token();
                out.push_str(&format!(
                    "<style>p.MsoNormal{{margin:0}} /* {t} */ a > b {{}}</style>"
                ));
            }
            3 => {
                let t = self.hidden_token();
                out.push_str(&format!("<script>var a = \"{t} < b\"; if (a<b) {{}}</script>"));
            }
            4 => {
                let (a, b) = (self.hidden_token(), self.hidden_token());
                out.push_str(&format!(
                    "<head><meta charset=\"utf-8\"><title>{a}</title><style>{b}</style></head>"
                ));
            }
            5 => {
                let t = self.hidden_token();
                out.push_str(&format!("<?xml:namespace prefix = o ns = \"urn:{t}\" />"));
            }
            6 => out.push_str("<!DOCTYPE html>"),
            7 => {
                let (a, b) = (self.hidden_token(), self.hidden_token());
                out.push_str(&format!(
                    "<meta name=Generator content=\"{a}\"><link rel=File-List href=\"{b}.xml\">"
                ));
            }
            8 => out.push_str("<!--StartFragment-->"),
            9 => out.push_str("<!--EndFragment-->"),
            10 => {
                // Downlevel-revealed: both halves are comments, the middle shows.
                out.push_str("<![if !supportLists]>");
                let shown = self.text(out);
                out.push_str("<![endif]>");
                return shown;
            }
            11 => {
                let t = self.hidden_token();
                out.push_str(&format!("<title>{t}</title>"));
            }
            12 => {
                let t = self.hidden_token();
                out.push_str(&format!("<noscript>{t}</noscript>"));
            }
            13 => {
                let t = self.hidden_token();
                out.push_str(&format!("<span title=\"{t} > x\" data-a='<{t}>'></span>"));
            }
            _ => {
                let t = self.hidden_token();
                out.push_str(&format!("<!--[if !mso]><!-- -->{t}<!--<![endif]-->"));
                // `<!--[if !mso]><!-- -->` closes the comment at once: this
                // one is shown.
                self.hidden.pop();
                self.shown.push(t.clone());
                return t;
            }
        }
        String::new()
    }

    fn inline(&mut self, depth: usize, out: &mut String) -> String {
        let mut shown = String::new();
        for _ in 0..1 + self.rng.below(3) {
            let pick = if depth >= 4 { self.rng.below(5) } else { self.rng.below(16) };
            match pick {
                0..=3 => shown += &self.text(out),
                4 => out.push_str(self.rng.pick(&[
                    "<br>",
                    "<br/>",
                    "<BR clear=all>",
                    "<wbr>",
                    "<img src=\"https://e.x/i.png\" alt=\"alt\">",
                    "<img>",
                    "<o:p></o:p>",
                    "<o:p/>",
                    "<input type=hidden value=v>",
                    "<span></span>",
                ])),
                5..=9 => {
                    let open = self.rng.pick(INLINE_TAGS);
                    out.push_str(&format!("<{open}>"));
                    shown += &self.inline(depth + 1, out);
                    if self.close(8) == Close::Tag {
                        out.push_str(&format!("</{}>", tag_name(open)));
                    }
                }
                10 => shown += &self.hidden(out),
                11 => {
                    // Misnested: <b>A<i>B</b>C</i>
                    self.risky();
                    out.push_str("<b>");
                    shown += &self.text(out);
                    out.push_str("<i>");
                    shown += &self.text(out);
                    out.push_str("</b>");
                    shown += &self.text(out);
                    out.push_str("</i>");
                }
                12 => {
                    self.risky();
                    out.push_str(self.rng.pick(&[
                        "</span>", "</o:p>", "</b>", "</a>", "</font>", "</my-widget>", "</>",
                        "</ >",
                    ]));
                }
                13 => {
                    let t = self.shown_token();
                    out.push_str(&format!("<textarea>{t}</textarea>"));
                    shown += &t;
                }
                _ => shown += &self.text(out),
            }
        }
        shown
    }

    fn list(&mut self, depth: usize, out: &mut String) -> String {
        let mut shown = String::new();
        let open = self.rng.pick(&[
            "ul",
            "ol",
            "ol start=3",
            "ul style='margin-top:0in' type=disc",
            "ul data-type=\"taskList\"",
        ]);
        out.push_str(&format!("<{open}>"));
        for _ in 0..1 + self.rng.below(3) {
            match self.rng.below(10) {
                0 => shown += &self.text(out),
                1 if depth < 4 => shown += &self.list(depth + 1, out),
                2 => shown += &self.hidden(out),
                3 if depth < 4 => shown += &self.block(depth + 1, out),
                _ => {
                    out.push_str(self.rng.pick(&[
                        "<li>",
                        "<li class=MsoListParagraph>",
                        "<li data-type=\"taskItem\" data-checked=\"true\">",
                        "<LI>",
                    ]));
                    shown += &if depth < 4 && self.rng.chance(30) {
                        self.blocks(depth + 1, out)
                    } else {
                        self.inline(depth + 1, out)
                    };
                    if self.close_implied(25) == Close::Tag {
                        out.push_str("</li>");
                    }
                }
            }
        }
        if self.close(5) == Close::Tag {
            out.push_str(&format!("</{}>", tag_name(open)));
        }
        shown
    }

    /// A cell's markup and text.
    fn cell(&mut self, depth: usize, out: &mut String) -> String {
        let open = self.rng.pick(&[
            "td",
            "th",
            "td colspan=2",
            "td rowspan=\"2\" style='border:solid windowtext 1.0pt'",
            "TD width=64",
            "td data-sheets-value=\"{&quot;1&quot;:2}\"",
        ]);
        out.push_str(&format!("<{open}>"));
        let shown = if depth < 4 && self.rng.chance(30) {
            self.blocks(depth + 1, out)
        } else {
            self.inline(depth + 1, out)
        };
        self.cell_open = self.close_implied(25) == Close::Omitted;
        if !self.cell_open {
            out.push_str(&format!("</{}>", tag_name(open)));
        }
        shown
    }

    /// Content with no place in a table: a browser moves it in front.
    fn stray(&mut self, depth: usize, out: &mut String) -> String {
        match self.rng.below(4) {
            0 => {
                let mut shown = String::new();
                out.push_str("<div>");
                shown += &self.text(out);
                out.push_str("</div>");
                shown
            }
            1 if depth < 4 => {
                let mut shown = String::new();
                out.push_str("<b>");
                shown += &self.text(out);
                out.push_str("</b>");
                shown
            }
            2 => self.hidden(out),
            _ => self.text(out),
        }
    }

    /// A row. Returns (text moved in front of the table, the cells' text).
    fn row(&mut self, depth: usize, out: &mut String) -> (String, String) {
        let (mut front, mut cells) = (String::new(), String::new());
        out.push_str(self.rng.pick(&["<tr>", "<tr style='height:15.0pt'>", "<TR>"]));
        self.cell_open = false;
        for _ in 0..1 + self.rng.below(3) {
            if !self.cell_open && self.rng.chance(8) {
                front += &self.stray(depth, out);
            } else {
                cells += &self.cell(depth, out);
            }
        }
        if self.close_implied(25) == Close::Tag {
            out.push_str("</tr>");
            self.cell_open = false;
        }
        (front, cells)
    }

    /// The children of a `<table>`. Returns (in front, captions, cells).
    fn table_parts(&mut self, depth: usize, out: &mut String) -> (String, String, String) {
        let (mut front, mut captions, mut cells) = (String::new(), String::new(), String::new());
        if self.rng.chance(20) {
            out.push_str("<caption>");
            captions += &self.inline(depth + 1, out);
            out.push_str("</caption>");
        }
        if self.rng.chance(30) {
            out.push_str(self.rng.pick(&[
                "<colgroup><col width=\"100\"><col width=\"100\"></colgroup>",
                "<col width=64 span=2 style='width:48pt'>",
                "<colgroup></colgroup>",
            ]));
        }
        for _ in 0..1 + self.rng.below(3) {
            match self.rng.below(12) {
                0 if !self.cell_open => front += &self.stray(depth, out),
                1 | 2 => {
                    let group = self.rng.pick(&["tbody", "thead", "tfoot"]);
                    out.push_str(&format!("<{group}>"));
                    self.cell_open = false;
                    for _ in 0..1 + self.rng.below(2) {
                        if !self.cell_open && self.rng.chance(8) {
                            front += &self.stray(depth, out);
                        }
                        let (f, c) = self.row(depth, out);
                        front += &f;
                        cells += &c;
                    }
                    if self.close_implied(20) == Close::Tag {
                        out.push_str(&format!("</{group}>"));
                        self.cell_open = false;
                    }
                }
                3 => cells += &self.cell(depth, out),
                _ => {
                    let (f, c) = self.row(depth, out);
                    front += &f;
                    cells += &c;
                }
            }
        }
        (front, captions, cells)
    }

    fn table(&mut self, depth: usize, out: &mut String) -> String {
        self.in_table += 1;
        out.push_str(self.rng.pick(&[
            "<table>",
            "<table border=0 cellpadding=0 cellspacing=0 width=128 style='border-collapse:collapse'>",
            "<table xmlns=\"http://www.w3.org/1999/xhtml\" data-sheets-root=\"1\">",
            "<TABLE class=MsoTableGrid>",
        ]));
        self.cell_open = false;
        let (front, captions, cells) = self.table_parts(depth, out);
        out.push_str("</table>");
        self.cell_open = false;
        self.in_table -= 1;
        front + &captions + &cells
    }

    /// Table parts with no `<table>` around them: read as a table.
    fn bare_table_parts(&mut self, depth: usize, out: &mut String) -> String {
        self.in_table += 1;
        self.cell_open = false;
        let omitted = self.omitted;
        let (mut front, mut captions, mut cells) = (String::new(), String::new(), String::new());
        match self.rng.below(6) {
            0 => {
                for _ in 0..1 + self.rng.below(3) {
                    cells += &self.cell(depth, out);
                }
            }
            1 => {
                for _ in 0..1 + self.rng.below(3) {
                    let (f, c) = self.row(depth, out);
                    front += &f;
                    cells += &c;
                }
            }
            2 => {
                out.push_str("<tbody>");
                let (f, c) = self.row(depth, out);
                front += &f;
                cells += &c;
                out.push_str("</tbody>");
            }
            3 => {
                out.push_str("<caption>");
                captions += &self.inline(depth + 1, out);
                out.push_str("</caption>");
            }
            4 => {
                out.push_str("<col width=64><colgroup><col></colgroup>");
                let (f, c) = self.row(depth, out);
                front += &f;
                cells += &c;
            }
            _ => {
                out.push_str("<thead>");
                let (f, c) = self.row(depth, out);
                front += &f;
                cells += &c;
                out.push_str("</thead><tbody>");
                let (f, c) = self.row(depth, out);
                front += &f;
                cells += &c;
                out.push_str("</tbody>");
            }
        }
        self.in_table -= 1;
        self.cell_open = false;
        let mut after = String::new();
        if self.omitted != omitted {
            // What follows may be read into the part left open.
            self.order_known = false;
        } else {
            // Parts that follow these would be one table with them.
            out.push_str("<p>");
            after = self.text(out);
            out.push_str("</p>");
        }
        front + &captions + &cells + &after
    }

    fn block(&mut self, depth: usize, out: &mut String) -> String {
        let mut shown = String::new();
        let pick = if depth >= 4 { self.rng.below(6) } else { self.rng.below(24) };
        match pick {
            0..=5 => {
                let open = self.rng.pick(BLOCK_TEXT_TAGS);
                out.push_str(&format!("<{open}>"));
                shown += &self.inline(depth + 1, out);
                if self.close(15) == Close::Tag {
                    out.push_str(&format!("</{}>", tag_name(open)));
                }
            }
            6..=8 => {
                let open = self.rng.pick(BLOCK_BOX_TAGS);
                out.push_str(&format!("<{open}>"));
                shown += &self.blocks(depth + 1, out);
                if self.close(10) == Close::Tag {
                    out.push_str(&format!("</{}>", tag_name(open)));
                }
            }
            9 | 10 => {
                let open = self.rng.pick(WRAPPER_TAGS);
                out.push_str(&format!("<{open}>"));
                shown += &self.blocks(depth + 1, out);
                if self.close(10) == Close::Tag {
                    out.push_str(&format!("</{}>", tag_name(open)));
                }
            }
            11 | 12 => shown += &self.list(depth, out),
            13 => {
                // Items with no list around them.
                for _ in 0..1 + self.rng.below(3) {
                    out.push_str("<li>");
                    shown += &self.inline(depth + 1, out);
                    if self.close_implied(25) == Close::Tag {
                        out.push_str("</li>");
                    }
                }
            }
            14 | 15 => shown += &self.table(depth, out),
            16 | 17 => shown += &self.bare_table_parts(depth, out),
            18 => {
                out.push_str(self.rng.pick(&["<pre>", "<pre><code>", "<pre class=\"x\">\n"]));
                shown += &self.inline(depth + 1, out);
                out.push_str("</pre>");
            }
            19 => out.push_str(self.rng.pick(&["<hr>", "<br>", "<hr/>", "<hr size=2 width=\"100%\">"])),
            20 => shown += &self.hidden(out),
            21 => {
                self.risky();
                out.push_str(self.rng.pick(&[
                    "</div>", "</p>", "</td>", "</tr>", "</table>", "</li>", "</ul>", "</o:p>",
                    "</body>", "</html>", "</section>",
                ]));
            }
            _ => shown += &self.inline(depth + 1, out),
        }
        shown
    }

    fn blocks(&mut self, depth: usize, out: &mut String) -> String {
        let mut shown = String::new();
        for _ in 0..1 + self.rng.below(3) {
            shown += &self.block(depth, out);
            if self.rng.chance(20) {
                out.push_str(self.rng.pick(&["\n", "\r\n\r\n", "  ", "\n\t"]));
            }
        }
        shown
    }

    /// A whole clipboard payload.
    fn document(&mut self, out: &mut String) -> String {
        let wrapped = self.rng.below(4);
        match wrapped {
            0 => out.push_str(
                "<html xmlns:o=\"urn:schemas-microsoft-com:office:office\">\r\n<head><meta \
                 http-equiv=Content-Type content=\"text/html; charset=utf-8\"><style><!-- \
                 p.MsoNormal {margin:0in;} --></style></head><body lang=EN-US>\
                 <!--StartFragment-->",
            ),
            1 => out.push_str("<meta charset='utf-8'>"),
            2 => out.push_str("<html><body>"),
            _ => {}
        }
        let shown = self.blocks(0, out);
        match wrapped {
            0 => out.push_str("<!--EndFragment--></body>\r\n</html>"),
            2 if self.rng.chance(70) => out.push_str("</body></html>"),
            _ => {}
        }
        shown
    }
}

fn no_space(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn doc_text(node: &Node, out: &mut String) {
    if let Some(t) = node.text() {
        out.push_str(t);
    }
    for child in node.content().children() {
        doc_text(child, out);
    }
    if node.is_block() {
        out.push('\n');
    }
}

/// Every node's content satisfies its type's expression and carries only the
/// marks its parent allows.
fn validity(node: &Node) -> Result<(), String> {
    let names: Vec<&str> = node.content().children().iter().map(Node::type_name).collect();
    if !node.node_type().content_match().matches(&names) {
        return Err(format!("<{}> holds {names:?}", node.type_name()));
    }
    for child in node.content().children() {
        for mark in child.marks() {
            if !node.node_type().spec().marks.allows(mark.type_name()) {
                return Err(format!(
                    "<{}> holds a {} mark",
                    node.type_name(),
                    mark.type_name()
                ));
            }
        }
        validity(child)?;
    }
    Ok(())
}

/// The document `load_html` would build from `html`, or `None` for nothing.
fn load(schema: &Schema, html: &str) -> Result<Option<Node>, String> {
    let slice = slice_from_html(schema, html).map_err(|e| format!("refused: {e}"))?;
    if slice.content.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        schema
            .branch("doc", slice.content.clone())
            .map_err(|e| format!("no doc: {e}"))?,
    ))
}

/// What is wrong with how `html` reads, as (kind, detail).
fn check(
    schema: &Schema,
    html: &str,
    expected: &str,
    shown: &[String],
    hidden: &[String],
    order_known: bool,
) -> Result<(), (&'static str, String)> {
    let read = std::panic::catch_unwind(|| load(schema, html))
        .map_err(|_| ("panic", String::new()))?
        .map_err(|e| ("refused", e))?;
    let Some(doc) = read else {
        return if shown.is_empty() {
            Ok(())
        } else {
            Err(("text lost", format!("everything: {shown:?}")))
        };
    };
    validity(&doc).map_err(|e| ("invalid", e))?;
    let mut text = String::new();
    doc_text(&doc, &mut text);
    for t in shown {
        match text.matches(t.as_str()).count() {
            1 => {}
            0 => return Err(("text lost", format!("{t} of {text:?}"))),
            n => return Err(("text repeated", format!("{t} x{n} in {text:?}"))),
        }
    }
    for t in hidden {
        if text.contains(t.as_str()) {
            return Err(("hidden text shown", format!("{t} in {text:?}")));
        }
    }
    if order_known && no_space(&text) != no_space(expected) {
        return Err((
            "text differs",
            format!("\n   read {:?}\n wanted {:?}", no_space(&text), no_space(expected)),
        ));
    }
    let written = node_to_html(&doc);
    let again = load(schema, &written).map_err(|e| ("reload refused", e))?;
    let rewritten = again.as_ref().map(node_to_html).unwrap_or_default();
    if again.as_ref() != Some(&doc) {
        return Err((
            "not a fixed point",
            format!("\n  wrote {written}\n reread {rewritten}"),
        ));
    }
    Ok(())
}

#[test]
fn any_html_reads_to_a_valid_document_holding_all_of_its_text() {
    let schema = Schema::starter_kit();
    let seeds: u64 = std::env::var("RINCH_HTML_TOTAL_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);
    let tally = std::env::var("RINCH_HTML_TOTAL_TALLY").is_ok();
    if tally {
        std::panic::set_hook(Box::new(|_| {}));
    }
    let mut kinds: BTreeMap<&'static str, (usize, String)> = BTreeMap::new();
    let (mut with_order, mut tokens) = (0usize, 0usize);
    for seed in 1..=seeds {
        let mut soup = Soup::new(seed);
        let mut html = String::new();
        let expected = soup.document(&mut html);
        with_order += usize::from(soup.order_known);
        tokens += soup.shown.len();
        let result = check(
            &schema,
            &html,
            &expected,
            &soup.shown,
            &soup.hidden,
            soup.order_known,
        );
        if let Err((kind, detail)) = result {
            assert!(tally, "seed {seed}: {kind}: {detail}\n html {html}");
            let entry = kinds.entry(kind).or_insert((0, format!("seed {seed}: {html}")));
            entry.0 += 1;
        }
    }
    // The generator generates: most cases have text, and an order to check.
    assert!(tokens as u64 > seeds * 4, "{tokens} tokens in {seeds} cases");
    assert!(with_order as u64 * 2 > seeds, "{with_order} of {seeds} ordered");
    if tally {
        for (kind, (count, first)) in &kinds {
            println!("{count:6} {kind}   first: {first}");
        }
        println!("{seeds} seeds, {with_order} with a known order, {tokens} shown tokens");
    }
}

fn read(html: &str) -> String {
    let schema = Schema::starter_kit();
    match load(&schema, html).unwrap() {
        Some(doc) => {
            validity(&doc).unwrap_or_else(|e| panic!("{html}: {e}"));
            let written = node_to_html(&doc);
            // `node_to_html` of a doc writes its children.
            let again = load(&schema, &written).unwrap();
            assert_eq!(again.as_ref(), Some(&doc), "{html} is not a fixed point: {written}");
            written
        }
        None => String::new(),
    }
}

/// #1397: Word ends every paragraph with `<o:p></o:p>`; the reader stopped
/// at the first one.
#[test]
fn a_namespaced_element_does_not_end_the_read() {
    assert_eq!(read("<p>a<o:p></o:p></p><p>b</p>"), "<p>a</p><p>b</p>");
    assert_eq!(
        read("<p>a</p><o:p>&nbsp;</o:p><p>b</p>"),
        "<p>a</p><p>\u{a0}</p><p>b</p>"
    );
    assert_eq!(read("<p>a<o:p/></p><p>b</p>"), "<p>a</p><p>b</p>");
    assert_eq!(
        read("<p>to <st1:place w:st=\"on\"><st1:City>Paris</st1:City></st1:place> by</p><p>b</p>"),
        "<p>to Paris by</p><p>b</p>"
    );
    assert_eq!(
        read("<w:sdt><p>a</p><p>b</p></w:sdt><p>c</p>"),
        "<p>a</p><p>b</p><p>c</p>"
    );
}

/// An element the reader does not know is inline unless it holds a block:
/// a custom element in a sentence does not split the sentence.
#[test]
fn an_unknown_element_is_inline_unless_it_holds_a_block() {
    assert_eq!(read("a <my-chip>b</my-chip> c"), "<p>a b c</p>");
    assert_eq!(read("<p>a <my-chip>b</my-chip> c</p>"), "<p>a b c</p>");
    assert_eq!(
        read("<my-card><h2>t</h2><p>b</p></my-card>"),
        "<h2>t</h2><p>b</p>"
    );
    // A block-level HTML element is a line of its own.
    assert_eq!(read("a<div>b</div>c"), "<p>a</p><p>b</p><p>c</p>");
    assert_eq!(
        read("<span><div>a</div><div>b</div></span>"),
        "<p>a</p><p>b</p>"
    );
}

/// The text of a `<button>`, a `<textarea>`, a `<caption>` and of content put
/// straight into a `<table>` or a `<tr>` is shown by a browser, and is read.
#[test]
fn text_a_browser_shows_is_never_dropped() {
    assert_eq!(read("<p>a <button>b</button> c</p>"), "<p>a b c</p>");
    assert_eq!(read("<p>a <textarea>b &amp; c</textarea></p>"), "<p>a b &amp; c</p>");
    assert_eq!(
        read("<table><caption>cap</caption><tr><td>x</td></tr></table>"),
        "<p>cap</p><table><tr><td><p>x</p></td></tr></table>"
    );
    assert_eq!(
        read("<table>loose<tr>in row<td>x</td></tr></table>"),
        "<p>loosein row</p><table><tr><td><p>x</p></td></tr></table>"
    );
    assert_eq!(read("<form><p>a</p><label>b</label></form>"), "<p>a</p><p>b</p>");
    assert_eq!(read("a < b, c<3, R&D"), "<p>a &lt; b, c&lt;3, R&amp;D</p>");
}

/// What a browser hides is not read.
#[test]
fn hidden_text_is_not_read() {
    assert_eq!(
        read(
            "<p>a</p><!--[if gte mso 9]><xml><o:x>hidden</o:x></xml><![endif]--><p>b</p>\
             <style>p{}</style><script>x<y</script><title>t</title><!-- c --><p>c</p>"
        ),
        "<p>a</p><p>b</p><p>c</p>"
    );
    // Downlevel-revealed: the two halves are comments, what is between shows.
    assert_eq!(
        read("<p><![if !supportLists]><span>1.</span><![endif]>item</p>"),
        "<p>1.item</p>"
    );
    assert_eq!(read("<?xml version=\"1.0\"?><!DOCTYPE html><p>a</p>"), "<p>a</p>");
    // An unclosed <head> ends where the body starts.
    assert_eq!(read("<html><head><title>t</title><body><p>a</p>"), "<p>a</p>");
}

/// #1392: table parts with no `<table>` around them are read as a table.
#[test]
fn bare_table_parts_are_read_as_a_table() {
    assert_eq!(
        read("<td>a</td><td>b</td>"),
        "<table><tr><td><p>a</p></td><td><p>b</p></td></tr></table>"
    );
    assert_eq!(
        read("<tr><td>a</td><td>b</td></tr><tr><th>c</th><td>d</td></tr>"),
        "<table><tr><td><p>a</p></td><td><p>b</p></td></tr>\
         <tr><th><p>c</p></th><td><p>d</p></td></tr></table>"
    );
    assert_eq!(
        read("<p>x</p><tr><td>a</td></tr>"),
        "<p>x</p><table><tr><td><p>a</p></td></tr></table>"
    );
    assert_eq!(
        read("<blockquote><td>a</td></blockquote>"),
        "<blockquote><table><tr><td><p>a</p></td></tr></table></blockquote>"
    );
    assert_eq!(
        read("<thead><tr><th>h</th></tr></thead><tbody><tr><td>a</td></tr></tbody>"),
        "<table><tr><th><p>h</p></th></tr><tr><td><p>a</p></td></tr></table>"
    );
    assert_eq!(
        read("<caption>cap</caption><col><tr><td>a</td></tr>"),
        "<p>cap</p><table><tr><td><p>a</p></td></tr></table>"
    );
    // Inside an inline wrapper and inside a list.
    assert_eq!(
        read("<span><tr><td>a</td></tr></span>"),
        "<table><tr><td><p>a</p></td></tr></table>"
    );
    assert_eq!(
        read("<ul><li><td>a</td></li></ul>"),
        "<ul><li><table><tr><td><p>a</p></td></tr></table></li></ul>"
    );
}

/// #1401: a mark element around blocks marks what the same element around
/// inline content marks, so the document reads back the same.
#[test]
fn a_mark_around_blocks_leaves_hard_breaks_bare() {
    let inline = read("<p><strong>a<br>b</strong></p>");
    assert_eq!(inline, "<p><strong>a</strong><br><strong>b</strong></p>");
    assert_eq!(read("<strong><p>a<br>b</p></strong>"), inline);
    assert_eq!(read("<strong><div>a<br>b</div></strong>"), inline);
    // An image is marked either way.
    let image = read("<p><a href=\"https://e.x/\">a<img src=\"https://e.x/i.png\"></a></p>");
    assert_eq!(
        read("<a href=\"https://e.x/\"><p>a<img src=\"https://e.x/i.png\"></p></a>"),
        image
    );
}

/// Elements nested without limit do not overflow the stack: past 512 open
/// elements (Chrome's limit too) a start tag opens nothing and its content
/// goes where it stands.
#[test]
fn deep_nesting_reads_without_overflowing_the_stack() {
    for tag in ["div", "blockquote", "span", "b", "ul><li", "table><tr><td", "o:p", "x-y"] {
        let close: String = tag
            .split('>')
            .rev()
            .map(|t| format!("</{t}>"))
            .collect();
        let html = format!(
            "{}deep{}<p>after</p>",
            format!("<{tag}>").repeat(10_000),
            close.repeat(10_000)
        );
        let schema = Schema::starter_kit();
        let doc = load(&schema, &html).unwrap().expect("content");
        validity(&doc).unwrap();
        let mut text = String::new();
        doc_text(&doc, &mut text);
        assert_eq!(text.matches("deep").count(), 1, "{tag}");
        assert_eq!(text.matches("after").count(), 1, "{tag}");
        // Unclosed, too.
        let html = format!("{}deep", format!("<{tag}>").repeat(10_000));
        let doc = load(&schema, &html).unwrap().expect("content");
        validity(&doc).unwrap();
    }
}
