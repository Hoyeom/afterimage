//! Release signing, run on the maintainer's PC. Not part of the app.
//!
//! ```text
//! cargo run --example sign -- keygen
//! cargo run --example sign -- manifest <version> <afterimage.exe>
//! ```
//!
//! `keygen` creates the private key and prints the public key for
//! `PUBLIC_KEY` in `src/update.rs`. `manifest` prints the line that goes into
//! `latest.txt`: `<version> <sha256 of exe> <signature>`.
//!
//! The private key stays in `%USERPROFILE%\.afterimage\release-signing.key`.
//! Never commit it or put it in GitHub. Back it up somewhere safe: without it,
//! installed apps can't be sent another update.

use std::path::PathBuf;
use std::process::ExitCode;

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

fn key_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("USERPROFILE").ok_or("USERPROFILE is not set")?;
    Ok(PathBuf::from(home)
        .join(".afterimage")
        .join("release-signing.key"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn load_key() -> Result<SigningKey, String> {
    let path = key_path()?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let text = text.trim();
    if text.len() != 64 {
        return Err(format!("{} is not a signing key", path.display()));
    }
    let mut seed = [0u8; 32];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("{} is not a signing key", path.display()))?;
    }
    Ok(SigningKey::from_bytes(&seed))
}

fn keygen() -> Result<(), String> {
    let path = key_path()?;
    if path.exists() {
        return Err(format!(
            "{} already exists; replacing it would cut off installed apps",
            path.display()
        ));
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| e.to_string())?;
    let key = SigningKey::from_bytes(&seed);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, hex(&seed)).map_err(|e| e.to_string())?;
    println!("{}", hex(key.verifying_key().as_bytes()));
    Ok(())
}

fn manifest(version: &str, exe: &str) -> Result<(), String> {
    let bytes = std::fs::read(exe).map_err(|e| format!("can't read {exe}: {e}"))?;
    if !bytes.starts_with(b"MZ") {
        return Err(format!("{exe} is not an exe"));
    }
    let payload = format!("{version} {}", hex(&Sha256::digest(&bytes)));
    let signature = load_key()?.sign(payload.as_bytes());
    println!("{payload} {}", hex(&signature.to_bytes()));
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["keygen"] => keygen(),
        ["manifest", version, exe] => manifest(version, exe),
        _ => Err("usage: sign keygen | sign manifest <version> <exe>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
