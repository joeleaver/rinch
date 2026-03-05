//! File Drop section -- showcases OS drag-and-drop with media preview.

use std::path::PathBuf;

use rinch::prelude::*;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use rinch_video::{VideoControls, VideoViewport, use_video_player_paused};

/// What kind of file was dropped.
#[derive(Clone, Debug, PartialEq)]
enum DroppedMedia {
    None,
    Image(String),
    Video(String),
    Other {
        name: String,
        size: String,
        ext: String,
    },
}

fn classify_path(path: &std::path::Path) -> DroppedMedia {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let path_str = path.to_string_lossy().to_string();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tiff" | "tga" => {
            DroppedMedia::Image(path_str)
        }
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "flv" | "wmv" | "m4v" | "ogg" | "ogv" => {
            DroppedMedia::Video(path_str)
        }
        _ => {
            let size = std::fs::metadata(path)
                .map(|m| format_size(m.len()))
                .unwrap_or_else(|_| "unknown".into());
            DroppedMedia::Other { name, size, ext }
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[component]
pub fn file_drop_section() -> NodeHandle {
    let hovering = Signal::new(false);
    let media = Signal::new(DroppedMedia::None);
    let all_paths = Signal::new(Vec::<PathBuf>::new());

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "File Drop" }
                Text { size: "lg", color: "dimmed",
                    "Drag files from your OS into the drop zone. Images display inline, videos play with full controls, and other files show metadata."
                }
            }
            Space { h: "xl" }

            // -- Drop zone --
            div {
                onfiledrop: move |paths: Vec<PathBuf>| {
                    hovering.set(false);
                    if let Some(first) = paths.first() {
                        media.set(classify_path(first));
                    }
                    all_paths.set(paths);
                },
                onfiledragenter: move || hovering.set(true),
                onfiledragleave: move || hovering.set(false),
                style: {|| {
                    let base = "min-height: 300px; border-radius: var(--rinch-radius-md); \
                                display: flex; flex-direction: column; align-items: center; \
                                justify-content: center; gap: 12px; padding: 32px; \
                                transition: border-color 150ms, background 150ms;";
                    if hovering.get() {
                        format!("{base} border: 2px dashed var(--rinch-color-blue-5); \
                                 background: var(--rinch-color-blue-0);")
                    } else {
                        format!("{base} border: 2px dashed var(--rinch-color-gray-4); \
                                 background: var(--rinch-color-gray-0);")
                    }
                }},

                // Content: either prompt or preview
                match media.get() {
                    DroppedMedia::None => {
                        drop_prompt(__scope, hovering)
                    },
                    DroppedMedia::Image(ref _path) => {
                        image_preview(__scope, _path.clone(), media)
                    },
                    DroppedMedia::Video(ref _path) => {
                        video_preview(__scope, _path.clone(), media)
                    },
                    DroppedMedia::Other { name: ref _name, size: ref _size, ext: ref _ext } => {
                        file_info(__scope, _name.clone(), _size.clone(), _ext.clone(), media)
                    },
                }
            }

            Space { h: "lg" }

            // -- All dropped paths list --
            if !all_paths.get().is_empty() {
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "xs",
                        Title { order: 3, "Dropped Paths" }
                        for path in all_paths.get() {
                            Text { size: "sm", style: "font-family: monospace; word-break: break-all;",
                                {path.to_string_lossy().to_string()}
                            }
                        }
                    }
                }
            }
        }
    }
}

// -- Sub-components --

#[component]
fn drop_prompt(hovering: Signal<bool>) -> NodeHandle {
    rsx! {
        Fragment {
            div { style: "opacity: 0.4;",
                {render_tabler_icon(__scope, TablerIcon::Upload, TablerIconStyle::Outline)}
            }
            Text { size: "xl", weight: "600", color: "dimmed",
                {|| if hovering.get() { "Release to drop" } else { "Drag files here" }}
            }
            Text { size: "sm", color: "dimmed",
                "Images, videos, or any file from your OS"
            }
        }
    }
}

#[component]
fn image_preview(path: String, media: Signal<DroppedMedia>) -> NodeHandle {
    let name = file_name_from_path(&path);
    let src = path;
    rsx! {
        Stack { gap: "md", align: "center", style: "width: 100%;",
            img {
                src: {src.clone()},
                style: "max-width: 100%; max-height: 500px; border-radius: var(--rinch-radius-sm); object-fit: contain;",
            }
            Group { gap: "sm", align: "center",
                Badge { color: "green", variant: "light", "Image" }
                Text { size: "sm", color: "dimmed", style: "font-family: monospace;",
                    {name}
                }
                Button { variant: "subtle", size: "xs", onclick: move || media.set(DroppedMedia::None),
                    "Clear"
                }
            }
        }
    }
}

#[component]
fn video_preview(path: String, media: Signal<DroppedMedia>) -> NodeHandle {
    let player = use_video_player_paused(&path);
    // Auto-play once the video has loaded (duration becomes known).
    // Wrap play() in untracked() to prevent re-entrant effect execution:
    // play() internally reads state.get() and sets signals, which would
    // re-trigger this effect while its closure RefCell is still borrowed.
    let p = player.clone();
    Effect::new(move || {
        if p.duration.get() > 0.0 {
            untracked(|| p.play());
        }
    });

    rsx! {
        Stack { gap: "md", style: "width: 100%;",
            Paper { p: "0", radius: "md", with_border: true,
                style: "overflow: hidden; width: 100%;",
                div { style: "display: flex; flex-direction: column;",
                    div { style: "width: 100%; height: 400px; background: black;",
                        VideoViewport { player: player.clone() }
                    }
                    VideoControls { player: player.clone() }
                }
            }
            Group { gap: "sm", align: "center", justify: "center",
                Badge { color: "violet", variant: "light", "Video" }
                Text { size: "sm", color: "dimmed", style: "font-family: monospace;",
                    {file_name_from_path(&path)}
                }
                Button { variant: "subtle", size: "xs", onclick: move || media.set(DroppedMedia::None),
                    "Clear"
                }
            }
        }
    }
}

#[component]
fn file_info(name: String, size: String, ext: String, media: Signal<DroppedMedia>) -> NodeHandle {
    let ext_badge = if ext.is_empty() {
        __scope.create_element("span") // empty placeholder
    } else {
        let badge_html = format!(".{ext}");
        rsx! { Badge { variant: "light", {badge_html} } }
    };

    rsx! {
        Stack { gap: "md", align: "center",
            div { style: "opacity: 0.4;",
                {render_tabler_icon(__scope, TablerIcon::File, TablerIconStyle::Outline)}
            }
            Text { size: "lg", weight: "600",
                {name}
            }
            Group { gap: "sm",
                {ext_badge}
                Badge { color: "gray", variant: "light",
                    {size}
                }
            }
            Button { variant: "subtle", size: "xs", onclick: move || media.set(DroppedMedia::None),
                "Clear"
            }
        }
    }
}

fn file_name_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}
