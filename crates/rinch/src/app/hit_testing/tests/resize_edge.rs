/// #423: a corner is where two edge zones meet, and nothing more.
///
/// The `corner = inset * 2` radius this function used to carry never took
/// effect — each of its four guards was implied by the `near_*` its arm
/// already required — so removing it is an identity. This pins that, and
/// pins the geometry CLAUDE.md now documents.
#[test]
fn the_corner_is_where_two_edge_zones_meet() {
    use super::detect_resize_edge;
    use rinch_platform::ResizeDirection::*;
    let at = |x, y| detect_resize_edge(x, y, 800.0, 600.0, 8.0);

    // Inside one zone only — an edge, on all four sides.
    assert_eq!(at(4.0, 300.0), Some(West));
    assert_eq!(at(796.0, 300.0), Some(East));
    assert_eq!(at(400.0, 4.0), Some(North));
    assert_eq!(at(400.0, 596.0), Some(South));

    // Inside two — a corner, on all four.
    assert_eq!(at(4.0, 4.0), Some(NorthWest));
    assert_eq!(at(796.0, 4.0), Some(NorthEast));
    assert_eq!(at(4.0, 596.0), Some(SouthWest));
    assert_eq!(at(796.0, 596.0), Some(SouthEast));

    // 12px in from the left and 4px down: inside the old `inset * 2` corner
    // radius on x, outside the edge zone on x. A corner that actually
    // enlarged would answer NorthWest here. It does not, and never did.
    assert_eq!(at(12.0, 4.0), Some(North));
    assert_eq!(at(4.0, 12.0), Some(West));

    // And outside everything.
    assert_eq!(at(400.0, 300.0), None);
    assert_eq!(at(12.0, 12.0), None);
}
