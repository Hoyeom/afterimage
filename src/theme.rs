use eframe::egui::{Color32, FontFamily, FontId};

pub const BG: Color32 = Color32::from_rgb(0x0B, 0x0C, 0x0E);
pub const PANEL: Color32 = Color32::from_rgba_unmultiplied_const(20, 22, 26, 240);
pub const SURFACE: Color32 = Color32::from_rgba_unmultiplied_const(22, 24, 28, 214);
pub const BORDER: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 20);
pub const DIVIDER: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 15);
pub const HOVER: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 14);
pub const PILL: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 12);
pub const PLACEHOLDER: Color32 = Color32::from_rgb(0x1A, 0x1D, 0x23);

pub const TEXT: Color32 = Color32::from_rgb(0xEC, 0xED, 0xEF);
pub const TEXT_2: Color32 = Color32::from_rgb(0x9A, 0x9E, 0xA6);
pub const ICON: Color32 = Color32::from_rgb(0xC9, 0xCC, 0xD1);

/// Close button hover, the system caption red.
pub const CLOSE_HOVER: Color32 = Color32::from_rgb(0xC4, 0x2B, 0x1C);

pub const ACCENT: Color32 = Color32::from_rgb(0x7A, 0xA2, 0xFF);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0xA9, 0xC2, 0xFF);
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_unmultiplied_const(122, 162, 255, 40);

pub fn body(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn strong(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("strong".into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}
