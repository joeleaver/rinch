//! The OS's text settings on Linux are the rendering properties of the font
//! fontconfig matches `sans-serif` to, read on the shared font scan's
//! configuration (#2773), with the configuration's `target="font"` rules
//! applied to the matched font.
//!
//! fontconfig runs on a configuration of its own: one font (Inter), a
//! `target="pattern"` rule, a `target="font"` rule that tests the matched
//! font's family (which only a match against the scanned fonts applies), and
//! one for a family that is not installed (which must not apply). The values
//! are all different from fontconfig's defaults and from each other, so a
//! property read from the wrong place shows. Its own test binary so
//! `FONTCONFIG_FILE` is set before fontconfig first loads.

#![cfg(all(feature = "windowing", target_os = "linux"))]

use rinch::text_rasterization::{LcdFilter, SubpixelOrder, SystemTextSettings, TextHinting};

const FONT: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

#[test]
fn the_settings_are_the_matched_font_s_on_the_shared_scan() {
    let dir = std::env::temp_dir().join(format!("rinch-fc-2773-{}", std::process::id()));
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::write(fonts.join("Inter-Regular.ttf"), FONT).unwrap();
    let conf = dir.join("fonts.conf");
    std::fs::write(
        &conf,
        format!(
            "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n\
             <fontconfig><dir>{}</dir><cachedir>{}</cachedir>\
             <match target=\"pattern\">\
               <edit name=\"hintstyle\" mode=\"assign\"><const>hintmedium</const></edit>\
               <edit name=\"lcdfilter\" mode=\"assign\"><const>lcdlegacy</const></edit>\
             </match>\
             <match target=\"font\">\
               <test name=\"family\"><string>Inter</string></test>\
               <edit name=\"rgba\" mode=\"assign\"><const>vbgr</const></edit>\
               <edit name=\"antialias\" mode=\"assign\"><bool>false</bool></edit>\
             </match>\
             <match target=\"font\">\
               <test name=\"family\"><string>Not Installed</string></test>\
               <edit name=\"hinting\" mode=\"assign\"><bool>false</bool></edit>\
             </match>\
             </fontconfig>\n",
            fonts.display(),
            dir.join("cache").display()
        ),
    )
    .unwrap();
    // SAFETY: the only test in this binary, before any thread or fontconfig
    // call reads the environment.
    unsafe { std::env::set_var("FONTCONFIG_FILE", &conf) };

    let settings = rinch::text_rasterization::system_text_settings();
    // Read on the one shared scan, which the read made (nothing else had):
    // before #2773 it loaded a configuration of its own and scanned nothing.
    assert_eq!(rinch_dom::fonts::system_font_scans(), 1);
    if !rinch_dom::fonts::system_font_paths()
        .config_files
        .contains(&conf)
    {
        // No libfontconfig on this host: nothing is read.
        eprintln!("libfontconfig not found; skipped");
        assert_eq!(settings, SystemTextSettings::default());
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert_eq!(
        settings,
        SystemTextSettings::default()
            .with_antialias(Some(false))
            .with_hinting(Some(TextHinting::Medium))
            .with_subpixel_order(Some(SubpixelOrder::VBgr))
            .with_lcd_filter(Some(LcdFilter::Legacy))
    );
    // And no second scan since.
    assert_eq!(rinch_dom::fonts::system_font_scans(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}
