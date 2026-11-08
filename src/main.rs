#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[cfg(windows)]
mod chrome;
#[cfg(windows)]
mod explorer;
mod loader;
mod spring;
mod theme;
#[cfg(windows)]
mod update;
#[cfg(windows)]
mod wic;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

fn main() -> eframe::Result {
    let start: Option<PathBuf> = std::env::args_os().nth(1).map(PathBuf::from);

    // Start decoding the first image and listing its folder right away,
    // while the window and GPU spin up.
    let loader = loader::Loader::new();
    if let Some(p) = &start {
        loader.want_full(vec![p.clone()]);
        if loader::is_supported(p) {
            loader.scan(p.clone());
        }
    }

    let icon = {
        let img = image::load_from_memory(include_bytes!("../assets/icon-256.png"))
            .expect("embedded icon")
            .into_rgba8();
        egui::IconData {
            width: img.width(),
            height: img.height(),
            rgba: img.into_raw(),
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Afterimage")
            .with_app_id("afterimage")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([480.0, 360.0])
            .with_drag_and_drop(true)
            .with_icon(Arc::new(icon)),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "Afterimage",
        options,
        Box::new(move |cc| {
            setup_style(&cc.egui_ctx);
            Ok(Box::new(app::Viewer::new(cc, loader, start)))
        }),
    )
}

fn setup_style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.set_fonts(fonts());
}

/// Segoe UI for Latin text and Malgun Gothic for Hangul, read from the
/// system at startup so nothing has to be bundled. Falls back to egui's
/// built-in fonts when they are missing.
fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();

    let dir = std::env::var_os("WINDIR")
        .map(|w| PathBuf::from(w).join("Fonts"))
        .unwrap_or_else(|| PathBuf::from("C:\\Windows\\Fonts"));
    let mut load = |key: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                defs.font_data
                    .insert(key.to_owned(), Arc::new(FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let segoe = load("segoe", "segoeui.ttf");
    let segoe_sb = load("segoe-sb", "seguisb.ttf");
    let malgun = load("malgun", "malgun.ttf");
    let malgun_b = load("malgun-b", "malgunbd.ttf");

    let prop = defs.families.entry(FontFamily::Proportional).or_default();
    if malgun {
        prop.insert(0, "malgun".into());
    }
    if segoe {
        prop.insert(0, "segoe".into());
    }

    let mut strong: Vec<String> = Vec::new();
    if segoe_sb {
        strong.push("segoe-sb".into());
    }
    if malgun_b {
        strong.push("malgun-b".into());
    }
    strong.extend(defs.families[&FontFamily::Proportional].iter().cloned());
    defs.families
        .insert(FontFamily::Name("strong".into()), strong);

    let mono = defs.families.entry(FontFamily::Monospace).or_default();
    if malgun {
        mono.push("malgun".into());
    }
    defs
}
