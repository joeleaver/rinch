//! `HtmlView`: a document someone else wrote, in the app's DOM.
//!
//! Give it the document and a [`Policy`](crate::Policy), and it cleans the
//! document at its own width, and again whenever a resize crosses one of the
//! document's `@media` breakpoints, so a narrow view gets the document's
//! narrow layout (what a phone shows) and a wide one its wide layout:
//!
//! ```ignore
//! rsx! {
//!     HtmlView {
//!         document: source,
//!         policy: Policy::default(),
//!         onlink: move |href: String| open_if_web(&href),
//!     }
//! }
//! ```
//!
//! An app's own CSS for the document, a user sheet, goes in the policy's
//! [`user_css`](crate::Policy::user_css) or, to change it without cleaning
//! again, `user_css_fn`; `onusermatches` says how many elements each rule
//! matched.
//!
//! Or give it HTML already cleaned ([`clean`](crate::clean)): `html` and
//! `body_style`, drawn as they are at any width.
//!
//! It renders `div.rinch-html-view` holding `div.rinch-html-body` (the
//! document's body: its style on it, its content inside). A click on a link
//! in the document calls `onlink` with the link's `href` (as written, after
//! the cleaner's scheme check); the view never navigates or opens anything
//! itself, and on the web the browser does not follow it either: a middle
//! click comes to `onlink` too (`rinch_core::click_context().button` says
//! which button), and the browser's link menu and link dragging are off
//! unless `link_menu` allows them. Links show
//! a pointer cursor and, by default, their address as a tooltip, with a
//! warning when it hides or disguises where it goes ([`Tooltips`]).
//!
//! To show another document, render a new view (key it on the document, in
//! a `for` loop).

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, InputCallback, ValueCallback};

use crate::{Cleaned, Policy, Tooltips};

/// Shows a document someone else wrote; see the module docs.
pub struct HtmlView {
    /// The document as written, cleaned by the view at its own width under
    /// `policy`. Takes the place of `html` and `body_style`.
    pub document: String,
    /// What the view's cleaning lets through (pictures, link schemes); the
    /// default policy when `None`.
    pub policy: Option<Policy>,
    /// HTML already cleaned ([`Cleaned::html`]), when `document` is empty.
    /// Put only cleaned HTML here: it goes into the DOM as it is.
    pub html: String,
    /// Its body's style ([`Cleaned::body_style`]).
    pub body_style: String,
    /// Called with a clicked link's `href`.
    pub onlink: Option<InputCallback>,
    /// The app's user style sheet for `document`, reactive: in place of the
    /// policy's [`user_css`](crate::Policy::user_css). Its selectors are
    /// matched against the document as written, and each matched element
    /// gets its rules' declarations, `!important`, after its own style. A
    /// change that only alters declarations, drops rules or empties the
    /// sheet restyles the matched elements with no new clean; one that names
    /// a selector the last clean did not match cleans the document again.
    pub user_css_fn: Option<Rc<dyn Fn() -> String>>,
    /// Called with how many elements each rule of the user sheet matched,
    /// in the sheet's order, once the view has cleaned the document and
    /// again whenever that changes (a new sheet). Not called for `html`.
    pub onusermatches: Option<ValueCallback<Vec<usize>>>,
    /// How links are titled: their address and a warning when it hides or
    /// disguises where it goes ([`Tooltips::Default`], in English), the
    /// app's own words, or nothing.
    pub tooltips: Tooltips,
    /// On the web, let the browser's own link menu ("Open link in new tab")
    /// and dragging a link to the tab bar work on the document's links:
    /// both open the address without `onlink`, so they are off by default.
    pub link_menu: bool,
    /// Whether the document's text can be selected and copied, as in a
    /// browser (the default). `false` makes it unselectable, as an app's own
    /// text is. It sets the view's `user-select` by a rule of zero
    /// specificity, so an app's own `user-select` on the view (a style sheet
    /// rule for a `class:` it gives the view, or a `style:`) overrides it.
    pub selectable: bool,
    /// Let the app's style sheets (its theme's base styles, its components'
    /// and its own CSS) match the document's elements, for an app showing
    /// HTML of its own that it wants themed. Off by default: the document
    /// is then laid out as a browser lays out the same HTML put in a
    /// `<div>` of a page with no style sheet, by the browser's own sheet,
    /// its presentational attributes and its own styles, so a theme's
    /// `* { box-sizing: border-box }` or heading sizes cannot change it
    /// (#2357). Inherited properties (`font-*`, `line-height`, …) reach it
    /// from the view either way.
    pub app_styles: bool,
    /// Draw the document black on white, as a browser draws a page with no
    /// colours of its own, whatever the app's theme. Off by default: the
    /// document then takes its `color` from the view's host and shows the
    /// host's background, as the same HTML in a `<div>` does (#2357), so a
    /// dark-themed app showing mail colours the element around the view
    /// (Postrinch's `.mail-html`: `background: #fff; color: #222`) or sets
    /// this. Set by a rule of zero specificity: an app's own `color` or
    /// `background-color` on the view wins.
    pub paper: bool,
}

impl Default for HtmlView {
    fn default() -> Self {
        Self {
            document: String::new(),
            policy: None,
            html: String::new(),
            body_style: String::new(),
            onlink: None,
            user_css_fn: None,
            onusermatches: None,
            tooltips: Tooltips::default(),
            link_menu: false,
            selectable: true,
            app_styles: false,
            paper: false,
        }
    }
}

impl std::fmt::Debug for HtmlView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HtmlView")
            .field("document", &format_args!("<{} bytes>", self.document.len()))
            .field("policy", &self.policy)
            .field("html", &format_args!("<{} bytes>", self.html.len()))
            .field("body_style", &self.body_style)
            .field("onlink", &self.onlink.as_ref().map(|_| "<callback>"))
            .field("user_css_fn", &self.user_css_fn.as_ref().map(|_| "<fn>"))
            .field(
                "onusermatches",
                &self.onusermatches.as_ref().map(|_| "<callback>"),
            )
            .field("tooltips", &self.tooltips)
            .field("link_menu", &self.link_menu)
            .field("selectable", &self.selectable)
            .field("app_styles", &self.app_styles)
            .field("paper", &self.paper)
            .finish()
    }
}

/// The view's own CSS, scoped to the view.
///
/// - Nothing the document draws reaches outside the view, to paint over the
///   app or take its clicks: the view clips on both axes and is the
///   stacking context and containing block of everything in it. A document
///   wider than the view scrolls sideways, as a browser scrolls a page wider
///   than its window. (The cleaner already leaves out `position: fixed` and
///   `absolute` and `z-index`; this holds for whatever moves content
///   otherwise: relative offsets, transforms, negative margins.)
/// - It sets no colours of its own (#2357): a document that sets none takes
///   the host's `color` and shows the host's background, as the same HTML
///   in a `<div>` does in a browser. Under `paper: true` it is black on
///   white, whatever the app's theme, by a zero-specificity rule an app's
///   own rule for the view overrides.
/// - Links show a pointer. (In an isolated view, the default, no rule of
///   the app's reaches the document: the user-agent sheet gives its links
///   the browser's style, a pointer among it.)
/// - Its text is selectable (`user-select: text`), or not under
///   `selectable: false`. Both rules are inside `:where()`, specificity zero,
///   so any rule of the app's that names the view (a `class:` on it, an
///   ancestor) overrides them per instance, wherever its sheet comes in the
///   document (#2231).
///
/// An app may restyle `.rinch-html-view` through the cascade (its own
/// colours, its font, which the document inherits); one that lets content
/// overflow takes that on.
const CSS: &str = "\
.rinch-html-view { position: relative; z-index: 0; overflow-x: auto; overflow-y: hidden; }
:where(.rinch-html-view.rinch-html-paper) { background-color: #ffffff; color: #000000; }
:where(.rinch-html-view) { user-select: text; }
:where(.rinch-html-view.rinch-html-unselectable) { user-select: none; }
.rinch-html-view a[href] { cursor: pointer; }";

/// The width a document is first cleaned at, before the view has one: wide,
/// as a desktop shows mail.
const FIRST_WIDTH: f32 = 10_000.0;

impl Component for HtmlView {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let view = scope.create_element("div");
        // A document's text can be selected and copied, as in a browser; an
        // app's own controls cannot, by default (`selectable`). Set by class
        // rules of zero specificity (`CSS`), so the app's own `user-select`
        // on the view wins; the document's elements take it from here
        // unless they say otherwise.
        let mut class = String::from("rinch-html-view");
        if !self.selectable {
            class.push_str(" rinch-html-unselectable");
        }
        // Black on white whatever the theme, by request (`paper`).
        if self.paper {
            class.push_str(" rinch-html-paper");
        }
        view.set_attribute("class", &class);
        // Its links are the app's to follow (`onlink`): on the web, the
        // browser never navigates the app's own tab to one.
        view.set_attribute("data-claim-links", if self.link_menu { "menu" } else { "" });
        let style = scope.create_element("style");
        style.append_child(&scope.create_text(CSS));
        view.append_child(&style);
        // The app's sheets stop here (#2357): below the view only the UA
        // sheet, the document's attributes and its own styles apply, as in
        // a browser. After the `<style>`, which styles the view itself (on
        // the web, a shadow root takes the children added from here on).
        if !self.app_styles {
            view.isolate_styles();
        }

        let body = scope.create_element("div");
        body.set_attribute("class", "rinch-html-body");
        view.append_child(&body);

        if self.document.is_empty() {
            show(
                &body,
                &Cleaned {
                    html: self.html.clone(),
                    body_style: self.body_style.clone(),
                    ..Cleaned::default()
                },
                &self.tooltips,
            );
        } else {
            let queries = crate::media::queries(&self.document);
            let policy = self.policy.clone().unwrap_or_default();
            let view_state = Rc::new(View {
                body: body.clone(),
                document: self.document.clone(),
                tooltips: self.tooltips.clone(),
                onusermatches: self.onusermatches.clone(),
                state: RefCell::new(State {
                    width: FIRST_WIDTH,
                    signature: queries.at(FIRST_WIDTH),
                    sheet: policy.user_css.clone(),
                    cleaned: false,
                    selectors: Vec::new(),
                    counts: Vec::new(),
                    marked: Vec::new(),
                    reported: None,
                }),
                policy,
            });
            match self.user_css_fn.clone() {
                // The sheet as the app says now: a change that names no
                // selector the last clean did not match only restyles the
                // marked elements; one that does cleans again.
                Some(sheet) => {
                    let v = view_state.clone();
                    scope.create_effect(move || {
                        let css = sheet();
                        v.sheet_changed(css);
                    });
                }
                None => v_reclean(&view_state),
            }
            // Clean again when the view's width crosses a breakpoint. The
            // queries are read once; a resize only evaluates them, and only
            // when the width changed.
            let bounds = view.bounds_signal();
            let v = view_state;
            scope.create_effect(move || {
                let width = bounds.get().width;
                if width <= 0.0 || v.state.borrow().width == width {
                    return;
                }
                let now = queries.at(width);
                {
                    let mut st = v.state.borrow_mut();
                    st.width = width;
                    if st.signature == now {
                        return;
                    }
                    st.signature = now;
                }
                v_reclean(&v);
            });
        }

        if let Some(onlink) = self.onlink.clone() {
            // One handler for the whole document: a click anywhere in it
            // lands here, and `click_link` says which link it went through.
            let handler = scope.register_handler(move || {
                if let Some(href) = rinch_core::click_link() {
                    onlink.invoke(href);
                }
            });
            view.set_attribute("data-rid", &handler.0.to_string());
        }
        view
    }
}

fn show(body: &NodeHandle, cleaned: &Cleaned, tooltips: &Tooltips) {
    if cleaned.body_style.is_empty() {
        body.remove_attribute("style");
    } else {
        body.set_attribute("style", &cleaned.body_style);
    }
    body.set_inner_html(&cleaned.html);
    // Each link says where it really goes, as its tooltip.
    for link in body.query_selector_all("a[href]") {
        let Some(href) = link.get_attribute("href") else {
            continue;
        };
        let text = link.text_content().unwrap_or_default();
        if let Some(title) = tooltips.title(&href, &text) {
            link.set_attribute("title", &title);
        }
    }
}

/// A view of a document it cleans itself.
struct View {
    body: NodeHandle,
    document: String,
    policy: Policy,
    tooltips: Tooltips,
    onusermatches: Option<ValueCallback<Vec<usize>>>,
    state: RefCell<State>,
}

struct State {
    /// The width it was last cleaned or laid out at, and which `@media`
    /// blocks match there.
    width: f32,
    signature: Vec<bool>,
    /// The user sheet as last given.
    sheet: String,
    /// Whether it has been cleaned yet.
    cleaned: bool,
    /// The last clean's user-sheet selectors and how many elements each
    /// matched ([`Cleaned::user_selectors`], [`Cleaned::user_matches`]).
    selectors: Vec<String>,
    counts: Vec<usize>,
    /// The elements that clean marked.
    marked: Vec<Marked>,
    /// The counts last reported to `onusermatches`.
    reported: Option<Vec<usize>>,
}

/// An element the user sheet matched.
struct Marked {
    node: NodeHandle,
    /// Its `style` as cleaned, before any user declarations.
    base: String,
    /// The indices (into [`State::selectors`]) of the rules it matched.
    rules: Vec<usize>,
    /// Its `style` now.
    current: String,
}

impl View {
    fn sheet_changed(&self, css: String) {
        let restyle_only = {
            let mut st = self.state.borrow_mut();
            st.sheet = css;
            st.cleaned
                && crate::user_sheet::rules(&st.sheet)
                    .iter()
                    .all(|r| st.selectors.contains(&r.selector))
        };
        if restyle_only {
            self.restyle();
        } else {
            v_reclean(self);
        }
    }

    /// The marked elements' styles for the sheet as last given (every rule's
    /// selector one the last clean matched), and the counts reported.
    fn restyle(&self) {
        let report = {
            let mut st = self.state.borrow_mut();
            let rules = crate::user_sheet::rules(&st.sheet);
            let at: Vec<Option<usize>> = rules
                .iter()
                .map(|r| st.selectors.iter().position(|s| *s == r.selector))
                .collect();
            for m in &mut st.marked {
                let mut style = m.base.trim().trim_end_matches(';').trim().to_owned();
                for (rule, i) in rules.iter().zip(&at) {
                    if rule.declarations.is_empty() || !i.is_some_and(|i| m.rules.contains(&i)) {
                        continue;
                    }
                    if !style.is_empty() {
                        style.push_str("; ");
                    }
                    style.push_str(&rule.declarations);
                }
                if style != m.current {
                    if style.is_empty() {
                        m.node.remove_attribute("style");
                    } else {
                        m.node.set_attribute("style", &style);
                    }
                    m.current = style;
                }
            }
            let counts: Vec<usize> = at.iter().map(|i| i.map_or(0, |i| st.counts[i])).collect();
            (st.reported.as_ref() != Some(&counts)).then(|| {
                st.reported = Some(counts.clone());
                counts
            })
        };
        if let (Some(counts), Some(callback)) = (report, &self.onusermatches) {
            callback.invoke(counts);
        }
    }
}

/// Clean the document again at the view's width with the sheet as last
/// given, show it, and give the marked elements their declarations.
fn v_reclean(v: &View) {
    let (width, sheet) = {
        let st = v.state.borrow();
        (st.width, st.sheet.clone())
    };
    let policy = Policy {
        user_css: sheet,
        ..v.policy.clone()
    };
    let cleaned = crate::clean_at(&v.document, &policy, width);
    show(&v.body, &cleaned, &v.tooltips);
    let marked = if cleaned.user_selectors.is_empty() {
        Vec::new()
    } else {
        v.body
            .query_selector_all(&format!("[{}]", crate::user_sheet::MARK))
            .into_iter()
            .map(|node| {
                let rules = node
                    .get_attribute(crate::user_sheet::MARK)
                    .unwrap_or_default()
                    .split_ascii_whitespace()
                    .filter_map(|i| i.parse().ok())
                    .collect();
                let base = node.get_attribute("style").unwrap_or_default();
                Marked {
                    node,
                    current: base.clone(),
                    base,
                    rules,
                }
            })
            .collect()
    };
    {
        let mut st = v.state.borrow_mut();
        st.cleaned = true;
        st.selectors = cleaned.user_selectors;
        st.counts = cleaned.user_matches;
        st.marked = marked;
    }
    v.restyle();
}
