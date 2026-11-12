//! Optional compile-time download of the upstream `libemulator` shared library.
//!
//! This build script only runs when the `emulator-download` feature is active.
//! It downloads a released shared library into `OUT_DIR` and embeds its
//! absolute path so `EmulatorEngine::open_default` can find it without runtime
//! environment variables.
//!
//! Upstream publishes only shared libraries, so this cannot statically link
//! `libemulator`; the crate continues to load it with `libloading` at runtime.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RELEASE_ENV: &str = "TONUTILS_EMULATOR_RELEASE";
const URL_ENV: &str = "TONUTILS_EMULATOR_LIB_URL";
const DEFAULT_RELEASE: &str = "latest";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed={RELEASE_ENV}");
    println!("cargo:rerun-if-env-changed={URL_ENV}");

    if env::var_os("CARGO_FEATURE_EMULATOR_DOWNLOAD").is_none() {
        return;
    }

    // docs.rs builds with `all-features`, but its sandbox has no reliable
    // network access and docs do not need a native library.
    if env::var_os("DOCS_RS").is_some() {
        println!("cargo:warning=skipping libemulator download on docs.rs");
        return;
    }

    let target_os = required_var("CARGO_CFG_TARGET_OS");
    let target_arch = required_var("CARGO_CFG_TARGET_ARCH");
    let Some(asset) = asset_name(&target_os, &target_arch) else {
        panic!(
            "`emulator-download` does not support target {target_arch}-{target_os}; \
             upstream publishes libemulator only for linux x86_64/aarch64, \
             macos x86_64/aarch64, and windows x86_64"
        );
    };

    let release = env::var(RELEASE_ENV).unwrap_or_else(|_| DEFAULT_RELEASE.to_owned());
    let custom_url = env::var(URL_ENV).ok().filter(|value| !value.is_empty());
    let url = custom_url.unwrap_or_else(|| release_url(&release, asset));

    let out_dir = PathBuf::from(required_var("OUT_DIR"));
    let destination = out_dir.join("libemulator").join(&release).join(asset);
    if destination.is_file() {
        println!(
            "cargo:rustc-env=TONUTILS_EMULATOR_DOWNLOADED_LIB={}",
            destination.display()
        );
        return;
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("cannot create {}: {error}", parent.display()));
    }

    println!("cargo:warning=downloading libemulator from {url}");
    download(&url, &destination);
    println!(
        "cargo:rustc-env=TONUTILS_EMULATOR_DOWNLOADED_LIB={}",
        destination.display()
    );
}

fn required_var(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("Cargo did not set {name}"))
}

fn asset_name(target_os: &str, target_arch: &str) -> Option<&'static str> {
    match (target_os, target_arch) {
        ("linux", "x86_64") => Some("libemulator-linux-x86_64.so"),
        ("linux", "aarch64") => Some("libemulator-linux-arm64.so"),
        ("macos", "x86_64") => Some("libemulator-mac-x86-64.dylib"),
        ("macos", "aarch64") => Some("libemulator-mac-arm64.dylib"),
        ("windows", "x86_64") => Some("libemulator.dll"),
        _ => None,
    }
}

fn release_url(release: &str, asset: &str) -> String {
    if release == "latest" {
        format!("https://github.com/ton-blockchain/ton/releases/latest/download/{asset}")
    } else {
        format!("https://github.com/ton-blockchain/ton/releases/download/{release}/{asset}")
    }
}

fn download(url: &str, destination: &Path) {
    let partial = destination.with_extension("part");
    let status = Command::new("curl")
        .arg("--fail")
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--retry")
        .arg("3")
        .arg("--retry-all-errors")
        .arg("--output")
        .arg(&partial)
        .arg(url)
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "cannot run curl to download libemulator; install curl or pre-download the \
                 library and set TONUTILS_EMULATOR_LIB at runtime: {error}"
            )
        });
    if !status.success() {
        let _ = fs::remove_file(&partial);
        panic!("curl failed to download libemulator from {url} with {status}");
    }
    fs::rename(&partial, destination)
        .unwrap_or_else(|error| panic!("cannot move {}: {error}", partial.display()));
}
