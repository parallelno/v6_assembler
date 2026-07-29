use std::env;
use std::path::Path;
use std::process::Command;

use chrono::Local;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("Cargo must provide CARGO_MANIFEST_DIR");
    if let Some(git_head) = git_path(&manifest_dir, "HEAD") {
        println!("cargo:rerun-if-changed={git_head}");
    }

    let date = Local::now().format("%Y.%m.%d").to_string();
    let hash = git_short_hash(&manifest_dir).unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=V6ASM_VERSION={}-{}", date, hash);
}

fn git_path(manifest_dir: &str, path: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", manifest_dir, "rev-parse", "--git-path", path])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8(output.stdout).ok()?;
    let path = Path::new(path.trim());
    Some(if path.is_absolute() {
        path.to_string_lossy().into_owned()
    } else {
        Path::new(manifest_dir).join(path).to_string_lossy().into_owned()
    })
}

fn git_short_hash(manifest_dir: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", manifest_dir, "rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let hash = String::from_utf8(output.stdout).ok()?;
    let hash = hash.trim();
    if hash.is_empty() {
        None
    } else {
        Some(hash.to_string())
    }
}