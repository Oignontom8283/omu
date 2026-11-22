//! `win-installer`: builds the Windows installers, one per processor (see `just package`). Runs on Windows and on Linux.

use clap::{Parser, Subcommand};
use omu_win_installer::{
    build, definitions, driver_for, fetch_driver, find_makensis, find_programs, out_name, stage,
    Arch, Plan, Signer,
};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "win-installer",
    version,
    about = "Builds the installer of each processor whose programs are there (x64, arm64); signs when a certificate is given in the environment",
    args_conflicts_with_subcommands = true
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    /// The processors to build for (default: those whose programs a cargo build left, or that are given below).
    #[arg(long = "arch", value_parser = ["x64", "arm64"])]
    archs: Vec<String>,
    /// The folder with the programs built for x64 (default: where `cargo build --release --target x86_64-pc-windows-msvc` puts them).
    #[arg(long)]
    x64: Option<PathBuf>,
    /// The folder with the programs built for ARM64 (default: where `cargo build --release --target aarch64-pc-windows-msvc` puts them).
    #[arg(long)]
    arm64: Option<PathBuf>,
    /// Where the installers are written (default: `dist` at the root of the repository).
    #[arg(long)]
    out: Option<PathBuf>,
    /// Where the driver's installers are kept once downloaded (default: `installer/cache` in cargo's target folder).
    #[arg(long)]
    cache: Option<PathBuf>,
    /// Do not put the driver's installer in: smaller, and the driver is not offered.
    #[arg(long)]
    no_driver: bool,
    /// Where `makensis` is (default: `MAKENSIS`, the path, the usual place on Windows).
    #[arg(long)]
    makensis: Option<PathBuf>,
    /// Only print what would be given to `makensis`.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Signs files with the certificate of the environment (what the installer's script runs on the uninstaller and on itself).
    Sign { files: Vec<PathBuf> },
}

fn sign(files: &[PathBuf]) -> Result<(), String> {
    let signer = Signer::from_env()?
        .ok_or("no certificate is given (OMU_SIGN_PFX or OMU_SIGN_PFX_BASE64)")?;
    for file in files {
        signer.sign(file)?;
    }
    Ok(())
}

fn run(args: Args) -> Result<(), String> {
    if let Some(Command::Sign { files }) = &args.command {
        return sign(files);
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| omu_win_installer::root().join("target"));
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| omu_win_installer::root().join("dist"));
    let work = target.join("installer");
    let cache = args.cache.clone().unwrap_or_else(|| work.join("cache"));
    let signer = Signer::from_env()?;
    // a signed build has the files signed one by one by this same program, which the script runs
    let sign_command = match &signer {
        Some(_) => {
            let me = std::env::current_exe().map_err(|e| e.to_string())?;
            Some(format!("\"{}\" sign \"%1\"", me.display()))
        }
        None => {
            eprintln!("no certificate in the environment: the installers are not signed");
            None
        }
    };

    let wanted: Vec<Arch> = Arch::ALL
        .into_iter()
        .filter(|a| args.archs.is_empty() || args.archs.iter().any(|n| n == a.name()))
        .collect();
    let mut built = 0;
    for arch in wanted {
        let given = match arch {
            Arch::X64 => args.x64.clone(),
            Arch::Arm64 => args.arm64.clone(),
        };
        let programs = match given {
            Some(dir) => {
                omu_win_installer::check_programs(&dir)?;
                Some(dir)
            }
            None => find_programs(&target, arch),
        };
        let Some(programs) = programs else {
            if args.archs.iter().any(|n| n == arch.name()) {
                return Err(format!("no programs built for {}: build them first (`cargo build --release --target {}`), or give their folder", arch.name(), match arch {
                    Arch::X64 => "x86_64-pc-windows-msvc",
                    Arch::Arm64 => "aarch64-pc-windows-msvc",
                }));
            }
            eprintln!("no programs built for {}: no installer for it", arch.name());
            continue;
        };
        let version = env!("CARGO_PKG_VERSION").to_string();
        let driver = if args.no_driver || args.dry_run {
            None
        } else {
            let package = driver_for(arch);
            Some((package, fetch_driver(&package, &cache)?))
        };
        let (staged, license, notices) = if args.dry_run {
            (
                programs.clone(),
                programs.join("LICENSE.txt"),
                programs.join("THIRD-PARTY-NOTICES.md"),
            )
        } else {
            stage(
                &programs,
                &work.join("staging").join(arch.name()),
                signer.as_ref(),
            )?
        };
        let plan = Plan {
            out_file: out.join(out_name(&version, arch)),
            version,
            arch,
            programs: staged,
            license,
            notices,
            driver,
            sign_command: sign_command.clone(),
        };
        if args.dry_run {
            for d in definitions(&plan) {
                println!("-D{d}");
            }
        } else {
            build(&plan, &find_makensis(args.makensis.as_deref())?)?;
            if let Some(signer) = &signer {
                signer.sign(&plan.out_file)?;
            }
            println!("{}", plan.out_file.display());
        }
        built += 1;
    }
    if built == 0 {
        return Err("no installer built: no programs were found. Build them first (`cargo build --release --target x86_64-pc-windows-msvc`), or give their folder with --x64 / --arm64".into());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
