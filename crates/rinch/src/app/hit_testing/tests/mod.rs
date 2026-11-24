use super::{
    ScrollAxis, detect_resize_edge, find_horizontal_scroll_container,
    find_horizontal_scroll_container_at_point, find_scroll_container,
    find_scroll_container_at_point, find_scrollbar_hit, painted_element_box,
};
use super::{flow_extent, hit_test};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

mod clip;
mod extent;
mod fixed_and_inline;
mod positioned;
mod resize_edge;
mod scroll;
mod transforms;

fn child_of(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let el = doc.create_element("div");
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}
