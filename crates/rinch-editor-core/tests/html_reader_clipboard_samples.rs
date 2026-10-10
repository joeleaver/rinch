//! What the HTML reader makes of what applications put on the clipboard
//! (#1397).
//!
//! The samples are written by hand after the markup each application is
//! known to write (its wrappers, classes, namespaced elements and comments);
//! they were not captured from the applications on this machine. Each one is
//! read, checked for validity and for reading back the same, and compared
//! with the document it should give. `all_the_text_arrives` checks, for every
//! sample, that each word a browser would show is in the document once.
use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{Node, Schema};

fn valid(node: &Node) {
    let names: Vec<&str> = node
        .content()
        .children()
        .iter()
        .map(Node::type_name)
        .collect();
    assert!(
        node.node_type().content_match().matches(&names),
        "<{}> holds {names:?}",
        node.type_name()
    );
    node.content().children().iter().for_each(valid);
}

fn read(html: &str) -> String {
    let schema = Schema::starter_kit();
    let slice = slice_from_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    valid(&doc);
    let written = node_to_html(&doc);
    let again = slice_from_html(&schema, &written).unwrap();
    assert_eq!(
        schema.branch("doc", again.content.clone()).unwrap(),
        doc,
        "not a fixed point: {written}"
    );
    written
}

/// Word, Windows (`CF_HTML`): every paragraph ends in `<o:p></o:p>`, an empty
/// paragraph is `<o:p>&nbsp;</o:p>`, a list is paragraphs whose bullet is a
/// span inside `<![if !supportLists]>`, and office settings sit in
/// conditional comments.
const WORD: &str = "<html xmlns:o=\"urn:schemas-microsoft-com:office:office\"\r\n\
xmlns:w=\"urn:schemas-microsoft-com:office:word\"\r\n\
xmlns:st1=\"urn:schemas-microsoft-com:office:smarttags\"\r\n\
xmlns=\"http://www.w3.org/TR/REC-html40\">\r\n\r\n\
<head>\r\n\
<meta http-equiv=Content-Type content=\"text/html; charset=utf-8\">\r\n\
<meta name=ProgId content=Word.Document>\r\n\
<meta name=Generator content=\"Microsoft Word 15\">\r\n\
<link rel=File-List href=\"file:///C:/Users/x/AppData/Local/Temp/msohtmlclip1/01/clip_filelist.xml\">\r\n\
<!--[if gte mso 9]><xml>\r\n <o:OfficeDocumentSettings>\r\n  <o:AllowPNG/>\r\n </o:OfficeDocumentSettings>\r\n</xml><![endif]-->\r\n\
<!--[if gte mso 9]><xml>\r\n <w:WordDocument>\r\n  <w:View>Normal</w:View>\r\n  <w:Zoom>0</w:Zoom>\r\n </w:WordDocument>\r\n</xml><![endif]-->\r\n\
<style>\r\n<!--\r\n /* Style Definitions */\r\n p.MsoNormal, li.MsoNormal, div.MsoNormal\r\n\t{mso-style-unhide:no;\r\n\tmargin:0in;\r\n\tfont-family:\"Calibri\",sans-serif;}\r\n-->\r\n</style>\r\n\
<!--[if gte mso 10]>\r\n<style>\r\n /* Style Definitions */\r\n table.MsoNormalTable\r\n\t{mso-style-name:\"Table Normal\";}\r\n</style>\r\n<![endif]-->\r\n\
</head>\r\n\r\n\
<body lang=EN-US style='tab-interval:.5in;word-wrap:break-word'>\r\n\
<!--StartFragment-->\r\n\r\n\
<h1>Quarterly report<o:p></o:p></h1>\r\n\r\n\
<p class=MsoNormal>Sales in <st1:place w:st=\"on\"><st1:City w:st=\"on\">Paris</st1:City></st1:place> \
were <b style='mso-bidi-font-weight:normal'>up</b> by <span style='color:red'>12%</span>.<o:p></o:p></p>\r\n\r\n\
<p class=MsoNormal><o:p>&nbsp;</o:p></p>\r\n\r\n\
<p class=MsoListParagraphCxSpFirst style='text-indent:-.25in;mso-list:l0 level1 lfo1'><![if !supportLists]><span \
style='font-family:Symbol;mso-fareast-font-family:Symbol;mso-bidi-font-family:Symbol'><span \
style='mso-list:Ignore'>\u{b7}<span style='font:7.0pt \"Times New Roman\"'>&nbsp;&nbsp;&nbsp;&nbsp; \
</span></span></span><![endif]>First point<o:p></o:p></p>\r\n\r\n\
<p class=MsoListParagraphCxSpLast style='text-indent:-.25in;mso-list:l0 level1 lfo1'><![if !supportLists]><span \
style='font-family:Symbol'><span style='mso-list:Ignore'>\u{b7}<span style='font:7.0pt \"Times New Roman\"'>&nbsp;&nbsp;&nbsp;&nbsp; \
</span></span></span><![endif]>Second point<o:p></o:p></p>\r\n\r\n\
<table class=MsoTableGrid border=1 cellspacing=0 cellpadding=0 style='border-collapse:collapse;border:none'>\r\n \
<tr style='mso-yfti-irow:0;mso-yfti-firstrow:yes'>\r\n  \
<td width=312 valign=top style='width:233.75pt;border:solid windowtext 1.0pt'>\r\n  \
<p class=MsoNormal>Region<o:p></o:p></p>\r\n  </td>\r\n  \
<td width=312 valign=top style='width:233.75pt'>\r\n  <p class=MsoNormal>Total<o:p></o:p></p>\r\n  </td>\r\n </tr>\r\n \
<tr style='mso-yfti-irow:1;mso-yfti-lastrow:yes'>\r\n  \
<td width=312 valign=top>\r\n  <p class=MsoNormal>North<o:p></o:p></p>\r\n  </td>\r\n  \
<td width=312 valign=top>\r\n  <p class=MsoNormal align=right style='text-align:right'>1,200<o:p></o:p></p>\r\n  </td>\r\n </tr>\r\n\
</table>\r\n\r\n\
<p class=MsoNormal>See <a href=\"https://example.com/report\">the full report</a> &#8212; it&#146;s \
long.<o:p></o:p></p>\r\n\r\n\
<!--EndFragment-->\r\n</body>\r\n\r\n</html>\r\n";

#[test]
fn word() {
    assert_eq!(
        read(WORD),
        "<h1>Quarterly report</h1>\
         <p>Sales in Paris were <strong>up</strong> by <span style=\"color:red\">12%</span>.</p>\
         <p>\u{a0}</p>\
         <p>\u{b7}\u{a0}\u{a0}\u{a0}\u{a0} First point</p>\
         <p>\u{b7}\u{a0}\u{a0}\u{a0}\u{a0} Second point</p>\
         <table><tr><td><p>Region</p></td><td><p>Total</p></td></tr>\
         <tr><td><p>North</p></td><td><p style=\"text-align:right\">1,200</p></td></tr></table>\
         <p>See <a href=\"https://example.com/report\">the full report</a> \u{2014} it\u{2019}s long.</p>"
    );
}

/// Excel: the fragment is the rows of a `<table>` whose start tag is outside
/// `StartFragment`, with a `<col>` first and `<style><!-- … --></style>` in
/// the head.
const EXCEL: &str = "<html xmlns:v=\"urn:schemas-microsoft-com:vml\"\r\n\
xmlns:o=\"urn:schemas-microsoft-com:office:office\"\r\n\
xmlns:x=\"urn:schemas-microsoft-com:office:excel\"\r\n\
xmlns=\"http://www.w3.org/TR/REC-html40\">\r\n\r\n<head>\r\n\
<meta http-equiv=Content-Type content=\"text/html; charset=utf-8\">\r\n\
<meta name=ProgId content=Excel.Sheet>\r\n\
<style>\r\n<!--table\r\n\t{mso-displayed-decimal-separator:\"\\.\";\r\n\tmso-displayed-thousand-separator:\"\\,\";}\r\n\
td\r\n\t{padding-top:1px;\r\n\tcolor:black;\r\n\tfont-size:11.0pt;}\r\n.xl65\r\n\t{font-weight:700;}\r\n-->\r\n</style>\r\n\
</head>\r\n\r\n<body link=\"#0563C1\" vlink=\"#954F72\">\r\n\r\n\
<table border=0 cellpadding=0 cellspacing=0 width=128 style='border-collapse:\r\n collapse;width:96pt'>\r\n\
<!--StartFragment-->\r\n \
<col width=64 span=2 style='width:48pt'>\r\n \
<tr height=20 style='height:15.0pt'>\r\n  \
<td height=20 class=xl65 width=64 style='height:15.0pt;width:48pt'>Name</td>\r\n  \
<td class=xl65 width=64 style='width:48pt'>Qty</td>\r\n </tr>\r\n \
<tr height=20 style='height:15.0pt'>\r\n  \
<td height=20 style='height:15.0pt'>Apples &amp; pears</td>\r\n  \
<td align=right>3</td>\r\n </tr>\r\n\
<!--EndFragment-->\r\n</table>\r\n\r\n</body>\r\n\r\n</html>\r\n";

/// The rows of the same, as a source that cuts at the fragment markers
/// hands them over: no `<table>` at all.
const EXCEL_ROWS: &str = " <col width=64 span=2 style='width:48pt'>\r\n \
<tr height=20 style='height:15.0pt'>\r\n  <td height=20 class=xl65 width=64>Name</td>\r\n  \
<td class=xl65 width=64>Qty</td>\r\n </tr>\r\n <tr height=20>\r\n  <td height=20>Apples &amp; pears</td>\r\n  \
<td align=right>3</td>\r\n </tr>\r\n";

#[test]
fn excel() {
    let table = "<table><tr><td><p>Name</p></td><td><p>Qty</p></td></tr>\
                 <tr><td><p>Apples &amp; pears</p></td><td><p>3</p></td></tr></table>";
    assert_eq!(read(EXCEL), table);
    assert_eq!(read(EXCEL_ROWS), table);
}

/// Google Sheets: a custom element around a `<style>` and the table; one
/// cell alone is a `<span>`.
const SHEETS: &str = "<meta charset='utf-8'><google-sheets-html-origin><style type=\"text/css\"><!--td \
{border: 1px solid #cccccc;}br {mso-data-placement:same-cell;}--></style><table \
xmlns=\"http://www.w3.org/1999/xhtml\" cellspacing=\"0\" cellpadding=\"0\" dir=\"ltr\" border=\"1\" \
style=\"table-layout:fixed;font-size:10pt;font-family:Arial;width:0px;border-collapse:collapse;border:none\" \
data-sheets-root=\"1\" data-sheets-baot=\"1\"><colgroup><col width=\"100\"/><col width=\"100\"/></colgroup>\
<tbody><tr style=\"height:21px;\"><td style=\"overflow:hidden;padding:2px 3px 2px 3px;vertical-align:bottom;font-weight:bold;\" \
data-sheets-value=\"{&quot;1&quot;:2,&quot;2&quot;:&quot;Name&quot;}\">Name</td><td \
style=\"overflow:hidden;padding:2px 3px 2px 3px;vertical-align:bottom;font-weight:bold;\" \
data-sheets-value=\"{&quot;1&quot;:2,&quot;2&quot;:&quot;Qty&quot;}\">Qty</td></tr><tr style=\"height:21px;\"><td \
style=\"overflow:hidden;padding:2px 3px 2px 3px;vertical-align:bottom;\" \
data-sheets-value=\"{&quot;1&quot;:2,&quot;2&quot;:&quot;Apples&quot;}\">Apples</td><td \
style=\"overflow:hidden;padding:2px 3px 2px 3px;vertical-align:bottom;text-align:right;\" \
data-sheets-value=\"{&quot;1&quot;:3,&quot;3&quot;:3}\">3</td></tr></tbody></table></google-sheets-html-origin>";

const SHEETS_CELL: &str = "<meta charset='utf-8'><google-sheets-html-origin><style type=\"text/css\"><!--td \
{border: 1px solid #cccccc;}--></style><span style=\"font-size:10pt;font-family:Arial;font-style:normal;\" \
data-sheets-root=\"1\" data-sheets-value=\"{&quot;1&quot;:2,&quot;2&quot;:&quot;Apples&quot;}\" \
data-sheets-userformat=\"{&quot;2&quot;:513,&quot;3&quot;:{&quot;1&quot;:0},&quot;12&quot;:0}\">Apples</span>\
</google-sheets-html-origin>";

#[test]
fn google_sheets() {
    assert_eq!(
        read(SHEETS),
        "<table><tr><td><p>Name</p></td><td><p>Qty</p></td></tr>\
         <tr><td><p>Apples</p></td><td><p>3</p></td></tr></table>"
    );
    assert_eq!(read(SHEETS_CELL), "<p>Apples</p>");
}

/// Google Docs: everything in a `<b style="font-weight:normal">`, text in
/// `<span style>`s (with `background-color:transparent`, which is no
/// highlight), a nested list as the sibling of the item it belongs to.
const DOCS: &str = "<meta charset='utf-8'><meta charset=\"utf-8\"><b style=\"font-weight:normal;\" \
id=\"docs-internal-guid-3b1f2f0e-7fff-1a2b-3c4d-5e6f7a8b9c0d\"><h1 dir=\"ltr\" \
style=\"line-height:1.38;margin-top:20pt;margin-bottom:6pt;\"><span \
style=\"font-size:20pt;font-family:Arial,sans-serif;font-weight:400;font-style:normal;\
text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;\">Trip plan</span></h1>\
<p dir=\"ltr\" style=\"line-height:1.38;margin-top:0pt;margin-bottom:0pt;\"><span \
style=\"font-size:11pt;font-family:Arial,sans-serif;background-color:transparent;font-weight:400;white-space:pre-wrap;\">Leave on </span><span \
style=\"font-size:11pt;font-family:Arial,sans-serif;font-weight:700;white-space:pre-wrap;\">Friday</span><span \
style=\"font-size:11pt;font-family:Arial,sans-serif;font-weight:400;white-space:pre-wrap;\">, see </span><a \
href=\"https://example.com/map\" style=\"text-decoration:none;\"><span \
style=\"font-size:11pt;font-family:Arial,sans-serif;color:#1155cc;font-weight:400;text-decoration:underline;\
-webkit-text-decoration-skip:none;white-space:pre-wrap;\">the map</span></a><span \
style=\"font-size:11pt;white-space:pre-wrap;\">.</span></p><br /><ul \
style=\"margin-top:0;margin-bottom:0;padding-inline-start:48px;\"><li dir=\"ltr\" \
style=\"list-style-type:disc;font-size:11pt;\" aria-level=\"1\"><p dir=\"ltr\" \
style=\"line-height:1.38;margin-top:0pt;margin-bottom:0pt;\" role=\"presentation\"><span \
style=\"font-size:11pt;white-space:pre-wrap;\">Tickets</span></p></li><ul \
style=\"margin-top:0;margin-bottom:0;padding-inline-start:48px;\"><li dir=\"ltr\" \
style=\"list-style-type:circle;font-size:11pt;\" aria-level=\"2\"><p dir=\"ltr\" role=\"presentation\"><span \
style=\"font-size:11pt;white-space:pre-wrap;\">Train</span></p></li></ul><li dir=\"ltr\" \
style=\"list-style-type:disc;font-size:11pt;\" aria-level=\"1\"><p dir=\"ltr\" role=\"presentation\"><span \
style=\"font-size:11pt;white-space:pre-wrap;\">Hotel</span></p></li></ul></b><br class=\"Apple-interchange-newline\">";

#[test]
fn google_docs() {
    assert_eq!(
        read(DOCS),
        "<h1>Trip plan</h1>\
         <p>Leave on Friday, see <span style=\"color:#1155cc\"><a href=\"https://example.com/map\">the map</a></span>.</p>\
         <p><br></p>\
         <ul><li><p>Tickets</p><ul><li><p>Train</p></li></ul></li><li><p>Hotel</p></li></ul>\
         <p><br></p>"
    );
}

/// Notion: plain structure, with an `<aside>` for a callout and
/// `<details>` for a toggle.
const NOTION: &str = "<meta charset='utf-8'><h2>Launch notes</h2><p>Ship <strong>v2</strong> with \
<code>--fast</code> on.</p><ul><li>Docs<ul><li>API page</li></ul></li><li>Blog post</li></ul>\
<aside>\u{1f4a1} Remember the changelog</aside><details open=\"\"><summary>More</summary><p>Hidden \
detail</p></details><ol start=\"3\"><li>Third</li></ol><blockquote>Quote</blockquote><hr>";

#[test]
fn notion() {
    assert_eq!(
        read(NOTION),
        "<h2>Launch notes</h2><p>Ship <strong>v2</strong> with <code>--fast</code> on.</p>\
         <ul><li><p>Docs</p><ul><li><p>API page</p></li></ul></li><li><p>Blog post</p></li></ul>\
         <p>\u{1f4a1} Remember the changelog</p><p>More</p><p>Hidden detail</p>\
         <ol start=\"3\"><li><p>Third</p></li></ol><blockquote><p>Quote</p></blockquote><hr>"
    );
}

/// GitHub, rendered Markdown: a heading in a `<div>` beside an anchor that
/// holds an `<svg>`, a code block in a `<div class="highlight">` with a
/// copy button, task-list checkboxes, a table with a `<thead>`.
const GITHUB: &str = "<meta charset='utf-8'><div class=\"markdown-heading\" dir=\"auto\" \
style=\"box-sizing: border-box; position: relative;\"><h2 tabindex=\"-1\" class=\"heading-element\" dir=\"auto\" \
style=\"box-sizing: border-box; margin-top: 24px;\">Install</h2><a id=\"user-content-install\" class=\"anchor\" \
aria-label=\"Permalink: Install\" href=\"https://github.com/o/r#install\" style=\"box-sizing: border-box;\"><svg \
class=\"octicon octicon-link\" viewBox=\"0 0 16 16\" version=\"1.1\" width=\"16\" height=\"16\" \
aria-hidden=\"true\"><path d=\"m7.775 3.275 1.25-1.25a3.5 3.5 0 1 1 4.95 4.95l-2.5 2.5\"></path></svg></a></div>\
<p dir=\"auto\" style=\"box-sizing: border-box; margin-top: 0px;\">Run <code style=\"box-sizing: border-box; \
font-family: ui-monospace, monospace;\">cargo add</code>, then <em>build</em>:</p><div class=\"highlight \
highlight-source-shell notranslate position-relative overflow-auto\" dir=\"auto\" style=\"box-sizing: \
border-box;\"><pre style=\"box-sizing: border-box; overflow: auto;\">cargo add rinch\n<span \
class=\"pl-c1\">cargo</span> build <span class=\"pl-k\">&amp;&amp;</span> ./run</pre><div \
class=\"zeroclipboard-container\"><clipboard-copy aria-label=\"Copy\" class=\"ClipboardButton btn btn-invisible\" \
value=\"cargo add rinch\" tabindex=\"0\" role=\"button\"><svg aria-hidden=\"true\" height=\"16\" viewBox=\"0 0 16 \
16\" class=\"octicon octicon-copy\"><path d=\"M0 6.75C0 5.784.784 5 1.75 5h1.5\"></path></svg></clipboard-copy>\
</div></div><ul class=\"contains-task-list\" style=\"box-sizing: border-box;\"><li class=\"task-list-item\"><input \
type=\"checkbox\" id=\"\" disabled=\"\" class=\"task-list-item-checkbox\" checked=\"\"><span>\u{a0}</span>done</li>\
<li class=\"task-list-item\"><input type=\"checkbox\" id=\"\" disabled=\"\" \
class=\"task-list-item-checkbox\"><span>\u{a0}</span>to do</li></ul><markdown-accessiblity-table><table \
style=\"box-sizing: border-box;\"><thead><tr><th>Flag</th><th align=\"right\">Default</th></tr></thead><tbody><tr>\
<td><code>--fast</code></td><td align=\"right\">off</td></tr></tbody></table></markdown-accessiblity-table>";

#[test]
fn github() {
    assert_eq!(
        read(GITHUB),
        "<h2>Install</h2>\
         <p>Run <code>cargo add</code>, then <em>build</em>:</p>\
         <pre>cargo add rinch\ncargo build &amp;&amp; ./run</pre>\
         <ul><li><p>\u{a0}done</p></li><li><p>\u{a0}to do</p></li></ul>\
         <table><tr><th><p>Flag</p></th><th><p>Default</p></th></tr>\
         <tr><td><p><code>--fast</code></p></td><td><p>off</p></td></tr></table>"
    );
}

/// VS Code: a `<div style="…white-space: pre;">` holding one `<div>` per
/// line, one `<span>` per token, and a `<br>` for an empty line.
const VS_CODE: &str = "<meta charset='utf-8'><div style=\"color: #cccccc;background-color: #1f1f1f;font-family: \
Consolas, 'Courier New', monospace;font-weight: normal;font-size: 14px;line-height: 19px;white-space: pre;\"><div>\
<span style=\"color: #569cd6;\">fn</span><span style=\"color: #cccccc;\"> </span><span style=\"color: \
#dcdcaa;\">main</span><span style=\"color: #cccccc;\">() {</span></div><div><span style=\"color: #cccccc;\">    \
</span><span style=\"color: #569cd6;\">let</span><span style=\"color: #cccccc;\"> </span><span style=\"color: \
#9cdcfe;\">x</span><span style=\"color: #cccccc;\"> </span><span style=\"color: #d4d4d4;\">=</span><span \
style=\"color: #cccccc;\"> </span><span style=\"color: #b5cea8;\">1</span><span style=\"color: #cccccc;\"> &lt; \
</span><span style=\"color: #b5cea8;\">2</span><span style=\"color: #cccccc;\">;</span></div><br><div><span \
style=\"color: #cccccc;\">}</span></div></div>";

/// One line copied from VS Code: the same wrapper around one `<div>`.
const VS_CODE_LINE: &str = "<meta charset='utf-8'><div style=\"color: #cccccc;background-color: #1f1f1f;\
font-family: Consolas, monospace;font-size: 14px;white-space: pre;\"><div><span style=\"color: \
#9cdcfe;\">count</span><span style=\"color: #cccccc;\"> </span><span style=\"color: #d4d4d4;\">+=</span><span \
style=\"color: #cccccc;\"> </span><span style=\"color: #b5cea8;\">1</span></div></div>";

#[test]
fn vs_code() {
    // A code block: the lines and their indentation are what was copied.
    // Paragraphs would keep the lines but show the indentation only as
    // text, and the token colours are a theme's, not the document's.
    assert_eq!(
        read(VS_CODE),
        "<pre>fn main() {\n    let x = 1 &lt; 2;\n\n}</pre>"
    );
    assert_eq!(read(VS_CODE_LINE), "<pre>count += 1</pre>");
}

/// Apple Notes and Mail (WebKit): `<div>` per line, `<div><br></div>` for an
/// empty one, `Apple-converted-space`, a quoted reply.
const APPLE_NOTES: &str = "<html><head><meta http-equiv=\"content-type\" content=\"text/html; charset=utf-8\">\
<style type=\"text/css\">\np.p1 {margin: 0.0px 0.0px 0.0px 0.0px; font: 13.0px 'Helvetica Neue'}\n\
li.li1 {margin: 0.0px 0.0px 0.0px 0.0px}\nspan.s1 {font: 9.0px Menlo}\n</style></head><body>\n\
<p class=\"p1\"><b>Shopping</b></p>\n<p class=\"p2\"><br></p>\n<ul class=\"ul1\">\n\
<li class=\"li1\"><span class=\"s1\"></span>Milk</li>\n<li class=\"li1\">Bread<span \
class=\"Apple-converted-space\">\u{a0}</span></li>\n</ul>\n</body></html>";

const APPLE_MAIL: &str = "<meta charset='utf-8'><div dir=\"auto\" style=\"overflow-wrap: break-word;\">Hi \
Sam,<div><br></div><div>See you <i>there</i>.<span class=\"Apple-converted-space\">\u{a0}</span></div><div><br>\
<blockquote type=\"cite\"><div>On 3 May, Sam wrote:</div><br class=\"Apple-interchange-newline\"><div><div \
dir=\"ltr\">Are you coming?</div></div></blockquote></div></div>";

#[test]
fn apple_notes_and_mail() {
    assert_eq!(
        read(APPLE_NOTES),
        "<p><strong>Shopping</strong></p><p><br></p><ul><li><p>Milk</p></li><li><p>Bread\u{a0}</p></li></ul>"
    );
    assert_eq!(
        read(APPLE_MAIL),
        "<p>Hi Sam,</p><p><br></p><p>See you <em>there</em>.\u{a0}</p><p><br></p>\
         <blockquote><p>On 3 May, Sam wrote:</p><p><br></p><p>Are you coming?</p></blockquote>"
    );
}

/// Slack: `<div>` sections with `data-stringify-*` attributes, an emoji as
/// an `<img>`, a mention in a `<span>`, `<br aria-hidden>` between lines.
const SLACK: &str = "<meta charset='utf-8'><div class=\"p-rich_text_section\">Hello <b \
data-stringify-type=\"bold\">team</b> <img data-stringify-type=\"emoji\" alt=\":wave:\" \
src=\"https://a.slack-edge.com/production-standard-emoji-assets/14.0/google-small/1f44b.png\" \
class=\"c-emoji c-emoji__small\"> see <a target=\"_blank\" class=\"c-link\" data-stringify-link=\"https://example.com\" \
href=\"https://example.com\" rel=\"noopener noreferrer\">the doc</a><span aria-label=\"\">\u{a0}</span><span \
data-stringify-type=\"mention\" class=\"c-member_slug\">@sam</span><br aria-hidden=\"true\">second line with <code \
data-stringify-type=\"code\" class=\"c-mrkdwn__code\">x = 1</code></div><ul data-stringify-type=\"unordered-list\" \
class=\"p-rich_text_list p-rich_text_list__bullet\" data-indent=\"0\" data-border=\"0\"><li \
data-stringify-indent=\"0\" data-stringify-border=\"0\">one</li><li data-stringify-indent=\"0\">two</li></ul><pre \
data-stringify-type=\"pre\" class=\"c-mrkdwn__pre\">let a = 1;<br>let b = 2;</pre><blockquote type=\"cite\" \
class=\"c-mrkdwn__quote\" data-stringify-type=\"quote\">quoted <i data-stringify-type=\"italic\">words</i>\
</blockquote>";

#[test]
fn slack() {
    assert_eq!(
        read(SLACK),
        // The image keeps Slack's own data attribute: an image keeps every
        // app data attribute a paste carries (`NodeSpec::data_attrs`).
        "<p>Hello <strong>team</strong> <img alt=\":wave:\" data-stringify-type=\"emoji\" \
         src=\"https://a.slack-edge.com/production-standard-emoji-assets/14.0/google-small/1f44b.png\"> see \
         <a href=\"https://example.com\" target=\"_blank\" rel=\"noopener noreferrer\">the doc</a>\u{a0}@sam<br>second line with \
         <code>x = 1</code></p>\
         <ul><li><p>one</p></li><li><p>two</p></li></ul>\
         <pre>let a = 1;\nlet b = 2;</pre>\
         <blockquote><p>quoted <em>words</em></p></blockquote>"
    );
}

/// Gmail: `<div>` per line, a quoted thread in `<blockquote class="gmail_quote">`,
/// and layout tables nested in each other in the mail it quotes.
const GMAIL: &str = "<meta charset='utf-8'><div dir=\"ltr\"><div>Thanks!</div><div><br></div><div \
class=\"gmail_quote\"><div dir=\"ltr\" class=\"gmail_attr\">On Mon, 3 May 2027 at 10:00, Ada &lt;<a \
href=\"mailto:ada@example.com\">ada@example.com</a>&gt; wrote:<br></div><blockquote class=\"gmail_quote\" \
style=\"margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);padding-left:1ex\"><table \
role=\"presentation\" width=\"100%\"><tbody><tr><td><table><tbody><tr><td><h1 style=\"margin:0\">Your \
order</h1></td></tr><tr><td>Shipped <b>today</b>.</td></tr></tbody></table></td></tr></tbody></table>\
</blockquote></div></div>";

#[test]
fn gmail() {
    assert_eq!(
        read(GMAIL),
        "<p>Thanks!</p><p><br></p>\
         <p>On Mon, 3 May 2027 at 10:00, Ada &lt;<a href=\"mailto:ada@example.com\">ada@example.com</a>&gt; wrote:<br></p>\
         <blockquote><table><tr><td><table><tr><td><h1>Your order</h1></td></tr>\
         <tr><td><p>Shipped <strong>today</strong>.</p></td></tr></table></td></tr></table></blockquote>"
    );
}

/// Every word a browser shows for a sample is in the document once, and no
/// word a browser hides is.
#[test]
fn all_the_text_arrives() {
    let schema = Schema::starter_kit();
    let samples: [(&str, &[&str], &[&str]); 10] = [
        (
            WORD,
            &[
                "Quarterly",
                "Paris",
                "up",
                "12%",
                "First point",
                "Second point",
                "Region",
                "Total",
                "North",
                "1,200",
                "the full report",
                "long.",
            ],
            &[
                "Normal",
                "MsoNormal",
                "Calibri",
                "AllowPNG",
                "Table Normal",
                "Style Definitions",
            ],
        ),
        (
            EXCEL,
            &["Name", "Qty", "Apples & pears", "3"],
            &["mso-displayed", "xl65", "padding"],
        ),
        (EXCEL_ROWS, &["Name", "Qty", "Apples & pears", "3"], &[]),
        (
            SHEETS,
            &["Name", "Qty", "Apples", "3"],
            &["border", "mso-data"],
        ),
        (
            DOCS,
            &[
                "Trip plan",
                "Leave on",
                "Friday",
                "the map",
                "Tickets",
                "Train",
                "Hotel",
            ],
            &["docs-internal"],
        ),
        (
            NOTION,
            &[
                "Launch notes",
                "v2",
                "--fast",
                "Docs",
                "API page",
                "Blog post",
                "changelog",
                "More",
                "Hidden detail",
                "Third",
                "Quote",
            ],
            &[],
        ),
        (
            GITHUB,
            &[
                "Install",
                "cargo add",
                "build",
                "cargo add rinch",
                "./run",
                "done",
                "to do",
                "Flag",
                "Default",
                "--fast",
                "off",
            ],
            &["Permalink", "m7.775"],
        ),
        (
            VS_CODE,
            &["fn", "main", "let", "x", "1", "2", "}"],
            &["Consolas"],
        ),
        (
            APPLE_MAIL,
            &["Hi Sam", "See you", "there", "On 3 May", "Are you coming?"],
            &[],
        ),
        (
            SLACK,
            &[
                "Hello",
                "team",
                "the doc",
                "@sam",
                "second line",
                "x = 1",
                "one",
                "two",
                "let a",
                "let b",
                "quoted",
                "words",
            ],
            &[],
        ),
    ];
    for (html, shown, hidden) in samples {
        let slice = slice_from_html(&schema, html).unwrap();
        let doc = schema.branch("doc", slice.content.clone()).unwrap();
        let mut text = String::new();
        fn gather(node: &Node, out: &mut String) {
            if let Some(t) = node.text() {
                out.push_str(t);
            }
            node.content()
                .children()
                .iter()
                .for_each(|c| gather(c, out));
            if node.is_block() {
                out.push('\n');
            }
        }
        gather(&doc, &mut text);
        for word in shown {
            assert!(text.contains(word), "{word:?} is missing from {text:?}");
        }
        for word in hidden {
            assert!(!text.contains(word), "{word:?} is shown in {text:?}");
        }
    }
}
