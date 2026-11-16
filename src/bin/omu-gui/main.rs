// No console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let _diagnostics = omu_diagnostics::init(omu_diagnostics::Settings {
        program: omu_branding::GUI_EXECUTABLE,
        data_dir: omu_discovery::data_dir(),
        dialog: true,
        console_log: cfg!(debug_assertions),
    });
    omu_gui::run();
}
