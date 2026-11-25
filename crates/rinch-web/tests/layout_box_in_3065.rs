//! `NodeHandle::layout_box_in` in the browser (#3065), the twin of
//! `rinch-dom/tests/layout_box_in_tests.rs`: a node's border box as laid out,
//! before any transform, in a positioned container's padding-box frame, by
//! the `offsetParent` chain.
//!
//! The container is bordered (wider on the left) and padded, and offset in
//! the page, so an answer from its border box, its content box or the
//! viewport differs from the right one; the nested box is positioned and
//! translated, and the item is scaled, so a painted answer differs too. The
//! web answers whole pixels (`offsetLeft`); `lb-frac` pins how it rounds.
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::ElementBounds;
use rinch_web::RootHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/engine.rs"]
mod engine;

const HOST_MARKER: &str = "data-layout-box-in-3065-host";

const CSS: &str = ":where([data-layout-box-in-3065-host]) div { margin: 0; padding: 0; border: 0 } \
    .lb-c { position: relative; left: 7px; top: 3px; width: 300px; height: 200px; \
            border: 5px solid black; border-left-width: 9px; padding: 4px 6px } \
    .lb-direct { margin-left: 13px; width: 40px; height: 10px } \
    .lb-p { position: relative; box-sizing: content-box; margin-left: 11px; width: 100px; height: 60px; \
            border: 2px solid black; padding: 3px; transform: translateX(40px) } \
    .lb-n { margin-top: 8px; width: 50px; height: 20px; transform: scale(2) } \
    .lb-fixed { position: fixed; left: 5px; top: 5px; width: 10px; height: 10px } \
    .lb-static { width: 100px; height: 40px; border: 3px solid black; padding: 2px } \
    .lb-in { width: 10px; height: 10px; margin-left: 4px } \
    .lb-t { width: 100px; height: 40px; border: 3px solid black; transform: translateX(1px) } \
    .lb-tin { width: 10px; height: 10px; margin-left: 4px } \
    .lb-frac { position: relative; margin-left: 2.25px; width: 20.5px; height: 10px }";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    style: web_sys::Element,
    handles: Vec<(&'static str, NodeHandle)>,
}

impl Fixture {
    fn mount() -> Self {
        let style = document().create_element("style").unwrap();
        style.set_text_content(Some(CSS));
        document().head().unwrap().append_child(&style).unwrap();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handles = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let out = handles.clone();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
            let mut h = out.borrow_mut();
            let mut el = |parent: Option<&NodeHandle>, class: &'static str| {
                let n = scope.create_element("div");
                n.set_attribute("class", class);
                if let Some(p) = parent {
                    p.append_child(&n);
                }
                h.push((class, n.clone()));
                n
            };
            let wrap = el(None, "lb-wrap");
            let c = el(Some(&wrap), "lb-c");
            el(Some(&c), "lb-direct");
            let p = el(Some(&c), "lb-p");
            el(Some(&p), "lb-n");
            el(Some(&c), "lb-fixed");
            el(Some(&c), "lb-frac");
            let s = el(Some(&wrap), "lb-static");
            el(Some(&s), "lb-in");
            let t = el(Some(&wrap), "lb-t");
            el(Some(&t), "lb-tin");
            wrap
        });
        let handles = handles.borrow().clone();
        Self {
            root,
            host,
            style,
            handles,
        }
    }

    fn h(&self, class: &str) -> &NodeHandle {
        &self.handles.iter().find(|(c, _)| *c == class).unwrap().1
    }

    fn box_in(&self, node: &str, container: &str) -> Option<ElementBounds> {
        self.h(node).layout_box_in(self.h(container))
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
        self.style.remove();
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Option<ElementBounds> {
    Some(ElementBounds {
        x,
        y,
        width,
        height,
    })
}

/// A child of a bordered, padded container: from its padding edge.
#[wasm_bindgen_test]
fn a_child_is_measured_from_the_containers_padding_edge() {
    let f = Fixture::mount();
    assert_eq!(f.box_in("lb-direct", "lb-c"), rect(19.0, 4.0, 40.0, 10.0));
    f.teardown();
}

/// Inside a positioned, bordered, translated box: its border and padding
/// count, its translate does not, nor the item's own scale.
#[wasm_bindgen_test]
fn a_nested_positioned_box_and_transforms() {
    let f = Fixture::mount();
    assert_eq!(f.box_in("lb-n", "lb-c"), rect(22.0, 27.0, 50.0, 20.0));
    assert_eq!(f.box_in("lb-n", "lb-p"), rect(3.0, 11.0, 50.0, 20.0));
    // A positive control: the browser paints `lb-n` scaled and translated.
    let painted = f.h("lb-n").bounding_rect().expect("a painted box");
    assert_eq!((painted.width, painted.height), (100.0, 40.0));
    f.teardown();
}

/// No `offsetParent` chain reaches the container: it is not positioned, or
/// the node is `position: fixed`.
#[wasm_bindgen_test]
fn none_without_an_offset_parent_chain_to_the_container() {
    let f = Fixture::mount();
    assert_eq!(f.box_in("lb-in", "lb-static"), None);
    assert_eq!(f.box_in("lb-fixed", "lb-c"), None);
    // The control: the static box's child is laid out and has an offset
    // parent, just not that box.
    let el: web_sys::HtmlElement = document()
        .query_selector(".lb-in")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert!(el.offset_parent().is_some());
    assert_eq!(el.offset_width(), 10);
    f.teardown();
}

/// A transformed box that is not positioned is an offset parent all the
/// same: it is a containing block for absolutes, which is what the engines
/// answer (Chrome 157, Firefox 160), and desktop with them.
#[wasm_bindgen_test]
fn a_transformed_container_is_an_offset_parent() {
    let f = Fixture::mount();
    assert_eq!(f.box_in("lb-tin", "lb-t"), rect(4.0, 0.0, 10.0, 10.0));
    f.teardown();
}

/// At a fractional offset (8.25px) with a fractional width (20.5px), the
/// browser answers whole pixels, as desktop's rounded layout does.
#[wasm_bindgen_test]
fn a_fractional_box_answers_whole_pixels() {
    let f = Fixture::mount();
    assert_eq!(f.box_in("lb-frac", "lb-c"), rect(8.0, 84.0, 21.0, 10.0));
    f.teardown();
}

/// What makes a box an `offsetParent`, in both engines (Chrome 158, Firefox
/// 160), the twin of desktop's list (`rinch-dom/tests/layout_box_in_tests.rs`).
const OFFSET_PARENTS: &[&str] = &[
    "position: relative",
    "transform: translateX(1px)",
    "filter: blur(0px)",
    "filter: opacity(1)",
    "backdrop-filter: blur(1px)",
    "perspective: 100px",
    "contain: paint",
    "contain: layout",
    "contain: strict",
    "contain: content",
    "will-change: transform",
    "will-change: filter",
    "will-change: backdrop-filter",
    "will-change: perspective",
    "will-change: contain",
    "will-change: position",
];

/// What makes neither.
const NOT_OFFSET_PARENTS: &[&str] = &[
    "",
    "transform: none",
    "filter: none",
    "backdrop-filter: none",
    "perspective: none",
    "contain: size",
    "contain: style",
    "contain: inline-size",
    "container-type: size",
    "clip-path: inset(0)",
    "isolation: isolate",
    "mix-blend-mode: multiply",
    "opacity: 0.5",
    "overflow: hidden",
    "z-index: 1",
    "will-change: opacity",
    "will-change: left",
];

/// For each declaration, a box 30px into the page holding a box 4px into it:
/// the inner box's box in the outer one.
fn offset_parent_answers(decls: &[&str]) -> Vec<(String, Option<ElementBounds>)> {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let pairs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let out = pairs.clone();
    let decls: Vec<String> = decls.iter().map(|d| d.to_string()).collect();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let wrap = scope.create_element("div");
        for decl in &decls {
            let c = scope.create_element("div");
            c.set_attribute(
                "style",
                &format!("margin: 0 0 0 30px; width: 200px; height: 20px; {decl}"),
            );
            let k = scope.create_element("div");
            k.set_attribute("style", "margin: 0 0 0 4px; width: 10px; height: 10px");
            c.append_child(&k);
            wrap.append_child(&c);
            out.borrow_mut().push((decl.clone(), c, k));
        }
        wrap
    });
    let answers = pairs
        .borrow()
        .iter()
        .map(|(decl, c, k)| (decl.clone(), k.layout_box_in(c)))
        .collect();
    root.unmount();
    host.remove();
    answers
}

/// Each declaration the engines count makes an offset parent (#3065 review:
/// desktop counted only `position`, `transform` and `filter`).
#[wasm_bindgen_test]
fn every_declaration_the_engines_count_makes_an_offset_parent() {
    let wrong: Vec<_> = offset_parent_answers(OFFSET_PARENTS)
        .into_iter()
        .filter(|(_, b)| *b != rect(4.0, 0.0, 10.0, 10.0))
        .collect();
    assert!(wrong.is_empty(), "not an offset parent: {wrong:?}");
}

/// And none of the others does.
#[wasm_bindgen_test]
fn no_other_declaration_makes_an_offset_parent() {
    let wrong: Vec<_> = offset_parent_answers(NOT_OFFSET_PARENTS)
        .into_iter()
        .filter(|(_, b)| b.is_some())
        .collect();
    assert!(wrong.is_empty(), "an offset parent: {wrong:?}");
}

/// A box at `margin-left: inner`, `width` wide, inside a positioned box at
/// `left: outer` (all px): its box in the positioned one.
fn rounded_in(outer: f32, inner: f32, width: f32) -> Option<ElementBounds> {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let pair = std::rc::Rc::new(std::cell::RefCell::new(None));
    let out = pair.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let c = scope.create_element("div");
        c.set_attribute(
            "style",
            &format!("position: relative; left: {outer}px; margin: 0; width: 100px; height: 10px"),
        );
        let k = scope.create_element("div");
        k.set_attribute(
            "style",
            &format!("margin: 0 0 0 {inner}px; width: {width}px; height: 5px"),
        );
        c.append_child(&k);
        *out.borrow_mut() = Some((c.clone(), k));
        c
    });
    let (c, k) = pair.borrow_mut().take().unwrap();
    let answer = k.layout_box_in(&c);
    root.unmount();
    host.remove();
    answer
}

/// Each offset rounded on its own: half up in Chrome, as desktop rounds; a
/// negative tie away from zero in Firefox. Every other row is both engines'
/// answer, and desktop's (`offsets_round_half_up_each_on_its_own`).
#[wasm_bindgen_test]
fn offsets_round_each_on_its_own() {
    let x = |outer, inner| rounded_in(outer, inner, 10.0).map(|b| b.x);
    // A negative tie: Chrome (and desktop) half up, Firefox away from zero.
    let tie = |chrome: f32| {
        Some(if engine::is_gecko() {
            chrome - 1.0
        } else {
            chrome
        })
    };
    assert_eq!(x(0.0, 0.5), Some(1.0));
    assert_eq!(x(0.0, 2.5), Some(3.0));
    assert_eq!(x(0.0, -0.5), tie(0.0));
    assert_eq!(x(0.0, -1.5), tie(-1.0));
    assert_eq!(x(0.0, -8.5), tie(-8.0));
    assert_eq!(x(0.0, -0.75), Some(-1.0));
    assert_eq!(x(0.0, 0.25), Some(0.0));
    assert_eq!(x(0.5, -0.5), tie(0.0));
    assert_eq!(x(0.5, 0.5), Some(1.0));
    assert_eq!(x(0.5, -8.5), tie(-8.0));
    assert_eq!(x(0.25, 0.25), Some(0.0));
    let w = |inner, width| rounded_in(0.0, inner, width).map(|b| b.width);
    assert_eq!(w(0.5, 20.5), Some(21.0));
    assert_eq!(w(0.5, 20.25), Some(20.0));
    assert_eq!(w(-0.5, 20.5), Some(21.0));
    assert_eq!(w(0.25, 20.5), Some(21.0));
}
