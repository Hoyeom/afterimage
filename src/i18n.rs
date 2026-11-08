//! UI text in the Windows display language.
//!
//! Korean Windows gets Korean; everything else gets English. To add a language,
//! add a `Strings` table and a case in `detect`, and check that the fonts in
//! `main.rs` cover its characters. `AFTERIMAGE_LANG=en|ko` overrides detection.

use std::sync::OnceLock;

pub struct Strings {
    pub images: &'static str,
    pub cannot_open: &'static str,
    pub loading: &'static str,
    pub drop_hint: &'static str,
    pub or: &'static str,
    pub open_file: &'static str,
    pub info: &'static str,
    pub histogram: &'static str,
    pub file_name: &'static str,
    pub resolution: &'static str,
    pub file_size: &'static str,
    pub format: &'static str,
    pub camera: &'static str,
    pub lens: &'static str,
    pub exposure: &'static str,
    pub taken: &'static str,
    pub modified: &'static str,
    pub animation: &'static str,
    pub paused: &'static str,
    pub decoding: &'static str,
    pub reveal: &'static str,
    pub copy_path: &'static str,
    pub update_ready: &'static str,
    pub update_no_permission: &'static str,
    pub other_format: &'static str,
    pub wic_init_failed: &'static str,
    pub wic_bad_size: &'static str,
    pub need_heic_codec: &'static str,
    pub need_avif_codec: &'static str,
    pub no_codec: &'static str,
    /// `{n}` frames, `{s}` seconds.
    pub frames: &'static str,
}

impl Strings {
    pub fn frames(&self, n: usize, seconds: f32) -> String {
        self.frames
            .replace("{n}", &n.to_string())
            .replace("{s}", &format!("{seconds:.1}"))
    }
}

const EN: Strings = Strings {
    images: "Images",
    cannot_open: "Can't open this file",
    loading: "Loading…",
    drop_hint: "Drop an image into this window",
    or: "or",
    open_file: "Open file  Ctrl+O",
    info: "Info",
    histogram: "Histogram",
    file_name: "File name",
    resolution: "Resolution",
    file_size: "File size",
    format: "Format",
    camera: "Camera",
    lens: "Lens",
    exposure: "Exposure",
    taken: "Taken",
    modified: "Modified",
    animation: "Animation",
    paused: " · Paused (P)",
    decoding: "Decoding",
    reveal: "Show in Explorer",
    copy_path: "Copy path",
    update_ready: "Update ready. It applies next time you open the app",
    update_no_permission: "New version available. Can't install it in this folder",
    other_format: "Other",
    wic_init_failed: "Couldn't start the Windows imaging component",
    wic_bad_size: "The image size isn't valid",
    need_heic_codec:
        "To open HEIC, install 'HEIF Image Extensions' and 'HEVC Video Extensions' from the Microsoft Store",
    need_avif_codec: "To open AVIF, install 'AV1 Video Extension' from the Microsoft Store",
    no_codec: "Windows has no codec for this format",
    frames: "{n} frames · {s} s",
};

const KO: Strings = Strings {
    images: "이미지",
    cannot_open: "이 파일은 열 수 없어요",
    loading: "불러오는 중…",
    drop_hint: "이미지를 이 창으로 끌어다 놓으세요",
    or: "또는",
    open_file: "파일 열기  Ctrl+O",
    info: "정보",
    histogram: "히스토그램",
    file_name: "파일 이름",
    resolution: "해상도",
    file_size: "파일 크기",
    format: "형식",
    camera: "카메라",
    lens: "렌즈",
    exposure: "노출",
    taken: "촬영일",
    modified: "수정한 날짜",
    animation: "애니메이션",
    paused: " · 일시정지 (P)",
    decoding: "디코딩",
    reveal: "탐색기에서 열기",
    copy_path: "경로 복사",
    update_ready: "업데이트가 준비됐어요. 다시 열면 적용돼요",
    update_no_permission: "새 버전이 있어요. 이 폴더에는 자동으로 설치할 수 없어요",
    other_format: "기타",
    wic_init_failed: "Windows 이미지 구성 요소를 초기화하지 못했어요",
    wic_bad_size: "이미지 크기가 올바르지 않아요",
    need_heic_codec:
        "HEIC를 열려면 Microsoft Store에서 'HEIF 이미지 확장'과 'HEVC 비디오 확장'을 설치해 주세요",
    need_avif_codec: "AVIF를 열려면 Microsoft Store에서 'AV1 비디오 확장'을 설치해 주세요",
    no_codec: "이 형식을 읽을 수 있는 코덱이 Windows에 없어요",
    frames: "{n}프레임 · {s}초",
};

/// The table for this run. Decided once, on first use.
pub fn s() -> &'static Strings {
    static CHOSEN: OnceLock<&'static Strings> = OnceLock::new();
    CHOSEN.get_or_init(detect)
}

fn detect() -> &'static Strings {
    let lang = std::env::var("AFTERIMAGE_LANG").unwrap_or_default();
    match lang.as_str() {
        "ko" => &KO,
        "en" => &EN,
        _ if system_is_korean() => &KO,
        _ => &EN,
    }
}

#[cfg(windows)]
fn system_is_korean() -> bool {
    const LANG_KOREAN: u16 = 0x12;
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    id & 0x3FF == LANG_KOREAN
}

#[cfg(not(windows))]
fn system_is_korean() -> bool {
    std::env::var("LANG").is_ok_and(|l| l.starts_with("ko"))
}
