use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

#[test]
fn test_text_node_has_nonzero_size() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "display: flex; width: 400px");
    doc.append_child(body, div);

    let text = doc.create_text("Hello, world!");
    doc.append_child(div, text);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(text.0).unwrap().layout;
    assert!(
        layout.width > 0.0,
        "text width should be > 0, got {}",
        layout.width
    );
    assert!(
        layout.height > 0.0,
        "text height should be > 0, got {}",
        layout.height
    );
}

#[test]
fn test_longer_text_is_wider() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: column; width: 800px",
    );
    doc.append_child(body, container);

    let short = doc.create_text("Hi");
    let long = doc.create_text("Hello, this is a much longer text string!");
    let div1 = doc.create_element("div");
    let div2 = doc.create_element("div");
    doc.set_attribute(div1, "style", "display: flex");
    doc.set_attribute(div2, "style", "display: flex");
    doc.append_child(container, div1);
    doc.append_child(container, div2);
    doc.append_child(div1, short);
    doc.append_child(div2, long);

    doc.resolve_layout(800.0, 600.0);

    let short_w = doc.tree.get(short.0).unwrap().layout.width;
    let long_w = doc.tree.get(long.0).unwrap().layout.width;
    assert!(
        long_w > short_w,
        "longer text ({}) should be wider than short text ({})",
        long_w,
        short_w
    );
}

#[test]
fn test_text_wraps_in_constrained_width() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "width: 80px");
    doc.append_child(body, container);

    let text = doc.create_text(
        "This is a long sentence that should wrap to multiple lines in a narrow container.",
    );
    doc.append_child(container, text);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(text.0).unwrap().layout;
    // Text should wrap - height should be more than a single line
    assert!(
        layout.height > 30.0,
        "wrapped text height should be > 30, got {}",
        layout.height
    );
    // Width should not exceed container
    assert!(
        layout.width <= 80.0,
        "text width ({}) should fit in container (80px)",
        layout.width
    );
}

#[test]
fn test_font_size_affects_text_height() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let small = doc.create_element("div");
    doc.set_attribute(
        small,
        "style",
        "display: flex; font-size: 12px; width: 400px",
    );
    doc.append_child(body, small);
    let small_text = doc.create_text("Hello");
    doc.append_child(small, small_text);

    let large = doc.create_element("div");
    doc.set_attribute(
        large,
        "style",
        "display: flex; font-size: 32px; width: 400px",
    );
    doc.append_child(body, large);
    let large_text = doc.create_text("Hello");
    doc.append_child(large, large_text);

    doc.resolve_layout(800.0, 600.0);

    let small_h = doc.tree.get(small_text.0).unwrap().layout.height;
    let large_h = doc.tree.get(large_text.0).unwrap().layout.height;
    assert!(
        large_h > small_h,
        "larger font ({}) should produce taller text ({})",
        large_h,
        small_h
    );
}

#[test]
fn test_text_next_to_element() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: row; width: 400px",
    );
    doc.append_child(body, container);

    let box_el = doc.create_element("div");
    doc.set_attribute(box_el, "style", "width: 50px; height: 50px");
    doc.append_child(container, box_el);

    let text = doc.create_text("Label text");
    doc.append_child(container, text);

    doc.resolve_layout(800.0, 600.0);

    let box_layout = doc.tree.get(box_el.0).unwrap().layout;
    let text_layout = doc.tree.get(text.0).unwrap().layout;
    assert_eq!(box_layout.x, 0.0);
    assert!(
        text_layout.x >= 50.0,
        "text x ({}) should be >= box width (50)",
        text_layout.x
    );
    assert!(text_layout.width > 0.0);
}

#[test]
fn test_empty_text_has_zero_width() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "display: flex; width: 400px");
    doc.append_child(body, div);

    let text = doc.create_text("");
    doc.append_child(div, text);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(text.0).unwrap().layout;
    assert_eq!(layout.width, 0.0);
}

#[test]
fn test_set_text_content_updates_measurement() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "display: flex; width: 400px");
    doc.append_child(body, div);

    let text = doc.create_text("Short");
    doc.append_child(div, text);
    doc.resolve_layout(800.0, 600.0);
    let w1 = doc.tree.get(text.0).unwrap().layout.width;

    doc.set_text_content(text, "This is a much longer string now");
    doc.resolve_layout(800.0, 600.0);
    let w2 = doc.tree.get(text.0).unwrap().layout.width;

    assert!(
        w2 > w1,
        "updated text ({}) should be wider than original ({})",
        w2,
        w1
    );
}

#[test]
fn test_nested_flex_column_layout() {
    let mut doc = RinchDocument::new();

    // Load CSS matching widget styles
    doc.load_css(r#"
        .rinch-borderlesswindow { display: flex; flex-direction: column; height: 100vh; width: 100vw; }
        .rinch-borderlesswindow__content { flex: 1; min-height: 0; overflow-y: auto; }
        .rinch-center { display: flex; align-items: center; justify-content: center; }
        .rinch-stack { display: flex; flex-direction: column; }
        .rinch-stack--gap-md { gap: 16px; }
        .rinch-title--1 { font-size: 34px; font-weight: 700; }
    "#);

    let body = doc.body();

    let bw = doc.create_element("div");
    doc.set_attribute(bw, "class", "rinch-borderlesswindow");
    doc.append_child(body, bw);

    let content = doc.create_element("div");
    doc.set_attribute(content, "class", "rinch-borderlesswindow__content");
    doc.append_child(bw, content);

    let center = doc.create_element("div");
    doc.set_attribute(center, "class", "rinch-center");
    doc.append_child(content, center);

    let stack = doc.create_element("div");
    doc.set_attribute(stack, "class", "rinch-stack rinch-stack--gap-md");
    doc.append_child(center, stack);

    let h1 = doc.create_element("h1");
    doc.set_attribute(h1, "class", "rinch-title--1");
    doc.append_child(stack, h1);

    let text = doc.create_text("Hello World");
    doc.append_child(h1, text);

    doc.resolve_layout(1200.0, 800.0);

    // Print all layouts for debugging
    for (id, node) in doc.tree.nodes.iter() {
        let tag = node
            .tag()
            .unwrap_or(if node.is_text() { "#text" } else { "?" });
        let cls = node
            .attributes
            .get("class")
            .map(|s| s.as_str())
            .unwrap_or("");
        let l = &node.layout;
        eprintln!(
            "  id={id} {tag}.{cls} {:.0}x{:.0} @{:.0},{:.0}",
            l.width, l.height, l.x, l.y
        );
    }

    let text_layout = doc.tree.get(text.0).unwrap().layout;
    assert!(
        text_layout.width > 0.0,
        "text width = {}",
        text_layout.width
    );
    assert!(
        text_layout.height > 0.0,
        "text height = {}",
        text_layout.height
    );

    let h1_layout = doc.tree.get(h1.0).unwrap().layout;
    assert!(h1_layout.width > 0.0, "h1 width = {}", h1_layout.width);
    assert!(h1_layout.height > 0.0, "h1 height = {}", h1_layout.height);

    let content_layout = doc.tree.get(content.0).unwrap().layout;
    assert!(
        content_layout.height > 0.0,
        "content height = {}",
        content_layout.height
    );
}

#[test]
fn test_style_tag_then_widget_nodes() {
    let mut doc = RinchDocument::new();

    // Load widget CSS first (like runtime does)
    doc.load_css(".rinch-center { display: flex; align-items: center; justify-content: center; }");
    doc.load_css(".rinch-stack { display: flex; flex-direction: column; }");

    let body = doc.body();

    // Create style element with app CSS (triggers recompute_all_styles)
    let style_el = doc.create_element("style");
    doc.append_child(body, style_el);
    let style_text = doc.create_text("* { box-sizing: border-box; margin: 0; padding: 0; }");
    doc.append_child(style_el, style_text);

    // Create widget nodes AFTER style tag
    let main = doc.create_element("div");
    doc.set_attribute(main, "class", "main-content");
    doc.append_child(body, main);

    let center = doc.create_element("div");
    doc.set_attribute(center, "class", "rinch-center");
    doc.append_child(main, center);

    let stack = doc.create_element("div");
    doc.set_attribute(stack, "class", "rinch-stack");
    doc.append_child(center, stack);

    let h1 = doc.create_element("h1");
    doc.append_child(stack, h1);
    let text = doc.create_text("Hello World");
    doc.append_child(h1, text);

    doc.resolve_layout(1200.0, 800.0);

    for (id, node) in doc.tree.nodes.iter() {
        let tag = node
            .tag()
            .unwrap_or(if node.is_text() { "#text" } else { "?" });
        let cls = node
            .attributes
            .get("class")
            .map(|s| s.as_str())
            .unwrap_or("");
        let l = &node.layout;
        eprintln!(
            "  id={id} {tag}.{cls} {:.0}x{:.0} @{:.0},{:.0}",
            l.width, l.height, l.x, l.y
        );
    }

    let text_layout = doc.tree.get(text.0).unwrap().layout;
    assert!(
        text_layout.width > 0.0,
        "text width = {}",
        text_layout.width
    );
    assert!(
        text_layout.height > 0.0,
        "text height = {}",
        text_layout.height
    );

    let center_layout = doc.tree.get(center.0).unwrap().layout;
    assert!(
        center_layout.width > 0.0,
        "center width = {}",
        center_layout.width
    );
    assert!(
        center_layout.height > 0.0,
        "center height = {}",
        center_layout.height
    );
}

#[test]
fn test_fragment_span_wrapper() {
    // Reproduces the runtime structure: main-content > span[data-fragment] > rinch-center > text
    let mut doc = RinchDocument::new();
    doc.load_css(".rinch-center { display: flex; align-items: center; justify-content: center; }");
    doc.load_css(".rinch-stack { display: flex; flex-direction: column; }");
    doc.load_css(".rinch-title--1 { font-size: 34px; font-weight: 700; }");
    doc.load_css(".main-content { padding: 32px; }");

    let body = doc.body();

    let main = doc.create_element("div");
    doc.set_attribute(main, "class", "main-content");
    doc.append_child(body, main);

    // Fragment div wrapper (like rsx! macro generates)
    let frag = doc.create_element("div");
    doc.set_attribute(frag, "data-fragment", "true");
    doc.append_child(main, frag);

    let center = doc.create_element("div");
    doc.set_attribute(center, "class", "rinch-center");
    doc.append_child(frag, center);

    let stack = doc.create_element("div");
    doc.set_attribute(stack, "class", "rinch-stack");
    doc.append_child(center, stack);

    let h1 = doc.create_element("h1");
    doc.set_attribute(h1, "class", "rinch-title--1");
    doc.append_child(stack, h1);

    let text = doc.create_text("Welcome to UI Zoo");
    doc.append_child(h1, text);

    doc.resolve_layout(1200.0, 800.0);

    for (id, node) in doc.tree.nodes.iter() {
        let tag = node
            .tag()
            .unwrap_or(if node.is_text() { "#text" } else { "?" });
        let cls = node
            .attributes
            .get("class")
            .map(|s| s.as_str())
            .unwrap_or("");
        let l = &node.layout;
        eprintln!(
            "  id={id} {tag}.{cls} {:.0}x{:.0} @{:.0},{:.0}",
            l.width, l.height, l.x, l.y
        );
    }

    let text_layout = doc.tree.get(text.0).unwrap().layout;
    assert!(
        text_layout.width > 0.0,
        "text width = {}",
        text_layout.width
    );

    let frag_layout = doc.tree.get(frag.0).unwrap().layout;
    assert!(
        frag_layout.width > 0.0,
        "fragment span width = {}",
        frag_layout.width
    );
    assert!(
        frag_layout.height > 0.0,
        "fragment span height = {}",
        frag_layout.height
    );
}
