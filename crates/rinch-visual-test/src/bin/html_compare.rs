//! `html-compare`: HTML files drawn three ways, side by side (see
//! `rinch_visual_test::html_compare`).
//!
//! ```text
//! html-compare <out> <file.html>... [--remote]
//! ```
//!
//! Each file is cleaned with `rinch_html::clean` (the default policy; with
//! `--remote`, pictures from the web too), screenshotted in Chrome as written
//! and as cleaned, drawn in a probe window by `HtmlView`, and put side by
//! side in `<out>/report.html`, worst first.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use rinch_html::{Policy, clean};
use rinch_visual_test::html_compare::{Result, Setup, capture, case, chrome, probe, report};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--probe") => {
            web_pictures();
            probe::run(
                args[1..].iter().map(PathBuf::from).collect(),
                Setup::default(),
            );
            Ok(())
        }
        Some(_) => run(&args),
        None => Err("usage: html-compare <out> <file.html>... [--remote]".into()),
    };
    if let Err(e) = result {
        eprintln!("html-compare: {e}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<()> {
    let remote = args.iter().any(|a| a == "--remote");
    let mut paths = args.iter().filter(|a| !a.starts_with("--"));
    let out = PathBuf::from(paths.next().ok_or("no output folder")?);
    let setup = Setup::default();
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
    let mut dirs = Vec::new();
    for file in paths {
        let file = PathBuf::from(file);
        let name = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "case".into());
        let original = std::fs::read_to_string(&file)?;
        let dir = out.join(&name);
        let meta = case::Meta {
            title: name,
            detail: file.display().to_string(),
        };
        case::write(&dir, &meta, &original, &clean(&original, &policy), &setup)?;
        dirs.push(dir);
    }
    let profile = out.join(".chrome-profile");
    for dir in &dirs {
        chrome::case(dir, setup.width, &profile)?;
    }
    let mut probe = Command::new(std::env::current_exe()?);
    probe.arg("--probe").args(&dirs);
    capture::capture(&dirs, probe, setup.width)?;
    println!("{}", report::report(&out, &dirs)?.display());
    Ok(())
}

/// Pictures from the web for the probe, without cookies, with a time limit.
fn web_pictures() {
    struct Web(ureq::Agent);
    impl rinch::image::ImageLoader for Web {
        fn load(&self, src: &str) -> rinch::image::ImageLoadResult {
            use rinch::image::ImageLoadResult::{Failed, Loaded};
            match self.0.get(src).call() {
                Ok(mut response) => match response
                    .body_mut()
                    .with_config()
                    .limit(15 << 20)
                    .read_to_vec()
                {
                    Ok(bytes) => Loaded(bytes),
                    Err(e) => Failed(e.to_string()),
                },
                Err(e) => Failed(e.to_string()),
            }
        }
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .build()
        .into();
    let web = Arc::new(Web(agent));
    rinch::image::register_image_scheme_arc("https", web.clone());
    rinch::image::register_image_scheme_arc("http", web);
}
