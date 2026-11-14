//! `omu-cli`: command-line client of the daemon. See `omu-cli --help`.

fn main() -> std::process::ExitCode {
    // A client has no log file and nothing to clean up: only a readable report if it ever panics.
    let _diagnostics = omu::diagnostics::init(omu::diagnostics::Settings {
        program: omu::branding::CLI_EXECUTABLE,
        data_dir: omu::discovery::data_dir(),
        dialog: false,
        console_log: false,
        track_session: false,
    });
    match omu::cli::execute(std::env::args_os(), &mut std::io::stdout()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
