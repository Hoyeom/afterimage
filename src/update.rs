//! Self-update from GitHub Releases.
//!
//! Once per launch, a background thread reads `latest.txt` from the newest
//! release (`<version> <sha256 of afterimage.exe> <ed25519 signature>`). If the
//! signature checks out and that version is newer than this build, it downloads
//! the exe, verifies the hash and swaps it in beside the running one: Windows
//! can't overwrite a running exe but can rename it. The new version takes effect
//! the next time the app starts.
//!
//! The signing key lives only on the maintainer's PC (`examples/sign.rs`), so
//! control of the GitHub account or the build alone can't push an update.
//!
//! Failures are silent on purpose; being offline must never bother anyone.
//! Set `AFTERIMAGE_NO_UPDATE=1` to turn this off.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use ed25519_dalek::{Signature, VerifyingKey};
use eframe::egui;
use sha2::{Digest, Sha256};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

const HOST: &str = "github.com";
const REPO: &str = "Hoyeom/afterimage";
const EXE_NAME: &str = "afterimage.exe";
/// Far above the real size; only here so a bad response can't fill the disk.
const MAX_DOWNLOAD: usize = 200 * 1024 * 1024;
/// Let the first image and the window have the machine to themselves.
const START_DELAY: Duration = Duration::from_secs(3);
/// GitHub sometimes answers release downloads with a passing 504.
const ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(20);
/// Replacing this key cuts off every installed copy from updates.
const PUBLIC_KEY: &str = "28fb757ba63482dc2dbe8de149b3de1d1d731a89acf0c87bb52be6b02fe96210";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Idle,
    /// A newer exe is in place and applies on the next launch.
    Ready,
    /// A newer version exists but this folder isn't writable.
    NoPermission,
}

static STATUS: AtomicU8 = AtomicU8::new(0);

pub fn status() -> Status {
    match STATUS.load(Ordering::Relaxed) {
        1 => Status::Ready,
        2 => Status::NoPermission,
        _ => Status::Idle,
    }
}

/// Start the once-per-launch check. Never blocks the caller.
pub fn start(ctx: egui::Context) {
    let _ = std::thread::Builder::new()
        .name("update".into())
        .spawn(move || {
            let Ok(exe) = std::env::current_exe() else {
                return;
            };
            let _ = std::fs::remove_file(old_path(&exe));
            if std::env::var_os("AFTERIMAGE_NO_UPDATE").is_some() || is_dev_build(&exe) {
                return;
            }
            std::thread::sleep(START_DELAY);
            let status = run(&exe).unwrap_or(Status::Idle);
            if status != Status::Idle {
                STATUS.store(status as u8, Ordering::Relaxed);
                ctx.request_repaint();
            }
        });
}

/// `cargo run` must not get its binary replaced by a release.
fn is_dev_build(exe: &Path) -> bool {
    exe.components().any(|c| c.as_os_str() == "target")
}

fn run(exe: &Path) -> Option<Status> {
    let manifest = get_retrying(
        &format!("/{REPO}/releases/latest/download/latest.txt"),
        1024,
    )?;
    let manifest = String::from_utf8(manifest).ok()?;
    let key = VerifyingKey::from_bytes(&decode_hex(PUBLIC_KEY)?).ok()?;
    let (version, sha) = parse_manifest(&manifest, &key)?;
    if !is_newer(version, env!("CARGO_PKG_VERSION")) {
        return None;
    }

    let new = exe.with_extension(format!("exe.new{}", std::process::id()));
    if std::fs::write(&new, b"").is_err() {
        return Some(Status::NoPermission);
    }
    let installed = (|| {
        let path = format!("/{REPO}/releases/download/v{version}/{EXE_NAME}");
        let bytes = get_retrying(&path, MAX_DOWNLOAD)?;
        if !bytes.starts_with(b"MZ") || hex(&Sha256::digest(&bytes)) != sha {
            return None;
        }
        std::fs::write(&new, &bytes).ok()?;
        swap(exe, &new).ok()
    })();
    let _ = std::fs::remove_file(&new);
    installed.map(|()| Status::Ready)
}

/// Move the running exe aside and the new one into its place, undoing the
/// first step if the second fails.
fn swap(exe: &Path, new: &Path) -> std::io::Result<()> {
    let old = old_path(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old)?;
    std::fs::rename(new, exe).inspect_err(|_| {
        let _ = std::fs::rename(&old, exe);
    })
}

fn old_path(exe: &Path) -> PathBuf {
    exe.with_extension("exe.old")
}

/// `"<version> <sha256 hex> <signature hex>"`, signed over the first two fields.
/// `None` unless the signature matches `key`.
fn parse_manifest<'a>(text: &'a str, key: &VerifyingKey) -> Option<(&'a str, &'a str)> {
    let mut parts = text.split_whitespace();
    let version = parts.next()?;
    let sha = parts.next()?;
    let signature = Signature::from_bytes(&decode_hex(parts.next()?)?);
    key.verify_strict(format!("{version} {sha}").as_bytes(), &signature)
        .ok()?;
    version_key(version)?;
    Some((version, sha))
}

fn decode_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn version_key(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.split('.').map(|p| p.parse::<u32>().ok());
    let key = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(key)
}

fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((version_key(candidate), version_key(current)), (Some(a), Some(b)) if a > b)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A WinHTTP handle that closes itself.
struct Handle(*mut core::ffi::c_void);

impl Handle {
    fn new(raw: *mut core::ffi::c_void) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

fn get_retrying(path: &str, limit: usize) -> Option<Vec<u8>> {
    for attempt in 1..=ATTEMPTS {
        if let Some(body) = get(path, limit) {
            return Some(body);
        }
        if attempt < ATTEMPTS {
            std::thread::sleep(RETRY_DELAY);
        }
    }
    None
}

/// HTTPS GET on `HOST`, following redirects. `None` unless the final answer
/// is 200 and fits in `limit` bytes.
fn get(path: &str, limit: usize) -> Option<Vec<u8>> {
    unsafe {
        let agent = HSTRING::from(concat!("Afterimage/", env!("CARGO_PKG_VERSION")));
        let session = Handle::new(WinHttpOpen(
            &agent,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 30_000).ok()?;
        let connect = Handle::new(WinHttpConnect(
            session.0,
            &HSTRING::from(HOST),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ))?;
        let request = Handle::new(WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            &HSTRING::from(path),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).ok()?;

        let mut code = 0u32;
        let mut size = 4u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut code).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .ok()?;
        if code != 200 {
            return None;
        }

        let mut body = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            )
            .ok()?;
            if read == 0 {
                return Some(body);
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > limit {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_compares_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("garbage", "0.2.0"));
    }

    fn signed(key: &ed25519_dalek::SigningKey, payload: &str) -> String {
        use ed25519_dalek::Signer;
        format!(
            "{payload} {}\n",
            hex(&key.sign(payload.as_bytes()).to_bytes())
        )
    }

    #[test]
    fn manifest_needs_a_valid_signature() {
        let signer = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let key = signer.verifying_key();
        let sha = "a".repeat(64);
        let payload = format!("1.2.3 {sha}");

        let good = signed(&signer, &payload);
        assert_eq!(parse_manifest(&good, &key), Some(("1.2.3", sha.as_str())));

        // No signature: what releases before 0.4.0 published.
        assert_eq!(parse_manifest(&payload, &key), None);
        // Signed by someone else.
        let other = ed25519_dalek::SigningKey::from_bytes(&[8; 32]);
        assert_eq!(parse_manifest(&signed(&other, &payload), &key), None);
        // Valid signature, but the version or the hash was changed afterwards.
        assert_eq!(parse_manifest(&good.replace("1.2.3", "9.9.9"), &key), None);
        let tampered = good.replacen(&sha, &"b".repeat(64), 1);
        assert_eq!(parse_manifest(&tampered, &key), None);
    }

    #[test]
    fn embedded_public_key_is_valid() {
        let bytes = decode_hex(PUBLIC_KEY).unwrap();
        assert!(VerifyingKey::from_bytes(&bytes).is_ok());
    }
}
