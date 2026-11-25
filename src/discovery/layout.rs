//! Where everything the program keeps is, in its data folder. One place says it, and nothing else writes a path by hand.
//!
//! ```text
//! <data folder>/
//!   config/          what a person may edit by hand
//!     settings.json
//!     certificates/  their own certificates
//!     backups/       earlier versions of the files above
//!   data/            what the program keeps: not to be edited
//!     state.json  layout.json  soundboards.json  modules.json
//!     sounds/  modules/  tls/
//!     backups/       earlier versions of the files above
//!   run/             what exists only while the program runs
//!     daemon.json  omud.session  restoring
//!   logs/            the journal
//!   crashes/         the reports of crashes
//! ```
//!
//! The first versions put everything at the root of the folder, in a heap. [`DataLayout::migrate`] moves what a previous version left
//! to where it belongs: nothing is ever overwritten, a file that cannot be moved stays where it is (and is reported), and it can be run
//! again without harm.

use std::io;
use std::path::{Path, PathBuf};

/// The paths of the data folder `root`.
#[derive(Debug, Clone)]
pub struct DataLayout {
    root: PathBuf,
}

/// What [`DataLayout::migrate`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Migration {
    /// What was moved, as `from -> to` (relative to the data folder).
    pub moved: Vec<String>,
    /// What stayed where it was, and why.
    pub left: Vec<String>,
}

impl DataLayout {
    pub fn new(root: impl Into<PathBuf>) -> DataLayout {
        DataLayout { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// What a person may edit by hand.
    pub fn config(&self) -> PathBuf {
        self.root.join("config")
    }

    /// What the program keeps (not to be edited).
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    /// What exists only while the program runs.
    pub fn run(&self) -> PathBuf {
        self.root.join("run")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config().join("settings.json")
    }

    /// The folder of the certificates people put there.
    pub fn certificates(&self) -> PathBuf {
        self.config().join("certificates")
    }

    pub fn state_file(&self) -> PathBuf {
        self.data().join("state.json")
    }

    pub fn layout_file(&self) -> PathBuf {
        self.data().join("layout.json")
    }

    pub fn soundboards_file(&self) -> PathBuf {
        self.data().join("soundboards.json")
    }

    pub fn modules_file(&self) -> PathBuf {
        self.data().join("modules.json")
    }

    pub fn sounds(&self) -> PathBuf {
        self.data().join("sounds")
    }

    /// Where the modules that were downloaded are.
    pub fn modules(&self) -> PathBuf {
        self.data().join("modules")
    }

    /// The certificate the program makes for its own secure server.
    pub fn tls(&self) -> PathBuf {
        self.data().join("tls")
    }

    /// What the running daemon publishes about itself (its address and its secret).
    pub fn daemon_file(&self) -> PathBuf {
        self.run().join("daemon.json")
    }

    /// The marker of a run of `program` (it tells the next start whether this one ended cleanly).
    pub fn session_file(&self, program: &str) -> PathBuf {
        self.run().join(format!("{program}.session"))
    }

    /// Present while the graph of the last run is being started again (a graph that takes the daemon down is not started twice).
    pub fn restoring_marker(&self) -> PathBuf {
        self.run().join("restoring")
    }

    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn crashes(&self) -> PathBuf {
        self.root.join("crashes")
    }

    /// Moves what an earlier version left at the root of the data folder to where it belongs now.
    pub fn migrate(&self) -> Migration {
        let mut report = Migration::default();
        // the documents, with the earlier versions of each (`name.bak1`...) and the ones that could not be read (`name.bad-...`)
        let documents = [
            ("settings.json", self.settings_file()),
            ("state.json", self.state_file()),
            ("layout.json", self.layout_file()),
            ("soundboards.json", self.soundboards_file()),
            ("modules.json", self.modules_file()),
        ];
        for (name, target) in &documents {
            self.move_one(&self.root.join(name), target, &mut report);
            let backups = target.parent().unwrap_or(&self.root).join("backups");
            for entry in std::fs::read_dir(&self.root)
                .into_iter()
                .flatten()
                .flatten()
            {
                let file = entry.file_name().to_string_lossy().into_owned();
                if file.starts_with(&format!("{name}.bak"))
                    || file.starts_with(&format!("{name}.bad-"))
                {
                    self.move_one(&entry.path(), &backups.join(&file), &mut report);
                }
            }
        }
        // the folders
        self.move_one(
            &self.root.join("certificates"),
            &self.certificates(),
            &mut report,
        );
        self.move_one(&self.root.join("sounds"), &self.sounds(), &mut report);
        self.move_one(&self.root.join("modules"), &self.modules(), &mut report);
        self.move_one(&self.root.join("tls"), &self.tls(), &mut report);
        // what exists while it runs
        self.move_one(
            &self.root.join("daemon.json"),
            &self.daemon_file(),
            &mut report,
        );
        self.move_one(
            &self.root.join("restoring"),
            &self.restoring_marker(),
            &mut report,
        );
        for entry in std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
        {
            let file = entry.file_name().to_string_lossy().into_owned();
            if file.ends_with(".session") && entry.path().is_file() {
                self.move_one(&entry.path(), &self.run().join(&file), &mut report);
            }
        }
        for line in &report.left {
            tracing::warn!("the data folder was not tidied: {line}");
        }
        if !report.moved.is_empty() {
            tracing::info!(
                "the data folder was tidied: {} thing(s) moved",
                report.moved.len()
            );
        }
        report
    }

    fn name(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn move_one(&self, from: &Path, to: &Path, report: &mut Migration) {
        if !from.exists() {
            return;
        }
        let label = format!("{} -> {}", self.name(from), self.name(to));
        if to.exists() {
            // an empty folder made in the meantime is not in the way; anything else is never replaced
            if !(to.is_dir() && std::fs::remove_dir(to).is_ok()) {
                report.left.push(format!("{label}: the new place is taken"));
                return;
            }
        }
        let moved = to
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| rename(from, to));
        match moved {
            Ok(()) => report.moved.push(label),
            Err(e) => report.left.push(format!("{label}: {e}")),
        }
    }
}

/// A move on the same disk; tried again a few times, because another program that has the file open (an antivirus) makes it fail for a moment.
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let mut last = None;
    for attempt in 0..4u64 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(40 * (attempt + 1)));
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("cannot move")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omu-layout-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn what_an_earlier_version_left_at_the_root_goes_where_it_belongs_with_its_backups_and_nothing_is_lost(
    ) {
        let root = folder("heap");
        for name in [
            "settings.json",
            "state.json",
            "layout.json",
            "soundboards.json",
            "modules.json",
            "daemon.json",
            "omud.session",
            "restoring",
        ] {
            touch(&root.join(name), name);
        }
        for backup in [
            "state.json.bak1",
            "state.json.bak2",
            "settings.json.bak1",
            "soundboards.json.bad-17",
        ] {
            touch(&root.join(backup), backup);
        }
        touch(&root.join("sounds/a.wav"), "sound");
        touch(&root.join("certificates/mine.pem"), "cert");
        touch(&root.join("modules/deepfilter/model.tar.gz"), "model");
        touch(&root.join("tls/server.pem"), "server");
        touch(&root.join("logs/omud.log"), "log");
        let layout = DataLayout::new(&root);

        let report = layout.migrate();
        assert!(report.left.is_empty(), "{:?}", report.left);

        let read = |p: PathBuf| std::fs::read_to_string(p).unwrap();
        assert_eq!(read(layout.settings_file()), "settings.json");
        assert_eq!(read(layout.state_file()), "state.json");
        assert_eq!(read(layout.layout_file()), "layout.json");
        assert_eq!(read(layout.soundboards_file()), "soundboards.json");
        assert_eq!(read(layout.modules_file()), "modules.json");
        assert_eq!(read(layout.daemon_file()), "daemon.json");
        assert_eq!(read(layout.session_file("omud")), "omud.session");
        assert_eq!(read(layout.restoring_marker()), "restoring");
        assert_eq!(
            read(layout.data().join("backups/state.json.bak2")),
            "state.json.bak2"
        );
        assert_eq!(
            read(layout.data().join("backups/soundboards.json.bad-17")),
            "soundboards.json.bad-17"
        );
        assert_eq!(
            read(layout.config().join("backups/settings.json.bak1")),
            "settings.json.bak1"
        );
        assert_eq!(read(layout.sounds().join("a.wav")), "sound");
        assert_eq!(read(layout.certificates().join("mine.pem")), "cert");
        assert_eq!(
            read(layout.modules().join("deepfilter/model.tar.gz")),
            "model"
        );
        assert_eq!(read(layout.tls().join("server.pem")), "server");
        assert_eq!(
            read(layout.logs().join("omud.log")),
            "log",
            "the journal stays where it is"
        );
        // nothing is left in the heap
        let mut at_root: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        at_root.sort();
        assert_eq!(at_root, ["config", "data", "logs", "run"]);

        // and again changes nothing
        assert_eq!(layout.migrate(), Migration::default());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_is_never_replaced_and_what_cannot_be_moved_is_reported_and_stays() {
        let root = folder("taken");
        touch(&root.join("state.json"), "old one");
        touch(&root.join("data/state.json"), "new one");
        let layout = DataLayout::new(&root);
        let report = layout.migrate();
        assert_eq!(
            std::fs::read_to_string(layout.state_file()).unwrap(),
            "new one"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("state.json")).unwrap(),
            "old one",
            "the old one is kept"
        );
        assert_eq!(report.moved.len(), 0);
        assert_eq!(report.left.len(), 1);
        assert!(report.left[0].contains("state.json"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_that_was_made_empty_in_the_meantime_does_not_stop_the_move() {
        let root = folder("empty");
        touch(&root.join("sounds/a.wav"), "sound");
        std::fs::create_dir_all(root.join("data/sounds")).unwrap();
        let layout = DataLayout::new(&root);
        assert!(layout.migrate().left.is_empty());
        assert!(layout.sounds().join("a.wav").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_fresh_folder_has_nothing_to_move() {
        let root = folder("fresh");
        assert_eq!(DataLayout::new(&root).migrate(), Migration::default());
        let _ = std::fs::remove_dir_all(&root);
    }
}
