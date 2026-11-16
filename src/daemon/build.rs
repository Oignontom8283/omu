//! Builds the web panel (with npm) when the daemon embeds it, so that building the daemon always embeds an up-to-date panel.
use std::{env, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("..")
        .join("..");
    omu_build::prepare_panel(&root, env::var_os("CARGO_FEATURE_WEB_PANEL").is_some());
}
