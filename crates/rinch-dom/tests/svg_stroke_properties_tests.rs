//! Inline SVG's stroke properties, on both renderers: `stroke-dasharray`,
//! `stroke-dashoffset`, `stroke-miterlimit`, `stroke-opacity`,
//! `fill-opacity`, `opacity` on shapes and groups, `paint-order`,
//! `vector-effect: non-scaling-stroke`, and `stroke-width` as a length
//! (initial `1`, percentages, units, the `style` attribute).
//!
//! `paint/svg.rs` used to read only `stroke`, `stroke-width` (a bare number,
//! initial `2`), `stroke-linecap` and `stroke-linejoin`: a dashed path drew
//! solid, an opacity drew opaque, and an unknown cap or join keyword
//! replaced the inherited one with butt / miter.
//!
//! **The oracle is Chrome 152**: each case's markup in a 100x100 `<svg>` with
//! the case's `viewBox`, drawn into a canvas, each probe pixel read with
//! `getImageData` (unpremultiplied RGBA). Every probe is at least 1.5px from
//! any edge of the geometry, so its pixel is fully covered or untouched on
//! every renderer. Probes sit off the fixed points: a percentage in a
//! viewBox whose diagonal is not 100, a non-zero dash offset (and a negative
//! one), a miter limit between the join's ratio and the default, each where
//! the plausible misreading paints the opposite answer.
//!
//! The software painter runs everywhere. The same table runs on Vello in
//! `gpu_*`, `#[ignore]`d for hosts without a GPU adapter:
//! `cargo test -p rinch-dom --test svg_stroke_properties_tests -- --ignored`.

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::Painter;

const G: [u8; 4] = [0, 128, 0, 255];
const R: [u8; 4] = [255, 0, 0, 255];
const E: [u8; 4] = [0, 0, 0, 0];
const fn green(a: u8) -> [u8; 4] {
    [0, 128, 0, a]
}
const fn blue(a: u8) -> [u8; 4] {
    [0, 0, 255, a]
}

/// One case: a viewBox, the markup inside the `<svg>`, and Chrome 152's
/// unpremultiplied pixel at each `(x, y)`.
struct Case {
    name: &'static str,
    viewbox: &'static str,
    markup: String,
    probes: Vec<(u32, u32, [u8; 4])>,
}

/// A 10-wide green horizontal line across the middle, with `attrs`.
fn line(attrs: &str) -> String {
    format!(r#"<line x1="0" y1="50" x2="100" y2="50" stroke="green" stroke-width="10" {attrs}/>"#)
}

/// `line` under a `<g>` with `group`.
fn in_group(group: &str, inner: String) -> String {
    format!("<g {group}>{inner}</g>")
}

/// A 90° V with its apex up at (50, 50), 20 wide: the miter's tip reaches
/// y = 35.9 (ratio √2), a bevel only y = 42.9, so (50, 39) is ink only for
/// a miter.
fn vee(attrs: &str) -> String {
    format!(
        r#"<polyline points="10,90 50,50 90,90" fill="none" stroke="green" stroke-width="20" {attrs}/>"#
    )
}

/// A blue-filled square (20..80) with a 10-wide green stroke (15..25 at
/// its left side).
fn framed(fill: &str, stroke: &str, attrs: &str) -> String {
    format!(
        r#"<rect x="20" y="20" width="60" height="60" fill="{fill}" stroke="{stroke}" stroke-width="10" {attrs}/>"#
    )
}

/// A line from (10, 50) to (90, 50) on the device, drawn 5x scaled.
fn scaled(attrs: &str) -> String {
    format!(
        r#"<g transform="scale(5)"><line x1="2" y1="10" x2="18" y2="10" stroke="green" stroke-width="2" {attrs}/></g>"#
    )
}

/// A width-less line covering exactly row 50 at width 1.
fn hairline(attrs: &str) -> String {
    format!(r#"<line x1="10" y1="50.5" x2="90" y2="50.5" stroke="green" {attrs}/>"#)
}

fn row(xs: &[u32], want: &[[u8; 4]]) -> Vec<(u32, u32, [u8; 4])> {
    xs.iter().zip(want).map(|(&x, &w)| (x, 50, w)).collect()
}

fn case(name: &'static str, markup: String, probes: Vec<(u32, u32, [u8; 4])>) -> Case {
    Case {
        name,
        viewbox: "0 0 100 100",
        markup,
        probes,
    }
}

fn cases() -> Vec<Case> {
    // 20 on, 10 off from x = 0.
    let dashed = || row(&[10, 25, 40, 55], &[G, E, G, E]);
    let solid = || row(&[10, 25, 40, 55], &[G, G, G, G]);
    let mut v = vec![
        // ── stroke-dasharray ────────────────────────────────────────────
        case("dasharray", line(r#"stroke-dasharray="20 10""#), dashed()),
        case("dasharray with commas", line(r#"stroke-dasharray="20,10""#), dashed()),
        case("dasharray with units", line(r#"stroke-dasharray="20px, 10px""#), dashed()),
        // An odd list repeats: 20 10 5 20 10 5.
        case(
            "odd dasharray repeats",
            line(r#"stroke-dasharray="20 10 5""#),
            row(&[10, 25, 32, 45, 60, 67, 80], &[G, E, G, E, G, E, G]),
        ),
        // A negative value is an error, so the declaration is ignored: the
        // group's dashes apply, and alone it is the initial `none`.
        case(
            "negative dasharray is ignored",
            in_group(r#"stroke-dasharray="20 10""#, line(r#"stroke-dasharray="-5 10""#)),
            dashed(),
        ),
        case("negative dasharray alone", line(r#"stroke-dasharray="-5 10""#), solid()),
        case(
            "negative dasharray in style is dropped",
            in_group(
                r#"stroke-dasharray="20 10""#,
                line(r#"style="stroke-dasharray: -5 10""#),
            ),
            dashed(),
        ),
        case(
            "unparsable dasharray is ignored",
            in_group(r#"stroke-dasharray="20 10""#, line(r#"stroke-dasharray="5 x""#)),
            dashed(),
        ),
        // All zero draws solid; so does `none` over a dashed group.
        case(
            "all-zero dasharray is solid",
            in_group(r#"stroke-dasharray="20 10""#, line(r#"stroke-dasharray="0 0""#)),
            solid(),
        ),
        case(
            "dasharray none over a dashed group",
            in_group(r#"stroke-dasharray="20 10""#, line(r#"stroke-dasharray="none""#)),
            row(&[25, 55], &[G, G]),
        ),
        case(
            "style beats the dasharray attribute",
            line(r#"stroke-dasharray="5 5" style="stroke-dasharray: 20 10""#),
            dashed(),
        ),
        // ── stroke-dashoffset ───────────────────────────────────────────
        // 5 into the pattern: on 0..15, off 15..25, on 25..45.
        case(
            "dashoffset",
            line(r#"stroke-dasharray="20 10" stroke-dashoffset="5""#),
            row(&[10, 20, 27, 50], &[G, E, G, E]),
        ),
        // -5: off 0..5, on 5..25, off 25..35, on 35..55.
        case(
            "negative dashoffset",
            line(r#"stroke-dasharray="20 10" stroke-dashoffset="-5""#),
            row(&[2, 15, 30, 45], &[E, G, E, G]),
        ),
        case(
            "dasharray and dashoffset inherit through a group",
            in_group(
                r#"stroke-dasharray="20 10" stroke-dashoffset="5""#,
                line(""),
            ),
            row(&[20, 27], &[E, G]),
        ),
        // ── stroke-miterlimit, stroke-linejoin, stroke-linecap ──────────
        case("miter join reaches its tip", vee(""), vec![(50, 39, G)]),
        case(
            "a miter limit under the ratio bevels",
            vee(r#"stroke-miterlimit="1.2""#),
            vec![(50, 39, E)],
        ),
        case(
            "a miter limit under 1 is ignored",
            in_group(r#"stroke-miterlimit="1.2""#, vee(r#"stroke-miterlimit="0.5""#)),
            vec![(50, 39, E)],
        ),
        case(
            "a child's miter join over a bevel group",
            in_group(r#"stroke-linejoin="bevel""#, vee(r#"stroke-linejoin="miter""#)),
            vec![(50, 39, G)],
        ),
        // Chrome 152 implements neither SVG 2 join: both are ignored.
        case(
            "miter-clip is ignored",
            in_group(r#"stroke-linejoin="bevel""#, vee(r#"stroke-linejoin="miter-clip""#)),
            vec![(50, 39, E)],
        ),
        case(
            "arcs is ignored",
            in_group(r#"stroke-linejoin="bevel""#, vee(r#"stroke-linejoin="arcs""#)),
            vec![(50, 39, E)],
        ),
        case(
            "an unknown join keyword is ignored",
            in_group(r#"stroke-linejoin="bevel""#, vee(r#"stroke-linejoin="bogus""#)),
            vec![(50, 39, E)],
        ),
        case(
            "an unknown cap keyword is ignored",
            in_group(
                r#"stroke-linecap="square""#,
                r#"<line x1="20" y1="50" x2="80" y2="50" stroke="green" stroke-width="20" stroke-linecap="bogus"/>"#
                    .to_string(),
            ),
            vec![(15, 50, G), (85, 50, G)],
        ),
        // ── stroke-opacity, fill-opacity ────────────────────────────────
        case(
            "stroke-opacity",
            line(r#"stroke-opacity="0.5""#),
            vec![(50, 50, green(128))],
        ),
        case(
            "stroke-opacity percentage inherits",
            in_group(r#"stroke-opacity="25%""#, line("")),
            vec![(50, 50, green(64))],
        ),
        case(
            "stroke-opacity clamps",
            line(r#"stroke-opacity="3""#),
            vec![(50, 50, G)],
        ),
        case(
            "style beats the stroke-opacity attribute",
            line(r#"stroke-opacity="1" style="stroke-opacity: .5""#),
            vec![(50, 50, green(128))],
        ),
        case(
            "fill-opacity",
            r#"<rect x="20" y="20" width="60" height="60" fill="green" fill-opacity=".5"/>"#.into(),
            vec![(50, 50, green(128))],
        ),
        // ── opacity ─────────────────────────────────────────────────────
        // The shape is composited whole: the stroke's inner half hides the
        // fill under it, so it is green at half alpha, not a mix.
        case(
            "opacity on a shape is a group",
            framed("blue", "green", r#"opacity=".5""#),
            vec![(22, 50, green(128)), (50, 50, blue(128))],
        ),
        case(
            "opacity in style",
            framed("blue", "green", r#"style="opacity: .5""#),
            vec![(22, 50, green(128)), (50, 50, blue(128))],
        ),
        case(
            "opacity percentage",
            framed("blue", "green", r#"opacity="50%""#),
            vec![(22, 50, green(128))],
        ),
        case(
            "opacity on a group",
            in_group(
                r#"opacity=".5""#,
                r#"<rect x="10" y="10" width="50" height="50" fill="blue"/>
                   <rect x="40" y="40" width="50" height="50" fill="green"/>"#
                    .into(),
            ),
            vec![(50, 50, green(128)), (20, 20, blue(128)), (80, 80, green(128))],
        ),
        case(
            "nested group opacities multiply",
            in_group(
                r#"opacity=".5""#,
                in_group(
                    r#"opacity=".5""#,
                    r#"<rect x="10" y="10" width="80" height="80" fill="green"/>"#.into(),
                ),
            ),
            vec![(50, 50, green(64))],
        ),
        case(
            "opacity does not inherit",
            in_group(
                r#"opacity=".5""#,
                r#"<rect x="10" y="10" width="80" height="80" fill="green" opacity="1"/>"#.into(),
            ),
            vec![(50, 50, green(128))],
        ),
        // ── paint-order ─────────────────────────────────────────────────
        case(
            "fill then stroke by default",
            framed("green", "red", ""),
            vec![(22, 50, R)],
        ),
        case(
            "paint-order stroke",
            framed("green", "red", r#"paint-order="stroke""#),
            vec![(22, 50, G), (17, 50, R)],
        ),
        case(
            "paint-order inherits and fills in the rest",
            in_group(r#"paint-order="markers stroke""#, framed("green", "red", "")),
            vec![(22, 50, G)],
        ),
        case(
            "paint-order fill stroke",
            framed("green", "red", r#"paint-order="fill stroke""#),
            vec![(22, 50, R)],
        ),
        case(
            "a repeated paint-order keyword is ignored",
            in_group(
                r#"paint-order="stroke""#,
                framed("green", "red", r#"paint-order="stroke stroke""#),
            ),
            vec![(22, 50, G)],
        ),
        // ── vector-effect ───────────────────────────────────────────────
        case(
            "a scaled stroke",
            scaled(""),
            vec![(50, 46, G), (50, 50, G)],
        ),
        case(
            "non-scaling-stroke",
            scaled(r#"vector-effect="non-scaling-stroke""#),
            vec![(50, 46, E), (50, 49, G), (50, 50, G)],
        ),
        case(
            "non-scaling-stroke in style",
            scaled(r#"style="vector-effect: non-scaling-stroke""#),
            vec![(50, 46, E)],
        ),
        case(
            "non-scaling-stroke dashes",
            scaled(r#"vector-effect="non-scaling-stroke" stroke-dasharray="10 10""#),
            row(&[15, 25, 35], &[G, E, G]),
        ),
        case(
            "scaled dashes",
            scaled(r#"stroke-dasharray="10 10""#),
            row(&[15, 25, 35], &[G, G, G]),
        ),
        case(
            "vector-effect does not inherit",
            r#"<g transform="scale(5)" vector-effect="non-scaling-stroke"><line x1="2" y1="10" x2="18" y2="10" stroke="green" stroke-width="2"/></g>"#.into(),
            vec![(50, 46, G)],
        ),
        // ── stroke-width ────────────────────────────────────────────────
        case(
            "initial stroke-width is 1",
            hairline(""),
            vec![(50, 50, G), (50, 51, E), (50, 49, E)],
        ),
        case(
            "zero stroke-width strokes nothing",
            hairline(r#"stroke-width="0""#),
            vec![(50, 50, E)],
        ),
        case(
            "negative stroke-width is ignored",
            in_group(r#"stroke-width="10""#, hairline(r#"stroke-width="-3""#)),
            vec![(50, 47, G)],
        ),
        case(
            "unparsable stroke-width is ignored",
            in_group(r#"stroke-width="10""#, hairline(r#"stroke-width="abc""#)),
            vec![(50, 47, G)],
        ),
        case(
            "stroke-width in style",
            hairline(r#"stroke-width="1" style="stroke-width: 10px""#),
            vec![(50, 47, G)],
        ),
        case(
            "stroke-width in em",
            in_group(r#"style="font-size: 20px""#, hairline(r#"stroke-width="0.5em""#)),
            vec![(50, 47, G), (50, 44, E)],
        ),
    ];
    // A 200-unit viewBox: its normalized diagonal is 200, so 10% is 20
    // units (10px on the device), where a percentage read as a number would
    // be 10 units.
    v.extend([
        Case {
            name: "percentage dasharray",
            viewbox: "0 0 200 200",
            markup: r#"<line x1="0" y1="100" x2="200" y2="100" stroke="green" stroke-width="20" stroke-dasharray="10% 5%"/>"#.into(),
            probes: row(&[5, 12, 20, 27], &[G, E, G, E]),
        },
        Case {
            name: "percentage dashoffset",
            viewbox: "0 0 200 200",
            markup: r#"<line x1="0" y1="100" x2="200" y2="100" stroke="green" stroke-width="20" stroke-dasharray="20 20" stroke-dashoffset="5%"/>"#.into(),
            probes: row(&[2, 7, 12], &[G, E, E]),
        },
        Case {
            name: "percentage stroke-width",
            viewbox: "0 0 200 200",
            markup: r#"<line x1="20" y1="100" x2="180" y2="100" stroke="green" stroke-width="10%"/>"#.into(),
            probes: vec![(50, 46, G), (50, 53, G), (50, 43, E), (50, 56, E)],
        },
    ]);
    v
}

/// Lay `case` out in a 100x100 document and paint it into `painter`.
fn paint_into(case: &Case, painter: &mut dyn Painter) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let host = doc.create_element("div");
    doc.append_child(body, host);
    doc.set_inner_html(
        host,
        &format!(
            "<svg viewBox=\"{}\" style=\"position: absolute; left: 0; top: 0; \
             display: block; width: 100px; height: 100px\">{}</svg>",
            case.viewbox, case.markup
        ),
    );
    doc.resolve_layout(100.0, 100.0);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        painter,
        1.0,
        (100.0, 100.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
}

/// Whether `got` (unpremultiplied) is Chrome's `want`: exact where opaque or
/// empty; within rounding of a half-covered alpha and its unpremultiply.
fn matches(got: [u8; 4], want: [u8; 4]) -> bool {
    if want[3] == 0 {
        return got[3] == 0;
    }
    if want[3] == 255 {
        return got == want;
    }
    (got[3] as i16 - want[3] as i16).abs() <= 2
        && (0..3).all(|c| (got[c] as i16 - want[c] as i16).abs() <= 4)
}

fn unpremultiply(p: [u8; 4]) -> [u8; 4] {
    match p[3] {
        0 | 255 => p,
        a => {
            let u = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            [u(p[0]), u(p[1]), u(p[2]), a]
        }
    }
}

/// Check every case's probes with `pixel`, reporting all that differ.
fn check_all(renderer: &str, mut paint: impl FnMut(&Case) -> Box<dyn Fn(u32, u32) -> [u8; 4]>) {
    let mut wrong = Vec::new();
    let mut probes = 0;
    for case in cases() {
        let pixel = paint(&case);
        for &(x, y, want) in &case.probes {
            probes += 1;
            let got = pixel(x, y);
            if !matches(got, want) {
                wrong.push(format!(
                    "{}: ({x},{y}) want {want:?} got {got:?}",
                    case.name
                ));
            }
        }
    }
    assert!(probes > 100, "positive control: {probes} probes ran");
    assert!(
        wrong.is_empty(),
        "{renderer}: probes that differ from Chrome 152: {wrong:#?}"
    );
}

#[cfg(feature = "software-renderer")]
#[test]
fn stroke_properties_match_chrome_on_the_software_painter() {
    use rinch_dom::paint::skia_painter::TinySkiaPainter;
    check_all("tiny-skia", |case| {
        let mut p = TinySkiaPainter::new(100, 100);
        paint_into(case, &mut p);
        let data = p.pixels().to_vec();
        Box::new(move |x, y| {
            let i = ((y * 100 + x) * 4) as usize;
            unpremultiply([data[i], data[i + 1], data[i + 2], data[i + 3]])
        })
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_stroke_properties_match_chrome_on_vello() {
    use rinch_dom::paint::vello_painter::VelloPainter;
    let gpu = gpu::Gpu::new();
    check_all("vello", |case| {
        let mut vp = VelloPainter::new();
        paint_into(case, &mut vp);
        let px = gpu.render(vp.scene());
        Box::new(move |x, y| px[(y * 100 + x) as usize])
    });
}

mod gpu {
    use peniko::Color;
    use vello::wgpu;

    const W: u32 = 100;
    const H: u32 = 100;

    pub struct Gpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderer: std::cell::RefCell<vello::Renderer>,
    }

    impl Gpu {
        pub fn new() -> Self {
            let instance = wgpu::Instance::new(
                wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
            );
            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    compatible_surface: None,
                    force_fallback_adapter: false,
                    apply_limit_buckets: false,
                }))
                .expect("a GPU adapter (this fixture is #[ignore]d for hosts without one)");
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                    .expect("a device");
            let renderer = vello::Renderer::new(
                &device,
                vello::RendererOptions {
                    antialiasing_support: vello::AaSupport::area_only(),
                    use_cpu: false,
                    num_init_threads: None,
                    pipeline_cache: None,
                },
            )
            .expect("vello");
            Self {
                device,
                queue,
                renderer: std::cell::RefCell::new(renderer),
            }
        }

        /// `scene` rendered over transparent, as RGBA rows.
        pub fn render(&self, scene: &vello::Scene) -> Vec<[u8; 4]> {
            let (device, queue) = (&self.device, &self.queue);
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: W,
                    height: H,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            self.renderer
                .borrow_mut()
                .render_to_texture(
                    device,
                    queue,
                    scene,
                    &view,
                    &vello::RenderParams {
                        base_color: Color::TRANSPARENT,
                        width: W,
                        height: H,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                )
                .expect("render");
            let row = 512u32;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (row * H) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(H),
                    },
                },
                wgpu::Extent3d {
                    width: W,
                    height: H,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(encoder.finish()));
            buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let data = buffer
                .slice(..)
                .get_mapped_range()
                .expect("the buffer is mapped");
            (0..H)
                .flat_map(|y| {
                    let r = &data[(y * row) as usize..];
                    (0..W as usize)
                        .map(|x| [r[x * 4], r[x * 4 + 1], r[x * 4 + 2], r[x * 4 + 3]])
                        .collect::<Vec<_>>()
                })
                .collect()
        }
    }
}
