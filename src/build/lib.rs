//! Helpers for the build scripts of the daemon and of the window: building the web panel (`ui/`) with npm, and the
//! window's Tauri configuration from `branding.toml`.

pub mod panel {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, SystemTime};

    /// Name of the npm executable: on Windows it is a `.cmd` file, which `Command` only finds when named in full.
    fn npm() -> &'static str {
        if cfg!(windows) {
            "npm.cmd"
        } else {
            "npm"
        }
    }

    fn newest(path: &Path, newest_seen: &mut SystemTime) {
        let Ok(meta) = std::fs::metadata(path) else {
            return;
        };
        if meta.is_dir() {
            let Ok(entries) = std::fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name == "node_modules" || name == "dist" {
                    continue;
                }
                newest(&entry.path(), newest_seen);
            }
        } else if let Ok(modified) = meta.modified() {
            if modified > *newest_seen {
                *newest_seen = modified;
            }
        }
    }

    /// Whether `ui/dist` is missing or older than the panel's sources (or the branding it reads).
    fn panel_is_stale(ui: &Path, branding: &Path) -> bool {
        let Ok(built) =
            std::fs::metadata(ui.join("dist").join("index.html")).and_then(|m| m.modified())
        else {
            return true;
        };
        let mut latest = SystemTime::UNIX_EPOCH;
        newest(ui, &mut latest);
        newest(branding, &mut latest);
        latest > built
    }

    fn run_npm(ui: &Path, args: &[&str]) -> Result<(), String> {
        let status = Command::new(npm())
            .args(args)
            .current_dir(ui)
            .status()
            .map_err(|e| {
                format!(
                    "cannot run `{} {}`: {e}. Node.js (with npm) is needed to build the web panel: install it from \
                     https://nodejs.org, or set OMU_SKIP_UI=1 to build without the panel",
                    npm(),
                    args.join(" ")
                )
            })?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| format!("`{} {}` failed ({status})", npm(), args.join(" ")))
    }

    struct Lock(PathBuf);

    impl Lock {
        /// Waits (up to two minutes) for the other build script, then takes the lock.
        fn take(path: PathBuf) -> Lock {
            for _ in 0..1200 {
                if std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .is_ok()
                {
                    return Lock(path);
                }
                // a lock older than ten minutes belongs to a build that died
                if std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t.elapsed().unwrap_or_default() > Duration::from_secs(600))
                {
                    let _ = std::fs::remove_file(&path);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Lock(path)
        }
    }

    impl Drop for Lock {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// Makes sure `<ui>/dist` holds the built panel. Panics (failing the build) with an explanation when npm is missing
    /// or fails, unless `OMU_SKIP_UI` is set.
    pub fn ensure_panel(ui: &Path, branding: &Path) {
        println!("cargo:rerun-if-env-changed=OMU_SKIP_UI");
        for input in [
            "src",
            "index.html",
            "package.json",
            "package-lock.json",
            "tsconfig.json",
            "vite.config.ts",
        ] {
            println!("cargo:rerun-if-changed={}", ui.join(input).display());
        }
        let _ = std::fs::create_dir_all(ui.join("dist"));
        if std::env::var_os("OMU_SKIP_UI").is_some() {
            println!("cargo:warning=OMU_SKIP_UI is set: building without the web panel");
            return;
        }
        if !ui.join("package.json").exists() {
            return;
        }
        let _lock = Lock::take(ui.join(".panel-build.lock"));
        // another build script may have finished the work while this one waited
        if !panel_is_stale(ui, branding) {
            return;
        }
        let installed = ui.join("node_modules").join(".package-lock.json");
        let lock = ui.join("package-lock.json");
        let deps_stale = match (
            std::fs::metadata(&installed).and_then(|m| m.modified()),
            std::fs::metadata(&lock).and_then(|m| m.modified()),
        ) {
            (Ok(i), Ok(l)) => l > i,
            (Err(_), _) => true,
            _ => false,
        };
        let result = (|| {
            if deps_stale {
                let install: &[&str] = if lock.exists() {
                    &["ci", "--no-audit", "--no-fund", "--loglevel=error"]
                } else {
                    &["install", "--no-audit", "--no-fund", "--loglevel=error"]
                };
                run_npm(ui, install)?;
            }
            run_npm(ui, &["run", "build"])
        })();
        if let Err(message) = result {
            panic!("the web panel could not be built: {message}");
        }
    }
}

/// The web panel: built when `build_it` (the daemon's `web-panel` feature, or the window), otherwise only its folder is made so
/// that the embedding has something to point at.
pub fn prepare_panel(root: &std::path::Path, build_it: bool) {
    let ui = root.join("ui");
    if build_it {
        panel::ensure_panel(&ui, &root.join("branding.toml"));
    } else {
        let _ = std::fs::create_dir_all(ui.join("dist"));
    }
}

/// The JSON the window's Tauri configuration is completed with: the product name, the identifier and the window.
pub fn tauri_config(branding: &std::path::Path) -> String {
    let text = std::fs::read_to_string(branding).unwrap();
    let table: toml::Table = text.parse().unwrap();
    let get = |key: &str| table[key].as_str().unwrap().to_owned();
    let (name, id) = (get("display_name"), get("app_id"));
    format!(
        r#"{{"productName":{name:?},"identifier":{id:?},"app":{{"windows":[{{"title":{name:?},"width":1280,"height":800,"minWidth":720,"minHeight":480}}]}}}}"#
    )
}
