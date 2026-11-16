//! The window embeds the panel (built with npm) and takes its name and identifier from `branding.toml`.
use std::{env, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("..")
        .join("..");
    let branding = root.join("branding.toml");
    println!("cargo:rerun-if-changed={}", branding.display());
    omu_build::prepare_panel(&root, true);
    let config = omu_build::tauri_config(&branding);
    env::set_var("TAURI_CONFIG", &config);
    println!("cargo:rustc-env=TAURI_CONFIG={config}");
    tauri_build::build();
}
