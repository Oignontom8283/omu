//! Which certificates the program trusts when it connects to a server over HTTPS, and the HTTP client that uses them.
//!
//! A connection is only safe if the program knows who it is talking to. The server proves it with a certificate, vouched
//! for by an authority, and the program must already believe that authority. Two ways to choose whom to believe
//! ([`TrustMode`]):
//!
//! * **System** (the default): the authorities the operating system trusts, plus any certificate the user put in the
//!   certificates folder (a company's own authority, the authority of a test server...);
//! * **Own**: only the certificates in the certificates folder. Nothing the system trusts counts. For the user who
//!   does not want to depend on what the operating system, a company or a program installed in it decided to trust.
//!
//! The folder holds `.pem`, `.crt`, `.cer` or `.der` files; a PEM file may hold several certificates (a bundle).

use omu_types::{CertificateInfo, Diagnostic, Settings, TrustMode, TrustReport};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::CertificateDer;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The name of the folder, in the program's data folder, where the user's certificates go unless they chose another one.
pub const FOLDER_NAME: &str = "certificates";

pub mod server;
pub use server::{local_names, server_identity, Identity};

/// The extensions a certificate file may have.
const EXTENSIONS: [&str; 5] = ["pem", "crt", "cer", "cert", "der"];

/// Whom to believe, and where the user's certificates are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trust {
    pub mode: TrustMode,
    pub folder: PathBuf,
}

impl Trust {
    /// What the settings ask for; without a folder of their own the certificates go in `<data_dir>/certificates`.
    pub fn from_settings(settings: &Settings, data_dir: &Path) -> Trust {
        let folder = match settings.certificates_dir.as_deref().map(str::trim) {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => data_dir.join(FOLDER_NAME),
        };
        Trust {
            mode: settings.trust,
            folder,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("only your own certificates are trusted, and there is none in {0}")]
    NoOwnCertificate(String),
    #[error("cannot read the certificates of the system: {0}")]
    System(String),
    #[error("{0}")]
    Server(String),
}

/// A certificate read from the folder, with what is known about it.
struct Found {
    info: CertificateInfo,
    der: CertificateDer<'static>,
}

struct Loaded {
    found: Vec<Found>,
    problems: Vec<Diagnostic>,
    folder_exists: bool,
}

/// Reads every certificate file of `folder`, in the order of their names. A file that cannot be read is reported, never fatal.
fn load_folder(folder: &Path) -> Loaded {
    let mut loaded = Loaded {
        found: vec![],
        problems: vec![],
        folder_exists: folder.is_dir(),
    };
    let Ok(entries) = std::fs::read_dir(folder) else {
        return loaded;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        })
        .collect();
    files.sort();
    for path in files {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                loaded.problems.push(
                    Diagnostic::warning(
                        "certificate_unreadable",
                        format!("{name} cannot be read: {e}"),
                    )
                    .hint("Check that the file exists and that you may read it."),
                );
                continue;
            }
        };
        let mut ders: Vec<CertificateDer<'static>> = vec![];
        if bytes.windows(10).any(|w| w == b"-----BEGIN") {
            for item in CertificateDer::pem_slice_iter(&bytes) {
                match item {
                    Ok(der) => ders.push(der.into_owned()),
                    Err(e) => loaded.problems.push(Diagnostic::warning(
                        "certificate_invalid",
                        format!("{name} holds a certificate that cannot be read: {e}"),
                    )),
                }
            }
        } else {
            ders.push(CertificateDer::from(bytes));
        }
        if ders.is_empty() {
            loaded.problems.push(
                Diagnostic::warning(
                    "certificate_missing",
                    format!("{name} holds no certificate"),
                )
                .hint("A certificate file is a .pem, .crt, .cer or .der file, not a private key."),
            );
        }
        for der in ders {
            match describe(&name, &der) {
                Ok(info) => {
                    if info.expired {
                        loaded.problems.push(Diagnostic::warning(
                            "certificate_expired",
                            format!("{} ({name}) expired on {}", info.subject, info.expires),
                        ));
                    }
                    loaded.found.push(Found { info, der });
                }
                Err(e) => loaded.problems.push(Diagnostic::warning(
                    "certificate_invalid",
                    format!("{name} is not a valid certificate: {e}"),
                )),
            }
        }
    }
    loaded
}

fn describe(file: &str, der: &CertificateDer<'_>) -> Result<CertificateInfo, String> {
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).map_err(|e| e.to_string())?;
    let day = cert.validity().not_after.to_datetime();
    Ok(CertificateInfo {
        file: file.to_owned(),
        subject: cert.subject().to_string(),
        issuer: cert.issuer().to_string(),
        expires: format!(
            "{}-{:02}-{:02}",
            day.year(),
            u8::from(day.month()),
            day.day()
        ),
        expired: cert.validity().time_to_expiration().is_none(),
    })
}

/// What the program trusts now, for the user to see: the mode, the folder and what is in it, and what is wrong.
pub fn report(trust: &Trust) -> TrustReport {
    let loaded = load_folder(&trust.folder);
    let mut problems = loaded.problems;
    if trust.mode == TrustMode::Own && loaded.found.is_empty() {
        problems.insert(
            0,
            Diagnostic::error(
                "nothing_trusted",
                "Only your own certificates are trusted, and there is none: no secure connection to a server can be made.",
            )
            .hint("Put the certificates in the folder, or go back to trusting the certificates of the system."),
        );
    }
    TrustReport {
        mode: trust.mode,
        folder: trust.folder.display().to_string(),
        folder_exists: loaded.folder_exists,
        certificates: loaded.found.into_iter().map(|f| f.info).collect(),
        problems,
    }
}

/// Writes the certificates of the folder as one PEM bundle at `path` (what FFmpeg's own TLS reads), and says whether there was one
/// to write.
pub fn ca_bundle(trust: &Trust, path: &Path) -> Result<bool, Error> {
    let loaded = load_folder(&trust.folder);
    if loaded.found.is_empty() {
        return Ok(false);
    }
    let text: String = loaded
        .found
        .iter()
        .map(|f| pem::encode(&pem::Pem::new("CERTIFICATE", f.der.as_ref().to_vec())))
        .collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| Error::Server(format!("cannot create {}: {e}", dir.display())))?;
    }
    // written only when it changed: it is asked for every time a stream block opens
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(true);
    }
    std::fs::write(path, text)
        .map_err(|e| Error::Server(format!("cannot write {}: {e}", path.display())))?;
    Ok(true)
}

/// The certificates the HTTP client will believe.
fn roots(trust: &Trust) -> Result<ureq::tls::RootCerts, Error> {
    let own: Vec<ureq::tls::Certificate<'static>> = load_folder(&trust.folder)
        .found
        .iter()
        .map(|f| ureq::tls::Certificate::from_der(f.der.as_ref()).to_owned())
        .collect();
    match trust.mode {
        TrustMode::Own if own.is_empty() => {
            Err(Error::NoOwnCertificate(trust.folder.display().to_string()))
        }
        TrustMode::Own => Ok(ureq::tls::RootCerts::new_with_certs(&own)),
        // nothing to add: the system's own verifier (which also knows about revocation and the system's rules)
        TrustMode::System if own.is_empty() => Ok(ureq::tls::RootCerts::PlatformVerifier),
        TrustMode::System => {
            let native = rustls_native_certs::load_native_certs();
            if native.certs.is_empty() && !native.errors.is_empty() {
                return Err(Error::System(native.errors[0].to_string()));
            }
            let mut all: Vec<ureq::tls::Certificate<'static>> = native
                .certs
                .iter()
                .map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned())
                .collect();
            all.extend(own);
            Ok(ureq::tls::RootCerts::Specific(Arc::new(all)))
        }
    }
}

/// An HTTP(S) client that trusts what `trust` says: rustls everywhere, the same on every system.
pub fn agent(trust: &Trust) -> Result<ureq::Agent, Error> {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(roots(trust)?)
        .build();
    Ok(ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(None)
        .timeout_connect(Some(Duration::from_secs(30)))
        // the answer must start within a minute; the body then takes as long as the connection needs
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .build()
        .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omu-tls-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A self-signed certificate for `localhost`, as PEM, with its key.
    fn certificate(name: &str) -> (String, String) {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        let cert = params.self_signed(&key).unwrap();
        (cert.pem(), key.serialize_pem())
    }

    /// An HTTPS server on this computer that answers `ok` to the next connection; returns its port.
    fn server(cert_pem: &str, key_pem: &str) -> u16 {
        let certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<Result<_, _>>()
                .unwrap();
        let key = rustls_pki_types::PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = Arc::new(
            rustls::ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            // a failed handshake (the client refused the certificate) is not an error of the server
            for stream in listener.incoming().take(8).flatten() {
                let Ok(conn) = rustls::ServerConnection::new(config.clone()) else {
                    continue;
                };
                let mut tls = rustls::StreamOwned::new(conn, stream);
                let mut request = [0u8; 1024];
                if tls.read(&mut request).is_ok() {
                    let _ = tls.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    );
                    let _ = tls.flush();
                }
            }
        });
        port
    }

    fn get(trust: &Trust, port: u16) -> Result<String, String> {
        let agent = agent(trust).map_err(|e| e.to_string())?;
        let mut response = agent
            .get(&format!("https://localhost:{port}/"))
            .call()
            .map_err(|e| e.to_string())?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())
    }

    #[test]
    fn the_folder_is_read_whatever_the_format_and_what_is_wrong_is_reported() {
        let dir = temp_dir("folder");
        let (pem_a, key_a) = certificate("Authority A");
        let (pem_b, _) = certificate("Authority B");
        std::fs::write(dir.join("a.pem"), &pem_a).unwrap();
        // a bundle of two certificates, with a private key in the same file (it is not a certificate: ignored)
        std::fs::write(dir.join("bundle.crt"), format!("{pem_b}\n{pem_a}\n{key_a}")).unwrap();
        // a DER certificate
        let der = CertificateDer::pem_slice_iter(pem_b.as_bytes())
            .next()
            .unwrap()
            .unwrap();
        std::fs::write(dir.join("b.der"), der.as_ref()).unwrap();
        // not certificates
        std::fs::write(
            dir.join("broken.pem"),
            "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        std::fs::write(dir.join("key-only.pem"), &key_a).unwrap();
        std::fs::write(dir.join("notes.txt"), "not a certificate file").unwrap();
        // a certificate that is no longer valid is listed, and reported
        let old_key = rcgen::KeyPair::generate().unwrap();
        let mut old = rcgen::CertificateParams::new(vec!["old.example".into()]).unwrap();
        old.not_before = rcgen::date_time_ymd(2019, 1, 1);
        old.not_after = rcgen::date_time_ymd(2020, 1, 1);
        std::fs::write(
            dir.join("old.pem"),
            old.self_signed(&old_key).unwrap().pem(),
        )
        .unwrap();

        let trust = Trust {
            mode: TrustMode::System,
            folder: dir.clone(),
        };
        let r = report(&trust);
        assert!(r.folder_exists);
        assert_eq!(
            r.certificates.len(),
            5,
            "a.pem, two in bundle.crt, b.der, old.pem"
        );
        let old = r.certificates.iter().find(|c| c.file == "old.pem").unwrap();
        assert!(old.expired && old.expires == "2020-01-01", "{old:?}");
        assert!(r.problems.iter().any(|p| p.code == "certificate_expired"));
        assert!(r
            .certificates
            .iter()
            .any(|c| c.subject.contains("Authority A") && c.file == "a.pem" && !c.expired));
        assert!(r.certificates.iter().any(|c| c.file == "b.der"));
        let codes: Vec<&str> = r.problems.iter().map(|p| p.code.as_str()).collect();
        assert!(
            codes.contains(&"certificate_invalid"),
            "the broken one is reported: {codes:?}"
        );
        assert!(
            codes.contains(&"certificate_missing"),
            "a key is not a certificate: {codes:?}"
        );
        assert!(
            !r.problems.iter().any(|p| p.message.contains("notes.txt")),
            "other files are ignored"
        );
        assert!(!r.problems.iter().any(|p| p.code == "nothing_trusted"));
    }

    #[test]
    fn a_folder_that_does_not_exist_is_empty_not_an_error_but_trusting_only_it_is() {
        let dir = std::env::temp_dir().join(format!("omu-tls-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let own = Trust {
            mode: TrustMode::Own,
            folder: dir.clone(),
        };
        let r = report(&own);
        assert!(!r.folder_exists && r.certificates.is_empty());
        assert!(
            r.problems.iter().any(|p| p.code == "nothing_trusted"),
            "no certificate, nothing to trust"
        );
        assert!(matches!(agent(&own), Err(Error::NoOwnCertificate(_))));
        let system = Trust {
            mode: TrustMode::System,
            folder: dir,
        };
        assert!(
            report(&system).problems.is_empty(),
            "the system's certificates are enough"
        );
        assert!(agent(&system).is_ok());
    }

    #[test]
    fn the_folder_comes_from_the_settings_or_is_the_default_one() {
        let data = Path::new("data");
        let mut settings = Settings::default();
        assert_eq!(
            Trust::from_settings(&settings, data).folder,
            data.join("certificates")
        );
        settings.certificates_dir = Some("  ".into());
        assert_eq!(
            Trust::from_settings(&settings, data).folder,
            data.join("certificates")
        );
        settings.certificates_dir = Some("C:/my/certs".into());
        settings.trust = TrustMode::Own;
        let t = Trust::from_settings(&settings, data);
        assert_eq!(
            (t.mode, t.folder),
            (TrustMode::Own, PathBuf::from("C:/my/certs"))
        );
    }

    #[test]
    fn a_server_is_only_believed_if_its_certificate_is_trusted() {
        let dir = temp_dir("handshake");
        let (cert, key) = certificate("Test server");
        let (other_cert, _) = certificate("Somebody else");
        let port = server(&cert, &key);
        let own = |folder: &Path| Trust {
            mode: TrustMode::Own,
            folder: folder.to_owned(),
        };
        let system = |folder: &Path| Trust {
            mode: TrustMode::System,
            folder: folder.to_owned(),
        };

        // the system does not know this server: refused
        assert!(
            get(&system(&dir), port).is_err(),
            "a self-signed certificate nobody vouches for"
        );

        // its certificate in the folder: believed, in both modes
        std::fs::write(dir.join("server.pem"), &cert).unwrap();
        assert_eq!(get(&own(&dir), port).unwrap(), "ok");
        assert_eq!(
            get(&system(&dir), port).unwrap(),
            "ok",
            "the folder adds to what the system trusts"
        );

        // a folder with only another certificate: the server is refused, whatever the mode
        let other = temp_dir("handshake-other");
        std::fs::write(other.join("other.pem"), &other_cert).unwrap();
        assert!(get(&own(&other), port).is_err());
        assert!(get(&system(&other), port).is_err());
    }
}
