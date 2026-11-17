//! What the reports say: per scene and size, each buffer's need against the
//! first size its rule gives, and the per-unit counts the rules are written in.

use rinch::vello::BumpReport;
use rinch::vello::low_level::vello_encoding::BumpSizes;

/// How to read one buffer's length out of a [`BumpSizes`].
type Length = fn(&BumpSizes) -> u32;

/// The buffers, in the order the table prints them.
pub const BUFFERS: &[(&str, Length)] = &[
    ("tiles", |s| s.tiles),
    ("ptcl", |s| s.ptcl),
    ("lines", |s| s.lines),
    ("segments", |s| s.segments),
    ("seg_counts", |s| s.seg_counts),
    ("binning", |s| s.binning),
    ("blend", |s| s.blend),
];

/// The worst of a set of frames of one scene at one size.
#[derive(Debug, Default)]
pub struct Summary {
    pub frames: usize,
    /// Frames that ran out of a buffer, and were drawn again.
    pub lost: usize,
    /// Per buffer, the smallest first size over need: the margin the rule
    /// left, `None` where no frame needed any.
    pub margin: Vec<Option<f64>>,
    /// The most lines and segments per path tag, tiles and ptcl words per
    /// target tile: the units the rules are written in.
    pub lines_per_tag: f64,
    pub segments_per_tag: f64,
    pub tiles_per_target_tile: f64,
    pub ptcl_per_target_tile: f64,
    /// The deepest nesting of clips and layers.
    pub nesting: u32,
}

impl Summary {
    pub fn of(reports: &[BumpReport]) -> Self {
        let mut s = Summary {
            margin: vec![None; BUFFERS.len()],
            ..Summary::default()
        };
        for r in reports {
            s.frames += 1;
            s.lost += usize::from(r.counters.failed != 0);
            let g = &r.config;
            let initial = r.initial();
            let needed = r.needed();
            for (i, (_, of)) in BUFFERS.iter().enumerate() {
                let need = of(&needed);
                if need == 0 {
                    continue;
                }
                let m = f64::from(of(&initial)) / f64::from(need);
                s.margin[i] = Some(s.margin[i].map_or(m, |old: f64| old.min(m)));
            }
            let tags = f64::from(g.layout.path_tags_size().max(1));
            let tiles = f64::from((g.width_in_tiles * g.height_in_tiles).max(1));
            s.lines_per_tag = s.lines_per_tag.max(f64::from(needed.lines) / tags);
            s.segments_per_tag = s
                .segments_per_tag
                .max(f64::from(needed.segments.max(needed.seg_counts)) / tags);
            s.tiles_per_target_tile = s.tiles_per_target_tile.max(f64::from(needed.tiles) / tiles);
            s.ptcl_per_target_tile = s.ptcl_per_target_tile.max(f64::from(needed.ptcl) / tiles);
            s.nesting = s.nesting.max(r.max_open_clips);
        }
        s
    }
}

/// The table's header, in Markdown.
pub fn header() -> String {
    let names: Vec<&str> = BUFFERS.iter().map(|(n, _)| *n).collect();
    format!(
        "| scene | size | frames | lost | {} | lines/tag | segments/tag | tiles/T | ptcl/T | nesting |\n|{}",
        names.join(" | "),
        "---|".repeat(9 + names.len())
    )
}

/// One row of the table.
pub fn row(scene: &str, width: u32, height: u32, s: &Summary) -> String {
    let margins: Vec<String> = s
        .margin
        .iter()
        .map(|m| m.map_or("-".into(), |m| format!("{m:.2}")))
        .collect();
    format!(
        "| {scene} | {width}x{height} | {} | {} | {} | {:.2} | {:.2} | {:.2} | {:.1} | {} |",
        s.frames,
        s.lost,
        margins.join(" | "),
        s.lines_per_tag,
        s.segments_per_tag,
        s.tiles_per_target_tile,
        s.ptcl_per_target_tile,
        s.nesting
    )
}
