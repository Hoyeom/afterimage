use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CursorIcon, Event, Key, MouseWheelSource, MouseWheelUnit,
    Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextureFilter, TextureHandle,
    TextureOptions, Ui, Vec2, ViewportCommand,
};

use crate::i18n::s;
use crate::loader::{self, Done, Loader, Meta};
use crate::spring::{self, Anim};
use crate::theme::*;

const TOP_H: f32 = 48.0;
const THUMB_W: f32 = 64.0;
const THUMB_H: f32 = 44.0;
const THUMB_GAP: f32 = 6.0;
const STRIP_PAD: f32 = 8.0;
const STRIP_BOTTOM: f32 = 20.0;
const PANEL_W: f32 = 300.0;

struct Loaded {
    /// One entry for stills; every frame with its delay (ms) for animations.
    frames: Vec<(TextureHandle, u32)>,
    meta: Meta,
}

pub struct Viewer {
    files: Vec<PathBuf>,
    idx: usize,
    /// The file whose folder listing is still on its way (`Done::Folder`).
    scanning: Option<PathBuf>,
    loader: Loader,
    full: HashMap<PathBuf, Loaded>,
    errors: HashMap<PathBuf, String>,
    thumbs: HashMap<PathBuf, TextureHandle>,
    thumb_failed: HashSet<PathBuf>,
    icon: TextureHandle,

    // View state. `scale` is screen points per image pixel.
    fit: bool,
    fitted_for: Option<(PathBuf, bool)>,
    scale: Anim,
    pan_x: Anim,
    pan_y: Anim,
    slide: Anim,
    fade: Anim,
    swiping: bool,

    info_open: bool,
    info: Anim,
    sel: HashMap<usize, Anim>,
    strip: Anim,
    nav_alpha: [f32; 2],
    fullscreen: bool,
    /// The top bar is the window's title bar (see `chrome.rs`).
    custom_chrome: bool,
    /// For drawing magnified images with our own shader (see `sharp.rs`).
    render_state: Option<eframe::egui_wgpu::RenderState>,

    panel_rect: Option<Rect>,
    strip_rect: Option<Rect>,
    last_title: String,

    // Animation playback (GIF / WebP / APNG).
    anim_ms: f64,
    anim_paused: bool,
    last_time: f64,
}

impl Viewer {
    pub fn new(cc: &eframe::CreationContext<'_>, loader: Loader, start: Option<PathBuf>) -> Self {
        loader.set_context(&cc.egui_ctx);
        #[cfg(windows)]
        crate::update::start(cc.egui_ctx.clone());
        let icon = {
            let img = image::load_from_memory(include_bytes!("../assets/icon-64.png"))
                .expect("embedded icon")
                .into_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            cc.egui_ctx.load_texture(
                "app-icon",
                egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
                tex_options(),
            )
        };
        let mut v = Self {
            files: Vec::new(),
            idx: 0,
            scanning: None,
            loader,
            full: HashMap::new(),
            errors: HashMap::new(),
            thumbs: HashMap::new(),
            thumb_failed: HashSet::new(),
            icon,
            fit: true,
            fitted_for: None,
            scale: Anim::new(1.0),
            pan_x: Anim::new(0.0),
            pan_y: Anim::new(0.0),
            slide: Anim::new(0.0),
            fade: Anim::new(1.0),
            swiping: false,
            info_open: false,
            info: Anim::new(0.0),
            sel: HashMap::new(),
            strip: Anim::new(0.0),
            nav_alpha: [0.0; 2],
            fullscreen: false,
            #[cfg(windows)]
            custom_chrome: crate::chrome::install(cc),
            #[cfg(not(windows))]
            custom_chrome: false,
            render_state: cc.wgpu_render_state.clone(),
            panel_rect: None,
            strip_rect: None,
            last_title: String::new(),
            anim_ms: 0.0,
            anim_paused: false,
            last_time: 0.0,
        };
        // `main` already asked the loader to decode and scan this one.
        if let Some(p) = start {
            if loader::is_supported(&p) {
                v.show_single(&p);
            }
        }
        v
    }

    fn current(&self) -> Option<&PathBuf> {
        self.files.get(self.idx)
    }

    // ---------------------------------------------------------------- loading

    fn open_path(&mut self, path: &Path) {
        if !loader::is_supported(path) {
            return;
        }
        self.show_single(path);
        self.loader.scan(path.to_path_buf());
    }

    /// Show `path` on its own right away; its neighbours join when the
    /// folder listing arrives.
    fn show_single(&mut self, path: &Path) {
        self.files = vec![path.to_path_buf()];
        self.idx = 0;
        self.scanning = Some(path.to_path_buf());
        self.full.clear();
        self.errors.clear();
        self.thumbs.clear();
        self.thumb_failed.clear();
        self.sel.clear();
        self.fit = true;
        self.fitted_for = None;
        self.pan_x.snap(0.0);
        self.pan_y.snap(0.0);
        self.slide.snap(0.0);
        self.fade.snap(1.0);
        self.strip
            .snap(self.strip_target(self.visible_thumbs(1280.0)));
        self.schedule();
    }

    /// Ask the decoder for the images around the current one, nearest first,
    /// and forget the ones that drifted out of range.
    fn schedule(&mut self) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len() as isize;
        let i = self.idx as isize;
        // Next / previous wrap around, so the neighbours do too.
        let order = [0isize, 1, -1, 2, -2];
        let mut wanted: Vec<PathBuf> = Vec::new();
        for d in order {
            let p = &self.files[(i + d).rem_euclid(n) as usize];
            if !wanted.contains(p) && !self.full.contains_key(p) && !self.errors.contains_key(p) {
                wanted.push(p.clone());
            }
        }
        self.loader.want_full(wanted);

        let keep: HashSet<&PathBuf> = (i - 3..=i + 3)
            .map(|j| &self.files[j.rem_euclid(n) as usize])
            .collect();
        self.full.retain(|p, _| keep.contains(p));

        let mut thumbs: Vec<(isize, PathBuf)> = (i - 14..=i + 14)
            .filter(|j| (0..n).contains(j))
            .map(|j| ((j - i).abs(), self.files[j as usize].clone()))
            .filter(|(_, p)| !self.thumbs.contains_key(p) && !self.thumb_failed.contains(p))
            .collect();
        thumbs.sort_by_key(|(d, _)| *d);
        self.loader
            .want_thumbs(thumbs.into_iter().map(|(_, p)| p).collect());

        if self.thumbs.len() > 200 {
            let keep: HashSet<&PathBuf> = (i - 60..=i + 60)
                .filter(|j| (0..n).contains(j))
                .map(|j| &self.files[j as usize])
                .collect();
            self.thumbs.retain(|p, _| keep.contains(p));
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let mut got = false;
        while let Ok(done) = self.loader.rx.try_recv() {
            got = true;
            match done {
                Done::Full { path, result } => {
                    if !self.files.contains(&path) {
                        continue;
                    }
                    match result {
                        Ok(d) => {
                            let frames: Vec<(TextureHandle, u32)> = d
                                .frames
                                .into_iter()
                                .enumerate()
                                .map(|(i, (img, ms))| {
                                    let name = format!("{}#{i}", path.display());
                                    (ctx.load_texture(name, img, tex_options()), ms)
                                })
                                .collect();
                            self.full.insert(
                                path,
                                Loaded {
                                    frames,
                                    meta: d.meta,
                                },
                            );
                        }
                        Err(e) => {
                            self.errors.insert(path, e);
                        }
                    }
                }
                Done::Folder { file, files, idx } => {
                    if self.scanning.as_ref() == Some(&file) {
                        self.scanning = None;
                        self.files = files;
                        self.idx = idx;
                        self.sel.clear();
                        self.strip
                            .snap(self.strip_target(self.visible_thumbs(1280.0)));
                    }
                }
                Done::Thumb { path, image } => match image {
                    Some(img) => {
                        let tex = ctx.load_texture(
                            format!("thumb:{}", path.display()),
                            img,
                            tex_options(),
                        );
                        self.thumbs.insert(path, tex);
                    }
                    None => {
                        self.thumb_failed.insert(path);
                    }
                },
            }
        }
        if got {
            self.schedule();
        }
    }

    // ------------------------------------------------------------- navigation

    fn go(&mut self, to: usize) {
        if self.files.is_empty() || to >= self.files.len() || to == self.idx {
            return;
        }
        self.idx = to;
        self.fit = true;
        self.fitted_for = None;
        self.pan_x.snap(0.0);
        self.pan_y.snap(0.0);
        // The new image fades up in place, so two images can be compared by
        // flipping between them.
        self.slide.snap(0.0);
        self.fade.x = self.fade.x.min(0.35);
        self.fade.to = 1.0;
        self.anim_ms = 0.0;
        self.anim_paused = false;
        self.schedule();
    }

    /// Next / previous, wrapping around at either end of the folder.
    fn step(&mut self, d: isize) {
        let n = self.files.len() as isize;
        if n < 2 {
            return;
        }
        let to = (self.idx as isize + d).rem_euclid(n);
        self.go(to as usize);
    }

    fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter(s().images, loader::EXTENSIONS)
            .pick_file()
        {
            self.open_path(&p);
        }
    }

    // ------------------------------------------------------------------ zoom

    fn image_size(&self) -> Option<Vec2> {
        let p = self.current()?;
        if let Some(l) = self.full.get(p) {
            return Some(vec2(l.meta.width as f32, l.meta.height as f32));
        }
        self.thumbs.get(p).map(|t| t.size_vec2())
    }

    fn fit_scale(&self, img: Vec2, area: Rect, ppp: f32) -> f32 {
        let s = (area.width() / img.x).min(area.height() / img.y);
        // Never blow small images past 100%.
        s.min(1.0 / ppp).max(1e-4)
    }

    fn clamp_pan(&mut self, img: Vec2, area: Rect) {
        let disp = img * self.scale.to;
        for (anim, d, a) in [
            (&mut self.pan_x, disp.x, area.width()),
            (&mut self.pan_y, disp.y, area.height()),
        ] {
            let max = ((d - a) / 2.0).max(0.0);
            anim.to = anim.to.clamp(-max, max);
        }
    }

    fn zoom_at(&mut self, p: Pos2, factor: f32, area: Rect, ppp: f32) {
        let Some(img) = self.image_size() else { return };
        let fit = self.fit_scale(img, area, ppp);
        let s0 = self.scale.to;
        let s1 = (s0 * factor).clamp(fit.min(1.0 / ppp) * 0.5, 32.0 / ppp);
        if s1 <= fit * 1.0001 && factor < 1.0 {
            self.fit = true;
            self.scale.to = fit;
            self.pan_x.to = 0.0;
            self.pan_y.to = 0.0;
            return;
        }
        let c = area.center() + vec2(self.pan_x.to, self.pan_y.to);
        let nc = p - (p - c) * (s1 / s0);
        self.pan_x.to = nc.x - area.center().x;
        self.pan_y.to = nc.y - area.center().y;
        self.scale.to = s1;
        self.fit = false;
        self.clamp_pan(img, area);
    }

    fn set_actual_size(&mut self, at: Pos2, area: Rect, ppp: f32) {
        let s0 = self.scale.to;
        self.zoom_at(at, (1.0 / ppp) / s0, area, ppp);
        self.fit = false;
    }

    fn reset_fit(&mut self) {
        self.fit = true;
        self.pan_x.to = 0.0;
        self.pan_y.to = 0.0;
    }

    // ------------------------------------------------------------- the frame

    fn image_area(&self, view: Rect) -> Rect {
        let mut a = view.shrink2(vec2(16.0, 16.0));
        if self.files.len() > 1 {
            a.max.y -= THUMB_H + STRIP_PAD * 2.0 + STRIP_BOTTOM - 8.0;
        }
        a
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.fullscreen));
    }

    fn handle_keys(&mut self, ctx: &egui::Context, area: Rect, ppp: f32) {
        let (keys, mods) = ctx.input(|i| {
            let keys: Vec<Key> = [
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::PageUp,
                Key::PageDown,
                Key::Space,
                Key::Backspace,
                Key::Home,
                Key::End,
                Key::Plus,
                Key::Equals,
                Key::Minus,
                Key::Num0,
                Key::Num1,
                Key::I,
                Key::F,
                Key::F11,
                Key::Escape,
                Key::O,
                Key::P,
            ]
            .into_iter()
            .filter(|k| i.key_pressed(*k))
            .collect();
            (keys, i.modifiers)
        });
        for k in keys {
            match k {
                Key::ArrowRight | Key::PageDown | Key::Space => self.step(1),
                Key::ArrowLeft | Key::PageUp | Key::Backspace => self.step(-1),
                Key::Home => self.go(0),
                Key::End => self.go(self.files.len().saturating_sub(1)),
                Key::Plus | Key::Equals => self.zoom_at(area.center(), 1.25, area, ppp),
                Key::Minus => self.zoom_at(area.center(), 0.8, area, ppp),
                Key::Num0 => self.reset_fit(),
                Key::Num1 => self.set_actual_size(area.center(), area, ppp),
                Key::I => self.info_open = !self.info_open,
                Key::F | Key::F11 => self.toggle_fullscreen(ctx),
                Key::Escape => {
                    if self.fullscreen {
                        self.fullscreen = false;
                        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                    } else if self.info_open {
                        self.info_open = false;
                    } else {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                }
                Key::O if mods.command => self.open_dialog(),
                Key::P => self.anim_paused = !self.anim_paused,
                _ => {}
            }
        }
    }

    fn draw_canvas(&mut self, ui: &mut Ui, view: Rect, area: Rect, ppp: f32, dt: f32) {
        let ctx = ui.ctx().clone();
        let canvas = ui.interact(view, ui.scope_id().with("canvas"), Sense::click_and_drag());
        let painter = ui.painter().with_clip_rect(view);

        let Some(path) = self.current().cloned() else {
            self.draw_empty(ui, view);
            return;
        };

        let loaded = self.full.get(&path);
        let (tex, img, is_full) = match (loaded, self.thumbs.get(&path)) {
            (Some(l), _) => (
                l.frames[frame_at(&l.frames, self.anim_ms)].0.id(),
                vec2(l.meta.width as f32, l.meta.height as f32),
                true,
            ),
            (None, Some(t)) => (t.id(), t.size_vec2(), false),
            (None, None) => {
                if let Some(err) = self.errors.get(&path) {
                    painter.text(
                        view.center() - vec2(0.0, 12.0),
                        Align2::CENTER_CENTER,
                        s().cannot_open,
                        strong(15.0),
                        TEXT,
                    );
                    painter.text(
                        view.center() + vec2(0.0, 14.0),
                        Align2::CENTER_CENTER,
                        err,
                        body(12.0),
                        TEXT_2,
                    );
                } else {
                    painter.text(
                        view.center(),
                        Align2::CENTER_CENTER,
                        s().loading,
                        body(13.0),
                        TEXT_2,
                    );
                }
                return;
            }
        };

        // First time we know this image's size: start fitted, without animating.
        let fit_s = self.fit_scale(img, area, ppp);
        let key = (path.clone(), is_full);
        if self.fitted_for.as_ref() != Some(&key) {
            if self.fit || !is_full {
                self.scale.snap(fit_s);
            } else if let Some((p, false)) = &self.fitted_for {
                if *p == path {
                    // Thumbnail was standing in; swap to real pixels at the same on-screen size.
                    self.scale.snap(fit_s);
                }
            }
            self.fitted_for = Some(key);
        }
        if self.fit {
            self.scale.to = fit_s;
        }

        // ---- pointer input on the image
        let fits = img.x * self.scale.to <= area.width() + 1.0
            && img.y * self.scale.to <= area.height() + 1.0;
        let over_chrome = |p: Pos2, s: &Self| {
            s.panel_rect.is_some_and(|r| r.contains(p))
                || s.strip_rect.is_some_and(|r| r.contains(p))
        };
        let pointer = ctx.pointer_hover_pos();

        if canvas.dragged() {
            let d = canvas.drag_delta();
            let v = d / dt.max(1e-3);
            if fits && self.files.len() > 1 {
                // Image fits: a sideways drag swipes to the neighbour.
                self.swiping = true;
                self.slide.x += d.x;
                self.slide.to = self.slide.x;
                self.slide.v = v.x;
            } else if !fits {
                self.pan_x.x += d.x;
                self.pan_y.x += d.y;
                self.pan_x.to = self.pan_x.x;
                self.pan_y.to = self.pan_y.x;
                self.pan_x.v = v.x;
                self.pan_y.v = v.y;
            }
            ctx.set_cursor_icon(CursorIcon::Grabbing);
        } else if canvas.hovered() && !fits {
            ctx.set_cursor_icon(CursorIcon::Grab);
        }
        if canvas.drag_stopped() {
            if self.swiping {
                self.swiping = false;
                let (x, v) = (self.slide.x, self.slide.v);
                if x < -90.0 || v < -700.0 {
                    self.step(1);
                } else if x > 90.0 || v > 700.0 {
                    self.step(-1);
                } else {
                    self.slide.to = 0.0;
                }
            } else {
                // Let go mid-pan: carry the hand's velocity a little further.
                self.pan_x.to = self.pan_x.x + self.pan_x.v * 0.12;
                self.pan_y.to = self.pan_y.x + self.pan_y.v * 0.12;
                self.clamp_pan(img, area);
            }
        }
        if canvas.double_clicked() {
            if self.fit {
                let at = canvas.interact_pointer_pos().unwrap_or(area.center());
                self.set_actual_size(at, area, ppp);
            } else {
                self.reset_fit();
            }
        }

        if let Some(p) = pointer.filter(|p| view.contains(*p) && !over_chrome(*p, self)) {
            let events = ctx.input(|i| i.raw.events.clone());
            for e in events {
                if let Event::MouseWheel {
                    unit,
                    delta,
                    source,
                    modifiers,
                    ..
                } = e
                {
                    if modifiers.ctrl || modifiers.command {
                        continue; // handled by zoom_delta below
                    }
                    if matches!(
                        source,
                        MouseWheelSource::Trackpad | MouseWheelSource::Momentum
                    ) {
                        if !fits {
                            self.pan_x.to += delta.x;
                            self.pan_y.to += delta.y;
                            self.clamp_pan(img, area);
                        }
                        continue;
                    }
                    let lines = match unit {
                        MouseWheelUnit::Line => delta.y,
                        MouseWheelUnit::Point => delta.y / 40.0,
                        MouseWheelUnit::Page => delta.y * 8.0,
                    };
                    if lines != 0.0 {
                        self.zoom_at(p, 1.2f32.powf(lines), area, ppp);
                    }
                }
            }
            let zd = ctx.input(|i| i.zoom_delta());
            if (zd - 1.0).abs() > 1e-4 {
                self.zoom_at(p, zd, area, ppp);
            }
        }

        // ---- paint
        let size = img * self.scale.x;
        let center = area.center() + vec2(self.pan_x.x + self.slide.x, self.pan_y.x);
        let rect = Rect::from_center_size(center, size);
        let alpha = self.fade.x.clamp(0.0, 1.0);
        // A thumbnail standing in for the full image stays smooth.
        let sharp = is_full
            && self
                .render_state
                .as_ref()
                .is_some_and(|state| crate::sharp::paint(&painter, state, tex, rect, ppp, alpha));
        if !sharp {
            painter.image(
                tex,
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE.gamma_multiply(alpha),
            );
        }
    }

    fn draw_empty(&mut self, ui: &mut Ui, view: Rect) {
        let p = ui.painter();
        let c = view.center();
        p.image(
            self.icon.id(),
            Rect::from_center_size(c - vec2(0.0, 64.0), vec2(72.0, 72.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        p.text(
            c + vec2(0.0, 4.0),
            Align2::CENTER_CENTER,
            s().drop_hint,
            strong(16.0),
            TEXT,
        );
        p.text(
            c + vec2(0.0, 30.0),
            Align2::CENTER_CENTER,
            s().or,
            body(13.0),
            TEXT_2,
        );
        let btn = Rect::from_center_size(c + vec2(0.0, 70.0), vec2(176.0, 40.0));
        let r = ui
            .interact(btn, ui.scope_id().with("open-btn"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let p = ui.painter();
        let fill = if r.hovered() {
            Color32::from_rgb(0x93, 0xB4, 0xFF)
        } else {
            ACCENT
        };
        p.rect_filled(btn, 8.0, fill);
        p.text(
            btn.center(),
            Align2::CENTER_CENTER,
            s().open_file,
            strong(13.0),
            BG,
        );
        if r.clicked() {
            self.open_dialog();
        }
    }

    fn draw_nav(&mut self, ui: &mut Ui, view: Rect, area: Rect, dt: f32) -> Option<isize> {
        if self.files.len() < 2 {
            return None;
        }
        let pointer = ui.ctx().pointer_hover_pos();
        let mut go = None;
        for (side, dir) in [(0usize, -1isize), (1, 1)] {
            let zone = if dir < 0 {
                Rect::from_min_max(
                    view.min,
                    pos2(view.left() + view.width() * 0.3, view.bottom()),
                )
            } else {
                Rect::from_min_max(
                    pos2(view.right() - view.width() * 0.3, view.top()),
                    view.max,
                )
            };
            let want = if pointer.is_some_and(|p| zone.contains(p)) {
                1.0
            } else {
                0.0
            };
            let a = &mut self.nav_alpha[side];
            *a += (want - *a) * (1.0 - (-dt / 0.06).exp());
            if *a < 0.01 {
                continue;
            }
            let cx = if dir < 0 {
                view.left() + 38.0
            } else {
                view.right() - 38.0
            };
            let rect = Rect::from_center_size(pos2(cx, area.center().y), vec2(44.0, 44.0));
            let r = ui
                .interact(rect, ui.scope_id().with(("nav", side)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            let alpha = *a;
            let p = ui.painter();
            let fill = if r.hovered() {
                Color32::from_rgba_unmultiplied(36, 39, 45, 230)
            } else {
                SURFACE
            };
            p.circle_filled(rect.center(), 22.0, fill.gamma_multiply(alpha));
            p.circle_stroke(
                rect.center(),
                21.5,
                Stroke::new(1.0, BORDER.gamma_multiply(alpha)),
            );
            let c = rect.center();
            let s = if dir < 0 { -1.0 } else { 1.0 };
            p.add(Shape::line(
                vec![
                    c + vec2(-3.0 * s, -6.0),
                    c + vec2(3.0 * s, 0.0),
                    c + vec2(-3.0 * s, 6.0),
                ],
                Stroke::new(2.0, TEXT.gamma_multiply(alpha)),
            ));
            if r.clicked() {
                go = Some(dir);
            }
        }
        go
    }

    fn visible_thumbs(&self, view_w: f32) -> usize {
        let fit = ((view_w - 64.0 - STRIP_PAD * 2.0 + THUMB_GAP) / (THUMB_W + THUMB_GAP)).floor()
            as usize;
        fit.clamp(1, 15).min(self.files.len())
    }

    fn strip_target(&self, count: usize) -> f32 {
        let n = self.files.len();
        let first = self
            .idx
            .saturating_sub(count / 2)
            .min(n.saturating_sub(count));
        first as f32 * (THUMB_W + THUMB_GAP)
    }

    fn draw_strip(&mut self, ui: &mut Ui, view: Rect, dt: f32) -> (bool, Option<usize>) {
        self.strip_rect = None;
        if self.files.len() < 2 {
            return (false, None);
        }
        let count = self.visible_thumbs(view.width());
        self.strip.to = self.strip_target(count);
        let mut moving = self.strip.step(spring::SWIPE, dt, 0.1);

        let step = THUMB_W + THUMB_GAP;
        let inner_w = count as f32 * step - THUMB_GAP;
        let pill = Rect::from_center_size(
            pos2(
                view.center().x,
                view.bottom() - STRIP_BOTTOM - (THUMB_H + STRIP_PAD * 2.0) / 2.0,
            ),
            vec2(inner_w + STRIP_PAD * 2.0, THUMB_H + STRIP_PAD * 2.0),
        );
        self.strip_rect = Some(pill);
        ui.interact(
            pill,
            ui.scope_id().with("strip-bg"),
            Sense::click_and_drag(),
        );

        let p = ui.painter();
        p.rect(
            pill,
            14.0,
            SURFACE,
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );
        let clip = pill.shrink(STRIP_PAD - 3.0);
        let p = p.with_clip_rect(clip);

        let first = (self.strip.x / step).floor().max(0.0) as usize;
        let last = (first + count + 1).min(self.files.len());
        let mut clicked = None;
        for i in first..last {
            let x = pill.left() + STRIP_PAD + i as f32 * step - self.strip.x;
            let cell = Rect::from_min_size(pos2(x, pill.top() + STRIP_PAD), vec2(THUMB_W, THUMB_H));
            let on = i == self.idx;
            let a = self.sel.entry(i).or_insert(Anim::new(0.86));
            a.to = if on { 1.0 } else { 0.86 };
            moving |= a.step(spring::THUMB, dt, 0.0005);
            let s = a.x;
            let r = Rect::from_center_size(cell.center(), cell.size() * s);
            let alpha = (0.62 + (s - 0.86) / 0.14 * 0.38).clamp(0.4, 1.0);

            let hit = ui
                .interact(
                    cell.intersect(clip),
                    ui.scope_id().with(("thumb", i)),
                    Sense::click(),
                )
                .on_hover_cursor(CursorIcon::PointingHand);
            if hit.clicked() {
                clicked = Some(i);
            }
            let alpha = if hit.hovered() && !on { 0.85 } else { alpha };

            match self.thumbs.get(&self.files[i]) {
                Some(t) => {
                    let uv = cover_uv(t.size_vec2(), r.size());
                    p.add(
                        egui::epaint::RectShape::filled(
                            r,
                            7.0,
                            Color32::WHITE.gamma_multiply(alpha),
                        )
                        .with_texture(t.id(), uv),
                    );
                }
                None => {
                    p.rect_filled(r, 7.0, PLACEHOLDER.gamma_multiply(alpha));
                }
            }
            if on {
                p.rect_stroke(r, 7.0, Stroke::new(2.0, ACCENT), StrokeKind::Outside);
            }
        }
        self.sel
            .retain(|i, _| *i + 30 >= self.idx && *i <= self.idx + 30);
        (moving, clicked)
    }

    fn draw_panel(&mut self, ui: &mut Ui, view: Rect) {
        self.panel_rect = None;
        let t = self.info.x;
        if t < 0.002 {
            return;
        }
        let w = PANEL_W.min(view.width() - 24.0);
        let x = view.right() - 12.0 - w + (1.0 - t) * (w + 24.0);
        let rect = Rect::from_min_max(
            pos2(x, view.top() + 12.0),
            pos2(x + w, view.bottom() - 12.0),
        );
        self.panel_rect = Some(rect);
        ui.interact(
            rect,
            ui.scope_id().with("panel-bg"),
            Sense::click_and_drag(),
        );
        let alpha = t.clamp(0.0, 1.0);
        let p = ui.painter().with_clip_rect(view);
        p.rect(
            rect,
            14.0,
            PANEL.gamma_multiply(alpha),
            Stroke::new(1.0, BORDER.gamma_multiply(alpha)),
            StrokeKind::Inside,
        );

        let inner = rect.shrink(16.0);
        let mut y = inner.top();
        p.text(
            pos2(inner.left(), y + 16.0),
            Align2::LEFT_CENTER,
            s().info,
            strong(14.0),
            TEXT.gamma_multiply(alpha),
        );
        let close = Rect::from_center_size(pos2(inner.right() - 12.0, y + 16.0), vec2(32.0, 32.0));
        if icon_button(ui, &p, close, "panel-close", false, alpha, |p, c, col| {
            let s = Stroke::new(1.8, col);
            p.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], s);
            p.line_segment([c + vec2(5.0, -5.0), c + vec2(-5.0, 5.0)], s);
        })
        .clicked()
        {
            self.info_open = false;
        }
        y += 48.0;

        let Some(path) = self.current().cloned() else {
            return;
        };
        let Some(loaded) = self.full.get(&path) else {
            p.text(
                pos2(inner.left(), y),
                Align2::LEFT_TOP,
                s().loading,
                body(12.0),
                TEXT_2.gamma_multiply(alpha),
            );
            return;
        };
        let m = &loaded.meta;

        p.text(
            pos2(inner.left(), y),
            Align2::LEFT_TOP,
            s().histogram,
            body(11.0),
            TEXT_2.gamma_multiply(alpha),
        );
        y += 20.0;
        let hist = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 64.0));
        p.rect_filled(
            hist,
            6.0,
            Color32::from_white_alpha(8).gamma_multiply(alpha),
        );
        let bw = hist.width() / loader::HIST_BINS as f32;
        for (i, v) in m.histogram.iter().enumerate() {
            let h = (hist.height() - 8.0) * v;
            let bar = Rect::from_min_max(
                pos2(hist.left() + i as f32 * bw + 0.5, hist.bottom() - 4.0 - h),
                pos2(hist.left() + (i + 1) as f32 * bw - 0.5, hist.bottom() - 4.0),
            );
            p.rect_filled(bar, 0.0, ACCENT.gamma_multiply(0.7 * alpha));
        }
        y += 64.0 + 20.0;

        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut rows: Vec<(&str, String)> = vec![
            (s().file_name, name),
            (s().resolution, format!("{} × {}", m.width, m.height)),
            (s().file_size, human_size(m.file_size)),
            (s().format, m.format.to_string()),
        ];
        if let Some(c) = &m.camera {
            rows.push((s().camera, c.clone()));
        }
        if let Some(l) = &m.lens {
            rows.push((s().lens, l.clone()));
        }
        if let Some(e) = &m.exposure {
            rows.push((s().exposure, e.clone()));
        }
        if let Some(t) = &m.taken {
            rows.push((s().taken, t.clone()));
        }
        if let Some(t) = m.modified {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            rows.push((s().modified, dt.format("%Y-%m-%d %H:%M").to_string()));
        }
        if loaded.frames.len() > 1 {
            let state = if self.anim_paused { s().paused } else { "" };
            rows.push((
                s().animation,
                s().frames(loaded.frames.len(), m.duration_ms as f32 / 1000.0) + state,
            ));
        }
        rows.push((s().decoding, format!("{} ms", m.decode_ms)));

        let label_w = 80.0;
        let value_w = inner.width() - label_w;
        let buttons_top = inner.bottom() - 36.0;
        for (label, value) in rows {
            let galley = p.layout(value, body(12.0), TEXT.gamma_multiply(alpha), value_w);
            let h = galley.size().y.max(16.0);
            if y + h > buttons_top - 12.0 {
                break;
            }
            p.text(
                pos2(inner.left(), y),
                Align2::LEFT_TOP,
                label,
                body(12.0),
                TEXT_2.gamma_multiply(alpha),
            );
            p.galley(pos2(inner.left() + label_w, y), galley, TEXT);
            y += h + 10.0;
        }

        let half = (inner.width() - 8.0) / 2.0;
        let b1 = Rect::from_min_size(pos2(inner.left(), buttons_top), vec2(half, 36.0));
        let b2 = Rect::from_min_size(
            pos2(inner.left() + half + 8.0, buttons_top),
            vec2(half, 36.0),
        );
        let r1 = ui
            .interact(b1, ui.scope_id().with("reveal"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let r2 = ui
            .interact(b2, ui.scope_id().with("copy-path"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        p.rect(
            b1,
            8.0,
            if r1.hovered() {
                HOVER
            } else {
                Color32::TRANSPARENT
            },
            Stroke::new(1.0, Color32::from_white_alpha(26).gamma_multiply(alpha)),
            StrokeKind::Inside,
        );
        p.text(
            b1.center(),
            Align2::CENTER_CENTER,
            s().reveal,
            body(12.5),
            TEXT.gamma_multiply(alpha),
        );
        let fill = if r2.hovered() {
            Color32::from_rgb(0x93, 0xB4, 0xFF)
        } else {
            ACCENT
        };
        p.rect_filled(b2, 8.0, fill.gamma_multiply(alpha));
        p.text(
            b2.center(),
            Align2::CENTER_CENTER,
            s().copy_path,
            strong(12.5),
            BG,
        );
        if r1.clicked() {
            reveal_in_explorer(&path);
        }
        if r2.clicked() {
            ui.ctx().copy_text(path.display().to_string());
        }
    }

    fn draw_top_bar(&mut self, ui: &mut Ui, bar: Rect, area: Rect, ppp: f32) {
        ui.interact(bar, ui.scope_id().with("top-bg"), Sense::click());
        let p = ui.painter().clone();
        p.rect_filled(bar, 0.0, BG);
        p.line_segment(
            [bar.left_bottom(), bar.right_bottom()],
            Stroke::new(1.0, DIVIDER),
        );
        let cy = bar.center().y;

        // Window buttons first, at the very edge, when the bar is the title bar.
        let mut hot: Vec<Rect> = Vec::new();
        let mut x = bar.right();
        if self.custom_chrome && !self.fullscreen {
            x = self.draw_caption_buttons(ui, &p, bar, &mut hot) - 6.0;
        } else {
            x -= 10.0;
        }

        // Right cluster, laid out from the right edge inwards.
        let info_r = Rect::from_center_size(pos2(x - 18.0, cy), vec2(36.0, 36.0));
        x -= 36.0 + 4.0;
        let full_r = Rect::from_center_size(pos2(x - 18.0, cy), vec2(36.0, 36.0));
        x -= 36.0 + 8.0;
        let pill = Rect::from_min_max(pos2(x - 116.0, cy - 16.0), pos2(x, cy + 16.0));
        x -= 116.0 + 12.0;
        hot.extend([info_r, full_r, pill]);

        let has_image = self.current().is_some();
        if icon_button(
            ui,
            &p,
            info_r,
            "info-btn",
            self.info_open,
            1.0,
            |p, c, col| {
                p.circle_stroke(c, 8.0, Stroke::new(1.6, col));
                p.line_segment(
                    [c + vec2(0.0, -0.5), c + vec2(0.0, 4.5)],
                    Stroke::new(1.8, col),
                );
                p.circle_filled(c + vec2(0.0, -4.0), 1.2, col);
            },
        )
        .clicked()
        {
            self.info_open = !self.info_open;
        }
        let fullscreen = self.fullscreen;
        if icon_button(
            ui,
            &p,
            full_r,
            "fullscreen-btn",
            fullscreen,
            1.0,
            |p, c, col| {
                let s = Stroke::new(1.6, col);
                // Corners point outwards to enter and inwards to leave.
                let (reach, arm) = if fullscreen { (3.0, -4.0) } else { (7.0, 4.0) };
                for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let corner = c + vec2(reach * sx, reach * sy);
                    p.add(Shape::line(
                        vec![
                            corner - vec2(0.0, arm * sy),
                            corner,
                            corner - vec2(arm * sx, 0.0),
                        ],
                        s,
                    ));
                }
            },
        )
        .clicked()
        {
            self.toggle_fullscreen(ui.ctx());
        }

        p.rect_filled(pill, 8.0, PILL);
        let minus = Rect::from_min_size(pill.min, vec2(32.0, 32.0));
        let plus = Rect::from_min_size(pos2(pill.right() - 32.0, pill.top()), vec2(32.0, 32.0));
        if icon_button(ui, &p, minus, "zoom-out", false, 1.0, |p, c, col| {
            p.line_segment(
                [c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)],
                Stroke::new(1.8, col),
            );
        })
        .clicked()
            && has_image
        {
            self.zoom_at(area.center(), 0.8, area, ppp);
        }
        if icon_button(ui, &p, plus, "zoom-in", false, 1.0, |p, c, col| {
            let s = Stroke::new(1.8, col);
            p.line_segment([c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)], s);
            p.line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], s);
        })
        .clicked()
            && has_image
        {
            self.zoom_at(area.center(), 1.25, area, ppp);
        }
        let pct = if has_image {
            format!("{:.0}%", self.scale.to * ppp * 100.0)
        } else {
            "–".into()
        };
        p.text(pill.center(), Align2::CENTER_CENTER, pct, mono(12.0), TEXT);

        #[cfg(windows)]
        {
            let notice = match crate::update::status() {
                crate::update::Status::Idle => None,
                crate::update::Status::Ready => Some(s().update_ready),
                crate::update::Status::NoPermission => Some(s().update_no_permission),
            };
            if let Some(text) = notice {
                let r = p.text(
                    pos2(x, cy),
                    Align2::RIGHT_CENTER,
                    text,
                    body(12.0),
                    ACCENT_TEXT,
                );
                x = r.left() - 16.0;
            }
        }

        if !self.files.is_empty() {
            let counter = format!("{} / {}", self.idx + 1, self.files.len());
            let r = p.text(
                pos2(x, cy),
                Align2::RIGHT_CENTER,
                counter,
                mono(12.0),
                TEXT_2,
            );
            x = r.left() - 16.0;
        }

        // Left: icon, file name, folder.
        p.image(
            self.icon.id(),
            Rect::from_center_size(pos2(bar.left() + 16.0 + 11.0, cy), vec2(22.0, 22.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let left = bar.left() + 48.0;
        let clip = p.with_clip_rect(Rect::from_min_max(
            pos2(left, bar.top()),
            pos2(x.max(left), bar.bottom()),
        ));
        if let Some(path) = self.current() {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let r = clip.text(
                pos2(left, cy),
                Align2::LEFT_CENTER,
                name,
                strong(14.0),
                TEXT,
            );
            if let Some(dir) = path.parent() {
                clip.text(
                    pos2(r.right() + 12.0, cy + 0.5),
                    Align2::LEFT_CENTER,
                    dir.display().to_string(),
                    body(13.0),
                    TEXT_2,
                );
            }
        } else {
            clip.text(
                pos2(left, cy),
                Align2::LEFT_CENTER,
                "Afterimage",
                strong(14.0),
                TEXT,
            );
        }

        #[cfg(windows)]
        if self.custom_chrome {
            // In fullscreen the bar must not drag the window around.
            let caption = if self.fullscreen { 0.0 } else { bar.height() };
            crate::chrome::publish(caption, &hot, ppp);
        }
        #[cfg(not(windows))]
        let _ = hot;
    }

    /// Minimise / maximise / close at the right edge of the bar, in the
    /// system's own layout (46pt wide, full bar height). Returns the left
    /// edge of the group.
    fn draw_caption_buttons(
        &mut self,
        ui: &mut Ui,
        p: &Painter,
        bar: Rect,
        hot: &mut Vec<Rect>,
    ) -> f32 {
        const W: f32 = 46.0;
        let ctx = ui.ctx().clone();
        let (maximized, focused) = ctx.input(|i| {
            let v = i.viewport();
            (v.maximized.unwrap_or(false), v.focused.unwrap_or(true))
        });
        let icon = if focused { ICON } else { TEXT_2 };
        let mut x = bar.right();
        for kind in [2u8, 1, 0] {
            let rect = Rect::from_min_max(pos2(x - W, bar.top()), pos2(x, bar.bottom() - 1.0));
            x -= W;
            hot.push(rect);
            let r = ui.interact(rect, ui.scope_id().with(("caption", kind)), Sense::click());
            let (bg, col) = match (kind, r.hovered()) {
                (2, true) => (CLOSE_HOVER, TEXT),
                (_, true) => (HOVER, icon),
                _ => (Color32::TRANSPARENT, icon),
            };
            p.rect_filled(rect, 0.0, bg);
            let c = rect.center();
            let s = Stroke::new(1.0, col);
            match kind {
                0 => {
                    p.line_segment([c - vec2(5.0, 0.0), c + vec2(5.0, 0.0)], s);
                }
                1 if maximized => {
                    // Restore: a square with another peeking out behind it.
                    let front = Rect::from_center_size(c + vec2(-1.0, 1.0), vec2(8.0, 8.0));
                    p.rect_stroke(front, 1.0, s, StrokeKind::Inside);
                    p.add(Shape::line(
                        vec![
                            front.left_top() + vec2(2.0, -2.0),
                            front.right_top() + vec2(2.0, -2.0),
                            front.right_bottom() + vec2(2.0, -2.0),
                        ],
                        s,
                    ));
                }
                1 => {
                    let sq = Rect::from_center_size(c, vec2(10.0, 10.0));
                    p.rect_stroke(sq, 1.0, s, StrokeKind::Inside);
                }
                _ => {
                    p.line_segment([c - vec2(5.0, 5.0), c + vec2(5.0, 5.0)], s);
                    p.line_segment([c + vec2(-5.0, 5.0), c + vec2(5.0, -5.0)], s);
                }
            }
            if r.clicked() {
                let cmd = match kind {
                    0 => ViewportCommand::Minimized(true),
                    1 => ViewportCommand::Maximized(!maximized),
                    _ => ViewportCommand::Close,
                };
                ctx.send_viewport_cmd(cmd);
            }
        }
        x
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.current().and_then(|p| p.file_name()) {
            Some(n) => format!("{} — Afterimage", n.to_string_lossy()),
            None => "Afterimage".to_string(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }
}

impl eframe::App for Viewer {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BG.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.loader.set_max_side(ctx.input(|i| i.max_texture_side));
        self.poll(&ctx);

        if let Some(p) = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()))
        {
            self.open_path(&p);
        }

        let dt = ctx.input(|i| i.stable_dt).clamp(1.0 / 500.0, 1.0 / 20.0);
        let ppp = ctx.pixels_per_point();

        // Animation clock runs on wall time, so frames keep their real delays
        // even when nothing else is repainting.
        let now = ctx.input(|i| i.time);
        let elapsed = (now - self.last_time).clamp(0.0, 1.0);
        self.last_time = now;
        if !self.anim_paused {
            self.anim_ms += elapsed * 1000.0;
        }
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, BG);
        let bar = Rect::from_min_size(full.min, vec2(full.width(), TOP_H));
        let view = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
        // On wide windows the info panel pushes the photo aside instead of covering it;
        // the photo re-fits with the zoom spring as the panel springs open.
        self.info.to = if self.info_open { 1.0 } else { 0.0 };
        let mut content = view;
        if view.width() >= 760.0 {
            content.max.x -= (PANEL_W + 12.0) * self.info.x.clamp(0.0, 1.0);
        }
        let area = self.image_area(content);

        self.handle_keys(&ctx, area, ppp);

        self.draw_canvas(ui, content, area, ppp, dt);
        let nav = self.draw_nav(ui, content, area, dt);
        let (strip_moving, picked) = self.draw_strip(ui, content, dt);
        self.draw_panel(ui, view);
        self.draw_top_bar(ui, bar, area, ppp);

        if let Some(d) = nav {
            self.step(d);
        }
        if let Some(i) = picked {
            self.go(i);
        }
        self.update_title(&ctx);

        // ---- advance every spring; keep painting only while something moves.
        let dragging = self.swiping || ctx.input(|i| i.pointer.any_down());
        let mut moving = strip_moving;
        moving |= self.scale.step(spring::ZOOM, dt, 1e-5);
        if !dragging {
            moving |= self.pan_x.step(spring::ZOOM, dt, 0.1);
            moving |= self.pan_y.step(spring::ZOOM, dt, 0.1);
        }
        if !self.swiping {
            moving |= self.slide.step(spring::SWIPE, dt, 0.1);
        }
        moving |= self.fade.step(spring::SWIPE, dt, 0.002);
        moving |= self.info.step(spring::PANEL, dt, 0.001);
        moving |= self.nav_alpha.iter().any(|a| *a > 0.01 && *a < 0.99);
        if moving {
            ctx.request_repaint();
        } else if !self.anim_paused {
            // An animation only needs a repaint when its next frame is due.
            if let Some(l) = self.current().and_then(|p| self.full.get(p)) {
                if l.frames.len() > 1 {
                    let wait = until_next_frame(&l.frames, self.anim_ms);
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait / 1000.0));
                }
            }
        }
    }
}

// ---------------------------------------------------------------- helpers

/// Which frame is showing `t` ms into a looping animation.
fn frame_at(frames: &[(TextureHandle, u32)], t: f64) -> usize {
    let total: u32 = frames.iter().map(|(_, d)| *d).sum();
    if frames.len() < 2 || total == 0 {
        return 0;
    }
    let mut t = t % total as f64;
    for (i, (_, d)) in frames.iter().enumerate() {
        if t < *d as f64 {
            return i;
        }
        t -= *d as f64;
    }
    frames.len() - 1
}

/// Milliseconds until the frame after the one showing at `t` comes up.
fn until_next_frame(frames: &[(TextureHandle, u32)], t: f64) -> f64 {
    let total: u32 = frames.iter().map(|(_, d)| *d).sum();
    let mut t = t % total.max(1) as f64;
    for (_, d) in frames {
        if t < *d as f64 {
            return *d as f64 - t;
        }
        t -= *d as f64;
    }
    1.0
}

fn tex_options() -> TextureOptions {
    // At 100% a fractional position mustn't blur the image. Above 100% `sharp.rs`
    // takes over, so this only shows for the frame before a texture is uploaded.
    TextureOptions {
        magnification: TextureFilter::Nearest,
        mipmap_mode: Some(TextureFilter::Linear),
        ..TextureOptions::LINEAR
    }
}

/// UV rect that covers `target` with the texture, cropping the overflow (CSS `object-fit: cover`).
fn cover_uv(tex: Vec2, target: Vec2) -> Rect {
    let ta = tex.x / tex.y;
    let ra = target.x / target.y;
    if ta > ra {
        let w = ra / ta;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ta / ra;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

fn icon_button(
    ui: &mut Ui,
    p: &Painter,
    rect: Rect,
    salt: &str,
    active: bool,
    alpha: f32,
    draw: impl FnOnce(&Painter, Pos2, Color32),
) -> Response {
    let r = ui
        .interact(rect, ui.scope_id().with(salt), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let bg = if active {
        ACCENT_SOFT
    } else if r.hovered() {
        HOVER
    } else {
        Color32::TRANSPARENT
    };
    p.rect_filled(rect, 8.0, bg.gamma_multiply(alpha));
    let col = if active { ACCENT_TEXT } else { ICON };
    draw(p, rect.center(), col.gamma_multiply(alpha));
    r
}

fn human_size(b: u64) -> String {
    let b = b as f64;
    if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / 1024.0 / 1024.0)
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{b} B")
    }
}

fn reveal_in_explorer(path: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(not(windows))]
    {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}
