use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
#[test]
#[ignore = "WIP probe for #2326; delete before the PR"]
fn probe() {
    let cases: std::collections::BTreeMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/zz_probe_2326_cases.json"
        ))
        .unwrap(),
    )
    .unwrap();
    for (k, v) in cases {
        use parley::fontique::{Blob, FontInfoOverride};
        let mut d = RinchDocument::new();
        d.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(FACE)),
            Some(FontInfoOverride {
                family_name: Some("P"),
                ..Default::default()
            }),
        );
        d.load_css("body{margin:0;font:16px/20px P} #host{width:560px}");
        let body = d.body();
        d.set_inner_html(body, &format!("<div id=host>{v}</div>"));
        d.resolve_layout(800.0, 800.0);
        let mut out = vec![];
        for id in ["ib", "f", "w", "t", "c", "o", "i"] {
            if let Some(n) = d.query_selector(&format!("#{id}")) {
                let l = d.tree.get(n.0).unwrap().layout;
                let (x, y) = rinch_dom::paint::compute_absolute_position(&d.tree, n.0, 1.0);
                out.push(format!(
                    "{id}:[{:.2},{:.2},{:.2},{:.2}]",
                    x, y, l.width, l.height
                ));
            }
        }
        println!("{k} {}", out.join(" "));
    }
}
