//! Item ids. Integers, issued by the [`Graph`](crate::Graph) and never reused
//! within one graph, so an id held across an undo still names the same item
//! when a redo brings it back.

use std::fmt;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name(pub u64);

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(self, f)
            }
        }
    };
}

id_type!(
    /// A node in a [`Graph`](crate::Graph).
    NodeId,
    "n"
);
id_type!(
    /// A port on a node.
    PortId,
    "p"
);
id_type!(
    /// An edge from an output port to an input port.
    EdgeId,
    "e"
);
