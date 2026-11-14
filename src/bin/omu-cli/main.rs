//! `omu-cli`: command-line client of the daemon. See `omu-cli --help`.

fn main() -> std::process::ExitCode {
    match omu::cli::execute(std::env::args_os(), &mut std::io::stdout()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
