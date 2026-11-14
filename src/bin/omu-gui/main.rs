// No console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let _diagnostics = omu::diagnostics::init(omu::diagnostics::Settings {
        program: omu::branding::GUI_EXECUTABLE,
        data_dir: omu::discovery::data_dir(),
        dialog: true,
        console_log: cfg!(debug_assertions),
        track_session: false,
    });
    omu::gui::run();
}
