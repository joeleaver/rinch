//! `html-compare`: HTML files drawn three ways, side by side (see
//! `rinch_visual_test::html_compare`).
//!
//! ```text
//! html-compare <out> <file.html>... [--remote] [--widths 480,720,1080]
//! ```
//!
//! Each file is cleaned with `rinch_html::clean_at` at each width (the
//! default policy; with `--remote`, pictures from the web too, which the
//! probe loads with rinch's own loader, `image-network`),
//! screenshotted in Chrome as written and as cleaned, drawn in a probe window
//! by `HtmlView`, and put side by side in `<out>/report.html`, worst first
//! (with several widths, `<out>/w<width>/report.html` and an
//! `<out>/index.html` linking them).

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use rinch_html::{Policy, clean_at};
use rinch_visual_test::html_compare::{Result, Setup, capture, case, chrome, probe, report};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--probe") => {
            let width: u32 = args.get(1).and_then(|w| w.parse().ok()).unwrap_or(720);
            let setup = Setup {
                width,
                ..Setup::default()
            };
            probe::run(args[2..].iter().map(PathBuf::from).collect(), setup);
            Ok(())
        }
        Some(_) => run(&args),
        None => Err(USAGE.into()),
    };
    if let Err(e) = result {
        eprintln!("html-compare: {e}");
        std::process::exit(1);
    }
}

const USAGE: &str = "usage: html-compare <out> <file.html>... [--remote] [--widths 480,720,1080]";

fn run(args: &[String]) -> Result<()> {
    let remote = args.iter().any(|a| a == "--remote");
    let widths: Vec<u32> = match args.iter().position(|a| a == "--widths") {
        Some(at) => args
            .get(at + 1)
            .ok_or(USAGE)?
            .split(',')
            .map(|w| w.trim().parse())
            .collect::<std::result::Result<_, _>>()?,
        None => vec![720],
    };
    let mut paths = args
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            !a.starts_with("--") && args.get(i.wrapping_sub(1)).is_none_or(|p| p != "--widths")
        })
        .map(|(_, a)| a);
    let out = PathBuf::from(paths.next().ok_or(USAGE)?);
    let files: Vec<PathBuf> = paths.map(PathBuf::from).collect();
    let mut policy = Policy::default();
    if remote {
        policy.picture = Arc::new(|src| {
            let lower = src.trim().to_ascii_lowercase();
            (lower.starts_with("https://")
                || lower.starts_with("http://")
                || lower.starts_with("data:image/"))
            .then(|| src.trim().to_owned())
        });
    }
    for &width in &widths {
        let setup = Setup {
            width,
            ..Setup::default()
        };
        let out = if widths.len() == 1 {
            out.clone()
        } else {
            out.join(format!("w{width}"))
        };
        let mut dirs = Vec::new();
        for file in &files {
            let name = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "case".into());
            let original = std::fs::read_to_string(file)?;
            let dir = out.join(&name);
            let meta = case::Meta {
                title: name,
                detail: format!("{} · {width}px", file.display()),
            };
            let cleaned = clean_at(&original, &policy, width as f32);
            case::write(&dir, &meta, &original, &cleaned, &setup)?;
            dirs.push(dir);
        }
        let profile = out.join(".chrome-profile");
        for dir in &dirs {
            chrome::case(dir, width, &profile, remote)?;
        }
        let mut probe = Command::new(std::env::current_exe()?);
        probe.arg("--probe").arg(width.to_string()).args(&dirs);
        capture::capture(&dirs, probe, width)?;
        println!("{}", report::report(&out, &dirs)?.display());
    }
    if widths.len() > 1 {
        println!("{}", report::index(&out, &widths)?.display());
    }
    Ok(())
}
