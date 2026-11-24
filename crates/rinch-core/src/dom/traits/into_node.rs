//! [`IntoNode`]: converting component results and text values into DOM nodes,
//! as `rsx!` does for every child expression.

use crate::dom::{NodeHandle, RenderScope};

/// Trait for converting values into DOM nodes.
///
/// This trait enables the rsx! macro to handle both NodeHandle returns
/// (from component functions) and text values in a uniform way.
pub trait IntoNode {
    /// Convert this value into a DOM node.
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle;
}

impl IntoNode for NodeHandle {
    #[inline]
    fn into_node(self, _scope: &mut RenderScope) -> NodeHandle {
        self
    }
}

impl IntoNode for String {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(&self)
    }
}

impl IntoNode for &str {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(self)
    }
}

impl IntoNode for &String {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(self.as_str())
    }
}

impl IntoNode for std::borrow::Cow<'_, str> {
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        scope.create_text(self.as_ref())
    }
}

impl IntoNode for Vec<NodeHandle> {
    /// Append all NodeHandles as children of a transparent wrapper.
    ///
    /// This enables the `.iter().map(|x| rsx!{...}).collect::<Vec<_>>()` pattern in RSX.
    /// For reactive lists that update when signals change, prefer `for` loops with keyed
    /// reconciliation instead.
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        let container = scope.create_element("div");
        container.set_attribute("style", "display:contents");
        for child in &self {
            container.append_child(child);
        }
        container
    }
}

impl IntoNode for Option<NodeHandle> {
    /// A present-or-absent child: `Some(node)` renders the node, `None` renders
    /// nothing. Routed through the `Vec<NodeHandle>` impl (a 0-or-1 list) so a
    /// `maybe.map(|t| rsx! { … })` embeds directly as `{maybe}` — no
    /// `.into_iter().collect::<Vec<_>>()` dance. For a child that toggles
    /// reactively, prefer an `if`/`if let` in rsx instead.
    #[inline]
    fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
        self.into_iter()
            .collect::<Vec<NodeHandle>>()
            .into_node(scope)
    }
}

// Implement IntoNode for numeric types
macro_rules! impl_into_node_for_display {
    ($($ty:ty),*) => {
        $(
            impl IntoNode for $ty {
                #[inline]
                fn into_node(self, scope: &mut RenderScope) -> NodeHandle {
                    scope.create_text(&self.to_string())
                }
            }
        )*
    };
}

impl_into_node_for_display!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, bool, char
);
