//! Follow the order File Explorer is showing.
//!
//! When an image is opened from Explorer, the open Explorer window that shows
//! its folder (preferably the one with that file selected) is asked for its
//! items in view order, so sort column, direction and grouping all match.
//! Without such a window, names are compared the way Explorer does
//! (`StrCmpLogicalW`).

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use windows::core::{Interface, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IDispatch, IServiceProvider,
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Shell::{
    IFolderView, IPersistFolder2, IShellBrowser, IShellItem, IShellItemArray, IShellWindows,
    SHCreateItemFromIDList, SID_STopLevelBrowser, ShellWindows, StrCmpLogicalW, _SVGIO,
    SIGDN_FILESYSPATH, SVGIO_ALLVIEW, SVGIO_FLAG_VIEWORDER, SVGIO_SELECTION,
};

/// Explorer gets this long to answer; after that we fall back to name order.
const TIMEOUT: Duration = Duration::from_millis(1500);

/// Compare two NUL-terminated UTF-16 file names like Explorer's name column.
pub fn compare_names(a: &[u16], b: &[u16]) -> Ordering {
    unsafe { StrCmpLogicalW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr())) }.cmp(&0)
}

/// Every item path in the Explorer window showing `dir`, in on-screen order.
pub fn view_order(dir: &Path, file: &Path) -> Option<Vec<PathBuf>> {
    let dir = std::path::absolute(dir).ok()?;
    let file = std::path::absolute(file).ok()?;
    let (tx, rx) = mpsc::channel();
    // A thread of its own so COM is set up cleanly, whatever the caller's
    // apartment, and a hung Explorer can't hold us up.
    std::thread::Builder::new()
        .name("explorer-order".into())
        .spawn(move || {
            let _ = tx.send(unsafe { query(&dir, &file) });
        })
        .ok()?;
    rx.recv_timeout(TIMEOUT).ok().flatten()
}

unsafe fn query(dir: &Path, file: &Path) -> Option<Vec<PathBuf>> {
    let inited = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
    let result = find(dir, file);
    if inited {
        CoUninitialize();
    }
    result
}

unsafe fn find(dir: &Path, file: &Path) -> Option<Vec<PathBuf>> {
    let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
    let mut first: Option<IFolderView> = None;
    for i in 0..windows.Count().ok()? {
        let Ok(disp) = windows.Item(&index(i)) else {
            continue;
        };
        let Some(view) = folder_view(&disp) else {
            continue;
        };
        if !folder_path(&view).is_some_and(|p| same_path(&p, dir)) {
            continue;
        }
        // Several windows (or tabs) may show the folder; the one where the
        // file is selected is the one it was opened from.
        if is_selected(&view, file) {
            return items(&view, view_order_flags());
        }
        first.get_or_insert(view);
    }
    items(&first?, view_order_flags())
}

fn view_order_flags() -> _SVGIO {
    _SVGIO(SVGIO_ALLVIEW.0 | SVGIO_FLAG_VIEWORDER.0)
}

/// `IShellWindows::Item` takes its index as a VARIANT (VT_I4).
fn index(i: i32) -> VARIANT {
    let mut v = VARIANT::default();
    unsafe {
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = i;
    }
    v
}

unsafe fn folder_view(disp: &IDispatch) -> Option<IFolderView> {
    let sp: IServiceProvider = disp.cast().ok()?;
    let browser: IShellBrowser = sp.QueryService(&SID_STopLevelBrowser).ok()?;
    browser.QueryActiveShellView().ok()?.cast().ok()
}

unsafe fn folder_path(view: &IFolderView) -> Option<PathBuf> {
    let folder: IPersistFolder2 = view.GetFolder().ok()?;
    let pidl = folder.GetCurFolder().ok()?;
    let item: windows::core::Result<IShellItem> = SHCreateItemFromIDList(pidl);
    CoTaskMemFree(Some(pidl as *const _));
    item_path(&item.ok()?)
}

unsafe fn is_selected(view: &IFolderView, file: &Path) -> bool {
    items(view, SVGIO_SELECTION).is_some_and(|sel| sel.iter().any(|p| same_path(p, file)))
}

unsafe fn items(view: &IFolderView, flags: _SVGIO) -> Option<Vec<PathBuf>> {
    let arr: IShellItemArray = view.Items(flags).ok()?;
    let n = arr.GetCount().ok()?;
    Some(
        (0..n)
            .filter_map(|i| arr.GetItemAt(i).ok())
            .filter_map(|item| item_path(&item))
            .collect(),
    )
}

unsafe fn item_path(item: &IShellItem) -> Option<PathBuf> {
    // Fails for virtual folders (search results, This PC, ...), which is fine.
    let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    let s = name.to_string().ok();
    CoTaskMemFree(Some(name.0 as *const _));
    s.map(PathBuf::from)
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase()
    };
    norm(a) == norm(b)
}
