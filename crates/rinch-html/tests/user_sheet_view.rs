//! `HtmlView`'s user sheet on the desktop DOM: matched elements get the
//! rules' declarations, `display: none` hides them in layout, and changing
//! only the declarations restyles them without cleaning again.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rinch_core::Component;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::Signal;
use rinch_dom::RinchDocument;
use rinch_html::{HtmlView, Policy};

const MAIL: &str = r#"<html><body>
<p class="reply">Sounds good.</p>
<div class="gmail_quote" style="color: blue">
  <blockquote type="cite">Shall we meet at noon?</blockquote>
</div>
<img src="data:image/png;base64,AAAA" width="1" height="1">
</body></html>"#;

struct Mounted {
    doc: Rc<RefCell<dyn DomDocument>>,
    body: NodeHandle,
    _scope: RenderScope,
    cleans: Arc<AtomicUsize>,
    reported: Rc<RefCell<Vec<Vec<usize>>>>,
}

fn mount(sheet: Signal<String>) -> Mounted {
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(RinchDocument::new()));
    let root = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), root);
    let body = NodeHandle::new(root, Rc::downgrade(&doc));
    // The picture rule runs once per clean (the mail has one picture).
    let cleans = Arc::new(AtomicUsize::new(0));
    let counted = cleans.clone();
    let policy = Policy {
        picture: Arc::new(move |src| {
            counted.fetch_add(1, Ordering::Relaxed);
            Some(src.to_owned())
        }),
        ..Policy::default()
    };
    let reported = Rc::new(RefCell::new(Vec::new()));
    let r = reported.clone();
    let view = HtmlView {
        document: MAIL.into(),
        policy: Some(policy),
        user_css_fn: Some(Rc::new(move || sheet.get())),
        onusermatches: Some((move |n: Vec<usize>| r.borrow_mut().push(n)).into()),
        ..Default::default()
    }
    .render(&mut scope, &[]);
    body.append_child(&view);
    Mounted {
        doc,
        body,
        _scope: scope,
        cleans,
        reported,
    }
}

impl Mounted {
    /// The laid-out height of the element whose own text starts with `text`.
    fn height(&self, text: &str) -> f32 {
        self.doc.borrow_mut().resolve_layout(800.0, 600.0);
        let mut stack = vec![self.body.clone()];
        while let Some(n) = stack.pop() {
            let id = n.node_id();
            let children = self.doc.borrow().get_children(id);
            let own = children.iter().find_map(|&c| {
                let doc = self.doc.borrow();
                doc.get_children(c)
                    .is_empty()
                    .then(|| doc.text_content(c))
                    .flatten()
            });
            if own.is_some_and(|t| t.trim().starts_with(text)) {
                return n.bounding_rect().map_or(0.0, |r| r.height);
            }
            stack.extend(
                children
                    .into_iter()
                    .map(|c| NodeHandle::new(c, Rc::downgrade(&self.doc))),
            );
        }
        panic!("no element with text {text:?}");
    }

    fn style_of(&self, text: &str) -> Option<String> {
        self.body
            .query_selector_all("[data-rinch-user]")
            .into_iter()
            .find(|n| {
                n.text_content()
                    .unwrap_or_default()
                    .trim()
                    .starts_with(text)
            })
            .and_then(|n| n.get_attribute("style"))
    }
}

#[test]
fn display_none_hides_the_matched_element_and_toggling_does_not_clean_again() {
    let sheet = Signal::new(".gmail_quote { display: none }".to_owned());
    let m = mount(sheet);
    assert_eq!(m.cleans.load(Ordering::Relaxed), 1);
    assert_eq!(*m.reported.borrow(), [vec![1]]);
    assert!(m.height("Sounds good.") > 0.0);
    assert_eq!(m.height("Shall we meet"), 0.0, "the quote is hidden");
    assert_eq!(
        m.style_of("Shall").as_deref(),
        Some("color: blue; display: none !important"),
        "after the document's own style"
    );

    // Show it: the same selector, no declarations. No new clean.
    sheet.set(".gmail_quote { }".into());
    assert!(m.height("Shall we meet") > 0.0, "the quote shows");
    assert_eq!(m.style_of("Shall").as_deref(), Some("color: blue"));
    // Hide again, then clear the sheet altogether: still no new clean.
    sheet.set(".gmail_quote { display: none }".into());
    assert_eq!(m.height("Shall we meet"), 0.0);
    sheet.set(String::new());
    assert!(m.height("Shall we meet") > 0.0);
    assert_eq!(
        m.cleans.load(Ordering::Relaxed),
        1,
        "toggling never cleaned again"
    );
    assert_eq!(*m.reported.borrow(), [vec![1], vec![]]);

    // A selector the last clean did not match: a new clean.
    sheet.set("blockquote[type=cite] { display: none } p { color: red }".into());
    assert_eq!(m.cleans.load(Ordering::Relaxed), 2);
    assert_eq!(m.height("Shall we meet"), 0.0);
    assert!(m.height("Sounds good.") > 0.0);
    assert_eq!(m.reported.borrow().last(), Some(&vec![1, 1]));
}

#[test]
fn a_sheet_that_starts_empty_still_cleans_first_then_restyles() {
    let sheet = Signal::new(String::new());
    let m = mount(sheet);
    assert_eq!(m.cleans.load(Ordering::Relaxed), 1);
    assert!(m.height("Sounds good.") > 0.0, "the document is shown");
    assert!(m.height("Shall we meet") > 0.0);
    assert_eq!(*m.reported.borrow(), [Vec::<usize>::new()]);
    // Its first selector is one no clean matched: it cleans again.
    sheet.set(".gmail_quote { display: none }".into());
    assert_eq!(m.cleans.load(Ordering::Relaxed), 2);
    assert_eq!(m.height("Shall we meet"), 0.0);
    assert_eq!(m.reported.borrow().last(), Some(&vec![1]));
}
