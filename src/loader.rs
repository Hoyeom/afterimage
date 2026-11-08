//! Folder scanning and background decoding.
//!
//! Decoding never happens on the UI thread. The UI asks for a window of
//! images around the current one (current first, then neighbours); worker
//! threads decode them and hand back GPU-ready pixels.

use std::collections::{HashSet, VecDeque};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Instant, SystemTime};

use eframe::egui::{self, ColorImage};
use image::{imageops::FilterType, DynamicImage, ImageFormat, ImageReader};

/// HEIC / AVIF go through the decoders built into Windows (see `wic.rs`).
#[cfg(windows)]
pub const EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "gif", "webp", "bmp", "tif", "tiff", "ico", "heic", "heif",
    "avif",
];
#[cfg(not(windows))]
pub const EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "gif", "webp", "bmp", "tif", "tiff", "ico",
];

/// Animations are capped so a huge GIF can't eat all memory.
const ANIM_BUDGET_BYTES: usize = 384 * 1024 * 1024;

pub const THUMB_SIDE: u32 = 192;
pub const HIST_BINS: usize = 64;

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// All supported images next to `file`, plus the index of `file`.
///
/// On Windows the order is the one the Explorer window showing this folder
/// uses (sort column, direction, grouping); otherwise, and when no such
/// window is open, it is natural name order the way Explorer compares names.
/// Talks to Explorer, so call it off the UI thread (`Loader::scan`).
pub fn scan_folder(file: &Path) -> (Vec<PathBuf>, usize) {
    let Some(dir) = file.parent() else {
        return (vec![file.to_path_buf()], 0);
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_supported(p))
                .collect()
        })
        .unwrap_or_default();
    sort_by_name(&mut files);
    #[cfg(windows)]
    if let Some(order) = crate::explorer::view_order(dir, file) {
        apply_order(&mut files, &order);
    }
    let name = file.file_name();
    let idx = files.iter().position(|p| p.file_name() == name);
    match idx {
        Some(i) => (files, i),
        None => {
            files.insert(0, file.to_path_buf());
            (files, 0)
        }
    }
}

#[cfg(windows)]
fn sort_by_name(files: &mut Vec<PathBuf>) {
    use std::os::windows::ffi::OsStrExt;
    let mut keyed: Vec<(Vec<u16>, PathBuf)> = files
        .drain(..)
        .map(|p| {
            let mut w: Vec<u16> = p.file_name().unwrap_or_default().encode_wide().collect();
            w.push(0);
            (w, p)
        })
        .collect();
    keyed.sort_by(|a, b| crate::explorer::compare_names(&a.0, &b.0));
    files.extend(keyed.into_iter().map(|(_, p)| p));
}

#[cfg(not(windows))]
fn sort_by_name(files: &mut [PathBuf]) {
    let key = |p: &PathBuf| {
        p.file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    files.sort_by(|a, b| natord::compare(&key(a), &key(b)));
}

/// Reorder `files` to follow `order` (Explorer's item list, which also holds
/// folders and other files). Anything Explorer doesn't list yet, such as a
/// file created a moment ago, goes last in name order.
#[cfg(windows)]
fn apply_order(files: &mut [PathBuf], order: &[PathBuf]) {
    use std::collections::HashMap;
    let key = |p: &Path| p.file_name().map(|s| s.to_string_lossy().to_lowercase());
    let rank: HashMap<String, usize> = order
        .iter()
        .enumerate()
        .filter_map(|(i, p)| Some((key(p)?, i)))
        .collect();
    files.sort_by_key(|p| {
        key(p)
            .and_then(|k| rank.get(&k).copied())
            .unwrap_or(usize::MAX)
    });
}

pub struct Meta {
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    pub format: &'static str,
    pub modified: Option<SystemTime>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub exposure: Option<String>,
    pub taken: Option<String>,
    pub histogram: [f32; HIST_BINS],
    pub decode_ms: u32,
    /// Total length of one loop, for animations.
    pub duration_ms: u32,
}

pub struct Decoded {
    /// One frame for still images; every frame with its delay (ms) for animations.
    pub frames: Vec<(ColorImage, u32)>,
    pub meta: Meta,
}

pub enum Done {
    Full {
        path: PathBuf,
        result: Result<Decoded, String>,
    },
    Thumb {
        path: PathBuf,
        image: Option<ColorImage>,
    },
    /// The folder listing for `file`, from `Loader::scan`.
    Folder {
        file: PathBuf,
        files: Vec<PathBuf>,
        idx: usize,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Full,
    Thumb,
}

struct Job {
    path: PathBuf,
    kind: Kind,
}

#[derive(Default)]
struct Queue {
    full: VecDeque<Job>,
    thumb: VecDeque<Job>,
    in_flight: HashSet<(PathBuf, bool)>,
}

pub struct Loader {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    pub rx: mpsc::Receiver<Done>,
    tx: mpsc::Sender<Done>,
    max_side: Arc<AtomicUsize>,
    repaint: Arc<Mutex<Option<egui::Context>>>,
}

impl Loader {
    pub fn new() -> Self {
        let shared: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        let (tx, rx) = mpsc::channel();
        let max_side = Arc::new(AtomicUsize::new(8192));
        let repaint: Arc<Mutex<Option<egui::Context>>> = Arc::default();

        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let workers = cores.saturating_sub(1).clamp(2, 4);
        for i in 0..workers {
            let shared = shared.clone();
            let tx = tx.clone();
            let max_side = max_side.clone();
            let repaint = repaint.clone();
            std::thread::Builder::new()
                .name(format!("decode-{i}"))
                .spawn(move || worker(shared, tx, max_side, repaint))
                .expect("spawn decode worker");
        }
        Self {
            shared,
            rx,
            tx,
            max_side,
            repaint,
        }
    }

    pub fn set_context(&self, ctx: &egui::Context) {
        *self.repaint.lock().unwrap() = Some(ctx.clone());
    }

    pub fn set_max_side(&self, side: usize) {
        self.max_side.store(side.max(1024), Ordering::Relaxed);
    }

    /// List the folder of `file` in the background; arrives as `Done::Folder`.
    /// Asking Explorer for its order can take a while, so it never blocks
    /// startup or the first image.
    pub fn scan(&self, file: PathBuf) {
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        let _ = std::thread::Builder::new()
            .name("scan".into())
            .spawn(move || {
                let (files, idx) = scan_folder(&file);
                if tx.send(Done::Folder { file, files, idx }).is_ok() {
                    if let Some(ctx) = repaint.lock().unwrap().as_ref() {
                        ctx.request_repaint();
                    }
                }
            });
    }

    /// Replace the pending full-size decodes, highest priority first.
    pub fn want_full(&self, paths: Vec<PathBuf>) {
        self.replace(paths, Kind::Full);
    }

    /// Replace the pending thumbnail decodes, highest priority first.
    pub fn want_thumbs(&self, paths: Vec<PathBuf>) {
        self.replace(paths, Kind::Thumb);
    }

    fn replace(&self, paths: Vec<PathBuf>, kind: Kind) {
        let (lock, cv) = &*self.shared;
        let mut q = lock.lock().unwrap();
        let is_full = kind == Kind::Full;
        let jobs: VecDeque<Job> = paths
            .into_iter()
            .filter(|p| !q.in_flight.contains(&(p.clone(), is_full)))
            .map(|path| Job { path, kind })
            .collect();
        if is_full {
            q.full = jobs;
        } else {
            q.thumb = jobs;
        }
        cv.notify_all();
    }
}

fn worker(
    shared: Arc<(Mutex<Queue>, Condvar)>,
    tx: mpsc::Sender<Done>,
    max_side: Arc<AtomicUsize>,
    repaint: Arc<Mutex<Option<egui::Context>>>,
) {
    let (lock, cv) = &*shared;
    loop {
        let job = {
            let mut q = lock.lock().unwrap();
            loop {
                if let Some(j) = q.full.pop_front() {
                    break j;
                }
                if let Some(j) = q.thumb.pop_front() {
                    break j;
                }
                q = cv.wait(q).unwrap();
            }
        };
        let key = (job.path.clone(), job.kind == Kind::Full);
        lock.lock().unwrap().in_flight.insert(key.clone());

        let done = match job.kind {
            Kind::Full => Done::Full {
                result: decode_full(&job.path, max_side.load(Ordering::Relaxed)),
                path: job.path,
            },
            Kind::Thumb => Done::Thumb {
                image: decode_thumb(&job.path),
                path: job.path,
            },
        };

        lock.lock().unwrap().in_flight.remove(&key);
        if tx.send(done).is_err() {
            return;
        }
        if let Some(ctx) = repaint.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }
}

fn open_oriented(bytes: &[u8]) -> Result<(DynamicImage, &'static str), String> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = format_name(reader.format());
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = image::ImageDecoder::orientation(&mut decoder).ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok((img, format))
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Decode a still image: the pure-Rust decoders first, then (on Windows)
/// the system's own codecs for HEIC / AVIF and anything else they know.
fn load_still(path: &Path, bytes: &[u8]) -> Result<(DynamicImage, &'static str), String> {
    #[cfg(windows)]
    {
        let ext = ext_of(path);
        if matches!(ext.as_str(), "heic" | "heif" | "avif") {
            let fmt = if ext == "avif" { "AVIF" } else { "HEIC" };
            return crate::wic::decode(path).map(|i| (DynamicImage::ImageRgba8(i), fmt));
        }
        match open_oriented(bytes) {
            Ok(x) => Ok(x),
            Err(e) => crate::wic::decode(path)
                .map(|i| (DynamicImage::ImageRgba8(i), "기타"))
                .map_err(|_| e),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        open_oriented(bytes)
    }
}

fn fit_texture(img: image::RgbaImage, max_side: usize) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    if (w.max(h) as usize) <= max_side {
        return img;
    }
    let s = max_side as f32 / w.max(h) as f32;
    image::imageops::resize(
        &img,
        ((w as f32 * s) as u32).max(1),
        ((h as f32 * s) as u32).max(1),
        FilterType::Triangle,
    )
}

/// Every frame of an animated GIF / WebP / APNG, or None for a still image.
fn decode_animation(
    bytes: &[u8],
    max_side: usize,
) -> Option<(Vec<(image::RgbaImage, u32)>, u32, u32)> {
    use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};
    use image::AnimationDecoder;

    let frames = match image::guess_format(bytes).ok()? {
        ImageFormat::Gif => GifDecoder::new(Cursor::new(bytes)).ok()?.into_frames(),
        ImageFormat::WebP => {
            let d = WebPDecoder::new(Cursor::new(bytes)).ok()?;
            if !d.has_animation() {
                return None;
            }
            d.into_frames()
        }
        ImageFormat::Png => {
            let d = PngDecoder::new(Cursor::new(bytes)).ok()?;
            if !d.is_apng().ok()? {
                return None;
            }
            d.apng().ok()?.into_frames()
        }
        _ => return None,
    };

    let mut out = Vec::new();
    let mut used = 0usize;
    let mut size = (0, 0);
    for f in frames {
        let Ok(f) = f else { break };
        let (n, d) = f.delay().numer_denom_ms();
        let ms = n / d.max(1);
        // Browsers treat near-zero delays as 100 ms; so do we.
        let ms = if ms < 20 { 100 } else { ms };
        let buf = f.into_buffer();
        if out.is_empty() {
            size = buf.dimensions();
        }
        let buf = fit_texture(buf, max_side);
        used += buf.as_raw().len();
        out.push((buf, ms));
        if used > ANIM_BUDGET_BYTES {
            break;
        }
    }
    if out.len() < 2 {
        return None;
    }
    Some((out, size.0, size.1))
}

fn to_color(img: &image::RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

fn decode_full(path: &Path, max_side: usize) -> Result<Decoded, String> {
    let t0 = Instant::now();
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;

    let (frames, width, height, format) =
        if let Some((anim, w, h)) = decode_animation(&bytes, max_side) {
            let fmt = format_name(image::guess_format(&bytes).ok());
            (anim, w, h, fmt)
        } else {
            let (img, fmt) = load_still(path, &bytes)?;
            let (w, h) = (img.width(), img.height());
            // Textures larger than the GPU allows are scaled down once, here.
            (
                vec![(fit_texture(img.into_rgba8(), max_side), 0)],
                w,
                h,
                fmt,
            )
        };

    let histogram = histogram(frames[0].0.as_raw());
    let duration_ms = frames.iter().map(|(_, d)| *d).sum();
    let frames: Vec<(ColorImage, u32)> =
        frames.iter().map(|(img, d)| (to_color(img), *d)).collect();

    let mut meta = Meta {
        width,
        height,
        file_size: bytes.len() as u64,
        format,
        modified: std::fs::metadata(path).and_then(|m| m.modified()).ok(),
        camera: None,
        lens: None,
        exposure: None,
        taken: None,
        histogram,
        decode_ms: t0.elapsed().as_millis() as u32,
        duration_ms,
    };
    read_exif(&bytes, &mut meta);
    Ok(Decoded { frames, meta })
}

fn decode_thumb(path: &Path) -> Option<ColorImage> {
    let bytes = std::fs::read(path).ok()?;
    let (img, _) = load_still(path, &bytes).ok()?;
    let t = img.thumbnail(THUMB_SIDE, THUMB_SIDE).into_rgba8();
    Some(ColorImage::from_rgba_unmultiplied(
        [t.width() as usize, t.height() as usize],
        t.as_raw(),
    ))
}

fn histogram(rgba: &[u8]) -> [f32; HIST_BINS] {
    let mut bins = [0u32; HIST_BINS];
    // Sampling every 7th pixel is plenty for a 64-bin overview.
    for px in rgba.chunks_exact(4).step_by(7) {
        let l = (px[0] as u32 * 54 + px[1] as u32 * 183 + px[2] as u32 * 19) >> 8;
        bins[(l as usize * HIST_BINS) >> 8] += 1;
    }
    let max = *bins.iter().max().unwrap_or(&1) as f32;
    let mut out = [0.0; HIST_BINS];
    for (o, b) in out.iter_mut().zip(bins) {
        *o = if max > 0.0 {
            (b as f32 / max).sqrt()
        } else {
            0.0
        };
    }
    out
}

fn format_name(f: Option<ImageFormat>) -> &'static str {
    match f {
        Some(ImageFormat::Jpeg) => "JPEG",
        Some(ImageFormat::Png) => "PNG",
        Some(ImageFormat::Gif) => "GIF",
        Some(ImageFormat::WebP) => "WebP",
        Some(ImageFormat::Bmp) => "BMP",
        Some(ImageFormat::Tiff) => "TIFF",
        Some(ImageFormat::Ico) => "ICO",
        _ => "기타",
    }
}

fn read_exif(bytes: &[u8], meta: &mut Meta) {
    use exif::{In, Tag};
    let Ok(ex) = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)) else {
        return;
    };
    let text = |tag: Tag| -> Option<String> {
        ex.get_field(tag, In::PRIMARY).map(|f| {
            f.display_value()
                .to_string()
                .trim_matches('"')
                .trim()
                .to_string()
        })
    };
    let unit = |tag: Tag| -> Option<String> {
        ex.get_field(tag, In::PRIMARY)
            .map(|f| f.display_value().with_unit(&ex).to_string())
    };

    let make = text(Tag::Make).filter(|s| !s.is_empty());
    let model = text(Tag::Model).filter(|s| !s.is_empty());
    meta.camera = match (make, model) {
        (Some(mk), Some(md))
            if !md.to_lowercase().starts_with(
                &mk.to_lowercase()
                    .split(' ')
                    .next()
                    .unwrap_or("")
                    .to_string(),
            ) =>
        {
            Some(format!("{mk} {md}"))
        }
        (_, Some(md)) => Some(md),
        (Some(mk), None) => Some(mk),
        _ => None,
    };
    meta.lens = text(Tag::LensModel).filter(|s| !s.is_empty());

    let parts: Vec<String> = [
        unit(Tag::FNumber),
        unit(Tag::ExposureTime),
        text(Tag::PhotographicSensitivity).map(|s| format!("ISO {s}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !parts.is_empty() {
        meta.exposure = Some(parts.join(" · "));
    }
    meta.taken = text(Tag::DateTimeOriginal).map(|s| s.chars().take(16).collect());
}
