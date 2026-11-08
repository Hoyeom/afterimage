//! Decoding through Windows Imaging Component (WIC).
//!
//! HEIC and AVIF use codecs that ship with Windows or the Microsoft Store
//! ("HEIF Image Extensions" + "HEVC Video Extensions", "AV1 Video Extension").
//! Borrowing them avoids bundling patent-encumbered decoders.

use std::path::Path;

use image::RgbaImage;
use windows::core::HSTRING;
use windows::Win32::Foundation::{GENERIC_READ, WINCODEC_ERR_COMPONENTNOTFOUND};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory,
    WICConvertBitmapSource, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

thread_local! {
    // Each decode worker initialises COM once and keeps its own factory.
    static FACTORY: Option<IWICImagingFactory> = unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()
    };
}

pub fn decode(path: &Path) -> Result<RgbaImage, String> {
    FACTORY.with(|factory| {
        let factory = factory
            .as_ref()
            .ok_or_else(|| "Windows 이미지 구성 요소를 초기화하지 못했어요".to_string())?;
        let explain = |e: windows::core::Error| explain(e, path);
        unsafe {
            let name = HSTRING::from(path.as_os_str());
            let decoder = factory
                .CreateDecoderFromFilename(
                    &name,
                    None,
                    GENERIC_READ,
                    WICDecodeMetadataCacheOnDemand,
                )
                .map_err(explain)?;
            let frame = decoder.GetFrame(0).map_err(explain)?;
            let rgba =
                WICConvertBitmapSource(&GUID_WICPixelFormat32bppRGBA, &frame).map_err(explain)?;
            let (mut w, mut h) = (0u32, 0u32);
            rgba.GetSize(&mut w, &mut h).map_err(explain)?;
            let stride = w * 4;
            let mut buf = vec![0u8; stride as usize * h as usize];
            rgba.CopyPixels(std::ptr::null(), stride, &mut buf)
                .map_err(explain)?;
            RgbaImage::from_raw(w, h, buf)
                .ok_or_else(|| "이미지 크기가 올바르지 않아요".to_string())
        }
    })
}

fn explain(e: windows::core::Error, path: &Path) -> String {
    if e.code() == WINCODEC_ERR_COMPONENTNOTFOUND {
        return match crate::loader::ext_of(path).as_str() {
            "heic" | "heif" => {
                "HEIC를 열려면 Microsoft Store에서 'HEIF 이미지 확장'과 'HEVC 비디오 확장'을 설치해 주세요".into()
            }
            "avif" => "AVIF를 열려면 Microsoft Store에서 'AV1 비디오 확장'을 설치해 주세요".into(),
            _ => "이 형식을 읽을 수 있는 코덱이 Windows에 없어요".into(),
        };
    }
    e.message().to_string()
}
