//! The app's top bar doubles as the window's title bar.
//!
//! The window keeps its native frame (shadow, resize edges, snapping), but
//! the caption is folded into the client area: `WM_NCCALCSIZE` drops it, and
//! `WM_NCHITTEST` reports the empty parts of the top bar as `HTCAPTION`, so
//! dragging, double-click to maximise, Aero Snap and the system menu still
//! come from Windows. The app draws its own minimise / maximise / close.

use std::sync::Mutex;

use eframe::egui::Rect;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowLongW, IsZoomed, SetWindowPos, GWL_STYLE, HTCAPTION, HTCLIENT, HTTOP,
    HTTOPLEFT, HTTOPRIGHT, NCCALCSIZE_PARAMS, SM_CXPADDEDBORDER, SM_CYFRAME, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_NCCALCSIZE, WM_NCHITTEST, WS_CAPTION,
};

/// What the app last drew, in physical pixels from the client's top-left.
struct Layout {
    caption_h: i32,
    /// Buttons inside the caption band; these stay clickable.
    hot: Vec<RECT>,
}

static LAYOUT: Mutex<Layout> = Mutex::new(Layout {
    caption_h: 0,
    hot: Vec::new(),
});

/// Take over the caption. Returns false (and changes nothing) when there is
/// no Win32 window to work with; the app then keeps the system title bar.
pub fn install(cc: &eframe::CreationContext<'_>) -> bool {
    let Ok(handle) = cc.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(h) = handle.as_raw() else {
        return false;
    };
    let hwnd = HWND(h.hwnd.get() as *mut _);
    unsafe {
        // Dark system menu and frame to match the app.
        set_dwm(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
        if !SetWindowSubclass(hwnd, Some(subclass), 1, 0).as_bool() {
            return false;
        }
        // Recompute the frame now that the caption is gone.
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    true
}

/// Tell the hit test where the caption band and its buttons are this frame.
/// `caption` is zero-height while the bar should not drag the window
/// (fullscreen).
pub fn publish(caption: f32, hot: &[Rect], ppp: f32) {
    let px = |v: f32| (v * ppp).round() as i32;
    let mut l = LAYOUT.lock().unwrap();
    l.caption_h = px(caption);
    l.hot.clear();
    l.hot.extend(hot.iter().map(|r| RECT {
        left: px(r.left()),
        top: px(r.top()),
        right: px(r.right()),
        bottom: px(r.bottom()),
    }));
}

unsafe fn set_dwm(hwnd: HWND, attr: DWMWINDOWATTRIBUTE, value: u32) {
    let _ = DwmSetWindowAttribute(hwnd, attr, (&raw const value).cast(), 4);
}

/// Resize border thickness (frame + padding) at the window's DPI.
unsafe fn frame_y(hwnd: HWND) -> i32 {
    let dpi = GetDpiForWindow(hwnd);
    GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
}

unsafe fn has_caption(hwnd: HWND) -> bool {
    // Borderless fullscreen drops WS_CAPTION; leave that to the defaults.
    (GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_CAPTION.0) == WS_CAPTION.0
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_NCCALCSIZE if wparam.0 != 0 && has_caption(hwnd) => {
            let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
            let top = params.rgrc[0].top;
            let r = DefSubclassProc(hwnd, msg, wparam, lparam);
            // Keep the side and bottom frame, drop the caption and top frame.
            // A maximised window hangs its frame off-screen, so push the top
            // back down by that much.
            params.rgrc[0].top = top
                + if IsZoomed(hwnd).as_bool() {
                    frame_y(hwnd)
                } else {
                    0
                };
            r
        }
        WM_NCHITTEST if has_caption(hwnd) => {
            let hit = DefSubclassProc(hwnd, msg, wparam, lparam);
            if hit.0 as u32 != HTCLIENT {
                return hit;
            }
            let mut pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let _ = ScreenToClient(hwnd, &mut pt);
            let maximized = IsZoomed(hwnd).as_bool();
            if !maximized {
                // The top resize edge now lies inside the client area.
                let edge = frame_y(hwnd);
                if pt.y < edge {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let code = if pt.x < edge * 2 {
                        HTTOPLEFT
                    } else if pt.x >= rc.right - edge * 2 {
                        HTTOPRIGHT
                    } else {
                        HTTOP
                    };
                    return LRESULT(code as isize);
                }
            }
            let l = LAYOUT.lock().unwrap();
            let on_button = l
                .hot
                .iter()
                .any(|r| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom);
            if pt.y < l.caption_h && !on_button {
                LRESULT(HTCAPTION as isize)
            } else {
                hit
            }
        }
        _ => DefSubclassProc(hwnd, msg, wparam, lparam),
    }
}
