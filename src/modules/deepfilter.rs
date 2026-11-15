//! The DeepFilterNet module: a larger noise reduction model, downloaded once into the modules folder.
//!
//! DeepFilterNet (Hendrik Schröter, MIT or Apache-2.0) is a neural network that separates a voice from everything else.
//! The program carries the code that runs it (when built with the `deepfilter` feature); the trained model, a few
//! megabytes, is what this module downloads. The address is fixed to one version of the model and its SHA-256 is
//! written here, so what is installed is exactly what was checked.

use super::download::{agent, fetch, Counters, CANCELLED};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

/// The model's file name.
pub const MODEL_FILE: &str = "DeepFilterNet3_onnx.tar.gz";
/// Where the model comes from: the project's repository, at a fixed version.
pub const MODEL_URL: &str = "https://github.com/Rikorose/DeepFilterNet/raw/7b14b895fe7fc42cab72b4a4c1e49c982bd6404b/models/DeepFilterNet3_onnx.tar.gz";
/// What the model's SHA-256 must be.
pub const MODEL_SHA256: &str = "c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616";
/// About how big the download is, in megabytes.
pub const DOWNLOAD_MB: u32 = 8;

/// Where the model is kept.
pub fn model_path(modules_dir: &Path) -> PathBuf {
    modules_dir.join("deepfilter").join(MODEL_FILE)
}

/// Downloads the model from `url` and keeps it only if its SHA-256 is `sha256`.
pub fn download(
    modules_dir: &Path,
    url: &str,
    sha256: &str,
    counters: &Counters,
) -> Result<(), String> {
    let target = model_path(modules_dir);
    let dir = target.parent().expect("the model has a folder");
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    counters.done.store(0, Ordering::Relaxed);
    counters.total.store(0, Ordering::Relaxed);
    let partial = target.with_extension("gz.part");
    let result = fetch(&agent(), url, &partial, counters).and_then(|got| {
        if got.eq_ignore_ascii_case(sha256) {
            std::fs::rename(&partial, &target).map_err(|e| e.to_string())
        } else {
            Err(
                "the downloaded model does not match its published checksum: it was not installed"
                    .into(),
            )
        }
    });
    let _ = std::fs::remove_file(&partial);
    if let Err(e) = &result {
        if e != CANCELLED {
            let _ = std::fs::remove_file(&target);
        }
    }
    result
}
