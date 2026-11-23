//! What the installer asks of the program once its files are in place (and before they are removed).

use omu_soundboard::library::Library;
use std::path::Path;

/// Makes the setting "start at logon" what `enabled` says, in the settings kept in `data_dir`. The daemon that runs there
/// (or starts later) makes the startup list follow it.
pub fn save_start_at_logon(data_dir: &Path, enabled: bool) -> Result<(), String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| format!("cannot create {}: {e}", data_dir.display()))?;
    let library = Library::load(data_dir);
    let mut settings = library.settings();
    if settings.start_at_logon == enabled {
        return Ok(());
    }
    settings.start_at_logon = enabled;
    library
        .set_settings(settings)
        .map_err(|e| format!("cannot save the settings: {e}"))
}

/// The same, and the startup list of the current user is made to match at once (this program's own file is what it runs), so that
/// it is right even if no daemon is running.
pub fn set_start_at_logon(data_dir: &Path, enabled: bool) -> Result<(), String> {
    save_start_at_logon(data_dir, enabled)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    omu_desktop::autostart::apply(omu_desktop::autostart::ENTRY, enabled, &exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_is_saved_and_changed_back_without_touching_the_others() {
        let dir = crate::store::temp_dir("setup-autostart");
        assert!(!Library::load(&dir).settings().start_at_logon);
        save_start_at_logon(&dir, true).unwrap();
        let mut now = Library::load(&dir).settings();
        assert!(now.start_at_logon);
        now.sound_cache_mb = 77;
        Library::load(&dir).set_settings(now).unwrap();
        save_start_at_logon(&dir, false).unwrap();
        let after = Library::load(&dir).settings();
        assert!(!after.start_at_logon);
        assert_eq!(after.sound_cache_mb, 77, "what else was set stays");
    }
}
