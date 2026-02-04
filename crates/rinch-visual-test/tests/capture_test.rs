//! Integration test for capture protocol.

use rinch_visual_test::capture::RinchCapture;

#[test]
#[ignore] // Requires a running rinch app with debug enabled
fn test_capture_screenshot() {
    let mut capture = RinchCapture::connect().expect("Failed to connect");
    let png_bytes = capture.screenshot().expect("Failed to capture screenshot");

    // Verify it's a PNG
    assert!(png_bytes.starts_with(b"\x89PNG"), "Should be a valid PNG");
    assert!(png_bytes.len() > 100, "PNG should have data");
}

#[test]
#[ignore] // Requires a running rinch app with debug enabled
fn test_capture_dom_tree() {
    let mut capture = RinchCapture::connect().expect("Failed to connect");
    let dom = capture.dom_tree().expect("Failed to get DOM tree");

    // Should have a root node
    assert!(dom.is_object(), "DOM should be an object");
}

#[test]
#[ignore] // Requires a running rinch app with debug enabled
fn test_capture_click() {
    let mut capture = RinchCapture::connect().expect("Failed to connect");
    capture.click(100.0, 100.0).expect("Failed to click");
    capture.wait_frame().expect("Failed to wait for frame");
}
