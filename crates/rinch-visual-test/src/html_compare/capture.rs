//! rinch's drawing of each case: start the probe, drive it over rinch's
//! debug server (Next, wait for pictures, screenshot a window's height at a
//! time), and stitch each document into `c-rinch.png`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use crate::capture::RinchCapture;
use image::{GenericImageView, RgbaImage};

use super::Result;
use super::probe::BAR;

/// How long a document is given at most for its pictures from the web.
const SETTLE: Duration = Duration::from_secs(40);
/// How long the card's height must hold still to count as settled.
const STILL: Duration = Duration::from_secs(3);

/// Draw every case in `dirs` with rinch and save `c-rinch.png` in each.
/// `probe` starts a process that runs [`probe::run`](super::probe::run) over
/// the same `dirs` (in order) with the same `width`; usually the caller's
/// own executable with an argument that says so.
pub fn capture(dirs: &[PathBuf], mut probe: Command, width: u32) -> Result<()> {
    let mut child = probe.spawn()?;
    let result = drive(&child, dirs, width);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(discovery(child.id()));
    result
}

fn drive(child: &Child, dirs: &[PathBuf], width: u32) -> Result<()> {
    let (port, token) = wait_for_port(child.id())?;
    let mut probe = RinchCapture::connect_with_token(port, token)?;
    probe.wait_frame()?;
    for dir in dirs {
        let next = probe
            .query_selector(".probe-next")?
            .into_iter()
            .next()
            .ok_or("the probe has no Next button")?;
        probe.click(next.x + next.width / 2.0, next.y + next.height / 2.0)?;
        settle(&mut probe)?;
        let (image, height) = stitch(&mut probe, width)?;
        image.save(dir.join("c-rinch.png"))?;
        let last_ink = super::chrome::last_ink_row(&image);
        std::fs::write(
            dir.join("rinch.json"),
            serde_json::to_vec(&serde_json::json!({
                // The card's height: rinch's document height.
                "height": height,
                // The last row that differs from the background, in
                // `c-rinch.png`: compare with `chrome.json`'s `last_ink`.
                "last_ink": last_ink,
                "cut_off": height > f64::from(super::chrome::TALLEST),
            }))?,
        )?;
        eprintln!("rinch: {}", dir.display());
    }
    Ok(())
}

/// Wait until the card stops growing as pictures arrive (or SETTLE).
fn settle(probe: &mut RinchCapture) -> Result<()> {
    let start = Instant::now();
    let (mut last, mut since) = (-1.0, Instant::now());
    while start.elapsed() < SETTLE {
        probe.wait_frame()?;
        let height = card(probe)?.height;
        if height != last {
            (last, since) = (height, Instant::now());
        } else if since.elapsed() >= STILL {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

/// The card drawn whole: screenshot a window's height of it, press Down,
/// and again until all of it is in, at the logical size. Returns the image
/// and the card's height.
fn stitch(probe: &mut RinchCapture, width: u32) -> Result<(RgbaImage, f64)> {
    let first = card(probe)?;
    let height = first.height.min(f64::from(super::chrome::TALLEST));
    let mut whole = RgbaImage::new(width, height.ceil() as u32);
    for _ in 0..64 {
        let at = card(probe)?;
        let shot = image::load_from_memory(&probe.screenshot()?)?;
        // The screenshot is in physical pixels; the boxes are logical. The
        // window's logical width is the probe root's (the window manager may
        // have made the window wider than asked: a narrow one usually is).
        let window = probe
            .query_selector(".probe")?
            .into_iter()
            .next()
            .map_or(f64::from(width + BAR), |p| p.width);
        let scale = f64::from(shot.width()) / window;
        let shot = if scale == 1.0 {
            shot.to_rgba8()
        } else {
            image::imageops::resize(
                &shot,
                (f64::from(shot.width()) / scale) as u32,
                (f64::from(shot.height()) / scale) as u32,
                image::imageops::FilterType::Triangle,
            )
        };
        // The card's rows this screenshot shows: from the window's top (or
        // the card's) to the window's bottom (or the card's).
        let top = (-at.y).max(0.0);
        let bottom = (shot.height() as f64 - at.y).min(height);
        if bottom > top {
            let from_y = (at.y + top).max(0.0) as u32;
            let rows = (bottom - top) as u32;
            let piece = shot.view(at.x.max(0.0) as u32, from_y, width, rows);
            image::imageops::replace(&mut whole, &piece.to_image(), 0, top as i64);
        }
        if bottom >= height {
            return Ok((whole, first.height));
        }
        let down = probe
            .query_selector(".probe-down")?
            .into_iter()
            .next()
            .ok_or("the probe has no Down button")?;
        // Clicks on one spot in quick succession count as a double click.
        std::thread::sleep(Duration::from_millis(600));
        probe.click(down.x + down.width / 2.0, down.y + down.height / 2.0)?;
        probe.wait_frame()?;
        probe.wait_frame()?;
    }
    Err("the probe's card did not move down".into())
}

fn card(probe: &mut RinchCapture) -> Result<crate::runner::NodeMatch> {
    Ok(probe
        .query_selector(".html-compare-host")?
        .into_iter()
        .next()
        .ok_or("the probe drew no card")?)
}

/// Where rinch's debug server says where a process listens.
fn discovery(pid: u32) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".rinch/debug").join(format!("{pid}.json"))
}

/// The probe's port and session token (#2481), once its discovery file
/// exists.
fn wait_for_port(pid: u32) -> Result<(u16, Option<String>)> {
    let path = discovery(pid);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        if let Some(found) = read_port(&path) {
            return Ok(found);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(format!(
        "the probe never opened its debug server ({})",
        path.display()
    )
    .into())
}

fn read_port(path: &Path) -> Option<(u16, Option<String>)> {
    let bytes = std::fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let port = value["port"].as_u64()? as u16;
    Some((port, value["token"].as_str().map(str::to_string)))
}
