//! The [`DomDocument`] trait: the five supertraits a document implements,
//! named as one bound.

use super::content::DomContent;
use super::focus::DomFocus;
use super::layout::DomLayout;
use super::scroll::DomScroll;
use super::tree::DomTree;

/// Trait for DOM documents that support mutation operations.
///
/// This trait abstracts the DOM mutation API, allowing different
/// implementations (rinch-dom, web-sys, etc.) to be used.
///
/// # Supertraits
///
/// `DomDocument` has no methods of its own. They live in five supertraits,
/// one per responsibility (#2999):
///
/// - [`DomTree`]: the document's identity, node creation and tree mutation,
///   retiring nodes, dirty tracking, selectors, traversal and HTML parsing.
/// - [`DomContent`]: what a node holds — text, attributes, inline styles and
///   highlight overlays.
/// - [`DomFocus`]: focus, text-control selection and the work deferred until
///   the document's borrow is released.
/// - [`DomLayout`]: layout results, node boxes and text geometry.
/// - [`DomScroll`]: scroll offsets, extents and requests, and the page scroll
///   lock.
///
/// Every type that implements all five implements `DomDocument` through the
/// blanket impl, so an implementor writes one `impl` per supertrait and none
/// for `DomDocument` itself.
///
/// Code that takes a document keeps naming `DomDocument`: `&dyn DomDocument`,
/// `Rc<RefCell<dyn DomDocument>>` and `D: DomDocument` call every supertrait's
/// methods without importing them. Calling one on a concrete document type
/// (`RinchDocument`, `MockDomDocument`), or by path (`DomTree::body(doc)`),
/// needs the supertrait that declares it in scope; `rinch_core::dom`, the
/// `rinch_core` root and `rinch::fine_grained` export all five.
///
/// # Required Operations
///
/// Implementors must provide the required methods of each supertrait; see
/// the lists on [`DomTree`] and [`DomContent`].
pub trait DomDocument: DomTree + DomContent + DomFocus + DomLayout + DomScroll {}

impl<T: DomTree + DomContent + DomFocus + DomLayout + DomScroll + ?Sized> DomDocument for T {}
