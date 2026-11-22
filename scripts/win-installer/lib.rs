//! Builds the Windows installer, one per processor (`x64`, `arm64`), on Windows or on Linux.
//!
//! The installer itself is a script for NSIS (`scripts/win-installer/omu.nsi`); this crate prepares what the script needs and runs `makensis`,
//! which exists for both systems, so that the same command builds it anywhere (a build server included):
//!
//! * the programs of a processor are looked for in the usual places of a cargo build, or given; only the processors whose programs
//!   exist get an installer (nobody has to build for ARM to try the x64 one);
//! * the driver's installer of a processor is downloaded once, and kept only if it is exactly the file that was chosen (a fixed
//!   SHA-256);
//! * the name of the product and of its executables come from the branding, never from a text of the script;
//! * when a certificate is given (the environment of a build server: see [`Signer`]), the programs, the uninstaller and the installer
//!   are signed; without one nothing is, which is what a build on a computer gives.

use omu_branding as branding;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// A processor the programs are built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Arch {
    X64,
    Arm64,
}

impl Arch {
    pub const ALL: [Arch; 2] = [Arch::X64, Arch::Arm64];

    /// How it is called in file names and in the script.
    pub fn name(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::Arm64 => "arm64",
        }
    }

    /// The cargo targets whose build folders hold its programs, best first.
    fn targets(self) -> &'static [&'static str] {
        match self {
            Arch::X64 => &["x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu"],
            Arch::Arm64 => &["aarch64-pc-windows-msvc", "aarch64-pc-windows-gnullvm"],
        }
    }
}

/// The installer of the driver (usbip-win2, made by someone else, signed by its publisher) that the installer carries.
#[derive(Debug, Clone, Copy)]
pub struct DriverPackage {
    pub arch: Arch,
    pub version: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    /// What the file must hash to (the one the publisher shows for it on its release page).
    pub sha256: &'static str,
}

/// The driver packages, one per processor. The one tried on a computer is the x64 one; there is no ARM64 package of that version, so
/// the ARM64 one is the nearest that exists, and it has not been tried.
pub const DRIVERS: [DriverPackage; 2] = [
    DriverPackage {
        arch: Arch::X64,
        version: "0.9.7.7",
        file: "USBip-0.9.7.7-x64.exe",
        url: "https://github.com/vadimgrn/usbip-win2/releases/download/v.0.9.7.7/USBip-0.9.7.7-x64.exe",
        sha256: "51620fa5f9f8be5932bc9d786deee557ce06d5407a99cab490dcfac71f185fea",
    },
    DriverPackage {
        arch: Arch::Arm64,
        version: "0.9.7.8",
        file: "USBip-0.9.7.8-arm64.exe",
        url: "https://github.com/vadimgrn/usbip-win2/releases/download/v.0.9.7.8/USBip-0.9.7.8-arm64.exe",
        sha256: "0fda9d3fc19e31753f57fcf8e3401939c41e259964a960b101f848c89fc19f19",
    },
];

/// The driver package of a processor.
pub fn driver_for(arch: Arch) -> DriverPackage {
    *DRIVERS
        .iter()
        .find(|d| d.arch == arch)
        .expect("every processor has a driver package")
}

/// Everything the script is told for one installer.
#[derive(Debug, Clone)]
pub struct Plan {
    pub version: String,
    pub arch: Arch,
    /// The folder with the programs, as they go into the installer.
    pub programs: PathBuf,
    pub license: PathBuf,
    pub notices: PathBuf,
    /// The driver's installer, when the installer carries it.
    pub driver: Option<(DriverPackage, PathBuf)>,
    pub out_file: PathBuf,
    /// The command that signs a file (`%1`), when a certificate is given.
    pub sign_command: Option<String>,
}

/// The repository's root (the installer's script and the licence are there).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// The three programs of the product, as file names.
pub fn programs() -> [String; 3] {
    let exe = |name: &str| format!("{name}.exe");
    [
        exe(branding::DAEMON_EXECUTABLE),
        exe(branding::CLI_EXECUTABLE),
        exe(branding::GUI_EXECUTABLE),
    ]
}

/// Whether `dir` holds the three programs.
pub fn check_programs(dir: &Path) -> Result<(), String> {
    for name in programs() {
        if !dir.join(&name).is_file() {
            return Err(format!("{} is missing in {}", name, dir.display()));
        }
    }
    Ok(())
}

/// Where a cargo build left the programs of `arch`, when it did: the folder of its target, or the plain one when this very
/// computer is that processor.
pub fn find_programs(target_dir: &Path, arch: Arch) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = arch
        .targets()
        .iter()
        .map(|t| target_dir.join(t).join("release"))
        .collect();
    if cfg!(all(windows, target_arch = "x86_64")) && arch == Arch::X64
        || cfg!(all(windows, target_arch = "aarch64")) && arch == Arch::Arm64
    {
        candidates.push(target_dir.join("release"));
    }
    candidates.into_iter().find(|d| check_programs(d).is_ok())
}

/// The SHA-256 of a file, in lower-case hexadecimal.
pub fn sha256_of(file: &Path) -> Result<String, String> {
    let mut input =
        std::fs::File::open(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut input, &mut hasher)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Checks that `file` is the file that was chosen.
pub fn verify(file: &Path, sha256: &str) -> Result<(), String> {
    let got = sha256_of(file)?;
    if got.eq_ignore_ascii_case(sha256) {
        Ok(())
    } else {
        Err(format!(
            "{} is not the file that was chosen (SHA-256 {got}, expected {sha256})",
            file.display()
        ))
    }
}

/// The driver's installer in `cache`: downloaded when it is not there, and kept only when it is the right file.
pub fn fetch_driver(package: &DriverPackage, cache: &Path) -> Result<PathBuf, String> {
    let file = cache.join(package.file);
    if file.is_file() && verify(&file, package.sha256).is_ok() {
        return Ok(file);
    }
    std::fs::create_dir_all(cache)
        .map_err(|e| format!("cannot create {}: {e}", cache.display()))?;
    eprintln!("downloading {}", package.url);
    let partial = cache.join(format!("{}.part", package.file));
    let mut response = ureq::get(package.url)
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(600)))
        .build()
        .call()
        .map_err(|e| format!("cannot download {}: {e}", package.url))?;
    let mut out = std::fs::File::create(&partial)
        .map_err(|e| format!("cannot write {}: {e}", partial.display()))?;
    std::io::copy(&mut response.body_mut().as_reader(), &mut out)
        .map_err(|e| format!("cannot download {}: {e}", package.url))?;
    drop(out);
    if let Err(e) = verify(&partial, package.sha256) {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    std::fs::rename(&partial, &file).map_err(|e| e.to_string())?;
    Ok(file)
}

/// `text` with every line ending written the Windows way (the installer shows the licence in a Windows control).
pub fn with_windows_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Copies what goes into the installer to `staging` (a folder of its own, so that nothing of a build is touched when the files are
/// signed): the three programs, the licence with Windows line endings, the notices. Signs the programs when `signer` is given.
/// Returns the folder, the licence and the notices.
pub fn stage(
    programs_dir: &Path,
    staging: &Path,
    signer: Option<&Signer>,
) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let _ = std::fs::remove_dir_all(staging);
    std::fs::create_dir_all(staging)
        .map_err(|e| format!("cannot create {}: {e}", staging.display()))?;
    for name in programs() {
        let to = staging.join(&name);
        std::fs::copy(programs_dir.join(&name), &to)
            .map_err(|e| format!("cannot copy {name}: {e}"))?;
        if let Some(signer) = signer {
            signer.sign(&to)?;
        }
    }
    let root = root();
    let text = std::fs::read_to_string(root.join("LICENSE"))
        .map_err(|e| format!("cannot read the licence: {e}"))?;
    let license = staging.join("LICENSE.txt");
    std::fs::write(&license, with_windows_line_endings(&text)).map_err(|e| e.to_string())?;
    let notices = staging.join("THIRD-PARTY-NOTICES.md");
    std::fs::copy(root.join("THIRD-PARTY-NOTICES.md"), &notices)
        .map_err(|e| format!("cannot copy the notices: {e}"))?;
    Ok((staging.to_path_buf(), license, notices))
}

/// `0.22.1` as the four numbers a Windows file version has (`0.22.1.0`); a pre-release suffix is left out.
pub fn version_quad(version: &str) -> String {
    let numbers: Vec<&str> = version
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .collect();
    let mut parts: Vec<String> = numbers
        .iter()
        .map(|n| {
            n.parse::<u16>()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "0".into())
        })
        .collect();
    parts.resize(4, "0".into());
    parts.truncate(4);
    parts.join(".")
}

/// The name of the installer of a processor.
pub fn out_name(version: &str, arch: Arch) -> String {
    format!("{}-{version}-setup-{}.exe", branding::SLUG, arch.name())
}

/// The definitions given to `makensis` (`-DNAME=value`): the script reads everything it needs from them.
pub fn definitions(plan: &Plan) -> Vec<String> {
    let root = root();
    let [daemon, cli, gui] = programs();
    let mut defs = vec![
        format!("PRODUCT_NAME={}", branding::DISPLAY_NAME),
        format!("SLUG={}", branding::SLUG),
        format!("DATA_DIR_NAME={}", branding::DATA_DIR_NAME),
        format!("APP_ID={}", branding::APP_ID),
        format!("VERSION={}", plan.version),
        format!("VERSION_QUAD={}", version_quad(&plan.version)),
        format!("ARCH={}", plan.arch.name()),
        format!("PROGRAMS_DIR={}", plan.programs.display()),
        format!("DAEMON_EXE={daemon}"),
        format!("CLI_EXE={cli}"),
        format!("GUI_EXE={gui}"),
        format!(
            "ICON_FILE={}",
            root.join("icons").join("icon.ico").display()
        ),
        format!("LICENSE_FILE={}", plan.license.display()),
        format!("NOTICES_FILE={}", plan.notices.display()),
        format!("OUT_FILE={}", plan.out_file.display()),
    ];
    if let Some((package, file)) = &plan.driver {
        defs.push(format!("DRIVER_FILE={}", file.display()));
        defs.push(format!("DRIVER_VERSION={}", package.version));
    }
    if let Some(command) = &plan.sign_command {
        defs.push(format!("SIGN_COMMAND={command}"));
    }
    defs
}

/// Where `makensis` is: the one asked for, `MAKENSIS` in the environment, the system's path, or the usual place on Windows.
pub fn find_makensis(asked: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = asked {
        return Ok(p.to_path_buf());
    }
    if let Some(p) = std::env::var_os("MAKENSIS") {
        return Ok(PathBuf::from(p));
    }
    let exe = if cfg!(windows) {
        "makensis.exe"
    } else {
        "makensis"
    };
    if let Some(found) = find_in_path(exe) {
        return Ok(found);
    }
    if cfg!(windows) {
        for base in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(dir) = std::env::var_os(base) {
                let p = Path::new(&dir).join("NSIS").join(exe);
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }
    Err("makensis (NSIS) is not installed: `winget install NSIS.NSIS` on Windows, `apt install nsis` on Debian and Ubuntu".into())
}

fn find_in_path(exe: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
}

/// Builds the installer of `plan` with `makensis`.
pub fn build(plan: &Plan, makensis: &Path) -> Result<(), String> {
    if let Some(dir) = plan.out_file.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("omu.nsi");
    let mut command = std::process::Command::new(makensis);
    command.arg("-V2");
    for d in definitions(plan) {
        command.arg(format!("-D{d}"));
    }
    command.arg(&script);
    let status = command
        .status()
        .map_err(|e| format!("cannot run {}: {e}", makensis.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("makensis failed ({status})"))
    }
}

/// How files are signed. It comes from the environment, so that a certificate is never written in a command line or a file of the
/// repository (a build server keeps it as a secret): `OMU_SIGN_PFX` (a `.pfx` file) or `OMU_SIGN_PFX_BASE64` (its content, as a
/// secret holds it), `OMU_SIGN_PASSWORD`, and optionally `OMU_SIGN_TIMESTAMP` (the time-stamping server). It is `osslsigncode` that
/// signs (it runs on Windows and on Linux): `OSSLSIGNCODE` or the path tell where it is.
#[derive(Debug, Clone)]
pub struct Signer {
    certificate: Certificate,
    password: String,
    timestamp: String,
}

#[derive(Debug, Clone)]
enum Certificate {
    File(PathBuf),
    Base64(String),
}

impl Signer {
    /// The signer the environment describes; `None` when no certificate is given.
    pub fn from_env() -> Result<Option<Signer>, String> {
        let get = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let certificate = match (get("OMU_SIGN_PFX"), get("OMU_SIGN_PFX_BASE64")) {
            (Some(file), _) => Certificate::File(PathBuf::from(file)),
            (None, Some(text)) => Certificate::Base64(text),
            (None, None) => return Ok(None),
        };
        let password = get("OMU_SIGN_PASSWORD")
            .ok_or("a certificate is given but OMU_SIGN_PASSWORD is not")?;
        Ok(Some(Signer {
            certificate,
            password,
            timestamp: get("OMU_SIGN_TIMESTAMP")
                .unwrap_or_else(|| "http://timestamp.digicert.com".into()),
        }))
    }

    /// Signs `file` in place.
    pub fn sign(&self, file: &Path) -> Result<(), String> {
        use base64::Engine;
        let tool = std::env::var_os("OSSLSIGNCODE")
            .map(PathBuf::from)
            .or_else(|| {
                find_in_path(if cfg!(windows) {
                    "osslsigncode.exe"
                } else {
                    "osslsigncode"
                })
            })
            .ok_or(
                "osslsigncode is not installed (`apt install osslsigncode`): it signs the files",
            )?;
        // the certificate and the password go through files only this process can read, never through a command line
        let secrets = SecretFolder::new()?;
        let pfx = match &self.certificate {
            Certificate::File(p) => p.clone(),
            Certificate::Base64(text) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(text.split_whitespace().collect::<String>())
                    .map_err(|e| format!("OMU_SIGN_PFX_BASE64 is not base64: {e}"))?;
                secrets.write("certificate.pfx", &bytes)?
            }
        };
        let password = secrets.write("password", self.password.as_bytes())?;
        let signed = file.with_extension("signed");
        let status = std::process::Command::new(tool)
            .args(["sign", "-pkcs12"])
            .arg(&pfx)
            .arg("-readpass")
            .arg(&password)
            .args([
                "-h",
                "sha256",
                "-n",
                branding::DISPLAY_NAME,
                "-t",
                &self.timestamp,
                "-in",
            ])
            .arg(file)
            .arg("-out")
            .arg(&signed)
            .status()
            .map_err(|e| format!("cannot run osslsigncode: {e}"))?;
        if !status.success() {
            let _ = std::fs::remove_file(&signed);
            return Err(format!(
                "osslsigncode could not sign {} ({status})",
                file.display()
            ));
        }
        std::fs::rename(&signed, file).map_err(|e| e.to_string())
    }
}

/// A folder that only this process can read, removed when it is dropped.
struct SecretFolder(PathBuf);

impl SecretFolder {
    fn new() -> Result<SecretFolder, String> {
        let dir = std::env::temp_dir().join(format!(
            "omu-installer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Ok(SecretFolder(dir))
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
        let file = self.0.join(name);
        std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
        Ok(file)
    }
}

impl Drop for SecretFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            version: "1.2.3".into(),
            arch: Arch::Arm64,
            programs: PathBuf::from("staging"),
            license: PathBuf::from("staging/LICENSE.txt"),
            notices: PathBuf::from("staging/NOTICES.md"),
            driver: Some((driver_for(Arch::Arm64), PathBuf::from("drv/x.exe"))),
            out_file: PathBuf::from("out/setup.exe"),
            sign_command: None,
        }
    }

    #[test]
    fn a_version_becomes_four_numbers() {
        assert_eq!(version_quad("0.22.1"), "0.22.1.0");
        assert_eq!(version_quad("1.2.3.4"), "1.2.3.4");
        assert_eq!(version_quad("1.2-beta.1"), "1.2.0.0");
        assert_eq!(version_quad("2"), "2.0.0.0");
    }

    #[test]
    fn the_name_says_the_processor() {
        assert_eq!(out_name("1.0.0", Arch::X64), "omu-1.0.0-setup-x64.exe");
        assert_eq!(out_name("1.0.0", Arch::Arm64), "omu-1.0.0-setup-arm64.exe");
    }

    #[test]
    fn the_script_is_told_what_the_installer_carries() {
        let defs = definitions(&plan());
        for expected in [
            "VERSION=1.2.3",
            "VERSION_QUAD=1.2.3.0",
            "ARCH=arm64",
            "PROGRAMS_DIR=staging",
            "DRIVER_FILE=drv/x.exe",
            "DRIVER_VERSION=0.9.7.8",
        ] {
            assert!(
                defs.iter().any(|d| d == expected),
                "{expected} is missing in {defs:?}"
            );
        }
        assert!(defs
            .iter()
            .any(|d| d == &format!("PRODUCT_NAME={}", branding::DISPLAY_NAME)));
        assert!(
            !defs.iter().any(|d| d.starts_with("SIGN_COMMAND")),
            "nothing is signed without a certificate"
        );
    }

    #[test]
    fn without_a_driver_the_script_is_not_told_of_one_and_with_a_certificate_it_signs() {
        let mut p = plan();
        p.driver = None;
        p.sign_command = Some("\"tool\" sign \"%1\"".into());
        let defs = definitions(&p);
        assert!(!defs.iter().any(|d| d.starts_with("DRIVER_")));
        assert!(defs
            .iter()
            .any(|d| d == "SIGN_COMMAND=\"tool\" sign \"%1\""));
    }

    #[test]
    fn a_file_is_only_taken_for_the_one_that_was_chosen() {
        let dir = std::env::temp_dir().join(format!("omu-installer-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.bin");
        std::fs::write(&file, b"abc").unwrap();
        // the SHA-256 of "abc"
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(&file, abc).is_ok());
        assert!(
            verify(&file, &abc.to_uppercase()).is_ok(),
            "case does not matter"
        );
        assert!(verify(&file, &abc.replace('b', "c")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn programs_are_found_in_the_folder_of_their_target() {
        let dir = std::env::temp_dir().join(format!("omu-installer-find-{}", std::process::id()));
        let release = dir.join("aarch64-pc-windows-msvc").join("release");
        std::fs::create_dir_all(&release).unwrap();
        assert_eq!(find_programs(&dir, Arch::Arm64), None, "nothing built yet");
        for name in programs() {
            std::fs::write(release.join(name), b"").unwrap();
        }
        assert_eq!(find_programs(&dir, Arch::Arm64), Some(release));
        assert_eq!(find_programs(&dir, Arch::X64), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn staging_copies_the_programs_and_gives_the_licence_windows_line_endings() {
        let dir = std::env::temp_dir().join(format!("omu-installer-stage-{}", std::process::id()));
        let built = dir.join("built");
        std::fs::create_dir_all(&built).unwrap();
        for name in programs() {
            std::fs::write(built.join(name), b"x").unwrap();
        }
        let (folder, license, _) = stage(&built, &dir.join("staging"), None).unwrap();
        assert!(check_programs(&folder).is_ok());
        let text = std::fs::read_to_string(license).unwrap();
        assert!(text.contains("\r\n") && !text.replace("\r\n", "").contains('\n'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn line_endings_are_not_doubled() {
        assert_eq!(with_windows_line_endings("a\nb\r\nc"), "a\r\nb\r\nc");
    }

    #[test]
    fn the_driver_packages_are_pinned() {
        for p in DRIVERS {
            assert_eq!(p.sha256.len(), 64, "{}", p.file);
            assert!(p.url.ends_with(p.file));
            assert!(p.url.starts_with("https://"));
        }
    }
}
