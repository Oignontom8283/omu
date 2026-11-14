//! Derives the window's identity (product name, application id, title) from the workspace's `branding.toml`, so the
//! desktop window carries the same name as the other executables. `tauri.conf.json` holds the rest.

use std::{env, fs, path::PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let branding = manifest.join("../../branding.toml");
    println!("cargo:rerun-if-changed={}", branding.display());
    let text = fs::read_to_string(&branding)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", branding.display()));
    let table: toml::Table = text
        .parse()
        .unwrap_or_else(|e| panic!("invalid branding.toml: {e}"));
    let get = |key: &str| -> String {
        table
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("branding.toml: `{key}` must be a string"))
            .to_owned()
    };
    let (name, id) = (get("display_name"), get("app_id"));
    // Tauri merges this JSON over tauri.conf.json, both for tauri-build and for the macro that embeds the config.
    let config = format!(
        r#"{{"productName":{name:?},"identifier":{id:?},"app":{{"windows":[{{"title":{name:?},"width":1280,"height":800,"minWidth":720,"minHeight":480}}]}}}}"#
    );
    env::set_var("TAURI_CONFIG", &config);
    println!("cargo:rustc-env=TAURI_CONFIG={config}");
    // The window embeds the panel too: build it first (see build-support/panel.rs). Without the `custom-protocol`
    // feature the window loads a development server instead and needs no build.
    if env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_some() {
        panel::ensure_panel(&manifest.join(".."), &branding);
    } else {
        let _ = fs::create_dir_all(manifest.join("..").join("dist"));
    }
    tauri_build::build();
}

mod panel {
    include!("../../build-support/panel.rs");
}
